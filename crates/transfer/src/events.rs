use std::pin::Pin;

use crate::Pipe;

pub type Settled<'a> = Pin<Box<dyn Future<Output = ()> + Send + 'a>>;

pub trait Events: Send + Sync {
    fn hashing(&self, _total: u64) {}
    fn hashed(&self, _bytes: u64) {}
    fn hashing_done(&self) {}
    fn connected(&self, _pipe: &Pipe, _size: u64) {}
    fn moved(&self, _bytes: u64) {}
    fn settle(&self) -> Settled<'_> {
        Box::pin(async {})
    }
    fn awaiting_ack(&self) {}
    fn stopped(&self) {}
    fn cancelled(&self) -> Settled<'_> {
        Box::pin(std::future::pending())
    }
}

#[derive(Debug)]
pub struct Cancelled;

impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the transfer was cancelled")
    }
}

impl std::error::Error for Cancelled {}

pub struct Quiet;

impl Events for Quiet {}
