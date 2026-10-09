# wyrmyon

A magic-wormhole client that goes faster than light when it meets its own kind.

wyrmyon speaks the magic-wormhole protocol, so it sends to and receives from the
Python `wormhole` CLI and every other client on the public network. When both
ends run wyrmyon, it moves the data over its own iroh-based transport instead:
QUIC with hole-punching, relay fallback, one stream per file, verified and
resumable transfers. You never pick a protocol or a server, and codes keep the
familiar `7-guitarist-revenge` shape.

**Status: feature-complete, packaged, no release yet.** Text, files,
directories and several paths at once work with any magic-wormhole client,
directly or through the transit relay, and over iroh between two wyrms,
verified chunk by chunk and resumable after an interruption, on Linux, macOS
and Windows.

## Usage

The package installs two identical binaries, `wyrmyon` and the short `wyrm`.

```
wyrm report.pdf photos/      # paths: send (several go as one bundle), prints a code
echo "hello" | wyrm          # piped stdin: send text
wyrm                         # no arguments: prompt for a code (Tab completes), receive
wyrm 7-guitarist-revenge     # a code: receive
wyrm receive --new           # receiver first: allocate a code and wait
wyrm send --code CODE FILE   # use a code the receiver allocated
```

`wyrm send` and `wyrm receive` take every option (`--accept-file`, `-o`,
`--code-length`, `--text`); `wyrm --help` lists the global ones
(`--relay-url`, `--transit-helper`, `--no-listen`, `--force-classic`,
`--hide-progress`...).

| Sender | Receiver | Transport |
| --- | --- | --- |
| `wormhole` (Python) | wyrmyon | classic transit |
| wyrmyon | `wormhole` (Python) | classic transit |
| wyrmyon | wyrmyon | iroh-v1 |

## Install

Once the first release is out, from the
[latest release](https://github.com/Arzaroth/wyrmyon/releases/latest), on
x86_64 and arm64:

| System | Install |
| --- | --- |
| Debian, Ubuntu | `sudo apt install ./wyrmyon_X.Y.Z_amd64.deb` |
| Fedora, RHEL, openSUSE | `sudo dnf install ./wyrmyon-X.Y.Z-1.x86_64.rpm` |
| Arch | `sudo pacman -U wyrmyon-bin-X.Y.Z-1-x86_64.pkg.tar.zst`, or `makepkg -si` from the PKGBUILDs in `wyrmyon-vX.Y.Z-pkgbuild.tar.gz` |
| Windows | the `.msi` (unsigned: SmartScreen asks once), or the script below |
| Any Linux, macOS | the script below, or a `.tar.gz` |

```
curl -fsSL https://raw.githubusercontent.com/Arzaroth/wyrmyon/master/scripts/install.sh | sh
powershell -c "irm https://raw.githubusercontent.com/Arzaroth/wyrmyon/master/scripts/install.ps1 | iex"
```

The script checks the download against the release's `SHA256SUMS` and installs
into `~/.local/bin`, with man pages and shell completions
(`%LOCALAPPDATA%\Programs\wyrmyon` on Windows, binaries only). The Linux binaries are static, so they run on any
distribution. PyPI, Homebrew and the AUR come later.

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
