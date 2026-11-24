# Architecture

| Doc | What it covers |
| --- | --- |
| [overview.md](overview.md) | The workspace, the crates and the one pipe over two transports |
| [wormhole-core.md](wormhole-core.md) | The mailbox protocol, SPAKE2, phases, key derivations, codes |
| [transit-classic.md](transit-classic.md) | Classic transit: hints, handshakes, the connection race, encrypted records |
| [negotiation.md](negotiation.md) | Choosing the transport after the key exchange, and why it cannot be downgraded |
| [servers.md](servers.md) | The mailbox, the relays, and the rules for sharing the public mailbox |
| [transfer.md](transfer.md) | The transfer library: sources, sinks, events, the engines the CLI and the app share |
| [iroh-v1.md](iroh-v1.md) | The iroh transport: endpoint auth, channel binding, streams, blobs |
| [mobile.md](mobile.md) | The Android app: the Rust side, the bridge to Dart, the build hook |
| [distribution.md](distribution.md) | Platforms, release artefacts and packages, install scripts, CI |
| [testing.md](testing.md) | The gate, and keeping tests off the public servers |
