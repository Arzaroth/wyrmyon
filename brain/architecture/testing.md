# Testing

The gate, run before every commit and in CI:

```bash
cargo fmt --all --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
```

## Coverage

```bash
scripts/coverage.sh            # summary, then files ranked by uncovered lines
scripts/coverage.sh --check    # fails below 98% of lines (WYRMYON_COVERAGE_FLOOR)
scripts/coverage.sh --html     # browsable report
```

`cargo-llvm-cov` over the whole workspace, the CLI tests' runs of the built
binaries included; `crates/testkit` is left out, being test infrastructure.
CI's `coverage` job runs `--check`, so a branch that drops line coverage under
the floor fails. The toolchain pin carries `llvm-tools-preview` for it.

What stays uncovered is what no test can provoke on demand: a UDP or TCP bind
failure, an `accept` error, a write failing in the instant after a handshake,
iroh waiting for a home relay. Everything a peer or a server can do wrong is
reached on purpose: `tests/peers.rs` drives `wyrm` against hand-written library
peers (bad offers and answers, forged acks, oversized zips over iroh, a closed
stdout, prompts answered through a pty from Python's `pty.spawn`), and
`MailboxServer::start_with(Quirks { .. })` makes the fake server allocate a
malformed nameplate, refuse allocation, flood messages, hang up after the
welcome, or chatter between replies.

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
| `crates/transport-iroh/tests/blobs.rs` | A verified blob over a bound connection with the cache cleared after; blobs bigger or smaller than the offer refused and dropped; a 64 MiB transfer cut mid-way (the receiver stalls in its progress callback while the sender is aborted) that must fail, then resume strictly between 0 and the full size on a fresh connection; a failed export that keeps the verified data; an import of a missing file |
| `crates/transport-iroh/tests/iroh.rs` | Two iroh endpoints on localhost, relays disabled: data and an ack over a bound channel, a different wormhole key refused by both sides, a stranger turned away while the real sender still connects, unusable addresses failing at once |
| `crates/cli/src/zipdir.rs` unit tests | A directory round-trips through zip with permissions and empty directories; extraction stops past the offered bytes or count; an entry leaving the directory is refused; directory symlinks followed but loops not; an empty top directory still has an entry; symlink entries skipped; cancelling stops building and extracting and removes the new directory |
| `crates/cli/tests/cli.rs` | The built binaries: version, exit status on a bad code, text and files (empty and 300 kB) between two `wyrm` processes, a refused offer, a file refused without confirmation and over an existing one, non-regular files refused, a hand-written sender that sends more than it offered, a directory through the relay only, a receiver-allocated code, conflicting code flags refused, `-o` naming an existing directory, two wyrms on iroh (a file and a directory) unless either forces classic, `--force-iroh` refused against a legacy peer on either side with the reason on both, bare arguments (a path, a code, piped stdin), a target that is both a code and a file, several paths as one bundle and duplicate names refused, a code piped into `receive` |
| `crates/cli/tests/peers.rs` | `wyrm` against scripted library peers: every refusal both ways, unknown messages and offers, a destination appearing mid-transfer, a zip larger than its offer over iroh, bad acks, a closed stdout, stdin text, the MOTD, the code and consent prompts through a pty, and bare `wyrm` completing a half-typed word with Tab |
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
relay; `tests/support` sets `WYRMYON_TRANSIT_HELPER=tcp:127.0.0.1:9` and
`WYRMYON_IROH_RELAYS=disabled` and a fresh `WYRMYON_CACHE_DIR` under
`target/tmp` for every `wyrm` it starts, so wyrm-to-wyrm
tests use iroh over localhost only. The server
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
- [scripts/coverage.sh](../../scripts/coverage.sh)
- [.github/workflows/ci.yml](../../.github/workflows/ci.yml)
