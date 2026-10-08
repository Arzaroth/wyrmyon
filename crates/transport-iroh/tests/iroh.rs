use std::time::Duration;

use wyrmyon_transport_iroh::{Error, IrohTransport, Relays, Role};
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
    assert!(
        matches!(upstream, Err(Error::WrongPeer)) || matches!(downstream, Err(Error::WrongPeer))
    );
    assert!(upstream.is_err() || downstream.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn the_receiver_only_takes_the_announced_sender() {
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
    let sender_info = sender.info().await;
    let receiver_info = receiver.info().await;
    let receiver_key = key.clone();
    let waiting = tokio::spawn(async move {
        tokio::time::timeout(
            Duration::from_secs(3),
            receiver.connect(&sender_info, &receiver_key),
        )
        .await
    });
    let intruder = stranger.connect(&receiver_info, &key).await;
    assert!(intruder.is_err());
    assert!(
        waiting.await.unwrap().is_err(),
        "the receiver accepted a stranger"
    );
    drop(sender);
}
