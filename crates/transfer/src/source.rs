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

#[cfg(test)]
mod tests {
    use tokio::io::AsyncReadExt;

    use super::*;

    struct Empty;

    impl Source for Empty {
        fn size(&self) -> u64 {
            0
        }

        fn open(&self) -> Opened<'_> {
            Box::pin(async { Ok(Box::new(tokio::io::empty()) as Reader) })
        }
    }

    #[tokio::test]
    async fn a_file_source_reads_its_file_and_names_it_when_it_cannot() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        std::fs::write(&path, b"hello").unwrap();
        let source = FileSource::new(path.clone(), 5);
        assert_eq!((source.size(), source.path()), (5, Some(path.as_path())));
        let mut read = String::new();
        source
            .open()
            .await
            .unwrap()
            .read_to_string(&mut read)
            .await
            .unwrap();
        assert_eq!(read, "hello");

        let missing = FileSource::new(dir.path().join("gone"), 1);
        let said = missing.open().await.err().unwrap().to_string();
        assert!(
            said.starts_with("opening ") && said.contains("gone"),
            "{said}"
        );
        assert_eq!(Empty.path(), None);
        assert_eq!(Empty.size(), 0);
        assert!(Empty.open().await.is_ok());
    }
}
