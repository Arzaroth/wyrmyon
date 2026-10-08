pub mod code;
mod connection;
mod crypto;
mod server;
mod wordlist_data;
mod wormhole;

pub use code::{Code, CodeError};
pub use crypto::{KEY_LEN, Key, NONCE_LEN};
pub use server::Mood;
pub use wormhole::{Config, Pending, Welcome, Wormhole, create, join};

pub const APPID: &str = "lothar.com/wormhole/text-or-file-xfer";
pub const PUBLIC_RELAY: &str = "ws://relay.magic-wormhole.io:4000/v1";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("the mailbox server refused us: {0}")]
    Welcome(String),
    #[error("the mailbox server reported an error: {0}")]
    Server(String),
    #[error("the mailbox server closed the connection")]
    ServerClosed,
    #[error("{0}")]
    Connection(String),
    #[error("the key exchange failed: the code was mistyped, or someone tried to guess it")]
    WrongCode,
    #[error("a message from the peer failed to decrypt: someone may be tampering with the mailbox")]
    Tampered,
    #[error("protocol error: {0}")]
    Protocol(String),
}

impl Error {
    #[must_use]
    pub fn mood(&self) -> Mood {
        match self {
            Self::WrongCode | Self::Tampered | Self::Protocol(_) => Mood::Scary,
            Self::Welcome(_) | Self::Server(_) | Self::ServerClosed | Self::Connection(_) => {
                Mood::Errory
            }
        }
    }
}
