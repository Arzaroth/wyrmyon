# Changelog

## [Unreleased]

### Fixed

- The README's Arch line installed the package from its URL, which fails:
  pacman then looks for a `.sig` the release does not have. It now downloads
  the package and installs the file.

## [0.2.0] - 2026-10-12

### Added

- `wyrm send --qr` also shows the code as a QR code of
  `wormhole-transfer:CODE`, as the Python client does. Off by default.
- The progress bar is a wyrm eating its way to the end, and the sender now
  draws one between two wyrms too. `--ascii` (or `WYRMYON_ASCII=1`) draws
  it, and the QR code, with ASCII characters only.
- Between two wyrms, a directory or several paths travel as a collection:
  one verified blob per file, with no temporary zip on the sender and no
  extraction on the receiver. An interrupted collection resumes when the same
  files are sent again. Older wyrms and other magic-wormhole clients still get
  a zip, which is now built only once the receiver's client is known.
- `wyrm --update` replaces a copy installed by `install.sh`, `install.ps1` or
  from a release archive with the latest release. A copy installed by the
  .deb, the .rpm, an Arch package, Homebrew, Nix or the MSI is refused, with
  how to upgrade it instead. The MSI now records its ProductCode and install
  directory under `HKCU\Software\wyrmyon`, so its copy can be told apart
  from one `install.ps1` put in the same directory.
- `wyrm netcheck`: local and public addresses, UDP, whether the NAT is easy
  or hard, UPnP, PCP and NAT-PMP, relay latencies, and what that means for
  direct connections.
- Between two wyrms, both sides dial each other, so with
  `--iroh-relays disabled` a transfer works when either side can reach the
  other, not only when the sender can reach the receiver. The cost: a
  sender whose dial fails now waits up to a minute for the receiver's dial
  before giving up, and then says why its own dial failed.
- `--iroh-relays` takes relay URLs (`http` or `https`, separated by commas)
  besides `default` and `disabled`, for relays of your own.
- Between two wyrms, a line says when hole-punching moves the connection
  from the relay to a direct path (or back), and the end of the transfer
  says how much went direct: `Went direct`, `Went through the relay`, or
  `Went 97% direct, the rest through the relay`.

### Changed

- The hint after the code names it: `wyrm CODE` (or `wormhole receive CODE`).
- The receiver's `ok?` prompt accepts on Enter: `ok? (Y/n)`. On Unix,
  anything typed before the prompt appeared is dropped first.
- Wire: the encrypted version message now also advertises
  `"offers": ["collection-v1"]`, and the collection offer goes only to a
  peer that advertised it. wyrmyon 0.1.0 and other magic-wormhole clients
  are unaffected: they get a zip, as before.

### Fixed

- Ctrl-C while `wyrm send` builds a zip file, while `wyrm receive` unpacks
  one, or at the receiver's `ok?` prompt, now ends `wyrm`. It used to print
  `error: interrupted` and hang.
- Between two wyrms, a receive whose final export failed or was cut short
  no longer leaves a cache entry that makes every retry fail: the entry is
  deleted, or started afresh if the process was killed mid-export.

### Security

- A relay the other wyrm advertises is dialled only over https, over http
  on loopback, or when it is one of your own `--iroh-relays`, and never with
  `--iroh-relays disabled`. Before, any URL the peer named was dialled, plain
  http included.

## [0.1.0] - 2026-10-10

### Added

- `wyrm send --text` and `wyrm receive`: text messages to and from any
  magic-wormhole client, over the public mailbox or `--relay-url`.
- `wyrm send FILE`: one file to any magic-wormhole client over a direct
  connection. `wyrm receive` asks before accepting, or takes `--accept-file`,
  and `-o` picks where the file goes. It never overwrites an existing file.
- Directories, sent as a zip file like the Python client does.
- The transit relay fallback, for peers that cannot connect directly:
  `--transit-helper` picks the relay, `--no-listen` turns off inbound
  connections.
- Progress bars (`--hide-progress` turns them off).
- Receiver-first codes: `wyrm receive --new` allocates the code, and
  `wyrm send --code CODE` (or `wormhole send --code`) uses it.
- The iroh-v1 transport: when both sides run wyrmyon, files and directories
  travel over iroh QUIC with hole-punching and n0's relays, bound to the code.
  `--force-classic` and `--force-iroh` override the choice; `--iroh-relays
  disabled` keeps iroh to direct connections.
- Between two wyrms, transfers are verified chunk by chunk (BLAKE3) and
  resume after an interruption: sending the same file again, with a new code,
  fetches only what the receiver does not have yet. Partial data waits in
  `~/.cache/wyrmyon/partial`, `%LOCALAPPDATA%\wyrmyon\partial` on Windows,
  or `WYRMYON_CACHE_DIR`.
- No subcommand needed: `wyrm FILE...` sends, `wyrm CODE` receives, `echo hi |
  wyrm` sends text, and plain `wyrm` asks for a code. An argument that is both
  a code and a file here is refused rather than guessed.
- The code prompt completes words with Tab.
- Several paths at once travel as one bundle, unpacked as a directory named
  `files`.
- Runs on Linux, macOS and Windows, on x86_64 and arm64. Each release has
  .deb, .rpm and Arch packages, a Windows MSI and archives, and
  `scripts/install.sh` and `scripts/install.ps1` install the latest one; the
  Linux binaries are static and run on any distribution.
- Man pages and bash, zsh and fish completions in the Linux packages, the
  archives and what `install.sh` installs; the archives add PowerShell and
  elvish completions.
