use std::net::SocketAddr;

use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::net::TcpStream;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use wyrmyon_wormhole::{Key, NONCE_LEN};

use crate::{Error, Role};

pub const MAX_RECORD: usize = 4 << 20;
const OVERHEAD: usize = NONCE_LEN + 16;

pub struct RecordPipe {
    reader: BufReader<OwnedReadHalf>,
    writer: BufWriter<OwnedWriteHalf>,
    send_key: Key,
    receive_key: Key,
    send_nonce: u128,
    receive_nonce: u128,
    description: String,
}

pub(crate) fn record_keys(transit_key: &Key, role: Role) -> (Key, Key) {
    let sender = transit_key.derive(b"transit_record_sender_key");
    let receiver = transit_key.derive(b"transit_record_receiver_key");
    match role {
        Role::Sender => (sender, receiver),
        Role::Receiver => (receiver, sender),
    }
}

fn nonce_bytes(counter: u128) -> [u8; NONCE_LEN] {
    let mut nonce = [0u8; NONCE_LEN];
    nonce[NONCE_LEN - 16..].copy_from_slice(&counter.to_be_bytes());
    nonce
}

impl RecordPipe {
    pub(crate) fn new(
        stream: TcpStream,
        transit_key: &Key,
        role: Role,
        peer: Option<SocketAddr>,
        relay: bool,
    ) -> Self {
        let (send_key, receive_key) = record_keys(transit_key, role);
        let (reader, writer) = stream.into_split();
        let description = match (peer, relay) {
            (Some(peer), true) => format!("via relay {peer}"),
            (Some(peer), false) => format!("directly to {peer}"),
            (None, _) => "connected".to_owned(),
        };
        Self {
            reader: BufReader::new(reader),
            writer: BufWriter::new(writer),
            send_key,
            receive_key,
            send_nonce: 0,
            receive_nonce: 0,
            description,
        }
    }

    #[must_use]
    pub fn describe(&self) -> &str {
        &self.description
    }

    pub async fn send_record(&mut self, record: &[u8]) -> Result<(), Error> {
        assert!(
            record.len() <= MAX_RECORD,
            "records are at most {MAX_RECORD} bytes"
        );
        let sealed = self
            .send_key
            .encrypt_with_nonce(&nonce_bytes(self.send_nonce), record);
        self.send_nonce += 1;
        let length = u32::try_from(sealed.len()).expect("records fit in u32");
        self.writer.write_all(&length.to_be_bytes()).await?;
        self.writer.write_all(&sealed).await?;
        Ok(())
    }

    pub async fn flush(&mut self) -> Result<(), Error> {
        self.writer.flush().await?;
        Ok(())
    }

    pub async fn receive_record(&mut self) -> Result<Vec<u8>, Error> {
        let length = self.reader.read_u32().await? as usize;
        if !(OVERHEAD..=MAX_RECORD + OVERHEAD).contains(&length) {
            return Err(Error::Record(format!("a record of {length} bytes")));
        }
        let mut sealed = vec![0u8; length];
        self.reader.read_exact(&mut sealed).await?;
        if sealed[..NONCE_LEN] != nonce_bytes(self.receive_nonce) {
            return Err(Error::Record("a record out of order".into()));
        }
        self.receive_nonce += 1;
        self.receive_key
            .decrypt(&sealed)
            .ok_or_else(|| Error::Record("a record failed to decrypt".into()))
    }

    pub async fn shutdown(mut self) {
        let _ = self.writer.shutdown().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nonces_are_big_endian_counters() {
        assert_eq!(nonce_bytes(0), [0; NONCE_LEN]);
        let one = nonce_bytes(258);
        assert_eq!(&one[NONCE_LEN - 2..], &[1, 2]);
        assert!(one[..NONCE_LEN - 2].iter().all(|&b| b == 0));
    }

    #[test]
    fn record_keys_match_the_python_client() {
        let transit = Key::from_bytes(
            hex::decode("9329a646acaba7172c557c7971191ec6a1e287f84abe58c60dce2659c44c2925")
                .unwrap()
                .try_into()
                .unwrap(),
        );
        let (send, _) = record_keys(&transit, Role::Sender);
        assert_eq!(
            hex::encode(send.as_bytes()),
            "4860c8f20d819f7caff59f3e9c6c90c7d70405c6e1247fc6eebb6686768b0d75"
        );
        let (_, receive) = record_keys(&transit, Role::Receiver);
        assert_eq!(receive.as_bytes(), send.as_bytes());
    }
}
