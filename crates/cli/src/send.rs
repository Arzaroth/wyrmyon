use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use clap::Args;
use tokio::io::AsyncReadExt;
use wyrmyon_transport_classic::{Role, Transit, TransitInfo};
use wyrmyon_transport_iroh::{IrohTransport, Offered, Role as IrohRole, Source};
use wyrmyon_wormhole::{Code, Wormhole};

use crate::protocol::{
    self, Answer, AppMessage, CollectionOffer, DirectoryOffer, FileOffer, Offer,
};
use crate::transfer::Pipe;
use crate::{CODE_LENGTH, Global, mood_for, show_code, show_qr, show_welcome, transfer, zipdir};

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
}

enum Payload {
    Text(String),
    File(PathBuf, FileOffer),
    Tree(String, Vec<zipdir::Entry>),
}

impl SendArgs {
    pub fn of_paths(paths: Vec<PathBuf>) -> Self {
        Self {
            text: None,
            paths,
            code_length: CODE_LENGTH,
            code: None,
            qr: false,
        }
    }

    pub fn of_stdin() -> Self {
        Self {
            text: Some("-".into()),
            paths: Vec::new(),
            code_length: CODE_LENGTH,
            code: None,
            qr: false,
        }
    }
}

pub async fn run(global: &Global, args: SendArgs) -> anyhow::Result<()> {
    let code: Option<Code> = args.code.as_deref().map(str::parse).transpose()?;
    let payload = match args.text {
        Some(text) if text == "-" => Payload::Text(stdin_text().await?),
        Some(text) => Payload::Text(text),
        None => paths_payload(args.paths).await?,
    };

    let pending = if let Some(code) = code {
        if args.qr {
            show_qr(&code, global.ascii);
        }
        wyrmyon_wormhole::join(&global.config(), code).await?
    } else {
        let pending =
            wyrmyon_wormhole::create(&global.config(), usize::from(args.code_length)).await?;
        let code = pending.code();
        let other_side = format!("  wyrm {code}\n\n(or: wormhole receive {code})");
        show_code(code, args.qr.then_some(global.ascii), &other_side);
        pending
    };
    show_welcome(pending.welcome());

    let mut wormhole = pending.pair().await?;
    let result = match payload {
        Payload::Text(text) => send_text(&mut wormhole, text).await,
        Payload::File(path, offer) => {
            let size = offer.filesize;
            send_data(&mut wormhole, global, Offer::File(offer), &path, size).await
        }
        Payload::Tree(name, entries) => send_tree(&mut wormhole, global, name, entries).await,
    };
    wormhole.close(mood_for(&result)).await;
    result
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

async fn paths_payload(mut paths: Vec<PathBuf>) -> anyhow::Result<Payload> {
    if paths.len() == 1 {
        return path_payload(paths.remove(0)).await;
    }
    let entries = listed(move |cancel| zipdir::list_bundle(&paths, cancel)).await?;
    Ok(Payload::Tree("files".into(), entries))
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
        return Ok(Payload::Tree(name, entries));
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
    name: String,
    entries: Vec<zipdir::Entry>,
) -> anyhow::Result<()> {
    if global.use_collection(wormhole) {
        return send_collection(wormhole, global, name, &entries).await;
    }
    require_iroh(wormhole, global).await?;
    eprintln!("Building zipfile..");
    let cancel = zipdir::Cancel::default();
    let _cancel_on_drop = cancel.on_drop();
    let zipped = tokio::task::spawn_blocking(move || zipdir::zip(&entries, &cancel))
        .await
        .context("building the zip file")
        .and_then(|built| built);
    let built = match zipped {
        Ok(built) => built,
        Err(e) => {
            protocol::send_error(wormhole, "the sender could not read its data").await?;
            return Err(e);
        }
    };
    eprintln!(
        "Sending directory ({} bytes compressed) named '{name}'",
        built.zipsize
    );
    let offer = DirectoryOffer {
        mode: "zipfile/deflated".into(),
        dirname: name,
        zipsize: built.zipsize,
        numbytes: built.numbytes,
        numfiles: built.numfiles,
    };
    let size = offer.zipsize;
    send_data(
        wormhole,
        global,
        Offer::Directory(offer),
        built.file.path(),
        size,
    )
    .await
}

async fn send_collection(
    wormhole: &mut Wormhole,
    global: &Global,
    name: String,
    entries: &[zipdir::Entry],
) -> anyhow::Result<()> {
    let (mut sources, mut dirs) = (Vec::new(), Vec::new());
    for entry in entries {
        match entry {
            zipdir::Entry::File {
                source, name, mode, ..
            } => sources.push(Source {
                path: source.clone(),
                name: name.clone(),
                mode: *mode,
            }),
            zipdir::Entry::EmptyDir(dir) if dir.is_empty() => {}
            zipdir::Entry::EmptyDir(dir) => dirs.push(dir.clone()),
        }
    }
    eprintln!("Hashing..");
    let (offered, manifest) = match Offered::import_collection(&sources, &dirs).await {
        Ok(imported) => imported,
        Err(e) => {
            protocol::send_error(wormhole, "the sender could not read its data").await?;
            return Err(e.into());
        }
    };
    let offer = CollectionOffer {
        name,
        numbytes: manifest.numbytes(),
        numfiles: manifest.files.len() as u64,
    };
    eprintln!(
        "Sending directory ({} files, {} bytes) named '{}'",
        offer.numfiles, offer.numbytes, offer.name
    );
    let size = offer.numbytes;
    let result = send_over(
        wormhole,
        global,
        Offer::Collection(offer),
        None,
        size,
        Some(&offered),
    )
    .await;
    offered.close().await;
    result
}

async fn send_data(
    wormhole: &mut Wormhole,
    global: &Global,
    offer: Offer,
    path: &Path,
    size: u64,
) -> anyhow::Result<()> {
    let use_iroh = require_iroh(wormhole, global).await?;
    let offered = if use_iroh {
        eprintln!("Hashing..");
        match Offered::import(path).await {
            Ok(offered) => Some(offered),
            Err(e) => {
                protocol::send_error(wormhole, "the sender could not read its data").await?;
                return Err(e.into());
            }
        }
    } else {
        None
    };
    let result = send_over(wormhole, global, offer, Some(path), size, offered.as_ref()).await;
    if let Some(offered) = offered {
        offered.close().await;
    }
    result
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
        protocol::send(wormhole, &AppMessage::Iroh(iroh.info().await)).await?;
        Prepared::Iroh(iroh)
    } else {
        let transit = global.transit(Role::Sender, wormhole.transit_key()).await;
        protocol::send(wormhole, &AppMessage::Transit(transit.info())).await?;
        Prepared::Classic(transit)
    };
    protocol::send(wormhole, &AppMessage::Offer(offer)).await?;

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
