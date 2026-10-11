use anyhow::{Context, bail};
use indicatif::{ProgressBar, ProgressStyle};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::watch;
use wyrmyon_transport_classic::RecordPipe;
use wyrmyon_transport_iroh::{IrohPipe, Route, Traffic};

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

    pub fn follow_route(&self, bar: &ProgressBar) {
        if let Self::Iroh(pipe) = self {
            follow_route(pipe, bar);
        }
    }

    pub fn went(&self) -> Option<String> {
        match self {
            Self::Classic(_) => None,
            Self::Iroh(pipe) => went(pipe.traffic()),
        }
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

pub fn follow_route(pipe: &IrohPipe, bar: &ProgressBar) {
    announce_routes(pipe.route(), pipe.first_route(), bar.clone());
}

fn announce_routes(
    mut route: watch::Receiver<Option<Route>>,
    mut last: Option<Route>,
    bar: ProgressBar,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        while route.changed().await.is_ok() {
            let now = *route.borrow_and_update();
            if let Some(said) = route_change(last, now) {
                bar.suspend(|| eprintln!("{said}"));
            }
            last = now.or(last);
        }
    })
}

fn route_change(before: Option<Route>, now: Option<Route>) -> Option<&'static str> {
    match (before?, now?) {
        (Route::Relay, Route::Direct) => Some("Now direct: hole-punching opened a path"),
        (Route::Direct, Route::Relay) => Some("Back through the relay: the direct path closed"),
        _ => None,
    }
}

fn went(traffic: Traffic) -> Option<String> {
    let Traffic { direct, relay } = traffic;
    match (direct, relay) {
        (0, 0) => None,
        (_, 0) => Some("Went direct".to_owned()),
        (0, _) => Some("Went through the relay".to_owned()),
        _ => {
            let total = u128::from(direct) + u128::from(relay);
            let share = (u128::from(direct) * 100 + total / 2) / total;
            Some(format!(
                "Went {}% direct, the rest through the relay",
                share.clamp(1, 99)
            ))
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
    pipe: &mut RecordPipe,
    source: &mut (dyn AsyncRead + Unpin + Send),
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
    sink: &mut (dyn AsyncWrite + Unpin + Send),
    size: u64,
    bar: &ProgressBar,
) -> anyhow::Result<[u8; 32]> {
    let mut hasher = Sha256::new();
    let mut received = 0u64;
    while received < size {
        let chunk = pipe.receive_record().await?;
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

pub async fn await_ack(pipe: &mut Pipe, digest: Option<&[u8; 32]>) -> anyhow::Result<()> {
    let raw = pipe.receive_last().await?;
    let ack: Value =
        serde_json::from_slice(&raw).context("the receiver's confirmation is not JSON")?;
    if ack["ack"] != "ok" {
        bail!("transfer failed, the receiver says: {ack}");
    }
    if let (Some(theirs), Some(digest)) = (ack["sha256"].as_str(), digest)
        && theirs != hex::encode(digest)
    {
        bail!("transfer failed: the receiver got different data");
    }
    Ok(())
}

pub async fn send_ack(pipe: &mut Pipe, digest: Option<&[u8; 32]>) -> anyhow::Result<()> {
    let ack = match digest {
        Some(digest) => json!({"ack": "ok", "sha256": hex::encode(digest)}),
        None => json!({"ack": "ok"}),
    }
    .to_string();
    pipe.send_last(ack.as_bytes()).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use wyrmyon_transport_classic::{Role, Transit};
    use wyrmyon_wormhole::Key;

    async fn pair() -> (RecordPipe, RecordPipe) {
        let sender = Transit::new(Role::Sender, Key::from_bytes([2; 32])).await;
        let receiver = Transit::new(Role::Receiver, Key::from_bytes([2; 32])).await;
        let (sender_info, receiver_info) = (sender.info(), receiver.info());
        let (a, b) = tokio::join!(
            sender.connect(&receiver_info),
            receiver.connect(&sender_info)
        );
        (a.unwrap(), b.unwrap())
    }

    #[test]
    fn the_route_summary_says_where_the_bytes_went() {
        let went = |direct, relay| went(Traffic { direct, relay });
        assert_eq!(went(0, 0), None);
        assert_eq!(went(5, 0).as_deref(), Some("Went direct"));
        assert_eq!(went(0, 5).as_deref(), Some("Went through the relay"));
        assert_eq!(
            went(97, 3).as_deref(),
            Some("Went 97% direct, the rest through the relay")
        );
        assert_eq!(
            went(u64::MAX, u64::MAX).as_deref(),
            Some("Went 50% direct, the rest through the relay")
        );
        let change = |before, now| route_change(before, now).unwrap_or_default();
        assert!(change(Some(Route::Relay), Some(Route::Direct)).starts_with("Now direct"));
        assert!(change(Some(Route::Direct), Some(Route::Relay)).starts_with("Back through"));
        for (before, now) in [
            (None, Some(Route::Direct)),
            (None, Some(Route::Relay)),
            (Some(Route::Relay), None),
            (Some(Route::Direct), Some(Route::Direct)),
        ] {
            assert_eq!(route_change(before, now), None, "{before:?} {now:?}");
        }
        assert_eq!(
            went(1, 1000).as_deref(),
            Some("Went 1% direct, the rest through the relay")
        );
        assert_eq!(
            went(999_999, 1).as_deref(),
            Some("Went 99% direct, the rest through the relay")
        );
        assert_eq!(
            went(2, 1).as_deref(),
            Some("Went 67% direct, the rest through the relay")
        );
    }

    #[tokio::test]
    async fn the_announcer_follows_the_route_until_the_pipe_goes() {
        let (tx, rx) = watch::channel(Some(Route::Relay));
        let announcer = announce_routes(rx, Some(Route::Relay), progress(0, true));
        for now in [Some(Route::Direct), None, Some(Route::Relay)] {
            tx.send(now).unwrap();
            tokio::task::yield_now().await;
        }
        drop(tx);
        announcer.await.unwrap();
    }

    #[tokio::test]
    async fn a_source_that_does_not_match_its_offer_fails_the_send() {
        let (mut pipe, _other) = pair().await;
        let bar = progress(0, true);
        let grew = send_stream(&mut pipe, &mut &b"longer than offered"[..], 4, &bar).await;
        assert!(grew.unwrap_err().to_string().contains("grew"));
        let (mut pipe, _other) = pair().await;
        let shrank = send_stream(&mut pipe, &mut &b"ab"[..], 4, &bar).await;
        assert!(shrank.unwrap_err().to_string().contains("shrank"));
    }
}
