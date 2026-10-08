# wyrmyon

A magic-wormhole client that goes faster than light when it meets its own kind.

wyrmyon speaks the magic-wormhole protocol, so it sends to and receives from the
Python `wormhole` CLI and every other client on the public network. When both
ends run wyrmyon, it moves the data over its own iroh-based transport instead:
QUIC with hole-punching, relay fallback, one stream per file, verified and
resumable transfers. You never pick a protocol or a server, and codes keep the
familiar `7-guitarist-revenge` shape.

**Status: early.** Text, files and directories work with any magic-wormhole
client, directly or through the transit relay (`wyrm send --text`,
`wyrm send PATH`, `wyrm receive`, `wyrm receive --new`); the iroh transport
follows [ROADMAP.md](ROADMAP.md).

## Usage (planned)

The package installs two identical binaries, `wyrmyon` and the short `wyrm`.

```
wyrm report.pdf photos/      # paths: send, prints a code
echo "hello" | wyrm          # piped stdin: send text
wyrm                         # no arguments: prompt for a code, receive
wyrm 7-guitarist-revenge     # a code: receive
wyrm receive --new           # receiver first: allocate a code and wait
```

| Sender | Receiver | Transport |
| --- | --- | --- |
| `wormhole` (Python) | wyrmyon | classic transit |
| wyrmyon | `wormhole` (Python) | classic transit |
| wyrmyon | wyrmyon | iroh-v1 |

## Install (planned)

```
uvx wyrmyon                  # or: pipx install wyrmyon
```

Wheels carry the native binary and need no Python runtime, the way ruff and uv
ship. GitHub releases add archives, shell and PowerShell installers, and a
Homebrew tap.

## How it works

Both peers meet on the public magic-wormhole mailbox and run SPAKE2 on the code,
like any other client. wyrmyon then advertises `iroh-v1` in the encrypted
version message. If the peer advertises it too, the file data moves over iroh;
otherwise over classic transit. The advert is authenticated by the code, so a
mailbox server cannot downgrade the connection. The details are in
[brain/](brain/BRAIN.md).

## Development

```
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test
```

`brain/` documents how everything works, `ROADMAP.md` what comes next and
`TODO.md` the small things.

## Licence

MIT, see [LICENSE](LICENSE).
