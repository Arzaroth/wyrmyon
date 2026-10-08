use serde_json::{Map, Value, json};
use wyrmyon_testkit::MailboxServer;
use wyrmyon_wormhole::{Code, Config, Error, Mood};

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
    let pending = wyrmyon_wormhole::create(&config(&server, json!({"a": 1})), 2)
        .await
        .unwrap();
    let code = pending.code().clone();
    assert_eq!(code.nameplate(), "1");

    let receiver = tokio::spawn({
        let config = config(&server, json!({"b": 2}));
        async move { wyrmyon_wormhole::connect(&config, code).await.unwrap() }
    });
    let mut sender = pending.pair().await.unwrap();
    let mut receiver = receiver.await.unwrap();

    assert_eq!(sender.key(), receiver.key());
    assert_eq!(sender.verifier(), receiver.verifier());
    assert_eq!(sender.their_app_versions(), &json!({"b": 2}));
    assert_eq!(receiver.their_app_versions(), &json!({"a": 1}));

    sender.send(b"one").await.unwrap();
    sender.send_json(&json!({"two": 2})).await.unwrap();
    receiver.send(b"back").await.unwrap();
    assert_eq!(receiver.receive().await.unwrap(), b"one");
    assert_eq!(receiver.receive_json().await.unwrap(), json!({"two": 2}));
    assert_eq!(sender.receive().await.unwrap(), b"back");

    sender.close(Mood::Happy).await;
    receiver.close(Mood::Happy).await;
}

#[tokio::test]
async fn a_wrong_code_fails_the_key_exchange() {
    let server = MailboxServer::start().await;
    let pending = wyrmyon_wormhole::create(&config(&server, json!({})), 2)
        .await
        .unwrap();
    let wrong: Code = format!("{}-not-this", pending.code().nameplate())
        .parse()
        .unwrap();
    let receiver = tokio::spawn({
        let config = config(&server, json!({}));
        async move { wyrmyon_wormhole::connect(&config, wrong).await }
    });
    assert!(matches!(pending.pair().await, Err(Error::WrongCode)));
    assert!(matches!(receiver.await.unwrap(), Err(Error::WrongCode)));
}

#[tokio::test]
async fn the_welcome_message_reaches_the_caller() {
    let mut welcome = Map::new();
    welcome.insert("motd".into(), json!("be nice"));
    let server = MailboxServer::start_with_welcome(welcome).await;
    let pending = wyrmyon_wormhole::create(&config(&server, json!({})), 2)
        .await
        .unwrap();
    assert_eq!(pending.welcome().motd.as_deref(), Some("be nice"));
    pending.abandon().await;

    let mut refusing = Map::new();
    refusing.insert("error".into(), json!("go away"));
    let server = MailboxServer::start_with_welcome(refusing).await;
    let result = wyrmyon_wormhole::create(&config(&server, json!({})), 2).await;
    assert!(matches!(result, Err(Error::Welcome(e)) if e == "go away"));
}
