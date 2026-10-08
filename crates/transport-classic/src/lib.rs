mod hints;
mod records;

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::task::JoinSet;
use wyrmyon_wormhole::Key;

pub use hints::{DirectHint, TransitInfo};
pub use records::{MAX_RECORD, RecordPipe};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Sender,
    Receiver,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("transit: {0}")]
    Io(#[from] std::io::Error),
    #[error("transit handshake failed: {0}")]
    Handshake(&'static str),
    #[error("could not connect to the other side, directly or through a relay")]
    NoConnection,
    #[error("transit: the peer sent {0}")]
    Record(String),
}

pub struct Transit {
    role: Role,
    key: Key,
    listener: Option<TcpListener>,
    direct: Vec<DirectHint>,
}

type Connected = (TcpStream, Option<SocketAddr>);

impl Transit {
    pub async fn new(role: Role, transit_key: Key) -> Self {
        let listener = TcpListener::bind((Ipv4Addr::UNSPECIFIED, 0)).await.ok();
        let direct = match listener.as_ref().and_then(|l| l.local_addr().ok()) {
            Some(addr) => local_addresses()
                .into_iter()
                .map(|ip| DirectHint {
                    hostname: ip.to_string(),
                    port: addr.port(),
                })
                .collect(),
            None => Vec::new(),
        };
        Self {
            role,
            key: transit_key,
            listener,
            direct,
        }
    }

    #[must_use]
    pub fn info(&self) -> TransitInfo {
        TransitInfo::new(&self.direct, &[])
    }

    pub async fn connect(self, theirs: &TransitInfo) -> Result<RecordPipe, Error> {
        let (tx, mut rx) = mpsc::channel::<Connected>(4);
        let mut tasks = JoinSet::new();
        if let Some(listener) = self.listener {
            tasks.spawn(accept(listener, self.role, self.key.clone(), tx.clone()));
        }
        for hint in theirs.direct_hints() {
            let (tx, key, role) = (tx.clone(), self.key.clone(), self.role);
            tasks.spawn(async move {
                let stream = TcpStream::connect((hint.hostname.as_str(), hint.port)).await?;
                let peer = stream.peer_addr().ok();
                let stream = handshake(stream, role, &key).await?;
                let _ = tx.send((stream, peer)).await;
                Ok(())
            });
        }
        drop(tx);

        let (mut stream, peer) = tokio::time::timeout(CONNECT_TIMEOUT, rx.recv())
            .await
            .ok()
            .flatten()
            .ok_or(Error::NoConnection)?;
        if self.role == Role::Sender {
            stream.write_all(b"go\n").await?;
        }
        drop(tasks);
        Ok(RecordPipe::new(stream, &self.key, self.role, peer, false))
    }
}

async fn accept(
    listener: TcpListener,
    role: Role,
    key: Key,
    tx: mpsc::Sender<Connected>,
) -> Result<(), Error> {
    let mut handshakes = JoinSet::new();
    loop {
        let (stream, peer) = listener.accept().await?;
        let (tx, key) = (tx.clone(), key.clone());
        handshakes.spawn(async move {
            if let Ok(stream) = handshake(stream, role, &key).await {
                let _ = tx.send((stream, Some(peer))).await;
            }
        });
    }
}

fn local_addresses() -> Vec<IpAddr> {
    let addresses: Vec<IpAddr> = if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .map(|i| i.ip())
        .filter(|ip| ip.is_ipv4() && !ip.is_loopback())
        .collect();
    if addresses.is_empty() {
        vec![IpAddr::V4(Ipv4Addr::LOCALHOST)]
    } else {
        addresses
    }
}

fn hex_id(key: &Key, purpose: &[u8]) -> String {
    hex::encode(key.derive(purpose).as_bytes())
}

fn handshake_line(key: &Key, role: Role) -> Vec<u8> {
    match role {
        Role::Sender => format!(
            "transit sender {} ready\n\n",
            hex_id(key, b"transit_sender")
        ),
        Role::Receiver => format!(
            "transit receiver {} ready\n\n",
            hex_id(key, b"transit_receiver")
        ),
    }
    .into_bytes()
}

async fn expect(stream: &mut TcpStream, want: &[u8], what: &'static str) -> Result<(), Error> {
    let mut got = vec![0u8; want.len()];
    stream.read_exact(&mut got).await?;
    if got == want {
        Ok(())
    } else {
        Err(Error::Handshake(what))
    }
}

async fn handshake(mut stream: TcpStream, role: Role, key: &Key) -> Result<TcpStream, Error> {
    let theirs = match role {
        Role::Sender => Role::Receiver,
        Role::Receiver => Role::Sender,
    };
    stream.write_all(&handshake_line(key, role)).await?;
    expect(
        &mut stream,
        &handshake_line(key, theirs),
        "unexpected greeting",
    )
    .await?;
    if role == Role::Receiver {
        expect(&mut stream, b"go\n", "the sender picked another connection").await?;
    }
    Ok(stream)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn transit_key() -> Key {
        Key::from_bytes(
            hex::decode("9329a646acaba7172c557c7971191ec6a1e287f84abe58c60dce2659c44c2925")
                .unwrap()
                .try_into()
                .unwrap(),
        )
    }

    #[test]
    fn handshakes_match_the_python_client() {
        assert_eq!(
            handshake_line(&transit_key(), Role::Sender),
            b"transit sender 6fa5ad1bb11461d5165ff989e5ce9bf1c2d464385caf06870527e62fc0643f4c ready\n\n"
        );
        assert_eq!(
            handshake_line(&transit_key(), Role::Receiver),
            b"transit receiver 0b5d03da4672ec8428bd2485e126faf13ef3ffb278196ddc2f95d96d3d82c1fd ready\n\n"
        );
    }
}
