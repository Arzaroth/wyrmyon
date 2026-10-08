use anyhow::{Context, bail};
use indicatif::{ProgressBar, ProgressStyle};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use wyrmyon_transport_classic::RecordPipe;
use wyrmyon_transport_iroh::IrohPipe;

const CHUNK: usize = 1 << 16;

pub enum Pipe {
    Classic(RecordPipe),
    Iroh(IrohPipe),
}

impl Pipe {
    pub fn describe(&self) -> String {
        match self {
            Self::Classic(pipe) => pipe.describe().to_owned(),
            Self::Iroh(pipe) => pipe.describe().to_owned(),
        }
    }

    async fn send_chunk(&mut self, data: &[u8]) -> anyhow::Result<()> {
        match self {
            Self::Classic(pipe) => pipe.send_record(data).await?,
            Self::Iroh(pipe) => pipe.send_chunk(data).await?,
        }
        Ok(())
    }

    async fn receive_chunk(&mut self) -> anyhow::Result<Vec<u8>> {
        Ok(match self {
            Self::Classic(pipe) => pipe.receive_record().await?,
            Self::Iroh(pipe) => pipe.receive_chunk(CHUNK).await?,
        })
    }

    async fn flush(&mut self) -> anyhow::Result<()> {
        if let Self::Classic(pipe) = self {
            pipe.flush().await?;
        }
        Ok(())
    }

    async fn send_last(&mut self, data: &[u8]) -> anyhow::Result<()> {
        match self {
            Self::Classic(pipe) => {
                pipe.send_record(data).await?;
                pipe.flush().await?;
            }
            Self::Iroh(pipe) => pipe.send_last(data).await?,
        }
        Ok(())
    }

    async fn receive_last(&mut self) -> anyhow::Result<Vec<u8>> {
        Ok(match self {
            Self::Classic(pipe) => pipe.receive_record().await?,
            Self::Iroh(pipe) => pipe.receive_last().await?,
        })
    }

    pub async fn abort(self) {
        match self {
            Self::Classic(pipe) => pipe.shutdown().await,
            Self::Iroh(pipe) => pipe.abort().await,
        }
    }

    pub async fn shutdown(self) {
        match self {
            Self::Classic(pipe) => pipe.shutdown().await,
            Self::Iroh(pipe) => pipe.finish().await,
        }
    }
}

pub fn progress(size: u64, hidden: bool) -> ProgressBar {
    if hidden {
        return ProgressBar::hidden();
    }
    let bar = ProgressBar::new(size);
    bar.set_style(
        ProgressStyle::with_template(
            "{bar:30} {bytes}/{total_bytes} {binary_bytes_per_sec} eta {eta}",
        )
        .expect("the progress template is valid"),
    );
    bar
}

pub async fn send_stream(
    pipe: &mut Pipe,
    mut source: impl AsyncRead + Unpin,
    size: u64,
    bar: &ProgressBar,
) -> anyhow::Result<[u8; 32]> {
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; CHUNK];
    let mut sent = 0u64;
    loop {
        let n = source.read(&mut buf).await.context("reading the data")?;
        if n == 0 {
            break;
        }
        sent += n as u64;
        if sent > size {
            bail!("the data grew while it was being sent");
        }
        hasher.update(&buf[..n]);
        pipe.send_chunk(&buf[..n]).await?;
        bar.inc(n as u64);
    }
    if sent != size {
        bail!("the data shrank while it was being sent");
    }
    pipe.flush().await?;
    bar.finish_and_clear();
    Ok(hasher.finalize().into())
}

pub async fn receive_stream(
    pipe: &mut Pipe,
    mut sink: impl AsyncWrite + Unpin,
    size: u64,
    bar: &ProgressBar,
) -> anyhow::Result<[u8; 32]> {
    let mut hasher = Sha256::new();
    let mut received = 0u64;
    while received < size {
        let chunk = pipe.receive_chunk().await?;
        received += chunk.len() as u64;
        if received > size {
            bail!("the sender sent more than the {size} bytes it offered");
        }
        hasher.update(&chunk);
        sink.write_all(&chunk).await.context("writing the data")?;
        bar.inc(chunk.len() as u64);
    }
    sink.flush().await.context("writing the data")?;
    bar.finish_and_clear();
    Ok(hasher.finalize().into())
}

pub async fn await_ack(pipe: &mut Pipe, digest: &[u8; 32]) -> anyhow::Result<()> {
    let raw = pipe.receive_last().await?;
    let ack: Value =
        serde_json::from_slice(&raw).context("the receiver's confirmation is not JSON")?;
    if ack["ack"] != "ok" {
        bail!("transfer failed, the receiver says: {ack}");
    }
    if let Some(theirs) = ack["sha256"].as_str()
        && theirs != hex::encode(digest)
    {
        bail!("transfer failed: the receiver got different data");
    }
    Ok(())
}

pub async fn send_ack(pipe: &mut Pipe, digest: &[u8; 32]) -> anyhow::Result<()> {
    let ack = json!({"ack": "ok", "sha256": hex::encode(digest)}).to_string();
    pipe.send_last(ack.as_bytes()).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use wyrmyon_transport_classic::{Role, Transit};
    use wyrmyon_wormhole::Key;

    async fn pair() -> (Pipe, Pipe) {
        let sender = Transit::new(Role::Sender, Key::from_bytes([2; 32])).await;
        let receiver = Transit::new(Role::Receiver, Key::from_bytes([2; 32])).await;
        let (sender_info, receiver_info) = (sender.info(), receiver.info());
        let (a, b) = tokio::join!(
            sender.connect(&receiver_info),
            receiver.connect(&sender_info)
        );
        (Pipe::Classic(a.unwrap()), Pipe::Classic(b.unwrap()))
    }

    #[tokio::test]
    async fn a_source_that_does_not_match_its_offer_fails_the_send() {
        let (mut pipe, _other) = pair().await;
        let bar = progress(0, true);
        let grew = send_stream(&mut pipe, &b"longer than offered"[..], 4, &bar).await;
        assert!(grew.unwrap_err().to_string().contains("grew"));
        let (mut pipe, _other) = pair().await;
        let shrank = send_stream(&mut pipe, &b"ab"[..], 4, &bar).await;
        assert!(shrank.unwrap_err().to_string().contains("shrank"));
    }
}
