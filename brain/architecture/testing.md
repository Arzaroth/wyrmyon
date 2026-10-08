# Testing

The gate, run before every commit and in CI:

```bash
cargo fmt --all --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
```

## Off the public servers

Tests never reach `relay.magic-wormhole.io`, `transit.magic-wormhole.io` or
n0's relays. `crates/testkit` provides `MailboxServer`, an in-process fake of
the mailbox server (welcome, bind, allocate, claim, open, add, release, close,
acks) on a random localhost port; `start_with_welcome` sets the MOTD or a
refusal. It is a dev-dependency only.

| Suite | What it covers |
| --- | --- |
| `crates/wormhole-core/src/*` unit tests | Key derivations pinned to the Python client's values, secretbox, code parsing and word choice, server message shapes |
| `crates/wormhole-core/tests/pairing.rs` | Two peers pairing through `MailboxServer`: same key and verifier, `app_versions` exchanged, phases in order, a wrong code, the welcome message |
| `crates/cli/tests/cli.rs` | The built binaries: version, exit status on a bad code, text between two `wyrm` processes |
| `crates/cli/tests/interop.rs` | Against the Python client; `#[ignore]`d, see below |

## Interop with the Python client

```bash
cargo test -p wyrmyon --test interop -- --ignored
```

Needs `wormhole` (the Python CLI) and `uvx` on PATH. Each test starts the real
Python mailbox server (`uvx --from magic-wormhole-mailbox-server twist
wormhole-mailbox`) on a free port and runs one transfer each way. It is the real
test of the legacy path; run it whenever a change touches the wire. CI does not
run it.

`tests/support/mod.rs` spawns the binaries with `WYRMYON_RELAY_URL` set, reads
the code off the sender's stderr and keeps draining it, so a sender never dies
of a closed pipe.

## Sources

- [crates/testkit/src/lib.rs](../../crates/testkit/src/lib.rs)
- [crates/wormhole-core/tests/pairing.rs](../../crates/wormhole-core/tests/pairing.rs)
- [crates/cli/tests/cli.rs](../../crates/cli/tests/cli.rs)
- [crates/cli/tests/interop.rs](../../crates/cli/tests/interop.rs)
- [crates/cli/tests/support/mod.rs](../../crates/cli/tests/support/mod.rs)
- [.github/workflows/ci.yml](../../.github/workflows/ci.yml)
