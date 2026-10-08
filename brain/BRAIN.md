# wyrmyon brain

How wyrmyon works, written so a developer or an agent can answer questions
without reading the source. Navigate through the indexes; every leaf doc ends
with the source paths it describes. The code is the source of truth: if a doc
disagrees with it, fix the doc. Docs marked **Planned** describe the agreed
design ahead of the code.

wyrmyon is a magic-wormhole client. It meets its peer on the public mailbox and
runs SPAKE2 like every other client, then picks the data transport: classic
transit with a legacy peer, its own iroh-v1 transport when both peers run
wyrmyon.

## Topics

| Doc | What it covers |
| --- | --- |
| [architecture/index.md](architecture/index.md) | How it is built: crates, negotiation, servers, the iroh-v1 protocol, distribution, tests |
| [features/index.md](features/index.md) | What it does, one doc per user-facing feature |
| [glossary.md](glossary.md) | The terms the code and the docs use |
| [decisions.md](decisions.md) | Non-obvious choices and why they were made |

## Find by question

| Question | Go to |
| --- | --- |
| Which crate does what? | [architecture/overview.md](architecture/overview.md) |
| How do two peers meet and agree on a key? | [architecture/wormhole-core.md](architecture/wormhole-core.md) |
| Which HKDF purposes derive which keys? | [architecture/wormhole-core.md](architecture/wormhole-core.md) |
| How do two peers decide between classic transit and iroh? | [architecture/negotiation.md](architecture/negotiation.md) |
| Can a malicious mailbox downgrade the connection? | [architecture/negotiation.md](architecture/negotiation.md) |
| Which servers does it talk to, and why always the public mailbox? | [architecture/servers.md](architecture/servers.md) |
| How is the iroh connection authenticated? | [architecture/iroh-v1.md](architecture/iroh-v1.md) |
| How is it built, packaged and installed? | [architecture/distribution.md](architecture/distribution.md) |
| How do tests avoid the public servers? | [architecture/testing.md](architecture/testing.md) |
| Why `wyrmyon` and `wyrm`? | [decisions.md](decisions.md) |

## Features

| Feature | Doc |
| --- | --- |
| Text messages | [features/text.md](features/text.md) |

What comes next is in [ROADMAP.md](../ROADMAP.md).
