# Overview

A Cargo workspace of four crates. The security-critical code sits in one small
crate, and the CLI is written once against a `Transport` trait, so the two
transports are interchangeable behind it.

## Crates

| Directory | Package | Role |
| --- | --- | --- |
| `crates/wormhole-core` | `wyrmyon-wormhole` | Mailbox client, SPAKE2, HKDF, secretbox, the version exchange |
| `crates/transport-classic` | `wyrmyon-transport-classic` | TCP hint racing, transit relay, encrypted records |
| `crates/transport-iroh` | `wyrmyon-transport-iroh` | iroh endpoint, NodeAddr exchange, blobs |
| `crates/cli` | `wyrmyon` | clap, progress, offer and answer; the `wyrmyon` and `wyrm` binaries |
| `crates/testkit` | `wyrmyon-testkit` | In-process mailbox server for tests; never shipped |

`wormhole-core` ([wormhole-core.md](wormhole-core.md)) and
`transport-classic` ([transit-classic.md](transit-classic.md)) are built;
`transport-iroh` is still empty. The CLI sends and receives text and files
(`send.rs`, `receive.rs`, the app messages in `protocol.rs`, streaming and
progress in `transfer.rs`, directories in `zipdir.rs`), on a multi-threaded tokio runtime started in
`wyrmyon::main`. Both binaries are thin `main`s over that function, returning
its exit status, so they cannot drift apart.

## The Transport trait (Planned)

```rust
#[async_trait]
trait Transport {
    async fn send_offer(&mut self, offer: Offer) -> Result<Answer>;
    async fn send_file(&mut self, f: FileSource, p: Progress) -> Result<()>;
    async fn receive(&mut self) -> Result<Incoming>;
}
```

After the version exchange, `negotiate` returns an `IrohTransport` when the
peer advertises `iroh-v1` and a `ClassicTransit` otherwise
([negotiation.md](negotiation.md)).

Planned dependencies: tokio, tokio-tungstenite, spake2, crypto_secretbox, hkdf,
sha2, serde_json, clap, indicatif, rand, iroh, iroh-blobs.

## Sources

- [Cargo.toml](../../Cargo.toml)
- [crates/cli/Cargo.toml](../../crates/cli/Cargo.toml)
- [crates/cli/src/lib.rs](../../crates/cli/src/lib.rs)
- [crates/cli/src/protocol.rs](../../crates/cli/src/protocol.rs)
- [crates/testkit/src/lib.rs](../../crates/testkit/src/lib.rs)
