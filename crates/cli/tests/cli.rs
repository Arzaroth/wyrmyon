mod support;

use std::process::Command;

use serde_json::json;
use support::{finish, read_code, wyrm};
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

#[test]
fn conflicting_code_flags_are_refused() {
    for args in [
        vec!["receive", "--new", "7-a-b"],
        vec!["receive", "--code-length", "3", "7-a-b"],
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

async fn transfer_with(
    server: &MailboxServer,
    send_flags: &[&str],
    receive_flags: &[&str],
) -> (bool, String) {
    let (from, to) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    std::fs::write(from.path().join("t.bin"), vec![5u8; 100_000]).unwrap();
    let mut sender = wyrm(&server.url())
        .args(send_flags)
        .args(["send", "t.bin"])
        .current_dir(from.path())
        .spawn()
        .unwrap();
    let code = read_code(sender.stderr.take().unwrap()).await;
    let (ok, _, stderr) = finish(
        wyrm(&server.url())
            .args(receive_flags)
            .args(["receive", "--accept-file", &code])
            .current_dir(to.path())
            .spawn()
            .unwrap(),
    )
    .await;
    let sent = finish(sender).await.0;
    let same = std::fs::read(to.path().join("t.bin")).is_ok_and(|d| d == vec![5u8; 100_000]);
    (ok && sent && same, stderr)
}

#[tokio::test(flavor = "multi_thread")]
async fn two_wyrms_pick_iroh_unless_either_forces_classic() {
    let server = MailboxServer::start().await;
    let (ok, stderr) = transfer_with(&server, &[], &[]).await;
    assert!(ok, "{stderr}");
    assert!(stderr.contains("Receiving (iroh"), "{stderr}");
    for (send_flags, receive_flags) in [
        (&["--force-classic"][..], &[][..]),
        (&[][..], &["--force-classic"][..]),
    ] {
        let (ok, stderr) = transfer_with(&server, send_flags, receive_flags).await;
        assert!(ok, "{stderr}");
        assert!(stderr.contains("Receiving (directly"), "{stderr}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn force_iroh_refuses_a_legacy_peer() {
    let server = MailboxServer::start().await;
    let (ok, stderr) = transfer_with(&server, &["--force-iroh"], &["--force-classic"]).await;
    assert!(!ok, "{stderr}");
}
