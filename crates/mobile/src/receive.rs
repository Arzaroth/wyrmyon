use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use tokio::io::{AsyncWrite, AsyncWriteExt};
use wyrmyon_transfer::protocol::{self, Answer, AppMessage, Offer};
use wyrmyon_transfer::{
    Connection, Events, Opening, Sink, await_offer, receive_into, receive_tree, refuse,
};
use wyrmyon_wormhole::{Mood, Wormhole};

use crate::Cancel;
use crate::api::types::{OfferInfo, OfferKind, Settings};
use crate::report::{Report, Reporter};
use crate::session;

pub struct Incoming {
    info: OfferInfo,
    waiting: Option<Waiting>,
    report: Report,
    cancel: Cancel,
}

struct Waiting {
    wormhole: Wormhole,
    connection: Connection,
    offer: Offer,
}

pub async fn receive(
    settings: &Settings,
    code: Option<String>,
    report: Report,
    cancel: &Cancel,
) -> anyhow::Result<Incoming> {
    let options = settings.options()?;
    let mut wormhole = session::until(cancel, session::pair(&options, code, &report)).await?;
    let awaited = Box::pin(session::until(cancel, await_offer(&mut wormhole, &options))).await;
    let (offer, connection) = match awaited {
        Ok(awaited) => awaited,
        Err(e) => return session::finish(wormhole, Err(e), &report).await,
    };
    let info = match &offer {
        Offer::File(file) => Some(OfferInfo {
            kind: OfferKind::File,
            name: file_name(&file.filename),
            size: file.filesize,
            count: 1,
            text: String::new(),
        }),
        Offer::Directory(dir) => Some(tree_info(
            &dir.dirname,
            dir.bundle,
            dir.numbytes,
            dir.numfiles,
        )),
        Offer::Collection(tree) => Some(tree_info(
            &tree.name,
            tree.bundle,
            tree.numbytes,
            tree.numfiles,
        )),
        _ => None,
    };
    match (info, connection) {
        (Some(info), Some(connection)) => Ok(Incoming {
            info,
            waiting: Some(Waiting {
                wormhole,
                connection,
                offer,
            }),
            report,
            cancel: cancel.clone(),
        }),
        (_, connection) => {
            if let Some(connection) = connection {
                connection.close().await;
            }
            let answered = match offer {
                Offer::Message(text) => {
                    let ack = AppMessage::Answer(Answer::MessageAck("ok".into()));
                    protocol::send(&mut wormhole, &ack).await.map(|()| text)
                }
                Offer::File(_) | Offer::Directory(_) | Offer::Collection(_) => {
                    refuse(&mut wormhole, "the sender did not offer a connection").await
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
                    count: 0,
                    text,
                },
                waiting: None,
                report,
                cancel: cancel.clone(),
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
            offer: Offer::File(file),
        }) = self.waiting
        else {
            return self.mismatched().await;
        };
        let size = file.filesize;
        let events: Arc<dyn Events> =
            Arc::new(Reporter::new(Arc::clone(&self.report), self.cancel.clone()));
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

    pub async fn accept_tree(
        self,
        open: &mut (dyn FnMut(&str, u32) -> Opening + Send),
        cache: PathBuf,
    ) -> anyhow::Result<()> {
        let Some(Waiting {
            mut wormhole,
            connection,
            offer: offer @ (Offer::Directory(_) | Offer::Collection(_)),
        }) = self.waiting
        else {
            return self.mismatched().await;
        };
        let events: Arc<dyn Events> =
            Arc::new(Reporter::new(Arc::clone(&self.report), self.cancel.clone()));
        let result = async {
            let received =
                receive_tree(&mut wormhole, connection, &offer, &cache, open, &events).await?;
            received.complete(Ok(())).await
        }
        .await;
        session::finish(wormhole, result, &self.report).await
    }

    async fn mismatched(self) -> anyhow::Result<()> {
        if self.waiting.is_none() {
            return Ok(());
        }
        let accepted = self.info.kind;
        self.refuse().await?;
        anyhow::bail!("this offer ({accepted:?}) is accepted another way")
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

fn tree_info(name: &str, bundle: bool, size: u64, count: u64) -> OfferInfo {
    OfferInfo {
        kind: if bundle {
            OfferKind::Files
        } else {
            OfferKind::Folder
        },
        name: file_name(name),
        size,
        count,
        text: String::new(),
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
