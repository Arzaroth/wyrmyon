use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures_util::StreamExt;
use iroh_blobs::api::blobs::{AddPathOptions, BlobStatus, ExportMode, ExportOptions, ImportMode};
use iroh_blobs::api::remote::GetProgressItem;
use iroh_blobs::api::{Store, TempTag};
use iroh_blobs::provider::events::{
    EventMask, EventSender, ProviderMessage, RequestMode, RequestUpdate,
};
use iroh_blobs::store::fs::FsStore;
use iroh_blobs::{BlobFormat, Hash};

use crate::{Error, IrohPipe};

pub struct Offered {
    pub(crate) store: FsStore,
    pub(crate) root: Hash,
    pub(crate) payload: Arc<HashSet<Hash>>,
    pub(crate) tags: Vec<TempTag>,
    pub(crate) _dir: tempfile::TempDir,
}

pub struct Fetched {
    pub(crate) store: FsStore,
    pub(crate) hash: Hash,
    pub(crate) dir: PathBuf,
}

pub(crate) async fn throwaway_store() -> Result<(tempfile::TempDir, FsStore), Error> {
    let dir = tempfile::tempdir().map_err(|e| Error::Blob(e.to_string()))?;
    let store = FsStore::load(dir.path())
        .await
        .map_err(|e| Error::Blob(e.to_string()))?;
    Ok((dir, store))
}

impl Offered {
    pub async fn import(path: &Path) -> Result<Self, Error> {
        let (dir, store) = throwaway_store().await?;
        let absolute = std::path::absolute(path).map_err(|e| Error::Blob(e.to_string()))?;
        let added = store
            .blobs()
            .add_path_with_opts(AddPathOptions {
                path: absolute,
                format: BlobFormat::Raw,
                mode: ImportMode::TryReference,
            })
            .temp_tag()
            .await;
        let tag = match added {
            Ok(tag) => tag,
            Err(e) => {
                let _ = store.shutdown().await;
                return Err(Error::Blob(e.to_string()));
            }
        };
        Ok(Self {
            store,
            root: tag.hash(),
            payload: Arc::new(HashSet::from([tag.hash()])),
            tags: vec![tag],
            _dir: dir,
        })
    }

    #[must_use]
    pub fn hash(&self) -> [u8; 32] {
        *self.root.as_bytes()
    }

    pub async fn close(self) {
        drop(self.tags);
        let _ = self.store.shutdown().await;
    }
}

impl Fetched {
    pub async fn export_to(self, target: &Path) -> Result<(), Error> {
        let exported = async {
            self.begin_export().await?;
            if tokio::fs::symlink_metadata(target).await.is_ok() {
                tokio::fs::remove_file(target).await?;
            }
            self.store
                .blobs()
                .export_with_opts(ExportOptions {
                    hash: self.hash,
                    mode: if cfg!(windows) {
                        ExportMode::Copy
                    } else {
                        ExportMode::TryReference
                    },
                    target: std::path::absolute(target)?,
                })
                .await
                .map_err(std::io::Error::other)?;
            tokio::fs::OpenOptions::new()
                .write(true)
                .open(target)
                .await?
                .sync_all()
                .await
        }
        .await;
        match exported {
            Ok(()) => {
                self.discard().await;
                Ok(())
            }
            Err(e) => Err(self.abandon(e).await),
        }
    }

    pub(crate) async fn begin_export(&self) -> std::io::Result<()> {
        tokio::fs::write(exporting(&self.dir), b"").await
    }

    pub(crate) async fn discard(self) {
        let Self { store, dir, .. } = self;
        let _ = store.shutdown().await;
        drop(store);
        remove_cache(&dir).await;
    }

    pub(crate) async fn abandon(self, cause: impl std::fmt::Display) -> Error {
        self.discard().await;
        Error::Blob(format!(
            "{cause}; the export may have moved part of the data, so what arrived is discarded"
        ))
    }

    pub(crate) async fn keep(self, cause: impl std::fmt::Display) -> Error {
        let _ = self.store.shutdown().await;
        Error::Blob(format!(
            "{cause}; what arrived is kept in {} for the next attempt (delete it to free the space)",
            self.dir.display()
        ))
    }
}

fn exporting(dir: &Path) -> PathBuf {
    dir.with_extension("exporting")
}

async fn remove_cache(dir: &Path) {
    for _ in 0..40 {
        if tokio::fs::remove_dir_all(dir).await.is_ok() || !dir.exists() {
            let _ = tokio::fs::remove_file(exporting(dir)).await;
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

impl IrohPipe {
    pub(crate) async fn open_fetch(&mut self, cache: &Path) -> Result<Fetched, Error> {
        let mut hash = [0u8; 32];
        self.recv
            .read_exact(&mut hash)
            .await
            .map_err(|e| Error::Stream(e.to_string()))?;
        let hash = Hash::from_bytes(hash);
        let dir = cache.join(hash.to_hex());
        if tokio::fs::try_exists(exporting(&dir))
            .await
            .unwrap_or(false)
        {
            remove_cache(&dir).await;
        }
        let store = FsStore::load(&dir)
            .await
            .map_err(|e| Error::Blob(e.to_string()))?;
        Ok(Fetched { store, hash, dir })
    }

    pub async fn provide(&mut self, offered: &Offered) -> Result<(), Error> {
        self.send
            .write_all(&offered.hash())
            .await
            .map_err(|e| Error::Stream(e.to_string()))?;
        let store: Store = (*offered.store).clone();
        let connection = self.connection.clone();
        let mask = EventMask {
            get: RequestMode::NotifyLog,
            get_many: RequestMode::NotifyLog,
            ..EventMask::DEFAULT
        };
        let (events, requests) = EventSender::channel(32, mask);
        tokio::spawn(count_sent(
            requests,
            Arc::clone(&offered.payload),
            self.sent.clone(),
        ));
        self.server = Some(tokio::spawn(async move {
            while let Ok((send, recv)) = connection.accept_bi().await {
                let pair = iroh_blobs::provider::StreamPair::new(
                    connection.stable_id() as u64,
                    recv,
                    send,
                    events.clone(),
                );
                tokio::spawn(iroh_blobs::provider::handle_stream(pair, store.clone()));
            }
        }));
        Ok(())
    }

    pub async fn fetch(
        &mut self,
        cache: &Path,
        size: u64,
        progress: &mut (dyn FnMut(u64) + Send),
    ) -> Result<Fetched, Error> {
        let fetched = self.open_fetch(cache).await?;
        let hash = fetched.hash;
        let local = fetched
            .store
            .remote()
            .local(hash)
            .await
            .map_or(0, |local| local.local_bytes());
        progress(local);
        let mut stream = fetched
            .store
            .remote()
            .fetch(self.connection.clone(), hash)
            .stream();
        let outcome = loop {
            match stream.next().await {
                Some(GetProgressItem::Progress(bytes)) if local + bytes > size => {
                    break Err(None);
                }
                Some(GetProgressItem::Progress(bytes)) => progress(local + bytes),
                Some(GetProgressItem::Done(_)) => break Ok(()),
                Some(GetProgressItem::Error(e)) => break Err(Some(e.to_string())),
                None => break Err(Some("the transfer stopped early".to_owned())),
            }
        };
        drop(stream);
        let complete = match outcome {
            Err(Some(interrupted)) => return Err(fetched.keep(interrupted).await),
            Err(None) => false,
            Ok(()) => matches!(
                fetched.store.blobs().status(hash).await,
                Ok(BlobStatus::Complete { size: got }) if got == size
            ),
        };
        if complete {
            Ok(fetched)
        } else {
            fetched.discard().await;
            Err(Error::Blob(format!(
                "the data does not match the {size} bytes offered"
            )))
        }
    }
}

async fn count_sent(
    mut requests: tokio::sync::mpsc::Receiver<ProviderMessage>,
    payload: Arc<HashSet<Hash>>,
    sent: tokio::sync::watch::Sender<u64>,
) {
    while let Some(request) = requests.recv().await {
        let mut updates = match request {
            ProviderMessage::GetRequestReceivedNotify(request) => request.rx,
            ProviderMessage::GetManyRequestReceivedNotify(request) => request.rx,
            _ => continue,
        };
        let (payload, sent) = (Arc::clone(&payload), sent.clone());
        tokio::spawn(async move {
            let (mut counting, mut reached, mut size) = (false, 0, 0);
            let credit = |counting: bool, reached: &mut u64, end: u64| {
                if counting && end > *reached {
                    sent.send_modify(|n| *n += end - *reached);
                    *reached = end;
                }
            };
            while let Ok(Some(update)) = updates.recv().await {
                match update {
                    RequestUpdate::Started(started) => {
                        credit(counting, &mut reached, size);
                        counting = payload.contains(&started.hash);
                        (reached, size) = (0, started.size);
                    }
                    RequestUpdate::Progress(progress) => {
                        credit(counting, &mut reached, progress.end_offset);
                    }
                    RequestUpdate::Completed(_) => credit(counting, &mut reached, size),
                    RequestUpdate::Aborted(_) => counting = false,
                }
            }
        });
    }
}
