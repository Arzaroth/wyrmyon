# wyrmyon

A magic-wormhole client in Rust that interoperates with the public network and
switches to its own iroh-based transport when both peers run wyrmyon. Start
with `brain/BRAIN.md` to find how anything works; `ROADMAP.md` is the agreed
backlog, `TODO.md` the small work and open decisions.

## The brain

`brain/` is the committed knowledge base: architecture, features, decisions,
glossary. It exists so the tool can be understood without reading the source.
Navigate it through `brain/BRAIN.md` and the two `index.md` files rather than
grepping the tree; the `brain` skill has the full routine.

- Keep it true to the code. A change that makes a brain doc wrong fixes that
  doc in the same branch.
- The code wins: if the brain disagrees with reality, correct the brain.
- Parts of the protocol docs describe the design ahead of the code and say so.
  When a milestone builds one, the doc moves from "planned" to describing the
  code, with its `## Sources`.

## The public mailbox is shared

wyrmyon uses the community's rendezvous server, and its codes must stay
readable by every other client. These rules follow:

- Use the official appid `lothar.com/wormhole/text-or-file-xfer` and the
  standard nameplate and code format.
- Send nothing nonstandard on the mailbox until the peer's encrypted version
  message advertises `iroh-v1`. Legacy clients must never see a message they do
  not understand.
- Keep mailbox traffic small. File data never goes through the mailbox.
- Surface the server's welcome message (MOTD, errors) to the user.

## Security rules

- The capability advert travels only inside the PAKE-encrypted version
  message; never decide the transport from anything a server could forge.
- The iroh connection only counts once the peer's node ID matches the one it
  sent through the mailbox and the channel-binding MACs check out.
- Crypto lives in `crates/wormhole-core` and stays small; other crates get keys
  from it and never derive their own from raw secrets.
- Never put a code or a key in a log line, an error message or a command line
  wyrmyon spawns.

Tests never reach the public servers: they run against a local mailbox and
transit relay, and the interop suite against the Python `wormhole` CLI runs
only when asked.

## Conventions

- `CHANGELOG.md` `[Unreleased]` gets an entry with every user-facing change; it
  becomes the release notes.
- Before finishing: `cargo fmt --all`, `cargo clippy --all-targets -- -D warnings`,
  `cargo test`. CI runs the same on x86_64 and aarch64.
- Protocol identifiers are part of the wire format; changing any of them breaks
  older wyrmyon peers: the `app_versions` key `wyrmyon`, the transport name
  `iroh-v1`, the ALPN `wyrmyon/1`, the HKDF label `wyrmyon/iroh-v1/confirm`.
- Releases go through the `release` skill.
