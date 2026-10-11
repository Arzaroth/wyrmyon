use std::path::Path;
use std::time::Duration;

use wyrmyon_transport_iroh::{IrohPipe, IrohTransport, Manifest, Offered, Relays, Role, Source};
use wyrmyon_wormhole::Key;

async fn within<T>(future: impl Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(60), future)
        .await
        .expect("timed out")
}

async fn connected() -> (IrohPipe, IrohPipe) {
    let key = Key::from_bytes([5; 32]);
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
    (0..len).map(|i| u8::try_from(i % 251).unwrap()).collect()
}

fn sources(dir: &Path, files: &[(&str, Vec<u8>)]) -> Vec<Source> {
    files
        .iter()
        .map(|(name, contents)| {
            let path = dir.join(name.replace('/', "_"));
            std::fs::write(&path, contents).unwrap();
            Source {
                path,
                name: (*name).to_owned(),
                mode: 0o640,
            }
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_collection_arrives_file_by_file_and_the_cache_is_cleared() {
    let dir = tempfile::tempdir().unwrap();
    let files = [
        ("a.txt", b"alpha".to_vec()),
        ("sub/b.bin", data(300_000)),
        ("sub/same.bin", data(300_000)),
        ("empty", Vec::new()),
    ];
    let sources = sources(dir.path(), &files);
    let (offered, manifest) = Offered::import_collection(&sources, &["sub/void".into()])
        .await
        .unwrap();
    assert_eq!(manifest.numbytes(), 600_005);
    let cache = dir.path().join("cache");

    let (mut upstream, mut downstream) = connected().await;
    upstream.provide(&offered).await.unwrap();
    let collection = within(downstream.fetch_manifest(&cache, 600_005, 4))
        .await
        .unwrap();
    assert_eq!(collection.manifest(), &manifest);
    let mut seen = 0;
    let collection = within(collection.fetch(&downstream, &mut |n| seen = n))
        .await
        .unwrap();
    assert_eq!(seen, 600_005);
    let out = dir.path().join("out");
    std::fs::create_dir(&out).unwrap();
    collection.export_into(&out).await.unwrap();
    for (name, contents) in &files {
        assert_eq!(&std::fs::read(out.join(name)).unwrap(), contents, "{name}");
    }
    assert!(out.join("sub/void").is_dir());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(out.join("a.txt"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o640);
    }
    assert_eq!(std::fs::read_dir(&cache).unwrap().count(), 0);
    upstream.abort().await;
    downstream.abort().await;
    offered.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_list_that_does_not_match_the_offer_is_refused_and_dropped() {
    let dir = tempfile::tempdir().unwrap();
    let sources = sources(dir.path(), &[("a", data(10)), ("b", data(20))]);
    let (offered, _) = Offered::import_collection(&sources, &[]).await.unwrap();
    for (numbytes, numfiles) in [(30, 3), (30, 1), (29, 2), (31, 2), (30, 100_000)] {
        let cache = dir.path().join(format!("cache-{numbytes}-{numfiles}"));
        let (mut upstream, mut downstream) = connected().await;
        upstream.provide(&offered).await.unwrap();
        let refused = within(downstream.fetch_manifest(&cache, numbytes, numfiles)).await;
        assert!(refused.is_err(), "{numbytes} {numfiles}");
        assert_eq!(std::fs::read_dir(&cache).unwrap().count(), 0);
        upstream.abort().await;
        downstream.abort().await;
    }
    offered.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_list_refused_by_the_receiver_is_dropped() {
    let dir = tempfile::tempdir().unwrap();
    let sources = sources(dir.path(), &[("a", data(10))]);
    let (offered, _) = Offered::import_collection(&sources, &[]).await.unwrap();
    let cache = dir.path().join("cache");
    let (mut upstream, mut downstream) = connected().await;
    upstream.provide(&offered).await.unwrap();
    let collection = within(downstream.fetch_manifest(&cache, 10, 1))
        .await
        .unwrap();
    let refused = collection.refuse("no").await.to_string();
    assert!(refused.contains("no"), "{refused}");
    assert_eq!(std::fs::read_dir(&cache).unwrap().count(), 0);
    upstream.abort().await;
    downstream.abort().await;
    offered.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_interrupted_collection_resumes_from_what_is_cached() {
    const SIZE: usize = 32 << 20;
    let dir = tempfile::tempdir().unwrap();
    let files = [("one", data(SIZE)), ("two", data(SIZE + 7))];
    let sources = sources(dir.path(), &files);
    let (offered, manifest) = Offered::import_collection(&sources, &[]).await.unwrap();
    let total = manifest.numbytes();
    let cache = dir.path().join("cache");

    let (mut upstream, mut downstream) = connected().await;
    upstream.provide(&offered).await.unwrap();
    let collection = within(downstream.fetch_manifest(&cache, total, 2))
        .await
        .unwrap();
    let (cut_tx, cut_rx) = std::sync::mpsc::channel::<()>();
    let cutter = tokio::spawn(async move {
        tokio::task::spawn_blocking(move || cut_rx.recv())
            .await
            .unwrap()
            .unwrap();
        upstream.abort().await;
    });
    let mut cut_tx = Some(cut_tx);
    let first = within(collection.fetch(&downstream, &mut move |n| {
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
    let collection = within(downstream.fetch_manifest(&cache, total, 2))
        .await
        .unwrap();
    let mut resumed_from = None;
    let collection = within(collection.fetch(&downstream, &mut |n| {
        resumed_from.get_or_insert(n);
    }))
    .await
    .unwrap();
    let resumed_from = resumed_from.unwrap();
    assert!(
        resumed_from > 0 && resumed_from < total,
        "resumed from {resumed_from}"
    );
    let out = dir.path().join("out");
    std::fs::create_dir(&out).unwrap();
    collection.export_into(&out).await.unwrap();
    for (name, contents) in &files {
        let same = &std::fs::read(out.join(name)).unwrap() == contents;
        assert!(same, "{name} differs from the source");
    }
    upstream.abort().await;
    downstream.abort().await;
    offered.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_sender_gone_before_the_list_arrives_keeps_the_cache_entry() {
    let dir = tempfile::tempdir().unwrap();
    let sources = sources(dir.path(), &[("a", data(10))]);
    let (offered, _) = Offered::import_collection(&sources, &[]).await.unwrap();
    let cache = dir.path().join("cache");
    let (mut upstream, mut downstream) = connected().await;
    upstream.send_last(&offered.hash()).await.unwrap();
    let gone = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(500)).await;
        upstream.abort().await;
    });
    let kept = within(downstream.fetch_manifest(&cache, 10, 1))
        .await
        .err()
        .expect("nothing was served")
        .to_string();
    assert!(kept.contains("kept in"), "{kept}");
    within(gone).await.unwrap();
    downstream.abort().await;
    offered.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_export_keeps_the_verified_collection() {
    let dir = tempfile::tempdir().unwrap();
    let sources = sources(dir.path(), &[("a", data(5_000))]);
    let (offered, _) = Offered::import_collection(&sources, &[]).await.unwrap();
    let cache = dir.path().join("cache");
    let (mut upstream, mut downstream) = connected().await;
    upstream.provide(&offered).await.unwrap();
    let collection = within(downstream.fetch_manifest(&cache, 5_000, 1))
        .await
        .unwrap();
    let collection = within(collection.fetch(&downstream, &mut |_| {}))
        .await
        .unwrap();
    let out = dir.path().join("out");
    std::fs::create_dir_all(out.join("a")).unwrap();
    let kept = collection.export_into(&out).await.unwrap_err().to_string();
    assert!(kept.contains("kept in"), "{kept}");
    assert_eq!(std::fs::read_dir(&cache).unwrap().count(), 1);
    upstream.abort().await;
    downstream.abort().await;
    offered.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_collection_with_a_missing_file_is_not_offered() {
    let dir = tempfile::tempdir().unwrap();
    let missing = Source {
        path: dir.path().join("missing"),
        name: "missing".into(),
        mode: 0o644,
    };
    assert!(Offered::import_collection(&[missing], &[]).await.is_err());
}

#[test]
fn an_empty_manifest_has_no_bytes() {
    let manifest = Manifest {
        files: Vec::new(),
        dirs: Vec::new(),
    };
    assert_eq!(manifest.numbytes(), 0);
}
