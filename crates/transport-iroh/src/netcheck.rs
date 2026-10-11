use std::net::SocketAddr;
use std::time::Duration;

use iroh::endpoint::presets;
use iroh::{Endpoint, Watcher};

use crate::{ADDRESS_WAIT, Error, Relays, relay_mode};

const REPORT_WAIT: Duration = Duration::from_secs(10);
const PORT_MAPPING_WAIT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PortMapping {
    pub upnp: bool,
    pub pcp: bool,
    pub nat_pmp: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NetCheck {
    pub relays_on: bool,
    pub local: Vec<SocketAddr>,
    pub udp_v4: bool,
    pub udp_v6: bool,
    pub public_v4: Option<SocketAddr>,
    pub public_v6: Option<SocketAddr>,
    pub varies_by_server: Option<bool>,
    pub port_mapping: Option<PortMapping>,
    pub captive_portal: Option<bool>,
    pub relays: Vec<(String, Option<Duration>)>,
}

pub async fn netcheck(relays: &Relays) -> Result<NetCheck, Error> {
    let mode = relay_mode(relays);
    let endpoint = Endpoint::builder(presets::Minimal)
        .relay_mode(mode.clone())
        .bind()
        .await
        .map_err(|e| Error::Bind(e.to_string()))?;
    let relays_on = *relays != Relays::Disabled;
    let report = async {
        if !relays_on {
            return None;
        }
        tokio::time::timeout(REPORT_WAIT, endpoint.net_report().initialized())
            .await
            .ok()
    };
    let local = async {
        let _ = tokio::time::timeout(ADDRESS_WAIT, async {
            while endpoint.addr().ip_addrs().next().is_none() {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await;
        endpoint.addr().ip_addrs().copied().collect::<Vec<_>>()
    };
    let (report, local, port_mapping) = tokio::join!(report, local, port_mapping());
    endpoint.close().await;

    let mut check = NetCheck {
        relays_on,
        local,
        port_mapping,
        relays: mode
            .relay_map()
            .urls::<Vec<_>>()
            .into_iter()
            .map(|url| (url.to_string(), None))
            .collect(),
        ..NetCheck::default()
    };
    if let Some(report) = report {
        check.udp_v4 = report.udp_v4;
        check.udp_v6 = report.udp_v6;
        check.public_v4 = report.global_v4.map(SocketAddr::V4);
        check.public_v6 = report.global_v6.map(SocketAddr::V6);
        check.varies_by_server = report.mapping_varies_by_dest();
        check.captive_portal = report.captive_portal;
        for (_, url, latency) in report.relay_latency.iter() {
            let url = url.to_string();
            match check.relays.iter_mut().find(|(known, _)| *known == url) {
                Some((_, best)) => *best = Some(best.map_or(latency, |b| b.min(latency))),
                None => check.relays.push((url, Some(latency))),
            }
        }
    }
    check
        .relays
        .sort_by_key(|(url, latency)| (latency.is_none(), *latency, url.clone()));
    Ok(check)
}

async fn port_mapping() -> Option<PortMapping> {
    let client = portmapper::Client::new(portmapper::Config::default());
    let probed = tokio::time::timeout(PORT_MAPPING_WAIT, client.probe())
        .await
        .ok()?
        .ok()?
        .ok()?;
    Some(PortMapping {
        upnp: probed.upnp,
        pcp: probed.pcp,
        nat_pmp: probed.nat_pmp,
    })
}
