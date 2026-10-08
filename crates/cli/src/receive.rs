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
    let dest = match &args.output_file {
        Some(path) => path.clone(),
        None => PathBuf::from(&name),
    };
    if dest.exists() {
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
    let (Some(transit), Some(theirs)) = (peer.transit, peer.theirs) else {
        return refuse(wormhole, "the sender did not offer a transit connection").await;
    };
    protocol::send(wormhole, &AppMessage::Answer(Answer::FileAck("ok".into()))).await?;

    let mut pipe = transit.connect(&theirs).await?;
    eprintln!("Receiving ({})..", pipe.describe());
    let partial = partial_path(&dest, &name);
    let written = write_records(&mut pipe, &partial, offer.filesize).await;
    let digest = match written {
        Ok(digest) => digest,
        Err(e) => {
            let _ = tokio::fs::remove_file(&partial).await;
            return Err(e);
        }
    };
    if dest.exists() {
        let _ = tokio::fs::remove_file(&partial).await;
        bail!(
            "{} appeared during the transfer; the data is discarded",
            dest.display()
        );
    }
    tokio::fs::rename(&partial, &dest)
        .await
        .with_context(|| format!("moving the file to {}", dest.display()))?;
    let ack = json!({"ack": "ok", "sha256": hex::encode(digest)}).to_string();
    pipe.send_record(ack.as_bytes()).await?;
    pipe.flush().await?;
    pipe.shutdown().await;
    eprintln!("Received file written to {}", dest.display());
    Ok(())
}

async fn write_records(
    pipe: &mut wyrmyon_transport_classic::RecordPipe,
    partial: &Path,
    size: u64,
) -> anyhow::Result<Vec<u8>> {
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(partial)
        .await
        .with_context(|| format!("creating {}", partial.display()))?;
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

fn partial_path(dest: &Path, name: &str) -> PathBuf {
    let dir = dest.parent().filter(|p| !p.as_os_str().is_empty());
    let file = format!(".{name}.wyrm-part");
    dir.map_or_else(|| PathBuf::from(&file), |d| d.join(&file))
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

    #[test]
    fn partial_files_sit_next_to_the_destination() {
        assert_eq!(
            partial_path(Path::new("x.bin"), "x.bin"),
            PathBuf::from(".x.bin.wyrm-part")
        );
        assert_eq!(
            partial_path(Path::new("/tmp/out/y"), "z"),
            PathBuf::from("/tmp/out/.z.wyrm-part")
        );
    }
}
