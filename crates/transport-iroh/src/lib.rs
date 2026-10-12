mod blobs;
mod collection;
mod netcheck;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::stream::FuturesUnordered;
use futures_util::{FutureExt, StreamExt};
use iroh::endpoint::{Connection, PathEvent, PathId, PathStats, RecvStream, SendStream, presets};
use iroh::{Endpoint, EndpointAddr, EndpointId, RelayMode, RelayUrl, SecretKey, TransportAddr};
use serde::{Deserialize, Serialize};
use tokio::sync::watch;
use wyrmyon_wormhole::Key;

pub use blobs::{Fetched, Offered};
pub use collection::{Collection, MAX_FILES, MAX_MANIFEST, Manifest, ManifestFile, Source};
pub use netcheck::{NetCheck, PortMapping, netcheck};

pub const TRANSPORT: &str = "iroh-v1";
pub const ALPN: &[u8] = b"wyrmyon/1";
pub const CONFIRM_LABEL: &[u8] = b"wyrmyon/iroh-v1/confirm";

const ADDRESS_WAIT: Duration = Duration::from_secs(5);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_ACK: usize = 4096;
const CLOSE_WAIT: Duration = Duration::from_secs(5);
const SNAPSHOT_EVERY: Duration = Duration::from_millis(250);
const MAX_HANDSHAKES: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Sender,
    Receiver,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Relays {
    Default,
    Disabled,
    Custom(Vec<RelayUrl>),
}

impl std::str::FromStr for Relays {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        match s.to_ascii_lowercase().as_str() {
            "default" => return Ok(Self::Default),
            "disabled" => return Ok(Self::Disabled),
            _ => {}
        }
        let urls: Vec<RelayUrl> = s
            .split(',')
            .map(str::trim)
            .filter(|url| !url.is_empty())
            .map(|url| match url.parse::<RelayUrl>() {
                Ok(relay) if matches!(relay.scheme(), "http" | "https") => Ok(relay),
                _ => Err(format!(
                    "`{url}` is not `default`, `disabled` or an http(s) relay URL"
                )),
            })
            .collect::<Result<_, _>>()?;
        if urls.is_empty() {
            return Err("give `default`, `disabled` or http(s) relay URLs".into());
        }
        Ok(Self::Custom(urls))
    }
}

impl Relays {
    fn accepts_from_peer(&self, relay: &RelayUrl) -> bool {
        match self {
            Self::Disabled => false,
            Self::Custom(ours) if ours.contains(relay) => true,
            _ => match relay.scheme() {
                "https" => true,
                "http" => relay.host_str().is_some_and(|host| {
                    host.eq_ignore_ascii_case("localhost")
                        || host
                            .trim_matches(['[', ']'])
                            .parse::<std::net::IpAddr>()
                            .is_ok_and(|ip| ip.is_loopback())
                }),
                _ => false,
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    Direct,
    Relay,
}

impl Route {
    fn of(addr: &TransportAddr) -> Self {
        if addr.is_relay() {
            Self::Relay
        } else {
            Self::Direct
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Traffic {
    pub direct: u64,
    pub relay: u64,
}

impl Traffic {
    fn add(&mut self, route: Route, bytes: u64) {
        match route {
            Route::Direct => self.direct += bytes,
            Route::Relay => self.relay += bytes,
        }
    }
}

type PathBytes = Arc<Mutex<HashMap<PathId, (Route, u64)>>>;

fn bytes(stats: &PathStats) -> u64 {
    stats.udp_tx.bytes + stats.udp_rx.bytes
}

fn snapshot(connection: &Connection, paths: &mut HashMap<PathId, (Route, u64)>) {
    for path in &connection.paths() {
        paths.insert(
            path.id(),
            (Route::of(path.remote_addr()), bytes(&path.stats())),
        );
    }
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
    #[error("iroh blobs: {0}")]
    Blob(String),
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
    fn to_addr(&self, ours: &Relays) -> Result<EndpointAddr, Error> {
        let id: EndpointId = self
            .id
            .parse()
            .map_err(|_| Error::BadAddress("node id".into()))?;
        let relays = self
            .relays
            .iter()
            .filter_map(|r| r.parse::<RelayUrl>().ok())
            .filter(|r| ours.accepts_from_peer(r))
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
    connect_timeout: Duration,
}

pub struct IrohPipe {
    role: Role,
    send: SendStream,
    recv: RecvStream,
    connection: Connection,
    endpoint: Endpoint,
    description: String,
    server: Option<tokio::task::JoinHandle<()>>,
    route: watch::Receiver<Option<Route>>,
    first_route: Option<Route>,
    paths: PathBytes,
    follower: tokio::task::JoinHandle<()>,
}

impl IrohTransport {
    pub async fn close(self) {
        self.endpoint.close().await;
    }

    pub async fn bind(role: Role, relays: Relays) -> Result<Self, Error> {
        let endpoint = Endpoint::builder(presets::Minimal)
            .secret_key(SecretKey::generate())
            .alpns(vec![ALPN.to_vec()])
            .relay_mode(relay_mode(&relays))
            .bind()
            .await
            .map_err(|e| Error::Bind(e.to_string()))?;
        Ok(Self {
            role,
            endpoint,
            relays,
            connect_timeout: CONNECT_TIMEOUT,
        })
    }

    #[doc(hidden)]
    #[must_use]
    pub fn with_connect_timeout(mut self, timeout: Duration) -> Self {
        self.connect_timeout = timeout;
        self
    }

    pub async fn info(&self) -> IrohInfo {
        let ready = async {
            if self.relays != Relays::Disabled {
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
            Ok((connection, send, recv)) => {
                let events = connection.path_events();
                let initial = selected_route(&connection);
                let description = match initial {
                    Some(Route::Relay) => "iroh, via relay until a direct path opens",
                    Some(Route::Direct) => "iroh, direct",
                    None => "iroh",
                }
                .to_owned();
                let (route_tx, route) = watch::channel(initial);
                let paths = PathBytes::default();
                let follower = tokio::spawn(follow_paths(
                    events,
                    connection.clone(),
                    route_tx,
                    Arc::clone(&paths),
                ));
                Ok(IrohPipe {
                    role: self.role,
                    send,
                    recv,
                    connection,
                    endpoint: self.endpoint,
                    description,
                    server: None,
                    route,
                    first_route: initial,
                    paths,
                    follower,
                })
            }
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
    ) -> Result<(Connection, SendStream, RecvStream), Error> {
        let addr = theirs.to_addr(&self.relays)?;
        let confirm = wormhole_key.derive(CONFIRM_LABEL);
        let ours = self.endpoint.id();
        let (connection, mut send, mut recv) = self.first_stream(&addr).await?;

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
        Ok((connection, send, recv))
    }
}

impl IrohTransport {
    async fn first_stream(
        &self,
        addr: &EndpointAddr,
    ) -> Result<(Connection, SendStream, RecvStream), Error> {
        let role = self.role;
        let stream = move |connection: Connection| async move {
            let streams = match role {
                Role::Sender => connection.open_bi().await,
                Role::Receiver => connection.accept_bi().await,
            };
            streams
                .map(|(send, recv)| (connection, send, recv))
                .map_err(|e| Some(e.to_string()))
        };
        let deadline = tokio::time::sleep(self.connect_timeout);
        let mut deadline = std::pin::pin!(deadline);
        let mut dialing = std::pin::pin!(self.endpoint.connect(addr.clone(), ALPN));
        let mut dial_failed = None;
        let mut stream_failed = None;
        let mut waiting = FuturesUnordered::new();
        loop {
            tokio::select! {
                dialled = &mut dialing, if dial_failed.is_none() => match dialled {
                    Ok(connection) => {
                        dial_failed = Some(String::new());
                        waiting.push(stream(connection).boxed());
                    }
                    Err(e) => dial_failed = Some(e.to_string()),
                },
                incoming = self.endpoint.accept() => {
                    let Some(incoming) = incoming else {
                        return Err(Error::Connect("the endpoint closed".into()));
                    };
                    if waiting.len() >= MAX_HANDSHAKES {
                        continue;
                    }
                    let id = addr.id;
                    waiting.push(
                        async move {
                            let connection = incoming.await.map_err(|_| None)?;
                            if connection.remote_id() != id {
                                connection.close(1u8.into(), b"not you");
                                return Err(None);
                            }
                            stream(connection).await
                        }
                        .boxed(),
                    );
                }
                Some(settled) = waiting.next() => match settled {
                    Ok(won) => return Ok(won),
                    Err(Some(e)) => stream_failed = Some(e),
                    Err(None) => {}
                },
                () = &mut deadline => {
                    return Err(Error::Connect(
                        dial_failed
                            .filter(|e| !e.is_empty())
                            .or(stream_failed)
                            .unwrap_or_else(|| "timed out".into()),
                    ));
                }
            }
        }
    }
}

fn selected_route(connection: &Connection) -> Option<Route> {
    connection
        .paths()
        .into_iter()
        .find(iroh::endpoint::Path::is_selected)
        .map(|path| Route::of(path.remote_addr()))
}

fn select(route: &watch::Sender<Option<Route>>, selected: Route) {
    route.send_if_modified(|now| now.replace(selected) != Some(selected));
}

async fn follow_paths(
    mut events: iroh::endpoint::PathEventStream,
    connection: Connection,
    route: watch::Sender<Option<Route>>,
    paths: PathBytes,
) {
    let mut tick = tokio::time::interval(SNAPSHOT_EVERY);
    loop {
        let event = tokio::select! {
            event = events.next() => match event {
                Some(event) => event,
                None => return,
            },
            _ = tick.tick() => {
                snapshot(&connection, &mut paths.lock().expect("the traffic lock is never poisoned"));
                continue;
            }
        };
        match event {
            PathEvent::Selected { remote_addr, .. } => select(&route, Route::of(&remote_addr)),
            PathEvent::Closed {
                id,
                remote_addr,
                last_stats,
                ..
            } => {
                paths
                    .lock()
                    .expect("the traffic lock is never poisoned")
                    .insert(id, (Route::of(&remote_addr), bytes(&last_stats)));
            }
            PathEvent::Lagged { .. } => {
                if let Some(selected) = selected_route(&connection) {
                    select(&route, selected);
                }
            }
            _ => {}
        }
    }
}

fn relay_mode(relays: &Relays) -> RelayMode {
    match relays {
        Relays::Default => iroh::endpoint::default_relay_mode(),
        Relays::Disabled => RelayMode::Disabled,
        Relays::Custom(urls) => RelayMode::Custom(urls.iter().cloned().collect()),
    }
}

fn constant_time_eq(a: &[u8; 32], b: &[u8; 32]) -> bool {
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

impl Drop for IrohPipe {
    fn drop(&mut self) {
        self.follower.abort();
        if let Some(server) = self.server.take() {
            server.abort();
        }
    }
}

impl IrohPipe {
    #[must_use]
    pub fn describe(&self) -> &str {
        &self.description
    }

    #[must_use]
    pub fn route(&self) -> watch::Receiver<Option<Route>> {
        self.route.clone()
    }

    #[must_use]
    pub fn traffic(&self) -> Traffic {
        let mut paths = self
            .paths
            .lock()
            .expect("the traffic lock is never poisoned")
            .clone();
        snapshot(&self.connection, &mut paths);
        let mut traffic = Traffic::default();
        for (route, bytes) in paths.into_values() {
            traffic.add(route, bytes);
        }
        traffic
    }

    #[must_use]
    pub fn first_route(&self) -> Option<Route> {
        self.first_route
    }

    pub async fn send_last(&mut self, data: &[u8]) -> Result<(), Error> {
        self.send
            .write_all(data)
            .await
            .map_err(|e| Error::Stream(e.to_string()))?;
        self.send.finish().map_err(|e| Error::Stream(e.to_string()))
    }

    pub async fn receive_last(&mut self) -> Result<Vec<u8>, Error> {
        self.recv
            .read_to_end(MAX_ACK)
            .await
            .map_err(|e| Error::Stream(e.to_string()))
    }

    pub async fn abort(mut self) {
        self.follower.abort();
        if let Some(server) = self.server.take() {
            server.abort();
        }
        self.connection.close(3u8.into(), b"failed");
        self.endpoint.close().await;
    }

    pub async fn finish(mut self) {
        self.follower.abort();
        if let Some(server) = self.server.take() {
            server.abort();
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relays_map_to_iroh_relay_modes() {
        assert_eq!(relay_mode(&Relays::Disabled), RelayMode::Disabled);
        assert_eq!(
            relay_mode(&Relays::Default),
            iroh::endpoint::default_relay_mode()
        );
        let url: RelayUrl = "http://127.0.0.1:3340".parse().unwrap();
        assert_eq!(
            relay_mode(&Relays::Custom(vec![url.clone()])),
            RelayMode::Custom(url.into())
        );
    }

    #[test]
    fn relays_parse_from_their_names_or_urls() {
        assert_eq!("default".parse(), Ok(Relays::Default));
        assert_eq!("disabled".parse(), Ok(Relays::Disabled));
        let Ok(Relays::Custom(urls)) = "http://127.0.0.1:3340, https://relay.example".parse()
        else {
            panic!("two relay URLs");
        };
        assert_eq!(urls.len(), 2);
        for bad in [
            "",
            " , ",
            "nowhere",
            "ftp://relay.example",
            "https://a, nope",
        ] {
            assert!(bad.parse::<Relays>().is_err(), "{bad}");
        }
        assert_eq!(" Default ".parse(), Ok(Relays::Default));
        assert_eq!("DISABLED".parse(), Ok(Relays::Disabled));
        assert!(matches!(
            "https://relay.example,".parse(),
            Ok(Relays::Custom(urls)) if urls.len() == 1
        ));
    }

    #[test]
    fn peer_relays_are_kept_only_when_we_would_trust_them() {
        let url = |u: &str| -> RelayUrl { u.parse().unwrap() };
        let lan = url("http://192.168.1.1/");
        let ours = Relays::Custom(vec![lan.clone()]);
        for (relays, relay, kept) in [
            (Relays::Default, url("https://relay.example/"), true),
            (Relays::Default, url("http://127.0.0.1:3340/"), true),
            (Relays::Default, url("http://[::1]:3340/"), true),
            (Relays::Default, url("http://LocalHost:3340/"), true),
            (Relays::Default, lan.clone(), false),
            (ours.clone(), lan.clone(), true),
            (ours, url("http://192.168.1.2/"), false),
            (Relays::Disabled, url("https://relay.example/"), false),
            (Relays::Default, url("ftp://relay.example/"), false),
        ] {
            assert_eq!(relays.accepts_from_peer(&relay), kept, "{relays:?} {relay}");
        }
        let info = IrohInfo {
            id: SecretKey::generate().public().to_string(),
            relays: vec![
                "http://192.168.1.1/".into(),
                "https://relay.example/".into(),
            ],
            direct: vec!["127.0.0.1:9".into()],
        };
        let addr = info.to_addr(&Relays::Default).unwrap();
        assert_eq!(addr.relay_urls().count(), 1);
        let addr = info.to_addr(&Relays::Disabled).unwrap();
        assert_eq!(addr.relay_urls().count(), 0);
    }

    #[test]
    fn tags_compare_whole() {
        assert!(constant_time_eq(&[7; 32], &[7; 32]));
        let mut other = [7; 32];
        other[31] = 8;
        assert!(!constant_time_eq(&[7; 32], &other));
    }
}
