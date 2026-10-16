mod support;

use std::process::Command;

use serde_json::json;
use support::{finish, read_code, read_code_keeping, read_hint, wyrm};
use wyrmyon_testkit::MailboxServer;
use wyrmyon_transport_classic::{Role, Transit};
use wyrmyon_wormhole::{Config, Mood};

fn version_of(bin: &str) -> String {
    let out = Command::new(bin).arg("--version").output().unwrap();
    assert!(out.status.success());
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn both_binaries_print_the_package_version() {
    let expected = format!("wyrmyon {}\n", env!("CARGO_PKG_VERSION"));
    assert_eq!(version_of(env!("CARGO_BIN_EXE_wyrmyon")), expected);
    assert_eq!(version_of(env!("CARGO_BIN_EXE_wyrm")), expected);
}

#[test]
fn a_malformed_code_is_refused_with_a_failing_exit_status() {
    let out = Command::new(env!("CARGO_BIN_EXE_wyrm"))
        .args(["receive", "not a code"])
        .env("WYRMYON_RELAY_URL", "ws://127.0.0.1:9/v1")
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("spaces"));
}

#[tokio::test(flavor = "multi_thread")]
async fn text_travels_between_two_wyrm_processes() {
    let server = MailboxServer::start().await;
    let mut sender = wyrm(&server.url())
        .args(["send", "--text", "hello there"])
        .spawn()
        .unwrap();
    let code = read_code(sender.stderr.take().unwrap()).await;

    let receiver = wyrm(&server.url())
        .args(["receive", &code])
        .spawn()
        .unwrap();
    let (ok, stdout, stderr) = finish(receiver).await;
    assert!(ok, "receiver failed: {stderr}");
    assert_eq!(stdout, "hello there\n");
    let (ok, _, _) = finish(sender).await;
    assert!(ok);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_hint_names_the_code_and_qr_draws_it() {
    let server = MailboxServer::start().await;
    let mut sender = wyrm(&server.url())
        .args(["send", "--text", "scan me", "--qr", "--ascii"])
        .spawn()
        .unwrap();
    let (code, hint) = read_hint(sender.stderr.take().unwrap()).await;
    assert!(hint.contains(&format!("\n  wyrm {code}\n")), "{hint}");
    assert!(
        hint.contains(&format!("(or: wormhole receive {code})")),
        "{hint}"
    );
    assert!(hint.contains("##############"), "{hint}");
    assert!(hint.is_ascii(), "{hint}");

    let mut drawn = wyrm(&server.url())
        .args(["send", "--text", "unused", "--qr"])
        .env("WYRMYON_ASCII", "0")
        .spawn()
        .unwrap();
    let (_, hint) = read_hint(drawn.stderr.take().unwrap()).await;
    assert!(hint.contains('█') && !hint.contains("##"), "{hint}");
    assert!(!hint.contains('\x1b'), "{hint}");
    drop(drawn);

    let mut receiver = wyrm(&server.url())
        .args(["receive", "--new"])
        .spawn()
        .unwrap();
    let (allocated, hint) = read_hint(receiver.stderr.take().unwrap()).await;
    assert!(
        hint.contains(&format!("wyrm send --code {allocated} FILE")),
        "{hint}"
    );
    drop(receiver);

    let receiver = wyrm(&server.url())
        .args(["receive", &code])
        .spawn()
        .unwrap();
    let (ok, stdout, stderr) = finish(receiver).await;
    assert!(ok, "{stderr}");
    assert_eq!(stdout, "scan me\n");
    assert!(finish(sender).await.0);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_refused_offer_fails_the_sender() {
    let server = MailboxServer::start().await;
    let mut sender = wyrm(&server.url())
        .args(["send", "--text", "unwanted"])
        .spawn()
        .unwrap();
    let code = read_code(sender.stderr.take().unwrap())
        .await
        .parse()
        .unwrap();
    let config = Config {
        relay_url: server.url(),
        ..Config::default()
    };
    let mut receiver = wyrmyon_wormhole::join(&config, code)
        .await
        .unwrap()
        .pair()
        .await
        .unwrap();
    let offer = receiver.receive_json().await.unwrap();
    assert_eq!(offer["offer"]["message"], "unwanted");
    receiver
        .send_json(&json!({"error": "transfer rejected"}))
        .await
        .unwrap();
    receiver.close(Mood::Happy).await;
    let (ok, _, _) = finish(sender).await;
    assert!(!ok);
    assert_eq!(server.moods(), ["happy", "happy"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn files_travel_between_two_wyrm_processes() {
    let server = MailboxServer::start().await;
    for size in [0u32, 300_000] {
        let (from, to) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let data: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
        std::fs::write(from.path().join("notes.txt"), &data).unwrap();
        let mut sender = wyrm(&server.url())
            .args(["send", "notes.txt"])
            .current_dir(from.path())
            .spawn()
            .unwrap();
        let code = read_code(sender.stderr.take().unwrap()).await;
        let (ok, _, stderr) = finish(
            wyrm(&server.url())
                .args(["receive", "--accept-file", &code])
                .current_dir(to.path())
                .spawn()
                .unwrap(),
        )
        .await;
        assert!(ok, "{stderr}");
        assert!(finish(sender).await.0);
        assert_eq!(std::fs::read(to.path().join("notes.txt")).unwrap(), data);
        assert_eq!(std::fs::read_dir(to.path()).unwrap().count(), 1);
    }
}

#[cfg(unix)]
#[test]
fn only_regular_files_are_offered() {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_wyrm"))
        .args(["send", "/dev/null"])
        .env("WYRMYON_RELAY_URL", "ws://127.0.0.1:9/v1")
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("not a regular file"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_sender_that_sends_more_than_it_offered_leaves_nothing_behind() {
    let server = MailboxServer::start().await;
    let config = Config {
        relay_url: server.url(),
        ..Config::default()
    };
    let to = tempfile::tempdir().unwrap();
    let pending = wyrmyon_wormhole::create(&config, 2).await.unwrap();
    let receiver = wyrm(&server.url())
        .args(["receive", "--accept-file", pending.code().as_str()])
        .current_dir(to.path())
        .spawn()
        .unwrap();
    let mut wormhole = pending.pair().await.unwrap();
    let transit = Transit::new(Role::Sender, wormhole.transit_key()).await;
    let ours = serde_json::to_value(transit.info()).unwrap();
    wormhole.send_json(&json!({"transit": ours})).await.unwrap();
    wormhole
        .send_json(&json!({"offer": {"file": {"filename": "x.bin", "filesize": 4}}}))
        .await
        .unwrap();
    let mut theirs = None;
    loop {
        let msg = wormhole.receive_json().await.unwrap();
        if let Some(transit) = msg.get("transit") {
            theirs = Some(transit.clone());
        }
        if msg.get("answer").is_some() {
            break;
        }
    }
    let theirs = serde_json::from_value(theirs.unwrap()).unwrap();
    let mut pipe = transit.connect(&theirs).await.unwrap();
    pipe.send_record(b"12345").await.unwrap();
    pipe.flush().await.unwrap();
    let (ok, _, stderr) = finish(receiver).await;
    assert!(!ok);
    assert!(stderr.contains("more than"), "{stderr}");
    assert_eq!(std::fs::read_dir(to.path()).unwrap().count(), 0);
    wormhole.close(Mood::Happy).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_file_is_refused_without_confirmation_or_over_an_existing_one() {
    let server = MailboxServer::start().await;
    let (from, to) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    std::fs::write(from.path().join("a.txt"), b"new").unwrap();

    for (args, kept) in [(vec![], None), (vec!["--accept-file"], Some(&b"old"[..]))] {
        if let Some(old) = kept {
            std::fs::write(to.path().join("a.txt"), old).unwrap();
        }
        let mut sender = wyrm(&server.url())
            .args(["send", "a.txt"])
            .current_dir(from.path())
            .spawn()
            .unwrap();
        let code = read_code(sender.stderr.take().unwrap()).await;
        let (ok, _, _) = finish(
            wyrm(&server.url())
                .arg("receive")
                .args(&args)
                .arg(&code)
                .current_dir(to.path())
                .spawn()
                .unwrap(),
        )
        .await;
        assert!(!ok);
        assert!(!finish(sender).await.0);
        match kept {
            None => assert!(!to.path().join("a.txt").exists()),
            Some(old) => assert_eq!(std::fs::read(to.path().join("a.txt")).unwrap(), old),
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_directory_travels_through_the_relay_only() {
    let server = MailboxServer::start().await;
    let relay = wyrmyon_testkit::TransitRelay::start().await;
    let (from, to) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let tree = from.path().join("tree");
    std::fs::create_dir_all(tree.join("sub/empty")).unwrap();
    std::fs::write(tree.join("a.txt"), b"alpha").unwrap();
    std::fs::write(tree.join("sub/b.bin"), vec![3u8; 70_000]).unwrap();
    let relayed = |cmd: &mut tokio::process::Command| {
        cmd.args([
            "--force-classic",
            "--no-listen",
            "--transit-helper",
            &relay.hint(),
        ]);
    };

    let mut send = wyrm(&server.url());
    relayed(&mut send);
    let mut sender = send
        .args(["send", "tree"])
        .current_dir(from.path())
        .spawn()
        .unwrap();
    let code = read_code(sender.stderr.take().unwrap()).await;
    let mut receive = wyrm(&server.url());
    relayed(&mut receive);
    let (ok, _, stderr) = finish(
        receive
            .args(["receive", "--accept-file", &code])
            .current_dir(to.path())
            .spawn()
            .unwrap(),
    )
    .await;
    assert!(ok, "{stderr}");
    assert!(stderr.contains("via relay"), "{stderr}");
    assert!(finish(sender).await.0);
    let got = to.path().join("tree");
    assert_eq!(std::fs::read(got.join("a.txt")).unwrap(), b"alpha");
    assert_eq!(
        std::fs::read(got.join("sub/b.bin")).unwrap(),
        vec![3u8; 70_000]
    );
    assert!(got.join("sub/empty").is_dir());
    assert_eq!(std::fs::read_dir(to.path()).unwrap().count(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_receiver_can_allocate_the_code() {
    let server = MailboxServer::start().await;
    let (from, to) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    std::fs::write(from.path().join("r.txt"), b"receiver first").unwrap();
    let mut receiver = wyrm(&server.url())
        .args(["receive", "--new", "--accept-file"])
        .current_dir(to.path())
        .spawn()
        .unwrap();
    let code = read_code(receiver.stderr.take().unwrap()).await;
    let (ok, _, stderr) = finish(
        wyrm(&server.url())
            .args(["send", "--code", &code, "r.txt"])
            .current_dir(from.path())
            .spawn()
            .unwrap(),
    )
    .await;
    assert!(ok, "{stderr}");
    assert!(finish(receiver).await.0);
    assert_eq!(
        std::fs::read(to.path().join("r.txt")).unwrap(),
        b"receiver first"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_sender_given_the_code_still_shows_it_and_the_hint() {
    let server = MailboxServer::start().await;
    let mut receiver = wyrm(&server.url())
        .args(["receive", "--new"])
        .spawn()
        .unwrap();
    let code = read_code(receiver.stderr.take().unwrap()).await;
    let mut sender = wyrm(&server.url())
        .args(["send", "--code", &code, "--text", "chosen"])
        .spawn()
        .unwrap();
    let (shown, hint) = read_hint(sender.stderr.take().unwrap()).await;
    assert_eq!(shown, code);
    assert!(hint.contains(&format!("\n  wyrm {code}\n")), "{hint}");
    let (ok, stdout, stderr) = finish(receiver).await;
    assert!(ok, "{stderr}");
    assert_eq!(stdout, "chosen\n");
    assert!(finish(sender).await.0);
}

#[test]
fn update_is_refused_beside_anything_to_send_or_receive() {
    for args in [
        vec!["--update", "send", "--text", "x"],
        vec!["--update", "receive"],
        vec!["--update", "7-a-b"],
        vec!["--update", "notes.txt"],
    ] {
        let out = Command::new(env!("CARGO_BIN_EXE_wyrm"))
            .args(&args)
            .env("WYRMYON_RELAY_URL", "ws://127.0.0.1:9/v1")
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(2), "{args:?} was accepted");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stderr.contains("--update"), "{args:?}: {stderr}");
        assert!(stderr.contains("Usage: wyrm"), "{args:?}: {stderr}");
    }
}

#[test]
fn an_update_that_cannot_reach_github_fails_and_says_so() {
    let cache = tempfile::tempdir().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_wyrm"))
        .args(["--update", "--hide-progress"])
        .env("HTTPS_PROXY", "http://127.0.0.1:9")
        .env("ALL_PROXY", "http://127.0.0.1:9")
        .env("NO_PROXY", "")
        .env("XDG_CACHE_HOME", cache.path())
        .env("LOCALAPPDATA", cache.path())
        .env_remove("WYRMYON_REPO")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("could not reach GitHub"), "{stderr}");
    assert!(!cache.path().join("wyrmyon").exists());
}

#[test]
fn conflicting_code_flags_are_refused() {
    for args in [
        vec!["receive", "--new", "7-a-b"],
        vec![
            "send",
            "--code",
            "7-a-b",
            "--code-length",
            "3",
            "--text",
            "x",
        ],
    ] {
        let out = Command::new(env!("CARGO_BIN_EXE_wyrm"))
            .args(&args)
            .env("WYRMYON_RELAY_URL", "ws://127.0.0.1:9/v1")
            .output()
            .unwrap();
        assert!(!out.status.success(), "{args:?} was accepted");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_existing_output_directory_receives_the_file_inside() {
    let server = MailboxServer::start().await;
    let (from, to) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    std::fs::write(from.path().join("in.txt"), b"inside").unwrap();
    let mut sender = wyrm(&server.url())
        .args(["send", "in.txt"])
        .current_dir(from.path())
        .spawn()
        .unwrap();
    let code = read_code(sender.stderr.take().unwrap()).await;
    let out = to.path().to_str().unwrap().to_owned();
    let (ok, _, stderr) = finish(
        wyrm(&server.url())
            .args(["receive", "--accept-file", "-o", &out, &code])
            .spawn()
            .unwrap(),
    )
    .await;
    assert!(ok, "{stderr}");
    assert!(finish(sender).await.0);
    assert_eq!(std::fs::read(to.path().join("in.txt")).unwrap(), b"inside");
}

struct Outcome {
    ok: bool,
    sender: String,
    receiver: String,
}

async fn transfer_with(
    server: &MailboxServer,
    send_flags: &[&str],
    receive_flags: &[&str],
    directory: bool,
) -> Outcome {
    let (from, to) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let what = if directory {
        std::fs::create_dir_all(from.path().join("t/sub")).unwrap();
        std::fs::write(from.path().join("t/sub/x"), vec![5u8; 100_000]).unwrap();
        "t"
    } else {
        std::fs::write(from.path().join("t"), vec![5u8; 100_000]).unwrap();
        "t"
    };
    let mut sender = wyrm(&server.url())
        .args(send_flags)
        .args(["send", what])
        .current_dir(from.path())
        .spawn()
        .unwrap();
    let (code, collected) = read_code_keeping(sender.stderr.take().unwrap()).await;
    let (receive_ok, _, receiver_log) = finish(
        wyrm(&server.url())
            .args(receive_flags)
            .args(["receive", "--accept-file", &code])
            .current_dir(to.path())
            .spawn()
            .unwrap(),
    )
    .await;
    let (sent, _, _) = finish(sender).await;
    let path = if directory { "t/sub/x" } else { "t" };
    let same = std::fs::read(to.path().join(path)).is_ok_and(|d| d == vec![5u8; 100_000]);
    Outcome {
        ok: receive_ok && sent && same,
        sender: collected.await.unwrap(),
        receiver: receiver_log,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn two_wyrms_pick_iroh_unless_either_forces_classic() {
    let server = MailboxServer::start().await;
    for directory in [false, true] {
        let outcome = transfer_with(&server, &[], &[], directory).await;
        assert!(outcome.ok, "{}", outcome.receiver);
        assert!(
            outcome.receiver.contains("Receiving (iroh"),
            "{}",
            outcome.receiver
        );
        assert!(
            outcome.receiver.contains("Went direct"),
            "{}",
            outcome.receiver
        );
        assert!(outcome.sender.contains("Went direct"), "{}", outcome.sender);
        assert!(!outcome.sender.contains("zipfile"), "{}", outcome.sender);
    }
    for directory in [false, true] {
        for (send_flags, receive_flags) in [
            (&["--force-classic"][..], &[][..]),
            (&[][..], &["--force-classic"][..]),
        ] {
            let outcome = transfer_with(&server, send_flags, receive_flags, directory).await;
            assert!(outcome.ok, "{}", outcome.receiver);
            assert!(
                outcome.receiver.contains("Receiving (directly"),
                "{}",
                outcome.receiver
            );
            assert!(!outcome.receiver.contains("Went"), "{}", outcome.receiver);
            assert_eq!(
                outcome.sender.contains("Building zipfile"),
                directory,
                "{}",
                outcome.sender
            );
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn two_wyrms_can_share_a_relay_of_their_own() {
    let server = MailboxServer::start().await;
    let relay = wyrmyon_testkit::IrohRelay::start().await;
    let flags = ["--iroh-relays", &relay.url()];
    let outcome = transfer_with(&server, &flags, &flags, false).await;
    assert!(outcome.ok, "{}", outcome.receiver);
    assert!(
        outcome.receiver.contains("Receiving (iroh"),
        "{}",
        outcome.receiver
    );
}

#[test]
fn an_iroh_relay_must_be_a_name_or_a_url() {
    let out = Command::new(env!("CARGO_BIN_EXE_wyrm"))
        .args(["--iroh-relays", "sometimes", "receive", "7-a-b"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("relay URL"));
}

#[tokio::test(flavor = "multi_thread")]
async fn empty_directories_arrive_in_a_collection() {
    let server = MailboxServer::start().await;
    let (from, to) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    std::fs::create_dir_all(from.path().join("t/void")).unwrap();
    std::fs::write(from.path().join("t/x"), b"x").unwrap();
    std::fs::create_dir(from.path().join("e")).unwrap();
    for what in ["t", "e"] {
        let mut sender = wyrm(&server.url())
            .args(["send", what])
            .current_dir(from.path())
            .spawn()
            .unwrap();
        let code = read_code(sender.stderr.take().unwrap()).await;
        let (ok, _, stderr) = finish(
            wyrm(&server.url())
                .args(["receive", "--accept-file", &code])
                .current_dir(to.path())
                .spawn()
                .unwrap(),
        )
        .await;
        assert!(ok, "{stderr}");
        assert!(stderr.contains("Receiving (iroh"), "{stderr}");
        assert!(finish(sender).await.0);
    }
    assert!(to.path().join("t/void").is_dir());
    assert_eq!(std::fs::read(to.path().join("t/x")).unwrap(), b"x");
    assert!(to.path().join("e").is_dir());
}

#[tokio::test(flavor = "multi_thread")]
async fn force_iroh_refuses_a_legacy_peer_on_either_side() {
    let server = MailboxServer::start().await;
    let outcome = transfer_with(&server, &["--force-iroh"], &["--force-classic"], false).await;
    assert!(!outcome.ok);
    assert!(
        outcome.sender.contains("does not speak iroh-v1"),
        "{}",
        outcome.sender
    );
    assert!(
        outcome.receiver.contains("requires iroh-v1"),
        "{}",
        outcome.receiver
    );

    let outcome = transfer_with(&server, &["--force-iroh"], &["--force-classic"], true).await;
    assert!(!outcome.ok);
    assert!(
        outcome.sender.contains("does not speak iroh-v1")
            && !outcome.sender.contains("Building zipfile"),
        "{}",
        outcome.sender
    );

    let outcome = transfer_with(&server, &["--force-classic"], &["--force-iroh"], false).await;
    assert!(!outcome.ok);
    assert!(
        outcome.receiver.contains("does not speak iroh-v1"),
        "{}",
        outcome.receiver
    );
    assert!(
        outcome.sender.contains("requires iroh-v1"),
        "{}",
        outcome.sender
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_iroh_transfer_leaves_nothing_in_the_cache() {
    let server = MailboxServer::start().await;
    let (from, to, cache) = (
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
    );
    std::fs::write(from.path().join("c.bin"), vec![4u8; 2_000_000]).unwrap();
    let mut sender = wyrm(&server.url())
        .args(["send", "c.bin"])
        .current_dir(from.path())
        .spawn()
        .unwrap();
    let code = read_code(sender.stderr.take().unwrap()).await;
    let (ok, _, stderr) = finish(
        wyrm(&server.url())
            .args(["receive", "--accept-file", &code])
            .env("WYRMYON_CACHE_DIR", cache.path())
            .current_dir(to.path())
            .spawn()
            .unwrap(),
    )
    .await;
    assert!(ok, "{stderr}");
    assert!(stderr.contains("Receiving (iroh"), "{stderr}");
    assert!(finish(sender).await.0);
    assert_eq!(
        std::fs::read(to.path().join("c.bin")).unwrap(),
        vec![4u8; 2_000_000]
    );
    assert_eq!(std::fs::read_dir(cache.path()).unwrap().count(), 0);
    assert_eq!(std::fs::read_dir(to.path()).unwrap().count(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn bare_arguments_pick_the_action() {
    let server = MailboxServer::start().await;
    let (from, to) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    std::fs::write(from.path().join("bare.txt"), b"no subcommand").unwrap();
    std::fs::create_dir(to.path().join("in")).unwrap();
    let mut sender = wyrm(&server.url())
        .arg("bare.txt")
        .current_dir(from.path())
        .spawn()
        .unwrap();
    let code = read_code(sender.stderr.take().unwrap()).await;
    let (ok, _, stderr) = finish(
        wyrm(&server.url())
            .args(["--hide-progress", &code])
            .current_dir(to.path())
            .stdin(std::process::Stdio::null())
            .spawn()
            .unwrap(),
    )
    .await;
    assert!(!ok, "a bare receive still asks before accepting: {stderr}");
    assert!(stderr.contains("--accept-file"), "{stderr}");
    let _ = finish(sender).await;

    let mut sender = wyrm(&server.url())
        .stdin(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = sender.stdin.take().unwrap();
    tokio::io::AsyncWriteExt::write_all(&mut stdin, b"piped, bare")
        .await
        .unwrap();
    drop(stdin);
    let code = read_code(sender.stderr.take().unwrap()).await;
    let (ok, stdout, stderr) = finish(wyrm(&server.url()).arg(&code).spawn().unwrap()).await;
    assert!(ok, "{stderr}");
    assert_eq!(stdout, "piped, bare\n");
    assert!(finish(sender).await.0);
}

#[test]
fn a_target_that_is_both_a_code_and_a_file_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("7-guitarist-revenge"), b"x").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_wyrm"))
        .arg("7-guitarist-revenge")
        .env("WYRMYON_RELAY_URL", "ws://127.0.0.1:9/v1")
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("both a code and a file"));
}

#[tokio::test(flavor = "multi_thread")]
async fn several_paths_travel_as_one_bundle() {
    let server = MailboxServer::start().await;
    let (from, to) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    std::fs::write(from.path().join("a.txt"), b"alpha").unwrap();
    std::fs::create_dir_all(from.path().join("photos/2026")).unwrap();
    std::fs::write(from.path().join("photos/2026/p.raw"), vec![1u8; 50_000]).unwrap();
    let mut sender = wyrm(&server.url())
        .args(["a.txt", "photos"])
        .current_dir(from.path())
        .spawn()
        .unwrap();
    let code = read_code(sender.stderr.take().unwrap()).await;
    let (ok, _, stderr) = finish(
        wyrm(&server.url())
            .args(["receive", "--accept-file", &code])
            .current_dir(to.path())
            .spawn()
            .unwrap(),
    )
    .await;
    assert!(ok, "{stderr}");
    assert!(finish(sender).await.0);
    let files = to.path().join("files");
    assert_eq!(std::fs::read(files.join("a.txt")).unwrap(), b"alpha");
    assert_eq!(
        std::fs::read(files.join("photos/2026/p.raw")).unwrap(),
        vec![1u8; 50_000]
    );

    std::fs::create_dir(from.path().join("again")).unwrap();
    std::fs::write(from.path().join("again/a.txt"), b"twin").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_wyrm"))
        .args(["send", "a.txt", "again/a.txt"])
        .env("WYRMYON_RELAY_URL", "ws://127.0.0.1:9/v1")
        .current_dir(from.path())
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("two of the paths"));
}

#[tokio::test(flavor = "multi_thread")]
async fn without_a_terminal_the_code_is_read_from_stdin() {
    let server = MailboxServer::start().await;
    let mut sender = wyrm(&server.url())
        .args(["send", "--text", "via stdin"])
        .spawn()
        .unwrap();
    let code = read_code(sender.stderr.take().unwrap()).await;
    let mut receiver = wyrm(&server.url())
        .arg("receive")
        .stdin(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = receiver.stdin.take().unwrap();
    tokio::io::AsyncWriteExt::write_all(&mut stdin, format!("{code}\n").as_bytes())
        .await
        .unwrap();
    drop(stdin);
    let (ok, stdout, stderr) = finish(receiver).await;
    assert!(ok, "{stderr}");
    assert_eq!(stdout, "via stdin\n");
    assert!(finish(sender).await.0);
}

#[tokio::test(flavor = "multi_thread")]
async fn netcheck_reports_this_network() {
    let relay = wyrmyon_testkit::IrohRelay::start().await;
    let (ok, out, _) = finish(
        wyrm("ws://127.0.0.1:9/v1")
            .args(["--iroh-relays", &relay.url(), "netcheck"])
            .spawn()
            .unwrap(),
    )
    .await;
    assert!(ok, "{out}");
    assert!(out.starts_with("Local addresses "), "{out}");
    assert!(!out.starts_with("Local addresses none"), "{out}");
    assert!(out.contains("No UDP answer"), "{out}");
    assert!(
        out.contains(&format!("Relays          {}/", relay.url())),
        "{out}"
    );

    let (ok, out, _) = finish(wyrm("ws://127.0.0.1:9/v1").arg("netcheck").spawn().unwrap()).await;
    assert!(ok, "{out}");
    assert!(out.contains("Relays          disabled"), "{out}");
    assert!(out.contains("Without relays"), "{out}");
}

#[tokio::test(flavor = "multi_thread")]
async fn one_code_serves_several_receivers_in_turn() {
    let server = MailboxServer::start().await;
    let from = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(from.path().join("t/sub")).unwrap();
    std::fs::write(from.path().join("t/sub/x"), vec![5u8; 100_000]).unwrap();
    let mut sender = wyrm(&server.url())
        .args(["send", "--times", "3", "t"])
        .current_dir(from.path())
        .spawn()
        .unwrap();
    let (code, collected) = read_code_keeping(sender.stderr.take().unwrap()).await;
    let nameplate: u32 = code.split('-').next().unwrap().parse().unwrap();
    assert!(nameplate >= 1_000_000, "{code}");
    for flags in [&[][..], &["--force-classic"], &["--force-classic"]] {
        let to = tempfile::tempdir().unwrap();
        let (ok, _, stderr) = finish(
            wyrm(&server.url())
                .args(flags)
                .args(["receive", "--accept-file", &code])
                .current_dir(to.path())
                .spawn()
                .unwrap(),
        )
        .await;
        assert!(ok, "{stderr}");
        assert_eq!(
            std::fs::read(to.path().join("t/sub/x")).unwrap(),
            vec![5u8; 100_000]
        );
    }
    let (ok, _, _) = finish(sender).await;
    let said = collected.await.unwrap();
    assert!(ok, "{said}");
    assert!(said.contains("serves 3 receivers"), "{said}");
    assert!(said.contains("Waiting for receiver 3 of 3"), "{said}");
    assert!(said.contains("Receiver 3 of 3: done"), "{said}");
    assert_eq!(said.matches("Hashing..").count(), 1, "{said}");
    assert_eq!(said.matches("Building zipfile..").count(), 1, "{said}");
    assert_eq!(server.claimed_nameplates(), Vec::<String>::new());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_transfer_uses_up_its_turn_and_a_wrong_code_ends_the_run() {
    let server = MailboxServer::start().await;
    let config = Config {
        relay_url: server.url(),
        ..Config::default()
    };
    let mut sender = wyrm(&server.url())
        .args(["send", "--times", "2", "--text", "hi"])
        .spawn()
        .unwrap();
    let (code, collected) = read_code_keeping(sender.stderr.take().unwrap()).await;
    let mut refusing = wyrmyon_wormhole::join(&config, code.parse().unwrap())
        .await
        .unwrap()
        .pair()
        .await
        .unwrap();
    refusing.receive_json().await.unwrap();
    refusing
        .send_json(&json!({"error": "transfer rejected"}))
        .await
        .unwrap();
    refusing.close(Mood::Happy).await;
    let (ok, stdout, stderr) = finish(
        wyrm(&server.url())
            .args(["receive", &code])
            .spawn()
            .unwrap(),
    )
    .await;
    assert!(ok, "{stderr}");
    assert_eq!(stdout, "hi\n");
    let (ok, _, _) = finish(sender).await;
    let said = collected.await.unwrap();
    assert!(!ok, "{said}");
    assert!(said.contains("Receiver 1 of 2: "), "{said}");
    assert!(said.contains("transfer rejected"), "{said}");
    assert!(said.contains("Receiver 2 of 2: done"), "{said}");
    assert!(said.contains("1 of 2 transfers failed"), "{said}");

    let mut sender = wyrm(&server.url())
        .args([
            "send",
            "--times",
            "2",
            "--code",
            "1234567-guitarist-revenge",
            "--text",
            "hi",
        ])
        .spawn()
        .unwrap();
    let (code, collected) = read_code_keeping(sender.stderr.take().unwrap()).await;
    assert_eq!(code, "1234567-guitarist-revenge");
    let (ok, _, stderr) = finish(
        wyrm(&server.url())
            .args(["receive", "1234567-wrong-guess"])
            .spawn()
            .unwrap(),
    )
    .await;
    assert!(!ok, "{stderr}");
    let (ok, _, _) = finish(sender).await;
    let said = collected.await.unwrap();
    assert!(!ok, "{said}");
    assert!(
        said.contains("receiver 1 of 2: the key exchange failed"),
        "{said}"
    );
    assert!(!said.contains("Waiting for receiver 2"), "{said}");
    assert_eq!(server.claimed_nameplates(), Vec::<String>::new());
}

#[test]
fn several_receivers_need_a_code_the_server_never_hands_out() {
    let out = Command::new(env!("CARGO_BIN_EXE_wyrm"))
        .args([
            "send",
            "--times",
            "2",
            "--code",
            "7-guitarist-revenge",
            "--text",
            "x",
        ])
        .env("WYRMYON_RELAY_URL", "ws://127.0.0.1:9/v1")
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("7 digits"));
}
