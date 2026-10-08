mod protocol;
mod receive;
mod send;
mod transfer;
mod zipdir;

use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};
use wyrmyon_transport_classic::{DirectHint, Role, Transit};
use wyrmyon_wormhole::{Config, Key, Mood, PUBLIC_RELAY, Welcome};

const PUBLIC_TRANSIT_HELPER: &str = "tcp:transit.magic-wormhole.io:4001";

#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[command(flatten)]
    global: Global,
    #[command(subcommand)]
    command: Command,
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
}

#[derive(Subcommand)]
enum Command {
    /// Send a text message or a file
    Send(send::SendArgs),
    /// Receive what the other side sends
    Receive(receive::ReceiveArgs),
}

impl Global {
    fn config(&self) -> Config {
        Config {
            relay_url: self.relay_url.clone(),
            ..Config::default()
        }
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
            match cli.command {
                Command::Send(args) => send::run(&cli.global, args).await,
                Command::Receive(args) => receive::run(&cli.global, args).await,
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

fn show_welcome(welcome: &Welcome) {
    if let Some(motd) = &welcome.motd {
        for line in motd.lines() {
            eprintln!("Server (at relay): {}", printable(line));
        }
    }
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
