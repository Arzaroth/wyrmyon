mod support;

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::OnceLock;

use support::{finish, read_code, read_hint, wyrm};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use wyrmyon_testkit::MailboxServer;

const EXE: &str = std::env::consts::EXE_SUFFIX;
const QR: char = '█';

fn named(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(format!("{name}{EXE}"));
    if std::fs::hard_link(env!("CARGO_BIN_EXE_wyrmyon"), &path).is_err() {
        std::fs::copy(env!("CARGO_BIN_EXE_wyrmyon"), &path).unwrap();
    }
    path
}

fn scratch() -> tempfile::TempDir {
    tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap()
}

fn wormhole_bin() -> &'static Path {
    static BIN: OnceLock<(tempfile::TempDir, PathBuf)> = OnceLock::new();
    &BIN.get_or_init(|| {
        let dir = scratch();
        let bin = named(dir.path(), "wormhole");
        (dir, bin)
    })
    .1
}

const SERVERS: [&str; 4] = [
    "WORMHOLE_RELAY_URL",
    "WORMHOLE_TRANSIT_HELPER",
    "WYRMYON_RELAY_URL",
    "WYRMYON_TRANSIT_HELPER",
];

fn wormhole(relay: &str) -> Command {
    let mut cmd = unaddressed();
    cmd.args(["--relay-url", relay, "--transit-helper", "tcp:127.0.0.1:9"]);
    cmd
}

fn unaddressed() -> Command {
    let mut cmd = Command::new(wormhole_bin());
    for name in SERVERS {
        cmd.env_remove(name);
    }
    cmd.env("WYRMYON_IROH_RELAYS", "disabled")
        .env("WYRMYON_CACHE_DIR", support::cache_dir())
        .env_remove("WYRMYON_ASCII")
        .env_remove("WORMHOLE_QR")
        .env_remove("WORMHOLE_ACCEPT_FILE")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    cmd
}

fn offline(args: &[&str]) -> (bool, String, String) {
    let out = std::process::Command::new(wormhole_bin())
        .args(args)
        .env("WORMHOLE_RELAY_URL", "ws://127.0.0.1:9/v1")
        .env_remove("WORMHOLE_TRANSIT_HELPER")
        .output()
        .unwrap();
    (
        out.status.code() == Some(0),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn the_name_wormhole_says_it_is_wyrmyon() {
    let (ok, stdout, _) = offline(&["--version"]);
    assert!(ok);
    assert_eq!(
        stdout,
        format!("wormhole {} (wyrmyon)\n", env!("CARGO_PKG_VERSION"))
    );
    let (ok, stdout, _) = offline(&["--help"]);
    assert!(ok && stdout.contains("[alias: tx]") && stdout.contains("[alias: rx]"));
}

#[test]
fn what_wyrmyon_cannot_do_fails_before_the_network() {
    for (args, said) in [
        (&["send", "-0", "--text", "x"][..], "-0 is not supported"),
        (&["rx", "--verify"], "--verify is not supported"),
        (
            &["--dump-timing", "t.json", "rx"],
            "--dump-timing is not supported",
        ),
        (
            &["send", "--ignore-unsendable-files", "x"],
            "--ignore-unsendable-files is not supported",
        ),
        (
            &["--appid", "example.com/x", "rx"],
            "only speaks the standard appid",
        ),
        (&["rx", "-c", "3"], "Must use --allocate with --code-length"),
    ] {
        let (ok, _, stderr) = offline(args);
        assert!(!ok, "{args:?}");
        assert!(stderr.contains(said), "{args:?}: {stderr}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn text_and_a_file_travel_between_two_wormholes() {
    let server = MailboxServer::start().await;
    let mut sender = wormhole(&server.url())
        .args(["tx", "--text", "hello there"])
        .spawn()
        .unwrap();
    let (code, hint) = read_hint(sender.stderr.take().unwrap()).await;
    assert!(hint.contains(QR), "{hint}");
    let (ok, stdout, stderr) =
        finish(wormhole(&server.url()).args(["rx", &code]).spawn().unwrap()).await;
    assert!(ok, "{stderr}");
    assert_eq!(stdout, "hello there\n");
    assert!(finish(sender).await.0);

    let (from, to) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    std::fs::write(from.path().join("notes.txt"), b"some notes").unwrap();
    let mut sender = wormhole(&server.url())
        .args(["send", "--no-qr", "notes.txt"])
        .current_dir(from.path())
        .spawn()
        .unwrap();
    let code = read_code(sender.stderr.take().unwrap()).await;
    let (ok, _, stderr) = finish(
        wormhole(&server.url())
            .args(["receive", "--accept-file", "-o", "kept.txt", &code])
            .current_dir(to.path())
            .spawn()
            .unwrap(),
    )
    .await;
    assert!(ok, "{stderr}");
    assert!(finish(sender).await.0);
    assert_eq!(
        std::fs::read(to.path().join("kept.txt")).unwrap(),
        b"some notes"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_qr_is_on_unless_turned_off() {
    let server = MailboxServer::start().await;
    for (flags, env, drawn) in [
        (&["--no-qr"][..], None, false),
        (&[], Some("0"), false),
        (&["--qr"], Some("0"), true),
    ] {
        let mut cmd = wormhole(&server.url());
        cmd.arg("send").args(flags).args(["--text", "unused"]);
        if let Some(env) = env {
            cmd.env("WORMHOLE_QR", env);
        }
        let mut sender = cmd.spawn().unwrap();
        let (_, hint) = read_hint(sender.stderr.take().unwrap()).await;
        assert_eq!(hint.contains(QR), drawn, "{flags:?} {env:?}: {hint}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn piped_stdin_is_the_text_and_wyrm_receives_it() {
    let server = MailboxServer::start().await;
    let mut sender = wormhole(&server.url())
        .arg("send")
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = sender.stdin.take().unwrap();
    stdin.write_all(b"from a pipe").await.unwrap();
    drop(stdin);
    let code = read_code(sender.stderr.take().unwrap()).await;
    let (ok, stdout, stderr) = finish(wyrm(&server.url()).arg(&code).spawn().unwrap()).await;
    assert!(ok, "{stderr}");
    assert_eq!(stdout, "from a pipe\n");
    assert!(finish(sender).await.0);
}

#[tokio::test(flavor = "multi_thread")]
async fn only_text_refuses_a_file() {
    let server = MailboxServer::start().await;
    let (from, to) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    std::fs::write(from.path().join("a.txt"), b"a file").unwrap();
    let mut sender = wyrm(&server.url())
        .args(["send", "a.txt"])
        .current_dir(from.path())
        .spawn()
        .unwrap();
    let code = read_code(sender.stderr.take().unwrap()).await;
    let (ok, _, stderr) = finish(
        wormhole(&server.url())
            .args(["receive", "--only-text", "--accept-file", &code])
            .current_dir(to.path())
            .spawn()
            .unwrap(),
    )
    .await;
    assert!(!ok);
    assert!(stderr.contains("text only"), "{stderr}");
    assert!(!finish(sender).await.0);
    assert_eq!(std::fs::read_dir(to.path()).unwrap().count(), 0);
}

fn shim(bin: &Path, home: &Path, path: &[&Path], args: &[&str]) -> (bool, String, String) {
    let out = std::process::Command::new(bin)
        .arg("shim")
        .args(args)
        .env("PATH", std::env::join_paths(path).unwrap())
        .env("HOME", home)
        .env("USERPROFILE", home)
        .output()
        .unwrap();
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn wyrm_shim_links_wormhole_beside_itself_and_takes_it_back() {
    let root = scratch();
    let bin = root.path().join("bin");
    let home = root.path().join("home");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::create_dir_all(&home).unwrap();
    let wyrmyon = named(&bin, "wyrmyon");
    let link = bin.join(format!("wormhole{EXE}"));

    let (ok, stdout, stderr) = shim(&wyrmyon, &home, &[&bin], &[]);
    assert!(ok, "{stderr}");
    assert!(
        stdout.starts_with(&format!("Linked {}", link.display())),
        "{stdout}"
    );
    let out = std::process::Command::new(&link)
        .arg("--version")
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&out.stdout).ends_with("(wyrmyon)\n"));

    let (ok, stdout, _) = shim(&wyrmyon, &home, &[&bin], &[]);
    assert!(ok);
    assert_eq!(stdout, format!("{} already runs wyrmyon\n", link.display()));

    let (ok, stdout, stderr) = shim(&wyrmyon, &home, &[&bin], &["--remove"]);
    assert!(ok, "{stderr}");
    assert_eq!(stdout, format!("Removed {}\n", link.display()));
    assert!(link.symlink_metadata().is_err());
}

#[test]
fn wyrm_shim_leaves_another_wormhole_alone() {
    let root = scratch();
    let python = root.path().join("python");
    let target = root.path().join("target");
    std::fs::create_dir_all(&python).unwrap();
    let theirs = python.join(format!("wormhole{EXE}"));
    std::fs::write(&theirs, b"#!/bin/sh\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&theirs, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let wyrm = Path::new(env!("CARGO_BIN_EXE_wyrm"));
    let dir = target.to_str().unwrap();

    let (ok, _, stderr) = shim(wyrm, root.path(), &[&python], &["--dir", dir]);
    assert!(!ok);
    assert!(stderr.contains(&theirs.display().to_string()), "{stderr}");
    assert!(!target.exists());

    let dir = python.to_str().unwrap();
    let (ok, _, stderr) = shim(wyrm, root.path(), &[], &["--remove", "--dir", dir]);
    assert!(!ok);
    assert!(stderr.contains("left in place"), "{stderr}");
    assert!(theirs.exists());
}

fn exit_of(args: &[&str], env: &[(&str, &str)]) -> (Option<i32>, String) {
    let out = std::process::Command::new(wormhole_bin())
        .args(args)
        .env("WORMHOLE_RELAY_URL", "ws://127.0.0.1:9/v1")
        .envs(env.iter().copied())
        .output()
        .unwrap();
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn usage_errors_exit_2_and_the_rest_1() {
    let (code, stderr) = exit_of(&["rx", "-c", "3"], &[]);
    assert_eq!(code, Some(2), "{stderr}");
    assert!(
        stderr.contains("Must use --allocate with --code-length"),
        "{stderr}"
    );
    let (code, stderr) = exit_of(&["rx", "7-a-b"], &[("WORMHOLE_TRANSIT_HELPER", "bogus")]);
    assert_eq!(code, Some(2), "{stderr}");
    let (code, stderr) = exit_of(&["ssh", "invite", "-c", "3"], &[]);
    assert_eq!(code, Some(1));
    assert!(
        stderr.contains("`wormhole ssh` is not supported by wyrmyon"),
        "{stderr}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_environment_names_the_servers_and_accepts_the_file() {
    let server = MailboxServer::start().await;
    let (from, to) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    std::fs::write(from.path().join("notes.txt"), b"by env").unwrap();
    let mut sender = unaddressed()
        .args(["send", "--no-qr", "notes.txt"])
        .env("WORMHOLE_RELAY_URL", server.url())
        .env("WORMHOLE_TRANSIT_HELPER", "tcp:127.0.0.1:9")
        .current_dir(from.path())
        .spawn()
        .unwrap();
    let (code, hint) = read_hint(sender.stderr.take().unwrap()).await;
    assert!(
        hint.contains(&format!("\n  wormhole receive {code}\n")),
        "{hint}"
    );
    assert!(hint.contains(&format!("(or: wyrm {code})")), "{hint}");
    let (ok, _, stderr) = finish(
        unaddressed()
            .args(["recv", &code])
            .env("WYRMYON_RELAY_URL", server.url())
            .env("WYRMYON_TRANSIT_HELPER", "tcp:127.0.0.1:9")
            .env("WORMHOLE_ACCEPT_FILE", "1")
            .current_dir(to.path())
            .spawn()
            .unwrap(),
    )
    .await;
    assert!(ok, "{stderr}");
    assert!(finish(sender).await.0);
    assert_eq!(
        std::fs::read(to.path().join("notes.txt")).unwrap(),
        b"by env"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_typed_line_is_sent_as_it_is() {
    let server = MailboxServer::start().await;
    let mut sender = wormhole(&server.url())
        .arg("send")
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = sender.stdin.take().unwrap();
    stdin.write_all(b"-\nnot this\n").await.unwrap();
    drop(stdin);
    let code = read_code(sender.stderr.take().unwrap()).await;
    let (ok, stdout, stderr) = finish(wyrm(&server.url()).arg(&code).spawn().unwrap()).await;
    assert!(ok, "{stderr}");
    assert_eq!(stdout, "-\n");
    let (ok, stdout, _) = finish(sender).await;
    assert!(ok);
    assert_eq!(stdout, "Text to send: ");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_receiver_first_hint_names_wormhole_first() {
    let server = MailboxServer::start().await;
    let mut receiver = wormhole(&server.url())
        .args(["receive", "--allocate"])
        .spawn()
        .unwrap();
    let (code, hint) = read_hint(receiver.stderr.take().unwrap()).await;
    assert!(
        hint.contains(&format!("\n  wormhole send --code {code} FILE\n")),
        "{hint}"
    );
    assert!(
        hint.contains(&format!("(or: wyrm send --code {code} FILE)")),
        "{hint}"
    );
}

async fn into(server: &MailboxServer, to: &Path, receiver: &mut Command) -> (bool, String) {
    let from = tempfile::tempdir().unwrap();
    std::fs::write(from.path().join("notes.txt"), b"new").unwrap();
    let mut sender = wyrm(&server.url())
        .args(["send", "notes.txt"])
        .current_dir(from.path())
        .spawn()
        .unwrap();
    let code = read_code(sender.stderr.take().unwrap()).await;
    let (ok, _, stderr) = finish(receiver.arg(&code).current_dir(to).spawn().unwrap()).await;
    finish(sender).await;
    (ok, stderr)
}

#[tokio::test(flavor = "multi_thread")]
async fn output_file_replaces_a_file_under_the_wormhole_name_only() {
    let server = MailboxServer::start().await;
    let to = tempfile::tempdir().unwrap();
    let kept = to.path().join("kept.txt");
    std::fs::write(&kept, b"old").unwrap();

    let mut refused = wyrm(&server.url());
    refused.args(["receive", "--accept-file", "-o", "kept.txt"]);
    let (ok, stderr) = into(&server, to.path(), &mut refused).await;
    assert!(!ok && stderr.contains("refusing to overwrite"), "{stderr}");
    assert_eq!(std::fs::read(&kept).unwrap(), b"old");

    let mut replaced = wormhole(&server.url());
    replaced.args(["receive", "--accept-file", "-o", "kept.txt"]);
    let (ok, stderr) = into(&server, to.path(), &mut replaced).await;
    assert!(ok, "{stderr}");
    assert!(stderr.contains(&format!("Overwriting {}", Path::new("kept.txt").display())));
    assert_eq!(std::fs::read(&kept).unwrap(), b"new");

    std::fs::create_dir_all(to.path().join("box").join("notes.txt")).unwrap();
    let mut boxed = wormhole(&server.url());
    boxed.args(["receive", "--accept-file", "-o", "box"]);
    let (ok, stderr) = into(&server, to.path(), &mut boxed).await;
    assert!(
        !ok && stderr.contains("not deleting the existing directory"),
        "{stderr}"
    );
    assert!(to.path().join("box").join("notes.txt").is_dir());

    let mut unnamed = wormhole(&server.url());
    unnamed.args(["receive", "--accept-file"]);
    let (ok, stderr) = into(&server, to.path().join("box").as_path(), &mut unnamed).await;
    assert!(!ok && stderr.contains("refusing to overwrite"), "{stderr}");
}

#[test]
fn wyrm_shim_beside_a_file_named_shim_refuses() {
    let root = scratch();
    std::fs::write(root.path().join("shim"), b"a file").unwrap();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_wyrm"))
        .args(["shim", "--dir", "elsewhere"])
        .current_dir(root.path())
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("`wyrm ./shim`"));
    assert!(!root.path().join("elsewhere").exists());
}
