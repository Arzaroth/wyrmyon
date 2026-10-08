use std::future::Future;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{Map, Value, json};
use tokio_tungstenite::tungstenite::Message as Frame;
use wyrmyon_testkit::MailboxServer;
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
