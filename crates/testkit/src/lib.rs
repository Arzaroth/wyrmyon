use std::collections::{BTreeSet, HashMap};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use futures_util::{SinkExt, StreamExt};
use serde_json::{Map, Value, json};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};
use tokio_tungstenite::tungstenite::Message as Frame;

#[derive(Default)]
pub struct Quirks {
    pub welcome: Map<String, Value>,
    pub allocate_as: Option<String>,
    pub refuse_allocate: bool,
    pub flood_on_claim: usize,
    pub hang_up_after_welcome: bool,
    pub chatty: bool,
}

pub struct MailboxServer {
    addr: SocketAddr,
    state: Arc<Mutex<State>>,
}

#[derive(Default)]
struct State {
    nameplates: HashMap<String, Nameplate>,
    mailboxes: HashMap<String, Mailbox>,
    next_mailbox: u64,
}

struct Nameplate {
    mailbox: String,
    sides: BTreeSet<String>,
}

#[derive(Default)]
struct Mailbox {
    messages: Vec<Value>,
    listeners: Vec<UnboundedSender<String>>,
    moods: Vec<String>,
}

impl MailboxServer {
    pub async fn start() -> Self {
        Self::start_with_welcome(Map::new()).await
    }

    pub async fn start_with_welcome(welcome: Map<String, Value>) -> Self {
        Self::start_with(Quirks {
            welcome,
            ..Quirks::default()
        })
        .await
    }

    pub async fn start_with(quirks: Quirks) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let state = Arc::new(Mutex::new(State::default()));
        let shared = state.clone();
        let quirks = Arc::new(quirks);
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                tokio::spawn(serve(stream, shared.clone(), quirks.clone()));
            }
        });
        Self { addr, state }
    }

    pub fn url(&self) -> String {
        format!("ws://{}/v1", self.addr)
    }

    pub fn claimed_nameplates(&self) -> Vec<String> {
        let state = self.state.lock().unwrap();
        let mut claimed: Vec<String> = state
            .nameplates
            .iter()
            .filter(|(_, n)| !n.sides.is_empty())
            .map(|(id, _)| id.clone())
            .collect();
        claimed.sort();
        claimed
    }

    pub fn moods(&self) -> Vec<String> {
        let state = self.state.lock().unwrap();
        let mut moods: Vec<String> = state
            .mailboxes
            .values()
            .flat_map(|m| m.moods.clone())
            .collect();
        moods.sort();
        moods
    }
}

fn claim(state: &mut State, nameplate: &str, side: &str) -> Result<String, &'static str> {
    let State {
        nameplates,
        next_mailbox,
        ..
    } = state;
    let entry = nameplates.entry(nameplate.to_owned()).or_insert_with(|| {
        *next_mailbox += 1;
        Nameplate {
            mailbox: format!("mb{next_mailbox}"),
            sides: BTreeSet::new(),
        }
    });
    if entry.sides.len() >= 2 && !entry.sides.contains(side) {
        return Err("crowded");
    }
    entry.sides.insert(side.to_owned());
    Ok(entry.mailbox.clone())
}

fn respond(
    state: &Mutex<State>,
    quirks: &Quirks,
    session: &mut Session,
    tx: &UnboundedSender<String>,
    msg: &Value,
) -> Result<Option<Value>, &'static str> {
    let field = |k: &str| msg[k].as_str().unwrap_or_default().to_owned();
    let mut state = state.lock().unwrap();
    match msg["type"].as_str().unwrap_or_default() {
        "bind" => {
            session.side = field("side");
            Ok(None)
        }
        _ if session.side.is_empty() => Err("must bind first"),
        "allocate" if quirks.refuse_allocate => Err("no nameplates left"),
        "allocate" if quirks.allocate_as.is_some() => Ok(Some(
            json!({"type": "allocated", "nameplate": quirks.allocate_as}),
        )),
        "allocate" => {
            let used: BTreeSet<u64> = state
                .nameplates
                .keys()
                .filter_map(|n| n.parse().ok())
                .collect();
            let free = (1..=u64::MAX)
                .find(|n| !used.contains(n))
                .unwrap()
                .to_string();
            claim(&mut state, &free, &session.side)
                .map(|_| Some(json!({"type": "allocated", "nameplate": free})))
        }
        "claim" if quirks.flood_on_claim > 0 => {
            for n in 0..quirks.flood_on_claim {
                let _ = tx.send(
                    json!({"type": "message", "side": format!("s{n}"), "phase": "x", "body": ""})
                        .to_string(),
                );
            }
            Ok(None)
        }
        "claim" => claim(&mut state, &field("nameplate"), &session.side)
            .map(|mailbox| Some(json!({"type": "claimed", "mailbox": mailbox}))),
        "release" => {
            let nameplate = field("nameplate");
            if let Some(entry) = state.nameplates.get_mut(&nameplate) {
                entry.sides.remove(&session.side);
                if entry.sides.is_empty() {
                    state.nameplates.remove(&nameplate);
                }
            }
            Ok(Some(json!({"type": "released"})))
        }
        "open" => {
            let id = field("mailbox");
            let mailbox = state.mailboxes.entry(id.clone()).or_default();
            for old in &mailbox.messages {
                let _ = tx.send(old.to_string());
            }
            mailbox.listeners.push(tx.clone());
            session.opened = Some(id);
            Ok(None)
        }
        "add" => match session
            .opened
            .as_ref()
            .and_then(|id| state.mailboxes.get_mut(id))
        {
            None => Err("must open mailbox before adding"),
            Some(mailbox) => {
                let message = json!({
                    "type": "message",
                    "side": session.side,
                    "phase": msg["phase"],
                    "body": msg["body"],
                    "id": msg["id"],
                });
                mailbox.messages.push(message.clone());
                mailbox
                    .listeners
                    .retain(|l| l.send(message.to_string()).is_ok());
                Ok(None)
            }
        },
        "close" => {
            if let Some(mailbox) = state.mailboxes.get_mut(&field("mailbox")) {
                mailbox.moods.push(field("mood"));
            }
            Ok(Some(json!({"type": "closed"})))
        }
        _ => Ok(None),
    }
}

struct Session {
    side: String,
    opened: Option<String>,
}

async fn serve(stream: TcpStream, state: Arc<Mutex<State>>, quirks: Arc<Quirks>) {
    let Ok(ws) = tokio_tungstenite::accept_async(stream).await else {
        return;
    };
    let (mut sink, mut source) = ws.split();
    let (tx, mut rx) = unbounded_channel::<String>();
    tokio::spawn(async move {
        while let Some(frame) = rx.recv().await {
            if frame.is_empty() {
                let _ = sink.send(Frame::Close(None)).await;
                break;
            }
            if sink.send(Frame::text(frame)).await.is_err() {
                break;
            }
        }
    });
    let chatter = || json!({"type": "released"}).to_string();
    if quirks.chatty {
        let _ = tx.send(chatter());
    }
    let _ = tx.send(json!({"type": "welcome", "welcome": quirks.welcome}).to_string());
    if quirks.hang_up_after_welcome {
        let _ = tx.send(String::new());
        return;
    }
    let mut session = Session {
        side: String::new(),
        opened: None,
    };
    while let Some(Ok(frame)) = source.next().await {
        let Frame::Text(text) = frame else { continue };
        let Ok(msg) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        let _ = tx.send(json!({"type": "ack", "id": msg["id"]}).to_string());
        let reply = respond(&state, &quirks, &mut session, &tx, &msg);
        let reply = reply
            .unwrap_or_else(|error| Some(json!({"type": "error", "error": error, "orig": msg})));
        if let Some(reply) = reply {
            if quirks.chatty {
                let _ = tx.send(chatter());
            }
            let _ = tx.send(reply.to_string());
        }
    }
}

pub struct TransitRelay {
    addr: SocketAddr,
}

type Waiting = Arc<Mutex<HashMap<String, (String, TcpStream)>>>;

impl TransitRelay {
    pub async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let waiting: Waiting = Arc::default();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                tokio::spawn(relay(stream, waiting.clone()));
            }
        });
        Self { addr }
    }

    pub fn hint(&self) -> String {
        format!("tcp:127.0.0.1:{}", self.addr.port())
    }
}

async fn relay(mut stream: TcpStream, waiting: Waiting) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut line = Vec::new();
    let mut byte = [0u8; 1];
    while line.len() < 256 {
        if stream.read_exact(&mut byte).await.is_err() {
            return;
        }
        if byte[0] == b'\n' {
            break;
        }
        line.push(byte[0]);
    }
    let line = String::from_utf8_lossy(&line).into_owned();
    let Some(rest) = line.strip_prefix("please relay ") else {
        return;
    };
    let (token, side) = rest.split_once(" for side ").unwrap_or((rest, ""));
    let partner = {
        let mut waiting = waiting.lock().unwrap();
        match waiting.remove(token) {
            Some((other_side, other)) if other_side != side => Some(other),
            _ => {
                waiting.insert(token.to_owned(), (side.to_owned(), stream));
                return;
            }
        }
    };
    let Some(mut other) = partner else { return };
    if stream.write_all(b"ok\n").await.is_err() || other.write_all(b"ok\n").await.is_err() {
        return;
    }
    let _ = tokio::io::copy_bidirectional(&mut stream, &mut other).await;
}
