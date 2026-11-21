use anyhow::Context;
use wyrmyon_transfer::Options;
use wyrmyon_wormhole::{Code, Mood, Wormhole};

use crate::api::types::Stage;
use crate::report::{Report, tell};

const CODE_WORDS: usize = 2;

pub(crate) async fn pair(
    options: &Options,
    code: Option<String>,
    report: &Report,
) -> anyhow::Result<Wormhole> {
    let config = options.config_offering(&[]);
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
    wormhole: Wormhole,
    result: anyhow::Result<T>,
    report: &Report,
) -> anyhow::Result<T> {
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
