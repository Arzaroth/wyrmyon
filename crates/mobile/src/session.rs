use anyhow::Context;
use wyrmyon_transfer::{Cancelled, Options, protocol};
use wyrmyon_wormhole::{Code, Mood, Wormhole};

use crate::Cancel;
use crate::api::types::{CODE_WORDS, Stage};
use crate::report::{Report, tell};

pub(crate) async fn until<T>(
    cancel: &Cancel,
    work: impl Future<Output = anyhow::Result<T>>,
) -> anyhow::Result<T> {
    tokio::select! {
        done = work => done,
        () = cancel.cancelled() => Err(Cancelled.into()),
    }
}

pub(crate) async fn pair(
    options: &Options,
    code: Option<String>,
    report: &Report,
) -> anyhow::Result<Wormhole> {
    let config = options.config();
    let pending = if let Some(code) = code {
        let code: Code = code
            .trim()
            .parse()
            .ok()
            .context("this is not a wormhole code")?;
        wyrmyon_wormhole::join(&config, code).await?
    } else {
        let pending = wyrmyon_wormhole::create(&config, CODE_WORDS).await?;
        tell(report, Stage::Code, pending.code().to_string());
        pending
    };
    if let Some(motd) = &pending.welcome().motd {
        tell(report, Stage::Notice, motd.clone());
    }
    let wormhole = pending.pair().await?;
    tell(report, Stage::Paired, String::new());
    Ok(wormhole)
}

pub(crate) async fn finish<T>(
    mut wormhole: Wormhole,
    result: anyhow::Result<T>,
    report: &Report,
) -> anyhow::Result<T> {
    if result.as_ref().is_err_and(anyhow::Error::is::<Cancelled>) {
        let _ = protocol::send_error(&mut wormhole, "transfer cancelled").await;
    }
    let mood = match &result {
        Ok(_) => Mood::Happy,
        Err(e) => e
            .downcast_ref::<wyrmyon_wormhole::Error>()
            .map_or(Mood::Happy, wyrmyon_wormhole::Error::mood),
    };
    wormhole.close(mood).await;
    if result.is_ok() {
        tell(report, Stage::Done, String::new());
    }
    result
}
