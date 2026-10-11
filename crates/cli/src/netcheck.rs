use std::fmt::Write;
use std::net::SocketAddr;

use wyrmyon_transport_iroh::{NetCheck, PortMapping};

use crate::Global;

pub async fn run(global: &Global) -> anyhow::Result<()> {
    eprintln!("Checking the network..");
    let check = wyrmyon_transport_iroh::netcheck(&global.iroh_relays).await?;
    print!("{}", render(&check));
    Ok(())
}

fn render(check: &NetCheck) -> String {
    let mut out = String::new();
    let mut line = |label: &str, value: &str| {
        let _ = writeln!(out, "{label:<16}{value}");
    };
    line("Local addresses", &addresses(&check.local));
    if check.relays_on {
        line(
            "UDP",
            &format!("IPv4 {}, IPv6 {}", yes(check.udp_v4), yes(check.udp_v6)),
        );
        let public: Vec<_> = check
            .public_v4
            .iter()
            .chain(&check.public_v6)
            .copied()
            .collect();
        line("Public address", &addresses(&public));
        line(
            "NAT mapping",
            match check.varies_by_server {
                Some(false) => "the same for every server (an easy NAT)",
                Some(true) => "varies by server (a hard NAT)",
                None => "unknown",
            },
        );
    }
    line(
        "Port mapping",
        &check
            .port_mapping
            .map_or_else(|| "unknown".to_owned(), mapping),
    );
    if let Some(portal) = check.captive_portal {
        line("Captive portal", yes(portal));
    }
    if check.relays_on {
        let mut relays = check.relays.iter().map(|(url, latency)| match latency {
            Some(latency) => format!("{url} {} ms", latency.as_millis()),
            None => format!("{url} not measured"),
        });
        line(
            "Relays",
            &relays.next().unwrap_or_else(|| "none".to_owned()),
        );
        for relay in relays {
            line("", &relay);
        }
    } else {
        line("Relays", "disabled");
    }
    let _ = writeln!(out, "\n{}", verdict(check));
    out
}

fn verdict(check: &NetCheck) -> &'static str {
    let mapped = check
        .port_mapping
        .is_some_and(|m| m.upnp || m.pcp || m.nat_pmp);
    if !check.relays_on {
        return if mapped {
            "Without relays, a peer connects only if one side can reach the other; port mapping should let peers reach this one."
        } else {
            "Without relays, a peer connects only if one side can reach the other: on the same network, or towards a public or port-mapped peer."
        };
    }
    if !check.relays.iter().any(|(_, latency)| latency.is_some()) {
        return "No relay answered: only peers that can reach each other directly will connect.";
    }
    if !check.udp_v4 && !check.udp_v6 {
        return "No UDP answer from the relays (UDP blocked, or relays without QUIC address discovery): transfers will likely go through a relay.";
    }
    if check.public_v6.is_some() {
        return "Direct connections should open with most peers (public IPv6).";
    }
    match check.varies_by_server {
        Some(true) if mapped => {
            "A hard NAT, but port mapping is available: direct connections should still open."
        }
        Some(true) => {
            "A hard NAT: direct connections open towards peers on an easy NAT or a public address; with another hard NAT, the relay carries the transfer."
        }
        Some(false) => "Direct connections should open with most peers.",
        None => {
            "Could not tell how this NAT behaves; transfers fall back to a relay when hole-punching fails."
        }
    }
}

fn addresses(addresses: &[SocketAddr]) -> String {
    if addresses.is_empty() {
        return "none".to_owned();
    }
    addresses
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

fn mapping(mapping: PortMapping) -> String {
    format!(
        "UPnP {}, PCP {}, NAT-PMP {}",
        yes(mapping.upnp),
        yes(mapping.pcp),
        yes(mapping.nat_pmp)
    )
}

fn yes(on: bool) -> &'static str {
    if on { "yes" } else { "no" }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn online() -> NetCheck {
        NetCheck {
            relays_on: true,
            local: vec!["192.168.1.5:41234".parse().unwrap()],
            udp_v4: true,
            public_v4: Some("203.0.113.5:41234".parse().unwrap()),
            varies_by_server: Some(false),
            port_mapping: Some(PortMapping::default()),
            captive_portal: Some(false),
            relays: vec![
                ("https://a.example/".into(), Some(Duration::from_millis(23))),
                ("https://b.example/".into(), None),
            ],
            ..NetCheck::default()
        }
    }

    #[test]
    fn the_report_names_every_finding() {
        let out = render(&online());
        for expected in [
            "Local addresses 192.168.1.5:41234",
            "UDP             IPv4 yes, IPv6 no",
            "Public address  203.0.113.5:41234",
            "NAT mapping     the same for every server (an easy NAT)",
            "Port mapping    UPnP no, PCP no, NAT-PMP no",
            "Captive portal  no",
            "Relays          https://a.example/ 23 ms",
            "                https://b.example/ not measured",
            "Direct connections should open with most peers.",
        ] {
            assert!(out.contains(expected), "{expected:?} missing from\n{out}");
        }
    }

    #[test]
    fn without_relays_only_what_is_known_is_reported() {
        let check = NetCheck {
            relays_on: false,
            ..NetCheck::default()
        };
        let out = render(&check);
        assert!(out.contains("Local addresses none"), "{out}");
        assert!(out.contains("Port mapping    unknown"), "{out}");
        assert!(out.contains("Relays          disabled"), "{out}");
        assert!(!out.contains("UDP"), "{out}");
        assert!(!out.contains("Captive"), "{out}");
        assert!(
            out.contains("or towards a public or port-mapped peer"),
            "{out}"
        );
        let mapped = NetCheck {
            port_mapping: Some(PortMapping {
                pcp: true,
                ..PortMapping::default()
            }),
            ..check
        };
        assert!(render(&mapped).contains("port mapping should let peers reach this one"));
    }

    #[test]
    fn the_verdict_follows_the_nat() {
        let with = |change: &dyn Fn(&mut NetCheck)| {
            let mut check = online();
            change(&mut check);
            (verdict(&check), render(&check))
        };
        assert!(
            with(&|c| c.relays.clear())
                .1
                .contains("Relays          none")
        );
        assert!(
            with(&|c| c.relays[0].1 = None)
                .0
                .starts_with("No relay answered")
        );
        assert!(with(&|c| c.udp_v4 = false).0.starts_with("No UDP answer"));
        let v6 = with(&|c| c.public_v6 = Some("[2001:db8::1]:41234".parse().unwrap()));
        assert!(v6.0.contains("public IPv6"));
        assert!(v6.1.contains("203.0.113.5:41234, [2001:db8::1]:41234"));
        let hard = with(&|c| c.varies_by_server = Some(true));
        assert!(hard.0.starts_with("A hard NAT: direct"));
        assert!(hard.1.contains("varies by server (a hard NAT)"));
        let mapped = with(&|c| {
            c.varies_by_server = Some(true);
            c.port_mapping = Some(PortMapping {
                upnp: true,
                ..PortMapping::default()
            });
        });
        assert!(mapped.0.contains("port mapping is available"));
        let unknown = with(&|c| {
            c.varies_by_server = None;
            c.captive_portal = None;
        });
        assert!(unknown.0.starts_with("Could not tell"));
        assert!(unknown.1.contains("NAT mapping     unknown"));
        assert!(!unknown.1.contains("Captive"));
    }
}
