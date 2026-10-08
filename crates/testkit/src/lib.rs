use std::collections::{BTreeSet, HashMap};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use futures_util::{SinkExt, StreamExt};
use serde_json::{Map, Value, json};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};
use tokio_tungstenite::tungstenite::Message as Frame;

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
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let state = Arc::new(Mutex::new(State::default()));
        let welcome = Value::Object(welcome);
        let shared = state.clone();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                tokio::spawn(serve(stream, shared.clone(), welcome.clone()));
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

async fn serve(stream: TcpStream, state: Arc<Mutex<State>>, welcome: Value) {
    let Ok(ws) = tokio_tungstenite::accept_async(stream).await else {
        return;
    };
    let (mut sink, mut source) = ws.split();
    let (tx, mut rx) = unbounded_channel::<String>();
    tokio::spawn(async move {
        while let Some(frame) = rx.recv().await {
            if sink.send(Frame::text(frame)).await.is_err() {
                break;
            }
        }
    });
    let _ = tx.send(json!({"type": "welcome", "welcome": welcome}).to_string());
    let mut side = String::new();
    let mut opened: Option<String> = None;
    while let Some(Ok(frame)) = source.next().await {
        let Frame::Text(text) = frame else { continue };
        let Ok(msg) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        let _ = tx.send(json!({"type": "ack", "id": msg["id"]}).to_string());
        let field = |k: &str| msg[k].as_str().unwrap_or_default().to_owned();
        let reply: Result<Option<Value>, &str> = {
            let mut state = state.lock().unwrap();
            match msg["type"].as_str().unwrap_or_default() {
                "bind" => {
                    side = field("side");
                    Ok(None)
                }
                _ if side.is_empty() => Err("must bind first"),
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
                    claim(&mut state, &free, &side)
                        .map(|_| Some(json!({"type": "allocated", "nameplate": free})))
                }
                "claim" => claim(&mut state, &field("nameplate"), &side)
                    .map(|mailbox| Some(json!({"type": "claimed", "mailbox": mailbox}))),
                "release" => {
                    let nameplate = field("nameplate");
                    if let Some(entry) = state.nameplates.get_mut(&nameplate) {
                        entry.sides.remove(&side);
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
                    opened = Some(id);
                    Ok(None)
                }
                "add" => match opened.as_ref().and_then(|id| state.mailboxes.get_mut(id)) {
                    None => Err("must open mailbox before adding"),
                    Some(mailbox) => {
                        let message = json!({
                            "type": "message",
                            "side": side,
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
        };
        let reply = reply
            .unwrap_or_else(|error| Some(json!({"type": "error", "error": error, "orig": msg})));
        if let Some(reply) = reply {
            let _ = tx.send(reply.to_string());
        }
    }
}
