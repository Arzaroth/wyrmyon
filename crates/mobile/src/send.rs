use std::sync::Arc;

use wyrmyon_transfer::protocol::{FileOffer, Offer};
use wyrmyon_transfer::{Events, Source, Staged, offer_text, send_data};

use crate::api::types::Settings;
use crate::report::{Report, Reporter};
use crate::session;

pub enum Outgoing {
    Text(String),
    File {
        name: String,
        source: Arc<dyn Source>,
    },
}

pub async fn send(
    settings: &Settings,
    code: Option<String>,
    outgoing: Outgoing,
    report: Report,
) -> anyhow::Result<()> {
    let options = settings.options()?;
    let mut wormhole = session::pair(&options, code, &report).await?;
    let result = match outgoing {
        Outgoing::Text(text) => offer_text(&mut wormhole, text).await,
        Outgoing::File { name, source } => {
            let events: Arc<dyn Events> = Arc::new(Reporter::new(Arc::clone(&report)));
            let mut staged = Staged::default();
            let offer = Offer::File(FileOffer {
                filename: name,
                filesize: source.size(),
            });
            let sent = send_data(
                &mut wormhole,
                &options,
                &mut staged,
                offer,
                &source,
                &events,
            )
            .await;
            staged.close().await;
            sent.map(drop)
        }
    };
    session::finish(wormhole, result, &report).await
}
