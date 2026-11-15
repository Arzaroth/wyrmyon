use std::collections::{BTreeMap, HashMap};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use anyhow::{Context, bail};
use clap::Args;
use indicatif::ProgressBar;
use tokio::net::TcpListener;
use wyrmyon_transport_iroh::{
    ForwardEvent, ForwardNext, Forwarding, IrohInfo, IrohTransport, Role as IrohRole,
};
use wyrmyon_wormhole::{Code, Wormhole};

use crate::protocol::{self, Answer, AppMessage, ForwardOffer, Offer};
use crate::receive::refuse;
use crate::{CODE_LENGTH, Global, mood_for, printable, show_code, show_welcome, transfer};

#[derive(Args)]
#[command(
    long_about = "Forward TCP ports between two wyrms, over the iroh-v1 transport.

One side shares ports of its own and gets a code; the other side gives the \
code and listens on those ports here, so a connection to them reaches the \
sharing side's. Both sides must run wyrmyon. The code is used once, and the \
forward lasts until either side stops it with Ctrl-C.

  wyrm forward 8080 5432            share localhost:8080 and localhost:5432
  wyrm forward db.lan:5432          share port 5432 of another host
  wyrm forward CODE                 listen on every shared port, same numbers
  wyrm forward CODE 18080:8080      listen on 18080 for their port 8080
  wyrm forward CODE 0:8080          let the system pick the port here"
)]
pub struct ForwardArgs {
    /// Ports to share (`8080`, `host:8080`), or a code then the ports to map here (`8080`, `18080:8080`)
    targets: Vec<String>,
    /// Number of words in the generated code
    #[arg(long, default_value_t = CODE_LENGTH, value_parser = clap::value_parser!(u8).range(1..=8))]
    code_length: u8,
    /// Use this code instead of generating one
    #[arg(long, conflicts_with = "code_length")]
    code: Option<String>,
    /// Also show the code as a QR code
    #[arg(long)]
    qr: bool,
    /// Address to listen on for the forwarded ports
    #[arg(long, default_value_t = IpAddr::V4(Ipv4Addr::LOCALHOST))]
    bind: IpAddr,
}

#[cfg_attr(test, derive(Debug, PartialEq))]
enum Mode {
    Share(BTreeMap<u16, String>),
    Connect(Option<Code>, Vec<(u16, u16)>),
}

fn mode(targets: &[String], code: Option<&str>) -> anyhow::Result<Mode> {
    match targets {
        [first, maps @ ..] if wyrmyon_wormhole::code::looks_like_code(first) => {
            if code.is_some() {
                bail!("give the code once: either as an argument or with --code");
            }
            Ok(Mode::Connect(
                Some(first.parse()?),
                maps.iter().map(|m| map(m)).collect::<Result<_, _>>()?,
            ))
        }
        [] => Ok(Mode::Connect(code.map(str::parse).transpose()?, Vec::new())),
        shared => {
            let mut ports = BTreeMap::new();
            for spec in shared {
                let (port, target) = share(spec)?;
                if ports.insert(port, target).is_some() {
                    bail!("port {port} is shared twice");
                }
            }
            Ok(Mode::Share(ports))
        }
    }
}

fn port(s: &str, what: &str) -> anyhow::Result<u16> {
    match s.parse::<u16>() {
        Ok(0) | Err(_) => bail!("`{s}` is not a port number for {what}"),
        Ok(port) => Ok(port),
    }
}

fn share(spec: &str) -> anyhow::Result<(u16, String)> {
    match spec.rsplit_once(':') {
        None => Ok((port(spec, "a shared port")?, format!("localhost:{spec}"))),
        Some((host, p)) if !host.is_empty() => Ok((port(p, spec)?, spec.to_owned())),
        Some(_) => bail!("`{spec}` names no host before its port"),
    }
}

fn map(spec: &str) -> anyhow::Result<(u16, u16)> {
    match spec.split_once(':') {
        None => {
            let remote = port(spec, "their port")?;
            Ok((remote, remote))
        }
        Some((local, remote)) => {
            let local = local
                .parse::<u16>()
                .with_context(|| format!("`{local}` is not a port number to listen on"))?;
            Ok((local, port(remote, "their port")?))
        }
    }
}

pub async fn run(global: &Global, args: ForwardArgs) -> anyhow::Result<()> {
    if global.force_classic {
        bail!("wyrm forward only runs over iroh-v1, which --force-classic turns off");
    }
    let mode = mode(&args.targets, args.code.as_deref())?;
    let mut interrupted = std::pin::pin!(tokio::signal::ctrl_c());
    let (mut forwarding, targets) = tokio::select! {
        pipe = negotiate(global, &args, mode) => pipe?,
        _ = &mut interrupted => bail!("interrupted"),
    };
    eprintln!(
        "Forwarding ({}) until Ctrl-C or the other side stops",
        forwarding.pipe().describe()
    );
    transfer::follow_route(forwarding.pipe(), &ProgressBar::hidden());
    let ended = loop {
        tokio::select! {
            next = forwarding.next() => match next {
                ForwardNext::Event(event) => eprintln!("{}", printable(&said(&event, &targets))),
                ForwardNext::Ended(result) => break result,
            },
            _ = &mut interrupted => {
                forwarding.close().await;
                eprintln!("Stopped forwarding");
                return Ok(());
            }
        }
    };
    forwarding.close().await;
    ended?;
    eprintln!("The other side stopped forwarding");
    Ok(())
}

fn said(event: &ForwardEvent, targets: &HashMap<u16, String>) -> String {
    let target = |port: &u16| {
        targets
            .get(port)
            .cloned()
            .unwrap_or_else(|| port.to_string())
    };
    match event {
        ForwardEvent::Opened { port } => format!("Connection to {}", target(port)),
        ForwardEvent::NotForwarded { port } if targets.contains_key(port) => {
            format!("Their side does not share port {port}")
        }
        ForwardEvent::NotForwarded { port } => {
            format!("Turned away a connection to port {port}: not shared")
        }
        ForwardEvent::Unreachable {
            port,
            why: Some(why),
        } => format!("Could not reach {}: {why}", target(port)),
        ForwardEvent::Unreachable { port, why: None } => {
            format!("Their side could not reach its port {port}")
        }
    }
}

async fn negotiate(
    global: &Global,
    args: &ForwardArgs,
    mode: Mode,
) -> anyhow::Result<(Forwarding, HashMap<u16, String>)> {
    let config = global.forward_config();
    let (pending, sharing) = match mode {
        Mode::Share(ports) => {
            let code: Option<Code> = args.code.as_deref().map(str::parse).transpose()?;
            let qr = args.qr.then_some(global.ascii);
            let pending = if let Some(code) = code {
                show_code(&code, qr, &other_side(&code));
                wyrmyon_wormhole::join(&config, code).await?
            } else {
                let pending =
                    wyrmyon_wormhole::create(&config, usize::from(args.code_length)).await?;
                show_code(pending.code(), qr, &other_side(pending.code()));
                pending
            };
            (pending, Ok(ports))
        }
        Mode::Connect(code, maps) => {
            let code = match code {
                Some(code) => code,
                None => crate::receive::prompt_code(usize::from(args.code_length)).await?,
            };
            (wyrmyon_wormhole::join(&config, code).await?, Err(maps))
        }
    };
    show_welcome(pending.welcome());
    let mut wormhole = pending.pair().await?;
    let result = match sharing {
        Ok(ports) => offer(&mut wormhole, global, ports).await,
        Err(maps) => take(&mut wormhole, global, args.bind, &maps).await,
    };
    wormhole.close(mood_for(&result)).await;
    result
}

fn other_side(code: &Code) -> String {
    format!("  wyrm forward {code}")
}

fn wants_forward(wormhole: &Wormhole) -> bool {
    crate::speaks_iroh(wormhole)
        && wormhole.their_app_versions()["wyrmyon"]["offers"]
            .as_array()
            .is_some_and(|o| o.iter().any(|o| o == protocol::FORWARD))
}

async fn offer(
    wormhole: &mut Wormhole,
    global: &Global,
    ports: BTreeMap<u16, String>,
) -> anyhow::Result<(Forwarding, HashMap<u16, String>)> {
    if !wants_forward(wormhole) {
        protocol::send_error(
            wormhole,
            "this code shares ports: take it with `wyrm forward CODE`",
        )
        .await?;
        bail!("the other side is not running `wyrm forward`");
    }
    let iroh = global.iroh_or_tell(wormhole, IrohRole::Sender).await?;
    let info = AppMessage::Iroh(iroh.info().await);
    let offer = AppMessage::Offer(Offer::Forward(ForwardOffer {
        ports: ports.keys().copied().collect(),
    }));
    let sent = async {
        protocol::send(wormhole, &info).await?;
        protocol::send(wormhole, &offer).await?;
        let mut theirs = None;
        loop {
            match protocol::next(wormhole, "other side").await? {
                AppMessage::Iroh(info) => theirs = Some(info),
                AppMessage::Answer(Answer::ForwardAck(ack)) if ack == "ok" => break,
                AppMessage::Answer(answer) => {
                    bail!("unexpected answer from the other side: {answer:?}")
                }
                _ => {}
            }
        }
        theirs.context("the other side accepted without an iroh address")
    }
    .await;
    let theirs = match sent {
        Ok(theirs) => theirs,
        Err(e) => {
            iroh.close().await;
            return Err(e);
        }
    };
    let pipe = iroh.connect(&theirs, wormhole.key()).await?;
    for (port, target) in &ports {
        eprintln!("Sharing {target} as port {port}");
    }
    let targets: HashMap<u16, String> = ports.into_iter().collect();
    Ok((pipe.serve_ports(targets.clone()), targets))
}

async fn take(
    wormhole: &mut Wormhole,
    global: &Global,
    bind: IpAddr,
    maps: &[(u16, u16)],
) -> anyhow::Result<(Forwarding, HashMap<u16, String>)> {
    let mut iroh = None;
    let listeners = match take_offer(wormhole, global, bind, maps, &mut iroh).await {
        Ok(listeners) => listeners,
        Err(e) => {
            if let Some((iroh, _)) = iroh {
                iroh.close().await;
            }
            return Err(e);
        }
    };
    let (iroh, theirs) = iroh.expect("a forward is only taken with an iroh address");
    let pipe = iroh.connect(&theirs, wormhole.key()).await?;
    let mut targets = HashMap::new();
    for (listener, remote) in &listeners {
        let here = listener
            .local_addr()
            .context("reading a listening address")?;
        eprintln!("Listening on {here} for their port {remote}");
        targets.insert(*remote, here.to_string());
    }
    Ok((pipe.forward_ports(listeners), targets))
}

async fn take_offer(
    wormhole: &mut Wormhole,
    global: &Global,
    bind: IpAddr,
    maps: &[(u16, u16)],
    iroh: &mut Option<(IrohTransport, IrohInfo)>,
) -> anyhow::Result<Vec<(TcpListener, u16)>> {
    let offered = loop {
        match protocol::next(wormhole, "other side").await? {
            AppMessage::Iroh(info) if iroh.is_none() && crate::speaks_iroh(wormhole) => {
                let ours = global.iroh_or_tell(wormhole, IrohRole::Receiver).await?;
                protocol::send(wormhole, &AppMessage::Iroh(ours.info().await)).await?;
                *iroh = Some((ours, info));
            }
            AppMessage::Offer(Offer::Forward(offered)) => break offered,
            AppMessage::Offer(_) => {
                return refuse(
                    wormhole,
                    "the other side is waiting for a port forward (`wyrm forward`), not a transfer",
                )
                .await;
            }
            _ => {}
        }
    };
    if iroh.is_none() {
        return refuse(wormhole, "a forward needs an iroh address").await;
    }
    match listen(bind, maps, &offered.ports).await {
        Ok(listeners) => {
            protocol::send(
                wormhole,
                &AppMessage::Answer(Answer::ForwardAck("ok".into())),
            )
            .await?;
            Ok(listeners)
        }
        Err(e) => {
            protocol::send_error(wormhole, "the other side cannot take this forward").await?;
            Err(e)
        }
    }
}

async fn listen(
    bind: IpAddr,
    maps: &[(u16, u16)],
    offered: &[u16],
) -> anyhow::Result<Vec<(TcpListener, u16)>> {
    let maps: Vec<(u16, u16)> = if maps.is_empty() {
        offered.iter().map(|&port| (port, port)).collect()
    } else {
        maps.to_vec()
    };
    if maps.is_empty() {
        bail!("the other side shares no port");
    }
    let mut listeners = Vec::with_capacity(maps.len());
    for (local, remote) in maps {
        if !offered.contains(&remote) {
            bail!(
                "the other side does not share port {remote} (it shares {})",
                offered
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        let at = SocketAddr::new(bind, local);
        let listener = TcpListener::bind(at)
            .await
            .with_context(|| format!("listening on {at}"))?;
        listeners.push((listener, remote));
    }
    Ok(listeners)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(targets: &[&str]) -> Vec<String> {
        targets.iter().map(|&t| t.to_owned()).collect()
    }

    #[test]
    fn the_first_argument_decides_between_sharing_and_connecting() {
        assert_eq!(
            mode(&args(&["8080", "db.lan:5432", "[::1]:22"]), None).unwrap(),
            Mode::Share(BTreeMap::from([
                (22, "[::1]:22".to_owned()),
                (5432, "db.lan:5432".to_owned()),
                (8080, "localhost:8080".to_owned()),
            ]))
        );
        let code: Code = "7-guitarist-revenge".parse().unwrap();
        assert_eq!(
            mode(
                &args(&["7-guitarist-revenge", "8080", "18080:8080", "0:22"]),
                None
            )
            .unwrap(),
            Mode::Connect(
                Some(code.clone()),
                vec![(8080, 8080), (18080, 8080), (0, 22)]
            )
        );
        assert_eq!(mode(&[], None).unwrap(), Mode::Connect(None, Vec::new()));
        assert_eq!(
            mode(&[], Some("7-guitarist-revenge")).unwrap(),
            Mode::Connect(Some(code), Vec::new())
        );
    }

    #[test]
    fn bad_specs_are_refused() {
        for bad in [
            &["0"][..],
            &["http"],
            &[":80"],
            &["host:"],
            &["host:0"],
            &["8080", "8080"],
            &["8080", "localhost:8080"],
            &["7-guitarist-revenge", "x"],
            &["7-guitarist-revenge", "1:0"],
            &["7-guitarist-revenge", "x:1"],
            &["7-guitarist-revenge", "70000"],
        ] {
            assert!(mode(&args(bad), None).is_err(), "{bad:?}");
        }
        assert!(mode(&args(&["7-guitarist-revenge"]), Some("7-guitarist-revenge")).is_err());
    }

    #[test]
    fn events_read_from_the_side_that_sees_them() {
        let targets = HashMap::from([(8080, "localhost:8080".to_owned())]);
        for (event, line) in [
            (
                ForwardEvent::Opened { port: 8080 },
                "Connection to localhost:8080",
            ),
            (
                ForwardEvent::NotForwarded { port: 8080 },
                "Their side does not share port 8080",
            ),
            (
                ForwardEvent::NotForwarded { port: 22 },
                "Turned away a connection to port 22: not shared",
            ),
            (
                ForwardEvent::Unreachable {
                    port: 8080,
                    why: Some("refused".into()),
                },
                "Could not reach localhost:8080: refused",
            ),
            (
                ForwardEvent::Unreachable {
                    port: 9,
                    why: Some("refused".into()),
                },
                "Could not reach 9: refused",
            ),
            (
                ForwardEvent::Unreachable {
                    port: 8080,
                    why: None,
                },
                "Their side could not reach its port 8080",
            ),
        ] {
            assert_eq!(said(&event, &targets), line);
        }
    }
}
