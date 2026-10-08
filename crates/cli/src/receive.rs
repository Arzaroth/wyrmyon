use std::io::Write;

use anyhow::{Context, bail};
use clap::Args;
use tokio::io::{AsyncBufReadExt, BufReader};
use wyrmyon_wormhole::{Code, Wormhole};

use crate::protocol::{self, Answer, AppMessage, Offer};
use crate::{Global, mood_for, show_welcome};

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
    let pending = wyrmyon_wormhole::join(&global.config(), code).await?;
    show_welcome(pending.welcome());
    let mut wormhole = pending.pair().await?;
    let result = receive_offer(&mut wormhole).await;
    wormhole.close(mood_for(&result)).await;
    result
}

async fn receive_offer(wormhole: &mut Wormhole) -> anyhow::Result<()> {
    let offer = loop {
        if let AppMessage::Offer(offer) = protocol::next(wormhole, "sender").await? {
            break offer;
        }
    };
    let Offer::Message(text) = offer else {
        protocol::send_error(wormhole, "wyrmyon cannot receive this kind of offer yet").await?;
        bail!("the sender offered something this version cannot receive");
    };
    if let Err(e) = writeln!(std::io::stdout().lock(), "{text}") {
        protocol::send_error(wormhole, "the receiver could not write the message out").await?;
        return Err(e).context("writing to stdout");
    }
    protocol::send(
        wormhole,
        &AppMessage::Answer(Answer::MessageAck("ok".into())),
    )
    .await
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
