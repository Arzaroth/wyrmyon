use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Map, json};
use tokio::sync::mpsc;
use wyrmyon_mobile::api::types::{OfferInfo, OfferKind, Settings, Stage, Update};
use wyrmyon_mobile::{Cancel, Outgoing, Report, receive, send};
use wyrmyon_testkit::MailboxServer;
use wyrmyon_transfer::protocol::{self, AppMessage, DirectoryOffer, FileOffer, Offer};
use wyrmyon_transfer::{
    Cancelled, Events, Member, Opened, Opening, Options, Quiet, Reader, Source, Staged, Tree,
    Writer, receive_tree, send_data, send_tree,
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

fn settings(server: &MailboxServer) -> Settings {
    Settings {
        mailbox: server.url(),
        transit_helper: "tcp:127.0.0.1:9".into(),
        iroh_relays: "disabled".into(),
    }
}

fn classic(server: &MailboxServer) -> Options {
    Options {
        relay_url: server.url(),
        transit_helper: "tcp:127.0.0.1:9".parse().unwrap(),
        no_listen: false,
        force_classic: true,
        force_iroh: false,
        iroh_relays: Relays::Disabled,
    }
}

fn data(len: usize) -> Vec<u8> {
    (0..len).map(|i| u8::try_from(i % 251).unwrap()).collect()
}

fn file(len: usize) -> Outgoing {
    Outgoing::File {
        name: "photo.jpg".into(),
        source: Arc::new(Memory(data(len))),
    }
}

async fn within<T>(future: impl Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(60), future)
        .await
        .expect("timed out")
}

struct Seen {
    updates: Arc<Mutex<Vec<Update>>>,
    codes: mpsc::UnboundedReceiver<String>,
}

fn watching() -> (Report, Seen) {
    let updates = Arc::new(Mutex::new(Vec::new()));
    let (tx, codes) = mpsc::unbounded_channel();
    let kept = Arc::clone(&updates);
    let report: Report = Arc::new(move |update: Update| {
        if update.stage == Stage::Code {
            let _ = tx.send(update.detail.clone());
        }
        kept.lock().unwrap().push(update);
    });
    (report, Seen { updates, codes })
}

impl Seen {
    async fn code(&mut self) -> String {
        within(self.codes.recv()).await.unwrap()
    }

    fn stages(&self) -> Vec<Stage> {
        let mut stages: Vec<_> = self
            .updates
            .lock()
            .unwrap()
            .iter()
            .map(|u| u.stage)
            .collect();
        stages.dedup();
        stages
    }

    fn last(&self, stage: Stage) -> Update {
        self.updates
            .lock()
            .unwrap()
            .iter()
            .rfind(|u| u.stage == stage)
            .cloned()
            .unwrap_or_else(|| panic!("no {stage:?} update"))
    }
}

async fn peer(server: &MailboxServer, code: String) -> Wormhole {
    peer_with(server, code, classic(server)).await
}

async fn peer_with(_server: &MailboxServer, code: String, options: Options) -> Wormhole {
    let config = options.config();
    within(async {
        wyrmyon_wormhole::join(&config, code.parse().unwrap())
            .await
            .unwrap()
            .pair()
            .await
            .unwrap()
    })
    .await
}

#[tokio::test(flavor = "multi_thread")]
async fn the_phone_sends_a_file_to_the_code_it_scanned() {
    let server = MailboxServer::start().await;
    let settings = settings(&server);
    let cache = tempfile::tempdir().unwrap();
    let (report, mut computer) = watching();
    let receiving = {
        let settings = settings.clone();
        tokio::spawn(async move { receive(&settings, None, report, &Cancel::new()).await })
    };
    let code = computer.code().await;
    let (report, phone) = watching();
    let sending = tokio::spawn(async move {
        send(&settings, Some(code), file(300_000), report, &Cancel::new()).await
    });

    let incoming = within(receiving).await.unwrap().unwrap();
    let info = incoming.info();
    assert_eq!(
        (info.kind, info.name.as_str(), info.size),
        (OfferKind::File, "photo.jpg", 300_000)
    );
    let mut written = Vec::new();
    within(incoming.accept(&mut written, cache.path().to_owned()))
        .await
        .unwrap();
    within(sending).await.unwrap().unwrap();
    assert_eq!(written, data(300_000));

    assert_eq!(
        phone.stages(),
        [
            Stage::Paired,
            Stage::Hashing,
            Stage::Connected,
            Stage::Moving,
            Stage::Done
        ]
    );
    assert!(phone.last(Stage::Connected).detail.contains("iroh"));
    let moved = computer.last(Stage::Moving);
    assert_eq!((moved.bytes, moved.total), (300_000, 300_000));
    assert_eq!(computer.stages().last(), Some(&Stage::Done));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_legacy_sender_reaches_the_phone_over_classic_transit() {
    let server = MailboxServer::start().await;
    let options = classic(&server);
    let pending = wyrmyon_wormhole::create(&options.config(), 2)
        .await
        .unwrap();
    let code = pending.code().to_string();
    let sending = tokio::spawn(async move {
        let mut wormhole = pending.pair().await.unwrap();
        let source: Arc<dyn Source> = Arc::new(Memory(data(70_000)));
        let events: Arc<dyn Events> = Arc::new(Quiet);
        let mut staged = Staged::default();
        let offer = Offer::File(FileOffer {
            filename: "../notes.txt".into(),
            filesize: 70_000,
        });
        let sent = send_data(
            &mut wormhole,
            &options,
            &mut staged,
            offer,
            &source,
            &events,
        )
        .await;
        wormhole.close(Mood::Happy).await;
        sent
    });

    let cache = tempfile::tempdir().unwrap();
    let (report, phone) = watching();
    let incoming = within(Box::pin(receive(
        &settings(&server),
        Some(code),
        report,
        &Cancel::new(),
    )))
    .await
    .unwrap();
    assert_eq!(incoming.info().name, "notes.txt");
    let mut written = Vec::new();
    within(incoming.accept(&mut written, cache.path().to_owned()))
        .await
        .unwrap();
    assert!(within(sending).await.unwrap().unwrap().is_none());
    assert_eq!(written, data(70_000));
    assert!(!phone.last(Stage::Connected).detail.contains("iroh"));
}

#[tokio::test(flavor = "multi_thread")]
async fn text_arrives_whole_and_the_server_s_notice_is_told() {
    let mut welcome = Map::new();
    welcome.insert("motd".into(), json!("be nice"));
    let server = MailboxServer::start_with_welcome(welcome).await;
    let settings = settings(&server);
    let (report, mut phone) = watching();
    let receiving = {
        let settings = settings.clone();
        tokio::spawn(async move { receive(&settings, None, report, &Cancel::new()).await })
    };
    let code = phone.code().await;
    let (report, _) = watching();
    within(Box::pin(send(
        &settings,
        Some(code),
        Outgoing::Text("hello".into()),
        report,
        &Cancel::new(),
    )))
    .await
    .unwrap();
    let incoming = within(receiving).await.unwrap().unwrap();
    let info = incoming.info();
    assert_eq!(
        (info.kind, info.text.as_str(), info.size),
        (OfferKind::Text, "hello", 5)
    );
    assert_eq!(phone.last(Stage::Notice).detail, "be nice");
    assert_eq!(phone.stages().last(), Some(&Stage::Done));
    let cache = tempfile::tempdir().unwrap();
    within(incoming.accept(&mut Vec::new(), cache.path().to_owned()))
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_refused_file_tells_the_sender() {
    let server = MailboxServer::start().await;
    let settings = settings(&server);
    let (report, mut phone) = watching();
    let sending = {
        let settings = settings.clone();
        tokio::spawn(async move { send(&settings, None, file(10), report, &Cancel::new()).await })
    };
    let code = phone.code().await;
    let (report, _) = watching();
    let incoming = within(Box::pin(receive(
        &settings,
        Some(code),
        report,
        &Cancel::new(),
    )))
    .await
    .unwrap();
    within(incoming.refuse()).await.unwrap();
    let said = within(sending).await.unwrap().unwrap_err().to_string();
    assert!(said.contains("transfer rejected"), "{said}");
    assert!(!phone.stages().contains(&Stage::Done));
}

#[tokio::test(flavor = "multi_thread")]
async fn offers_without_a_connection_are_refused() {
    let server = MailboxServer::start().await;
    let folder = Offer::Directory(DirectoryOffer {
        mode: "zipfile/deflated".into(),
        dirname: "photos".into(),
        zipsize: 10,
        numbytes: 10,
        numfiles: 1,
        bundle: false,
        names: Vec::new(),
    });
    let bare = Offer::File(FileOffer {
        filename: "photo.jpg".into(),
        filesize: 10,
    });
    for offer in [folder, bare] {
        let says = "did not offer a connection";
        let (report, mut phone) = watching();
        let receiving = {
            let settings = settings(&server);
            tokio::spawn(async move { receive(&settings, None, report, &Cancel::new()).await })
        };
        let mut wormhole = peer(&server, phone.code().await).await;
        protocol::send(&mut wormhole, &AppMessage::Offer(offer))
            .await
            .unwrap();
        let refused = within(receiving).await.unwrap().err().expect("refused");
        assert!(refused.to_string().contains(says), "{refused}");
        let answer = within(protocol::next(&mut wormhole, "receiver")).await;
        assert!(answer.unwrap_err().to_string().contains(says));
        wormhole.close(Mood::Happy).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_peer_that_gives_up_before_offering_ends_the_wait() {
    let server = MailboxServer::start().await;
    let (report, mut phone) = watching();
    let receiving = {
        let settings = settings(&server);
        tokio::spawn(async move { receive(&settings, None, report, &Cancel::new()).await })
    };
    let mut wormhole = peer(&server, phone.code().await).await;
    protocol::send_error(&mut wormhole, "gave up")
        .await
        .unwrap();
    let said = within(receiving).await.unwrap().err().expect("ended");
    assert!(said.to_string().contains("gave up"), "{said}");
    wormhole.close(Mood::Happy).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_mistyped_code_or_server_goes_nowhere() {
    let server = MailboxServer::start().await;
    let (report, _) = watching();
    let said = send(
        &settings(&server),
        Some("guitarist".into()),
        file(1),
        report.clone(),
        &Cancel::new(),
    )
    .await
    .unwrap_err()
    .to_string();
    assert_eq!(said, "this is not a wormhole code");
    let nowhere = Settings {
        transit_helper: "nowhere".into(),
        ..settings(&server)
    };
    assert!(
        receive(&nowhere, None, report, &Cancel::new())
            .await
            .is_err()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn cancelling_while_waiting_for_the_peer_ends_at_once() {
    let server = MailboxServer::start().await;
    let settings = settings(&server);
    let (report, mut phone) = watching();
    let cancel = Cancel::new();
    let receiving = {
        let cancel = cancel.clone();
        tokio::spawn(async move { receive(&settings, None, report, &cancel).await })
    };
    phone.code().await;
    cancel.cancel();
    let refused = within(receiving).await.unwrap().err().unwrap();
    assert!(refused.is::<Cancelled>(), "{refused:#}");
    assert_eq!(refused.to_string(), "the transfer was cancelled");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_phone_cancelling_while_the_receiver_decides_tells_it() {
    let server = MailboxServer::start().await;
    let settings = settings(&server);
    let (report, mut phone) = watching();
    let cancel = Cancel::new();
    let sending = {
        let cancel = cancel.clone();
        tokio::spawn(async move { send(&settings, None, file(10), report, &cancel).await })
    };
    let code = phone.code().await;
    let mut computer = peer(&server, code).await;
    let options = classic(&server);
    let (offer, connection) = within(Box::pin(wyrmyon_transfer::await_offer(
        &mut computer,
        &options,
    )))
    .await
    .unwrap();
    assert!(matches!(offer, Offer::File(_)));
    cancel.cancel();
    let cancelled = within(sending).await.unwrap().err().unwrap();
    assert!(cancelled.is::<Cancelled>(), "{cancelled:#}");
    let told = within(protocol::next(&mut computer, "sender")).await;
    assert!(format!("{:#}", told.err().unwrap()).contains("transfer cancelled"));
    if let Some(connection) = connection {
        connection.close().await;
    }
    computer.close(Mood::Happy).await;
}

fn iroh(server: &MailboxServer) -> Options {
    Options {
        force_classic: false,
        ..classic(server)
    }
}

fn album(bundle: bool) -> Arc<Tree> {
    Arc::new(Tree {
        name: "album".into(),
        bundle,
        members: vec![
            Member {
                name: "a.jpg".into(),
                mode: 0o644,
                source: Arc::new(Memory(data(200_000))),
            },
            Member {
                name: "day 2/b.jpg".into(),
                mode: 0o644,
                source: Arc::new(Memory(data(1000))),
            },
        ],
        dirs: Vec::new(),
    })
}

fn into_dir(dir: &std::path::Path) -> impl FnMut(&str, u32) -> Opening + Send + use<> {
    let dir = dir.to_owned();
    move |name: &str, _| {
        let target = dir.join(name);
        Box::pin(async move {
            tokio::fs::create_dir_all(target.parent().unwrap()).await?;
            Ok(Box::new(tokio::fs::File::create(target).await?) as Writer)
        })
    }
}

fn assert_album(dir: &std::path::Path) {
    assert_eq!(std::fs::read(dir.join("a.jpg")).unwrap(), data(200_000));
    assert_eq!(std::fs::read(dir.join("day 2/b.jpg")).unwrap(), data(1000));
}

async fn tree_to_phone(options: Options, bundle: bool) -> OfferInfo {
    let server_options = options;
    let (report, mut phone) = watching();
    let settings = Settings {
        mailbox: server_options.relay_url.clone(),
        transit_helper: "tcp:127.0.0.1:9".into(),
        iroh_relays: "disabled".into(),
    };
    let receiving =
        tokio::spawn(async move { receive(&settings, None, report, &Cancel::new()).await });
    let code = phone.code().await;
    let config = server_options.config();
    let sending = tokio::spawn(async move {
        let mut wormhole = wyrmyon_wormhole::join(&config, code.parse().unwrap())
            .await
            .unwrap()
            .pair()
            .await
            .unwrap();
        let scratch = tempfile::tempdir().unwrap();
        let (mut staged, mut zipped) = (Staged::default(), None);
        let events: Arc<dyn Events> = Arc::new(Quiet);
        let sent = send_tree(
            &mut wormhole,
            &server_options,
            &mut staged,
            &mut zipped,
            &album(bundle),
            scratch.path(),
            &events,
        )
        .await;
        wormhole.close(Mood::Happy).await;
        staged.close().await;
        sent
    });
    let incoming = within(receiving).await.unwrap().unwrap();
    let info = incoming.info();
    let (cache, out) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    within(Box::pin(incoming.accept_tree(
        &mut into_dir(out.path()),
        cache.path().to_owned(),
    )))
    .await
    .unwrap();
    within(sending).await.unwrap().unwrap();
    assert_album(out.path());
    assert_eq!(phone.stages().last(), Some(&Stage::Done));
    info
}

#[tokio::test(flavor = "multi_thread")]
async fn a_folder_from_a_wyrm_arrives_as_a_collection() {
    let server = MailboxServer::start().await;
    let info = Box::pin(tree_to_phone(iroh(&server), false)).await;
    assert_eq!(
        (info.kind, info.name.as_str(), info.count, info.size),
        (OfferKind::Folder, "album", 2, 201_000)
    );
    assert!(info.iroh, "a wyrm offers over iroh");
}

#[tokio::test(flavor = "multi_thread")]
async fn files_from_a_legacy_sender_arrive_from_its_zip() {
    let server = MailboxServer::start().await;
    let info = Box::pin(tree_to_phone(classic(&server), false)).await;
    assert_eq!((info.kind, info.count), (OfferKind::Folder, 2));
    assert!(!info.iroh, "a legacy sender offers over classic transit");
}

#[tokio::test(flavor = "multi_thread")]
async fn several_files_are_offered_as_a_bundle() {
    let server = MailboxServer::start().await;
    let info = Box::pin(tree_to_phone(iroh(&server), true)).await;
    assert_eq!((info.kind, info.count), (OfferKind::Files, 2));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_phone_sends_a_folder_to_a_wyrm() {
    let server = MailboxServer::start().await;
    let settings = settings(&server);
    let (report, mut phone) = watching();
    let scratch = tempfile::tempdir().unwrap();
    let outgoing = Outgoing::Tree {
        tree: album(false),
        scratch: scratch.path().to_owned(),
    };
    let sending =
        tokio::spawn(async move { send(&settings, None, outgoing, report, &Cancel::new()).await });
    let mut computer = peer_with(&server, phone.code().await, iroh(&server)).await;
    let options = iroh(&server);
    let (offer, connection) = within(Box::pin(wyrmyon_transfer::await_offer(
        &mut computer,
        &options,
    )))
    .await
    .unwrap();
    assert!(matches!(offer, Offer::Collection(_)), "{offer:?}");
    let (cache, out) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let events: Arc<dyn Events> = Arc::new(Quiet);
    let received = within(Box::pin(receive_tree(
        &mut computer,
        connection.unwrap(),
        &offer,
        cache.path(),
        &mut into_dir(out.path()),
        &events,
    )))
    .await
    .unwrap();
    within(received.complete(Ok(()))).await.unwrap();
    computer.close(Mood::Happy).await;
    within(sending).await.unwrap().unwrap();
    assert_album(out.path());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_folder_accepted_as_a_file_is_refused() {
    let server = MailboxServer::start().await;
    let (report, mut phone) = watching();
    let receiving = {
        let settings = settings(&server);
        tokio::spawn(async move { receive(&settings, None, report, &Cancel::new()).await })
    };
    let code = phone.code().await;
    let options = classic(&server);
    let sending = tokio::spawn(async move {
        let config = options.config();
        let mut wormhole = wyrmyon_wormhole::join(&config, code.parse().unwrap())
            .await
            .unwrap()
            .pair()
            .await
            .unwrap();
        let scratch = tempfile::tempdir().unwrap();
        let events: Arc<dyn Events> = Arc::new(Quiet);
        let sent = send_tree(
            &mut wormhole,
            &options,
            &mut Staged::default(),
            &mut None,
            &album(false),
            scratch.path(),
            &events,
        )
        .await;
        wormhole.close(Mood::Happy).await;
        sent
    });
    let incoming = within(receiving).await.unwrap().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let mut sink = Vec::new();
    let refused = within(incoming.accept(&mut sink, cache.path().to_owned())).await;
    assert!(
        refused
            .unwrap_err()
            .to_string()
            .contains("accepted another way")
    );
    let told = within(sending).await.unwrap().unwrap_err();
    assert!(
        format!("{told:#}").contains("transfer rejected"),
        "{told:#}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_file_accepted_as_a_folder_is_refused() {
    let server = MailboxServer::start().await;
    let (report, mut phone) = watching();
    let receiving = {
        let settings = settings(&server);
        tokio::spawn(async move { receive(&settings, None, report, &Cancel::new()).await })
    };
    let code = phone.code().await;
    let options = classic(&server);
    let sending = tokio::spawn(async move {
        let mut wormhole = wyrmyon_wormhole::join(&options.config(), code.parse().unwrap())
            .await
            .unwrap()
            .pair()
            .await
            .unwrap();
        let source: Arc<dyn Source> = Arc::new(Memory(data(10)));
        let events: Arc<dyn Events> = Arc::new(Quiet);
        let offer = Offer::File(FileOffer {
            filename: "photo.jpg".into(),
            filesize: 10,
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
    });
    let incoming = within(receiving).await.unwrap().unwrap();
    let (cache, out) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let refused = within(Box::pin(
        incoming.accept_tree(&mut into_dir(out.path()), cache.path().to_owned()),
    ))
    .await;
    assert!(
        refused
            .unwrap_err()
            .to_string()
            .contains("accepted another way")
    );
    assert!(within(sending).await.unwrap().is_err());
}
