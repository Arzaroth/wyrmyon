# Changelog

## [Unreleased]

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
  `~/.cache/wyrmyon/partial` (or `WYRMYON_CACHE_DIR`).
- No subcommand needed: `wyrm FILE...` sends, `wyrm CODE` receives, `echo hi |
  wyrm` sends text, and plain `wyrm` asks for a code. An argument that is both
  a code and a file here is refused rather than guessed.
- The code prompt completes words with Tab.
- Several paths at once travel as one bundle, unpacked as a directory named
  `files`.
