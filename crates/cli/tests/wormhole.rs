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

fn wormhole(relay: &str) -> Command {
    let mut cmd = Command::new(wormhole_bin());
    cmd.args(["--relay-url", relay, "--transit-helper", "tcp:127.0.0.1:9"])
        .env("WYRMYON_IROH_RELAYS", "disabled")
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
        .output()
        .unwrap();
    (
        out.status.success(),
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
