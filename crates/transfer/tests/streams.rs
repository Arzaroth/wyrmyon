use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use wyrmyon_testkit::MailboxServer;
use wyrmyon_transfer::protocol::{FileOffer, Offer};
use wyrmyon_transfer::{
    Cancelled, Events, Opened, Options, Quiet, Reader, Settled, Sink, Source, Staged, await_offer,
    receive_into, send_data,
};
use wyrmyon_transport_iroh::Relays;
use wyrmyon_wormhole::Mood;

struct Memory(Vec<u8>);

impl Source for Memory {
    fn size(&self) -> u64 {
        self.0.len() as u64
    }

    fn open(&self) -> Opened<'_> {
        let data = self.0.clone();
        Box::pin(async move { Ok(Box::new(std::io::Cursor::new(data)) as Reader) })
    }
}

struct Never;

impl tokio::io::AsyncRead for Never {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
        _: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Pending
    }
}

struct Stalling(Vec<u8>);

impl Source for Stalling {
    fn size(&self) -> u64 {
        2 * self.0.len() as u64
    }

    fn open(&self) -> Opened<'_> {
        let data = self.0.clone();
        Box::pin(async move {
            let stalled = tokio::io::AsyncReadExt::chain(std::io::Cursor::new(data), Never);
            Ok(Box::new(stalled) as Reader)
        })
    }
}

#[derive(Default)]
struct Counted(AtomicU64);

impl Events for Counted {
    fn moved(&self, bytes: u64) {
        self.0.fetch_max(bytes, Ordering::Relaxed);
    }
}

#[derive(Default)]
struct CancelOnData(tokio::sync::Notify);

impl Events for CancelOnData {
    fn moved(&self, bytes: u64) {
        if bytes > 0 {
            self.0.notify_one();
        }
    }

    fn cancelled(&self) -> Settled<'_> {
        Box::pin(self.0.notified())
    }
}

struct CancelAtOnce;

impl Events for CancelAtOnce {
    fn cancelled(&self) -> Settled<'_> {
        Box::pin(async {})
    }
}

fn options(server: &MailboxServer, force_classic: bool) -> Options {
    Options {
        relay_url: server.url(),
        transit_helper: "tcp:127.0.0.1:9".parse().unwrap(),
        no_listen: false,
        force_classic,
        force_iroh: false,
        iroh_relays: Relays::Disabled,
    }
}

fn data(len: usize) -> Vec<u8> {
    (0..len).map(|i| u8::try_from(i % 251).unwrap()).collect()
}

async fn within<T>(future: impl Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(60), future)
        .await
        .expect("timed out")
}

async fn stream_through(force_classic: bool) {
    let server = MailboxServer::start().await;
    let options = options(&server, force_classic);
    let cache = tempfile::tempdir().unwrap();
    let sent = data(400_000);
    let pending = wyrmyon_wormhole::create(&options.config(), 2)
        .await
        .unwrap();
    let code = pending.code().clone();

    let sender = {
        let (options, sent) = (options.clone(), sent.clone());
        tokio::spawn(async move {
            let mut wormhole = pending.pair().await.unwrap();
            let source: Arc<dyn Source> = Arc::new(Memory(sent));
            let events: Arc<dyn Events> = Arc::new(Quiet);
            let mut staged = Staged::default();
            let offer = Offer::File(FileOffer {
                filename: "photo.jpg".into(),
                filesize: source.size(),
            });
            let result = send_data(
                &mut wormhole,
                &options,
                &mut staged,
                offer,
                &source,
                &events,
            )
            .await;
            wormhole.close(Mood::Happy).await;
            staged.close().await;
            result
        })
    };

    let mut wormhole = wyrmyon_wormhole::join(&options.config(), code)
        .await
        .unwrap()
        .pair()
        .await
        .unwrap();
    let (offer, connection) = within(Box::pin(await_offer(&mut wormhole, &options)))
        .await
        .unwrap();
    let Offer::File(file) = offer else {
        panic!("a file was offered");
    };
    assert_eq!(
        (file.filename.as_str(), file.filesize),
        ("photo.jpg", 400_000)
    );
    let counted = Arc::new(Counted::default());
    let events: Arc<dyn Events> = counted.clone();
    let mut written = Vec::new();
    let received = within(receive_into(
        &mut wormhole,
        connection.unwrap(),
        file.filesize,
        Ok(cache.path().to_owned()),
        Sink::Writer(&mut written),
        &events,
    ))
    .await
    .unwrap();
    assert_eq!(received.digest.is_some(), force_classic);
    assert_eq!(received.traffic().is_none(), force_classic);
    within(received.complete(Ok(()))).await.unwrap();
    wormhole.close(Mood::Happy).await;
    let traffic = within(sender).await.unwrap().unwrap();
    assert_eq!(traffic.is_none(), force_classic);
    assert_eq!(written, sent);
    assert_eq!(counted.0.load(Ordering::Relaxed), 400_000);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stream_arrives_into_a_writer_over_classic_transit() {
    Box::pin(stream_through(true)).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stream_arrives_into_a_writer_over_iroh() {
    Box::pin(stream_through(false)).await;
}

async fn paired(
    options: &Options,
) -> (
    wyrmyon_wormhole::Wormhole,
    tokio::task::JoinHandle<wyrmyon_wormhole::Wormhole>,
) {
    let pending = wyrmyon_wormhole::create(&options.config(), 2)
        .await
        .unwrap();
    let code = pending.code().clone();
    let sender = tokio::spawn(async move { pending.pair().await.unwrap() });
    let receiver = wyrmyon_wormhole::join(&options.config(), code)
        .await
        .unwrap()
        .pair()
        .await
        .unwrap();
    (receiver, sender)
}

fn send_from(
    options: &Options,
    wormhole: tokio::task::JoinHandle<wyrmyon_wormhole::Wormhole>,
    source: Arc<dyn Source>,
    events: Arc<dyn Events>,
) -> tokio::task::JoinHandle<anyhow::Result<()>> {
    let options = options.clone();
    tokio::spawn(async move {
        let mut wormhole = wormhole.await.unwrap();
        let mut staged = Staged::default();
        let offer = Offer::File(FileOffer {
            filename: "photo.jpg".into(),
            filesize: source.size(),
        });
        let result = send_data(
            &mut wormhole,
            &options,
            &mut staged,
            offer,
            &source,
            &events,
        )
        .await;
        wormhole.close(Mood::Happy).await;
        staged.close().await;
        result.map(drop)
    })
}

async fn cancel_mid_stream(force_classic: bool) {
    let server = MailboxServer::start().await;
    let options = options(&server, force_classic);
    let cache = tempfile::tempdir().unwrap();
    let (mut wormhole, sender) = paired(&options).await;
    let source: Arc<dyn Source> = if force_classic {
        Arc::new(Stalling(data(1_000_000)))
    } else {
        Arc::new(Memory(data(4_000_000)))
    };
    let sender = send_from(&options, sender, source, Arc::new(Quiet));
    let (offer, connection) = within(Box::pin(await_offer(&mut wormhole, &options)))
        .await
        .unwrap();
    let Offer::File(file) = offer else {
        panic!("a file was offered");
    };
    let events: Arc<dyn Events> = Arc::new(CancelOnData::default());
    let mut written = Vec::new();
    let received = within(receive_into(
        &mut wormhole,
        connection.unwrap(),
        file.filesize,
        Ok(cache.path().to_owned()),
        Sink::Writer(&mut written),
        &events,
    ))
    .await;
    assert!(received.err().unwrap().is::<Cancelled>());
    wormhole.close(Mood::Happy).await;
    if force_classic {
        sender.abort();
    } else {
        assert!(within(sender).await.unwrap().is_err());
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_receiver_cancelling_mid_stream_ends_classic_transit() {
    Box::pin(cancel_mid_stream(true)).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_receiver_cancelling_mid_stream_ends_iroh() {
    Box::pin(cancel_mid_stream(false)).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_sender_cancelling_while_hashing_tells_the_receiver() {
    let server = MailboxServer::start().await;
    let options = options(&server, false);
    let (mut wormhole, sender) = paired(&options).await;
    let sender = send_from(
        &options,
        sender,
        Arc::new(Memory(data(4_000_000))),
        Arc::new(CancelAtOnce),
    );
    let awaited = within(Box::pin(await_offer(&mut wormhole, &options))).await;
    let error = format!("{:#}", awaited.err().unwrap());
    assert!(error.contains("the sender cancelled"), "{error}");
    wormhole.close(Mood::Happy).await;
    assert!(within(sender).await.unwrap().unwrap_err().is::<Cancelled>());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_sender_cancelling_before_the_answer_stops_waiting() {
    let server = MailboxServer::start().await;
    let options = options(&server, true);
    let (mut wormhole, sender) = paired(&options).await;
    let sender = send_from(
        &options,
        sender,
        Arc::new(Memory(data(1000))),
        Arc::new(CancelAtOnce),
    );
    let (offer, connection) = within(Box::pin(await_offer(&mut wormhole, &options)))
        .await
        .unwrap();
    assert!(matches!(offer, Offer::File(_)));
    assert!(within(sender).await.unwrap().unwrap_err().is::<Cancelled>());
    if let Some(connection) = connection {
        connection.close().await;
    }
    wormhole.close(Mood::Happy).await;
}
