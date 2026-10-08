use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use clap::Args;
use serde_json::json;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use wyrmyon_transport_classic::{Role, Transit, TransitInfo};
use wyrmyon_wormhole::{Code, Wormhole};

use crate::protocol::{self, Answer, AppMessage, FileOffer, Offer};
use crate::{Global, mood_for, printable, show_welcome};

#[derive(Args)]
pub struct ReceiveArgs {
    /// The code the sender gave you; asked for when left out
    code: Option<String>,
    /// Accept a file without asking
    #[arg(long)]
    accept_file: bool,
    /// Where to write a received file, instead of its name in the current directory
    #[arg(long, short = 'o')]
    output_file: Option<PathBuf>,
}

struct Peer {
    transit: Option<Transit>,
    theirs: Option<TransitInfo>,
}

pub async fn run(global: &Global, args: ReceiveArgs) -> anyhow::Result<()> {
    let code: Code = match &args.code {
        Some(code) => code.parse()?,
        None => prompt_code().await?,
    };
    let pending = wyrmyon_wormhole::join(&global.config(), code).await?;
    show_welcome(pending.welcome());
    let mut wormhole = pending.pair().await?;
    let result = receive_offer(&mut wormhole, &args).await;
    wormhole.close(mood_for(&result)).await;
    result
}

async fn receive_offer(wormhole: &mut Wormhole, args: &ReceiveArgs) -> anyhow::Result<()> {
    let mut peer = Peer {
        transit: None,
        theirs: None,
    };
    let offer = loop {
        match protocol::next(wormhole, "sender").await? {
            AppMessage::Transit(info) => {
                peer.theirs = Some(info);
                if peer.transit.is_none() {
                    let transit = Transit::new(Role::Receiver, wormhole.transit_key()).await;
                    protocol::send(wormhole, &AppMessage::Transit(transit.info())).await?;
                    peer.transit = Some(transit);
                }
            }
            AppMessage::Offer(offer) => break offer,
            _ => {}
        }
    };
    match offer {
        Offer::Message(text) => receive_text(wormhole, &text).await,
        Offer::File(file) => receive_file(wormhole, file, peer, args).await,
        Offer::Directory(_) | Offer::Other(_) => {
            protocol::send_error(wormhole, "wyrmyon cannot receive this kind of offer yet").await?;
            bail!("the sender offered something this version cannot receive");
        }
    }
}

async fn receive_text(wormhole: &mut Wormhole, text: &str) -> anyhow::Result<()> {
    if let Err(e) = writeln!(std::io::stdout().lock(), "{text}") {
        protocol::send_error(wormhole, "the receiver could not write the message out").await?;
        return Err(e).context("writing to stdout");
    }
    protocol::send(
        wormhole,
        &AppMessage::Answer(Answer::MessageAck("ok".into())),
    )
    .await
}

async fn refuse(wormhole: &mut Wormhole, why: &str) -> anyhow::Result<()> {
    protocol::send_error(wormhole, why).await?;
    bail!("{why}")
}

async fn receive_file(
    wormhole: &mut Wormhole,
    offer: FileOffer,
    peer: Peer,
    args: &ReceiveArgs,
) -> anyhow::Result<()> {
    let Some(name) = safe_name(&offer.filename) else {
        return refuse(wormhole, "the offered file name is not usable").await;
    };
    let (Some(transit), Some(theirs)) = (peer.transit, peer.theirs) else {
        return refuse(wormhole, "the sender did not offer a transit connection").await;
    };
    let dest = match &args.output_file {
        Some(path) => path.clone(),
        None => PathBuf::from(&name),
    };
    if dest.symlink_metadata().is_ok() {
        return refuse(
            wormhole,
            &format!("refusing to overwrite {}", dest.display()),
        )
        .await;
    }
    eprintln!(
        "Receiving file ({} bytes) into: {}",
        offer.filesize,
        dest.display()
    );
    if !args.accept_file {
        match confirm("ok? (y/N) ").await {
            Ok(true) => {}
            Ok(false) => return refuse(wormhole, "transfer rejected").await,
            Err(e) => {
                protocol::send_error(wormhole, "transfer rejected").await?;
                return Err(e);
            }
        }
    }
    let (partial, file) = match Partial::create(&dest, &name).await {
        Ok(created) => created,
        Err(e) => {
            protocol::send_error(wormhole, "the receiver cannot write the file").await?;
            return Err(e);
        }
    };
    protocol::send(wormhole, &AppMessage::Answer(Answer::FileAck("ok".into()))).await?;

    let mut pipe = transit.connect(&theirs).await?;
    eprintln!("Receiving ({})..", pipe.describe());
    let digest = write_records(&mut pipe, file, offer.filesize).await?;
    partial.finish(&dest)?;
    let ack = json!({"ack": "ok", "sha256": hex::encode(digest)}).to_string();
    pipe.send_record(ack.as_bytes()).await?;
    pipe.flush().await?;
    pipe.shutdown().await;
    eprintln!("Received file written to {}", dest.display());
    Ok(())
}

struct Partial {
    path: PathBuf,
}

impl Partial {
    async fn create(dest: &Path, name: &str) -> anyhow::Result<(Self, tokio::fs::File)> {
        let dir = dest.parent().filter(|p| !p.as_os_str().is_empty());
        let file = format!(".{name}.{}.wyrm-part", unique_suffix());
        let path = dir.map_or_else(|| PathBuf::from(&file), |d| d.join(&file));
        let handle = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .await
            .with_context(|| format!("creating {}", path.display()))?;
        Ok((Self { path }, handle))
    }

    fn finish(self, dest: &Path) -> anyhow::Result<()> {
        let appeared = || {
            format!(
                "{} appeared during the transfer; the data is discarded",
                dest.display()
            )
        };
        match std::fs::hard_link(&self.path, dest) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                Err(e).with_context(appeared)
            }
            Err(_) if dest.symlink_metadata().is_ok() => bail!(appeared()),
            Err(_) => std::fs::rename(&self.path, dest)
                .with_context(|| format!("moving the file to {}", dest.display())),
        }
    }
}

impl Drop for Partial {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn unique_suffix() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    format!("{:x}{nanos:x}", std::process::id())
}

async fn write_records(
    pipe: &mut wyrmyon_transport_classic::RecordPipe,
    mut file: tokio::fs::File,
    size: u64,
) -> anyhow::Result<Vec<u8>> {
    let mut hasher = Sha256::new();
    let mut received = 0u64;
    while received < size {
        let record = pipe.receive_record().await?;
        received += record.len() as u64;
        if received > size {
            bail!("the sender sent more than the {size} bytes it offered");
        }
        hasher.update(&record);
        file.write_all(&record).await.context("writing the file")?;
    }
    file.sync_all().await.context("writing the file")?;
    Ok(hasher.finalize().to_vec())
}

fn safe_name(offered: &str) -> Option<String> {
    let name = printable(Path::new(offered).file_name()?.to_str()?);
    (!name.is_empty() && name != "." && name != "..").then_some(name)
}

async fn confirm(question: &str) -> anyhow::Result<bool> {
    if !std::io::stdin().is_terminal() {
        bail!("not asking for confirmation without a terminal: pass --accept-file");
    }
    eprint!("{question}");
    let mut line = String::new();
    BufReader::new(tokio::io::stdin())
        .read_line(&mut line)
        .await
        .context("reading the answer")?;
    Ok(matches!(line.trim(), "y" | "Y" | "yes"))
}

async fn prompt_code() -> anyhow::Result<Code> {
    eprint!("Enter receive wormhole code: ");
    let mut line = String::new();
    BufReader::new(tokio::io::stdin())
        .read_line(&mut line)
        .await
        .context("reading the code")?;
    Ok(line.trim().parse()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offered_names_lose_their_directories_and_control_characters() {
        assert_eq!(safe_name("report.pdf").as_deref(), Some("report.pdf"));
        assert_eq!(safe_name("../../etc/passwd").as_deref(), Some("passwd"));
        assert_eq!(safe_name("/abs/x").as_deref(), Some("x"));
        assert_eq!(safe_name("a\x1b[2Jb").as_deref(), Some("a[2Jb"));
        assert_eq!(safe_name(".."), None);
        assert_eq!(safe_name(""), None);
    }
}
