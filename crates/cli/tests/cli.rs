mod support;

use std::process::Command;

use serde_json::json;
use support::{finish, read_code, wyrm};
use wyrmyon_testkit::MailboxServer;
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
async fn a_file_travels_between_two_wyrm_processes() {
    let server = MailboxServer::start().await;
    let (from, to) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let data: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
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
    let leftovers: Vec<_> = std::fs::read_dir(to.path()).unwrap().collect();
    assert_eq!(leftovers.len(), 1);
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
