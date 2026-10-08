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
}

#[derive(Default)]
struct State {
    nameplates: HashMap<String, String>,
    mailboxes: HashMap<String, Mailbox>,
    next_mailbox: u64,
}

#[derive(Default)]
struct Mailbox {
    messages: Vec<Value>,
    listeners: Vec<UnboundedSender<String>>,
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
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                tokio::spawn(serve(stream, state.clone(), welcome.clone()));
            }
        });
        Self { addr }
    }

    pub fn url(&self) -> String {
        format!("ws://{}/v1", self.addr)
    }
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
    while let Some(Ok(frame)) = source.next().await {
        let Frame::Text(text) = frame else { continue };
        let Ok(msg) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        let _ = tx.send(json!({"type": "ack", "id": msg["id"]}).to_string());
        let field = |k: &str| msg[k].as_str().unwrap_or_default().to_owned();
        let reply = match msg["type"].as_str().unwrap_or_default() {
            "bind" => {
                side = field("side");
                None
            }
            "allocate" => {
                let mut state = state.lock().unwrap();
                let used: BTreeSet<u64> = state
                    .nameplates
                    .keys()
                    .filter_map(|n| n.parse().ok())
                    .collect();
                let free = (1..=u64::MAX)
                    .find(|n| !used.contains(n))
                    .unwrap()
                    .to_string();
                state.next_mailbox += 1;
                let mailbox = format!("mb{}", state.next_mailbox);
                state.nameplates.insert(free.clone(), mailbox);
                Some(json!({"type": "allocated", "nameplate": free}))
            }
            "claim" => {
                let mut state = state.lock().unwrap();
                let State {
                    nameplates,
                    next_mailbox,
                    ..
                } = &mut *state;
                let mailbox = nameplates
                    .entry(field("nameplate"))
                    .or_insert_with(|| {
                        *next_mailbox += 1;
                        format!("mb{next_mailbox}")
                    })
                    .clone();
                Some(json!({"type": "claimed", "mailbox": mailbox}))
            }
            "release" => Some(json!({"type": "released"})),
            "open" => {
                let mut state = state.lock().unwrap();
                let mailbox = state.mailboxes.entry(field("mailbox")).or_default();
                for old in &mailbox.messages {
                    let _ = tx.send(old.to_string());
                }
                mailbox.listeners.push(tx.clone());
                None
            }
            "add" => {
                let mut state = state.lock().unwrap();
                let message = json!({
                    "type": "message",
                    "side": side,
                    "phase": msg["phase"],
                    "body": msg["body"],
                    "id": msg["id"],
                });
                for mailbox in state.mailboxes.values_mut() {
                    if mailbox.listeners.iter().any(|l| l.same_channel(&tx)) {
                        mailbox.messages.push(message.clone());
                        mailbox
                            .listeners
                            .retain(|l| l.send(message.to_string()).is_ok());
                    }
                }
                None
            }
            "close" => Some(json!({"type": "closed"})),
            _ => None,
        };
        if let Some(reply) = reply {
            let _ = tx.send(reply.to_string());
        }
    }
}
