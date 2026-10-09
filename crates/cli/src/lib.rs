mod protocol;
mod receive;
mod send;
mod transfer;
mod zipdir;

use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};
use wyrmyon_transport_classic::{DirectHint, Role, Transit};
use wyrmyon_transport_iroh::IrohTransport;
use wyrmyon_wormhole::{Config, Key, Mood, PUBLIC_RELAY, Welcome};

const PUBLIC_TRANSIT_HELPER: &str = "tcp:transit.magic-wormhole.io:4001";

#[derive(Parser)]
#[command(
    version,
    about,
    after_help = "Without a subcommand: paths send, a code receives, piped stdin sends text, and nothing at all asks for a code."
)]
struct Cli {
    #[command(flatten)]
    global: Global,
    #[command(subcommand)]
    command: Option<Command>,
    /// Paths to send, or a code to receive
    targets: Vec<String>,
}

#[derive(Args)]
struct Global {
    /// Mailbox server to meet the peer on
    #[arg(long, global = true, env = "WYRMYON_RELAY_URL", default_value = PUBLIC_RELAY)]
    relay_url: String,
    /// Transit relay to fall back on when a direct connection fails
    #[arg(long, global = true, env = "WYRMYON_TRANSIT_HELPER", default_value = PUBLIC_TRANSIT_HELPER)]
    transit_helper: DirectHint,
    /// Do not accept inbound connections: connect out, or through the relay
    #[arg(long, global = true)]
    no_listen: bool,
    /// Do not show progress bars
    #[arg(long, global = true)]
    hide_progress: bool,
    /// Never use the iroh-v1 transport, even with another wyrmyon
    #[arg(long, global = true, conflicts_with = "force_iroh")]
    force_classic: bool,
    /// Fail rather than fall back to classic transit with a legacy peer
    #[arg(long, global = true)]
    force_iroh: bool,
    /// iroh relays: n0's public ones, or none (direct connections only)
    #[arg(long, global = true, env = "WYRMYON_IROH_RELAYS", value_enum, default_value_t = IrohRelays::Default)]
    iroh_relays: IrohRelays,
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum IrohRelays {
    Default,
    Disabled,
}

impl From<IrohRelays> for wyrmyon_transport_iroh::Relays {
    fn from(relays: IrohRelays) -> Self {
        match relays {
            IrohRelays::Default => Self::Default,
            IrohRelays::Disabled => Self::Disabled,
        }
    }
}

#[derive(Subcommand)]
enum Command {
    /// Send text, a file, a directory, or several paths as one bundle
    Send(send::SendArgs),
    /// Receive what the other side sends
    Receive(receive::ReceiveArgs),
}

impl Global {
    fn config(&self) -> Config {
        let app_versions = if self.force_classic {
            serde_json::json!({})
        } else {
            serde_json::json!({"wyrmyon": {"transports": [wyrmyon_transport_iroh::TRANSPORT]}})
        };
        Config {
            relay_url: self.relay_url.clone(),
            app_versions,
            ..Config::default()
        }
    }

    fn use_iroh(&self, wormhole: &wyrmyon_wormhole::Wormhole) -> bool {
        let theirs = wormhole.their_app_versions()["wyrmyon"]["transports"]
            .as_array()
            .is_some_and(|t| t.iter().any(|t| t == wyrmyon_transport_iroh::TRANSPORT));
        theirs && !self.force_classic
    }

    async fn iroh_or_tell(
        &self,
        wormhole: &mut wyrmyon_wormhole::Wormhole,
        role: wyrmyon_transport_iroh::Role,
    ) -> anyhow::Result<IrohTransport> {
        let bound = IrohTransport::bind(role, self.iroh_relays.into()).await;
        if bound.is_err() {
            protocol::send_error(wormhole, "cannot open an iroh endpoint").await?;
        }
        Ok(bound?)
    }

    async fn transit(&self, role: Role, key: Key) -> Transit {
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

pub fn main() -> ExitCode {
    let cli = Cli::parse();
    let runtime = tokio::runtime::Runtime::new().expect("start the async runtime");
    let result = runtime.block_on(async {
        let work = async {
            match cli.command.map_or_else(|| bare(&cli.targets), Ok) {
                Ok(Command::Send(args)) => send::run(&cli.global, args).await,
                Ok(Command::Receive(args)) => receive::run(&cli.global, args).await,
                Err(e) => Err(e),
            }
        };
        tokio::select! {
            result = work => result,
            _ = tokio::signal::ctrl_c() => Err(anyhow::anyhow!("interrupted")),
        }
    });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {}", printable(&format!("{e:#}")));
            ExitCode::FAILURE
        }
    }
}

fn bare(targets: &[String]) -> anyhow::Result<Command> {
    use std::io::IsTerminal;
    match targets {
        [] if std::io::stdin().is_terminal() => {
            Ok(Command::Receive(receive::ReceiveArgs::of_code(None)))
        }
        [] => Ok(Command::Send(send::SendArgs::of_stdin())),
        [one] if wyrmyon_wormhole::code::looks_like_code(one) => {
            if std::path::Path::new(one).exists() {
                anyhow::bail!(
                    "{one} is both a code and a file here: say `wyrm send {one}` or `wyrm receive {one}`"
                );
            }
            Ok(Command::Receive(receive::ReceiveArgs::of_code(Some(
                one.clone(),
            ))))
        }
        paths => Ok(Command::Send(send::SendArgs::of_paths(
            paths.iter().map(std::path::PathBuf::from).collect(),
        ))),
    }
}

fn show_welcome(welcome: &Welcome) {
    if let Some(motd) = &welcome.motd {
        for line in motd.lines() {
            eprintln!("Server (at relay): {}", printable(line));
        }
    }
}

fn cache_dir() -> anyhow::Result<std::path::PathBuf> {
    cache_dir_from(
        std::env::var_os("WYRMYON_CACHE_DIR"),
        std::env::var_os("XDG_CACHE_HOME"),
        std::env::var_os("HOME"),
    )
}

fn cache_dir_from(
    ours: Option<std::ffi::OsString>,
    xdg: Option<std::ffi::OsString>,
    home: Option<std::ffi::OsString>,
) -> anyhow::Result<std::path::PathBuf> {
    use std::path::PathBuf;
    let absolute =
        |dir: Option<std::ffi::OsString>| dir.map(PathBuf::from).filter(|d| d.is_absolute());
    if let Some(dir) = absolute(ours) {
        return Ok(dir);
    }
    let base = match (absolute(xdg), absolute(home)) {
        (Some(xdg), _) => xdg,
        (None, Some(home)) => home.join(".cache"),
        (None, None) => anyhow::bail!("no absolute HOME to keep partial transfers in"),
    };
    Ok(base.join("wyrmyon").join("partial"))
}

fn printable(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).collect()
}

fn mood_for(result: &anyhow::Result<()>) -> Mood {
    match result {
        Ok(()) => Mood::Happy,
        Err(e) => e
            .downcast_ref::<wyrmyon_wormhole::Error>()
            .map_or(Mood::Happy, wyrmyon_wormhole::Error::mood),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_transfers_go_where_the_environment_says() {
        use std::path::Path;
        let os = |s: &str| Some(std::ffi::OsString::from(s));
        assert_eq!(
            cache_dir_from(os("/c"), os("/x"), os("/h")).unwrap(),
            Path::new("/c")
        );
        assert_eq!(
            cache_dir_from(None, os("/x"), os("/h")).unwrap(),
            Path::new("/x/wyrmyon/partial")
        );
        assert_eq!(
            cache_dir_from(None, None, os("/h")).unwrap(),
            Path::new("/h/.cache/wyrmyon/partial")
        );
        assert!(cache_dir_from(None, None, None).is_err());
        assert_eq!(
            cache_dir_from(os(""), os("relative"), os("/h")).unwrap(),
            Path::new("/h/.cache/wyrmyon/partial")
        );
    }

    #[test]
    fn iroh_relay_flags_map_to_the_transport() {
        use wyrmyon_transport_iroh::Relays;
        assert_eq!(Relays::from(IrohRelays::Default), Relays::Default);
        assert_eq!(Relays::from(IrohRelays::Disabled), Relays::Disabled);
    }
}
