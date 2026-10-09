# Architecture

| Doc | What it covers |
| --- | --- |
| [overview.md](overview.md) | The workspace, the crates and the `Transport` trait |
| [wormhole-core.md](wormhole-core.md) | The mailbox protocol, SPAKE2, phases, key derivations, codes |
| [transit-classic.md](transit-classic.md) | Classic transit: hints, handshakes, the connection race, encrypted records |
| [negotiation.md](negotiation.md) | Choosing the transport after the key exchange, and why it cannot be downgraded |
| [servers.md](servers.md) | The mailbox, the relays, and the rules for sharing the public mailbox |
| [iroh-v1.md](iroh-v1.md) | The iroh transport: endpoint auth, channel binding, streams, blobs |
| [distribution.md](distribution.md) | Platforms, release archives, installers, CI |
| [testing.md](testing.md) | The gate, and keeping tests off the public servers |
