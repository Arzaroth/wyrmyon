use std::ffi::OsString;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, bail};
use clap::builder::BoolishValueParser;
use clap::{Args, Parser, Subcommand};
use wyrmyon_transport_classic::DirectHint;
use wyrmyon_wormhole::{APPID, PUBLIC_RELAY};

use crate::{CODE_LENGTH, Command, Global, PUBLIC_TRANSIT_HELPER, receive, send};

pub const NAME: &str = "wormhole";

#[derive(Parser)]
#[command(
    name = NAME,
    version = concat!(env!("CARGO_PKG_VERSION"), " (wyrmyon)"),
    about = "Create a Magic Wormhole and communicate through it.\n\nThis is wyrmyon answering to the name wormhole, with the options of the Python client. `wyrm --help` shows the rest of what it does."
)]
struct Cli {
    /// appid to use; only the standard one is accepted
    #[arg(long, value_name = "APPID")]
    appid: Option<String>,
    /// rendezvous relay to use
    #[arg(long, value_name = "URL", env = "WORMHOLE_RELAY_URL", default_value = PUBLIC_RELAY)]
    relay_url: String,
    /// transit relay to use
    #[arg(
        long,
        value_name = "tcp:HOST:PORT",
        env = "WORMHOLE_TRANSIT_HELPER",
        default_value = PUBLIC_TRANSIT_HELPER
    )]
    transit_helper: DirectHint,
    /// (debug) write timing data to file; not supported
    #[arg(long, value_name = "FILE.json")]
    dump_timing: Option<PathBuf>,
    #[command(flatten)]
    wyrm: Wyrm,
    #[command(subcommand)]
    command: Sub,
}

#[derive(Args)]
struct Wyrm {
    #[arg(
        long,
        global = true,
        hide = true,
        env = "WYRMYON_ASCII",
        value_parser = BoolishValueParser::new()
    )]
    ascii: bool,
    #[arg(long, global = true, hide = true, conflicts_with = "force_iroh")]
    force_classic: bool,
    #[arg(long, global = true, hide = true)]
    force_iroh: bool,
    #[arg(
        long,
        global = true,
        hide = true,
        env = "WYRMYON_IROH_RELAYS",
        default_value = "default"
    )]
    iroh_relays: wyrmyon_transport_iroh::Relays,
}

#[derive(Subcommand)]
enum Sub {
    /// Send a text message, file, or directory
    #[command(visible_alias = "tx")]
    Send(SendArgs),
    /// Receive a text message, file, or directory (from 'wormhole send')
    #[command(visible_alias = "rx")]
    Receive(ReceiveArgs),
}

#[derive(Args)]
struct Common {
    /// enable no-code anything-goes mode; not supported
    #[arg(short = '0')]
    zeromode: bool,
    /// length of code (in bytes/words)
    #[arg(
        short,
        long,
        value_name = "NUMWORDS",
        value_parser = clap::value_parser!(u8).range(1..=8)
    )]
    code_length: Option<u8>,
    /// display verification string (and wait for approval); not supported
    #[arg(short, long)]
    verify: bool,
    /// suppress progress-bar display
    #[arg(long)]
    hide_progress: bool,
    /// open a listening socket for Transit (the default)
    #[arg(long, overrides_with = "no_listen")]
    listen: bool,
    /// (debug) don't open a listening socket for Transit
    #[arg(long, overrides_with = "listen")]
    no_listen: bool,
    /// use Tor when connecting; not supported
    #[arg(long)]
    tor: bool,
    /// launch Tor, rather than use existing control/socks port; not supported
    #[arg(long)]
    launch_tor: bool,
    /// endpoint descriptor for Tor control port; not supported
    #[arg(long, value_name = "ENDPOINT")]
    tor_control_port: Option<String>,
    /// debug state-machine transitions; not supported
    #[arg(long, value_name = "MACHINES", num_args = 0..=1, default_missing_value = "all")]
    debug_state: Option<String>,
}

#[derive(Args)]
struct SendArgs {
    #[command(flatten)]
    common: Common,
    /// human-generated code phrase
    #[arg(long, value_name = "CODE")]
    code: Option<String>,
    /// text message to send, instead of a file. Use '-' to read from stdin.
    #[arg(long, value_name = "MESSAGE", conflicts_with = "what")]
    text: Option<String>,
    /// Don't raise an error if a file can't be read; not supported
    #[arg(long)]
    ignore_unsendable_files: bool,
    /// Generate and show a QR code (the default)
    #[arg(
        long,
        overrides_with = "no_qr",
        env = "WORMHOLE_QR",
        value_parser = BoolishValueParser::new(),
        default_value_t = true
    )]
    qr: bool,
    /// Do not show the QR code
    #[arg(long, overrides_with = "qr")]
    no_qr: bool,
    /// The file or directory to send; without one, the text to send is asked for
    #[arg(value_name = "WHAT")]
    what: Vec<PathBuf>,
}

#[derive(Args)]
struct ReceiveArgs {
    #[command(flatten)]
    common: Common,
    /// refuse file transfers, only accept text transfers
    #[arg(short = 't', long)]
    only_text: bool,
    /// accept file transfer without asking for confirmation
    #[arg(long, env = "WORMHOLE_ACCEPT_FILE", value_parser = BoolishValueParser::new())]
    accept_file: bool,
    /// The file or directory to create, overriding the name suggested by the sender
    #[arg(short, long, value_name = "FILENAME|DIRNAME")]
    output_file: Option<PathBuf>,
    /// Allocate a fresh code (do not prompt for one)
    #[arg(short, long, conflicts_with = "code")]
    allocate: bool,
    /// The code the sender gave you; asked for when left out
    code: Option<String>,
}

pub fn invoked_as(argv0: Option<&OsString>) -> bool {
    argv0
        .and_then(|a| Path::new(a).file_stem())
        .is_some_and(|stem| stem.eq_ignore_ascii_case(NAME))
}

pub fn main(args: Vec<OsString>) -> ExitCode {
    let cli = Cli::parse_from(args);
    match translate(cli, &mut ask_text) {
        Ok((global, command)) => crate::run(&global, Ok(command)),
        Err(e) => crate::exit_code(Err(e)),
    }
}

fn translate(
    cli: Cli,
    ask: &mut dyn FnMut() -> anyhow::Result<String>,
) -> anyhow::Result<(Global, Command)> {
    if let Some(appid) = &cli.appid
        && appid != APPID
    {
        bail!("wyrmyon only speaks the standard appid {APPID}, not {appid}");
    }
    refuse(&[("--dump-timing", cli.dump_timing.is_some())])?;
    let common = match &cli.command {
        Sub::Send(args) => &args.common,
        Sub::Receive(args) => &args.common,
    };
    refuse(&[
        ("-0", common.zeromode),
        ("--verify", common.verify),
        ("--tor", common.tor),
        ("--launch-tor", common.launch_tor),
        ("--tor-control-port", common.tor_control_port.is_some()),
        ("--debug-state", common.debug_state.is_some()),
    ])?;
    let global = Global {
        relay_url: cli.relay_url,
        transit_helper: cli.transit_helper,
        no_listen: common.no_listen,
        hide_progress: common.hide_progress,
        ascii: cli.wyrm.ascii,
        force_classic: cli.wyrm.force_classic,
        force_iroh: cli.wyrm.force_iroh,
        iroh_relays: cli.wyrm.iroh_relays,
    };
    let command = match cli.command {
        Sub::Send(args) => Command::Send(send_args(args, ask)?),
        Sub::Receive(args) => Command::Receive(receive_args(args)?),
    };
    Ok((global, command))
}

fn refuse(flags: &[(&str, bool)]) -> anyhow::Result<()> {
    match flags.iter().find(|(_, set)| *set) {
        Some((flag, _)) => bail!("{flag} is not supported by wyrmyon"),
        None => Ok(()),
    }
}

fn send_args(
    args: SendArgs,
    ask: &mut dyn FnMut() -> anyhow::Result<String>,
) -> anyhow::Result<send::SendArgs> {
    refuse(&[("--ignore-unsendable-files", args.ignore_unsendable_files)])?;
    let text = match args.text {
        None if args.what.is_empty() => Some(ask()?),
        text => text,
    };
    Ok(send::SendArgs {
        text,
        paths: args.what,
        code_length: args.common.code_length.unwrap_or(CODE_LENGTH),
        code: args.code,
        qr: args.qr && !args.no_qr,
    })
}

fn receive_args(args: ReceiveArgs) -> anyhow::Result<receive::ReceiveArgs> {
    if args.common.code_length.is_some() && !args.allocate {
        bail!("Must use --allocate with --code-length");
    }
    Ok(receive::ReceiveArgs {
        code: args.code,
        new: args.allocate,
        code_length: args.common.code_length.unwrap_or(CODE_LENGTH),
        accept_file: args.accept_file,
        output_file: args.output_file,
        only_text: args.only_text,
    })
}

fn ask_text() -> anyhow::Result<String> {
    use std::io::IsTerminal;
    let stdin = std::io::stdin();
    if !stdin.is_terminal() {
        return Ok("-".into());
    }
    typed_text(&mut stdin.lock(), &mut std::io::stderr())
}

fn typed_text(input: &mut dyn BufRead, prompt: &mut dyn Write) -> anyhow::Result<String> {
    write!(prompt, "Text to send: ")?;
    prompt.flush()?;
    let mut line = String::new();
    if input
        .read_line(&mut line)
        .context("reading the text to send")?
        == 0
    {
        bail!("no text to send");
    }
    Ok(line.trim_end_matches(['\r', '\n']).to_owned())
}
