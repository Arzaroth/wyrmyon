use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use flutter_rust_bridge::{DartFnFuture, frb};
use wyrmyon_transfer::{Opened, Reader, Source};

use crate::api::types::{OfferInfo, Settings, Stage, Update};
use crate::frb_generated::StreamSink;
use crate::report::{Report, ended, tell};
use crate::{Cancel, Incoming, Outgoing};

#[frb(opaque)]
pub struct Transfer {
    cancel: Cancel,
    incoming: Mutex<Option<(Incoming, Report)>>,
}

impl Transfer {
    #[frb(sync)]
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {
            cancel: Cancel::new(),
            incoming: Mutex::new(None),
        }
    }

    #[frb(sync)]
    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    #[frb(sync)]
    pub fn offer(&self) -> Option<OfferInfo> {
        self.slot().as_ref().map(|(incoming, _)| incoming.info())
    }

    fn slot(&self) -> std::sync::MutexGuard<'_, Option<(Incoming, Report)>> {
        self.incoming.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

pub async fn send_text(
    settings: Settings,
    code: Option<String>,
    text: String,
    transfer: &Transfer,
    updates: StreamSink<Update>,
) {
    let report = reporting(updates);
    let outgoing = Outgoing::Text(text);
    let sent = crate::send(
        &settings,
        code,
        outgoing,
        Arc::clone(&report),
        &transfer.cancel,
    )
    .await;
    ended(&report, sent);
}

pub async fn send_file(
    settings: Settings,
    code: Option<String>,
    name: String,
    size: u64,
    open: impl Fn() -> DartFnFuture<i32> + Send + Sync + 'static,
    transfer: &Transfer,
    updates: StreamSink<Update>,
) {
    let report = reporting(updates);
    let source = Arc::new(Descriptor {
        size,
        open: Box::new(open),
    });
    let outgoing = Outgoing::File { name, source };
    let sent = crate::send(
        &settings,
        code,
        outgoing,
        Arc::clone(&report),
        &transfer.cancel,
    )
    .await;
    ended(&report, sent);
}

pub async fn receive(
    settings: Settings,
    code: Option<String>,
    transfer: &Transfer,
    updates: StreamSink<Update>,
) {
    let report = reporting(updates);
    let received = Box::pin(crate::receive(
        &settings,
        code,
        Arc::clone(&report),
        &transfer.cancel,
    ))
    .await;
    match received {
        Ok(incoming) => {
            *transfer.slot() = Some((incoming, Arc::clone(&report)));
            tell(&report, Stage::Offer, String::new());
        }
        Err(e) => ended(&report, Err(e)),
    }
}

pub async fn accept(transfer: &Transfer, fd: i32, cache_dir: String) {
    let Some((incoming, report)) = transfer.slot().take() else {
        return;
    };
    let accepted = match owned_file(fd) {
        Ok(file) => {
            let mut file = tokio::fs::File::from_std(file);
            incoming.accept(&mut file, PathBuf::from(cache_dir)).await
        }
        Err(e) => {
            let _ = incoming.refuse().await;
            Err(e)
        }
    };
    ended(&report, accepted);
}

pub async fn refuse(transfer: &Transfer) {
    let Some((incoming, report)) = transfer.slot().take() else {
        return;
    };
    ended(&report, incoming.refuse().await);
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
