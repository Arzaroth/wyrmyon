# Port forwarding between two wyrms

```
wyrm forward 8080 5432            # share localhost:8080 and localhost:5432, prints a code
wyrm forward db.lan:5432          # share port 5432 of another host this side reaches
wyrm forward CODE                 # listen on every shared port here, same numbers
wyrm forward CODE 18080:8080      # listen on 18080 for their port 8080
wyrm forward CODE 0:8080          # let the system pick the port here
```

`wyrm forward` carries TCP connections from one wyrm to another over the
iroh-v1 transport ([iroh-v1.md](../architecture/iroh-v1.md)). The **sharing**
side names ports and gets a code. The **taking** side gives the code and
listens on those ports, so a connection to them reaches the sharing side's
target. It runs only between two wyrms, and only over iroh: there is no
classic transit fallback, and `--force-classic` is refused at once.

## The arguments

The first argument decides the role, as with bare `wyrm`
([cli.md](cli.md)). A code (`looks_like_code`) means take, and the rest are
maps. Port specs mean share. No argument at all means take, with the code
from `--code` or the prompt (stdin when it is not a terminal).

| Side | Spec | Means |
| --- | --- | --- |
| sharing | `8080` | port 8080, connecting to `localhost:8080` |
| sharing | `host:8080`, `[::1]:22` | port 8080 (or 22), connecting to that host |
| taking | `8080` | listen on 8080 here for their 8080 |
| taking | `18080:8080` | listen on 18080 here for their 8080 |
| taking | `0:8080` | listen on a port the system picks, printed |

A port shared twice, port 0 on the sharing side, or a host with no port is
refused before anything is sent. Without maps, the taking side listens on
every shared port under its own number. It listens on `127.0.0.1` unless
`--bind` says otherwise. `--code`, `--code-length` and `--qr` work as with
`send`.

## Negotiation

The taking side advertises `offers: ["forward-v1"]` in its version message
(`Global::forward_config`), and nothing else. A plain `wyrm receive` never
advertises it. A legacy client advertises nothing at all.

1. The sharing side checks the peer's advert for `iroh-v1` and `forward-v1`
   (`wants_forward`). Without both, it sends the standard `error`
   (`this code shares ports: take it with wyrm forward CODE`) and fails, so
   a `wyrm receive` or a Python client on that code learns why.
2. It binds an iroh endpoint and sends `{"wyrmyon-iroh-v1": ...}`, then the
   offer:

   ```json
   {"offer": {"wyrmyon-forward-v1": {"ports": [5432, 8080]}}}
   ```

3. The taking side answers the address with its own. On any other offer it
   sends an `error` (`waiting for a port forward`) and fails, so a
   `wyrm send` on that code is turned away. An offer before any iroh address
   is refused the same way.
4. The taking side binds a listener for each map. A map naming a port that
   was not offered (`does not share port 80 (it shares 22)`), an empty offer,
   or a port already in use fails here, before accepting: it sends `error`
   (`the other side cannot take this forward`) and both sides fail.
5. It answers `{"answer": {"forward_ack": "ok"}}`. Both sides connect
   (pinned and bound like a transfer) and close the mailbox. The forward
   then runs on iroh alone, so the mailbox carries only these few messages
   however long it lasts.

## On the connection

The sharing side holds the iroh `Sender` role and the taking side the
`Receiver` role. After the binding tags, the first stream stays idle.
Each TCP connection accepted on a taking-side listener gets one new
bidirectional QUIC stream (`IrohPipe::forward_ports`):

1. The taking side writes the remote port, 2 bytes big-endian.
2. The sharing side (`IrohPipe::serve_ports`) looks the port up in what it
   shares, and connects to that target. It writes one status byte: `0`
   connected, `1` that port is not shared, `2` the target could not be
   reached. On `1` or `2` it finishes the stream.
3. On `0`, both sides copy bytes both ways (`copy_bidirectional`) until each
   direction ends. An end of input on one side finishes that direction on the
   other, so half-closed protocols work.

The taking side reads the status before it forwards any byte, so a refused
connection is closed at once and never half-sent. The port in the header is
the only thing the taking side chooses. The host was fixed by the sharing
side's spec, so a taker cannot reach anything the sharer did not name.

## Lines it prints

| Side | Line |
| --- | --- |
| sharing | `Sharing localhost:8080 as port 8080`, once per port |
| taking | `Listening on 127.0.0.1:8080 for their port 8080`, once per map |
| both | `Forwarding (iroh, direct) until Ctrl-C or the other side stops` |
| sharing | `Connection to localhost:8080` for each connection |
| sharing | `Could not reach localhost:8080: <why>` |
| sharing | `Turned away a connection to port 22: not shared` |
| taking | `Their side could not reach its port 8080` |
| both | `Now direct: ...` / `Back through the relay: ...` on a route change |

## How it ends

Either side stops with Ctrl-C (`Stopped forwarding`). It closes the
connection with code 0 and closes its endpoint. The other side sees that
clean close, prints `The other side stopped forwarding` and exits 0. Any other
end (an idle timeout, a reset, an error close) is an error exit. Ctrl-C
before the connection is up exits with `interrupted`, like `send`. One code
serves one taker: the code is spent once the two sides pair.

The Ctrl-C handler is installed once for the whole run (`forward::run`),
not raced by `lib.rs`'s generic one, so the endpoint is always closed, never
just dropped ([decisions.md](../decisions.md)).

## Sources

- [crates/cli/src/forward.rs](../../crates/cli/src/forward.rs)
- [crates/cli/src/protocol.rs](../../crates/cli/src/protocol.rs) (`FORWARD`, `ForwardOffer`, `Answer::ForwardAck`)
- [crates/cli/src/lib.rs](../../crates/cli/src/lib.rs) (`forward_config`, `run`)
- [crates/transport-iroh/src/forward.rs](../../crates/transport-iroh/src/forward.rs)
- [crates/cli/tests/forward.rs](../../crates/cli/tests/forward.rs)
- [crates/transport-iroh/tests/forward.rs](../../crates/transport-iroh/tests/forward.rs)
