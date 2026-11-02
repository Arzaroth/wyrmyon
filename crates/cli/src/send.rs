use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context, anyhow, bail};
use clap::Args;
use tokio::io::AsyncReadExt;
use wyrmyon_transport_classic::{Role, Transit, TransitInfo};
use wyrmyon_transport_iroh::{IrohTransport, Manifest, Offered, Role as IrohRole, Source};
use wyrmyon_wormhole::{Code, Wormhole};

use crate::protocol::{
    self, Answer, AppMessage, CollectionOffer, DirectoryOffer, FileOffer, Offer,
};
use crate::transfer::Pipe;
use crate::{CODE_LENGTH, Global, mood_for, printable, show_code, show_welcome, transfer, zipdir};

#[derive(Args)]
#[command(group = clap::ArgGroup::new("what").required(true).args(["text", "paths"]))]
pub struct SendArgs {
    /// Text to send; `-` reads it from stdin
    #[arg(long)]
    pub(crate) text: Option<String>,
    /// Files or directories to send; several travel as one bundle named `files`
    pub(crate) paths: Vec<PathBuf>,
    /// Number of words in the generated code
    #[arg(long, default_value_t = CODE_LENGTH, value_parser = clap::value_parser!(u8).range(1..=8))]
    pub(crate) code_length: u8,
    /// Use this code instead of generating one, e.g. the one a receiver chose
    #[arg(long, conflicts_with = "code_length")]
    pub(crate) code: Option<String>,
    /// Also show the code as a QR code, for a phone to scan
    #[arg(long)]
    pub(crate) qr: bool,
    /// Serve this many receivers with the same code, one after another
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u32).range(1..))]
    pub(crate) times: u32,
    #[arg(skip)]
    pub(crate) literal_text: bool,
    #[arg(skip)]
    pub(crate) as_wormhole: bool,
}

enum Payload {
    Text(String),
    File(PathBuf, FileOffer),
    Tree(String, Arc<Vec<zipdir::Entry>>),
}

#[derive(Default)]
struct Staged {
    zip: Option<zipdir::Built>,
    blob: Option<Offered>,
    collection: Option<(Offered, CollectionOffer)>,
    blob_ahead: Option<Hashing<Offered>>,
    collection_ahead: Option<Hashing<(Offered, Manifest)>>,
}

impl Staged {
    fn hash_ahead(&mut self, payload: &Payload) {
        match payload {
            Payload::Text(_) => {}
            Payload::File(path, offer) => {
                self.blob_ahead = Some(Hashing::blob(path.clone(), offer.filesize));
            }
            Payload::Tree(_, entries) => {
                self.collection_ahead = Some(Hashing::collection(entries));
            }
        }
    }

    async fn drop_blob_ahead(&mut self) {
        if let Some(ahead) = self.blob_ahead.take()
            && let Some(offered) = ahead.abort().await
        {
            offered.close().await;
        }
    }

    async fn drop_collection_ahead(&mut self) {
        if let Some(ahead) = self.collection_ahead.take()
            && let Some((offered, _)) = ahead.abort().await
        {
            offered.close().await;
        }
    }

    async fn close(mut self) {
        self.drop_blob_ahead().await;
        self.drop_collection_ahead().await;
        if let Some(blob) = self.blob {
            blob.close().await;
        }
        if let Some((offered, _)) = self.collection {
            offered.close().await;
        }
    }
}

struct Hashing<T> {
    hashed: Arc<AtomicU64>,
    total: u64,
    task: Option<tokio::task::JoinHandle<Result<T, wyrmyon_transport_iroh::Error>>>,
}

impl<T: Send + 'static> Hashing<T> {
    fn start<F>(total: u64, import: impl FnOnce(Box<dyn FnMut(u64) + Send>) -> F) -> Self
    where
        F: Future<Output = Result<T, wyrmyon_transport_iroh::Error>> + Send + 'static,
    {
        let hashed = Arc::new(AtomicU64::new(0));
        let counter = Arc::clone(&hashed);
        let progress = Box::new(move |n| counter.store(n, Ordering::Relaxed));
        Self {
            hashed,
            total,
            task: Some(tokio::spawn(import(progress))),
        }
    }

    async fn finish(mut self, global: &Global) -> anyhow::Result<T> {
        let mut task = self
            .task
            .take()
            .expect("a hashing task runs until finished");
        if !task.is_finished() {
            eprintln!("Hashing..");
            let bar = transfer::progress(self.total, global.hide_progress, global.ascii, false);
            let mut tick = tokio::time::interval(std::time::Duration::from_millis(100));
            let joined = loop {
                tokio::select! {
                    joined = &mut task => break joined,
                    _ = tick.tick() => bar.set_position(self.hashed.load(Ordering::Relaxed)),
                }
            };
            bar.finish_and_clear();
            return Ok(joined.context("hashing the data")??);
        }
        Ok(task.await.context("hashing the data")??)
    }

    async fn abort(mut self) -> Option<T> {
        let task = self.task.take()?;
        task.abort();
        task.await.ok()?.ok()
    }
}

impl Hashing<Offered> {
    fn blob(path: PathBuf, total: u64) -> Self {
        Self::start(total, move |mut progress| async move {
            Offered::import_with_progress(&path, &mut progress).await
        })
    }
}

impl Hashing<(Offered, Manifest)> {
    fn collection(entries: &[zipdir::Entry]) -> Self {
        let (mut sources, mut dirs, mut total) = (Vec::new(), Vec::new(), 0);
        for entry in entries {
            match entry {
                zipdir::Entry::File {
                    source,
                    name,
                    mode,
                    size,
                } => {
                    total += size;
                    sources.push(Source {
                        path: source.clone(),
                        name: name.clone(),
                        mode: *mode,
                    });
                }
                zipdir::Entry::EmptyDir(dir) if dir.is_empty() => {}
                zipdir::Entry::EmptyDir(dir) => dirs.push(dir.clone()),
            }
        }
        Self::start(total, move |mut progress| async move {
            Offered::import_collection_with_progress(&sources, &dirs, &mut progress).await
        })
    }
}

impl<T> Drop for Hashing<T> {
    fn drop(&mut self) {
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}

impl SendArgs {
    pub fn of_paths(paths: Vec<PathBuf>) -> Self {
        Self {
            text: None,
            paths,
            code_length: CODE_LENGTH,
            code: None,
            qr: false,
            times: 1,
            literal_text: false,
            as_wormhole: false,
        }
    }

    pub fn of_stdin() -> Self {
        Self {
            text: Some("-".into()),
            paths: Vec::new(),
            code_length: CODE_LENGTH,
            code: None,
            qr: false,
            times: 1,
            literal_text: false,
            as_wormhole: false,
        }
    }
}

pub async fn run(global: &Global, args: SendArgs) -> anyhow::Result<()> {
    let code: Option<Code> = args.code.as_deref().map(str::parse).transpose()?;
    let payload = match args.text {
        Some(text) if text == "-" && !args.literal_text => Payload::Text(stdin_text().await?),
        Some(text) => Payload::Text(text),
        None => paths_payload(args.paths).await?,
    };

    let qr = args.qr.then_some(global.ascii);
    let times = args.times;
    let config = global.config();
    if times > 1 && code.as_ref().is_some_and(|code| !code.is_reusable()) {
        bail!(
            "--times needs a code on a nameplate of 7 digits or more, which the server never hands out; leave out --code to get one"
        );
    }
    let given = code.or_else(|| (times > 1).then(|| Code::reusable(usize::from(args.code_length))));
    let mut pending = if let Some(code) = given {
        show_code(&code, qr, &other_side(&code, args.as_wormhole));
        wyrmyon_wormhole::join(&config, code).await?
    } else {
        let pending = wyrmyon_wormhole::create(&config, usize::from(args.code_length)).await?;
        show_code(
            pending.code(),
            qr,
            &other_side(pending.code(), args.as_wormhole),
        );
        pending
    };
    show_welcome(pending.welcome());
    if times > 1 {
        eprintln!("This code serves {times} receivers, one after another. A wrong code ends it.");
    }

    let code = pending.code().clone();
    let mut staged = Staged::default();
    if !global.force_classic {
        staged.hash_ahead(&payload);
    }
    let (mut round, mut failed) = (1, 0);
    let outcome = loop {
        let mut wormhole = match pending.pair().await {
            Ok(wormhole) => wormhole,
            Err(e) if times > 1 => {
                break Err(anyhow!(e).context(format!("receiver {round} of {times}")));
            }
            Err(e) => break Err(e.into()),
        };
        let next = if round < times {
            Some(wyrmyon_wormhole::join(&config, code.clone()).await)
        } else {
            None
        };
        let result = deliver(&mut wormhole, global, &payload, &mut staged).await;
        wormhole.close(mood_for(&result)).await;
        if times == 1 {
            break result;
        }
        match result {
            Ok(()) => eprintln!("Receiver {round} of {times}: done"),
            Err(e) => {
                failed += 1;
                eprintln!(
                    "Receiver {round} of {times}: {}",
                    printable(&format!("{e:#}"))
                );
            }
        }
        round += 1;
        pending = match next {
            None if failed == 0 => break Ok(()),
            None => break Err(anyhow!("{failed} of {times} transfers failed")),
            Some(Ok(pending)) => pending,
            Some(Err(e)) => break Err(anyhow!(e).context(format!("receiver {round} of {times}"))),
        };
        eprintln!("Waiting for receiver {round} of {times}..");
    };
    staged.close().await;
    outcome
}

async fn deliver(
    wormhole: &mut Wormhole,
    global: &Global,
    payload: &Payload,
    staged: &mut Staged,
) -> anyhow::Result<()> {
    match payload {
        Payload::Text(text) => send_text(wormhole, text.clone()).await,
        Payload::File(path, offer) => {
            let size = offer.filesize;
            send_data(
                wormhole,
                global,
                staged,
                Offer::File(offer.clone()),
                path,
                size,
            )
            .await
        }
        Payload::Tree(name, entries) => send_tree(wormhole, global, staged, name, entries).await,
    }
}

fn other_side(code: &Code, as_wormhole: bool) -> String {
    if as_wormhole {
        format!("  wormhole receive {code}\n\n(or: wyrm {code})")
    } else {
        format!("  wyrm {code}\n\n(or: wormhole receive {code})")
    }
}

async fn stdin_text() -> anyhow::Result<String> {
    let mut text = String::new();
    tokio::io::stdin()
        .read_to_string(&mut text)
        .await
        .context("reading stdin")?;
    Ok(text)
}

enum Prepared {
    Iroh(IrohTransport),
    Classic(Transit),
}

impl Prepared {
    async fn after(self, sent: anyhow::Result<()>) -> anyhow::Result<Self> {
        match (sent, self) {
            (Ok(()), prepared) => Ok(prepared),
            (Err(e), Self::Iroh(iroh)) => {
                iroh.close().await;
                Err(e)
            }
            (Err(e), Self::Classic(_)) => Err(e),
        }
    }
}

async fn paths_payload(mut paths: Vec<PathBuf>) -> anyhow::Result<Payload> {
    if paths.len() == 1 {
        return path_payload(paths.remove(0)).await;
    }
    let entries = listed(move |cancel| zipdir::list_bundle(&paths, cancel)).await?;
    Ok(Payload::Tree("files".into(), Arc::new(entries)))
}

async fn listed(
    list: impl FnOnce(&zipdir::Cancel) -> anyhow::Result<Vec<zipdir::Entry>> + Send + 'static,
) -> anyhow::Result<Vec<zipdir::Entry>> {
    let cancel = zipdir::Cancel::default();
    let _cancel_on_drop = cancel.on_drop();
    tokio::task::spawn_blocking(move || list(&cancel))
        .await
        .context("reading the files to send")?
}

async fn path_payload(path: PathBuf) -> anyhow::Result<Payload> {
    let meta = tokio::fs::metadata(&path)
        .await
        .with_context(|| format!("cannot send {}", path.display()))?;
    let name = display_name(&path)?;
    if meta.is_dir() {
        let entries = listed(move |cancel| zipdir::list(&path, cancel)).await?;
        return Ok(Payload::Tree(name, Arc::new(entries)));
    }
    if !meta.is_file() {
        bail!("{} is not a regular file", path.display());
    }
    eprintln!("Sending {} bytes file named '{name}'", meta.len());
    let offer = FileOffer {
        filename: name,
        filesize: meta.len(),
    };
    Ok(Payload::File(path, offer))
}

pub fn display_name(path: &Path) -> anyhow::Result<String> {
    let resolved =
        std::fs::canonicalize(path).with_context(|| format!("cannot send {}", path.display()))?;
    Ok(resolved
        .file_name()
        .with_context(|| format!("{} has no name", path.display()))?
        .to_string_lossy()
        .into_owned())
}

async fn send_text(wormhole: &mut Wormhole, text: String) -> anyhow::Result<()> {
    protocol::send(wormhole, &AppMessage::Offer(Offer::Message(text))).await?;
    loop {
        if let AppMessage::Answer(answer) = protocol::next(wormhole, "receiver").await? {
            if answer == Answer::MessageAck("ok".into()) {
                eprintln!("text message sent");
                return Ok(());
            }
            bail!("unexpected answer from the receiver: {answer:?}");
        }
    }
}

async fn require_iroh(wormhole: &mut Wormhole, global: &Global) -> anyhow::Result<bool> {
    let use_iroh = global.use_iroh(wormhole);
    if global.force_iroh && !use_iroh {
        protocol::send_error(wormhole, "the sender requires iroh-v1").await?;
        bail!("the other side does not speak iroh-v1 (--force-iroh)");
    }
    Ok(use_iroh)
}

async fn send_tree(
    wormhole: &mut Wormhole,
    global: &Global,
    staged: &mut Staged,
    name: &str,
    entries: &Arc<Vec<zipdir::Entry>>,
) -> anyhow::Result<()> {
    if global.use_collection(wormhole) {
        return send_collection(wormhole, global, staged, name, entries).await;
    }
    staged.drop_collection_ahead().await;
    require_iroh(wormhole, global).await?;
    if staged.zip.is_none() {
        eprintln!("Building zipfile..");
        let cancel = zipdir::Cancel::default();
        let _cancel_on_drop = cancel.on_drop();
        let entries = Arc::clone(entries);
        let zipped = tokio::task::spawn_blocking(move || zipdir::zip(&entries, &cancel))
            .await
            .context("building the zip file")
            .and_then(|built| built);
        match zipped {
            Ok(built) => staged.zip = Some(built),
            Err(e) => {
                protocol::send_error(wormhole, "the sender could not read its data").await?;
                return Err(e);
            }
        }
    }
    let built = staged.zip.as_ref().expect("the zip file was just built");
    eprintln!(
        "Sending directory ({} bytes compressed) named '{name}'",
        built.zipsize
    );
    let offer = DirectoryOffer {
        mode: "zipfile/deflated".into(),
        dirname: name.to_owned(),
        zipsize: built.zipsize,
        numbytes: built.numbytes,
        numfiles: built.numfiles,
    };
    let size = offer.zipsize;
    let path = built.file.path().to_owned();
    send_data(
        wormhole,
        global,
        staged,
        Offer::Directory(offer),
        &path,
        size,
    )
    .await
}

async fn send_collection(
    wormhole: &mut Wormhole,
    global: &Global,
    staged: &mut Staged,
    name: &str,
    entries: &[zipdir::Entry],
) -> anyhow::Result<()> {
    if staged.collection.is_none() {
        let hashing = staged
            .collection_ahead
            .take()
            .unwrap_or_else(|| Hashing::collection(entries));
        let (offered, manifest) = match hashing.finish(global).await {
            Ok(imported) => imported,
            Err(e) => {
                protocol::send_error(wormhole, "the sender could not read its data").await?;
                return Err(e);
            }
        };
        let offer = CollectionOffer {
            name: name.to_owned(),
            numbytes: manifest.numbytes(),
            numfiles: manifest.files.len() as u64,
        };
        staged.collection = Some((offered, offer));
    }
    let (offered, offer) = staged
        .collection
        .as_ref()
        .expect("the collection was just imported");
    eprintln!(
        "Sending directory ({} files, {} bytes) named '{}'",
        offer.numfiles, offer.numbytes, offer.name
    );
    send_over(
        wormhole,
        global,
        Offer::Collection(offer.clone()),
        None,
        offer.numbytes,
        Some(offered),
    )
    .await
}

async fn send_data(
    wormhole: &mut Wormhole,
    global: &Global,
    staged: &mut Staged,
    offer: Offer,
    path: &Path,
    size: u64,
) -> anyhow::Result<()> {
    let use_iroh = require_iroh(wormhole, global).await?;
    if !use_iroh {
        staged.drop_blob_ahead().await;
    }
    if use_iroh && staged.blob.is_none() {
        let hashing = staged
            .blob_ahead
            .take()
            .unwrap_or_else(|| Hashing::blob(path.to_owned(), size));
        match hashing.finish(global).await {
            Ok(offered) => staged.blob = Some(offered),
            Err(e) => {
                protocol::send_error(wormhole, "the sender could not read its data").await?;
                return Err(e);
            }
        }
    }
    let offered = if use_iroh { staged.blob.as_ref() } else { None };
    send_over(wormhole, global, offer, Some(path), size, offered).await
}

async fn send_over(
    wormhole: &mut Wormhole,
    global: &Global,
    offer: Offer,
    path: Option<&Path>,
    size: u64,
    offered: Option<&Offered>,
) -> anyhow::Result<()> {
    let prepared = if offered.is_some() {
        let iroh = global.iroh_or_tell(wormhole, IrohRole::Sender).await?;
        let info = AppMessage::Iroh(iroh.info().await);
        Prepared::Iroh(iroh).after(protocol::send(wormhole, &info).await)
    } else {
        let transit = global.transit(Role::Sender, wormhole.transit_key()).await;
        let info = AppMessage::Transit(transit.info());
        Prepared::Classic(transit).after(protocol::send(wormhole, &info).await)
    }
    .await?;
    let prepared = prepared
        .after(protocol::send(wormhole, &AppMessage::Offer(offer)).await)
        .await?;

    let answered = async {
        let (mut their_iroh, mut their_transit) = (None, TransitInfo::default());
        loop {
            match protocol::next(wormhole, "receiver").await? {
                AppMessage::Iroh(info) => their_iroh = Some(info),
                AppMessage::Transit(info) => their_transit = info,
                AppMessage::Answer(Answer::FileAck(ack)) if ack == "ok" => {
                    return Ok((their_iroh, their_transit));
                }
                AppMessage::Answer(answer) => {
                    bail!("unexpected answer from the receiver: {answer:?}")
                }
                _ => {}
            }
        }
    }
    .await;
    let mut pipe = match (answered, prepared) {
        (Ok((Some(theirs), _)), Prepared::Iroh(iroh)) => {
            Pipe::Iroh(iroh.connect(&theirs, wormhole.key()).await?)
        }
        (Ok((_, theirs)), Prepared::Classic(transit)) => {
            Pipe::Classic(transit.connect(&theirs).await?)
        }
        (Ok(_), Prepared::Iroh(iroh)) => {
            iroh.close().await;
            bail!("the receiver accepted without an iroh address");
        }
        (Err(e), Prepared::Iroh(iroh)) => {
            iroh.close().await;
            return Err(e);
        }
        (Err(e), Prepared::Classic(_)) => return Err(e),
    };
    eprintln!("Sending ({})..", pipe.describe());
    let bar = transfer::progress(
        size,
        global.hide_progress,
        global.ascii,
        pipe.through_wormhole(),
    );
    pipe.follow_route(&bar);
    let sent = async {
        let digest = match &mut pipe {
            Pipe::Iroh(iroh) => {
                iroh.provide(offered.expect("iroh pipes carry a blob"))
                    .await?;
                transfer::follow_sent(iroh, &bar);
                None
            }
            Pipe::Classic(records) => {
                let path = path.expect("classic pipes carry a file");
                let mut source = tokio::fs::File::open(path)
                    .await
                    .with_context(|| format!("opening {}", path.display()))?;
                Some(transfer::send_stream(records, &mut source, size, &bar).await?)
            }
        };
        if digest.is_some() {
            eprintln!("Waiting for the receiver to confirm..");
        }
        let acked = transfer::await_ack(&mut pipe, digest.as_ref()).await;
        if acked.is_ok() {
            transfer::finale(&bar).await;
        }
        bar.finish_and_clear();
        acked
    }
    .await;
    if let Err(e) = sent {
        bar.finish_and_clear();
        pipe.abort().await;
        return Err(e);
    }
    let went = pipe.went();
    pipe.shutdown().await;
    eprintln!("Confirmation received. Transfer complete.");
    if let Some(went) = went {
        eprintln!("{went}");
    }
    Ok(())
}
