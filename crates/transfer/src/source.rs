use std::path::{Path, PathBuf};
use std::pin::Pin;

pub use wyrmyon_transport_iroh::Reader;

pub type Opened<'a> = Pin<Box<dyn Future<Output = std::io::Result<Reader>> + Send + 'a>>;

pub trait Source: Send + Sync {
    fn size(&self) -> u64;
    fn path(&self) -> Option<&Path> {
        None
    }
    fn open(&self) -> Opened<'_>;
}

pub struct FileSource {
    path: PathBuf,
    size: u64,
}

impl FileSource {
    pub fn new(path: PathBuf, size: u64) -> Self {
        Self { path, size }
    }
}

impl Source for FileSource {
    fn size(&self) -> u64 {
        self.size
    }

    fn path(&self) -> Option<&Path> {
        Some(&self.path)
    }

    fn open(&self) -> Opened<'_> {
        Box::pin(async {
            let file = tokio::fs::File::open(&self.path).await.map_err(|e| {
                std::io::Error::new(e.kind(), format!("opening {}: {e}", self.path.display()))
            })?;
            Ok(Box::new(file) as Reader)
        })
    }
}
