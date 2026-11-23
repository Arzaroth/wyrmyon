use anyhow::anyhow;
use flutter_rust_bridge::frb;
use wyrmyon_transfer::{Options, PUBLIC_TRANSIT_HELPER};
use wyrmyon_wormhole::{Code, PUBLIC_RELAY};

const URI_SCHEME: &str = "wormhole-transfer:";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    pub mailbox: String,
    pub transit_helper: String,
    pub iroh_relays: String,
}

#[frb(sync)]
pub fn default_settings() -> Settings {
    Settings {
        mailbox: PUBLIC_RELAY.to_owned(),
        transit_helper: PUBLIC_TRANSIT_HELPER.to_owned(),
        iroh_relays: "default".to_owned(),
    }
}

#[frb(sync)]
pub fn check_settings(settings: Settings) -> anyhow::Result<()> {
    let options = settings.options()?;
    anyhow::ensure!(
        options.relay_url.starts_with("ws://") || options.relay_url.starts_with("wss://"),
        "the mailbox: a ws:// or wss:// address"
    );
    Ok(())
}

impl Settings {
    pub(crate) fn options(&self) -> anyhow::Result<Options> {
        Ok(Options {
            relay_url: self.mailbox.clone(),
            transit_helper: self
                .transit_helper
                .parse()
                .map_err(|e| anyhow!("the transit helper: {e}"))?,
            no_listen: false,
            force_classic: false,
            force_iroh: false,
            iroh_relays: self
                .iroh_relays
                .parse()
                .map_err(|e| anyhow!("the iroh relays: {e}"))?,
        })
    }
}

#[frb(sync)]
pub fn completions(typed: String) -> Vec<String> {
    wyrmyon_wormhole::code::completions(typed.trim(), CODE_WORDS)
}

pub(crate) const CODE_WORDS: usize = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Send,
    Receive,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scanned {
    pub code: String,
    pub direction: Direction,
}

#[frb(sync)]
pub fn scan(text: String) -> anyhow::Result<Scanned> {
    let text = text.trim();
    let (code, query) = match text.get(..URI_SCHEME.len()) {
        Some(scheme) if scheme.eq_ignore_ascii_case(URI_SCHEME) => {
            let rest = &text[URI_SCHEME.len()..];
            rest.split_once('?').unwrap_or((rest, ""))
        }
        _ => (text, ""),
    };
    let code: Code = code
        .parse()
        .map_err(|_| anyhow!("this is not a wormhole code"))?;
    let leader = query.split('&').any(|pair| pair == "role=leader");
    Ok(Scanned {
        code: code.to_string(),
        direction: if leader {
            Direction::Send
        } else {
            Direction::Receive
        },
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Code,
    Notice,
    Paired,
    Hashing,
    Connected,
    Moving,
    AwaitingAck,
    Offer,
    Done,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Update {
    pub stage: Stage,
    pub bytes: u64,
    pub total: u64,
    pub detail: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OfferKind {
    Text,
    File,
    Folder,
    Files,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OfferInfo {
    pub kind: OfferKind,
    pub name: String,
    pub size: u64,
    pub count: u64,
    pub text: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_typed_code_completes_from_the_word_list() {
        assert!(completions("7-guitar".into()).contains(&"7-guitarist-".to_owned()));
        assert!(completions("7-guitarist-reve".into()).contains(&"7-guitarist-revenge".to_owned()));
        assert_eq!(completions("7".into()), Vec::<String>::new());
    }

    #[test]
    fn a_leader_link_asks_the_phone_to_send() {
        let scanned = scan("wormhole-transfer:7-guitarist-revenge?role=leader".into()).unwrap();
        assert_eq!(scanned.code, "7-guitarist-revenge");
        assert_eq!(scanned.direction, Direction::Send);
    }

    #[test]
    fn a_follower_link_or_a_bare_code_asks_the_phone_to_receive() {
        for text in [
            "wormhole-transfer:7-guitarist-revenge",
            "WORMHOLE-TRANSFER:7-guitarist-revenge?role=follower",
            "  7-guitarist-revenge\n",
        ] {
            let scanned = scan(text.into()).unwrap();
            assert_eq!(scanned.code, "7-guitarist-revenge", "{text}");
            assert_eq!(scanned.direction, Direction::Receive, "{text}");
        }
    }

    #[test]
    fn anything_else_is_not_a_code() {
        for text in ["", "https://example.com", "wormhole-transfer:", "7", "x-y"] {
            let refused = scan(text.into()).unwrap_err().to_string();
            assert_eq!(refused, "this is not a wormhole code", "{text}");
        }
    }

    #[test]
    fn the_defaults_are_the_public_servers() {
        let options = default_settings().options().unwrap();
        assert_eq!(options.relay_url, PUBLIC_RELAY);
        assert!(!options.force_classic && !options.force_iroh && !options.no_listen);
    }

    #[test]
    fn unusable_servers_are_named() {
        let helper = Settings {
            transit_helper: "nowhere".into(),
            ..default_settings()
        };
        let said = helper.options().unwrap_err().to_string();
        assert!(said.starts_with("the transit helper: "), "{said}");
        let relays = Settings {
            iroh_relays: "not a url".into(),
            ..default_settings()
        };
        let said = relays.options().unwrap_err().to_string();
        assert!(said.starts_with("the iroh relays: "), "{said}");
    }
}
