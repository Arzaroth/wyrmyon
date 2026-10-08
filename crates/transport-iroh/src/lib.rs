use std::net::SocketAddr;
use std::time::Duration;

use iroh::endpoint::{Connection, RecvStream, SendStream, presets};
use iroh::{Endpoint, EndpointAddr, EndpointId, RelayMode, RelayUrl, SecretKey, TransportAddr};
use serde::{Deserialize, Serialize};
use wyrmyon_wormhole::Key;

pub const TRANSPORT: &str = "iroh-v1";
pub const ALPN: &[u8] = b"wyrmyon/1";
pub const CONFIRM_LABEL: &[u8] = b"wyrmyon/iroh-v1/confirm";

const ADDRESS_WAIT: Duration = Duration::from_secs(5);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_ACK: usize = 4096;
const CLOSE_WAIT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Sender,
    Receiver,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Relays {
    Default,
    Disabled,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("iroh: could not bind an endpoint: {0}")]
    Bind(String),
    #[error("iroh: the peer's address is unusable: {0}")]
    BadAddress(String),
    #[error("iroh: could not connect to the other side: {0}")]
    Connect(String),
    #[error("iroh: the connection is not the one the code agreed on")]
    WrongPeer,
    #[error("iroh: {0}")]
    Stream(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IrohInfo {
    pub id: String,
    #[serde(default)]
    pub relays: Vec<String>,
    #[serde(default)]
    pub direct: Vec<String>,
}

impl IrohInfo {
    fn to_addr(&self) -> Result<EndpointAddr, Error> {
        let id: EndpointId = self
            .id
            .parse()
            .map_err(|_| Error::BadAddress("node id".into()))?;
        let relays = self
            .relays
            .iter()
            .filter_map(|r| r.parse::<RelayUrl>().ok())
            .map(TransportAddr::Relay);
        let direct = self
            .direct
            .iter()
            .filter_map(|d| d.parse::<SocketAddr>().ok())
            .map(TransportAddr::Ip);
        let addr = EndpointAddr::from_parts(id, relays.chain(direct));
        if addr.is_empty() {
            return Err(Error::BadAddress("no relay and no direct address".into()));
        }
        Ok(addr)
    }

    fn from_addr(addr: &EndpointAddr) -> Self {
        Self {
            id: addr.id.to_string(),
            relays: addr.relay_urls().map(ToString::to_string).collect(),
            direct: addr.ip_addrs().map(ToString::to_string).collect(),
        }
    }
}

pub struct IrohTransport {
    role: Role,
    endpoint: Endpoint,
    relays: Relays,
}

pub struct IrohPipe {
    role: Role,
    send: SendStream,
    recv: RecvStream,
    connection: Connection,
    endpoint: Endpoint,
    description: String,
}

impl IrohTransport {
    pub async fn bind(role: Role, relays: Relays) -> Result<Self, Error> {
        let relay_mode = match relays {
            Relays::Default => iroh::endpoint::default_relay_mode(),
            Relays::Disabled => RelayMode::Disabled,
        };
        let endpoint = Endpoint::builder(presets::Minimal)
            .secret_key(SecretKey::generate())
            .alpns(vec![ALPN.to_vec()])
            .relay_mode(relay_mode)
            .bind()
            .await
            .map_err(|e| Error::Bind(e.to_string()))?;
        Ok(Self {
            role,
            endpoint,
            relays,
        })
    }

    pub async fn info(&self) -> IrohInfo {
        let ready = async {
            if self.relays == Relays::Default {
                self.endpoint.online().await;
            }
            while self.endpoint.addr().ip_addrs().next().is_none() {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        };
        let _ = tokio::time::timeout(ADDRESS_WAIT, ready).await;
        IrohInfo::from_addr(&self.endpoint.addr())
    }

    pub async fn connect(self, theirs: &IrohInfo, wormhole_key: &Key) -> Result<IrohPipe, Error> {
        match self.establish(theirs, wormhole_key).await {
            Ok((connection, send, recv, description)) => Ok(IrohPipe {
                role: self.role,
                send,
                recv,
                connection,
                endpoint: self.endpoint,
                description,
            }),
            Err(e) => {
                self.endpoint.close().await;
                Err(e)
            }
        }
    }

    async fn establish(
        &self,
        theirs: &IrohInfo,
        wormhole_key: &Key,
    ) -> Result<(Connection, SendStream, RecvStream, String), Error> {
        let addr = theirs.to_addr()?;
        let confirm = wormhole_key.derive(CONFIRM_LABEL);
        let ours = self.endpoint.id();
        let attempt = async {
            match self.role {
                Role::Sender => {
                    let connection = self
                        .endpoint
                        .connect(addr.clone(), ALPN)
                        .await
                        .map_err(|e| Error::Connect(e.to_string()))?;
                    if connection.remote_id() != addr.id {
                        return Err(Error::WrongPeer);
                    }
                    let (send, recv) = connection
                        .open_bi()
                        .await
                        .map_err(|e| Error::Stream(e.to_string()))?;
                    Ok((connection, send, recv))
                }
                Role::Receiver => loop {
                    let Some(incoming) = self.endpoint.accept().await else {
                        return Err(Error::Connect("the endpoint closed".into()));
                    };
                    let Ok(connection) = incoming.await else {
                        continue;
                    };
                    if connection.remote_id() != addr.id {
                        connection.close(1u8.into(), b"not you");
                        continue;
                    }
                    let (send, recv) = connection
                        .accept_bi()
                        .await
                        .map_err(|e| Error::Stream(e.to_string()))?;
                    break Ok((connection, send, recv));
                },
            }
        };
        let (connection, mut send, mut recv) = tokio::time::timeout(CONNECT_TIMEOUT, attempt)
            .await
            .map_err(|_| Error::Connect("timed out".into()))??;

        let (sender_id, receiver_id) = match self.role {
            Role::Sender => (ours, addr.id),
            Role::Receiver => (addr.id, ours),
        };
        let tag = |role: &str| {
            *confirm
                .derive(format!("{role}:{sender_id}:{receiver_id}").as_bytes())
                .as_bytes()
        };
        let (mine, expected) = match self.role {
            Role::Sender => (tag("sender"), tag("receiver")),
            Role::Receiver => (tag("receiver"), tag("sender")),
        };
        send.write_all(&mine)
            .await
            .map_err(|e| Error::Stream(e.to_string()))?;
        let mut got = [0u8; 32];
        recv.read_exact(&mut got)
            .await
            .map_err(|e| Error::Stream(e.to_string()))?;
        if !constant_time_eq(&got, &expected) {
            connection.close(2u8.into(), b"binding");
            return Err(Error::WrongPeer);
        }

        let description = match connection
            .paths()
            .into_iter()
            .find(iroh::endpoint::Path::is_selected)
        {
            Some(path) if path.is_relay() => "iroh, via relay".to_owned(),
            Some(_) => "iroh, direct".to_owned(),
            None => "iroh".to_owned(),
        };
        Ok((connection, send, recv, description))
    }
}

fn constant_time_eq(a: &[u8; 32], b: &[u8; 32]) -> bool {
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

impl IrohPipe {
    #[must_use]
    pub fn describe(&self) -> &str {
        &self.description
    }

    pub async fn send_chunk(&mut self, data: &[u8]) -> Result<(), Error> {
        self.send
            .write_all(data)
            .await
            .map_err(|e| Error::Stream(e.to_string()))
    }

    pub async fn receive_chunk(&mut self, max: usize) -> Result<Vec<u8>, Error> {
        let mut buf = vec![0u8; max];
        match self
            .recv
            .read(&mut buf)
            .await
            .map_err(|e| Error::Stream(e.to_string()))?
        {
            Some(n) => {
                buf.truncate(n);
                Ok(buf)
            }
            None => Err(Error::Stream("the peer closed the stream early".into())),
        }
    }

    pub async fn send_last(&mut self, data: &[u8]) -> Result<(), Error> {
        self.send_chunk(data).await?;
        self.send.finish().map_err(|e| Error::Stream(e.to_string()))
    }

    pub async fn receive_last(&mut self) -> Result<Vec<u8>, Error> {
        self.recv
            .read_to_end(MAX_ACK)
            .await
            .map_err(|e| Error::Stream(e.to_string()))
    }

    pub async fn finish(mut self) {
        match self.role {
            Role::Sender => self.connection.close(0u8.into(), b"done"),
            Role::Receiver => {
                let _ = self.send.finish();
                let _ = tokio::time::timeout(CLOSE_WAIT, self.connection.closed()).await;
            }
        }
        self.endpoint.close().await;
    }
}
