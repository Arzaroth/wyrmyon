use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use futures_util::StreamExt;
use iroh_blobs::api::TempTag;
use iroh_blobs::api::blobs::{AddPathOptions, BlobStatus, ExportMode, ExportOptions, ImportMode};
use iroh_blobs::api::remote::GetProgressItem;
use iroh_blobs::hashseq::HashSeq;
use iroh_blobs::protocol::{ChunkRanges, GetRequest};
use iroh_blobs::store::fs::FsStore;
use iroh_blobs::{BlobFormat, Hash, HashAndFormat};
use serde::{Deserialize, Serialize};

use crate::{Error, Fetched, IrohPipe, Offered};

pub const MAX_MANIFEST: u64 = 16 << 20;
pub const MAX_FILES: u64 = 1 << 20;
const MAX_DIRS: usize = 1 << 16;
const MAX_PATH: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub files: Vec<ManifestFile>,
    #[serde(default)]
    pub dirs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestFile {
    pub path: String,
    pub size: u64,
    #[serde(default = "default_mode")]
    pub mode: u32,
}

const fn default_mode() -> u32 {
    0o644
}

pub struct Source {
    pub path: PathBuf,
    pub name: String,
    pub mode: u32,
}

pub struct Collection {
    fetched: Fetched,
    manifest: Manifest,
    children: Vec<Hash>,
    overhead: u64,
}

impl Manifest {
    #[must_use]
    pub fn numbytes(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }

    fn check(&self, numbytes: u64, numfiles: u64) -> Result<(), String> {
        if self.files.len() as u64 != numfiles {
            return Err(format!(
                "it lists {} files, not the {numfiles} offered",
                self.files.len()
            ));
        }
        let total = self
            .files
            .iter()
            .try_fold(0u64, |sum, f| sum.checked_add(f.size));
        if total != Some(numbytes) {
            return Err(format!(
                "its sizes do not add up to the {numbytes} bytes offered"
            ));
        }
        if self.dirs.len() > MAX_DIRS {
            return Err(format!("it lists more than {MAX_DIRS} empty directories"));
        }
        let mut tree = Tree::default();
        let files = self.files.iter().map(|f| (&f.path, true));
        for (path, file) in files.chain(self.dirs.iter().map(|d| (d, false))) {
            tree.insert(path, file)?;
        }
        Ok(())
    }
}

#[derive(Default)]
struct Tree<'a> {
    file: bool,
    children: std::collections::HashMap<&'a str, Tree<'a>>,
}

impl<'a> Tree<'a> {
    fn insert(&mut self, path: &'a str, file: bool) -> Result<(), String> {
        if path.len() > MAX_PATH {
            return Err(format!("a path is longer than {MAX_PATH} bytes"));
        }
        if path.contains('\\') {
            return Err("a path holds a backslash".into());
        }
        let mut node = self;
        for part in path.split('/') {
            if matches!(part, "" | "." | "..") {
                return Err("it lists an empty, `.` or `..` path segment".into());
            }
            if node.file {
                return Err(format!("{path} sits under a file"));
            }
            node = node.children.entry(part).or_default();
        }
        if node.file || (file && !node.children.is_empty()) {
            return Err(format!(
                "{path} is listed twice, or as both a file and a directory"
            ));
        }
        node.file = file;
        Ok(())
    }
}

impl Offered {
    pub async fn import_collection(
        sources: &[Source],
        dirs: &[String],
    ) -> Result<(Self, Manifest), Error> {
        let (dir, store) = crate::blobs::throwaway_store().await?;
        match import_all(&store, sources, dirs).await {
            Ok((root, tags, manifest)) => Ok((
                Self {
                    store,
                    root,
                    tags,
                    _dir: dir,
                },
                manifest,
            )),
            Err(e) => {
                let _ = store.shutdown().await;
                Err(Error::Blob(e.to_string()))
            }
        }
    }
}

type Failed = Box<dyn std::error::Error + Send + Sync>;

async fn import_file(store: &FsStore, source: &Source) -> Result<(TempTag, u64), Failed> {
    let added = store
        .blobs()
        .add_path_with_opts(AddPathOptions {
            path: std::path::absolute(&source.path)?,
            format: BlobFormat::Raw,
            mode: ImportMode::TryReference,
        })
        .temp_tag()
        .await;
    let tag = match added {
        Ok(tag) => tag,
        Err(e) => return Err(format!("{}: {e}", source.path.display()).into()),
    };
    let size = store.remote().local(tag.hash()).await?.local_bytes();
    Ok((tag, size))
}

async fn import_all(
    store: &FsStore,
    sources: &[Source],
    dirs: &[String],
) -> Result<(Hash, Vec<TempTag>, Manifest), Failed> {
    let mut tags = Vec::with_capacity(sources.len() + 2);
    let mut files = Vec::with_capacity(sources.len());
    for source in sources {
        let (tag, size) = import_file(store, source).await?;
        files.push(ManifestFile {
            path: source.name.clone(),
            size,
            mode: source.mode,
        });
        tags.push(tag);
    }
    let manifest = Manifest {
        files,
        dirs: dirs.to_vec(),
    };
    let encoded = serde_json::to_vec(&manifest)?;
    let manifest_tag = store.blobs().add_bytes(encoded).temp_tag().await?;
    let seq: HashSeq = std::iter::once(manifest_tag.hash())
        .chain(tags.iter().map(TempTag::hash))
        .collect();
    let root_tag = store.blobs().add_bytes(seq.into_inner()).temp_tag().await?;
    let root = root_tag.hash();
    tags.push(manifest_tag);
    tags.push(root_tag);
    Ok((root, tags, manifest))
}

impl IrohPipe {
    pub async fn fetch_manifest(
        &mut self,
        cache: &Path,
        numbytes: u64,
        numfiles: u64,
    ) -> Result<Collection, Error> {
        if numfiles > MAX_FILES {
            return Err(Error::Blob(format!("more than {MAX_FILES} files offered")));
        }
        let fetched = self.open_fetch(cache).await?;
        let root_size = numfiles.saturating_add(1).saturating_mul(32);
        let request = GetRequest::builder()
            .root(ChunkRanges::all())
            .child(0, ChunkRanges::all())
            .build(fetched.hash);
        let got = get_bounded(
            &fetched,
            self.connection.clone(),
            request,
            root_size.saturating_add(MAX_MANIFEST),
            &mut |_| {},
        )
        .await;
        if let Err(e) = got {
            return Err(settle(fetched, e).await);
        }
        match read_manifest(&fetched, numbytes, numfiles).await {
            Ok((manifest, children, listed)) => Ok(Collection {
                fetched,
                manifest,
                children,
                overhead: root_size + listed,
            }),
            Err(why) => {
                fetched.discard().await;
                Err(Error::Blob(format!("the list of files is unusable: {why}")))
            }
        }
    }
}

enum Failure {
    Interrupted(String),
    Oversized,
}

async fn settle(fetched: Fetched, failure: Failure) -> Error {
    match failure {
        Failure::Interrupted(cause) => fetched.keep(cause).await,
        Failure::Oversized => {
            fetched.discard().await;
            Error::Blob("the data is bigger than what was offered".into())
        }
    }
}

async fn get_bounded(
    fetched: &Fetched,
    connection: iroh::endpoint::Connection,
    request: GetRequest,
    allowed: u64,
    progress: &mut (dyn FnMut(u64) + Send),
) -> Result<(), Failure> {
    let remote = fetched.store.remote();
    let local = remote
        .local_for_request(request.clone())
        .await
        .map_err(|e| Failure::Interrupted(e.to_string()))?;
    let already = local.local_bytes();
    progress(already);
    if local.is_complete() {
        return Ok(());
    }
    let mut stream = remote.execute_get(connection, local.missing()).stream();
    loop {
        match stream.next().await {
            Some(GetProgressItem::Progress(bytes)) if already + bytes > allowed => {
                return Err(Failure::Oversized);
            }
            Some(GetProgressItem::Progress(bytes)) => progress(already + bytes),
            Some(GetProgressItem::Done(_)) => return Ok(()),
            Some(GetProgressItem::Error(e)) => return Err(Failure::Interrupted(e.to_string())),
            None => return Err(Failure::Interrupted("the transfer stopped early".into())),
        }
    }
}

async fn read_manifest(
    fetched: &Fetched,
    numbytes: u64,
    numfiles: u64,
) -> Result<(Manifest, Vec<Hash>, u64), String> {
    let blobs = fetched.store.blobs();
    let root = blobs
        .get_bytes(fetched.hash)
        .await
        .ok()
        .ok_or("the root is missing")?;
    let seq = HashSeq::new(root).ok_or("the root is not a list of hashes")?;
    let children: Vec<Hash> = seq.iter().collect();
    if children.len() as u64 != numfiles.saturating_add(1) {
        return Err(format!(
            "it has {} entries, not the {numfiles} files offered",
            children.len().saturating_sub(1)
        ));
    }
    let encoded = blobs
        .get_bytes(children[0])
        .await
        .ok()
        .ok_or("the list is missing")?;
    let manifest: Manifest =
        serde_json::from_slice(&encoded).map_err(|e| format!("it is not valid: {e}"))?;
    manifest.check(numbytes, numfiles)?;
    Ok((manifest, children, encoded.len() as u64))
}

impl Collection {
    #[must_use]
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    pub async fn refuse(self, why: &str) -> Error {
        self.fetched.discard().await;
        Error::Blob(format!("the list of files is unusable: {why}"))
    }

    pub async fn fetch(
        self,
        pipe: &IrohPipe,
        progress: &mut (dyn FnMut(u64) + Send),
    ) -> Result<Self, Error> {
        let numbytes = self.manifest.numbytes();
        let overhead = self.overhead;
        let got = get_bounded(
            &self.fetched,
            pipe.connection.clone(),
            GetRequest::from(HashAndFormat::hash_seq(self.fetched.hash)),
            overhead.saturating_add(numbytes),
            &mut |n| progress(n.saturating_sub(overhead)),
        )
        .await;
        if let Err(e) = got {
            return Err(settle(self.fetched, e).await);
        }
        for (hash, file) in self.children[1..].iter().zip(&self.manifest.files) {
            let complete = matches!(
                self.fetched.store.blobs().status(*hash).await,
                Ok(BlobStatus::Complete { size }) if size == file.size
            );
            if !complete {
                self.fetched.discard().await;
                return Err(Error::Blob(format!(
                    "{} does not match the {} bytes listed",
                    file.path, file.size
                )));
            }
        }
        Ok(self)
    }

    pub async fn export_into(self, dir: &Path) -> Result<(), Error> {
        let exported = match self.fetched.begin_export().await {
            Ok(()) => self.export_all(dir).await,
            Err(e) => Err(e),
        };
        match exported {
            Ok(()) => {
                self.fetched.discard().await;
                Ok(())
            }
            Err(e) => Err(self.fetched.abandon(e).await),
        }
    }

    async fn export_all(&self, dir: &Path) -> std::io::Result<()> {
        let dir = std::path::absolute(dir)?;
        for empty in &self.manifest.dirs {
            tokio::fs::create_dir_all(dir.join(empty)).await?;
        }
        let mut seen = BTreeSet::new();
        for (hash, file) in self.children[1..].iter().zip(&self.manifest.files) {
            let target = dir.join(&file.path);
            if let Some(parent) = target.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
            let mode = if cfg!(windows) || !seen.insert(*hash) {
                ExportMode::Copy
            } else {
                ExportMode::TryReference
            };
            self.fetched
                .store
                .blobs()
                .export_with_opts(ExportOptions {
                    hash: *hash,
                    mode,
                    target: target.clone(),
                })
                .await
                .map_err(std::io::Error::other)?;
            tokio::fs::OpenOptions::new()
                .write(true)
                .open(&target)
                .await?
                .sync_all()
                .await?;
            set_mode(&target, file.mode & 0o777)?;
        }
        Ok(())
    }
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
}

#[cfg(not(unix))]
#[allow(clippy::unnecessary_wraps)]
fn set_mode(_path: &Path, _mode: u32) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str, size: u64) -> ManifestFile {
        ManifestFile {
            path: path.into(),
            size,
            mode: 0o644,
        }
    }

    #[test]
    fn a_manifest_must_match_its_offer_and_hold_together() {
        let ok = Manifest {
            files: vec![file("a", 1), file("d/b", 2)],
            dirs: vec!["d/e".into()],
        };
        assert_eq!(ok.check(3, 2), Ok(()));
        assert!(ok.check(3, 3).is_err());
        assert!(ok.check(4, 2).is_err());
        let bad = |files, dirs: &[&str]| Manifest {
            files,
            dirs: dirs.iter().map(|d| (*d).to_owned()).collect(),
        };
        for manifest in [
            bad(vec![file("a", 1), file("a", 2)], &[]),
            bad(vec![file("a", 1), file("a/b", 2)], &[]),
            bad(vec![file("a", 1), file("c", 2)], &["a"]),
            bad(vec![file("a", 1), file("c", 2)], &["a/x"]),
            bad(vec![file("", 1), file("c", 2)], &[]),
            bad(vec![file("a//b", 1), file("c", 2)], &[]),
            bad(vec![file("a", u64::MAX), file("c", 4)], &[]),
            bad(vec![file("d/b", 1), file("a", 2)], &["d/b/x"]),
            bad(vec![file("d/b", 1), file("d/./b", 2)], &[]),
            bad(vec![file("d/../a", 1), file("c", 2)], &[]),
            bad(vec![file("d\\b", 1), file("c", 2)], &[]),
            bad(vec![file(&"x".repeat(MAX_PATH + 1), 1), file("c", 2)], &[]),
        ] {
            assert!(manifest.check(3, 2).is_err(), "{manifest:?}");
        }
        let many: Vec<String> = (0..=MAX_DIRS).map(|n| n.to_string()).collect();
        let crowded = Manifest {
            files: vec![file("a", 3)],
            dirs: many,
        };
        assert!(crowded.check(3, 1).is_err());
        let repeated = Manifest {
            files: vec![file("a", 3)],
            dirs: vec!["e".into(), "e".into(), "d/e".into(), "d".into()],
        };
        assert_eq!(repeated.check(3, 1), Ok(()));
    }

    #[test]
    fn a_deep_path_is_checked_in_linear_time() {
        let deep = vec!["a"; MAX_PATH / 4].join("/");
        let manifest = Manifest {
            files: (0..64).map(|n| file(&format!("{deep}{n}"), 0)).collect(),
            dirs: Vec::new(),
        };
        let started = std::time::Instant::now();
        assert_eq!(manifest.check(0, 64), Ok(()));
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
    }

    #[test]
    fn a_manifest_reads_without_modes_or_dirs() {
        let manifest: Manifest =
            serde_json::from_str(r#"{"files": [{"path": "a", "size": 3}]}"#).unwrap();
        assert_eq!(manifest.files[0].mode, 0o644);
        assert_eq!(manifest.dirs, Vec::<String>::new());
        assert_eq!(manifest.numbytes(), 3);
    }

    async fn forged(dir: &Path, real: &[(&str, usize)], listed: &[u8]) -> Offered {
        let sources: Vec<Source> = real
            .iter()
            .map(|(name, len)| {
                let path = dir.join(name);
                std::fs::write(&path, vec![1u8; *len]).unwrap();
                Source {
                    path,
                    name: (*name).to_owned(),
                    mode: 0o644,
                }
            })
            .collect();
        let (mut offered, _) = Offered::import_collection(&sources, &[]).await.unwrap();
        let blobs = offered.store.blobs();
        let manifest = blobs.add_slice(listed).temp_tag().await.unwrap();
        let seq: HashSeq = std::iter::once(manifest.hash())
            .chain(offered.tags[..sources.len()].iter().map(TempTag::hash))
            .collect();
        let root = blobs.add_bytes(seq.into_inner()).temp_tag().await.unwrap();
        offered.root = root.hash();
        offered.tags.extend([manifest, root]);
        offered
    }

    async fn pair() -> (IrohPipe, IrohPipe) {
        let key = wyrmyon_wormhole::Key::from_bytes([3; 32]);
        let sender = crate::IrohTransport::bind(crate::Role::Sender, crate::Relays::Disabled)
            .await
            .unwrap();
        let receiver = crate::IrohTransport::bind(crate::Role::Receiver, crate::Relays::Disabled)
            .await
            .unwrap();
        let (sender_info, receiver_info) = (sender.info().await, receiver.info().await);
        let (a, b) = tokio::join!(
            sender.connect(&receiver_info, &key),
            receiver.connect(&sender_info, &key)
        );
        (a.unwrap(), b.unwrap())
    }

    async fn received(offered: &Offered, cache: &Path, numbytes: u64, numfiles: u64) -> String {
        let (mut upstream, mut downstream) = pair().await;
        upstream.provide(offered).await.unwrap();
        let outcome = match downstream.fetch_manifest(cache, numbytes, numfiles).await {
            Ok(collection) => collection.fetch(&downstream, &mut |_| {}).await.map(|_| ()),
            Err(e) => Err(e),
        };
        upstream.abort().await;
        downstream.abort().await;
        outcome.unwrap_err().to_string()
    }

    fn listing(files: &[(&str, u64)]) -> Vec<u8> {
        serde_json::to_vec(&Manifest {
            files: files.iter().map(|(p, s)| file(p, *s)).collect(),
            dirs: Vec::new(),
        })
        .unwrap()
    }

    type Lie = (
        &'static [(&'static str, usize)],
        Vec<u8>,
        u64,
        u64,
        &'static str,
    );

    #[tokio::test(flavor = "multi_thread")]
    async fn a_sender_lying_about_its_files_is_caught_and_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let cases: [Lie; 4] = [
            (
                &[("a", 10), ("b", 0)],
                listing(&[("a", 5), ("b", 5)]),
                10,
                2,
                "does not match",
            ),
            (&[("a", 100_000)], listing(&[("a", 10)]), 10, 1, "bigger"),
            (
                &[("a", 10)],
                listing(&[("a", 5), ("b", 5)]),
                10,
                2,
                "entries",
            ),
            (&[("a", 10)], b"not json".to_vec(), 10, 1, "not valid"),
        ];
        for (index, (real, listed, numbytes, numfiles, expected)) in cases.into_iter().enumerate() {
            let src = dir.path().join(format!("src-{index}"));
            std::fs::create_dir(&src).unwrap();
            let offered = forged(&src, real, &listed).await;
            let cache = dir.path().join(format!("cache-{index}"));
            let error = received(&offered, &cache, numbytes, numfiles).await;
            assert!(error.contains(expected), "{index}: {error}");
            assert_eq!(std::fs::read_dir(&cache).unwrap().count(), 0, "{index}");
            offered.close().await;
        }
    }
}
