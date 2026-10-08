mod hints;
mod records;

use std::net::{IpAddr, Ipv4Addr};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::task::JoinSet;
use wyrmyon_wormhole::Key;

pub use hints::{DirectHint, TransitInfo};
pub use records::{MAX_RECORD, RecordPipe};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(120);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_PENDING_HANDSHAKES: usize = 32;
const RELAY_DELAY: Duration = Duration::from_secs(2);

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
    relays: Vec<DirectHint>,
    side: String,
    timeout: Duration,
}

struct Connected {
    stream: TcpStream,
    description: String,
}

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
            relays: Vec::new(),
            side: hex::encode(rand::random::<[u8; 8]>()),
            timeout: CONNECT_TIMEOUT,
        }
    }

    #[must_use]
    pub fn with_relays(mut self, relays: Vec<DirectHint>) -> Self {
        self.relays = relays;
        self
    }

    #[must_use]
    pub fn without_listener(mut self) -> Self {
        self.listener = None;
        self.direct.clear();
        self
    }

    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    #[must_use]
    pub fn info(&self) -> TransitInfo {
        TransitInfo::new(&self.direct, &self.relays)
    }

    pub async fn connect(self, theirs: &TransitInfo) -> Result<RecordPipe, Error> {
        let (tx, mut rx) = mpsc::channel::<Connected>(4);
        let mut tasks = JoinSet::new();
        if let Some(listener) = self.listener {
            tasks.spawn(accept(listener, self.role, self.key.clone(), tx.clone()));
        }
        let direct = theirs.direct_hints();
        let relay_delay = if direct.is_empty() {
            Duration::ZERO
        } else {
            RELAY_DELAY
        };
        for hint in direct {
            let (tx, key, role) = (tx.clone(), self.key.clone(), self.role);
            tasks.spawn(async move {
                if let Ok(connected) = dial(&hint, role, &key, None).await {
                    let _ = tx.send(connected).await;
                }
            });
        }
        let mut relays = self.relays.clone();
        for hint in theirs.relay_hints() {
            if !relays.contains(&hint) {
                relays.push(hint);
            }
        }
        for hint in relays {
            let (tx, key, role, side) =
                (tx.clone(), self.key.clone(), self.role, self.side.clone());
            tasks.spawn(async move {
                tokio::time::sleep(relay_delay).await;
                if let Ok(connected) = dial(&hint, role, &key, Some(&side)).await {
                    let _ = tx.send(connected).await;
                }
            });
        }
        drop(tx);

        let deadline = tokio::time::Instant::now() + self.timeout;
        loop {
            let mut connected = tokio::time::timeout_at(deadline, rx.recv())
                .await
                .ok()
                .flatten()
                .ok_or(Error::NoConnection)?;
            if self.role == Role::Sender && connected.stream.write_all(b"go\n").await.is_err() {
                continue;
            }
            return Ok(RecordPipe::new(
                connected.stream,
                &self.key,
                self.role,
                connected.description,
            ));
        }
    }
}

async fn accept(listener: TcpListener, role: Role, key: Key, tx: mpsc::Sender<Connected>) {
    let mut handshakes = JoinSet::new();
    loop {
        let Ok((stream, peer)) = listener.accept().await else {
            tokio::time::sleep(Duration::from_millis(100)).await;
            continue;
        };
        while handshakes.try_join_next().is_some() {}
        if handshakes.len() >= MAX_PENDING_HANDSHAKES {
            continue;
        }
        let (tx, key) = (tx.clone(), key.clone());
        handshakes.spawn(async move {
            if let Ok(Ok(stream)) =
                tokio::time::timeout(HANDSHAKE_TIMEOUT, handshake(stream, role, &key)).await
            {
                let description = format!("directly from {peer}");
                let _ = tx
                    .send(Connected {
                        stream,
                        description,
                    })
                    .await;
            }
        });
    }
}

async fn dial(
    hint: &DirectHint,
    role: Role,
    key: &Key,
    relay_side: Option<&str>,
) -> Result<Connected, Error> {
    let mut stream = TcpStream::connect((hint.hostname.as_str(), hint.port)).await?;
    let peer = stream
        .peer_addr()
        .map_or_else(|_| hint.hostname.clone(), |a| a.to_string());
    let attempt = async {
        if let Some(side) = relay_side {
            stream.write_all(&relay_line(key, side)).await?;
            expect(&mut stream, b"ok\n", "the relay refused").await?;
        }
        handshake(stream, role, key).await
    };
    let stream = tokio::time::timeout(HANDSHAKE_TIMEOUT, attempt)
        .await
        .map_err(|_| Error::Handshake("timed out"))??;
    let description = match relay_side {
        Some(_) => format!("via relay {peer}"),
        None => format!("directly to {peer}"),
    };
    Ok(Connected {
        stream,
        description,
    })
}

fn relay_line(key: &Key, side: &str) -> Vec<u8> {
    format!(
        "please relay {} for side {side}\n",
        hex_id(key, b"transit_relay_token")
    )
    .into_bytes()
}

fn local_addresses() -> Vec<IpAddr> {
    usable_addresses(
        if_addrs::get_if_addrs()
            .unwrap_or_default()
            .into_iter()
            .map(|i| i.ip()),
    )
}

fn usable_addresses(all: impl Iterator<Item = IpAddr>) -> Vec<IpAddr> {
    let addresses: Vec<IpAddr> = all.filter(|ip| ip.is_ipv4() && !ip.is_loopback()).collect();
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
    fn the_relay_line_matches_the_python_client() {
        assert_eq!(
            relay_line(&transit_key(), "0123456789abcdef"),
            b"please relay ecd85f320731169f5768239a248b8b0f3e7834f1c05937a5939b13d6bfa933e1 for side 0123456789abcdef\n"
        );
    }

    #[test]
    fn hints_use_lan_ipv4_addresses_or_fall_back_to_loopback() {
        let lan: IpAddr = "192.168.1.5".parse().unwrap();
        let v6: IpAddr = "fe80::1".parse().unwrap();
        let lo: IpAddr = "127.0.0.1".parse().unwrap();
        assert_eq!(usable_addresses([lo, v6, lan].into_iter()), [lan]);
        assert_eq!(usable_addresses([lo, v6].into_iter()), [lo]);
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
