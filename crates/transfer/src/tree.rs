use std::io::Read;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, bail};
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc;
use tokio_util::io::SyncIoBridge;
use wyrmyon_transport_iroh::{MAX_FILES, Opening, Traffic};
use wyrmyon_wormhole::Wormhole;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::events::{Cancelled, Events};
use crate::protocol::{self, CollectionOffer, DirectoryOffer, Offer};
use crate::send::unreadable;
use crate::{
    Connection, FileSource, Hashing, Options, Pipe, Received, Sink, Source, Staged, receive_into,
    refuse, require_iroh, send_data, send_over, speaks_iroh,
};

pub const ZIP_MODE: &str = "zipfile/deflated";
const ZIP64_FROM: u64 = 0xF000_0000;
const CHUNK: usize = 1 << 16;

pub struct Member {
    pub name: String,
    pub mode: u32,
    pub source: Arc<dyn Source>,
}

pub struct Tree {
    pub name: String,
    pub bundle: bool,
    pub members: Vec<Member>,
    pub dirs: Vec<String>,
}

impl Tree {
    pub fn numbytes(&self) -> u64 {
        self.members.iter().map(|m| m.source.size()).sum()
    }

    pub fn top_names(&self) -> Vec<String> {
        let files = self.members.iter().map(|m| match m.name.split_once('/') {
            Some((first, _)) => format!("{first}/"),
            None => m.name.clone(),
        });
        let dirs = self.dirs.iter().map(|dir| {
            let first = dir.split_once('/').map_or(dir.as_str(), |(first, _)| first);
            format!("{first}/")
        });
        let top: std::collections::BTreeSet<String> = files.chain(dirs).collect();
        top.into_iter().collect()
    }

    fn names(&self) -> Vec<String> {
        if self.bundle {
            self.top_names()
        } else {
            Vec::new()
        }
    }
}

pub struct Zipped {
    file: tempfile::NamedTempFile,
    pub zipsize: u64,
    pub numbytes: u64,
    pub numfiles: u64,
}

pub async fn zip_tree(
    tree: &Arc<Tree>,
    scratch: &Path,
    events: &dyn Events,
) -> anyhow::Result<Zipped> {
    let file = tempfile::NamedTempFile::new_in(scratch).context("creating the zip file")?;
    let writer = file.reopen().context("opening the zip file")?;
    let stop = Arc::new(AtomicBool::new(false));
    let building = {
        let (tree, stop) = (Arc::clone(tree), Arc::clone(&stop));
        let handle = tokio::runtime::Handle::current();
        tokio::task::spawn_blocking(move || build_zip(&tree, writer, &handle, &stop))
    };
    let built = tokio::select! {
        built = building => built.context("building the zip file")?,
        () = events.cancelled() => {
            stop.store(true, Ordering::Relaxed);
            return Err(Cancelled.into());
        }
    };
    let (numbytes, numfiles) = built?;
    let zipsize = file.as_file().metadata()?.len();
    Ok(Zipped {
        file,
        zipsize,
        numbytes,
        numfiles,
    })
}

fn build_zip(
    tree: &Tree,
    writer: std::fs::File,
    handle: &tokio::runtime::Handle,
    stop: &AtomicBool,
) -> anyhow::Result<(u64, u64)> {
    let mut zip = ZipWriter::new(writer);
    for dir in &tree.dirs {
        zip.add_directory(dir.as_str(), SimpleFileOptions::default())?;
    }
    let (mut numbytes, mut numfiles) = (0, 0);
    for member in &tree.members {
        let size = member.source.size();
        let options = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Deflated)
            .unix_permissions(member.mode)
            .large_file(size >= ZIP64_FROM);
        zip.start_file(member.name.as_str(), options)?;
        let reader = handle
            .block_on(member.source.open())
            .with_context(|| format!("opening {}", member.name))?;
        let reader = SyncIoBridge::new_with_handle(reader, handle.clone());
        let mut reader = Stoppable {
            inner: reader,
            stop,
        }
        .take(size.saturating_add(1));
        let copied = std::io::copy(&mut reader, &mut zip)
            .with_context(|| format!("reading {}", member.name))?;
        if copied != size {
            bail!("{} changed while it was being read", member.name);
        }
        numbytes += copied;
        numfiles += 1;
    }
    zip.finish().context("writing the zip file")?;
    Ok((numbytes, numfiles))
}

struct Stoppable<'a, R> {
    inner: R,
    stop: &'a AtomicBool,
}

impl<R: Read> Read for Stoppable<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.stop.load(Ordering::Relaxed) {
            return Err(std::io::Error::other(Cancelled));
        }
        self.inner.read(buf)
    }
}

pub async fn send_tree(
    wormhole: &mut Wormhole,
    options: &Options,
    staged: &mut Staged,
    zipped: &mut Option<Zipped>,
    tree: &Arc<Tree>,
    scratch: &Path,
    events: &Arc<dyn Events>,
) -> anyhow::Result<Option<Traffic>> {
    if options.use_collection(wormhole) {
        if staged.collection.is_none() {
            let hashing = staged
                .collection_ahead
                .take()
                .unwrap_or_else(|| Hashing::tree(Arc::clone(tree)));
            let (offered, manifest) = match hashing.finish(events.as_ref()).await {
                Ok(imported) => imported,
                Err(e) => {
                    protocol::send_error(wormhole, unreadable(&e)).await?;
                    return Err(e);
                }
            };
            let offer = CollectionOffer {
                name: tree.name.clone(),
                numbytes: manifest.numbytes(),
                numfiles: manifest.files.len() as u64,
                bundle: tree.bundle,
                names: tree.names(),
            };
            staged.collection = Some((offered, offer));
        }
        let (offered, offer) = staged
            .collection
            .as_ref()
            .expect("the collection was just imported");
        return send_over(
            wormhole,
            options,
            Offer::Collection(offer.clone()),
            None,
            offer.numbytes,
            Some(offered),
            events,
        )
        .await;
    }
    staged.drop_collection_ahead().await;
    require_iroh(wormhole, options).await?;
    if zipped.is_none() {
        match zip_tree(tree, scratch, events.as_ref()).await {
            Ok(built) => *zipped = Some(built),
            Err(e) => {
                protocol::send_error(wormhole, unreadable(&e)).await?;
                return Err(e);
            }
        }
    }
    let built = zipped.as_ref().expect("the zip file was just built");
    let bundle = tree.bundle && speaks_iroh(wormhole);
    let offer = DirectoryOffer {
        mode: ZIP_MODE.into(),
        dirname: tree.name.clone(),
        zipsize: built.zipsize,
        numbytes: built.numbytes,
        numfiles: built.numfiles,
        bundle,
        names: if bundle { tree.names() } else { Vec::new() },
    };
    let source: Arc<dyn Source> =
        Arc::new(FileSource::new(built.file.path().to_owned(), built.zipsize));
    send_data(
        wormhole,
        options,
        staged,
        Offer::Directory(offer),
        &source,
        events,
    )
    .await
}

pub fn usable_path(path: &str) -> bool {
    !path.is_empty()
        && path.split('/').all(|part| {
            !matches!(part, "" | "." | "..")
                && !part.contains('\\')
                && !part.chars().any(unprintable)
        })
}

pub fn unprintable(c: char) -> bool {
    c.is_control()
        || matches!(c, '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}

pub async fn receive_tree(
    wormhole: &mut Wormhole,
    connection: Connection,
    offer: &Offer,
    cache: &Path,
    open: &mut (dyn FnMut(&str, u32) -> Opening + Send),
    events: &Arc<dyn Events>,
) -> anyhow::Result<Received> {
    match offer {
        Offer::Collection(offer) => {
            receive_collection(wormhole, connection, offer, cache, open, events).await
        }
        Offer::Directory(offer) => {
            receive_zip(wormhole, connection, offer, cache, open, events).await
        }
        _ => {
            connection.close().await;
            refuse(wormhole, "this is not a directory or several files").await
        }
    }
}

async fn receive_collection(
    wormhole: &mut Wormhole,
    connection: Connection,
    offer: &CollectionOffer,
    cache: &Path,
    open: &mut (dyn FnMut(&str, u32) -> Opening + Send),
    events: &Arc<dyn Events>,
) -> anyhow::Result<Received> {
    let Connection::Iroh(iroh, theirs) = connection else {
        connection.close().await;
        return refuse(wormhole, "the sender offered a collection without iroh").await;
    };
    if offer.numfiles > MAX_FILES {
        iroh.close().await;
        return refuse(wormhole, "the sender offered too many files").await;
    }
    let mut pipe = Connection::Iroh(iroh, theirs)
        .accept(wormhole, events.as_ref())
        .await?;
    events.connected(&pipe, offer.numbytes);
    let received = async {
        let Pipe::Iroh(iroh) = &mut pipe else {
            unreachable!("a collection arrives over iroh")
        };
        let listed = iroh
            .fetch_manifest(cache, offer.numbytes, offer.numfiles)
            .await?;
        let manifest = listed.manifest();
        let paths = manifest.files.iter().map(|f| &f.path).chain(&manifest.dirs);
        if let Some(bad) = paths.into_iter().find(|p| !usable_path(p)) {
            let why = format!("{} cannot be written here", bad.escape_debug());
            return Err(listed.refuse(&why).await.into());
        }
        let fetched = listed.fetch(iroh, &mut |n| events.moved(n)).await?;
        events.settle().await;
        fetched
            .export_each(&mut |file| open(&file.path, file.mode))
            .await?;
        anyhow::Ok(())
    };
    let received = tokio::select! {
        received = received => received,
        () = events.cancelled() => Err(Cancelled.into()),
    };
    match received {
        Ok(()) => Ok(Received { digest: None, pipe }),
        Err(e) => {
            events.stopped();
            pipe.abort().await;
            Err(e)
        }
    }
}

async fn receive_zip(
    wormhole: &mut Wormhole,
    connection: Connection,
    offer: &DirectoryOffer,
    cache: &Path,
    open: &mut (dyn FnMut(&str, u32) -> Opening + Send),
    events: &Arc<dyn Events>,
) -> anyhow::Result<Received> {
    if offer.mode != ZIP_MODE {
        connection.close().await;
        return refuse(wormhole, "unknown directory transfer mode").await;
    }
    let zip = match scratch_zip(cache).await {
        Ok(zip) => zip,
        Err(e) => {
            connection.close().await;
            return Err(e);
        }
    };
    let file = tokio::fs::File::from_std(zip.reopen().context("opening the zip file")?);
    let sink = Sink::File {
        file,
        path: zip.path(),
    };
    let received = receive_into(
        wormhole,
        connection,
        offer.zipsize,
        Ok(cache.to_owned()),
        sink,
        events,
    )
    .await?;
    let limits = Limits {
        numbytes: offer.numbytes,
        numfiles: offer.numfiles,
    };
    let unpacked = match std::fs::File::open(zip.path()).context("opening the zip file") {
        Ok(file) => tokio::select! {
            unpacked = unzip(file, limits, open) => unpacked,
            () = events.cancelled() => Err(Cancelled.into()),
        },
        Err(e) => Err(e),
    };
    match unpacked {
        Ok(()) => Ok(received),
        Err(e) => {
            events.stopped();
            Err(received.complete(Err(e)).await.unwrap_err())
        }
    }
}

async fn scratch_zip(cache: &Path) -> anyhow::Result<tempfile::NamedTempFile> {
    tokio::fs::create_dir_all(cache)
        .await
        .context("creating the cache")?;
    tempfile::NamedTempFile::new_in(cache).context("creating the zip file")
}

struct Limits {
    numbytes: u64,
    numfiles: u64,
}

enum Unzipped {
    Entry(String, u32),
    Data(Vec<u8>),
    End,
}

async fn unzip(
    zip: std::fs::File,
    limits: Limits,
    open: &mut (dyn FnMut(&str, u32) -> Opening + Send),
) -> anyhow::Result<()> {
    let (tx, mut rx) = mpsc::channel(8);
    let stop = Arc::new(AtomicBool::new(false));
    let reading = {
        let stop = Arc::clone(&stop);
        tokio::task::spawn_blocking(move || read_zip(zip, &limits, &tx, &stop))
    };
    let written = async {
        let mut writer = None;
        while let Some(next) = rx.recv().await {
            match next {
                Unzipped::Entry(name, mode) => {
                    finish(writer.take()).await?;
                    writer = Some(open(&name, mode).await?);
                }
                Unzipped::Data(data) => {
                    if let Some(writer) = writer.as_mut() {
                        writer.write_all(&data).await?;
                    }
                }
                Unzipped::End => finish(writer.take()).await?,
            }
        }
        std::io::Result::Ok(())
    }
    .await;
    if let Err(e) = written {
        stop.store(true, Ordering::Relaxed);
        rx.close();
        let _ = reading.await;
        return Err(e).context("writing the received files");
    }
    reading.await.context("unpacking the zip file")?
}

async fn finish(writer: Option<wyrmyon_transport_iroh::Writer>) -> std::io::Result<()> {
    if let Some(mut writer) = writer {
        writer.shutdown().await?;
    }
    Ok(())
}

fn read_zip(
    zip: std::fs::File,
    limits: &Limits,
    tx: &mpsc::Sender<Unzipped>,
    stop: &AtomicBool,
) -> anyhow::Result<()> {
    let send = |message| {
        tx.blocking_send(message)
            .map_err(|_| anyhow::anyhow!("the received files could not be written"))
    };
    let mut archive = ZipArchive::new(zip).context("the received data is not a zip file")?;
    let (mut bytes, mut files) = (0u64, 0u64);
    for index in 0..archive.len() {
        if stop.load(Ordering::Relaxed) {
            bail!("the received files could not be written");
        }
        let mut entry = archive.by_index(index)?;
        let enclosed = entry
            .enclosed_name()
            .and_then(|p| p.to_str().map(|p| p.replace('\\', "/")));
        if entry.is_dir() && enclosed.as_deref() == Some("") {
            continue;
        }
        let name = enclosed.filter(|p| usable_path(p));
        let Some(name) = name else {
            bail!(
                "the zip file has an entry outside its directory or unusable here: {}",
                entry.name().escape_debug()
            );
        };
        if entry.is_dir() || entry.is_symlink() {
            continue;
        }
        files += 1;
        if files > limits.numfiles {
            bail!(
                "the directory has more files than the {} offered",
                limits.numfiles
            );
        }
        let mode = entry.unix_mode().map_or(0o644, |m| m & 0o777);
        send(Unzipped::Entry(name, mode))?;
        let budget = limits.numbytes - bytes;
        let mut limited = (&mut entry).take(budget.saturating_add(1));
        let mut chunk = vec![0; CHUNK];
        loop {
            let n = limited.read(&mut chunk)?;
            if n == 0 {
                break;
            }
            bytes += n as u64;
            if bytes > limits.numbytes {
                bail!(
                    "the directory holds more than the {} bytes offered",
                    limits.numbytes
                );
            }
            send(Unzipped::Data(chunk[..n].to_vec()))?;
        }
    }
    send(Unzipped::End)?;
    Ok(())
}
