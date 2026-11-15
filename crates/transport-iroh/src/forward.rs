use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use iroh::endpoint::{Connection, ConnectionError, RecvStream, SendStream};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::task::JoinSet;

use crate::{Error, IrohPipe};

pub const OPENED: u8 = 0;
pub const NOT_FORWARDED: u8 = 1;
pub const UNREACHABLE: u8 = 2;

const ACCEPT_RETRY: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Opened { port: u16 },
    NotForwarded { port: u16 },
    Unreachable { port: u16, why: Option<String> },
}

pub struct Forwarding {
    pipe: IrohPipe,
    tasks: JoinSet<()>,
    events: mpsc::UnboundedReceiver<Event>,
}

pub enum Next {
    Event(Event),
    Ended(Result<(), Error>),
}

impl IrohPipe {
    #[must_use]
    pub fn serve_ports(self, targets: HashMap<u16, String>) -> Forwarding {
        let (tell, events) = mpsc::unbounded_channel();
        let mut tasks = JoinSet::new();
        tasks.spawn(serve(self.connection.clone(), Arc::new(targets), tell));
        Forwarding {
            pipe: self,
            tasks,
            events,
        }
    }

    #[must_use]
    pub fn forward_ports(self, listeners: Vec<(TcpListener, u16)>) -> Forwarding {
        let (tell, events) = mpsc::unbounded_channel();
        let mut tasks = JoinSet::new();
        for (listener, port) in listeners {
            tasks.spawn(listen(
                listener,
                port,
                self.connection.clone(),
                tell.clone(),
            ));
        }
        Forwarding {
            pipe: self,
            tasks,
            events,
        }
    }
}

impl Forwarding {
    #[must_use]
    pub fn pipe(&self) -> &IrohPipe {
        &self.pipe
    }

    pub async fn next(&mut self) -> Next {
        tokio::select! {
            biased;
            Some(event) = self.events.recv() => Next::Event(event),
            closed = self.pipe.connection.closed() => Next::Ended(ended(closed)),
        }
    }

    pub async fn close(mut self) {
        self.tasks.abort_all();
        self.pipe.follower.abort();
        self.pipe.connection.close(0u8.into(), b"done");
        self.pipe.endpoint.close().await;
    }
}

fn ended(closed: ConnectionError) -> Result<(), Error> {
    match closed {
        ConnectionError::ApplicationClosed(close) if close.error_code == 0u8.into() => Ok(()),
        ConnectionError::LocallyClosed => Ok(()),
        other => Err(Error::Stream(other.to_string())),
    }
}

async fn serve(
    connection: Connection,
    targets: Arc<HashMap<u16, String>>,
    tell: mpsc::UnboundedSender<Event>,
) {
    let mut streams = JoinSet::new();
    while let Ok((send, recv)) = connection.accept_bi().await {
        streams.spawn(serve_one(send, recv, Arc::clone(&targets), tell.clone()));
        while streams.try_join_next().is_some() {}
    }
}

async fn serve_one(
    mut send: SendStream,
    mut recv: RecvStream,
    targets: Arc<HashMap<u16, String>>,
    tell: mpsc::UnboundedSender<Event>,
) {
    let mut header = [0u8; 2];
    if recv.read_exact(&mut header).await.is_err() {
        return;
    }
    let port = u16::from_be_bytes(header);
    let Some(target) = targets.get(&port) else {
        let _ = tell.send(Event::NotForwarded { port });
        refuse(send, NOT_FORWARDED).await;
        return;
    };
    match TcpStream::connect(target.as_str()).await {
        Ok(tcp) => {
            if send.write_all(&[OPENED]).await.is_ok() {
                let _ = tell.send(Event::Opened { port });
                splice(tcp, send, recv).await;
            }
        }
        Err(e) => {
            let _ = tell.send(Event::Unreachable {
                port,
                why: Some(e.to_string()),
            });
            refuse(send, UNREACHABLE).await;
        }
    }
}

async fn refuse(mut send: SendStream, status: u8) {
    if send.write_all(&[status]).await.is_ok() {
        let _ = send.finish();
        let _ = send.stopped().await;
    }
}

async fn listen(
    listener: TcpListener,
    port: u16,
    connection: Connection,
    tell: mpsc::UnboundedSender<Event>,
) {
    let mut streams = JoinSet::new();
    loop {
        let Ok((tcp, _)) = listener.accept().await else {
            tokio::time::sleep(ACCEPT_RETRY).await;
            continue;
        };
        streams.spawn(forward_one(tcp, port, connection.clone(), tell.clone()));
        while streams.try_join_next().is_some() {}
    }
}

async fn forward_one(
    tcp: TcpStream,
    port: u16,
    connection: Connection,
    tell: mpsc::UnboundedSender<Event>,
) {
    let Ok((mut send, mut recv)) = connection.open_bi().await else {
        return;
    };
    if send.write_all(&port.to_be_bytes()).await.is_err() {
        return;
    }
    let mut status = [0u8; 1];
    let event = match recv.read_exact(&mut status).await {
        Ok(()) if status[0] == OPENED => {
            splice(tcp, send, recv).await;
            return;
        }
        Ok(()) if status[0] == NOT_FORWARDED => Event::NotForwarded { port },
        Ok(()) => Event::Unreachable { port, why: None },
        Err(_) => return,
    };
    let _ = tell.send(event);
}

async fn splice(mut tcp: TcpStream, send: SendStream, recv: RecvStream) {
    let mut quic = tokio::io::join(recv, send);
    let _ = tokio::io::copy_bidirectional(&mut tcp, &mut quic).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_clean_close_ends_well() {
        assert!(ended(ConnectionError::LocallyClosed).is_ok());
        assert!(ended(ConnectionError::TimedOut).is_err());
        assert!(ended(ConnectionError::Reset).is_err());
    }
}
