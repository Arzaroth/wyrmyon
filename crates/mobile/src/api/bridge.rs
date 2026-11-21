use std::path::PathBuf;
use std::sync::Arc;

use flutter_rust_bridge::{DartFnFuture, frb};
use wyrmyon_transfer::{Opened, Reader, Source};

use crate::api::types::{OfferInfo, Settings, Update};
use crate::frb_generated::StreamSink;
use crate::report::Report;
use crate::{Incoming, Outgoing};

pub async fn send_text(
    settings: Settings,
    code: Option<String>,
    text: String,
    updates: StreamSink<Update>,
) -> anyhow::Result<()> {
    crate::send(&settings, code, Outgoing::Text(text), reporting(updates)).await
}

pub async fn send_file(
    settings: Settings,
    code: Option<String>,
    name: String,
    size: u64,
    open: impl Fn() -> DartFnFuture<i32> + Send + Sync + 'static,
    updates: StreamSink<Update>,
) -> anyhow::Result<()> {
    let source = Arc::new(Descriptor {
        size,
        open: Box::new(open),
    });
    let outgoing = Outgoing::File { name, source };
    crate::send(&settings, code, outgoing, reporting(updates)).await
}

pub async fn receive(
    settings: Settings,
    code: Option<String>,
    updates: StreamSink<Update>,
) -> anyhow::Result<Incoming> {
    crate::receive(&settings, code, reporting(updates)).await
}

#[frb(sync)]
pub fn offer_info(incoming: &Incoming) -> OfferInfo {
    incoming.info()
}

pub async fn accept(incoming: Incoming, fd: i32, cache_dir: String) -> anyhow::Result<()> {
    let mut file = tokio::fs::File::from_std(owned_file(fd)?);
    incoming.accept(&mut file, PathBuf::from(cache_dir)).await
}

pub async fn refuse(incoming: Incoming) -> anyhow::Result<()> {
    incoming.refuse().await
}

fn reporting(updates: StreamSink<Update>) -> Report {
    Arc::new(move |update| {
        let _ = updates.add(update);
    })
}

type Open = Box<dyn Fn() -> DartFnFuture<i32> + Send + Sync>;

struct Descriptor {
    size: u64,
    open: Open,
}

impl Source for Descriptor {
    fn size(&self) -> u64 {
        self.size
    }

    fn open(&self) -> Opened<'_> {
        Box::pin(async move {
            let fd = (self.open)().await;
            let file = owned_file(fd).map_err(std::io::Error::other)?;
            Ok(Box::new(tokio::fs::File::from_std(file)) as Reader)
        })
    }
}

#[cfg(unix)]
fn owned_file(fd: i32) -> anyhow::Result<std::fs::File> {
    use std::os::fd::{FromRawFd, OwnedFd};
    anyhow::ensure!(fd >= 0, "the file could not be opened");
    Ok(std::fs::File::from(unsafe { OwnedFd::from_raw_fd(fd) }))
}

#[cfg(not(unix))]
fn owned_file(_: i32) -> anyhow::Result<std::fs::File> {
    anyhow::bail!("files are handed over as descriptors only on Android")
}
