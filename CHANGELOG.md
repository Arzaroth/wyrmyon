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
