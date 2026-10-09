use std::path::{Path, PathBuf};

use futures_util::StreamExt;
use iroh_blobs::api::blobs::{AddPathOptions, BlobStatus, ExportMode, ExportOptions, ImportMode};
use iroh_blobs::api::remote::GetProgressItem;
use iroh_blobs::api::{Store, TempTag};
use iroh_blobs::provider::events::EventSender;
use iroh_blobs::store::fs::FsStore;
use iroh_blobs::{BlobFormat, Hash};

use crate::{Error, IrohPipe};

pub struct Offered {
    store: FsStore,
    tag: TempTag,
    _dir: tempfile::TempDir,
}

pub struct Fetched {
    store: FsStore,
    hash: Hash,
    dir: PathBuf,
}

impl Offered {
    pub async fn import(path: &Path) -> Result<Self, Error> {
        let dir = tempfile::tempdir().map_err(|e| Error::Blob(e.to_string()))?;
        let store = FsStore::load(dir.path())
            .await
            .map_err(|e| Error::Blob(e.to_string()))?;
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
            tag,
            _dir: dir,
        })
    }

    #[must_use]
    pub fn hash(&self) -> [u8; 32] {
        *self.tag.hash().as_bytes()
    }

    pub async fn close(self) {
        drop(self.tag);
        let _ = self.store.shutdown().await;
    }
}

impl Fetched {
    pub async fn export_to(self, target: &Path) -> Result<(), Error> {
        let exported = async {
            if tokio::fs::symlink_metadata(target).await.is_ok() {
                tokio::fs::remove_file(target).await?;
            }
            self.store
                .blobs()
                .export_with_opts(ExportOptions {
                    hash: self.hash,
                    mode: ExportMode::TryReference,
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
            Err(e) => Err(self.keep(e).await),
        }
    }

    async fn discard(self) {
        let _ = self.store.shutdown().await;
        let _ = tokio::fs::remove_dir_all(&self.dir).await;
    }

    async fn keep(self, cause: impl std::fmt::Display) -> Error {
        let _ = self.store.shutdown().await;
        Error::Blob(format!(
            "{cause}; what arrived is kept in {} for the next attempt (delete it to free the space)",
            self.dir.display()
        ))
    }
}

impl IrohPipe {
    pub async fn provide(&mut self, offered: &Offered) -> Result<(), Error> {
        self.send
            .write_all(&offered.hash())
            .await
            .map_err(|e| Error::Stream(e.to_string()))?;
        let store: Store = (*offered.store).clone();
        let connection = self.connection.clone();
        self.server = Some(tokio::spawn(async move {
            while let Ok((send, recv)) = connection.accept_bi().await {
                let pair = iroh_blobs::provider::StreamPair::new(
                    connection.stable_id() as u64,
                    recv,
                    send,
                    EventSender::DEFAULT,
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
        let mut hash = [0u8; 32];
        self.recv
            .read_exact(&mut hash)
            .await
            .map_err(|e| Error::Stream(e.to_string()))?;
        let hash = Hash::from_bytes(hash);
        let dir = cache.join(hash.to_hex());
        let store = FsStore::load(&dir)
            .await
            .map_err(|e| Error::Blob(e.to_string()))?;
        let fetched = Fetched { store, hash, dir };
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
