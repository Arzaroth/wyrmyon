use std::time::Duration;

use wyrmyon_transport_iroh::{Error, IrohInfo, IrohTransport, Relays, Role};
use wyrmyon_wormhole::Key;

async fn within<T>(future: impl Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(20), future)
        .await
        .expect("timed out")
}

#[tokio::test(flavor = "multi_thread")]
async fn two_endpoints_bind_the_channel_and_move_data() {
    let key = Key::from_bytes([4; 32]);
    let sender = IrohTransport::bind(Role::Sender, Relays::Disabled)
        .await
        .unwrap();
    let receiver = IrohTransport::bind(Role::Receiver, Relays::Disabled)
        .await
        .unwrap();
    let (sender_info, receiver_info) = (sender.info().await, receiver.info().await);
    assert_ne!(receiver_info.direct, Vec::<String>::new());

    let (upstream, downstream) = within(async {
        tokio::join!(
            sender.connect(&receiver_info, &key),
            receiver.connect(&sender_info, &key)
        )
    })
    .await;
    let (mut upstream, mut downstream) = (upstream.unwrap(), downstream.unwrap());
    assert!(upstream.describe().starts_with("iroh"));

    let data = vec![9u8; 300_000];
    let writer = tokio::spawn(async move {
        upstream.send_chunk(&data).await.unwrap();
        let ack = upstream.receive_last().await.unwrap();
        upstream.finish().await;
        ack
    });
    let mut got = Vec::new();
    while got.len() < 300_000 {
        got.extend(within(downstream.receive_chunk(1 << 16)).await.unwrap());
    }
    assert_eq!(got, vec![9u8; 300_000]);
    downstream.send_last(b"done").await.unwrap();
    assert_eq!(within(writer).await.unwrap(), b"done");
    downstream.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_different_wormhole_key_fails_the_binding() {
    let sender = IrohTransport::bind(Role::Sender, Relays::Disabled)
        .await
        .unwrap();
    let receiver = IrohTransport::bind(Role::Receiver, Relays::Disabled)
        .await
        .unwrap();
    let (sender_info, receiver_info) = (sender.info().await, receiver.info().await);
    let (one, two) = (Key::from_bytes([1; 32]), Key::from_bytes([2; 32]));
    let (upstream, downstream) = within(async {
        tokio::join!(
            sender.connect(&receiver_info, &one),
            receiver.connect(&sender_info, &two)
        )
    })
    .await;
    assert!(upstream.is_err(), "the sender accepted a wrong binding");
    assert!(downstream.is_err(), "the receiver accepted a wrong binding");
    assert!(
        matches!(upstream, Err(Error::WrongPeer)) || matches!(downstream, Err(Error::WrongPeer))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_receiver_turns_a_stranger_away_and_still_takes_the_sender() {
    let key = Key::from_bytes([3; 32]);
    let sender = IrohTransport::bind(Role::Sender, Relays::Disabled)
        .await
        .unwrap();
    let stranger = IrohTransport::bind(Role::Sender, Relays::Disabled)
        .await
        .unwrap();
    let receiver = IrohTransport::bind(Role::Receiver, Relays::Disabled)
        .await
        .unwrap();
    let (sender_info, receiver_info) = (sender.info().await, receiver.info().await);
    let receiver_key = key.clone();
    let waiting = tokio::spawn(async move { receiver.connect(&sender_info, &receiver_key).await });
    assert!(
        within(stranger.connect(&receiver_info, &key))
            .await
            .is_err()
    );
    let upstream = within(sender.connect(&receiver_info, &key)).await;
    let downstream = within(waiting).await.unwrap();
    assert!(upstream.is_ok(), "{:?}", upstream.err());
    assert!(downstream.is_ok(), "{:?}", downstream.err());
}

#[tokio::test(flavor = "multi_thread")]
async fn unusable_addresses_fail_at_once() {
    let key = Key::from_bytes([8; 32]);
    let real = IrohTransport::bind(Role::Receiver, Relays::Disabled)
        .await
        .unwrap();
    let id = real.info().await.id;
    let bad = [
        IrohInfo {
            id: "garbage".into(),
            relays: vec![],
            direct: vec!["127.0.0.1:9".into()],
        },
        IrohInfo {
            id,
            relays: vec!["not a url".into()],
            direct: vec!["nope".into()],
        },
    ];
    for info in bad {
        let sender = IrohTransport::bind(Role::Sender, Relays::Disabled)
            .await
            .unwrap();
        let result =
            tokio::time::timeout(Duration::from_secs(2), sender.connect(&info, &key)).await;
        assert!(matches!(result, Ok(Err(Error::BadAddress(_)))));
    }
    real.close().await;
}
