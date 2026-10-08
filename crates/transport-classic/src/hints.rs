use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct TransitInfo {
    #[serde(rename = "abilities-v1", default)]
    pub abilities: Vec<Value>,
    #[serde(rename = "hints-v1", default)]
    pub hints: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DirectHint {
    pub hostname: String,
    pub port: u16,
}

impl DirectHint {
    #[must_use]
    pub fn to_json(&self) -> Value {
        json!({
            "type": "direct-tcp-v1",
            "priority": 0.0,
            "hostname": self.hostname,
            "port": self.port,
        })
    }

    fn parse(value: &Value) -> Option<Self> {
        if value["type"] != "direct-tcp-v1" {
            return None;
        }
        let hostname = value["hostname"].as_str()?;
        if hostname.is_empty() || hostname.len() > 255 {
            return None;
        }
        Some(Self {
            hostname: hostname.to_owned(),
            port: u16::try_from(value["port"].as_u64()?).ok()?,
        })
    }
}

impl std::str::FromStr for DirectHint {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let rest = s.strip_prefix("tcp:").unwrap_or(s);
        let rest = match rest.rsplit_once(':') {
            Some((head, priority)) if priority.starts_with("priority=") => head,
            _ => rest,
        };
        let (host, port) = rest
            .rsplit_once(':')
            .ok_or_else(|| format!("{s}: expected tcp:HOST:PORT"))?;
        let host = host.trim_start_matches('[').trim_end_matches(']');
        let port = port.parse().map_err(|_| format!("{s}: bad port"))?;
        if host.is_empty() {
            return Err(format!("{s}: empty host"));
        }
        Ok(Self {
            hostname: host.to_owned(),
            port,
        })
    }
}

impl TransitInfo {
    #[must_use]
    pub fn new(direct: &[DirectHint], relays: &[DirectHint]) -> Self {
        let mut abilities = vec![json!({"type": "direct-tcp-v1"})];
        let mut hints: Vec<Value> = direct.iter().map(DirectHint::to_json).collect();
        if !relays.is_empty() {
            abilities.push(json!({"type": "relay-v1"}));
            hints.push(json!({
                "type": "relay-v1",
                "hints": relays.iter().map(DirectHint::to_json).collect::<Vec<_>>(),
            }));
        }
        Self { abilities, hints }
    }

    #[must_use]
    pub fn direct_hints(&self) -> Vec<DirectHint> {
        self.hints.iter().filter_map(DirectHint::parse).collect()
    }

    #[must_use]
    pub fn relay_hints(&self) -> Vec<DirectHint> {
        self.hints
            .iter()
            .filter(|h| h["type"] == "relay-v1")
            .filter_map(|h| h["hints"].as_array())
            .flatten()
            .filter_map(DirectHint::parse)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_python_clients_hints_and_skips_the_rest() {
        let info: TransitInfo = serde_json::from_value(json!({
            "abilities-v1": [{"type": "direct-tcp-v1"}, {"type": "relay-v1"}],
            "hints-v1": [
                {"type": "direct-tcp-v1", "priority": 0.0, "hostname": "192.168.1.5", "port": 4321},
                {"type": "tor-tcp-v1", "priority": 0.0, "hostname": "x.onion", "port": 80},
                {"type": "direct-tcp-v1", "hostname": "bad", "port": 99999},
                {"type": "relay-v1", "hints": [
                    {"type": "direct-tcp-v1", "priority": 0.0, "hostname": "transit.example", "port": 4001}
                ]}
            ]
        }))
        .unwrap();
        assert_eq!(
            info.direct_hints(),
            [DirectHint {
                hostname: "192.168.1.5".into(),
                port: 4321
            }]
        );
        assert_eq!(
            info.relay_hints(),
            [DirectHint {
                hostname: "transit.example".into(),
                port: 4001
            }]
        );
    }

    #[test]
    fn parses_helper_arguments() {
        let hint: DirectHint = "tcp:transit.magic-wormhole.io:4001".parse().unwrap();
        assert_eq!(hint.hostname, "transit.magic-wormhole.io");
        assert_eq!(hint.port, 4001);
        let hint: DirectHint = "tcp:[::1]:9".parse().unwrap();
        assert_eq!(hint.hostname, "::1");
        let hint: DirectHint = "tcp:relay.example:4001:priority=0.5".parse().unwrap();
        assert_eq!((hint.hostname.as_str(), hint.port), ("relay.example", 4001));
        assert!("tcp:host".parse::<DirectHint>().is_err());
        let info = TransitInfo {
            abilities: vec![],
            hints: vec![json!({"type": "direct-tcp-v1", "hostname": "", "port": 1})],
        };
        assert_eq!(info.direct_hints(), []);
        assert!("tcp::80".parse::<DirectHint>().is_err());
    }

    #[test]
    fn writes_hints_the_python_client_reads() {
        let direct = [DirectHint {
            hostname: "10.0.0.2".into(),
            port: 7,
        }];
        let info = serde_json::to_value(TransitInfo::new(&direct, &[])).unwrap();
        assert_eq!(info["hints-v1"][0]["type"], "direct-tcp-v1");
        assert_eq!(info["hints-v1"][0]["port"], 7);
        assert_eq!(info["abilities-v1"], json!([{"type": "direct-tcp-v1"}]));

        let relay = [DirectHint {
            hostname: "relay.example".into(),
            port: 4001,
        }];
        let info = serde_json::to_value(TransitInfo::new(&direct, &relay)).unwrap();
        assert_eq!(info["hints-v1"][1]["type"], "relay-v1");
        assert_eq!(info["hints-v1"][1]["hints"][0]["hostname"], "relay.example");
        assert_eq!(info["abilities-v1"][1]["type"], "relay-v1");
    }
}
