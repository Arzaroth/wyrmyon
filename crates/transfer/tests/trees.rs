use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use wyrmyon_testkit::MailboxServer;
use wyrmyon_transfer::protocol::{DirectoryOffer, Offer};
use wyrmyon_transfer::{
    Cancelled, Events, FileSource, Member, Opened, Opening, Options, Quiet, Reader, Settled,
    Source, Staged, Tree, Writer, ZIP_MODE, Zipped, await_offer, receive_tree, send_data,
    send_tree, usable_path, zip_tree,
};
use wyrmyon_transport_iroh::Relays;
use wyrmyon_wormhole::{Mood, Wormhole};

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

fn data(len: usize, seed: u8) -> Vec<u8> {
    (0..len)
        .map(|i| u8::try_from(i % 251).unwrap() ^ seed)
        .collect()
}

fn member(name: &str, data: Vec<u8>) -> Member {
    Member {
        name: name.into(),
        mode: 0o644,
        source: Arc::new(Memory(data)),
    }
}

fn photos(bundle: bool) -> Arc<Tree> {
    Arc::new(Tree {
        name: "photos".into(),
        bundle,
        members: vec![
            member("a.jpg", data(300_000, 1)),
            member("trip/b.jpg", data(200_000, 2)),
            member("trip/notes.txt", b"hello".to_vec()),
        ],
        dirs: vec!["empty".into()],
    })
}

async fn within<T>(future: impl Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(60), future)
        .await
        .expect("timed out")
}

async fn paired(options: &Options) -> (Wormhole, tokio::task::JoinHandle<Wormhole>) {
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

fn opener(dir: &Path) -> impl FnMut(&str, u32) -> Opening + Send + use<> {
    let dir = dir.to_owned();
    move |name: &str, _mode: u32| {
        let target = dir.join(name);
        Box::pin(async move {
            tokio::fs::create_dir_all(target.parent().unwrap()).await?;
            Ok(Box::new(tokio::fs::File::create(target).await?) as Writer)
        })
    }
}

fn read_tree(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut found = BTreeMap::new();
    let mut stack = vec![dir.to_owned()];
    while let Some(at) = stack.pop() {
        for entry in std::fs::read_dir(at).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let name = path.strip_prefix(dir).unwrap().to_string_lossy();
                found.insert(name.replace('\\', "/"), std::fs::read(&path).unwrap());
            }
        }
    }
    found
}

fn expected() -> BTreeMap<String, Vec<u8>> {
    BTreeMap::from([
        ("a.jpg".to_owned(), data(300_000, 1)),
        ("trip/b.jpg".to_owned(), data(200_000, 2)),
        ("trip/notes.txt".to_owned(), b"hello".to_vec()),
    ])
}

async fn tree_through(force_classic: bool) -> Offer {
    let server = MailboxServer::start().await;
    let options = options(&server, force_classic);
    let scratch = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    let (mut wormhole, sender) = paired(&options).await;
    let sender = {
        let (options, scratch) = (options.clone(), scratch.path().to_owned());
        tokio::spawn(async move {
            let mut wormhole = sender.await.unwrap();
            let (mut staged, mut zipped) = (Staged::default(), None);
            let events: Arc<dyn Events> = Arc::new(Quiet);
            let sent = send_tree(
                &mut wormhole,
                &options,
                &mut staged,
                &mut zipped,
                &photos(true),
                &scratch,
                &events,
            )
            .await;
            wormhole.close(Mood::Happy).await;
            staged.close().await;
            sent
        })
    };
    let (offer, connection) = within(Box::pin(await_offer(&mut wormhole, &options)))
        .await
        .unwrap();
    let events: Arc<dyn Events> = Arc::new(Quiet);
    let received = within(Box::pin(receive_tree(
        &mut wormhole,
        connection.unwrap(),
        &offer,
        cache.path(),
        &mut opener(out.path()),
        &events,
    )))
    .await
    .unwrap();
    within(received.complete(Ok(()))).await.unwrap();
    wormhole.close(Mood::Happy).await;
    within(sender).await.unwrap().unwrap();
    assert_eq!(read_tree(out.path()), expected());
    assert_eq!(std::fs::read_dir(scratch.path()).unwrap().count(), 0);
    offer
}

#[tokio::test(flavor = "multi_thread")]
async fn streams_arrive_as_a_collection_between_wyrmyon_peers() {
    let Offer::Collection(offer) = Box::pin(tree_through(false)).await else {
        panic!("a collection was offered");
    };
    assert_eq!((offer.numfiles, offer.numbytes), (3, 500_005));
    assert!(offer.bundle);
    assert_eq!(offer.names, ["a.jpg", "empty/", "trip/"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn streams_arrive_as_a_zip_over_classic_transit() {
    let Offer::Directory(offer) = Box::pin(tree_through(true)).await else {
        panic!("a directory was offered");
    };
    assert_eq!(offer.mode, ZIP_MODE);
    assert_eq!((offer.numfiles, offer.numbytes), (3, 500_005));
    assert!(!offer.bundle, "a legacy receiver is not told of bundles");
}

#[tokio::test]
async fn a_zip_of_streams_holds_every_member_and_empty_directory() {
    let scratch = tempfile::tempdir().unwrap();
    let zipped: Zipped = zip_tree(&photos(false), scratch.path(), &Quiet)
        .await
        .unwrap();
    assert_eq!((zipped.numfiles, zipped.numbytes), (3, 500_005));
    assert!(zipped.zipsize > 0);
    let cancelled = zip_tree(&photos(false), scratch.path(), &CancelAtOnce).await;
    assert!(cancelled.err().unwrap().is::<Cancelled>());
}

#[tokio::test]
async fn a_source_that_shrank_fails_the_zip() {
    struct Shrunk;
    impl Source for Shrunk {
        fn size(&self) -> u64 {
            10
        }
        fn open(&self) -> Opened<'_> {
            Box::pin(async { Ok(Box::new(std::io::Cursor::new(vec![1u8, 2])) as Reader) })
        }
    }
    let tree = Arc::new(Tree {
        name: "t".into(),
        bundle: false,
        members: vec![Member {
            name: "short".into(),
            mode: 0o644,
            source: Arc::new(Shrunk),
        }],
        dirs: Vec::new(),
    });
    let scratch = tempfile::tempdir().unwrap();
    let failed = zip_tree(&tree, scratch.path(), &Quiet).await.err().unwrap();
    assert!(
        format!("{failed:#}").contains("short changed"),
        "{failed:#}"
    );
}

#[test]
fn only_plain_relative_paths_are_usable() {
    assert!(!usable_path("invoice\u{202e}fdp.apk"));
    assert!(!usable_path("a\u{2066}b"));
    for good in ["a", "a/b.txt", "trip/2024/c d.jpg"] {
        assert!(usable_path(good), "{good}");
    }
    for bad in ["", "/a", "a/", "a//b", "../a", "a/./b", "a\\b", "bell\u{7}"] {
        assert!(!usable_path(bad), "{bad:?}");
    }
}

fn crafted_zip(dir: &Path, entries: &[(&str, &[u8])]) -> (PathBuf, u64) {
    use std::io::Write;
    let path = dir.join("crafted.zip");
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
    for (name, data) in entries {
        if name.ends_with('/') {
            zip.add_directory(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            continue;
        }
        zip.start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(data).unwrap();
    }
    zip.finish().unwrap();
    let size = std::fs::metadata(&path).unwrap().len();
    (path, size)
}

struct Crafted<'a> {
    entries: &'a [(&'a str, &'a [u8])],
    numbytes: u64,
    numfiles: u64,
    mode: &'a str,
    force_classic: bool,
}

async fn offer_crafted(
    entries: &[(&str, &[u8])],
    numbytes: u64,
    numfiles: u64,
    mode: &str,
) -> anyhow::Error {
    let out = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let crafted = Crafted {
        entries,
        numbytes,
        numfiles,
        mode,
        force_classic: true,
    };
    let (sent, received) = Box::pin(send_crafted(&crafted, cache.path(), out.path())).await;
    assert!(sent.is_err());
    received.expect_err("the zip was refused")
}

async fn send_crafted(
    crafted: &Crafted<'_>,
    cache: &Path,
    out: &Path,
) -> (anyhow::Result<()>, anyhow::Result<()>) {
    let Crafted {
        entries,
        numbytes,
        numfiles,
        mode,
        force_classic,
    } = *crafted;
    let server = MailboxServer::start().await;
    let options = options(&server, force_classic);
    let scratch = tempfile::tempdir().unwrap();
    let (zip, zipsize) = crafted_zip(scratch.path(), entries);
    let (mut wormhole, sender) = paired(&options).await;
    let offer = DirectoryOffer {
        mode: mode.into(),
        dirname: "crafted".into(),
        zipsize,
        numbytes,
        numfiles,
        bundle: false,
        names: Vec::new(),
    };
    let sender = {
        let options = options.clone();
        tokio::spawn(async move {
            let mut wormhole = sender.await.unwrap();
            let source: Arc<dyn Source> = Arc::new(FileSource::new(zip, zipsize));
            let events: Arc<dyn Events> = Arc::new(Quiet);
            let sent = send_data(
                &mut wormhole,
                &options,
                &mut Staged::default(),
                Offer::Directory(offer),
                &source,
                &events,
            )
            .await;
            wormhole.close(Mood::Happy).await;
            sent.map(drop)
        })
    };
    let (offer, connection) = within(Box::pin(await_offer(&mut wormhole, &options)))
        .await
        .unwrap();
    let events: Arc<dyn Events> = Arc::new(Quiet);
    let received = async {
        let received = Box::pin(receive_tree(
            &mut wormhole,
            connection.unwrap(),
            &offer,
            cache,
            &mut opener(out),
            &events,
        ))
        .await?;
        received.complete(Ok(())).await
    };
    let received = within(received).await;
    wormhole.close(Mood::Happy).await;
    (within(sender).await.unwrap(), received)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_zip_over_iroh_unpacks_into_a_cache_made_on_the_way() {
    let out = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let crafted = Crafted {
        entries: &[
            ("./", b""),
            ("a.txt", b"hello"),
            ("sub/", b""),
            ("sub/b.txt", b"there"),
        ],
        numbytes: 10,
        numfiles: 2,
        mode: ZIP_MODE,
        force_classic: false,
    };
    let (sent, received) = Box::pin(send_crafted(
        &crafted,
        &cache.path().join("not/yet"),
        out.path(),
    ))
    .await;
    received.unwrap();
    sent.unwrap();
    assert_eq!(
        read_tree(out.path()),
        BTreeMap::from([
            ("a.txt".to_owned(), b"hello".to_vec()),
            ("sub/b.txt".to_owned(), b"there".to_vec()),
        ])
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_zip_entry_outside_its_directory_is_refused() {
    let refused = Box::pin(offer_crafted(&[("../evil", b"x")], 1, 1, ZIP_MODE)).await;
    assert!(
        format!("{refused:#}").contains("outside its directory"),
        "{refused:#}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_zip_holding_more_than_offered_is_refused() {
    let bytes = Box::pin(offer_crafted(&[("a", b"0123456789")], 4, 1, ZIP_MODE)).await;
    assert!(
        format!("{bytes:#}").contains("more than the 4 bytes"),
        "{bytes:#}"
    );
    let files = Box::pin(offer_crafted(&[("a", b"0"), ("b", b"1")], 2, 1, ZIP_MODE)).await;
    assert!(
        format!("{files:#}").contains("more files than the 1"),
        "{files:#}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unknown_directory_mode_is_refused() {
    let refused = Box::pin(offer_crafted(&[("a", b"0")], 1, 1, "tarball")).await;
    assert!(format!("{refused:#}").contains("unknown directory transfer mode"));
}

struct Unreadable;

impl Source for Unreadable {
    fn size(&self) -> u64 {
        10
    }

    fn open(&self) -> Opened<'_> {
        Box::pin(async { Err(std::io::Error::other("permission denied")) })
    }
}

fn single(name: &str, source: Arc<dyn Source>) -> Arc<Tree> {
    Arc::new(Tree {
        name: "t".into(),
        bundle: false,
        members: vec![Member {
            name: name.into(),
            mode: 0o644,
            source,
        }],
        dirs: Vec::new(),
    })
}

fn failing_writer() -> impl FnMut(&str, u32) -> Opening + Send {
    |_: &str, _: u32| -> Opening { Box::pin(async { Err(std::io::Error::other("disk full")) }) }
}

async fn exchange(
    force_classic: bool,
    tree: Arc<Tree>,
    open: &mut (dyn FnMut(&str, u32) -> Opening + Send),
) -> (anyhow::Result<()>, anyhow::Result<()>) {
    let server = MailboxServer::start().await;
    let options = options(&server, force_classic);
    let cache = tempfile::tempdir().unwrap();
    let (mut wormhole, sender) = paired(&options).await;
    let sender = {
        let options = options.clone();
        tokio::spawn(async move {
            let mut wormhole = sender.await.unwrap();
            let scratch = tempfile::tempdir().unwrap();
            let (mut staged, mut zipped) = (Staged::default(), None);
            let events: Arc<dyn Events> = Arc::new(Quiet);
            let sent = send_tree(
                &mut wormhole,
                &options,
                &mut staged,
                &mut zipped,
                &tree,
                scratch.path(),
                &events,
            )
            .await;
            wormhole.close(Mood::Happy).await;
            staged.close().await;
            sent.map(drop)
        })
    };
    let received = async {
        let (offer, connection) = Box::pin(await_offer(&mut wormhole, &options)).await?;
        let events: Arc<dyn Events> = Arc::new(Quiet);
        let received = Box::pin(receive_tree(
            &mut wormhole,
            connection.unwrap(),
            &offer,
            cache.path(),
            open,
            &events,
        ))
        .await?;
        received.complete(Ok(())).await
    };
    let received = within(received).await;
    wormhole.close(Mood::Happy).await;
    (within(sender).await.unwrap(), received)
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unreadable_member_tells_the_receiver_on_either_way() {
    for force_classic in [false, true] {
        let out = tempfile::tempdir().unwrap();
        let tree = single("locked.txt", Arc::new(Unreadable));
        let (sent, received) =
            Box::pin(exchange(force_classic, tree, &mut opener(out.path()))).await;
        assert!(format!("{:#}", sent.unwrap_err()).contains("permission denied"));
        let told = format!("{:#}", received.unwrap_err());
        assert!(told.contains("could not read its data"), "{told}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_member_name_unusable_here_is_refused() {
    let out = tempfile::tempdir().unwrap();
    let tree = single("bell\u{7}.txt", Arc::new(Memory(b"ding".to_vec())));
    let (sent, received) = Box::pin(exchange(false, tree, &mut opener(out.path()))).await;
    assert!(sent.is_err());
    let refused = format!("{:#}", received.unwrap_err());
    assert!(refused.contains("cannot be written here"), "{refused}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_sink_that_fails_fails_the_transfer_on_either_way() {
    for force_classic in [false, true] {
        let (sent, received) = Box::pin(exchange(
            force_classic,
            photos(false),
            &mut failing_writer(),
        ))
        .await;
        assert!(sent.is_err());
        let failed = format!("{:#}", received.unwrap_err());
        assert!(failed.contains("disk full"), "{failed}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_single_file_offer_is_not_a_tree() {
    let server = MailboxServer::start().await;
    let options = options(&server, true);
    let (mut wormhole, sender) = paired(&options).await;
    let sender = {
        let options = options.clone();
        tokio::spawn(async move {
            let mut wormhole = sender.await.unwrap();
            let source: Arc<dyn Source> = Arc::new(Memory(b"x".to_vec()));
            let events: Arc<dyn Events> = Arc::new(Quiet);
            let offer = Offer::File(wyrmyon_transfer::protocol::FileOffer {
                filename: "x".into(),
                filesize: 1,
            });
            let sent = send_data(
                &mut wormhole,
                &options,
                &mut Staged::default(),
                offer,
                &source,
                &events,
            )
            .await;
            wormhole.close(Mood::Happy).await;
            sent
        })
    };
    let (offer, connection) = within(Box::pin(await_offer(&mut wormhole, &options)))
        .await
        .unwrap();
    let out = tempfile::tempdir().unwrap();
    let events: Arc<dyn Events> = Arc::new(Quiet);
    let refused = within(Box::pin(receive_tree(
        &mut wormhole,
        connection.unwrap(),
        &offer,
        out.path(),
        &mut opener(out.path()),
        &events,
    )))
    .await;
    assert!(format!("{:#}", refused.err().unwrap()).contains("not a directory"));
    wormhole.close(Mood::Happy).await;
    assert!(within(sender).await.unwrap().is_err());
}
