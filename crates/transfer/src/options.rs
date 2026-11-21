use wyrmyon_transport_classic::{DirectHint, Role, Transit};
use wyrmyon_transport_iroh::{IrohTransport, Relays};
use wyrmyon_wormhole::{Config, Key, Wormhole};

use crate::protocol;

pub const PUBLIC_TRANSIT_HELPER: &str = "tcp:transit.magic-wormhole.io:4001";

#[derive(Clone, Debug)]
pub struct Options {
    pub relay_url: String,
    pub transit_helper: DirectHint,
    pub no_listen: bool,
    pub force_classic: bool,
    pub force_iroh: bool,
    pub iroh_relays: Relays,
}

pub fn speaks_iroh(wormhole: &Wormhole) -> bool {
    wormhole.their_app_versions()["wyrmyon"]["transports"]
        .as_array()
        .is_some_and(|t| t.iter().any(|t| t == wyrmyon_transport_iroh::TRANSPORT))
}

impl Options {
    pub fn config(&self) -> Config {
        self.config_offering(&[protocol::COLLECTION])
    }

    pub fn forward_config(&self) -> Config {
        self.config_offering(&[protocol::FORWARD])
    }

    pub fn config_offering(&self, offers: &[&str]) -> Config {
        let app_versions = if self.force_classic {
            serde_json::json!({})
        } else {
            serde_json::json!({"wyrmyon": {
                "transports": [wyrmyon_transport_iroh::TRANSPORT],
                "offers": offers,
            }})
        };
        Config {
            relay_url: self.relay_url.clone(),
            app_versions,
            ..Config::default()
        }
    }

    pub fn use_iroh(&self, wormhole: &Wormhole) -> bool {
        speaks_iroh(wormhole) && !self.force_classic
    }

    pub fn use_collection(&self, wormhole: &Wormhole) -> bool {
        let theirs = wormhole.their_app_versions()["wyrmyon"]["offers"]
            .as_array()
            .is_some_and(|o| o.iter().any(|o| o == protocol::COLLECTION));
        theirs && self.use_iroh(wormhole)
    }

    pub async fn iroh_or_tell(
        &self,
        wormhole: &mut Wormhole,
        role: wyrmyon_transport_iroh::Role,
    ) -> anyhow::Result<IrohTransport> {
        let bound = IrohTransport::bind(role, self.iroh_relays.clone()).await;
        if bound.is_err() {
            protocol::send_error(wormhole, "cannot open an iroh endpoint").await?;
        }
        Ok(bound?)
    }

    pub async fn transit(&self, role: Role, key: Key) -> Transit {
        let transit = Transit::new(role, key)
            .await
            .with_relays(vec![self.transit_helper.clone()]);
        if self.no_listen {
            transit.without_listener()
        } else {
            transit
        }
    }
}
