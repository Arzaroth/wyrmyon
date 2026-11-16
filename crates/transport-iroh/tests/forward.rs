use std::collections::HashMap;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpSocket, TcpStream};
use wyrmyon_transport_iroh::{
    ForwardEvent, ForwardNext, Forwarding, IrohPipe, IrohTransport, Relays, Role,
};
use wyrmyon_wormhole::Key;

async fn within<T>(future: impl Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(20), future)
        .await
        .expect("timed out")
}

async fn pipes() -> (IrohPipe, IrohPipe) {
    let key = Key::from_bytes([5; 32]);
    let server = IrohTransport::bind(Role::Sender, Relays::Disabled)
        .await
        .unwrap();
    let client = IrohTransport::bind(Role::Receiver, Relays::Disabled)
        .await
        .unwrap();
    let (server_info, client_info) = (server.info().await, client.info().await);
    let (server, client) = within(async {
        tokio::join!(
            server.connect(&client_info, &key),
            client.connect(&server_info, &key)
        )
    })
    .await;
    (server.unwrap(), client.unwrap())
}

async fn echo() -> (u16, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let task = tokio::spawn(async move {
        loop {
            let (mut tcp, _) = listener.accept().await.unwrap();
            tokio::spawn(async move {
                let mut got = Vec::new();
                tcp.read_to_end(&mut got).await.unwrap();
                got.reverse();
                tcp.write_all(&got).await.unwrap();
            });
        }
    });
    (port, task)
}

async fn local() -> (TcpListener, u16) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    (listener, port)
}

fn refusing() -> (TcpSocket, u16) {
    let socket = TcpSocket::new_v4().unwrap();
    socket.bind("127.0.0.1:0".parse().unwrap()).unwrap();
    let port = socket.local_addr().unwrap().port();
    (socket, port)
}

async fn event(forwarding: &mut Forwarding) -> ForwardEvent {
    match within(forwarding.next()).await {
        ForwardNext::Event(event) => event,
        ForwardNext::Ended(result) => panic!("ended instead: {result:?}"),
    }
}

async fn round_trip(port: u16, data: &[u8]) -> Vec<u8> {
    let mut tcp = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let _ = tcp.write_all(data).await;
    let _ = tcp.shutdown().await;
    let mut back = Vec::new();
    let _ = within(tcp.read_to_end(&mut back)).await;
    back
}

#[tokio::test(flavor = "multi_thread")]
async fn connections_reach_the_served_port_and_half_close_carries_through() {
    let (target, _echo) = echo().await;
    let (sharing, client) = pipes().await;
    let mut served = sharing.serve_ports(HashMap::from([(target, format!("127.0.0.1:{target}"))]));
    let (listener, here) = local().await;
    let forwarding = client.forward_ports(vec![(listener, target)]);
    assert!(forwarding.pipe().describe().starts_with("iroh"));

    for _ in 0..3 {
        assert_eq!(round_trip(here, b"stressed").await, b"desserts");
        assert_eq!(
            event(&mut served).await,
            ForwardEvent::Opened { port: target }
        );
    }
    let big: Vec<u8> = (0..1_000_000u32).map(|n| (n % 251) as u8).collect();
    let mut reversed = big.clone();
    reversed.reverse();
    assert_eq!(round_trip(here, &big).await, reversed);
    assert_eq!(
        event(&mut served).await,
        ForwardEvent::Opened { port: target }
    );

    forwarding.close().await;
    assert!(matches!(
        within(served.next()).await,
        ForwardNext::Ended(Ok(()))
    ));
    served.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_port_not_served_and_a_dead_target_are_reported_on_both_sides() {
    let (_dead, dead_port) = refusing();
    let (sharing, client) = pipes().await;
    let mut served = sharing.serve_ports(HashMap::from([(
        dead_port,
        format!("127.0.0.1:{dead_port}"),
    )]));
    let (unserved_listener, unserved) = local().await;
    let (dead_listener, here) = local().await;
    let mut forwarding =
        client.forward_ports(vec![(unserved_listener, 1), (dead_listener, dead_port)]);

    assert_eq!(round_trip(unserved, b"x").await, b"");
    assert_eq!(
        event(&mut served).await,
        ForwardEvent::NotForwarded { port: 1 }
    );
    assert_eq!(
        event(&mut forwarding).await,
        ForwardEvent::NotForwarded { port: 1 }
    );

    assert_eq!(round_trip(here, b"x").await, b"");
    assert!(matches!(
        event(&mut served).await,
        ForwardEvent::Unreachable { port, why: Some(_) } if port == dead_port
    ));
    assert_eq!(
        event(&mut forwarding).await,
        ForwardEvent::Unreachable {
            port: dead_port,
            why: None
        }
    );

    served.close().await;
    assert!(matches!(
        within(forwarding.next()).await,
        ForwardNext::Ended(Ok(()))
    ));
    assert_eq!(round_trip(here, b"after").await, b"");
    forwarding.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reset_on_one_end_resets_the_other() {
    let (target, port) = local().await;
    tokio::spawn(async move {
        let (tcp, _) = target.accept().await.unwrap();
        let mut got = [0u8; 1];
        let mut tcp = tcp;
        tcp.read_exact(&mut got).await.unwrap();
        tcp.set_zero_linger().unwrap();
    });
    let (sharing, client) = pipes().await;
    let served = sharing.serve_ports(HashMap::from([(port, format!("127.0.0.1:{port}"))]));
    let (listener, here) = local().await;
    let forwarding = client.forward_ports(vec![(listener, port)]);

    let mut tcp = TcpStream::connect(("127.0.0.1", here)).await.unwrap();
    tcp.write_all(b"x").await.unwrap();
    let mut back = Vec::new();
    let read = within(tcp.read_to_end(&mut back)).await;
    assert!(read.is_err(), "{read:?} {back:?}");

    forwarding.close().await;
    served.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_connection_can_be_given_up_before_it_is_up() {
    let key = Key::from_bytes([5; 32]);
    let waiting = IrohTransport::bind(Role::Sender, Relays::Disabled)
        .await
        .unwrap();
    let other = IrohTransport::bind(Role::Receiver, Relays::Disabled)
        .await
        .unwrap();
    let theirs = other.info().await;
    assert!(
        waiting
            .connect_unless(&theirs, &key, async {})
            .await
            .is_none()
    );
    other.close().await;
}
