use anyhow::bail;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use wyrmyon_transport_classic::TransitInfo;
use wyrmyon_wormhole::Wormhole;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AppMessage {
    Offer(Offer),
    Answer(Answer),
    Transit(TransitInfo),
    Error(Value),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Offer {
    Message(String),
    File(FileOffer),
    Directory(DirectoryOffer),
    #[serde(untagged)]
    Other(Value),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileOffer {
    pub filename: String,
    pub filesize: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirectoryOffer {
    pub mode: String,
    pub dirname: String,
    pub zipsize: u64,
    pub numbytes: u64,
    pub numfiles: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Answer {
    MessageAck(String),
    FileAck(String),
    #[serde(untagged)]
    Other(Value),
}

pub async fn send(wormhole: &mut Wormhole, msg: &AppMessage) -> anyhow::Result<()> {
    wormhole
        .send_json(&serde_json::to_value(msg).expect("app messages serialise"))
        .await?;
    Ok(())
}

pub async fn send_error(wormhole: &mut Wormhole, error: &str) -> anyhow::Result<()> {
    send(
        wormhole,
        &AppMessage::Error(Value::String(error.to_owned())),
    )
    .await
}

pub async fn next(wormhole: &mut Wormhole, peer: &str) -> anyhow::Result<AppMessage> {
    loop {
        let value = wormhole.receive_json().await?;
        match serde_json::from_value::<AppMessage>(value) {
            Ok(AppMessage::Error(error)) => bail!("the {peer} reported an error: {error}"),
            Ok(msg) => return Ok(msg),
            Err(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn messages_match_the_python_client_shapes() {
        let offer = AppMessage::Offer(Offer::Message("hi".into()));
        assert_eq!(
            serde_json::to_value(&offer).unwrap(),
            json!({"offer": {"message": "hi"}})
        );
        let answer = AppMessage::Answer(Answer::MessageAck("ok".into()));
        assert_eq!(
            serde_json::to_value(&answer).unwrap(),
            json!({"answer": {"message_ack": "ok"}})
        );
    }

    #[test]
    fn unknown_offers_and_answers_still_parse() {
        let msg: AppMessage =
            serde_json::from_value(json!({"offer": {"hologram": {"size": 3}}})).unwrap();
        assert!(matches!(msg, AppMessage::Offer(Offer::Other(_))));
        let msg: AppMessage =
            serde_json::from_value(json!({"offer": {"file": {"filename": "a", "filesize": 3}}}))
                .unwrap();
        assert!(matches!(msg, AppMessage::Offer(Offer::File(f)) if f.filesize == 3));
        let msg: AppMessage = serde_json::from_value(json!({"answer": {"maybe": 1}})).unwrap();
        assert!(matches!(msg, AppMessage::Answer(Answer::Other(_))));
        assert!(serde_json::from_value::<AppMessage>(json!({"chat": 1})).is_err());
    }
}
