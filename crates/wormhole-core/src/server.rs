use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Outbound {
    Bind {
        appid: String,
        side: String,
        client_version: (String, String),
    },
    Allocate,
    Claim {
        nameplate: String,
    },
    Release {
        nameplate: String,
    },
    Open {
        mailbox: String,
    },
    Add {
        phase: String,
        body: String,
    },
    Close {
        mailbox: String,
        mood: Mood,
    },
}

impl Outbound {
    #[must_use]
    pub fn to_frame(&self) -> String {
        let mut value = serde_json::to_value(self).expect("outbound messages serialise");
        value["id"] = Value::String(hex::encode(rand::random::<[u8; 2]>()));
        value.to_string()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mood {
    Happy,
    Lonely,
    Scary,
    Errory,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Inbound {
    Welcome {
        #[serde(default)]
        welcome: Map<String, Value>,
    },
    Ack,
    Allocated {
        nameplate: String,
    },
    Claimed {
        mailbox: String,
    },
    Released,
    Message {
        side: String,
        phase: String,
        body: String,
    },
    Closed,
    Error {
        error: String,
    },
    #[serde(other)]
    Unknown,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outbound_frames_carry_type_and_id() {
        let frame: Value = serde_json::from_str(
            &Outbound::Close {
                mailbox: "m".into(),
                mood: Mood::Happy,
            }
            .to_frame(),
        )
        .unwrap();
        assert_eq!(frame["type"], "close");
        assert_eq!(frame["mood"], "happy");
        assert_eq!(frame["id"].as_str().unwrap().len(), 4);
    }

    #[test]
    fn inbound_tolerates_unknown_types_and_extra_fields() {
        let msg: Inbound = serde_json::from_str(
            r#"{"type":"message","side":"s","phase":"pake","body":"00","id":"x","server_rx":1}"#,
        )
        .unwrap();
        assert!(matches!(msg, Inbound::Message { phase, .. } if phase == "pake"));
        let msg: Inbound = serde_json::from_str(r#"{"type":"pong","pong":1}"#).unwrap();
        assert!(matches!(msg, Inbound::Unknown));
    }
}
