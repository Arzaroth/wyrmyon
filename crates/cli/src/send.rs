use std::ops::ControlFlow;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

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
    /// Files or directories to send; several travel as one bundle, and land side by side
    pub(crate) paths: Vec<PathBuf>,
    /// Keep each path as typed, not only its last name, like `rsync -R`
    #[arg(
        short = 'R',
        long,
        long_help = "Keep each path as typed, not only its last name, like `rsync -R` or `tar`. \
The leading `/` is dropped, a `/./` in the path marks where the kept part starts, \
`..` cannot be kept, and a trailing slash changes nothing.

  wyrm send a/b.txt              b.txt
  wyrm send -R a/b.txt           a/b.txt
  wyrm send -R /etc/x/y.conf     etc/x/y.conf
  wyrm send -R /etc/./x/y.conf   x/y.conf
  wyrm send -R photos/           photos/... (the same as photos)
  wyrm send -R photos/./         the files inside photos, without photos/
  wyrm send -R ../a.txt          refused: use /./ to say what to keep

The receiver puts the kept paths under its current directory, or under the \
directory `-o` names: `wyrm receive -o / CODE` puts etc/x/y.conf back at \
/etc/x/y.conf. It creates the directories that are missing, uses the ones \
already there, and refuses a file that is already there unless it was given \
--overwrite."
    )]
    pub(crate) relative: bool,
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
    Tree(String, Arc<Vec<zipdir::Entry>>, bool),
}

#[derive(Default)]
struct Staged {
    zip: Option<zipdir::Built>,
    blob: Option<Offered>,
    collection: Option<(Offered, CollectionOffer)>,
    blob_ahead: Option<Hashing<Offered>>,
    collection_ahead: Option<Hashing<(Offered, Manifest)>>,
    several: bool,
}

impl Staged {
    fn hash_ahead(&mut self, payload: &Payload) {
        match payload {
            Payload::Text(_) => {}
            Payload::File(path, offer) => {
                self.blob_ahead = Some(Hashing::blob(path.clone(), offer.filesize));
            }
            Payload::Tree(_, entries, _) => {
                self.collection_ahead = Some(Hashing::collection(entries));
            }
        }
    }

    async fn drop_blob_ahead(&mut self) {
        if self.several {
            return;
        }
        if let Some(ahead) = self.blob_ahead.take()
            && let Some(offered) = ahead.stop().await
        {
            offered.close().await;
        }
    }

    async fn drop_collection_ahead(&mut self) {
        if self.several {
            return;
        }
        if let Some(ahead) = self.collection_ahead.take()
            && let Some((offered, _)) = ahead.stop().await
        {
            offered.close().await;
        }
    }

    async fn close(mut self) {
        self.several = false;
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
    stop: Arc<AtomicBool>,
    total: u64,
    task: Option<tokio::task::JoinHandle<Result<T, wyrmyon_transport_iroh::Error>>>,
}

impl<T: Send + 'static> Hashing<T> {
    fn start<F>(total: u64, import: impl FnOnce(Progress) -> F) -> Self
    where
        F: Future<Output = Result<T, wyrmyon_transport_iroh::Error>> + Send + 'static,
    {
        let hashed = Arc::new(AtomicU64::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let (counter, stopped) = (Arc::clone(&hashed), Arc::clone(&stop));
        let progress = Box::new(move |n| {
            counter.store(n, Ordering::Relaxed);
            if stopped.load(Ordering::Relaxed) {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        });
        Self {
            hashed,
            stop,
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
            let bar = transfer::Cleared(transfer::progress(
                self.total,
                global.hide_progress,
                global.ascii,
                false,
            ));
            let mut tick = tokio::time::interval(std::time::Duration::from_millis(100));
            let joined = loop {
                tokio::select! {
                    joined = &mut task => break joined,
                    _ = tick.tick() => bar.0.set_position(self.hashed.load(Ordering::Relaxed)),
                }
            };
            drop(bar);
            return Ok(joined.context("hashing the data")??);
        }
        Ok(task.await.context("hashing the data")??)
    }

    async fn stop(mut self) -> Option<T> {
        let task = self.task.take()?;
        self.stop.store(true, Ordering::Relaxed);
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
        self.stop.store(true, Ordering::Relaxed);
    }
}

type Progress = Box<dyn FnMut(u64) -> ControlFlow<()> + Send>;

impl SendArgs {
    pub fn of_paths(paths: Vec<PathBuf>) -> Self {
        Self {
            text: None,
            paths,
            relative: false,
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
            relative: false,
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
        None => paths_payload(args.paths, args.relative).await?,
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
    let mut staged = Staged {
        several: times > 1,
        ..Staged::default()
    };
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
        Payload::Tree(name, entries, bundle) => {
            send_tree(wormhole, global, staged, name, entries, *bundle).await
        }
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

async fn paths_payload(mut paths: Vec<PathBuf>, relative: bool) -> anyhow::Result<Payload> {
    if paths.len() == 1 && !relative {
        return path_payload(paths.remove(0)).await;
    }
    let entries = listed(move |cancel| zipdir::list_bundle(&paths, relative, cancel)).await?;
    Ok(Payload::Tree("files".into(), Arc::new(entries), true))
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
        return Ok(Payload::Tree(name, Arc::new(entries), false));
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

pub fn kept(path: &Path) -> anyhow::Result<PathBuf> {
    let text = path
        .to_str()
        .with_context(|| format!("{} is not valid UTF-8", path.display()))?;
    let markers: &[&str] = if cfg!(windows) {
        &["/./", "\\.\\", "/.\\", "\\./"]
    } else {
        &["/./"]
    };
    let tail = markers
        .iter()
        .filter_map(|m| text.find(m).map(|at| at + m.len()))
        .min()
        .map_or(text, |from| &text[from..]);
    let mut kept = PathBuf::new();
    for part in Path::new(tail).components() {
        match part {
            Component::Normal(name) => kept.push(name),
            Component::ParentDir => bail!(
                "-R cannot keep `..` in {}: mark where the kept path starts with /./",
                path.display()
            ),
            Component::Prefix(_) | Component::RootDir | Component::CurDir => {}
        }
    }
    Ok(kept)
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
    bundle: bool,
) -> anyhow::Result<()> {
    if global.use_collection(wormhole) {
        return send_collection(wormhole, global, staged, name, entries, bundle).await;
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
    let bundle = bundle && crate::speaks_iroh(wormhole);
    if bundle {
        eprintln!(
            "Sending {} files ({} bytes compressed)",
            built.numfiles, built.zipsize
        );
    } else {
        eprintln!(
            "Sending directory ({} bytes compressed) named '{name}'",
            built.zipsize
        );
    }
    let offer = DirectoryOffer {
        mode: "zipfile/deflated".into(),
        dirname: name.to_owned(),
        zipsize: built.zipsize,
        numbytes: built.numbytes,
        numfiles: built.numfiles,
        bundle,
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
    bundle: bool,
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
            bundle,
        };
        staged.collection = Some((offered, offer));
    }
    let (offered, offer) = staged
        .collection
        .as_ref()
        .expect("the collection was just imported");
    if offer.bundle {
        eprintln!(
            "Sending {} files ({} bytes)",
            offer.numfiles, offer.numbytes
        );
    } else {
        eprintln!(
            "Sending directory ({} files, {} bytes) named '{}'",
            offer.numfiles, offer.numbytes, offer.name
        );
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_keeps_the_path_as_typed_from_the_marker_on() {
        let kept =
            |path: &str| kept(Path::new(path)).map(|p| p.to_string_lossy().replace('\\', "/"));
        for (typed, landing) in [
            ("a/b.txt", "a/b.txt"),
            ("/etc/x/y.conf", "etc/x/y.conf"),
            ("/etc/./x/y.conf", "x/y.conf"),
            ("./a/./b/c", "b/c"),
            ("photos/", "photos"),
            ("photos/./", ""),
            (".././a.txt", "a.txt"),
        ] {
            assert_eq!(kept(typed).unwrap(), landing, "{typed}");
        }
        let refused = kept("../a.txt").unwrap_err().to_string();
        assert!(refused.contains("cannot keep `..`"), "{refused}");
        assert!(kept("a/./../b").is_err());
    }

    fn running() -> (Hashing<()>, tokio::sync::oneshot::Receiver<()>) {
        let (ended, released) = tokio::sync::oneshot::channel::<()>();
        let hashing = Hashing::start(0, move |mut progress| async move {
            let _ended = ended;
            while progress(1).is_continue() {
                tokio::task::yield_now().await;
            }
            Err(wyrmyon_transport_iroh::Error::Blob("stopped".into()))
        });
        (hashing, released)
    }

    #[tokio::test]
    async fn dropping_or_stopping_a_hashing_ends_its_import() {
        let (hashing, released) = running();
        drop(hashing);
        assert!(released.await.is_err());
        let (hashing, released) = running();
        assert_eq!(hashing.stop().await, None);
        assert!(released.await.is_err());
    }

    #[tokio::test]
    async fn aborting_a_finished_hashing_hands_back_what_it_imported() {
        let hashing = Hashing::start(0, |mut progress| async move {
            let _ = progress(7);
            Ok(7)
        });
        while !hashing.task.as_ref().unwrap().is_finished() {
            tokio::task::yield_now().await;
        }
        assert_eq!(hashing.hashed.load(Ordering::Relaxed), 7);
        assert_eq!(hashing.stop().await, Some(7));
    }
}
