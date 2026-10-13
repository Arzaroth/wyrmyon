use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use clap::Args;
use tokio::io::{AsyncBufReadExt, BufReader};
use wyrmyon_transport_classic::{Role, Transit, TransitInfo};
use wyrmyon_transport_iroh::{IrohInfo, IrohTransport, MAX_FILES, Role as IrohRole};
use wyrmyon_wormhole::{Code, Wormhole};

use crate::protocol::{
    self, Answer, AppMessage, CollectionOffer, DirectoryOffer, FileOffer, Offer,
};
use crate::transfer::Pipe;
use crate::{
    CODE_LENGTH, Global, cache_dir, mood_for, printable, show_code, show_welcome, transfer, zipdir,
};

#[derive(Args)]
pub struct ReceiveArgs {
    /// The code the sender gave you; asked for when left out
    #[arg(conflicts_with = "new")]
    pub(crate) code: Option<String>,
    /// Allocate a code here, for the sender to use with `send --code`
    #[arg(long, visible_alias = "allocate")]
    pub(crate) new: bool,
    /// Number of words in the code: the one `--new` allocates, or the one the prompt completes
    #[arg(long, default_value_t = CODE_LENGTH, value_parser = clap::value_parser!(u8).range(1..=8))]
    pub(crate) code_length: u8,
    /// Accept a file or directory without asking
    #[arg(long)]
    pub(crate) accept_file: bool,
    /// Where to write a received file or directory, instead of its name in the current directory
    #[arg(long, short = 'o')]
    pub(crate) output_file: Option<PathBuf>,
}

impl ReceiveArgs {
    pub fn of_code(code: Option<String>) -> Self {
        Self {
            code,
            new: false,
            code_length: CODE_LENGTH,
            accept_file: false,
            output_file: None,
        }
    }
}

pub async fn run(global: &Global, args: ReceiveArgs) -> anyhow::Result<()> {
    let pending = if args.new {
        let pending =
            wyrmyon_wormhole::create(&global.config(), usize::from(args.code_length)).await?;
        let code = pending.code();
        let other_side =
            format!("  wyrm send --code {code} FILE\n\n(or: wormhole send --code {code} FILE)");
        show_code(code, None, &other_side);
        pending
    } else {
        let code: Code = match &args.code {
            Some(code) => code.parse()?,
            None => prompt_code(usize::from(args.code_length)).await?,
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
    let iroh_allowed = global.use_iroh(wormhole);
    let mut connection: Option<Connection> = None;
    let offer = loop {
        match protocol::next(wormhole, "sender").await {
            Ok(AppMessage::Transit(info)) if connection.is_none() => {
                let transit = global.transit(Role::Receiver, wormhole.transit_key()).await;
                protocol::send(wormhole, &AppMessage::Transit(transit.info())).await?;
                connection = Some(Connection::Classic(transit, info));
            }
            Ok(AppMessage::Iroh(info)) if iroh_allowed && connection.is_none() => {
                let iroh = global.iroh_or_tell(wormhole, IrohRole::Receiver).await?;
                protocol::send(wormhole, &AppMessage::Iroh(iroh.info().await)).await?;
                connection = Some(Connection::Iroh(iroh, info));
            }
            Ok(AppMessage::Offer(offer)) => break offer,
            Ok(_) => {}
            Err(e) => {
                if let Some(connection) = connection {
                    connection.close().await;
                }
                return Err(e);
            }
        }
    };
    match offer {
        Offer::File(file) => receive_file(wormhole, file, connection, global, args).await,
        Offer::Directory(dir) => receive_directory(wormhole, dir, connection, global, args).await,
        Offer::Collection(collection) => {
            receive_collection(wormhole, collection, connection, global, args).await
        }
        Offer::Message(text) => {
            if let Some(connection) = connection {
                connection.close().await;
            }
            receive_text(wormhole, &text).await
        }
        Offer::Other(_) => {
            if let Some(connection) = connection {
                connection.close().await;
            }
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

async fn refuse<T>(wormhole: &mut Wormhole, why: &str) -> anyhow::Result<T> {
    protocol::send_error(wormhole, why).await?;
    bail!("{why}")
}

struct Incoming {
    partial: Partial,
    dest: PathBuf,
    digest: Option<[u8; 32]>,
    pipe: Pipe,
}

impl Incoming {
    async fn complete(mut self, finished: anyhow::Result<()>) -> anyhow::Result<PathBuf> {
        let done = match finished {
            Ok(()) => transfer::send_ack(&mut self.pipe, self.digest.as_ref()).await,
            Err(e) => Err(e),
        };
        match done {
            Ok(()) => {
                self.pipe.shutdown().await;
                Ok(self.dest)
            }
            Err(e) => {
                self.pipe.abort().await;
                Err(e)
            }
        }
    }
}

enum Connection {
    Iroh(IrohTransport, IrohInfo),
    Classic(Transit, TransitInfo),
}

impl Connection {
    async fn close(self) {
        if let Self::Iroh(iroh, _) = self {
            iroh.close().await;
        }
    }
}

async fn receive_payload(
    wormhole: &mut Wormhole,
    offered_name: &str,
    size: u64,
    describe: &str,
    connection: Option<Connection>,
    global: &Global,
    args: &ReceiveArgs,
) -> anyhow::Result<Incoming> {
    let Some(connection) = connection else {
        return refuse(wormhole, "the sender did not offer a connection").await;
    };
    let accepted = accept(wormhole, offered_name, &connection, global, args, describe).await;
    let (dest, partial, mut file) = match accepted {
        Ok(accepted) => accepted,
        Err(e) => {
            connection.close().await;
            return Err(e);
        }
    };
    protocol::send(wormhole, &AppMessage::Answer(Answer::FileAck("ok".into()))).await?;

    let mut pipe = match connection {
        Connection::Iroh(iroh, theirs) => Pipe::Iroh(iroh.connect(&theirs, wormhole.key()).await?),
        Connection::Classic(transit, theirs) => Pipe::Classic(transit.connect(&theirs).await?),
    };
    eprintln!("Receiving ({})..", pipe.describe());
    let bar = transfer::progress(
        size,
        global.hide_progress,
        global.ascii,
        pipe.through_wormhole(),
    );
    pipe.follow_route(&bar);
    let received = async {
        match &mut pipe {
            Pipe::Iroh(iroh) => {
                let fetched = iroh
                    .fetch(&cache_dir()?, size, &mut |n| bar.set_position(n))
                    .await?;
                transfer::finale(&bar).await;
                drop(file.into_std().await);
                fetched.export_to(&partial.path).await?;
                Ok(None)
            }
            Pipe::Classic(records) => {
                let digest = transfer::receive_stream(records, &mut file, size, &bar).await?;
                file.sync_all().await.context("writing the data")?;
                anyhow::Ok(Some(digest))
            }
        }
    }
    .await;
    match received {
        Ok(digest) => {
            if let Some(went) = pipe.went() {
                eprintln!("{went}");
            }
            Ok(Incoming {
                partial,
                dest,
                digest,
                pipe,
            })
        }
        Err(e) => {
            bar.finish_and_clear();
            pipe.abort().await;
            Err(e)
        }
    }
}

async fn accept(
    wormhole: &mut Wormhole,
    offered_name: &str,
    connection: &Connection,
    global: &Global,
    args: &ReceiveArgs,
    describe: &str,
) -> anyhow::Result<(PathBuf, Partial, tokio::fs::File)> {
    let classic = matches!(connection, Connection::Classic(..));
    let (dest, name) = destination(wormhole, offered_name, classic, global, args, describe).await?;
    match Partial::create(&dest, &name).await {
        Ok((partial, file)) => Ok((dest, partial, file)),
        Err(e) => {
            protocol::send_error(wormhole, "the receiver cannot write there").await?;
            Err(e)
        }
    }
}

async fn destination(
    wormhole: &mut Wormhole,
    offered_name: &str,
    classic: bool,
    global: &Global,
    args: &ReceiveArgs,
    describe: &str,
) -> anyhow::Result<(PathBuf, String)> {
    if global.force_iroh && classic {
        protocol::send_error(wormhole, "the receiver requires iroh-v1").await?;
        bail!("the other side does not speak iroh-v1 (--force-iroh)");
    }
    let Some(name) = safe_name(offered_name) else {
        return refuse(wormhole, "the offered name is not usable").await;
    };
    let dest = match &args.output_file {
        Some(path) if path.is_dir() => path.join(&name),
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
    eprintln!("Receiving {describe} into: {}", dest.display());
    if !args.accept_file {
        match confirm("ok? (Y/n) ").await {
            Ok(true) => {}
            Ok(false) => return refuse(wormhole, "transfer rejected").await,
            Err(e) => {
                protocol::send_error(wormhole, "transfer rejected").await?;
                return Err(e);
            }
        }
    }
    Ok((dest, name))
}

async fn receive_file(
    wormhole: &mut Wormhole,
    offer: FileOffer,
    connection: Option<Connection>,
    global: &Global,
    args: &ReceiveArgs,
) -> anyhow::Result<()> {
    let describe = format!("file ({} bytes)", offer.filesize);
    let incoming = receive_payload(
        wormhole,
        &offer.filename,
        offer.filesize,
        &describe,
        connection,
        global,
        args,
    )
    .await?;
    let finished = incoming.partial.finish(&incoming.dest);
    let dest = incoming.complete(finished).await?;
    eprintln!("Received file written to {}", dest.display());
    Ok(())
}

async fn receive_directory(
    wormhole: &mut Wormhole,
    offer: DirectoryOffer,
    connection: Option<Connection>,
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
    let incoming = receive_payload(
        wormhole,
        &offer.dirname,
        offer.zipsize,
        &describe,
        connection,
        global,
        args,
    )
    .await?;

    eprintln!("Unpacking zipfile..");
    let limits = zipdir::Limits {
        numbytes: offer.numbytes,
        numfiles: offer.numfiles,
    };
    let (target, zip) = (incoming.dest.clone(), incoming.partial.path.clone());
    let cancel = zipdir::Cancel::default();
    let _cancel_on_drop = cancel.on_drop();
    let unpacked = tokio::task::spawn_blocking(move || {
        let dir = zipdir::NewDir::create(&target)?;
        zipdir::extract(&zip, &target, &limits, &cancel)?;
        dir.keep();
        anyhow::Ok(())
    })
    .await
    .context("unpacking the zip file");
    let dest = incoming.complete(unpacked.and_then(|r| r)).await?;
    eprintln!("Received files written to {}", dest.display());
    Ok(())
}

async fn receive_collection(
    wormhole: &mut Wormhole,
    offer: CollectionOffer,
    connection: Option<Connection>,
    global: &Global,
    args: &ReceiveArgs,
) -> anyhow::Result<()> {
    let Some(Connection::Iroh(iroh, theirs)) = connection else {
        return refuse(wormhole, "the sender offered a collection without iroh").await;
    };
    if offer.numfiles > MAX_FILES {
        iroh.close().await;
        return refuse(wormhole, "the sender offered too many files").await;
    }
    let describe = format!(
        "directory ({} files, {} bytes)",
        offer.numfiles, offer.numbytes
    );
    let chosen = async {
        let (dest, _) = destination(wormhole, &offer.name, false, global, args, &describe).await?;
        match zipdir::NewDir::create(&dest) {
            Ok(dir) => Ok((dest, dir)),
            Err(e) => {
                protocol::send_error(wormhole, "the receiver cannot write there").await?;
                Err(e)
            }
        }
    }
    .await;
    let (dest, dir) = match chosen {
        Ok(chosen) => chosen,
        Err(e) => {
            iroh.close().await;
            return Err(e);
        }
    };
    protocol::send(wormhole, &AppMessage::Answer(Answer::FileAck("ok".into()))).await?;
    let mut pipe = iroh.connect(&theirs, wormhole.key()).await?;
    eprintln!("Receiving ({})..", pipe.describe());
    let bar = transfer::progress(offer.numbytes, global.hide_progress, global.ascii, false);
    transfer::follow_route(&pipe, &bar);
    let received = async {
        let listed = pipe
            .fetch_manifest(&cache_dir()?, offer.numbytes, offer.numfiles)
            .await?;
        let manifest = listed.manifest();
        let paths = manifest.files.iter().map(|f| &f.path).chain(&manifest.dirs);
        if let Some(bad) = paths
            .into_iter()
            .find(|p| !zipdir::holdable(Path::new(p), cfg!(windows)))
        {
            let why = format!("{} cannot be written here", printable(bad));
            return Err(listed.refuse(&why).await.into());
        }
        let fetched = listed.fetch(&pipe, &mut |n| bar.set_position(n)).await?;
        transfer::finale(&bar).await;
        fetched.export_into(&dest).await?;
        anyhow::Ok(())
    }
    .await;
    let mut pipe = Pipe::Iroh(pipe);
    let done = match received {
        Ok(()) => {
            dir.keep();
            if let Some(went) = pipe.went() {
                eprintln!("{went}");
            }
            transfer::send_ack(&mut pipe, None).await
        }
        Err(e) => Err(e),
    };
    match done {
        Ok(()) => {
            pipe.shutdown().await;
            eprintln!("Received files written to {}", dest.display());
            Ok(())
        }
        Err(e) => {
            bar.finish_and_clear();
            pipe.abort().await;
            Err(e)
        }
    }
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

    fn finish(&self, dest: &Path) -> anyhow::Result<()> {
        settle(std::fs::hard_link(&self.path, dest), &self.path, dest)
    }
}

fn settle(linked: std::io::Result<()>, partial: &Path, dest: &Path) -> anyhow::Result<()> {
    let appeared = || {
        format!(
            "{} appeared during the transfer; the data is discarded",
            dest.display()
        )
    };
    match linked {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Err(e).with_context(appeared),
        Err(_) if dest.symlink_metadata().is_ok() => bail!(appeared()),
        Err(_) => std::fs::rename(partial, dest)
            .with_context(|| format!("moving the file to {}", dest.display())),
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
    crate::usable_name(&name, cfg!(windows)).then_some(name)
}

async fn confirm(question: &str) -> anyhow::Result<bool> {
    if !std::io::stdin().is_terminal() {
        bail!("not asking for confirmation without a terminal: pass --accept-file");
    }
    drop_typed_ahead();
    eprint!("{question}");
    let (answered, answer) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        let _ = answered.send(
            std::io::stdin()
                .read_line(&mut line)
                .map(|read| (read, line)),
        );
    });
    let (read, line) = answer
        .await
        .context("reading the answer")?
        .context("reading the answer")?;
    Ok(read > 0 && consents(&line))
}

#[cfg(unix)]
fn drop_typed_ahead() {
    let _ = rustix::termios::tcflush(std::io::stdin(), rustix::termios::QueueSelector::IFlush);
}

#[cfg(not(unix))]
fn drop_typed_ahead() {}

fn consents(answer: &str) -> bool {
    matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "" | "y" | "yes"
    )
}

async fn prompt_code(words: usize) -> anyhow::Result<Code> {
    const PROMPT: &str = "Enter receive wormhole code: ";
    if !std::io::stdin().is_terminal() {
        eprint!("{PROMPT}");
        let mut line = String::new();
        BufReader::new(tokio::io::stdin())
            .read_line(&mut line)
            .await
            .context("reading the code")?;
        return Ok(line.trim().parse()?);
    }
    let line = tokio::task::spawn_blocking(move || {
        let mut editor =
            rustyline::Editor::<CodeCompleter, rustyline::history::DefaultHistory>::new()?;
        editor.set_helper(Some(CodeCompleter { words }));
        editor.readline(PROMPT)
    })
    .await
    .context("reading the code")?
    .context("reading the code")?;
    Ok(line.trim().parse()?)
}

#[derive(rustyline::Helper, rustyline::Hinter, rustyline::Highlighter, rustyline::Validator)]
struct CodeCompleter {
    words: usize,
}

impl rustyline::completion::Completer for CodeCompleter {
    type Candidate = String;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        _: &rustyline::Context<'_>,
    ) -> rustyline::Result<(usize, Vec<String>)> {
        Ok((
            0,
            wyrmyon_wormhole::code::completions(&line[..pos], self.words),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn without_hard_links_the_partial_file_is_renamed_unless_something_appeared() {
        let dir = tempfile::tempdir().unwrap();
        let (partial, dest) = (dir.path().join("p"), dir.path().join("d"));
        let unsupported = || Err(std::io::Error::from(std::io::ErrorKind::Unsupported));
        std::fs::write(&partial, b"data").unwrap();
        settle(unsupported(), &partial, &dest).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"data");

        std::fs::write(&partial, b"more").unwrap();
        assert!(settle(unsupported(), &partial, &dest).is_err());
        let exists = Err(std::io::Error::from(std::io::ErrorKind::AlreadyExists));
        assert!(settle(exists, &partial, &dest).is_err());
        assert!(settle(unsupported(), &partial, &dir.path().join("no/such")).is_err());
        assert_eq!(std::fs::read(&dest).unwrap(), b"data");
    }

    #[test]
    fn enter_alone_consents_and_anything_but_yes_refuses() {
        for yes in ["\n", "y\n", "Y", " yes ", "YES\r\n"] {
            assert!(consents(yes), "{yes:?}");
        }
        for no in ["n\n", "no", "N", "yep", "ok", "q"] {
            assert!(!consents(no), "{no:?}");
        }
    }

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
