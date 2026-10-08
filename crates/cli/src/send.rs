use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use clap::Args;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;
use wyrmyon_transport_classic::{Role, Transit, TransitInfo};
use wyrmyon_wormhole::Wormhole;

use crate::protocol::{self, Answer, AppMessage, FileOffer, Offer};
use crate::{Global, mood_for, show_welcome};

const CHUNK: usize = 1 << 16;

#[derive(Args)]
#[command(group = clap::ArgGroup::new("what").required(true).args(["text", "path"]))]
pub struct SendArgs {
    /// Text to send; `-` reads it from stdin
    #[arg(long)]
    text: Option<String>,
    /// File to send
    path: Option<PathBuf>,
    /// Number of words in the generated code
    #[arg(long, default_value_t = 2, value_parser = clap::value_parser!(u8).range(1..=8))]
    code_length: u8,
}

enum Payload {
    Text(String),
    File(PathBuf, FileOffer),
}

pub async fn run(global: &Global, args: SendArgs) -> anyhow::Result<()> {
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
        (None, Some(path)) => file_payload(path).await?,
        (None, None) => unreachable!("clap requires text or a path"),
    };

    let pending = wyrmyon_wormhole::create(&global.config(), usize::from(args.code_length)).await?;
    show_welcome(pending.welcome());
    eprintln!("Wormhole code is: {}", pending.code());
    eprintln!("On the other computer, please run:\n\n  wyrm receive\n\n(or: wormhole receive)\n");

    let mut wormhole = pending.pair().await?;
    let result = match payload {
        Payload::Text(text) => send_text(&mut wormhole, text).await,
        Payload::File(path, offer) => send_file(&mut wormhole, &path, offer).await,
    };
    wormhole.close(mood_for(&result)).await;
    result
}

async fn file_payload(path: PathBuf) -> anyhow::Result<Payload> {
    let meta = tokio::fs::metadata(&path)
        .await
        .with_context(|| format!("cannot send {}", path.display()))?;
    if meta.is_dir() {
        bail!(
            "{} is a directory; sending directories is not supported yet",
            path.display()
        );
    }
    if !meta.is_file() {
        bail!("{} is not a regular file", path.display());
    }
    let filename = path
        .file_name()
        .with_context(|| format!("{} has no file name", path.display()))?
        .to_string_lossy()
        .into_owned();
    eprintln!("Sending {} bytes file named '{filename}'", meta.len());
    let offer = FileOffer {
        filename,
        filesize: meta.len(),
    };
    Ok(Payload::File(path, offer))
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

async fn send_file(wormhole: &mut Wormhole, path: &Path, offer: FileOffer) -> anyhow::Result<()> {
    let transit = Transit::new(Role::Sender, wormhole.transit_key()).await;
    protocol::send(wormhole, &AppMessage::Transit(transit.info())).await?;
    let filesize = offer.filesize;
    protocol::send(wormhole, &AppMessage::Offer(Offer::File(offer))).await?;

    let mut theirs = TransitInfo::default();
    loop {
        match protocol::next(wormhole, "receiver").await? {
            AppMessage::Transit(info) => theirs = info,
            AppMessage::Answer(Answer::FileAck(ack)) if ack == "ok" => break,
            AppMessage::Answer(answer) => bail!("unexpected answer from the receiver: {answer:?}"),
            _ => {}
        }
    }

    let mut pipe = transit.connect(&theirs).await?;
    eprintln!("Sending ({})..", pipe.describe());
    let mut file = tokio::fs::File::open(path)
        .await
        .with_context(|| format!("opening {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; CHUNK];
    let mut sent = 0u64;
    loop {
        let n = file.read(&mut buf).await.context("reading the file")?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        pipe.send_record(&buf[..n]).await?;
        sent += n as u64;
        if sent > filesize {
            bail!("{} grew while it was being sent", path.display());
        }
    }
    if sent != filesize {
        bail!("{} shrank while it was being sent", path.display());
    }
    pipe.flush().await?;
    eprintln!("File sent.. waiting for confirmation");

    let ack: Value = serde_json::from_slice(&pipe.receive_record().await?)
        .context("the receiver's confirmation is not JSON")?;
    if ack["ack"] != "ok" {
        bail!("transfer failed, the receiver says: {ack}");
    }
    if let Some(theirs) = ack["sha256"].as_str()
        && theirs != hex::encode(hasher.finalize())
    {
        bail!("transfer failed: the receiver got different data");
    }
    pipe.shutdown().await;
    eprintln!("Confirmation received. Transfer complete.");
    Ok(())
}
