mod support;

use std::io::Write as _;
use std::path::Path;
use std::process::Stdio;

use serde_json::{Value, json};
use support::{TIMEOUT, finish, read_code, wyrm};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Child;
use wyrmyon_testkit::MailboxServer;
use wyrmyon_transport_classic::{Role, Transit, TransitInfo};
use wyrmyon_transport_iroh::{IrohInfo, IrohTransport, Relays, Role as IrohRole};
use wyrmyon_wormhole::{Config, Mood, Wormhole};

fn config(server: &MailboxServer, iroh: bool) -> Config {
    let app_versions = if iroh {
        json!({"wyrmyon": {"transports": ["iroh-v1"]}})
    } else {
        json!({})
    };
    Config {
        relay_url: server.url(),
        app_versions,
        ..Config::default()
    }
}

async fn wyrm_receives(
    server: &MailboxServer,
    args: &[&str],
    dir: &Path,
    iroh: bool,
) -> (Wormhole, Child) {
    let pending = wyrmyon_wormhole::create(&config(server, iroh), 2)
        .await
        .unwrap();
    let child = wyrm(&server.url())
        .arg("receive")
        .args(args)
        .arg(pending.code().as_str())
        .current_dir(dir)
        .spawn()
        .unwrap();
    (pending.pair().await.unwrap(), child)
}

async fn wyrm_sends(
    server: &MailboxServer,
    args: &[&str],
    dir: &Path,
    iroh: bool,
) -> (Wormhole, Child) {
    let mut child = wyrm(&server.url())
        .arg("send")
        .args(args)
        .current_dir(dir)
        .spawn()
        .unwrap();
    let code = read_code(child.stderr.take().unwrap())
        .await
        .parse()
        .unwrap();
    let wormhole = wyrmyon_wormhole::join(&config(server, iroh), code)
        .await
        .unwrap()
        .pair()
        .await
        .unwrap();
    (wormhole, child)
}

async fn until(wormhole: &mut Wormhole, key: &str) -> Value {
    loop {
        let msg = wormhole.receive_json().await.unwrap();
        if let Some(value) = msg.get(key) {
            return value.clone();
        }
    }
}

async fn fails(child: Child) -> String {
    let (ok, _, stderr) = finish(child).await;
    assert!(!ok, "wyrm succeeded: {stderr}");
    stderr
}

#[tokio::test(flavor = "multi_thread")]
async fn the_receiver_skips_what_it_does_not_know_and_still_takes_text() {
    let server = MailboxServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let (mut sender, receiver) =
        wyrm_receives(&server, &["--hide-progress"], dir.path(), false).await;
    let transit = Transit::new(Role::Sender, sender.transit_key()).await;
    sender.send_json(&json!({"chat": "hello?"})).await.unwrap();
    sender
        .send_json(&json!({"answer": {"file_ack": "ok"}}))
        .await
        .unwrap();
    sender
        .send_json(&json!({"transit": serde_json::to_value(transit.info()).unwrap()}))
        .await
        .unwrap();
    sender
        .send_json(&json!({"offer": {"message": "still here"}}))
        .await
        .unwrap();
    assert_eq!(
        until(&mut sender, "answer").await,
        json!({"message_ack": "ok"})
    );
    sender.close(Mood::Happy).await;
    let (ok, stdout, stderr) = finish(receiver).await;
    assert!(ok, "{stderr}");
    assert_eq!(stdout, "still here\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_receiver_refuses_bad_offers_and_says_why() {
    let server = MailboxServer::start().await;
    let cases: Vec<(Value, bool, &str)> = vec![
        (
            json!({"hologram": {"size": 1}}),
            true,
            "cannot receive this kind",
        ),
        (
            json!({"hologram": {"size": 2}}),
            false,
            "cannot receive this kind",
        ),
        (
            json!({"file": {"filename": "x", "filesize": 1}}),
            false,
            "did not offer a connection",
        ),
        (
            json!({"file": {"filename": "..", "filesize": 1}}),
            true,
            "name is not usable",
        ),
        (
            json!({"directory": {"mode": "tar", "dirname": "d", "zipsize": 1, "numbytes": 1, "numfiles": 1}}),
            true,
            "unknown directory transfer mode",
        ),
    ];
    for (offer, with_transit, why) in cases {
        let dir = tempfile::tempdir().unwrap();
        let (mut sender, receiver) =
            wyrm_receives(&server, &["--accept-file"], dir.path(), false).await;
        if with_transit {
            let transit = Transit::new(Role::Sender, sender.transit_key()).await;
            sender
                .send_json(&json!({"transit": serde_json::to_value(transit.info()).unwrap()}))
                .await
                .unwrap();
        }
        sender.send_json(&json!({ "offer": offer })).await.unwrap();
        let error = until(&mut sender, "error").await;
        assert!(error.as_str().unwrap().contains(why), "{error}");
        sender.close(Mood::Happy).await;
        fails(receiver).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_sender_error_after_transit_ends_the_receiver() {
    let server = MailboxServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let (mut sender, receiver) = wyrm_receives(&server, &[], dir.path(), false).await;
    let transit = Transit::new(Role::Sender, sender.transit_key()).await;
    sender
        .send_json(&json!({"transit": serde_json::to_value(transit.info()).unwrap()}))
        .await
        .unwrap();
    sender
        .send_json(&json!({"error": "changed my mind"}))
        .await
        .unwrap();
    sender.close(Mood::Happy).await;
    assert!(fails(receiver).await.contains("changed my mind"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_closed_stdout_is_reported_to_the_sender() {
    let server = MailboxServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let (mut sender, mut receiver) = wyrm_receives(&server, &[], dir.path(), false).await;
    drop(receiver.stdout.take());
    sender
        .send_json(&json!({"offer": {"message": "nobody reads this"}}))
        .await
        .unwrap();
    let error = until(&mut sender, "error").await;
    assert!(
        error.as_str().unwrap().contains("could not write"),
        "{error}"
    );
    sender.close(Mood::Happy).await;
    fails(receiver).await;
}

async fn offer_file(sender: &mut Wormhole, name: &str, data: &[u8]) -> (Transit, TransitInfo) {
    let transit = Transit::new(Role::Sender, sender.transit_key()).await;
    sender
        .send_json(&json!({"transit": serde_json::to_value(transit.info()).unwrap()}))
        .await
        .unwrap();
    sender
        .send_json(&json!({"offer": {"file": {"filename": name, "filesize": data.len()}}}))
        .await
        .unwrap();
    let mut theirs = None;
    loop {
        let msg = sender.receive_json().await.unwrap();
        if let Some(info) = msg.get("transit") {
            theirs = Some(serde_json::from_value(info.clone()).unwrap());
        }
        if msg.get("answer").is_some() || msg.get("error").is_some() {
            return (transit, theirs.unwrap());
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn output_paths_are_honoured_or_refused() {
    let server = MailboxServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let (mut sender, receiver) = wyrm_receives(
        &server,
        &["--accept-file", "-o", "renamed.bin"],
        dir.path(),
        false,
    )
    .await;
    let (transit, theirs) = offer_file(&mut sender, "original.bin", b"data").await;
    let mut pipe = transit.connect(&theirs).await.unwrap();
    pipe.send_record(b"data").await.unwrap();
    pipe.flush().await.unwrap();
    let ack: Value = serde_json::from_slice(&pipe.receive_record().await.unwrap()).unwrap();
    assert_eq!(ack["ack"], "ok");
    sender.close(Mood::Happy).await;
    assert!(finish(receiver).await.0);
    assert_eq!(
        std::fs::read(dir.path().join("renamed.bin")).unwrap(),
        b"data"
    );

    let (mut sender, receiver) = wyrm_receives(
        &server,
        &["--accept-file", "-o", "missing/dir/x"],
        dir.path(),
        false,
    )
    .await;
    let transit = Transit::new(Role::Sender, sender.transit_key()).await;
    sender
        .send_json(&json!({"transit": serde_json::to_value(transit.info()).unwrap()}))
        .await
        .unwrap();
    sender
        .send_json(&json!({"offer": {"file": {"filename": "x", "filesize": 1}}}))
        .await
        .unwrap();
    let error = until(&mut sender, "error").await;
    assert!(error.as_str().unwrap().contains("cannot write"), "{error}");
    sender.close(Mood::Happy).await;
    fails(receiver).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_destination_that_appears_mid_transfer_is_never_overwritten() {
    let server = MailboxServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let (mut sender, receiver) =
        wyrm_receives(&server, &["--accept-file"], dir.path(), false).await;
    let (transit, theirs) = offer_file(&mut sender, "late.txt", b"theirs").await;
    std::fs::write(dir.path().join("late.txt"), b"mine").unwrap();
    let mut pipe = transit.connect(&theirs).await.unwrap();
    pipe.send_record(b"theirs").await.unwrap();
    pipe.flush().await.unwrap();
    assert!(fails(receiver).await.contains("appeared"));
    assert_eq!(std::fs::read(dir.path().join("late.txt")).unwrap(), b"mine");
    sender.close(Mood::Happy).await;
}

fn zip_with(size: usize) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    zip.start_file("big", zip::write::SimpleFileOptions::default())
        .unwrap();
    zip.write_all(&vec![1u8; size]).unwrap();
    zip.finish().unwrap().into_inner()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_zip_bigger_than_its_offer_over_iroh_leaves_nothing_behind() {
    let server = MailboxServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let (mut sender, receiver) = wyrm_receives(&server, &["--accept-file"], dir.path(), true).await;
    let iroh = IrohTransport::bind(IrohRole::Sender, Relays::Disabled)
        .await
        .unwrap();
    let zip = zip_with(10_000);
    let zip_path = dir.path().join("big.zip");
    std::fs::write(&zip_path, &zip).unwrap();
    let offered = wyrmyon_transport_iroh::Offered::import(&zip_path)
        .await
        .unwrap();
    std::fs::remove_file(&zip_path).unwrap();
    sender
        .send_json(&json!({"wyrmyon-iroh-v1": serde_json::to_value(iroh.info().await).unwrap()}))
        .await
        .unwrap();
    sender
        .send_json(&json!({"offer": {"directory": {
            "mode": "zipfile/deflated", "dirname": "d",
            "zipsize": zip.len(), "numbytes": 10, "numfiles": 1}}}))
        .await
        .unwrap();
    let theirs: IrohInfo =
        serde_json::from_value(until(&mut sender, "wyrmyon-iroh-v1").await).unwrap();
    until(&mut sender, "answer").await;
    let mut pipe = iroh.connect(&theirs, sender.key()).await.unwrap();
    pipe.provide(&offered).await.unwrap();
    assert!(fails(receiver).await.contains("more than"));
    assert!(!dir.path().join("d").exists());
    pipe.abort().await;
    offered.close().await;
    sender.close(Mood::Happy).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_sender_fails_on_answers_it_does_not_expect() {
    let server = MailboxServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let (mut receiver, sender) = wyrm_sends(&server, &["--text", "hi"], dir.path(), false).await;
    until(&mut receiver, "offer").await;
    receiver.send_json(&json!({"transit": {}})).await.unwrap();
    receiver
        .send_json(&json!({"answer": {"maybe": true}}))
        .await
        .unwrap();
    receiver.close(Mood::Happy).await;
    fails(sender).await;

    std::fs::write(dir.path().join("f"), b"file").unwrap();
    let (mut receiver, sender) = wyrm_sends(&server, &["f"], dir.path(), false).await;
    until(&mut receiver, "offer").await;
    receiver
        .send_json(&json!({"offer": {"message": "me too"}}))
        .await
        .unwrap();
    receiver
        .send_json(&json!({"answer": {"file_ack": "later"}}))
        .await
        .unwrap();
    receiver.close(Mood::Happy).await;
    fails(sender).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_iroh_receiver_must_send_its_address() {
    let server = MailboxServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("f"), b"file").unwrap();
    let (mut receiver, sender) = wyrm_sends(&server, &["f"], dir.path(), true).await;
    until(&mut receiver, "offer").await;
    receiver
        .send_json(&json!({"answer": {"file_ack": "ok"}}))
        .await
        .unwrap();
    receiver.close(Mood::Happy).await;
    fails(sender).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_bad_acknowledgement_fails_the_sender() {
    let server = MailboxServer::start().await;
    for ack in [json!({"ack": "no"}), json!({"ack": "ok", "sha256": "00"})] {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f"), b"file").unwrap();
        let (mut receiver, sender) = wyrm_sends(&server, &["f"], dir.path(), false).await;
        let theirs: TransitInfo =
            serde_json::from_value(until(&mut receiver, "transit").await).unwrap();
        let transit = Transit::new(Role::Receiver, receiver.transit_key()).await;
        receiver
            .send_json(&json!({"transit": serde_json::to_value(transit.info()).unwrap()}))
            .await
            .unwrap();
        receiver
            .send_json(&json!({"answer": {"file_ack": "ok"}}))
            .await
            .unwrap();
        let mut pipe = transit.connect(&theirs).await.unwrap();
        assert_eq!(pipe.receive_record().await.unwrap(), b"file");
        pipe.send_record(ack.to_string().as_bytes()).await.unwrap();
        pipe.flush().await.unwrap();
        fails(sender).await;
        receiver.close(Mood::Happy).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn text_can_come_from_stdin() {
    let server = MailboxServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let mut child = wyrm(&server.url())
        .args(["send", "--text", "-"])
        .stdin(Stdio::piped())
        .current_dir(dir.path())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(b"piped words").await.unwrap();
    drop(stdin);
    let code = read_code(child.stderr.take().unwrap())
        .await
        .parse()
        .unwrap();
    let mut receiver = wyrmyon_wormhole::join(&config(&server, false), code)
        .await
        .unwrap()
        .pair()
        .await
        .unwrap();
    assert_eq!(
        until(&mut receiver, "offer").await,
        json!({"message": "piped words"})
    );
    receiver
        .send_json(&json!({"answer": {"message_ack": "ok"}}))
        .await
        .unwrap();
    receiver.close(Mood::Happy).await;
    assert!(finish(child).await.0);
}

async fn in_a_terminal(
    server: &MailboxServer,
    dir: &Path,
    command: &str,
    answers: &[(&str, &str)],
) -> (bool, String) {
    let mut child = tokio::process::Command::new("python3")
        .args([
            "-c",
            "import pty, sys; sys.exit(pty.spawn(sys.argv[1:]) >> 8)",
        ])
        .arg(env!("CARGO_BIN_EXE_wyrm"))
        .args(command.split(' '))
        .env("WYRMYON_RELAY_URL", server.url())
        .env("WYRMYON_TRANSIT_HELPER", "tcp:127.0.0.1:9")
        .env("WYRMYON_IROH_RELAYS", "disabled")
        .env("WYRMYON_CACHE_DIR", support::cache_dir())
        .current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .expect("python3 on PATH");
    let (mut stdin, mut stdout) = (child.stdin.take().unwrap(), child.stdout.take().unwrap());
    let mut seen = String::new();
    for (prompt, answer) in answers {
        let wait = async {
            while !seen.contains(prompt) {
                let mut buf = [0u8; 1024];
                let n = stdout.read(&mut buf).await.unwrap();
                assert!(n > 0, "the prompt never came: {seen}");
                seen.push_str(&String::from_utf8_lossy(&buf[..n]));
            }
        };
        tokio::time::timeout(TIMEOUT, wait).await.unwrap();
        seen.clear();
        stdin
            .write_all(format!("{answer}\n").as_bytes())
            .await
            .unwrap();
    }
    let mut rest = String::new();
    let _ = tokio::time::timeout(TIMEOUT, stdout.read_to_string(&mut rest)).await;
    let status = tokio::time::timeout(TIMEOUT, child.wait())
        .await
        .unwrap()
        .unwrap();
    (status.success(), rest)
}

#[tokio::test(flavor = "multi_thread")]
async fn in_a_terminal_the_receiver_asks_for_the_code_and_for_consent() {
    let server = MailboxServer::start().await;
    for (answer, accepted) in [("y", true), ("n", false)] {
        let dir = tempfile::tempdir().unwrap();
        let pending = wyrmyon_wormhole::create(&config(&server, false), 2)
            .await
            .unwrap();
        let code = pending.code().to_string();
        let questions = [("wormhole code", code.as_str()), ("(y/N)", answer)];
        let terminal = in_a_terminal(&server, dir.path(), "receive", &questions);
        let peer = async {
            let mut sender = pending.pair().await.unwrap();
            let (transit, theirs) = offer_file(&mut sender, "asked.txt", b"ask").await;
            if accepted {
                let mut pipe = transit.connect(&theirs).await.unwrap();
                pipe.send_record(b"ask").await.unwrap();
                pipe.flush().await.unwrap();
                let _ = pipe.receive_record().await.unwrap();
            }
            sender.close(Mood::Happy).await;
        };
        let ((ok, _), ()) = tokio::join!(terminal, peer);
        assert_eq!(ok, accepted);
        assert_eq!(dir.path().join("asked.txt").exists(), accepted);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_server_motd_is_shown_and_progress_can_be_hidden() {
    let mut welcome = serde_json::Map::new();
    welcome.insert("motd".into(), json!("maintenance tonight"));
    let server = MailboxServer::start_with_welcome(welcome).await;
    let dir = tempfile::tempdir().unwrap();
    let (mut sender, receiver) = wyrm_receives(
        &server,
        &["--accept-file", "--hide-progress"],
        dir.path(),
        false,
    )
    .await;
    let (transit, theirs) = offer_file(&mut sender, "m.txt", b"motd").await;
    let mut pipe = transit.connect(&theirs).await.unwrap();
    pipe.send_record(b"motd").await.unwrap();
    pipe.flush().await.unwrap();
    let _ = pipe.receive_record().await.unwrap();
    sender.close(Mood::Happy).await;
    let (ok, _, stderr) = finish(receiver).await;
    assert!(ok, "{stderr}");
    assert!(
        stderr.contains("Server (at relay): maintenance tonight"),
        "{stderr}"
    );
}
