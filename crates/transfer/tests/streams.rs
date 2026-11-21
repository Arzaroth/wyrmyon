use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use wyrmyon_testkit::MailboxServer;
use wyrmyon_transfer::protocol::{FileOffer, Offer};
use wyrmyon_transfer::{
    Events, Opened, Options, Quiet, Reader, Sink, Source, Staged, await_offer, receive_into,
    send_data,
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

#[derive(Default)]
struct Counted(AtomicU64);

impl Events for Counted {
    fn moved(&self, bytes: u64) {
        self.0.fetch_max(bytes, Ordering::Relaxed);
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
