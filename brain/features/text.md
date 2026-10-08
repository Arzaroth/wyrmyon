# Text messages

```
wyrm send --text "hello"         # or --text - to read stdin
wyrm receive 7-guitarist-revenge # or no code: prompts for it
```

The sender allocates a code (`--code-length`, default 2 words), prints it on
stderr with the server's MOTD, and waits for the receiver. It then sends phase
0, `{"offer": {"message": <text>}}`, and waits for
`{"answer": {"message_ack": "ok"}}`. The receiver prints the text on stdout
with a trailing newline, acks, and both close the mailbox `happy`.

An `{"error": ...}` from either side ends the transfer with a failing exit
status. An offer the receiver cannot handle yet (files, directories) is
answered with an `error`, as the Python client does, and so is a message the
receiver could not write to stdout, so the sender never waits forever. Such an
application-level refusal still closes the mailbox `happy`, as Python does;
only protocol and server failures close it `scary` or `errory`.

Both commands print the server's MOTD as soon as they are connected, before
waiting for the peer. Server-supplied text (MOTD, errors) has control
characters stripped before it reaches the terminal.

`--relay-url` (or `WYRMYON_RELAY_URL`) points both commands at another mailbox
server.

Works both ways with the Python `wormhole` CLI; `crates/cli/tests/interop.rs`
proves it.

## Sources

- [crates/cli/src/send.rs](../../crates/cli/src/send.rs)
- [crates/cli/src/receive.rs](../../crates/cli/src/receive.rs)
- [crates/cli/src/lib.rs](../../crates/cli/src/lib.rs)
