# Classic transit

`crates/transport-classic` (package `wyrmyon-transport-classic`) is the data
transport every magic-wormhole client speaks: TCP connections from exchanged
hints, a handshake keyed by the transit key, then encrypted records. Checked
against the Python client's `transit.py` (`wormhole` 0.22). Relays are not
used yet (M3).

## API

| Item | What it does |
| --- | --- |
| `Transit::new(Role, transit_key)` | Binds a listener on `0.0.0.0:<random port>` and builds direct hints from the machine's non-loopback IPv4 addresses (loopback only when there are none) |
| `Transit::without_listener()` / `with_timeout(d)` | No inbound connections and no direct hints of our own; a connect window other than 120 s |
| `Transit::info()` | Our `TransitInfo`, sent to the peer as `{"transit": ...}`. It advertises only `direct-tcp-v1` until relays are built |
| `Transit::connect(&their_info)` | Races inbound connections against outbound ones to each of their direct hints; returns the first `RecordPipe` that completes the handshake, or `NoConnection` when the window closes |
| `RecordPipe::send_record` / `flush` / `receive_record` | Encrypted records, at most `MAX_RECORD` (4 MiB) of plaintext each |
| `TransitInfo::direct_hints()` / `relay_hints()` | Parse the peer's hints leniently: unknown types, bad ports and empty hostnames are skipped, never fatal |

The transit key comes from `Wormhole::transit_key()`; every other key here is
`Key::derive`d from it, never from raw bytes.

## Wire format

Hints, as the Python client writes and reads them:

```json
{"abilities-v1": [{"type": "direct-tcp-v1"}, {"type": "relay-v1"}],
 "hints-v1": [{"type": "direct-tcp-v1", "priority": 0.0, "hostname": "192.168.1.5", "port": 4321},
              {"type": "relay-v1", "hints": [{"type": "direct-tcp-v1", "hostname": "...", "port": 4001}]}]}
```

On every TCP connection, whichever side opened it:

1. Each side writes its line: `transit sender <hex> ready\n\n` or
   `transit receiver <hex> ready\n\n`, the hex being HKDF of the transit key
   with `transit_sender` / `transit_receiver`.
2. Each reads the other's line, byte for byte; any difference drops the
   connection.
3. The sender takes the first connection to finish and writes `go\n` on it;
   the others are dropped. The receiver waits for `go\n`.

Then records, each `u32` big-endian length + 24-byte nonce + secretbox
ciphertext. The nonce is a counter from 0, big-endian, one per direction, and a
record out of order is an error. Keys: HKDF of the transit key with
`transit_record_sender_key` / `transit_record_receiver_key`, the sender
sending with the first.

A record bigger than 4 MiB of plaintext is refused before it is read.

## Limits

The listener on `0.0.0.0` answers anyone on the network, so it only ever
holds 32 handshakes at once (more connections are dropped on arrival), each
handshake has 30 s to complete, and an `accept` error is waited out rather than
ending the listener. A connection is used only after the peer sent the exact
line derived from the transit key. If writing `go\n` fails on the first
connection to finish, the sender moves on to the next one.

## Sources

- [crates/transport-classic/src/lib.rs](../../crates/transport-classic/src/lib.rs)
- [crates/transport-classic/src/hints.rs](../../crates/transport-classic/src/hints.rs)
- [crates/transport-classic/src/records.rs](../../crates/transport-classic/src/records.rs)
