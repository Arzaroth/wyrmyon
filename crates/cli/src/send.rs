use anyhow::{Context, bail};
use clap::Args;
use serde_json::json;
use tokio::io::AsyncReadExt;
use wyrmyon_wormhole::Mood;

use crate::{Global, show_welcome};

#[derive(Args)]
pub struct SendArgs {
    /// Text to send; `-` reads it from stdin
    #[arg(long)]
    text: String,
    /// Number of words in the generated code
    #[arg(long, default_value_t = 2, value_parser = clap::value_parser!(u8).range(1..=8))]
    code_length: u8,
}

pub async fn run(global: &Global, args: SendArgs) -> anyhow::Result<()> {
    let text = if args.text == "-" {
        let mut text = String::new();
        tokio::io::stdin()
            .read_to_string(&mut text)
            .await
            .context("reading stdin")?;
        text
    } else {
        args.text
    };

    let pending = wyrmyon_wormhole::create(&global.config(), usize::from(args.code_length)).await?;
    show_welcome(pending.welcome());
    eprintln!("Wormhole code is: {}", pending.code());
    eprintln!("On the other computer, please run:\n\n  wyrm receive\n\n(or: wormhole receive)\n");

    let mut wormhole = pending.pair().await?;
    wormhole
        .send_json(&json!({ "offer": { "message": text } }))
        .await?;
    loop {
        let msg = wormhole.receive_json().await?;
        if let Some(error) = msg.get("error") {
            wormhole.close(Mood::Errory).await;
            bail!("the receiver refused: {error}");
        }
        if let Some(answer) = msg.get("answer") {
            if answer.get("message_ack").and_then(|a| a.as_str()) == Some("ok") {
                eprintln!("text message sent");
                wormhole.close(Mood::Happy).await;
                return Ok(());
            }
            wormhole.close(Mood::Errory).await;
            bail!("unexpected answer from the receiver: {answer}");
        }
    }
}
