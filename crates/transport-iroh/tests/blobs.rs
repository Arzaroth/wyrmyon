use std::ops::ControlFlow;
use std::time::Duration;

use wyrmyon_transport_iroh::{IrohPipe, IrohTransport, Offered, Relays, Role};
use wyrmyon_wormhole::Key;

async fn within<T>(future: impl Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(60), future)
        .await
        .expect("timed out")
}

async fn connected() -> (IrohPipe, IrohPipe) {
    let key = Key::from_bytes([9; 32]);
    let sender = IrohTransport::bind(Role::Sender, Relays::Disabled)
        .await
        .unwrap();
    let receiver = IrohTransport::bind(Role::Receiver, Relays::Disabled)
        .await
        .unwrap();
    let (sender_info, receiver_info) = (sender.info().await, receiver.info().await);
    let (upstream, downstream) = within(async {
        tokio::join!(
            sender.connect(&receiver_info, &key),
            receiver.connect(&sender_info, &key)
        )
    })
    .await;
    (upstream.unwrap(), downstream.unwrap())
}

fn data(len: usize) -> Vec<u8> {
    (0..len).map(|i| u8::try_from(i % 249).unwrap()).collect()
}

struct Unreadable;
impl tokio::io::AsyncRead for Unreadable {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
        _: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Ready(Err(std::io::Error::other("revoked")))
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_file_arrives_verified_and_the_cache_is_cleared() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.bin");
    std::fs::write(&source, data(1_000_000)).unwrap();
    let cache = dir.path().join("cache");
    let offered = Offered::import(&source).await.unwrap();

    let (mut upstream, mut downstream) = connected().await;
    upstream.provide(&offered).await.unwrap();
    let mut seen = 0;
    let fetched = within(downstream.fetch(&cache, 1_000_000, &mut |n| seen = n))
        .await
        .unwrap();
    assert_eq!(seen, 1_000_000);
    let mut sent = upstream.sent();
    within(sent.wait_for(|n| *n == 1_000_000)).await.unwrap();
    let target = dir.path().join("target.bin");
    std::fs::write(&target, b"placeholder").unwrap();
    fetched.export_to(&target).await.unwrap();
    assert_eq!(std::fs::read(&target).unwrap(), data(1_000_000));
    assert_eq!(std::fs::read_dir(&cache).unwrap().count(), 0);
    downstream.send_last(b"ok").await.unwrap();
    assert_eq!(within(upstream.receive_last()).await.unwrap(), b"ok");
    upstream.finish().await;
    downstream.finish().await;
    offered.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_blob_bigger_or_smaller_than_the_offer_is_refused_and_dropped() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.bin");
    std::fs::write(&source, data(200_000)).unwrap();
    let offered = Offered::import(&source).await.unwrap();
    for offered_size in [1_000, 300_000] {
        let cache = dir.path().join(format!("cache-{offered_size}"));
        let (mut upstream, mut downstream) = connected().await;
        upstream.provide(&offered).await.unwrap();
        let mut seen = 0;
        let result = within(downstream.fetch(&cache, offered_size, &mut |n| seen = n)).await;
        assert!(result.is_err());
        assert!(seen <= offered_size);
        assert_eq!(std::fs::read_dir(&cache).unwrap().count(), 0);
        upstream.abort().await;
        downstream.abort().await;
    }
    offered.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_interrupted_transfer_resumes_from_what_is_cached() {
    const SIZE: usize = 64 << 20;
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("big.bin");
    std::fs::write(&source, data(SIZE)).unwrap();
    let cache = dir.path().join("cache");
    let offered = Offered::import(&source).await.unwrap();

    let (mut upstream, mut downstream) = connected().await;
    upstream.provide(&offered).await.unwrap();
    let (cut_tx, cut_rx) = std::sync::mpsc::channel::<()>();
    let cutter = tokio::spawn(async move {
        tokio::task::spawn_blocking(move || cut_rx.recv())
            .await
            .unwrap()
            .unwrap();
        upstream.abort().await;
    });
    let mut cut_tx = Some(cut_tx);
    let first = within(downstream.fetch(&cache, SIZE as u64, &mut move |n| {
        if n > 4 << 20
            && let Some(cut) = cut_tx.take()
        {
            let _ = cut.send(());
            std::thread::sleep(Duration::from_millis(500));
        }
    }))
    .await;
    within(cutter).await.unwrap();
    downstream.abort().await;
    let kept = first.err().expect("the first attempt was cut").to_string();
    assert!(kept.contains("kept in"), "{kept}");

    let (mut upstream, mut downstream) = connected().await;
    upstream.provide(&offered).await.unwrap();
    let mut resumed_from = None;
    let fetched = within(downstream.fetch(&cache, SIZE as u64, &mut |n| {
        resumed_from.get_or_insert(n);
    }))
    .await
    .unwrap();
    let resumed_from = resumed_from.unwrap();
    assert!(
        resumed_from > 0 && resumed_from < SIZE as u64,
        "resumed from {resumed_from}"
    );
    let target = dir.path().join("out.bin");
    fetched.export_to(&target).await.unwrap();
    let same = std::fs::read(&target).unwrap() == data(SIZE);
    assert!(same, "the resumed file differs from the source");
    upstream.abort().await;
    downstream.abort().await;
    offered.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_export_discards_what_it_may_have_moved() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.bin");
    std::fs::write(&source, data(5_000)).unwrap();
    let cache = dir.path().join("cache");
    let offered = Offered::import(&source).await.unwrap();
    let (mut upstream, mut downstream) = connected().await;
    upstream.provide(&offered).await.unwrap();
    let fetched = within(downstream.fetch(&cache, 5_000, &mut |_| {}))
        .await
        .unwrap();
    let occupied = dir.path().join("occupied");
    std::fs::create_dir(&occupied).unwrap();
    assert!(fetched.export_to(&occupied).await.is_err());
    assert_eq!(std::fs::read_dir(&cache).unwrap().count(), 0);
    upstream.abort().await;
    downstream.abort().await;
    offered.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn importing_something_that_is_not_there_fails() {
    let dir = tempfile::tempdir().unwrap();
    assert!(Offered::import(&dir.path().join("missing")).await.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn importing_a_file_counts_no_more_than_its_size() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.bin");
    std::fs::write(&source, data(1_000_000)).unwrap();
    let mut seen = Vec::new();
    let offered = Offered::import_with_progress(&source, &mut |n| {
        seen.push(n);
        ControlFlow::Continue(())
    })
    .await
    .unwrap();
    assert!(seen.is_sorted(), "{seen:?}");
    assert!(seen.iter().all(|&n| n <= 1_000_000), "{seen:?}");
    offered.close().await;
    let mut seen = Vec::new();
    let missing = dir.path().join("missing");
    assert!(
        Offered::import_with_progress(&missing, &mut |n| {
            seen.push(n);
            ControlFlow::Continue(())
        })
        .await
        .is_err()
    );
    assert_eq!(seen, Vec::<u64>::new());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stream_offers_the_file_s_hash_and_arrives_into_a_writer() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.bin");
    std::fs::write(&source, data(300_000)).unwrap();
    let from_file = Offered::import(&source).await.unwrap();
    let mut reported = Vec::new();
    let streamed = Offered::import_reader(
        Box::new(std::io::Cursor::new(data(300_000))),
        300_000,
        &mut |n| {
            reported.push(n);
            ControlFlow::Continue(())
        },
    )
    .await
    .unwrap();
    assert_eq!(streamed.hash(), from_file.hash());
    assert!(reported.is_sorted() && reported.last() <= Some(&300_000));
    from_file.close().await;

    let cache = dir.path().join("cache");
    let (mut upstream, mut downstream) = connected().await;
    upstream.provide(&streamed).await.unwrap();
    let fetched = within(downstream.fetch(&cache, 300_000, &mut |_| {}))
        .await
        .unwrap();
    let mut written = Vec::new();
    fetched.export_writer(&mut written).await.unwrap();
    assert_eq!(written, data(300_000));
    assert_eq!(std::fs::read_dir(&cache).unwrap().count(), 0);
    upstream.finish().await;
    downstream.finish().await;
    streamed.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stream_longer_or_shorter_than_its_size_or_stopped_is_not_offered() {
    for (len, size, says) in [(10, 4, "grew"), (2, 4, "shrank")] {
        let refused =
            Offered::import_reader(Box::new(std::io::Cursor::new(data(len))), size, &mut |_| {
                ControlFlow::Continue(())
            })
            .await;
        let said = refused.err().expect("refused").to_string();
        assert!(said.contains(says), "{said}");
    }
    let unreadable =
        Offered::import_reader(Box::new(Unreadable), 4, &mut |_| ControlFlow::Continue(())).await;
    let said = unreadable.err().expect("refused").to_string();
    assert!(said.contains("revoked"), "{said}");
    let stopped = Offered::import_reader(
        Box::new(std::io::Cursor::new(data(1 << 20))),
        1 << 20,
        &mut |_| ControlFlow::Break(()),
    )
    .await;
    assert!(stopped.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_writer_that_fails_keeps_what_arrived() {
    struct Broken;
    impl tokio::io::AsyncWrite for Broken {
        fn poll_write(
            self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
            _: &[u8],
        ) -> std::task::Poll<std::io::Result<usize>> {
            std::task::Poll::Ready(Err(std::io::Error::other("disk full")))
        }
        fn poll_flush(
            self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::task::Poll::Ready(Ok(()))
        }
        fn poll_shutdown(
            self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::task::Poll::Ready(Ok(()))
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let cache = dir.path().join("cache");
    let offered = Offered::import_reader(
        Box::new(std::io::Cursor::new(data(50_000))),
        50_000,
        &mut |_| ControlFlow::Continue(()),
    )
    .await
    .unwrap();
    let (mut upstream, mut downstream) = connected().await;
    upstream.provide(&offered).await.unwrap();
    let fetched = within(downstream.fetch(&cache, 50_000, &mut |_| {}))
        .await
        .unwrap();
    let kept = fetched
        .export_writer(&mut Broken)
        .await
        .unwrap_err()
        .to_string();
    assert!(
        kept.contains("disk full") && kept.contains("kept in"),
        "{kept}"
    );
    assert_eq!(std::fs::read_dir(&cache).unwrap().count(), 1);
    upstream.abort().await;
    downstream.abort().await;
    offered.close().await;
}
