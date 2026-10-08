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
