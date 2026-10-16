# Several receivers, one code

```
wyrm send --times 3 photos/     # one code, three receivers in turn
wyrm send --times 5 --code 7-guitarist-revenge --text "see you at 8"
```

`wyrm send --times N` serves N receivers with the same code, one after
another. The receivers run anything: another wyrm, the Python client, any
magic-wormhole client. To them it is an ordinary code.

## The code

Without `--code`, the code is `Code::reusable`: the usual words
(`--code-length`) on a random 7-digit nameplate. The server allocates 1 to
999 and, only when those run out, 1000 to 999999, so it never gives a 7-digit
nameplate to someone else while it sits free between two receivers. With
`--code` the sender reuses the code given, whatever its nameplate.

## A run

1. The sender prints the code and the hint once, then
   `This code serves N receivers, one after another. A wrong code ends it.`
2. Each turn opens a fresh mailbox connection (a new side), joins the code,
   pairs, and delivers the payload exactly as a single send would, choosing
   the transport for that receiver: a collection to a newer wyrm, a zip to an
   older one or a legacy client, a blob or classic transit for a file.
3. After each turn: `Receiver K of N: done`, or `Receiver K of N:` and the
   error. A transfer that fails after pairing (refused, cut, a peer that
   cannot connect) uses up its turn and the run goes on.
4. Between turns: `Waiting for receiver K of N..`.
5. The run fails at the end with `F of N transfers failed` if any did.

The payload is staged once for the whole run (`Staged` in `send.rs`): the
zip is built at most once, the blob or the collection hashed at most once,
each the first time a receiver needs it, and all of it closed when the run
ends.

## Why a wrong code ends the run

A failed pairing (`WrongCode`, a forged or unreadable PAKE message, a server
error) stops the run with `receiver K of N:` and the error. Each turn would
otherwise hand an attacker another online guess at the code; ending at the
first one keeps the guarantee of an ordinary code, one guess per code. A
friend who mistypes ends it too, and the sender starts again.

## Turns, not at once

A nameplate holds two sides at a time, and a released side keeps its place
until every side on the nameplate has released (the real server's rule,
which `MailboxServer` follows). A receiver who arrives while another is
pairing gets `crowded`; one who arrives during a transfer waits in the
mailbox until the sender's next turn joins it. `pair()` returns only after
the server confirms the nameplate's release, and the peer releases before it
sends its version message, so the nameplate is free when the next turn
claims it ([wormhole-core.md](../architecture/wormhole-core.md)).

## Sources

- [crates/cli/src/send.rs](../../crates/cli/src/send.rs)
- [crates/wormhole-core/src/code.rs](../../crates/wormhole-core/src/code.rs)
- [crates/wormhole-core/src/wormhole.rs](../../crates/wormhole-core/src/wormhole.rs)
- [crates/cli/tests/cli.rs](../../crates/cli/tests/cli.rs)
- [crates/cli/tests/interop.rs](../../crates/cli/tests/interop.rs)
