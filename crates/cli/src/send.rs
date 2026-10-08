use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use clap::Args;
use tokio::io::AsyncReadExt;
use wyrmyon_transport_classic::{Role, TransitInfo};
use wyrmyon_transport_iroh::Role as IrohRole;
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
    let payload = match (args.text, args.path) {
        (Some(text), _) if text == "-" => {
            let mut text = String::new();
            tokio::io::stdin()
                .read_to_string(&mut text)
                .await
                .context("reading stdin")?;
            Payload::Text(text)
        }
        (Some(text), _) => Payload::Text(text),
        (None, Some(path)) => path_payload(path).await?,
        (None, None) => unreachable!("clap requires text or a path"),
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
            let source = tokio::fs::File::open(&path)
                .await
                .with_context(|| format!("opening {}", path.display()))?;
            send_data(&mut wormhole, global, Offer::File(offer), source, size).await
        }
        Payload::Directory(built, offer) => {
            let size = offer.zipsize;
            let source =
                tokio::fs::File::from_std(built.file.reopen().context("opening the zip file")?);
            send_data(&mut wormhole, global, Offer::Directory(offer), source, size).await
        }
    };
    wormhole.close(mood_for(&result)).await;
    result
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
    source: tokio::fs::File,
    size: u64,
) -> anyhow::Result<()> {
    let use_iroh = global.use_iroh(wormhole);
    if global.force_iroh && !use_iroh {
        protocol::send_error(wormhole, "the sender requires iroh-v1").await?;
        bail!("the other side does not speak iroh-v1 (--force-iroh)");
    }
    let (iroh, transit) = if use_iroh {
        let iroh = match global.iroh(IrohRole::Sender).await {
            Ok(iroh) => iroh,
            Err(e) => {
                protocol::send_error(wormhole, "the sender cannot open an iroh endpoint").await?;
                return Err(e);
            }
        };
        protocol::send(wormhole, &AppMessage::Iroh(iroh.info().await)).await?;
        (Some(iroh), None)
    } else {
        let transit = global.transit(Role::Sender, wormhole.transit_key()).await;
        protocol::send(wormhole, &AppMessage::Transit(transit.info())).await?;
        (None, Some(transit))
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
    let mut pipe = match (answered, iroh, transit) {
        (Ok((Some(theirs), _)), Some(iroh), _) => {
            Pipe::Iroh(iroh.connect(&theirs, wormhole.key()).await?)
        }
        (Ok(_), Some(iroh), _) => {
            iroh.close().await;
            bail!("the receiver accepted without an iroh address");
        }
        (Ok((_, theirs)), None, Some(transit)) => Pipe::Classic(transit.connect(&theirs).await?),
        (Err(e), iroh, _) => {
            if let Some(iroh) = iroh {
                iroh.close().await;
            }
            return Err(e);
        }
        (Ok(_), None, None) => unreachable!("one transport is always prepared"),
    };
    eprintln!("Sending ({})..", pipe.describe());
    let bar = transfer::progress(size, global.hide_progress);
    let sent = async {
        let digest = transfer::send_stream(&mut pipe, source, size, &bar).await?;
        eprintln!("File sent.. waiting for confirmation");
        transfer::await_ack(&mut pipe, &digest).await
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
