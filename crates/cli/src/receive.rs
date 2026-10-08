use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use clap::Args;
use tokio::io::{AsyncBufReadExt, BufReader};
use wyrmyon_transport_classic::{Role, Transit, TransitInfo};
use wyrmyon_wormhole::{Code, Wormhole};

use crate::protocol::{self, Answer, AppMessage, DirectoryOffer, FileOffer, Offer};
use crate::{Global, mood_for, printable, show_welcome, transfer, zipdir};

#[derive(Args)]
pub struct ReceiveArgs {
    /// The code the sender gave you; asked for when left out
    #[arg(conflicts_with = "new")]
    code: Option<String>,
    /// Allocate a code here, for the sender to use with `send --code`
    #[arg(long)]
    new: bool,
    /// Number of words in the code `--new` allocates
    #[arg(long, default_value_t = 2, requires = "new", value_parser = clap::value_parser!(u8).range(1..=8))]
    code_length: u8,
    /// Accept a file or directory without asking
    #[arg(long)]
    accept_file: bool,
    /// Where to write a received file or directory, instead of its name in the current directory
    #[arg(long, short = 'o')]
    output_file: Option<PathBuf>,
}

struct Peer {
    transit: Option<Transit>,
    theirs: Option<TransitInfo>,
}

pub async fn run(global: &Global, args: ReceiveArgs) -> anyhow::Result<()> {
    let pending = if args.new {
        let pending =
            wyrmyon_wormhole::create(&global.config(), usize::from(args.code_length)).await?;
        eprintln!("Wormhole code is: {}", pending.code());
        eprintln!(
            "On the other computer, please run:\n\n  wyrm send --code {} FILE\n\n(or: wormhole send --code {} FILE)\n",
            pending.code(),
            pending.code()
        );
        pending
    } else {
        let code: Code = match &args.code {
            Some(code) => code.parse()?,
            None => prompt_code().await?,
        };
        wyrmyon_wormhole::join(&global.config(), code).await?
    };
    show_welcome(pending.welcome());
    let mut wormhole = pending.pair().await?;
    let result = receive_offer(&mut wormhole, global, &args).await;
    wormhole.close(mood_for(&result)).await;
    result
}

async fn receive_offer(
    wormhole: &mut Wormhole,
    global: &Global,
    args: &ReceiveArgs,
) -> anyhow::Result<()> {
    let mut peer = Peer {
        transit: None,
        theirs: None,
    };
    let offer = loop {
        match protocol::next(wormhole, "sender").await? {
            AppMessage::Transit(info) => {
                peer.theirs = Some(info);
                if peer.transit.is_none() {
                    let transit = global.transit(Role::Receiver, wormhole.transit_key()).await;
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
        Offer::File(file) => receive_file(wormhole, file, peer, global, args).await,
        Offer::Directory(dir) => receive_directory(wormhole, dir, peer, global, args).await,
        Offer::Other(_) => {
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

struct Accepted {
    name: String,
    dest: PathBuf,
    transit: Transit,
    theirs: TransitInfo,
}

async fn accept(
    wormhole: &mut Wormhole,
    offered_name: &str,
    peer: Peer,
    args: &ReceiveArgs,
    describe: &str,
) -> anyhow::Result<Accepted> {
    let Some(name) = safe_name(offered_name) else {
        refuse(wormhole, "the offered name is not usable").await?;
        unreachable!("refuse always fails");
    };
    let (Some(transit), Some(theirs)) = (peer.transit, peer.theirs) else {
        refuse(wormhole, "the sender did not offer a transit connection").await?;
        unreachable!("refuse always fails");
    };
    let dest = match &args.output_file {
        Some(path) => path.clone(),
        None => PathBuf::from(&name),
    };
    if dest.symlink_metadata().is_ok() {
        refuse(
            wormhole,
            &format!("refusing to overwrite {}", dest.display()),
        )
        .await?;
    }
    eprintln!("Receiving {describe} into: {}", dest.display());
    if !args.accept_file {
        match confirm("ok? (y/N) ").await {
            Ok(true) => {}
            Ok(false) => refuse(wormhole, "transfer rejected").await?,
            Err(e) => {
                protocol::send_error(wormhole, "transfer rejected").await?;
                return Err(e);
            }
        }
    }
    Ok(Accepted {
        name,
        dest,
        transit,
        theirs,
    })
}

async fn receive_file(
    wormhole: &mut Wormhole,
    offer: FileOffer,
    peer: Peer,
    global: &Global,
    args: &ReceiveArgs,
) -> anyhow::Result<()> {
    let describe = format!("file ({} bytes)", offer.filesize);
    let accepted = accept(wormhole, &offer.filename, peer, args, &describe).await?;
    let (partial, mut file) = match Partial::create(&accepted.dest, &accepted.name).await {
        Ok(created) => created,
        Err(e) => {
            protocol::send_error(wormhole, "the receiver cannot write the file").await?;
            return Err(e);
        }
    };
    protocol::send(wormhole, &AppMessage::Answer(Answer::FileAck("ok".into()))).await?;

    let mut pipe = accepted.transit.connect(&accepted.theirs).await?;
    eprintln!("Receiving ({})..", pipe.describe());
    let bar = transfer::progress(offer.filesize, global.hide_progress);
    let digest = transfer::receive_stream(&mut pipe, &mut file, offer.filesize, &bar).await?;
    file.sync_all().await.context("writing the file")?;
    partial.finish(&accepted.dest)?;
    transfer::send_ack(&mut pipe, &digest).await?;
    pipe.shutdown().await;
    eprintln!("Received file written to {}", accepted.dest.display());
    Ok(())
}

async fn receive_directory(
    wormhole: &mut Wormhole,
    offer: DirectoryOffer,
    peer: Peer,
    global: &Global,
    args: &ReceiveArgs,
) -> anyhow::Result<()> {
    if offer.mode != "zipfile/deflated" {
        return refuse(wormhole, "unknown directory transfer mode").await;
    }
    let describe = format!(
        "directory ({} files, {} bytes, {} bytes compressed)",
        offer.numfiles, offer.numbytes, offer.zipsize
    );
    let accepted = accept(wormhole, &offer.dirname, peer, args, &describe).await?;
    let (partial, mut file) = match Partial::create(&accepted.dest, &accepted.name).await {
        Ok(created) => created,
        Err(e) => {
            protocol::send_error(wormhole, "the receiver cannot write the directory").await?;
            return Err(e);
        }
    };
    protocol::send(wormhole, &AppMessage::Answer(Answer::FileAck("ok".into()))).await?;

    let mut pipe = accepted.transit.connect(&accepted.theirs).await?;
    eprintln!("Receiving ({})..", pipe.describe());
    let bar = transfer::progress(offer.zipsize, global.hide_progress);
    let digest = transfer::receive_stream(&mut pipe, &mut file, offer.zipsize, &bar).await?;
    drop(file);

    eprintln!("Unpacking zipfile..");
    let dest = accepted.dest.clone();
    let zip = partial.path.clone();
    let limits = zipdir::Limits {
        numbytes: offer.numbytes,
        numfiles: offer.numfiles,
    };
    tokio::task::spawn_blocking(move || {
        let dir = zipdir::NewDir::create(&dest)?;
        zipdir::extract(&zip, &dest, &limits)?;
        dir.keep();
        anyhow::Ok(())
    })
    .await
    .context("unpacking the zip file")??;
    drop(partial);
    transfer::send_ack(&mut pipe, &digest).await?;
    pipe.shutdown().await;
    eprintln!("Received files written to {}", accepted.dest.display());
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
