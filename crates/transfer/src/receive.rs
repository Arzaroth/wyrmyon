use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, bail};
use tokio::io::AsyncWrite;
use wyrmyon_transport_classic::{Role, Transit, TransitInfo};
use wyrmyon_transport_iroh::{IrohInfo, IrohTransport, Role as IrohRole, Traffic};
use wyrmyon_wormhole::Wormhole;

use crate::events::Events;
use crate::protocol::{self, Answer, AppMessage, Offer};
use crate::{Options, Pipe, receive_stream, send_ack};

pub enum Connection {
    Iroh(IrohTransport, IrohInfo),
    Classic(Transit, TransitInfo),
}

impl Connection {
    pub fn is_classic(&self) -> bool {
        matches!(self, Self::Classic(..))
    }

    pub async fn close(self) {
        if let Self::Iroh(iroh, _) = self {
            iroh.close().await;
        }
    }
}

pub async fn await_offer(
    wormhole: &mut Wormhole,
    options: &Options,
) -> anyhow::Result<(Offer, Option<Connection>)> {
    let iroh_allowed = options.use_iroh(wormhole);
    let mut connection: Option<Connection> = None;
    loop {
        match protocol::next(wormhole, "sender").await {
            Ok(AppMessage::Transit(info)) if connection.is_none() => {
                let transit = options
                    .transit(Role::Receiver, wormhole.transit_key())
                    .await;
                protocol::send(wormhole, &AppMessage::Transit(transit.info())).await?;
                connection = Some(Connection::Classic(transit, info));
            }
            Ok(AppMessage::Iroh(info)) if iroh_allowed && connection.is_none() => {
                let iroh = options.iroh_or_tell(wormhole, IrohRole::Receiver).await?;
                protocol::send(wormhole, &AppMessage::Iroh(iroh.info().await)).await?;
                connection = Some(Connection::Iroh(iroh, info));
            }
            Ok(AppMessage::Offer(offer)) => return Ok((offer, connection)),
            Ok(_) => {}
            Err(e) => {
                if let Some(connection) = connection {
                    connection.close().await;
                }
                return Err(e);
            }
        }
    }
}

pub async fn refuse<T>(wormhole: &mut Wormhole, why: &str) -> anyhow::Result<T> {
    protocol::send_error(wormhole, why).await?;
    bail!("{why}")
}

pub async fn receiver_requires_iroh(
    wormhole: &mut Wormhole,
    options: &Options,
    classic: bool,
) -> anyhow::Result<()> {
    if options.force_iroh && classic {
        protocol::send_error(wormhole, "the receiver requires iroh-v1").await?;
        bail!("the other side does not speak iroh-v1 (--force-iroh)");
    }
    Ok(())
}

pub enum Sink<'a> {
    File {
        file: tokio::fs::File,
        path: &'a Path,
    },
    Writer(&'a mut (dyn AsyncWrite + Send + Unpin)),
}

pub struct Received {
    pub digest: Option<[u8; 32]>,
    pub pipe: Pipe,
}

impl Received {
    pub fn traffic(&self) -> Option<Traffic> {
        self.pipe.traffic()
    }

    pub async fn complete(mut self, finished: anyhow::Result<()>) -> anyhow::Result<()> {
        let done = match finished {
            Ok(()) => send_ack(&mut self.pipe, self.digest.as_ref()).await,
            Err(e) => Err(e),
        };
        match done {
            Ok(()) => {
                self.pipe.shutdown().await;
                Ok(())
            }
            Err(e) => {
                self.pipe.abort().await;
                Err(e)
            }
        }
    }
}

pub async fn receive_into(
    wormhole: &mut Wormhole,
    connection: Connection,
    size: u64,
    cache: anyhow::Result<PathBuf>,
    sink: Sink<'_>,
    events: &Arc<dyn Events>,
) -> anyhow::Result<Received> {
    protocol::send(wormhole, &AppMessage::Answer(Answer::FileAck("ok".into()))).await?;
    let mut pipe = match connection {
        Connection::Iroh(iroh, theirs) => Pipe::Iroh(iroh.connect(&theirs, wormhole.key()).await?),
        Connection::Classic(transit, theirs) => Pipe::Classic(transit.connect(&theirs).await?),
    };
    events.connected(&pipe, size);
    let received = async {
        match (&mut pipe, sink) {
            (Pipe::Iroh(iroh), sink) => {
                let fetched = iroh.fetch(&cache?, size, &mut |n| events.moved(n)).await?;
                events.settle().await;
                match sink {
                    Sink::File { file, path } => {
                        drop(file.into_std().await);
                        fetched.export_to(path).await?;
                    }
                    Sink::Writer(writer) => fetched.export_writer(writer).await?,
                }
                Ok(None)
            }
            (Pipe::Classic(records), Sink::File { mut file, .. }) => {
                let digest = receive_stream(records, &mut file, size, &|n| events.moved(n)).await?;
                events.settle().await;
                file.sync_all().await.context("writing the data")?;
                anyhow::Ok(Some(digest))
            }
            (Pipe::Classic(records), Sink::Writer(writer)) => {
                let digest = receive_stream(records, writer, size, &|n| events.moved(n)).await?;
                events.settle().await;
                Ok(Some(digest))
            }
        }
    }
    .await;
    match received {
        Ok(digest) => Ok(Received { digest, pipe }),
        Err(e) => {
            events.stopped();
            pipe.abort().await;
            Err(e)
        }
    }
}
