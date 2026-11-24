# Overview

A Cargo workspace of six crates, and the Android app in `apps/mobile`. The security-critical code sits in one small
crate, and the CLI and the app share one transfer library
(`crates/transfer`), so the two transports are interchangeable behind it.

## Crates

| Directory | Package | Role |
| --- | --- | --- |
| `crates/wormhole-core` | `wyrmyon-wormhole` | Mailbox client, SPAKE2, HKDF, secretbox, the version exchange |
| `crates/transport-classic` | `wyrmyon-transport-classic` | TCP hint racing, transit relay, encrypted records |
| `crates/transport-iroh` | `wyrmyon-transport-iroh` | iroh endpoint, address exchange, peer pinning, channel binding, verified blobs (iroh-blobs) |
| `crates/transfer` | `wyrmyon-transfer` | App messages, transport choice, the pipe, sending a source and receiving into a sink ([transfer.md](transfer.md)) |
| `crates/mobile` | `wyrmyon-mobile` | The Android app's Rust side: sending a stream or text, receiving into a descriptor, progress updates, bridged to Dart ([mobile.md](mobile.md)) |
| `crates/cli` | `wyrmyon` | clap, progress, prompts, destinations, zips; the `wyrmyon` and `wyrm` binaries |
| `crates/testkit` | `wyrmyon-testkit` | In-process mailbox server for tests; never shipped |

`wormhole-core` ([wormhole-core.md](wormhole-core.md)),
`transport-classic` ([transit-classic.md](transit-classic.md)) and
`transport-iroh` ([iroh-v1.md](iroh-v1.md)) are the protocol;
`transfer` ([transfer.md](transfer.md)) is a send or a receive over them. The
CLI drives it (`send.rs`, `receive.rs`, progress in `transfer.rs`, directories
in `zipdir.rs`), on a multi-threaded tokio runtime started in
`wyrmyon::main`. Both binaries are thin `main`s over that function, returning
its exit status, so they cannot drift apart.

## One pipe, two transports

There is no `Transport` trait: `wyrmyon_transfer::Pipe` is an enum over the classic
`RecordPipe` and the `IrohPipe`, and `send_stream`, `receive_stream`,
`await_ack` and `send_ack` work on either. The choice between them is made
once per transfer ([negotiation.md](negotiation.md)); everything after it,
offers, progress, hashing and the ack, is shared.

## Sources

- [Cargo.toml](../../Cargo.toml)
- [crates/cli/Cargo.toml](../../crates/cli/Cargo.toml)
- [crates/cli/src/lib.rs](../../crates/cli/src/lib.rs)
- [crates/transfer/src/protocol.rs](../../crates/transfer/src/protocol.rs)
- [crates/transfer/src/pipe.rs](../../crates/transfer/src/pipe.rs)
- [crates/testkit/src/lib.rs](../../crates/testkit/src/lib.rs)
