# The iroh-v1 transport

`crates/transport-iroh` (package `wyrmyon-transport-iroh`, on iroh 1.3) moves
file data over iroh QUIC when both peers advertise `iroh-v1`
([negotiation.md](negotiation.md)). The endpoint is authenticated by node IDs
exchanged through the PAKE channel and bound to the code by a MAC exchange.

## API

| Item | What it does |
| --- | --- |
| `IrohTransport::bind(Role, Relays)` | A fresh endpoint with a fresh secret key, ALPN `wyrmyon/1`, the `Minimal` preset (no pkarr or DNS publishing: addresses only ever travel through the mailbox), n0's default relays or none |
| `IrohTransport::info()` | Our `IrohInfo`, after waiting up to 5 s for the home relay (when relays are on) and a direct address |
| `IrohTransport::connect(&their_info, wormhole_key)` | Sender dials, receiver accepts; pins the node ID; channel binding; returns an `IrohPipe` on one bidirectional stream, within 60 s |
| `IrohPipe::send_chunk` / `receive_chunk` / `send_last` / `receive_last` / `finish` / `abort` | Raw bytes on the stream; the last message finishes it; `finish` closes in order, `abort` closes at once with an error code |
| `IrohTransport::close()` | Closes an endpoint that never connected, e.g. when the receiver refuses the offer |

## Wire format

On the mailbox, after the version exchange showed both sides speak `iroh-v1`,
each side sends one app message, the sender first:

```json
{"wyrmyon-iroh-v1": {"id": "<node id hex>", "relays": ["https://..."], "direct": ["192.168.1.5:41234"]}}
```

This is our own shape, not iroh's serde form of `EndpointAddr`, so an iroh
upgrade cannot change the wire. Unparsable relays and addresses are skipped; an
address with neither is refused.

On the connection:

1. The sender dials the receiver's address with ALPN `wyrmyon/1`. The receiver
   accepts only a connection whose remote node ID is the one the sender
   announced; any other is closed and it keeps waiting.
2. The sender opens one bidirectional stream. Each side writes a 32-byte tag,
   HKDF of `wormhole_key.derive("wyrmyon/iroh-v1/confirm")` with info
   `<role>:<sender id>:<receiver id>`, and reads the other's. Tags are compared
   in constant time; a mismatch closes the connection with `WrongPeer`.
3. The sender writes the file bytes, as many as offered. The receiver writes
   `{"ack": "ok", "sha256": ...}` and finishes its side; the sender reads it to
   the end (at most 4 KiB).
4. The sender closes the connection; the receiver waits up to 5 s for that
   close before closing its endpoint, so the ack is never lost to an early
   exit. Every failed `connect` closes its endpoint, and any error after the
   connection is up (a short file, a failed unzip, a disk error) aborts the
   pipe, so the peer learns at once instead of at the 30 s idle timeout.

The `Receiving (...)` line says `iroh, direct` or `iroh, via relay until a
direct path opens`: it is a snapshot at connect time, and iroh moves to a
direct path by itself once hole-punching succeeds.

The node ID pin proves the connection reaches the endpoint the peer announced
through the encrypted mailbox; the binding proves that endpoint belongs to
whoever knows the code. QUIC encrypts and authenticates the bytes, so there is
no record layer on top.

## Sources

- [crates/transport-iroh/src/lib.rs](../../crates/transport-iroh/src/lib.rs)
- [crates/cli/src/send.rs](../../crates/cli/src/send.rs)
- [crates/cli/src/receive.rs](../../crates/cli/src/receive.rs)
- [crates/cli/src/transfer.rs](../../crates/cli/src/transfer.rs)
