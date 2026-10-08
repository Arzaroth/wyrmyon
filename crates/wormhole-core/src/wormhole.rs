use std::collections::BTreeMap;
use std::time::Duration;

use serde_json::{Map, Value, json};
use spake2::{Ed25519Group, Identity, Password, Spake2};
use unicode_normalization::UnicodeNormalization;

use crate::code::{self, Code};
use crate::connection::Connection;
use crate::crypto::{KEY_LEN, Key};
use crate::server::{Inbound, Mood, Outbound};
use crate::{APPID, Error, PUBLIC_RELAY};

const CLOSE_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_EARLY_PHASES: usize = 64;

#[derive(Debug, Clone)]
pub struct Config {
    pub relay_url: String,
    pub appid: String,
    pub app_versions: Value,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            relay_url: PUBLIC_RELAY.to_owned(),
            appid: APPID.to_owned(),
            app_versions: Value::Object(Map::new()),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Welcome {
    pub motd: Option<String>,
}

pub struct Pending {
    conn: Connection,
    side: String,
    code: Code,
    nameplate: Option<String>,
    mailbox: String,
    spake: Option<Spake2<Ed25519Group>>,
    app_versions: Value,
    welcome: Welcome,
}

pub struct Wormhole {
    conn: Connection,
    side: String,
    mailbox: String,
    key: Key,
    inbox: Inbox,
    their_app_versions: Value,
    welcome: Welcome,
    next_tx: u64,
}

struct Inbox {
    their_side: String,
    key: Key,
    next_rx: u64,
    received: BTreeMap<u64, Vec<u8>>,
}

pub async fn create(config: &Config, words: usize) -> Result<Pending, Error> {
    let (mut conn, side, welcome) = bind(config).await?;
    let allocated = async {
        conn.send(&Outbound::Allocate).await?;
        let nameplate = conn
            .expect(|msg| match msg {
                Inbound::Allocated { nameplate } => Some(nameplate),
                _ => None,
            })
            .await?;
        if code::is_nameplate(&nameplate) {
            Ok(nameplate)
        } else {
            Err(Error::Protocol(
                "the server allocated a malformed nameplate".into(),
            ))
        }
    }
    .await;
    match allocated {
        Ok(nameplate) => {
            let code = Code::with_nameplate(&nameplate, &code::choose_words(words));
            Pending::start(conn, side, welcome, code, config).await
        }
        Err(e) => {
            conn.shutdown().await;
            Err(e)
        }
    }
}

pub async fn join(config: &Config, code: Code) -> Result<Pending, Error> {
    let (conn, side, welcome) = bind(config).await?;
    Pending::start(conn, side, welcome, code, config).await
}

async fn bind(config: &Config) -> Result<(Connection, String, Welcome), Error> {
    let side = hex::encode(rand::random::<[u8; 5]>());
    let (mut conn, welcome) = Connection::open(&config.relay_url, side.clone()).await?;
    if let Some(error) = welcome.get("error") {
        conn.shutdown().await;
        return Err(Error::Welcome(
            error.as_str().unwrap_or("refused").to_owned(),
        ));
    }
    let welcome = Welcome {
        motd: welcome
            .get("motd")
            .and_then(Value::as_str)
            .map(str::to_owned),
    };
    let bound = conn
        .send(&Outbound::Bind {
            appid: config.appid.clone(),
            side: side.clone(),
            client_version: ("wyrmyon".into(), env!("CARGO_PKG_VERSION").into()),
        })
        .await;
    if let Err(e) = bound {
        conn.shutdown().await;
        return Err(e);
    }
    Ok((conn, side, welcome))
}

impl Pending {
    async fn start(
        mut conn: Connection,
        side: String,
        welcome: Welcome,
        code: Code,
        config: &Config,
    ) -> Result<Self, Error> {
        let nameplate = code.nameplate().to_owned();
        let mailbox = match claim_and_open(&mut conn, &nameplate).await {
            Ok(mailbox) => mailbox,
            Err(e) => {
                let mood = e.mood();
                close_session(conn, Some(nameplate), None, mood).await;
                return Err(e);
            }
        };
        let password: String = code.as_str().nfc().collect();
        let (spake, outbound) = Spake2::<Ed25519Group>::start_symmetric(
            &Password::new(password.as_bytes()),
            &Identity::new(config.appid.as_bytes()),
        );
        let pake = json!({ "pake_v1": hex::encode(outbound) }).to_string();
        let added = conn
            .send(&Outbound::Add {
                phase: "pake".into(),
                body: hex::encode(pake),
            })
            .await;
        if let Err(e) = added {
            close_session(conn, Some(nameplate), Some(mailbox), e.mood()).await;
            return Err(e);
        }
        Ok(Self {
            conn,
            side,
            code,
            nameplate: Some(nameplate),
            mailbox,
            spake: Some(spake),
            app_versions: config.app_versions.clone(),
            welcome,
        })
    }

    #[must_use]
    pub fn code(&self) -> &Code {
        &self.code
    }

    #[must_use]
    pub fn welcome(&self) -> &Welcome {
        &self.welcome
    }

    pub async fn pair(mut self) -> Result<Wormhole, Error> {
        match self.exchange().await {
            Ok((inbox, their_app_versions)) => Ok(Wormhole {
                conn: self.conn,
                side: self.side,
                mailbox: self.mailbox,
                key: inbox.key.clone(),
                inbox,
                their_app_versions,
                welcome: self.welcome,
                next_tx: 0,
            }),
            Err(e) => {
                let mood = e.mood();
                close_session(self.conn, self.nameplate, Some(self.mailbox), mood).await;
                Err(e)
            }
        }
    }

    async fn exchange(&mut self) -> Result<(Inbox, Value), Error> {
        let (their_side, their_pake) = loop {
            let msg = self.conn.next_message().await?;
            if msg.phase == "pake" {
                break (msg.side, msg.body);
            }
        };
        if let Some(nameplate) = self.nameplate.take() {
            self.conn.send(&Outbound::Release { nameplate }).await?;
        }
        let their_pake: Value = serde_json::from_slice(&their_pake)
            .map_err(|_| Error::Protocol("unreadable PAKE message".into()))?;
        let their_pake = their_pake["pake_v1"]
            .as_str()
            .and_then(|h| hex::decode(h).ok())
            .ok_or_else(|| Error::Protocol("PAKE message without pake_v1".into()))?;
        let shared = self
            .spake
            .take()
            .expect("exchange runs once")
            .finish(&their_pake)
            .map_err(|_| Error::Protocol("invalid PAKE message".into()))?;
        let key = Key::from_bytes(
            <[u8; KEY_LEN]>::try_from(shared.as_slice())
                .map_err(|_| Error::Protocol("PAKE key length".into()))?,
        );

        let versions = json!({ "app_versions": self.app_versions }).to_string();
        let sealed = key
            .derive_phase(&self.side, "version")
            .encrypt(versions.as_bytes());
        self.conn
            .send(&Outbound::Add {
                phase: "version".into(),
                body: hex::encode(sealed),
            })
            .await?;

        let mut inbox = Inbox {
            their_side,
            key,
            next_rx: 0,
            received: BTreeMap::new(),
        };
        let their_versions = loop {
            let msg = self.conn.next_message().await?;
            if msg.side != inbox.their_side {
                continue;
            }
            if msg.phase == "version" {
                break msg.body;
            }
            inbox.stash(&msg.phase, &msg.body)?;
        };
        let their_versions = inbox
            .key
            .derive_phase(&inbox.their_side, "version")
            .decrypt(&their_versions)
            .ok_or(Error::WrongCode)?;
        let their_versions: Value = serde_json::from_slice(&their_versions)
            .map_err(|_| Error::Protocol("unreadable version message".into()))?;
        let their_app_versions = their_versions
            .get("app_versions")
            .cloned()
            .unwrap_or(Value::Object(Map::new()));
        Ok((inbox, their_app_versions))
    }

    pub async fn abandon(self) {
        close_session(self.conn, self.nameplate, Some(self.mailbox), Mood::Lonely).await;
    }
}

impl Inbox {
    fn stash(&mut self, phase: &str, body: &[u8]) -> Result<(), Error> {
        let Ok(index) = phase.parse::<u64>() else {
            return Ok(());
        };
        if index < self.next_rx || self.received.contains_key(&index) {
            return Ok(());
        }
        if self.received.len() >= MAX_EARLY_PHASES {
            return Err(Error::Protocol("too many out-of-order messages".into()));
        }
        let plaintext = self
            .key
            .derive_phase(&self.their_side, phase)
            .decrypt(body)
            .ok_or(Error::Tampered)?;
        self.received.insert(index, plaintext);
        Ok(())
    }

    fn take_next(&mut self) -> Option<Vec<u8>> {
        let plaintext = self.received.remove(&self.next_rx)?;
        self.next_rx += 1;
        Some(plaintext)
    }
}

impl Wormhole {
    #[must_use]
    pub fn key(&self) -> &Key {
        &self.key
    }

    #[must_use]
    pub fn transit_key(&self) -> Key {
        self.key.derive(format!("{APPID}/transit-key").as_bytes())
    }

    #[must_use]
    pub fn verifier(&self) -> [u8; KEY_LEN] {
        *self.key.derive(b"wormhole:verifier").as_bytes()
    }

    #[must_use]
    pub fn their_app_versions(&self) -> &Value {
        &self.their_app_versions
    }

    #[must_use]
    pub fn welcome(&self) -> &Welcome {
        &self.welcome
    }

    pub async fn send(&mut self, plaintext: &[u8]) -> Result<(), Error> {
        let phase = self.next_tx.to_string();
        self.next_tx += 1;
        let sealed = self.key.derive_phase(&self.side, &phase).encrypt(plaintext);
        self.conn
            .send(&Outbound::Add {
                phase,
                body: hex::encode(sealed),
            })
            .await
    }

    pub async fn send_json(&mut self, value: &Value) -> Result<(), Error> {
        self.send(value.to_string().as_bytes()).await
    }

    pub async fn receive(&mut self) -> Result<Vec<u8>, Error> {
        loop {
            if let Some(plaintext) = self.inbox.take_next() {
                return Ok(plaintext);
            }
            let msg = self.conn.next_message().await?;
            if msg.side == self.inbox.their_side {
                self.inbox.stash(&msg.phase, &msg.body)?;
            }
        }
    }

    pub async fn receive_json(&mut self) -> Result<Value, Error> {
        let plaintext = self.receive().await?;
        serde_json::from_slice(&plaintext)
            .map_err(|_| Error::Protocol("peer sent a message that is not JSON".into()))
    }

    pub async fn close(self, mood: Mood) {
        close_session(self.conn, None, Some(self.mailbox), mood).await;
    }
}

async fn claim_and_open(conn: &mut Connection, nameplate: &str) -> Result<String, Error> {
    conn.send(&Outbound::Claim {
        nameplate: nameplate.to_owned(),
    })
    .await?;
    let mailbox = conn
        .expect(|msg| match msg {
            Inbound::Claimed { mailbox } => Some(mailbox),
            _ => None,
        })
        .await?;
    conn.send(&Outbound::Open {
        mailbox: mailbox.clone(),
    })
    .await?;
    Ok(mailbox)
}

async fn close_session(
    mut conn: Connection,
    nameplate: Option<String>,
    mailbox: Option<String>,
    mood: Mood,
) {
    let closing = async {
        if let Some(nameplate) = nameplate {
            conn.send(&Outbound::Release { nameplate }).await?;
        }
        if let Some(mailbox) = mailbox {
            conn.send(&Outbound::Close { mailbox, mood }).await?;
            conn.expect(|msg| matches!(msg, Inbound::Closed).then_some(()))
                .await?;
        }
        Ok::<_, Error>(())
    };
    let _ = tokio::time::timeout(CLOSE_TIMEOUT, closing).await;
    conn.shutdown().await;
}
