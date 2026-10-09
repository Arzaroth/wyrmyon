use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use clap::Args;
use tokio::io::AsyncReadExt;
use wyrmyon_transport_classic::{Role, Transit, TransitInfo};
use wyrmyon_transport_iroh::{IrohTransport, Offered, Role as IrohRole};
use wyrmyon_wormhole::{Code, Wormhole};

use crate::protocol::{self, Answer, AppMessage, DirectoryOffer, FileOffer, Offer};
use crate::transfer::Pipe;
use crate::{Global, mood_for, show_welcome, transfer, zipdir};

#[derive(Args)]
#[command(group = clap::ArgGroup::new("what").required(true).args(["text", "path"]))]
pub struct SendArgs {
    /// Text to send; `-` reads it from stdin
    #[arg(long)]
    text: Option<String>,
    /// File or directory to send
    path: Option<PathBuf>,
    /// Number of words in the generated code
    #[arg(long, default_value_t = 2, value_parser = clap::value_parser!(u8).range(1..=8))]
    code_length: u8,
    /// Use this code instead of generating one, e.g. the one a receiver chose
    #[arg(long, conflicts_with = "code_length")]
    code: Option<String>,
}

enum Payload {
    Text(String),
    File(PathBuf, FileOffer),
    Directory(zipdir::Built, DirectoryOffer),
}

pub async fn run(global: &Global, args: SendArgs) -> anyhow::Result<()> {
    let code: Option<Code> = args.code.as_deref().map(str::parse).transpose()?;
    let payload = match args.text {
        Some(text) if text == "-" => Payload::Text(stdin_text().await?),
        Some(text) => Payload::Text(text),
        None => path_payload(args.path.context("nothing to send")?).await?,
    };

    let pending = if let Some(code) = code {
        wyrmyon_wormhole::join(&global.config(), code).await?
    } else {
        let pending =
            wyrmyon_wormhole::create(&global.config(), usize::from(args.code_length)).await?;
        eprintln!("Wormhole code is: {}", pending.code());
        eprintln!(
            "On the other computer, please run:\n\n  wyrm receive\n\n(or: wormhole receive)\n"
        );
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
        Payload::Directory(built, offer) => {
            let size = offer.zipsize;
            send_data(
                &mut wormhole,
                global,
                Offer::Directory(offer),
                built.file.path(),
                size,
            )
            .await
        }
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

async fn path_payload(path: PathBuf) -> anyhow::Result<Payload> {
    let meta = tokio::fs::metadata(&path)
        .await
        .with_context(|| format!("cannot send {}", path.display()))?;
    let name = display_name(&path)?;
    if meta.is_dir() {
        eprintln!("Building zipfile..");
        let dir = path.clone();
        let cancel = zipdir::Cancel::default();
        let _cancel_on_drop = cancel.on_drop();
        let built = tokio::task::spawn_blocking(move || zipdir::build(&dir, &cancel))
            .await
            .context("building the zip file")??;
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
        return Ok(Payload::Directory(built, offer));
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

fn display_name(path: &Path) -> anyhow::Result<String> {
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

async fn send_data(
    wormhole: &mut Wormhole,
    global: &Global,
    offer: Offer,
    path: &Path,
    size: u64,
) -> anyhow::Result<()> {
    let use_iroh = global.use_iroh(wormhole);
    if global.force_iroh && !use_iroh {
        protocol::send_error(wormhole, "the sender requires iroh-v1").await?;
        bail!("the other side does not speak iroh-v1 (--force-iroh)");
    }
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
    let result = send_over(wormhole, global, offer, path, size, offered.as_ref()).await;
    if let Some(offered) = offered {
        offered.close().await;
    }
    result
}

async fn send_over(
    wormhole: &mut Wormhole,
    global: &Global,
    offer: Offer,
    path: &Path,
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
    let bar = transfer::progress(size, global.hide_progress);
    let sent = async {
        let digest = match &mut pipe {
            Pipe::Iroh(iroh) => {
                iroh.provide(offered.expect("iroh pipes carry a blob"))
                    .await?;
                None
            }
            Pipe::Classic(records) => {
                let mut source = tokio::fs::File::open(path)
                    .await
                    .with_context(|| format!("opening {}", path.display()))?;
                Some(transfer::send_stream(records, &mut source, size, &bar).await?)
            }
        };
        eprintln!("Waiting for the receiver to confirm..");
        transfer::await_ack(&mut pipe, digest.as_ref()).await
    }
    .await;
    if let Err(e) = sent {
        pipe.abort().await;
        return Err(e);
    }
    pipe.shutdown().await;
    eprintln!("Confirmation received. Transfer complete.");
    Ok(())
}
