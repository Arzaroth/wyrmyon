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
acks) on a random localhost port, and `TransitRelay`, a fake transit relay that
pairs two `please relay` lines and splices them. `start_with_welcome` sets the MOTD or a
refusal. Like the real server it answers `error` to a third side on a
nameplate (`crowded`), to anything before `bind` and to `add` before `open`.
`claimed_nameplates()` and `moods()` let tests check that a session released
its nameplate and closed with the right mood. It is a dev-dependency only.

| Suite | What it covers |
| --- | --- |
| `crates/wormhole-core/src/*` unit tests | Key derivations pinned to the Python client's values, secretbox, code parsing and word choice, server message shapes |
| `crates/wormhole-core/tests/pairing.rs` | Two peers pairing through `MailboxServer`: same keys and verifier, `app_versions` exchanged, phases in order, a wrong code, the welcome message, a crowded nameplate, a malicious peer's malformed PAKE; nameplates released and moods recorded every time. Every await has a 10 s timeout |
| `crates/transport-classic/src/records.rs` unit tests | Oversized length, replayed record and tampered record all refused |
| `crates/transport-classic/tests/transit.rs` | Two transits on localhost: handshake, records both ways including a 4 MiB one, mismatched keys never connecting, `NoConnection` when nothing is reachable, peers meeting only through `TransitRelay`, the relay taking over after the delay when the direct hints are dead |
| `crates/cli/src/zipdir.rs` unit tests | A directory round-trips through zip with permissions and empty directories; extraction stops past the offered bytes or count; an entry leaving the directory is refused; directory symlinks followed but loops not; an empty top directory still has an entry; symlink entries skipped; cancelling stops building and extracting and removes the new directory |
| `crates/cli/tests/cli.rs` | The built binaries: version, exit status on a bad code, text and files (empty and 300 kB) between two `wyrm` processes, a refused offer, a file refused without confirmation and over an existing one, non-regular files refused, a hand-written sender that sends more than it offered, a directory through the relay only, a receiver-allocated code, conflicting code flags refused, `-o` naming an existing directory |
| `crates/cli/tests/interop.rs` | Against the Python client; `#[ignore]`d, see below |

## Interop with the Python client

```bash
cargo test -p wyrmyon --test interop -- --ignored
```

Needs `wormhole` (the Python CLI) and `uvx` on PATH. Each test starts the real
Python mailbox server (`uvx --from magic-wormhole-mailbox-server twist
wormhole-mailbox`) on a free port and runs text and a 3 MB file each way, a
directory each way through the real Python transit relay with both sides
`--no-listen`, and a Python sender using a code wyrm allocated. Both clients
get a local or dead `--transit-helper`, so nothing reaches the public transit
relay; `tests/support` sets `WYRMYON_TRANSIT_HELPER=tcp:127.0.0.1:9` for every
`wyrm` it starts. The server
runs in its own process group, killed whole when the test ends: killing only
`uvx` would orphan the `twist` process it starts. It is the real
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
