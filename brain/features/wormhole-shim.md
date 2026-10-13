# Answering to `wormhole`

```
wyrm shim                 # link wormhole to wyrmyon, unless another wormhole is on PATH
wyrm shim --dir ~/bin     # put the link there instead
wyrm shim --remove        # take the link back; only one that runs wyrmyon
wormhole send report.pdf  # the Python client's options, run by wyrmyon
```

Scripts and habits call `wormhole`. A wyrmyon binary run under that name
(the file stem of `argv[0]`, so `wormhole` or `wormhole.exe`, whatever the
link resolves to) parses the Python client's command line (magic-wormhole
0.22) instead of wyrm's, turns it into wyrm's own send and receive arguments,
and runs them: the transfer, the iroh switch with another wyrmyon, resuming
and every check are the same code.

## The Python options

| Option | Under `wormhole` |
| --- | --- |
| `--relay-url`, `--transit-helper` | As wyrm's, read from `WORMHOLE_RELAY_URL` and `WORMHOLE_TRANSIT_HELPER` rather than the `WYRMYON_` ones |
| `--appid` | Only `lothar.com/wormhole/text-or-file-xfer`; any other is refused, since the shared mailbox rules pin it |
| `--version` | `wormhole X.Y.Z (wyrmyon)` |
| `send` / `tx` | `--code`, `-c/--code-length`, `--text` (`-` reads stdin), `--hide-progress`, `--listen/--no-listen`, `WHAT` (several paths go as one bundle, as with wyrm) |
| `send --qr/--no-qr` | The QR code is on by default, as in Python; `WORMHOLE_QR` sets it, the last flag wins. `wyrm send --qr` stays off by default |
| `send` with no `WHAT` | Asks `Text to send: ` and sends the line typed, as Python does; with stdin not a terminal it sends all of stdin, as `wyrm` does |
| `receive` / `rx` | Optional `CODE`, `-a/--allocate`, `-c` (only with `--allocate`, as Python insists), `--accept-file` (or `WORMHOLE_ACCEPT_FILE`), `-o/--output-file`, `--hide-progress`, `--listen/--no-listen` |
| `receive -t/--only-text` | Refuses a file or directory offer with an error to the sender. Python declares the flag and ignores it; here it does what it says |
| Refused | `-0`, `-v/--verify`, `--tor`, `--launch-tor`, `--tor-control-port`, `--debug-state`, `--ignore-unsendable-files`, `--dump-timing`: an error naming the flag, before anything reaches the network, rather than a silent difference |

wyrm's own global options (`--ascii`, `--force-classic`, `--force-iroh`,
`--iroh-relays`, with their `WYRMYON_` environment variables) still work
under `wormhole`, hidden from its help.

## `wyrm shim`

The link goes beside the running binary when that directory is writable,
else into `~/.local/bin`, which is created and named in a hint when it is not
on PATH. `--dir` picks the directory. The link points at `wyrmyon` beside
the running binary, or the running binary itself:

| Platform | Link |
| --- | --- |
| Unix | A symlink: `wyrmyon` when it sits in the same directory, else the absolute path |
| Windows (and anything without symlinks) | A hard link `wormhole.exe`, since a symlink needs a privilege |

A `wormhole` counts as wyrmyon's when it is the same file (`same-file`:
device and inode, or the Windows file index) as the running binary or the
`wyrmyon` and `wyrm` beside it, which follows symlinks and recognises hard
links. `wyrm shim` refuses when the first `wormhole` on PATH (`PATHEXT`
names on Windows, executable files on Unix) is not wyrmyon's, naming it, and
when the link's own path holds something else; it succeeds without a change
when wyrmyon's link is already there. `--remove` looks in `--dir`, or beside
the binary and in `~/.local/bin`, removes what is wyrmyon's and fails on a
`wormhole` that is not.

## Installs and updates

`install.sh` adds `wormhole -> wyrmyon` next to `wyrm` when `command -v
wormhole` finds nothing and the path is free. `install.ps1` adds the hard
link when `Get-Command wormhole` finds nothing, and on a reinstall refreshes
one whose content matched the old `wyrmyon.exe`. The .deb, .rpm, Arch
packages and the MSI ship no `wormhole`, which would clash with the
distributions' magic-wormhole packages; their users run `wyrm shim`.

The update replaces `wyrmyon` by a rename, so a symlink follows it, while a
hard link would keep the old binary. `wyrm --update` lists the hard-linked
shims that are wyrmyon's before it updates and, when the version changed,
renames each aside, hard-links it to the new binary and removes the old copy.
An MSI upgrade does not run that step: a `wormhole.exe` from `wyrm shim` beside
an MSI install keeps the old version, and is no longer recognised as
wyrmyon's, until it is deleted and `wyrm shim` runs again.

## Sources

- [crates/cli/src/wormhole_cli.rs](../../crates/cli/src/wormhole_cli.rs)
- [crates/cli/src/shim.rs](../../crates/cli/src/shim.rs)
- [crates/cli/src/update.rs](../../crates/cli/src/update.rs)
- [crates/cli/src/receive.rs](../../crates/cli/src/receive.rs)
- [scripts/install.sh](../../scripts/install.sh), [scripts/install.ps1](../../scripts/install.ps1)
- [crates/cli/tests/wormhole.rs](../../crates/cli/tests/wormhole.rs)
