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
    /// rendezvous relay to use, else `WYRMYON_RELAY_URL`, else the public one
    #[arg(long, value_name = "URL", env = "WORMHOLE_RELAY_URL")]
    relay_url: Option<String>,
    /// transit relay to use, else `WYRMYON_TRANSIT_HELPER`, else the public one
    #[arg(long, value_name = "tcp:HOST:PORT", env = "WORMHOLE_TRANSIT_HELPER")]
    transit_helper: Option<DirectHint>,
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
    #[command(visible_alias = "rx", aliases = ["recv", "recieve"])]
    Receive(ReceiveArgs),
    /// Facilitate sending/receiving SSH public keys; not supported
    #[command(disable_help_flag = true)]
    Ssh(SshArgs),
}

#[derive(Args)]
struct SshArgs {
    #[arg(trailing_var_arg = true, allow_hyphen_values = true, hide = true)]
    args: Vec<String>,
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
    #[arg(long, value_name = "MESSAGE")]
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
    if let Some(e) = misused(&cli) {
        e.exit();
    }
    match translate(cli, &mut ask_text, &|name| std::env::var(name).ok()) {
        Ok((global, command)) => crate::run(&global, Ok(command)),
        Err(e) => crate::exit_code(Err(e)),
    }
}

fn translate(
    cli: Cli,
    ask: &mut dyn FnMut() -> anyhow::Result<String>,
    env: &dyn Fn(&str) -> Option<String>,
) -> anyhow::Result<(Global, Command)> {
    if let Some(appid) = &cli.appid
        && appid != APPID
    {
        bail!("wyrmyon only speaks the standard appid {APPID}, not {appid}");
    }
    refuse(&[("--dump-timing", cli.dump_timing.is_some())])?;
    let ((no_listen, hide_progress), command) = match cli.command {
        Sub::Send(args) => (checked(&args.common)?, Command::Send(send_args(args, ask)?)),
        Sub::Receive(args) => (checked(&args.common)?, Command::Receive(receive_args(args))),
        Sub::Ssh(_) => bail!("`wormhole ssh` is not supported by wyrmyon"),
    };
    let relay_url = cli
        .relay_url
        .or_else(|| env("WYRMYON_RELAY_URL"))
        .unwrap_or_else(|| PUBLIC_RELAY.into());
    let transit_helper = match cli.transit_helper {
        Some(helper) => helper,
        None => env("WYRMYON_TRANSIT_HELPER")
            .as_deref()
            .unwrap_or(PUBLIC_TRANSIT_HELPER)
            .parse()
            .map_err(|e| anyhow::anyhow!("WYRMYON_TRANSIT_HELPER: {e}"))?,
    };
    let global = Global {
        relay_url,
        transit_helper,
        no_listen,
        hide_progress,
        ascii: cli.wyrm.ascii,
        force_classic: cli.wyrm.force_classic,
        force_iroh: cli.wyrm.force_iroh,
        iroh_relays: cli.wyrm.iroh_relays,
    };
    Ok((global, command))
}

fn misused(cli: &Cli) -> Option<clap::Error> {
    let Sub::Receive(args) = &cli.command else {
        return None;
    };
    if args.common.code_length.is_none() || args.allocate {
        return None;
    }
    let mut cmd = <Cli as clap::CommandFactory>::command();
    cmd.build();
    cmd.find_subcommand_mut("receive").map(|receive| {
        receive.error(
            clap::error::ErrorKind::MissingRequiredArgument,
            "Must use --allocate with --code-length",
        )
    })
}

fn checked(common: &Common) -> anyhow::Result<(bool, bool)> {
    refuse(&[
        ("-0", common.zeromode),
        ("--verify", common.verify),
        ("--tor", common.tor),
        ("--launch-tor", common.launch_tor),
        ("--tor-control-port", common.tor_control_port.is_some()),
        ("--debug-state", common.debug_state.is_some()),
    ])?;
    Ok((common.no_listen, common.hide_progress))
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
    let (text, literal_text, paths) = match args.text {
        Some(text) => (Some(text), false, Vec::new()),
        None if args.what.is_empty() => (Some(ask()?), true, Vec::new()),
        None => (None, false, args.what),
    };
    Ok(send::SendArgs {
        text,
        literal_text,
        paths,
        code_length: args.common.code_length.unwrap_or(CODE_LENGTH),
        code: args.code,
        qr: args.qr && !args.no_qr,
        times: 1,
        as_wormhole: true,
    })
}

fn receive_args(args: ReceiveArgs) -> receive::ReceiveArgs {
    receive::ReceiveArgs {
        code: args.code,
        new: args.allocate,
        code_length: args.common.code_length.unwrap_or(CODE_LENGTH),
        accept_file: args.accept_file,
        output_file: args.output_file,
        only_text: args.only_text,
        as_wormhole: true,
    }
}

fn ask_text() -> anyhow::Result<String> {
    typed_text(&mut std::io::stdin().lock(), &mut std::io::stdout())
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

#[cfg(test)]
mod tests {
    use super::*;

    fn no_env(cmd: clap::Command) -> clap::Command {
        cmd.mut_args(|arg| arg.env(None)).mut_subcommands(no_env)
    }

    fn cli_of(args: &[&str]) -> Result<Cli, clap::Error> {
        let matches = no_env(<Cli as clap::CommandFactory>::command())
            .try_get_matches_from(std::iter::once(NAME).chain(args.iter().copied()))?;
        <Cli as clap::FromArgMatches>::from_arg_matches(&matches)
    }

    fn parsed_with(
        args: &[&str],
        env: &dyn Fn(&str) -> Option<String>,
    ) -> anyhow::Result<(Global, Command)> {
        translate(cli_of(args)?, &mut || Ok("typed".into()), env)
    }

    fn parsed(args: &[&str]) -> anyhow::Result<(Global, Command)> {
        parsed_with(args, &|_| None)
    }

    fn sent(args: &[&str]) -> send::SendArgs {
        match parsed(args).unwrap().1 {
            Command::Send(args) => args,
            _ => panic!("not a send"),
        }
    }

    fn received(args: &[&str]) -> receive::ReceiveArgs {
        match parsed(args).unwrap().1 {
            Command::Receive(args) => args,
            _ => panic!("not a receive"),
        }
    }

    fn refusal(args: &[&str]) -> String {
        match parsed(args) {
            Ok(_) => panic!("{args:?} was accepted"),
            Err(e) => e.to_string(),
        }
    }

    #[test]
    fn only_the_name_wormhole_switches_the_options() {
        let os = |s: &str| Some(OsString::from(s));
        for name in [
            "wormhole",
            "/usr/bin/wormhole",
            "wormhole.exe",
            "WORMHOLE.EXE",
        ] {
            assert!(invoked_as(os(name).as_ref()), "{name}");
        }
        for name in ["wyrm", "wyrmyon", "wormhole-x", "/opt/wormhole/wyrm"] {
            assert!(!invoked_as(os(name).as_ref()), "{name}");
        }
        assert!(!invoked_as(None));
    }

    #[test]
    fn send_maps_onto_wyrm_send_with_the_qr_on() {
        let args = sent(&["send", "a.txt"]);
        assert_eq!(args.paths, [PathBuf::from("a.txt")]);
        assert_eq!(
            (args.text, args.code, args.code_length, args.qr),
            (None, None, CODE_LENGTH, true)
        );
        let args = sent(&[
            "tx", "--no-qr", "-c", "4", "--code", "7-a-b", "--text", "hi",
        ]);
        assert_eq!(
            (
                args.text.as_deref(),
                args.code.as_deref(),
                args.code_length,
                args.qr
            ),
            (Some("hi"), Some("7-a-b"), 4, false)
        );
        assert!(sent(&["send", "--no-qr", "--qr", "a"]).qr);
        assert!(!sent(&["send", "--qr", "--no-qr", "a"]).qr);
        assert_eq!(sent(&["send", "--text", "-"]).text.as_deref(), Some("-"));
        assert_eq!(sent(&["send"]).text.as_deref(), Some("typed"));
    }

    #[test]
    fn the_global_options_reach_wyrm() {
        let (global, _) = parsed(&[
            "--relay-url",
            "ws://127.0.0.1:1/v1",
            "--transit-helper",
            "tcp:127.0.0.1:2",
            "--appid",
            APPID,
            "send",
            "--no-listen",
            "--hide-progress",
            "--force-classic",
            "a",
        ])
        .unwrap();
        assert_eq!(global.relay_url, "ws://127.0.0.1:1/v1");
        assert_eq!(
            (
                global.transit_helper.hostname.as_str(),
                global.transit_helper.port
            ),
            ("127.0.0.1", 2)
        );
        assert!(global.no_listen && global.hide_progress && global.force_classic);
        assert!(!global.force_iroh);
        let (global, _) = parsed(&["rx", "--no-listen", "--listen"]).unwrap();
        assert!(!global.no_listen && !global.hide_progress);
    }

    #[test]
    fn receive_maps_onto_wyrm_receive() {
        let args = received(&["receive", "7-a-b"]);
        assert_eq!(args.code.as_deref(), Some("7-a-b"));
        assert!(!args.new && !args.accept_file && !args.only_text);
        assert_eq!((args.code_length, args.output_file), (CODE_LENGTH, None));
        let args = received(&["rx", "-a", "-c", "3", "-t", "--accept-file", "-o", "out"]);
        assert!(args.new && args.accept_file && args.only_text && args.code.is_none());
        assert_eq!(args.code_length, 3);
        assert_eq!(args.output_file, Some(PathBuf::from("out")));
        assert!(received(&["receive", "--allocate", "--only-text"]).new);
        let cli = cli_of(&["receive", "-c", "3"]).unwrap();
        let misuse = misused(&cli).unwrap();
        assert_eq!(misuse.exit_code(), 2);
        assert!(
            misuse
                .to_string()
                .contains("Must use --allocate with --code-length")
        );
        let cli = cli_of(&["receive", "-a", "-c", "3"]).unwrap();
        assert!(misused(&cli).is_none());
        assert!(parsed(&["receive", "-a", "7-a-b"]).is_err());
        assert!(parsed(&["receive", "7-a-b", "8-c-d"]).is_err());
    }

    #[test]
    fn what_wyrmyon_cannot_do_is_refused_by_name() {
        for (args, flag) in [
            (&["send", "-0", "a"][..], "-0"),
            (&["send", "-v", "a"], "--verify"),
            (&["rx", "--verify"], "--verify"),
            (&["rx", "--tor"], "--tor"),
            (&["rx", "--launch-tor"], "--launch-tor"),
            (&["rx", "--tor-control-port", "tcp:1"], "--tor-control-port"),
            (&["rx", "--debug-state"], "--debug-state"),
            (&["tx", "--debug-state", "B,N", "a"], "--debug-state"),
            (
                &["send", "--ignore-unsendable-files", "a"],
                "--ignore-unsendable-files",
            ),
            (&["--dump-timing", "t.json", "rx"], "--dump-timing"),
        ] {
            let said = refusal(args);
            assert_eq!(
                said,
                format!("{flag} is not supported by wyrmyon"),
                "{args:?}"
            );
        }
        let said = refusal(&["--appid", "example.com/other", "rx"]);
        assert!(
            said.contains(APPID) && said.contains("example.com/other"),
            "{said}"
        );
        let args = sent(&["send", "--text", "x", "a"]);
        assert!(args.text.as_deref() == Some("x") && args.paths.is_empty());
    }

    #[test]
    fn the_text_to_send_is_one_typed_line() {
        let mut prompt = Vec::new();
        let text = typed_text(&mut &b"hello there\r\nmore\n"[..], &mut prompt).unwrap();
        assert_eq!(text, "hello there");
        assert_eq!(prompt, b"Text to send: ");
        let none = typed_text(&mut &b""[..], &mut Vec::new()).unwrap_err();
        assert_eq!(none.to_string(), "no text to send");
    }

    #[test]
    fn the_prompt_failing_fails_the_send() {
        let cli = cli_of(&["send"]).unwrap();
        let err = translate(cli, &mut || anyhow::bail!("no terminal"), &|_| None).err();
        assert_eq!(err.unwrap().to_string(), "no terminal");
    }

    #[test]
    fn the_parse_ignores_the_environment() {
        let cli = cli_of(&["send", "a"]).unwrap();
        assert!(cli.relay_url.is_none() && cli.transit_helper.is_none());
        let Sub::Send(args) = cli.command else {
            panic!("not a send")
        };
        assert!(args.qr);
    }

    #[test]
    fn the_wyrmyon_servers_stand_in_for_the_wormhole_ones() {
        let env = |name: &str| match name {
            "WYRMYON_RELAY_URL" => Some("ws://127.0.0.1:3/v1".to_owned()),
            "WYRMYON_TRANSIT_HELPER" => Some("tcp:127.0.0.1:4".to_owned()),
            _ => None,
        };
        let (global, _) = parsed_with(&["rx"], &env).unwrap();
        assert_eq!(global.relay_url, "ws://127.0.0.1:3/v1");
        assert_eq!(global.transit_helper.port, 4);
        let flags = [
            "--relay-url",
            "ws://127.0.0.1:5/v1",
            "--transit-helper",
            "tcp:h:6",
            "rx",
        ];
        let (global, _) = parsed_with(&flags, &env).unwrap();
        assert_eq!(global.relay_url, "ws://127.0.0.1:5/v1");
        assert_eq!(global.transit_helper.port, 6);
        let (global, _) = parsed(&["rx"]).unwrap();
        assert_eq!(global.relay_url, PUBLIC_RELAY);
        assert_eq!(global.transit_helper.hostname, "transit.magic-wormhole.io");
        let bad = |name: &str| (name == "WYRMYON_TRANSIT_HELPER").then(|| "nowhere".to_owned());
        let err = parsed_with(&["rx"], &bad).err().unwrap().to_string();
        assert!(err.starts_with("WYRMYON_TRANSIT_HELPER: nowhere"), "{err}");
    }

    #[test]
    fn python_spellings_of_receive_and_ssh_are_known() {
        for name in ["receive", "rx", "recv", "recieve"] {
            assert_eq!(received(&[name, "7-a-b"]).code.as_deref(), Some("7-a-b"));
        }
        for args in [
            &["ssh"][..],
            &["ssh", "invite", "-c", "3"],
            &["ssh", "accept", "--help"],
        ] {
            assert_eq!(refusal(args), "`wormhole ssh` is not supported by wyrmyon");
        }
    }

    #[test]
    fn the_shim_marks_what_it_hands_to_wyrm() {
        assert!(sent(&["send", "a"]).as_wormhole && received(&["rx"]).as_wormhole);
        let typed = sent(&["send"]);
        assert!(typed.literal_text && typed.paths.is_empty());
        assert!(!sent(&["send", "--text", "-"]).literal_text);
        let args = sent(&["send", "--text", "x", "a", "b"]);
        assert!(!args.literal_text && args.paths.is_empty());
    }
}
