use std::collections::VecDeque;

use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message as Frame;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use crate::Error;
use crate::server::{Inbound, Outbound};

pub(crate) struct MailboxMessage {
    pub side: String,
    pub phase: String,
    pub body: Vec<u8>,
}

pub(crate) struct Connection {
    ws: Box<WebSocketStream<MaybeTlsStream<TcpStream>>>,
    side: String,
    buffered: VecDeque<MailboxMessage>,
}

impl Connection {
    pub async fn open(url: &str, side: String) -> Result<(Self, Inbound), Error> {
        let (ws, _) = tokio_tungstenite::connect_async(url)
            .await
            .map_err(|e| Error::Connection(format!("mailbox server {url}: {e}")))?;
        let mut conn = Self {
            ws: Box::new(ws),
            side,
            buffered: VecDeque::new(),
        };
        let welcome = conn
            .expect(|msg| matches!(msg, Inbound::Welcome { .. }).then_some(msg))
            .await?;
        Ok((conn, welcome))
    }

    pub async fn send(&mut self, msg: &Outbound) -> Result<(), Error> {
        self.ws
            .send(Frame::text(msg.to_frame()))
            .await
            .map_err(|e| Error::Connection(format!("mailbox server: {e}")))
    }

    async fn recv(&mut self) -> Result<Inbound, Error> {
        loop {
            let frame = self
                .ws
                .next()
                .await
                .ok_or(Error::ServerClosed)?
                .map_err(|e| Error::Connection(format!("mailbox server: {e}")))?;
            let text = match frame {
                Frame::Text(text) => text,
                Frame::Close(_) => return Err(Error::ServerClosed),
                _ => continue,
            };
            let msg: Inbound = serde_json::from_str(&text)
                .map_err(|e| Error::Protocol(format!("unreadable server message: {e}")))?;
            match msg {
                Inbound::Ack | Inbound::Unknown => {}
                Inbound::Error { error, .. } => return Err(Error::Server(error)),
                msg => return Ok(msg),
            }
        }
    }

    pub async fn expect<T>(
        &mut self,
        mut pick: impl FnMut(Inbound) -> Option<T>,
    ) -> Result<T, Error> {
        loop {
            match self.recv().await? {
                Inbound::Message { side, phase, body } => self.buffer(side, phase, &body)?,
                msg => {
                    if let Some(found) = pick(msg) {
                        return Ok(found);
                    }
                }
            }
        }
    }

    pub async fn next_message(&mut self) -> Result<MailboxMessage, Error> {
        loop {
            if let Some(msg) = self.buffered.pop_front() {
                return Ok(msg);
            }
            if let Inbound::Message { side, phase, body } = self.recv().await? {
                self.buffer(side, phase, &body)?;
            }
        }
    }

    fn buffer(&mut self, side: String, phase: String, body: &str) -> Result<(), Error> {
        if side == self.side {
            return Ok(());
        }
        let body =
            hex::decode(body).map_err(|_| Error::Protocol("mailbox body is not hex".into()))?;
        self.buffered
            .push_back(MailboxMessage { side, phase, body });
        Ok(())
    }

    pub async fn shutdown(mut self) {
        let _ = self.ws.close(None).await;
    }
}
