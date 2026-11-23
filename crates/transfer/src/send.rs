use std::ops::ControlFlow;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use anyhow::{Context, bail};
use wyrmyon_transport_classic::{Role, Transit, TransitInfo};
use wyrmyon_transport_iroh::{
    IrohPipe, IrohTransport, Manifest, Offered, Role as IrohRole, Source as Member, Traffic,
};
use wyrmyon_wormhole::Wormhole;

use crate::events::{Cancelled, Events};
use crate::protocol::{self, Answer, AppMessage, CollectionOffer, Offer};
use crate::source::Source;
use crate::{Options, Pipe, await_ack, send_stream};

#[derive(Default)]
pub struct Staged {
    pub blob: Option<Offered>,
    pub collection: Option<(Offered, CollectionOffer)>,
    pub blob_ahead: Option<Hashing<Offered>>,
    pub collection_ahead: Option<Hashing<(Offered, Manifest)>>,
    pub several: bool,
}

impl Staged {
    pub async fn drop_blob_ahead(&mut self) {
        if self.several {
            return;
        }
        if let Some(ahead) = self.blob_ahead.take()
            && let Some(offered) = ahead.stop().await
        {
            offered.close().await;
        }
    }

    pub async fn drop_collection_ahead(&mut self) {
        if self.several {
            return;
        }
        if let Some(ahead) = self.collection_ahead.take()
            && let Some((offered, _)) = ahead.stop().await
        {
            offered.close().await;
        }
    }

    pub async fn close(mut self) {
        self.several = false;
        self.drop_blob_ahead().await;
        self.drop_collection_ahead().await;
        if let Some(blob) = self.blob {
            blob.close().await;
        }
        if let Some((offered, _)) = self.collection {
            offered.close().await;
        }
    }
}

pub struct Hashing<T> {
    hashed: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    total: u64,
    task: Option<tokio::task::JoinHandle<Result<T, wyrmyon_transport_iroh::Error>>>,
}

type Progress = Box<dyn FnMut(u64) -> ControlFlow<()> + Send>;

impl<T: Send + 'static> Hashing<T> {
    fn start<F>(total: u64, import: impl FnOnce(Progress) -> F) -> Self
    where
        F: Future<Output = Result<T, wyrmyon_transport_iroh::Error>> + Send + 'static,
    {
        let hashed = Arc::new(AtomicU64::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let (counter, stopped) = (Arc::clone(&hashed), Arc::clone(&stop));
        let progress = Box::new(move |n| {
            counter.store(n, Ordering::Relaxed);
            if stopped.load(Ordering::Relaxed) {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        });
        Self {
            hashed,
            stop,
            total,
            task: Some(tokio::spawn(import(progress))),
        }
    }

    pub async fn finish(mut self, events: &dyn Events) -> anyhow::Result<T> {
        let mut task = self
            .task
            .take()
            .expect("a hashing task runs until finished");
        if !task.is_finished() {
            events.hashing(self.total);
            let mut tick = tokio::time::interval(std::time::Duration::from_millis(100));
            let joined = loop {
                tokio::select! {
                    joined = &mut task => break joined,
                    _ = tick.tick() => events.hashed(self.hashed.load(Ordering::Relaxed)),
                    () = events.cancelled() => {
                        events.hashing_done();
                        return Err(Cancelled.into());
                    }
                }
            };
            events.hashing_done();
            return Ok(joined.context("hashing the data")??);
        }
        Ok(task.await.context("hashing the data")??)
    }

    pub async fn stop(mut self) -> Option<T> {
        let task = self.task.take()?;
        self.stop.store(true, Ordering::Relaxed);
        task.await.ok()?.ok()
    }
}

impl Hashing<Offered> {
    pub fn blob(source: Arc<dyn Source>) -> Self {
        Self::start(source.size(), move |mut progress| async move {
            if let Some(path) = source.path() {
                return Offered::import_with_progress(path, &mut progress).await;
            }
            let reader = source
                .open()
                .await
                .map_err(|e| wyrmyon_transport_iroh::Error::Blob(e.to_string()))?;
            Offered::import_reader(reader, source.size(), &mut progress).await
        })
    }
}

impl Hashing<(Offered, Manifest)> {
    pub fn collection(members: Vec<Member>, dirs: Vec<String>, total: u64) -> Self {
        Self::start(total, move |mut progress| async move {
            Offered::import_collection_with_progress(&members, &dirs, &mut progress).await
        })
    }
}

impl<T> Drop for Hashing<T> {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

pub async fn offer_text(wormhole: &mut Wormhole, text: String) -> anyhow::Result<()> {
    protocol::send(wormhole, &AppMessage::Offer(Offer::Message(text))).await?;
    loop {
        if let AppMessage::Answer(answer) = protocol::next(wormhole, "receiver").await? {
            if answer == Answer::MessageAck("ok".into()) {
                return Ok(());
            }
            bail!("unexpected answer from the receiver: {answer:?}");
        }
    }
}

pub async fn require_iroh(wormhole: &mut Wormhole, options: &Options) -> anyhow::Result<bool> {
    let use_iroh = options.use_iroh(wormhole);
    if options.force_iroh && !use_iroh {
        protocol::send_error(wormhole, "the sender requires iroh-v1").await?;
        bail!("the other side does not speak iroh-v1 (--force-iroh)");
    }
    Ok(use_iroh)
}

pub async fn send_data(
    wormhole: &mut Wormhole,
    options: &Options,
    staged: &mut Staged,
    offer: Offer,
    source: &Arc<dyn Source>,
    events: &Arc<dyn Events>,
) -> anyhow::Result<Option<Traffic>> {
    let use_iroh = require_iroh(wormhole, options).await?;
    if !use_iroh {
        staged.drop_blob_ahead().await;
    }
    if use_iroh && staged.blob.is_none() {
        let hashing = staged
            .blob_ahead
            .take()
            .unwrap_or_else(|| Hashing::blob(Arc::clone(source)));
        match hashing.finish(events.as_ref()).await {
            Ok(offered) => staged.blob = Some(offered),
            Err(e) => {
                let why = if e.is::<Cancelled>() {
                    "the sender cancelled"
                } else {
                    "the sender could not read its data"
                };
                protocol::send_error(wormhole, why).await?;
                return Err(e);
            }
        }
    }
    let offered = if use_iroh { staged.blob.as_ref() } else { None };
    send_over(
        wormhole,
        options,
        offer,
        Some(source.as_ref()),
        source.size(),
        offered,
        events,
    )
    .await
}

enum Prepared {
    Iroh(IrohTransport),
    Classic(Transit),
}

impl Prepared {
    async fn after(self, sent: anyhow::Result<()>) -> anyhow::Result<Self> {
        match (sent, self) {
            (Ok(()), prepared) => Ok(prepared),
            (Err(e), Self::Iroh(iroh)) => {
                iroh.close().await;
                Err(e)
            }
            (Err(e), Self::Classic(_)) => Err(e),
        }
    }
}

pub async fn send_over(
    wormhole: &mut Wormhole,
    options: &Options,
    offer: Offer,
    source: Option<&dyn Source>,
    size: u64,
    offered: Option<&Offered>,
    events: &Arc<dyn Events>,
) -> anyhow::Result<Option<Traffic>> {
    let prepared = if offered.is_some() {
        let iroh = options.iroh_or_tell(wormhole, IrohRole::Sender).await?;
        let info = AppMessage::Iroh(iroh.info().await);
        Prepared::Iroh(iroh).after(protocol::send(wormhole, &info).await)
    } else {
        let transit = options.transit(Role::Sender, wormhole.transit_key()).await;
        let info = AppMessage::Transit(transit.info());
        Prepared::Classic(transit).after(protocol::send(wormhole, &info).await)
    }
    .await?;
    let prepared = prepared
        .after(protocol::send(wormhole, &AppMessage::Offer(offer)).await)
        .await?;

    let answered = async {
        let (mut their_iroh, mut their_transit) = (None, TransitInfo::default());
        loop {
            match protocol::next(wormhole, "receiver").await? {
                AppMessage::Iroh(info) => their_iroh = Some(info),
                AppMessage::Transit(info) => their_transit = info,
                AppMessage::Answer(Answer::FileAck(ack)) if ack == "ok" => {
                    return Ok((their_iroh, their_transit));
                }
                AppMessage::Answer(answer) => {
                    bail!("unexpected answer from the receiver: {answer:?}")
                }
                _ => {}
            }
        }
    };
    let answered = tokio::select! {
        answered = answered => answered,
        () = events.cancelled() => Err(Cancelled.into()),
    };
    let mut pipe = match (answered, prepared) {
        (Ok((Some(theirs), _)), Prepared::Iroh(iroh)) => {
            Pipe::Iroh(iroh.connect(&theirs, wormhole.key()).await?)
        }
        (Ok((_, theirs)), Prepared::Classic(transit)) => {
            Pipe::Classic(transit.connect(&theirs).await?)
        }
        (Ok(_), Prepared::Iroh(iroh)) => {
            iroh.close().await;
            bail!("the receiver accepted without an iroh address");
        }
        (Err(e), Prepared::Iroh(iroh)) => {
            iroh.close().await;
            return Err(e);
        }
        (Err(e), Prepared::Classic(_)) => return Err(e),
    };
    events.connected(&pipe, size);
    let sent = async {
        let digest = match &mut pipe {
            Pipe::Iroh(iroh) => {
                iroh.provide(offered.expect("iroh pipes carry a blob"))
                    .await?;
                follow_sent(iroh, size, Arc::clone(events));
                None
            }
            Pipe::Classic(records) => {
                let source = source.expect("classic pipes carry a source");
                let mut reader = source.open().await?;
                let digest = send_stream(records, &mut reader, size, &|n| events.moved(n)).await?;
                events.settle().await;
                Some(digest)
            }
        };
        if digest.is_some() {
            events.awaiting_ack();
        }
        let acked = await_ack(&mut pipe, digest.as_ref()).await;
        if acked.is_ok() {
            events.settle().await;
        }
        events.stopped();
        acked
    };
    let sent = tokio::select! {
        sent = sent => sent,
        () = events.cancelled() => Err(Cancelled.into()),
    };
    if let Err(e) = sent {
        events.stopped();
        pipe.abort().await;
        return Err(e);
    }
    let traffic = pipe.traffic();
    pipe.shutdown().await;
    Ok(traffic)
}

fn follow_sent(pipe: &IrohPipe, size: u64, events: Arc<dyn Events>) {
    let mut sent = pipe.sent();
    tokio::spawn(async move {
        while sent.changed().await.is_ok() {
            let n = *sent.borrow_and_update();
            events.moved(n.min(size));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn running() -> (Hashing<()>, tokio::sync::oneshot::Receiver<()>) {
        let (ended, released) = tokio::sync::oneshot::channel::<()>();
        let hashing = Hashing::start(0, move |mut progress| async move {
            let _ended = ended;
            while progress(1).is_continue() {
                tokio::task::yield_now().await;
            }
            Err(wyrmyon_transport_iroh::Error::Blob("stopped".into()))
        });
        (hashing, released)
    }

    #[tokio::test]
    async fn dropping_or_stopping_a_hashing_ends_its_import() {
        let (hashing, released) = running();
        drop(hashing);
        assert!(released.await.is_err());
        let (hashing, released) = running();
        assert_eq!(hashing.stop().await, None);
        assert!(released.await.is_err());
    }

    #[tokio::test]
    async fn aborting_a_finished_hashing_hands_back_what_it_imported() {
        let hashing = Hashing::start(0, |mut progress| async move {
            let _ = progress(7);
            Ok(7)
        });
        while !hashing.task.as_ref().unwrap().is_finished() {
            tokio::task::yield_now().await;
        }
        assert_eq!(hashing.hashed.load(Ordering::Relaxed), 7);
        assert_eq!(hashing.stop().await, Some(7));
    }
}
