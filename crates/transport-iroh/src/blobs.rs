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

fn store_error(e: impl std::fmt::Display) -> Error {
    Error::Blob(e.to_string())
}

impl Offered {
    pub async fn import(path: &Path) -> Result<Self, Error> {
        let dir = tempfile::tempdir().map_err(store_error)?;
        let store = FsStore::load(dir.path()).await.map_err(store_error)?;
        let absolute = std::path::absolute(path).map_err(store_error)?;
        let tag = store
            .blobs()
            .add_path_with_opts(AddPathOptions {
                path: absolute,
                format: BlobFormat::Raw,
                mode: ImportMode::TryReference,
            })
            .temp_tag()
            .await
            .map_err(store_error)?;
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
    pub async fn export(&self, target: &Path) -> Result<(), Error> {
        let target = std::path::absolute(target).map_err(store_error)?;
        self.store
            .blobs()
            .export_with_opts(ExportOptions {
                hash: self.hash,
                mode: ExportMode::Copy,
                target,
            })
            .await
            .map_err(store_error)?;
        Ok(())
    }

    pub async fn export_to(self, target: &Path) -> Result<Self, Error> {
        let exported = async {
            if tokio::fs::symlink_metadata(target).await.is_ok() {
                tokio::fs::remove_file(target).await.map_err(store_error)?;
            }
            self.export(target).await
        }
        .await;
        match exported {
            Ok(()) => Ok(self),
            Err(e) => {
                self.discard().await;
                Err(e)
            }
        }
    }

    pub async fn discard(self) {
        let _ = self.store.shutdown().await;
        let _ = tokio::fs::remove_dir_all(&self.dir).await;
    }

    pub async fn keep(self) {
        let _ = self.store.shutdown().await;
    }
}

impl IrohPipe {
    pub async fn provide(&mut self, offered: &Offered) -> Result<(), Error> {
        self.send_chunk(&offered.hash()).await?;
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
        mut progress: impl FnMut(u64),
    ) -> Result<Fetched, Error> {
        let mut hash = [0u8; 32];
        let mut filled = 0;
        while filled < hash.len() {
            let chunk = self.receive_chunk(hash.len() - filled).await?;
            hash[filled..filled + chunk.len()].copy_from_slice(&chunk);
            filled += chunk.len();
        }
        let hash = Hash::from_bytes(hash);
        let dir = cache.join(hash.to_hex());
        let store = FsStore::load(&dir).await.map_err(store_error)?;
        let fetched = Fetched { store, hash, dir };
        let local = fetched
            .store
            .remote()
            .local(hash)
            .await
            .map_err(store_error)?
            .local_bytes();
        progress(local);
        let mut stream = fetched
            .store
            .remote()
            .fetch(self.connection.clone(), hash)
            .stream();
        while let Some(item) = stream.next().await {
            match item {
                GetProgressItem::Progress(bytes) => progress(local + bytes),
                GetProgressItem::Done(_) => break,
                GetProgressItem::Error(e) => {
                    fetched.keep().await;
                    return Err(store_error(e));
                }
            }
        }
        match fetched
            .store
            .blobs()
            .status(hash)
            .await
            .map_err(store_error)?
        {
            BlobStatus::Complete { size: got } if got == size => Ok(fetched),
            _ => {
                fetched.discard().await;
                Err(Error::Blob(format!(
                    "the data does not match the {size} bytes offered"
                )))
            }
        }
    }
}
