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
    spake: Spake2<Ed25519Group>,
    app_versions: Value,
    welcome: Welcome,
}

pub struct Wormhole {
    conn: Connection,
    side: String,
    their_side: String,
    mailbox: String,
    key: Key,
    their_app_versions: Value,
    welcome: Welcome,
    next_tx: u64,
    next_rx: u64,
    received: BTreeMap<u64, Vec<u8>>,
}

pub async fn create(config: &Config, words: usize) -> Result<Pending, Error> {
    let (mut conn, side, welcome) = bind(config).await?;
    conn.send(&Outbound::Allocate).await?;
    let nameplate = conn
        .expect(|msg| match msg {
            Inbound::Allocated { nameplate } => Some(nameplate),
            _ => None,
        })
        .await?;
    let code = Code::with_nameplate(&nameplate, &code::choose_words(words));
    Pending::start(conn, side, welcome, code, config).await
}

pub async fn connect(config: &Config, code: Code) -> Result<Wormhole, Error> {
    let (conn, side, welcome) = bind(config).await?;
    Pending::start(conn, side, welcome, code, config)
        .await?
        .pair()
        .await
}

async fn bind(config: &Config) -> Result<(Connection, String, Welcome), Error> {
    let side = hex::encode(rand::random::<[u8; 5]>());
    let (mut conn, welcome) = Connection::open(&config.relay_url, side.clone()).await?;
    let Inbound::Welcome { welcome } = welcome else {
        unreachable!("Connection::open returns the welcome message");
    };
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
    conn.send(&Outbound::Bind {
        appid: config.appid.clone(),
        side: side.clone(),
        client_version: ("wyrmyon".into(), env!("CARGO_PKG_VERSION").into()),
    })
    .await?;
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
        conn.send(&Outbound::Claim {
            nameplate: nameplate.clone(),
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
        let password: String = code.as_str().nfc().collect();
        let (spake, outbound) = Spake2::<Ed25519Group>::start_symmetric(
            &Password::new(password.as_bytes()),
            &Identity::new(config.appid.as_bytes()),
        );
        let pake = json!({ "pake_v1": hex::encode(outbound) }).to_string();
        conn.send(&Outbound::Add {
            phase: "pake".into(),
            body: hex::encode(pake),
        })
        .await?;
        Ok(Self {
            conn,
            side,
            code,
            nameplate: Some(nameplate),
            mailbox,
            spake,
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

        let mut wormhole = Wormhole {
            conn: self.conn,
            side: self.side,
            their_side,
            mailbox: self.mailbox,
            key,
            their_app_versions: Value::Null,
            welcome: self.welcome,
            next_tx: 0,
            next_rx: 0,
            received: BTreeMap::new(),
        };
        let their_versions = loop {
            let msg = wormhole.conn.next_message().await?;
            if msg.side != wormhole.their_side {
                continue;
            }
            if msg.phase == "version" {
                break msg.body;
            }
            wormhole.stash(&msg.phase, &msg.body)?;
        };
        let Some(their_versions) = wormhole
            .key
            .derive_phase(&wormhole.their_side, "version")
            .decrypt(&their_versions)
        else {
            wormhole.close(Mood::Scary).await;
            return Err(Error::WrongCode);
        };
        let their_versions: Value = serde_json::from_slice(&their_versions)
            .map_err(|_| Error::Protocol("unreadable version message".into()))?;
        wormhole.their_app_versions = their_versions
            .get("app_versions")
            .cloned()
            .unwrap_or(Value::Object(Map::new()));
        Ok(wormhole)
    }

    pub async fn abandon(mut self) {
        if let Some(nameplate) = self.nameplate.take() {
            let _ = self.conn.send(&Outbound::Release { nameplate }).await;
        }
        close_mailbox(self.conn, &self.mailbox, Mood::Lonely).await;
    }
}

impl Wormhole {
    #[must_use]
    pub fn key(&self) -> &Key {
        &self.key
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
    pub fn side(&self) -> &str {
        &self.side
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
            if let Some(plaintext) = self.received.remove(&self.next_rx) {
                self.next_rx += 1;
                return Ok(plaintext);
            }
            let msg = self.conn.next_message().await?;
            if msg.side == self.their_side {
                self.stash(&msg.phase, &msg.body)?;
            }
        }
    }

    pub async fn receive_json(&mut self) -> Result<Value, Error> {
        let plaintext = self.receive().await?;
        serde_json::from_slice(&plaintext)
            .map_err(|_| Error::Protocol("peer sent a message that is not JSON".into()))
    }

    fn stash(&mut self, phase: &str, body: &[u8]) -> Result<(), Error> {
        let Ok(index) = phase.parse::<u64>() else {
            return Ok(());
        };
        if index < self.next_rx || self.received.contains_key(&index) {
            return Ok(());
        }
        let Some(plaintext) = self.key.derive_phase(&self.their_side, phase).decrypt(body) else {
            return Err(Error::Protocol(
                "a message from the peer failed to decrypt".into(),
            ));
        };
        self.received.insert(index, plaintext);
        Ok(())
    }

    pub async fn close(self, mood: Mood) {
        close_mailbox(self.conn, &self.mailbox, mood).await;
    }
}

async fn close_mailbox(mut conn: Connection, mailbox: &str, mood: Mood) {
    let closing = async {
        conn.send(&Outbound::Close {
            mailbox: mailbox.to_owned(),
            mood,
        })
        .await?;
        conn.expect(|msg| matches!(msg, Inbound::Closed).then_some(()))
            .await
    };
    let _ = tokio::time::timeout(CLOSE_TIMEOUT, closing).await;
    conn.shutdown().await;
}
