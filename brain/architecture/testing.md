# Testing

The gate, run before every commit and in CI:

```bash
cargo fmt --all --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
```

`crates/cli/tests/cli.rs` runs both built binaries and checks they print the
same version.

## Off the public servers

Tests never reach `relay.magic-wormhole.io`, `transit.magic-wormhole.io` or
n0's relays. Planned: the suite runs against a local mailbox and transit relay
(how they are provided is an open decision in [TODO.md](../../TODO.md)), and
iroh endpoints run relay-less on localhost.

The interop suite against the Python `wormhole` CLI is the real test of the
legacy path. It needs the CLI installed and runs only when asked, never in the
default `cargo test`.

## Sources

- [crates/cli/tests/cli.rs](../../crates/cli/tests/cli.rs)
- [.github/workflows/ci.yml](../../.github/workflows/ci.yml)
