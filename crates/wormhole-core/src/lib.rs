pub mod code;
pub mod crypto;
pub mod server;
mod wordlist_data;

pub use code::{Code, CodeError};
pub use crypto::Key;
pub use server::Mood;

pub const APPID: &str = "lothar.com/wormhole/text-or-file-xfer";
pub const PUBLIC_RELAY: &str = "ws://relay.magic-wormhole.io:4000/v1";
