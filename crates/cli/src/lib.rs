mod protocol;
mod receive;
mod send;

use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};
use wyrmyon_wormhole::{Config, Mood, PUBLIC_RELAY, Welcome};

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
