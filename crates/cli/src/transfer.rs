use anyhow::{Context, bail};
use indicatif::{ProgressBar, ProgressStyle};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use wyrmyon_transport_classic::RecordPipe;

const CHUNK: usize = 1 << 16;

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
    pipe: &mut RecordPipe,
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
        pipe.send_record(&buf[..n]).await?;
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
    pipe: &mut RecordPipe,
    mut sink: impl AsyncWrite + Unpin,
    size: u64,
    bar: &ProgressBar,
) -> anyhow::Result<[u8; 32]> {
    let mut hasher = Sha256::new();
    let mut received = 0u64;
    while received < size {
        let record = pipe.receive_record().await?;
        received += record.len() as u64;
        if received > size {
            bail!("the sender sent more than the {size} bytes it offered");
        }
        hasher.update(&record);
        sink.write_all(&record).await.context("writing the data")?;
        bar.inc(record.len() as u64);
    }
    sink.flush().await.context("writing the data")?;
    bar.finish_and_clear();
    Ok(hasher.finalize().into())
}

pub async fn await_ack(pipe: &mut RecordPipe, digest: &[u8; 32]) -> anyhow::Result<()> {
    let ack: Value = serde_json::from_slice(&pipe.receive_record().await?)
        .context("the receiver's confirmation is not JSON")?;
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

pub async fn send_ack(pipe: &mut RecordPipe, digest: &[u8; 32]) -> anyhow::Result<()> {
    let ack = json!({"ack": "ok", "sha256": hex::encode(digest)}).to_string();
    pipe.send_record(ack.as_bytes()).await?;
    pipe.flush().await?;
    Ok(())
}
