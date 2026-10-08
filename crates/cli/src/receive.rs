use std::io::Write;

use anyhow::{Context, bail};
use clap::Args;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, BufReader};
use wyrmyon_wormhole::{Code, Mood, Wormhole};

use crate::{Global, show_welcome};

#[derive(Args)]
pub struct ReceiveArgs {
    /// The code the sender gave you; asked for when left out
    code: Option<String>,
}

pub async fn run(global: &Global, args: ReceiveArgs) -> anyhow::Result<()> {
    let code: Code = match args.code {
        Some(code) => code.parse()?,
        None => prompt_code().await?,
    };
    let mut wormhole = wyrmyon_wormhole::connect(&global.config(), code).await?;
    show_welcome(wormhole.welcome());

    loop {
        let msg = wormhole.receive_json().await?;
        if let Some(error) = msg.get("error") {
            wormhole.close(Mood::Errory).await;
            bail!("the sender reported an error: {error}");
        }
        if let Some(offer) = msg.get("offer") {
            return handle_offer(wormhole, offer).await;
        }
    }
}

async fn handle_offer(mut wormhole: Wormhole, offer: &Value) -> anyhow::Result<()> {
    if let Some(text) = offer.get("message").and_then(Value::as_str) {
        let mut stdout = std::io::stdout().lock();
        writeln!(stdout, "{text}").context("writing to stdout")?;
        drop(stdout);
        wormhole
            .send_json(&json!({ "answer": { "message_ack": "ok" } }))
            .await?;
        wormhole.close(Mood::Happy).await;
        return Ok(());
    }
    wormhole
        .send_json(&json!({ "error": "wyrmyon cannot receive this kind of offer yet" }))
        .await?;
    wormhole.close(Mood::Errory).await;
    bail!("the sender offered something this version cannot receive")
}

async fn prompt_code() -> anyhow::Result<Code> {
    eprint!("Enter receive wormhole code: ");
    let mut line = String::new();
    BufReader::new(tokio::io::stdin())
        .read_line(&mut line)
        .await
        .context("reading the code")?;
    Ok(line.trim().parse()?)
}
