# Answering to `wormhole`

```
wyrm shim                 # link wormhole to wyrmyon, unless another wormhole is on PATH
wyrm shim --dir ~/bin     # put the link there instead
wyrm shim --remove        # take the link back; only one that runs wyrmyon
wormhole send report.pdf  # the Python client's options, run by wyrmyon
wormhole ssh invite       # wait for an SSH public key, add it to authorized_keys
wormhole ssh accept CODE  # send yours (wyrm ssh does the same)
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
| `--relay-url`, `--transit-helper` | As wyrm's, from `WORMHOLE_RELAY_URL` and `WORMHOLE_TRANSIT_HELPER`, else `WYRMYON_RELAY_URL` and `WYRMYON_TRANSIT_HELPER`, else the public servers |
| `--appid` | Only `lothar.com/wormhole/text-or-file-xfer`, plus `lothar.com/wormhole/ssh-add` for `ssh`; any other is refused, since the shared mailbox rules pin it |
| `send` / `tx` | `--code`, `-c/--code-length`, `--text` (`-` reads all of stdin), `--hide-progress`, `--listen/--no-listen`, `WHAT` (several paths go as one bundle, as with wyrm). With `--text`, `WHAT` is ignored, as in Python |
| `send --qr/--no-qr` | The QR code is on by default, as in Python; `WORMHOLE_QR` sets it, the last flag wins. `wyrm send --qr` stays off by default |
| `send` with no `WHAT` | Prints `Text to send: ` on stdout, reads one line from stdin, terminal or not, and sends it with the newline stripped; a `-` typed there is sent as `-`. End of input with nothing read is an error |
| `receive` / `rx` | Also `recv` and `recieve`, hidden, as Python accepts them. Optional `CODE`, `-a/--allocate`, `-c` (only with `--allocate`: without it, a usage error, exit 2), `--accept-file` (or `WORMHOLE_ACCEPT_FILE`), `-o/--output-file`, `--hide-progress`, `--listen/--no-listen` |
| `receive -o` over an existing file | Prints `Overwriting <path>`, asks unless `--accept-file`, then receives beside it and replaces it once the data is complete, as Python does. An existing directory there is refused, as Python refuses to delete one. Without `-o` an existing destination is refused; under `wyrm` it takes `--overwrite` |
| `receive -t/--only-text` | Refuses a file or directory offer with an error to the sender. Python declares the flag and ignores it; here it does what it says |
| Refused | `-0`, `-v/--verify`, `--tor`, `--launch-tor`, `--tor-control-port`, `--debug-state`, `--ignore-unsendable-files`, `--dump-timing`: an error naming the flag, before anything reaches the network, rather than a silent difference |

The hints name `wormhole receive CODE` (or `wormhole send --code CODE FILE`
after `--allocate`) first and the `wyrm` form second. The accept prompt
takes Enter or any answer starting with `y`, under both names.

wyrm's own global options (`--ascii`, `--force-classic`, `--force-iroh`,
`--iroh-relays`, with their `WYRMYON_` environment variables) still work
under `wormhole`, hidden from its help.

### What still differs from Python

- `--version` prints `wormhole X.Y.Z (wyrmyon)`, not `magic-wormhole X.Y.Z`.
- `-c/--code-length` takes 1 to 8 words, as wyrm's does; Python takes any
  number.
- Two codes given to `receive` are a clap usage error (exit 2); Python prints
  "Pass either no code or just one code" and exits 1.
- `-h` and `-V` work alongside `--help` and `--version`.
- `ssh invite` refuses what is not one public key, where Python appends
  whatever text it gets, and acknowledges only after the write.
- `ssh invite` creates a missing `~/.ssh`. Python announces it will, then
  stops at `Can't read`. It also exits 1 on a destination it cannot write,
  where Python prints the reason and exits 0.
- `ssh invite --user` finds users in `/etc/passwd` only, so not those from
  LDAP or macOS Directory Services, nor any on Windows.
- `ssh accept -F` takes a file as well as a directory; Python's fails on a
  file.

## `wormhole ssh` and `wyrm ssh`

`ssh` works under both names, with the same options and code
(`ssh::SshArgs`); `wormhole ssh` adds Tor's options, only to refuse them.
`wyrm ssh` refuses to run when the current directory holds a file named
`ssh`, as `wyrm shim` does for `shim`: `wyrm ./ssh` sends it. Like the Python
client, it meets on the appid `lothar.com/wormhole/ssh-add`
(or the one `--appid` gives) and sends one text offer, answered by a
`message_ack`. It advertises nothing in `app_versions`, since no file data
moves.

`ssh invite` takes `-c/--code-length` and `-u/--user`. It checks that it can
write `authorized_keys` before it allocates a code, then prints
`wormhole ssh accept CODE` on stdout, with the `wyrm` form after it (under
`wyrm`, the other way round). `--user` reads the home, uid and gid
from `/etc/passwd`, and what it creates goes to that user, since sshd reads
the file as them. What it creates is mode 0700 (`.ssh`) or 0600
(`authorized_keys`).

The invite trusts whoever holds the code with a login, so what it writes is
held to one shape (`PublicKey::parse`): a known key type, base64 data that
names the same type, and an optional comment, on one line. Anything else is
refused with an error to the sender, which keeps out `authorized_keys`
options such as `command=` and a second key on another line. The line written
is rebuilt from those fields, on a line of its own even when the file did not
end in a newline, and the key's fingerprint is printed, as the comment is the
sender's word only. The acknowledgement goes out after the write.

`ssh accept CODE` takes `-F/--key-file` and `-y/--yes`. It looks for `*.pub`
in `~/.ssh`, or in the directory `-F` names; several are listed, sorted, and
asked about. `-F` also takes a file, which must parse as a public key, so a
private key given by mistake never leaves. Without `--yes` it asks, defaulting
to no. A malformed code is refused before any of that. The choice runs on its
own thread, so Ctrl-C still ends the wait.

## `wyrm shim`

The link goes beside the `wyrm` or `wyrmyon` that PATH finds for the running
binary (the first PATH entry of either name that is the same file as it),
else beside the running binary itself, when that directory is writable;
otherwise into `~/.local/bin`, which is created and named in a hint when it
is not on PATH. `--dir` picks the directory. Starting from the PATH entry
matters where the binary is reached through a symlink into a versioned
directory (Linuxbrew, mise, nix): the link then points at the stable name,
not at a path the next upgrade removes. The link points at `wyrmyon` in that
directory when there is one, else at that binary:

| Platform | Link |
| --- | --- |
| Unix | A symlink: `wyrmyon` when it sits in the same directory, else the absolute path |
| Windows (and anything without symlinks) | A hard link `wormhole.exe`, since a symlink needs a privilege |

A `wormhole` counts as wyrmyon's when it is the same file (`same-file`:
device and inode, or the Windows file index) as the running binary, its PATH
entry, or the `wyrmyon` and `wyrm` beside that, which follows symlinks and
recognises hard links. A dangling symlink whose target is named `wyrmyon`,
`wyrm`, `wyrmyon.exe` or `wyrm.exe` counts too, so `wyrm shim` replaces it
and `--remove` removes it.

`wyrm shim` refuses when the first `wormhole` on PATH (`PATHEXT` names on
Windows, executable files on Unix) is not wyrmyon's, naming it, and when the
link's own path holds something else; it succeeds without a change when
wyrmyon's link is already there. It also refuses when the current directory
holds a file named `shim`, which a bare `wyrm shim` could have meant to send:
`wyrm ./shim` sends it. `--remove` looks in `--dir`, or beside the binary and
in `~/.local/bin`, and removes every `wormhole` there that is wyrmyon's. It
fails only when it removed nothing and found one that is not wyrmyon's, which
it leaves in place.

## Installs and updates

`install.sh` adds `wormhole -> wyrmyon` next to `wyrm` when `command -v
wormhole` finds nothing and the path is free. `install.ps1` adds the hard
link when `Get-Command wormhole` finds nothing, and on a reinstall refreshes
one whose content matched the old `wyrmyon.exe`. The .deb, .rpm, Arch
packages and the MSI ship no `wormhole`, which would clash with the
distributions' magic-wormhole packages; their users run `wyrm shim`.

The update replaces `wyrmyon` by a rename, so a symlink follows it, while a
hard link would keep the old binary. `wyrm --update` lists the hard-linked
shims that are wyrmyon's, beside the binary and in `~/.local/bin`, before it
updates. When the version changed, it renames each to a unique temporary name
in its directory, hard-links it to the new binary and removes the old copy.
A shim that cannot be relinked does not fail the update: the outcome gains a
`warning:` line, saying the shim still runs the old wyrmyon, or, when putting
it back failed too, that it is missing and where the old one is.

Two cases stay stale:

- A hard link that `wyrm shim --dir` put anywhere else is not relinked; it
  keeps the old version, and is no longer recognised as wyrmyon's, until it
  is deleted and `wyrm shim` runs again.
- An MSI upgrade does not run that step, so a `wormhole.exe` from `wyrm shim`
  beside an MSI install goes stale the same way.

## Sources

- [crates/cli/src/wormhole_cli.rs](../../crates/cli/src/wormhole_cli.rs)
- [crates/cli/src/shim.rs](../../crates/cli/src/shim.rs)
- [crates/cli/src/ssh.rs](../../crates/cli/src/ssh.rs)
- [crates/cli/src/update.rs](../../crates/cli/src/update.rs)
- [crates/cli/src/receive.rs](../../crates/cli/src/receive.rs)
- [scripts/install.sh](../../scripts/install.sh), [scripts/install.ps1](../../scripts/install.ps1)
- [crates/cli/tests/wormhole.rs](../../crates/cli/tests/wormhole.rs)
