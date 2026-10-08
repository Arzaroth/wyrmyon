use std::time::Duration;

use wyrmyon_transport_classic::{Error, Role, Transit, TransitInfo};
use wyrmyon_wormhole::Key;

fn key(byte: u8) -> Key {
    Key::from_bytes([byte; 32])
}

async fn within<T>(future: impl Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(10), future)
        .await
        .expect("timed out")
}

#[tokio::test]
async fn sender_and_receiver_meet_and_exchange_records_both_ways() {
    let sender = Transit::new(Role::Sender, key(7)).await;
    let receiver = Transit::new(Role::Receiver, key(7)).await;
    let (sender_info, receiver_info) = (sender.info(), receiver.info());
    assert_ne!(sender_info.direct_hints(), []);

    let (upstream, downstream) = within(async {
        tokio::join!(
            sender.connect(&receiver_info),
            receiver.connect(&sender_info)
        )
    })
    .await;
    let (mut upstream, mut downstream) = (upstream.unwrap(), downstream.unwrap());

    let big = vec![0x5a; wyrmyon_transport_classic::MAX_RECORD];
    let writer = tokio::spawn({
        let big = big.clone();
        async move {
            upstream.send_record(b"first").await.unwrap();
            upstream.send_record(&big).await.unwrap();
            upstream.flush().await.unwrap();
            upstream
        }
    });
    assert_eq!(within(downstream.receive_record()).await.unwrap(), b"first");
    assert_eq!(within(downstream.receive_record()).await.unwrap(), big);
    let mut upstream = within(writer).await.unwrap();

    downstream.send_record(b"ack").await.unwrap();
    downstream.flush().await.unwrap();
    assert_eq!(within(upstream.receive_record()).await.unwrap(), b"ack");
}

#[tokio::test]
async fn different_keys_never_connect() {
    let sender = Transit::new(Role::Sender, key(1)).await;
    let receiver = Transit::new(Role::Receiver, key(2)).await;
    let (sender_info, receiver_info) = (sender.info(), receiver.info());
    let race = async {
        tokio::join!(
            sender.connect(&receiver_info),
            receiver.connect(&sender_info)
        )
    };
    assert!(
        tokio::time::timeout(Duration::from_secs(2), race)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn with_no_way_to_reach_the_peer_connect_gives_up() {
    let receiver = Transit::new(Role::Receiver, key(3))
        .await
        .without_listener()
        .with_timeout(Duration::from_millis(200));
    assert_eq!(receiver.info().direct_hints(), []);
    let result = within(receiver.connect(&TransitInfo::default())).await;
    assert!(matches!(result, Err(Error::NoConnection)));
}
