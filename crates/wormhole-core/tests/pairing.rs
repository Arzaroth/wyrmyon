use std::future::Future;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{Map, Value, json};
use tokio_tungstenite::tungstenite::Message as Frame;
use wyrmyon_testkit::{MailboxServer, Quirks};
use wyrmyon_wormhole::{Code, Config, Error, Mood};

async fn within<T>(future: impl Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(10), future)
        .await
        .expect("timed out")
}

fn config(server: &MailboxServer, versions: Value) -> Config {
    Config {
        relay_url: server.url(),
        app_versions: versions,
        ..Config::default()
    }
}

#[tokio::test]
async fn two_peers_agree_on_a_key_and_exchange_messages_in_order() {
    let server = MailboxServer::start().await;
    let pending = within(wyrmyon_wormhole::create(
        &config(&server, json!({"a": 1})),
        2,
    ))
    .await
    .unwrap();
    let code = pending.code().clone();
    assert_eq!(code.nameplate(), "1");

    let receiver = tokio::spawn({
        let config = config(&server, json!({"b": 2}));
        async move {
            within(async { wyrmyon_wormhole::join(&config, code).await?.pair().await })
                .await
                .unwrap()
        }
    });
    let mut sender = within(pending.pair()).await.unwrap();
    let mut receiver = receiver.await.unwrap();

    assert_eq!(sender.key(), receiver.key());
    assert_eq!(sender.transit_key(), receiver.transit_key());
    assert_eq!(sender.verifier(), receiver.verifier());
    assert_eq!(sender.their_app_versions(), &json!({"b": 2}));
    assert_eq!(receiver.their_app_versions(), &json!({"a": 1}));

    sender.send(b"one").await.unwrap();
    sender.send_json(&json!({"two": 2})).await.unwrap();
    receiver.send(b"back").await.unwrap();
    assert_eq!(within(receiver.receive()).await.unwrap(), b"one");
    assert_eq!(
        within(receiver.receive_json()).await.unwrap(),
        json!({"two": 2})
    );
    assert_eq!(within(sender.receive()).await.unwrap(), b"back");

    within(sender.close(Mood::Happy)).await;
    within(receiver.close(Mood::Happy)).await;
    assert_eq!(server.claimed_nameplates(), Vec::<String>::new());
    assert_eq!(server.moods(), ["happy", "happy"]);
}

#[tokio::test]
async fn a_wrong_code_fails_the_key_exchange_and_closes_scary() {
    let server = MailboxServer::start().await;
    let pending = within(wyrmyon_wormhole::create(&config(&server, json!({})), 2))
        .await
        .unwrap();
    let wrong: Code = format!("{}-not-this", pending.code().nameplate())
        .parse()
        .unwrap();
    let receiver = tokio::spawn({
        let config = config(&server, json!({}));
        async move { within(async { wyrmyon_wormhole::join(&config, wrong).await?.pair().await }).await }
    });
    assert!(matches!(
        within(pending.pair()).await,
        Err(Error::WrongCode)
    ));
    assert!(matches!(receiver.await.unwrap(), Err(Error::WrongCode)));
    assert_eq!(server.claimed_nameplates(), Vec::<String>::new());
    assert_eq!(server.moods(), ["scary", "scary"]);
}

#[tokio::test]
async fn the_welcome_message_reaches_the_caller() {
    let mut welcome = Map::new();
    welcome.insert("motd".into(), json!("be nice"));
    let server = MailboxServer::start_with_welcome(welcome).await;
    let pending = within(wyrmyon_wormhole::create(&config(&server, json!({})), 2))
        .await
        .unwrap();
    assert_eq!(pending.welcome().motd.as_deref(), Some("be nice"));
    within(pending.abandon()).await;
    assert_eq!(server.claimed_nameplates(), Vec::<String>::new());
    assert_eq!(server.moods(), ["lonely"]);

    let mut refusing = Map::new();
    refusing.insert("error".into(), json!("go away"));
    let server = MailboxServer::start_with_welcome(refusing).await;
    let result = within(wyrmyon_wormhole::create(&config(&server, json!({})), 2)).await;
    assert!(matches!(result, Err(Error::Welcome(e)) if e == "go away"));
}

#[tokio::test]
async fn a_third_side_on_the_nameplate_is_turned_away() {
    let server = MailboxServer::start().await;
    let config = config(&server, json!({}));
    let first = within(wyrmyon_wormhole::create(&config, 2)).await.unwrap();
    let second = within(wyrmyon_wormhole::join(&config, first.code().clone()))
        .await
        .unwrap();
    let third = within(wyrmyon_wormhole::join(&config, first.code().clone())).await;
    assert!(matches!(third, Err(Error::Server(e)) if e == "crowded"));
    within(first.abandon()).await;
    within(second.abandon()).await;
    assert_eq!(server.claimed_nameplates(), Vec::<String>::new());
}

#[tokio::test]
async fn a_malformed_pake_message_fails_cleanly() {
    let server = MailboxServer::start().await;
    let pending = within(wyrmyon_wormhole::create(&config(&server, json!({})), 2))
        .await
        .unwrap();
    let nameplate = pending.code().nameplate().to_owned();

    let (mut peer, _) = tokio_tungstenite::connect_async(server.url())
        .await
        .unwrap();
    let send = |v: Value| Frame::text(v.to_string());
    peer.send(send(
        json!({"type": "bind", "appid": "x", "side": "evil", "id": "1"}),
    ))
    .await
    .unwrap();
    peer.send(send(
        json!({"type": "claim", "nameplate": nameplate, "id": "2"}),
    ))
    .await
    .unwrap();
    let mailbox = loop {
        let Some(Ok(Frame::Text(text))) = peer.next().await else {
            panic!("peer connection ended");
        };
        let msg: Value = serde_json::from_str(&text).unwrap();
        if msg["type"] == "claimed" {
            break msg["mailbox"].as_str().unwrap().to_owned();
        }
    };
    peer.send(send(json!({"type": "open", "mailbox": mailbox, "id": "3"})))
        .await
        .unwrap();
    peer.send(send(
        json!({"type": "add", "phase": "pake", "body": hex::encode("not json"), "id": "4"}),
    ))
    .await
    .unwrap();

    let result = within(pending.pair()).await;
    assert!(matches!(result, Err(Error::Protocol(_))));
    assert_eq!(server.moods(), ["scary"]);
}

#[tokio::test]
async fn a_misbehaving_server_fails_the_session_cleanly() {
    for quirks in [
        Quirks {
            allocate_as: Some("1; rm -rf".into()),
            ..Quirks::default()
        },
        Quirks {
            refuse_allocate: true,
            ..Quirks::default()
        },
        Quirks {
            flood_on_claim: 100,
            ..Quirks::default()
        },
        Quirks {
            hang_up_after_welcome: true,
            ..Quirks::default()
        },
    ] {
        let server = MailboxServer::start_with(quirks).await;
        let result = within(wyrmyon_wormhole::create(&config(&server, json!({})), 2)).await;
        assert!(result.is_err());
    }
}

#[test]
fn codes_and_keys_never_print_their_secrets() {
    let code: Code = "7-guitarist-revenge".parse().unwrap();
    assert_eq!(format!("{code:?}"), "Code(..)");
    assert_eq!(
        format!("{:?}", wyrmyon_wormhole::Key::from_bytes([1; 32])),
        "Key(..)"
    );
}

#[tokio::test]
async fn a_chatty_server_and_a_stranger_in_the_mailbox_do_not_disturb_the_peers() {
    let server = MailboxServer::start_with(Quirks {
        chatty: true,
        ..Quirks::default()
    })
    .await;
    let first = within(wyrmyon_wormhole::create(&config(&server, json!({})), 2))
        .await
        .unwrap();
    let code = first.code().clone();

    let (mut stranger, _) = tokio_tungstenite::connect_async(server.url())
        .await
        .unwrap();
    let say = |v: Value| Frame::text(v.to_string());
    for msg in [
        json!({"type": "bind", "appid": "x", "side": "stranger", "id": "1"}),
        json!({"type": "open", "mailbox": "mb1", "id": "2"}),
        json!({"type": "add", "phase": "noise", "body": "00", "id": "3"}),
    ] {
        stranger.send(say(msg)).await.unwrap();
    }

    let second = tokio::spawn({
        let config = config(&server, json!({}));
        async move { within(async { wyrmyon_wormhole::join(&config, code).await?.pair().await }).await }
    });
    let mut first = within(first.pair()).await.unwrap();
    let mut second = second.await.unwrap().unwrap();
    stranger
        .send(say(
            json!({"type": "add", "phase": "0", "body": "00", "id": "4"}),
        ))
        .await
        .unwrap();
    first.send(b"hello").await.unwrap();
    assert_eq!(within(second.receive()).await.unwrap(), b"hello");
    within(first.close(Mood::Happy)).await;
    within(second.close(Mood::Happy)).await;
}

async fn raw_peer(
    server: &MailboxServer,
    side: &str,
) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>> {
    let (mut ws, _) = tokio_tungstenite::connect_async(server.url())
        .await
        .unwrap();
    for msg in [
        json!({"type": "bind", "appid": "x", "side": side, "id": "1"}),
        json!({"type": "open", "mailbox": "mb1", "id": "2"}),
    ] {
        ws.send(Frame::text(msg.to_string())).await.unwrap();
    }
    ws
}

#[tokio::test]
async fn a_forged_phase_under_the_peers_side_fails_as_tampering() {
    let server = MailboxServer::start().await;
    let first = within(wyrmyon_wormhole::create(&config(&server, json!({})), 2))
        .await
        .unwrap();
    let second = within(wyrmyon_wormhole::join(
        &config(&server, json!({})),
        first.code().clone(),
    ))
    .await
    .unwrap();

    let mut watcher = raw_peer(&server, "watcher").await;
    let mut sides = Vec::new();
    while sides.len() < 2 {
        let Some(Ok(Frame::Text(text))) = within(watcher.next()).await else {
            panic!("the watcher lost the mailbox");
        };
        let msg: Value = serde_json::from_str(&text).unwrap();
        if msg["type"] == "message" && msg["phase"] == "pake" {
            sides.push(msg["side"].as_str().unwrap().to_owned());
        }
    }
    let mut impostor = raw_peer(&server, &sides[1]).await;
    impostor
        .send(Frame::text(
            json!({"type": "add", "phase": "0", "body": "00", "id": "3"}).to_string(),
        ))
        .await
        .unwrap();
    while let Some(Ok(Frame::Text(text))) = within(impostor.next()).await {
        if text.contains("\"phase\":\"0\"") || text.contains("\"phase\": \"0\"") {
            break;
        }
    }

    let second = tokio::spawn(async move { within(second.pair()).await });
    let first = within(first.pair()).await;
    assert!(matches!(first, Err(Error::Tampered)), "{:?}", first.err());
    let _ = second.await;
}
