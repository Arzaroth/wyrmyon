use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use tokio::io::{AsyncWrite, AsyncWriteExt};
use wyrmyon_transfer::protocol::{self, Answer, AppMessage, Offer};
use wyrmyon_transfer::{Connection, Events, Sink, await_offer, receive_into, refuse};
use wyrmyon_wormhole::{Mood, Wormhole};

use crate::api::types::{OfferInfo, OfferKind, Settings};
use crate::report::{Report, Reporter};
use crate::session;

pub struct Incoming {
    info: OfferInfo,
    waiting: Option<Waiting>,
    report: Report,
}

struct Waiting {
    wormhole: Wormhole,
    connection: Connection,
    size: u64,
}

pub async fn receive(
    settings: &Settings,
    code: Option<String>,
    report: Report,
) -> anyhow::Result<Incoming> {
    let options = settings.options()?;
    let mut wormhole = session::pair(&options, code, &report).await?;
    let (offer, connection) = match await_offer(&mut wormhole, &options).await {
        Ok(awaited) => awaited,
        Err(e) => return session::finish(wormhole, Err(e), &report).await,
    };
    match (offer, connection) {
        (Offer::File(file), Some(connection)) => Ok(Incoming {
            info: OfferInfo {
                kind: OfferKind::File,
                name: file_name(&file.filename),
                size: file.filesize,
                text: String::new(),
            },
            waiting: Some(Waiting {
                wormhole,
                connection,
                size: file.filesize,
            }),
            report,
        }),
        (offer, connection) => {
            if let Some(connection) = connection {
                connection.close().await;
            }
            let answered = match offer {
                Offer::Message(text) => {
                    let ack = AppMessage::Answer(Answer::MessageAck("ok".into()));
                    protocol::send(&mut wormhole, &ack).await.map(|()| text)
                }
                Offer::File(_) => {
                    refuse(&mut wormhole, "the sender did not offer a connection").await
                }
                Offer::Directory(_) | Offer::Collection(_) => {
                    refuse(
                        &mut wormhole,
                        "the app cannot receive folders or several files yet",
                    )
                    .await
                }
                Offer::Forward(_) | Offer::Other(_) => {
                    refuse(&mut wormhole, "the app cannot receive this kind of offer").await
                }
            };
            let text = session::finish(wormhole, answered, &report).await?;
            Ok(Incoming {
                info: OfferInfo {
                    kind: OfferKind::Text,
                    name: String::new(),
                    size: text.len() as u64,
                    text,
                },
                waiting: None,
                report,
            })
        }
    }
}

impl Incoming {
    pub fn info(&self) -> OfferInfo {
        self.info.clone()
    }

    pub async fn accept(
        self,
        writer: &mut (dyn AsyncWrite + Send + Unpin),
        cache: PathBuf,
    ) -> anyhow::Result<()> {
        let Some(Waiting {
            mut wormhole,
            connection,
            size,
        }) = self.waiting
        else {
            return Ok(());
        };
        let events: Arc<dyn Events> = Arc::new(Reporter::new(Arc::clone(&self.report)));
        let result = async {
            let sink = Sink::Writer(&mut *writer);
            let received =
                receive_into(&mut wormhole, connection, size, Ok(cache), sink, &events).await?;
            let flushed = writer.flush().await.context("writing the data");
            received.complete(flushed).await
        }
        .await;
        session::finish(wormhole, result, &self.report).await
    }

    pub async fn refuse(self) -> anyhow::Result<()> {
        let Some(Waiting {
            mut wormhole,
            connection,
            ..
        }) = self.waiting
        else {
            return Ok(());
        };
        connection.close().await;
        let refused = protocol::send_error(&mut wormhole, "transfer rejected").await;
        wormhole.close(Mood::Happy).await;
        refused
    }
}

fn file_name(offered: &str) -> String {
    let last = offered.rsplit(['/', '\\']).next().unwrap_or_default();
    let name: String = last.chars().filter(|c| !c.is_control()).collect();
    if name.is_empty() || name == "." || name == ".." {
        "received".to_owned()
    } else {
        name
    }
}

#[cfg(test)]
mod tests {
    use super::file_name;

    #[test]
    fn an_offered_name_keeps_only_its_last_printable_part() {
        assert_eq!(file_name("photo.jpg"), "photo.jpg");
        assert_eq!(file_name("../../etc/passwd"), "passwd");
        assert_eq!(file_name("C:\\Users\\me\\notes.txt"), "notes.txt");
        assert_eq!(file_name("bell\u{7}.txt"), "bell.txt");
        for unusable in ["", "dir/", "..", "a/.", "\u{1b}"] {
            assert_eq!(file_name(unusable), "received", "{unusable:?}");
        }
    }
}
