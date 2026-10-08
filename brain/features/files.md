# Files

```
wyrm send report.pdf
wyrm receive 7-guitarist-revenge              # asks before accepting
wyrm receive --accept-file -o ~/in/r.pdf CODE # no question, chosen path
```

One regular file per transfer, with any magic-wormhole client. Directories
have their own doc ([directories.md](directories.md)); pipes, devices and
anything else whose size the sender cannot know are refused.

## Options both sides share

| Option | Effect |
| --- | --- |
| `--transit-helper tcp:HOST:PORT` | The transit relay to fall back on (default `transit.magic-wormhole.io:4001`) |
| `--no-listen` | No inbound connections: connect out, or through a relay |
| `--hide-progress` | No progress bar (none is drawn when stderr is not a terminal either) |

## Receiver first

`wyrm receive --new` allocates the code and waits; the sender then runs
`wyrm send --code CODE FILE`, or `wormhole send --code CODE FILE` with the
Python client. Only the side that allocates prints the code.

## Sender

1. Pairs, then sends `{"transit": <hints>}` and
   `{"offer": {"file": {"filename", "filesize"}}}`.
2. Waits for the receiver's `transit` and `{"answer": {"file_ack": "ok"}}`.
   An `error` or any other answer fails the transfer.
3. Connects ([transit-classic.md](../architecture/transit-classic.md)) and
   streams the file in 64 KiB records with a progress bar, hashing it with
   SHA-256 (`transfer.rs`). A file that grows or shrinks during the send fails
   it.
4. Reads the receiver's last record, `{"ack": "ok", "sha256": <hex>}`, and
   checks the hash when there is one.

## Receiver

1. On the sender's `transit`, starts its own `Transit` and answers with its
   hints. Then waits for the offer.
2. Takes only the last path component of the offered name, with control
   characters stripped; `.`, `..` and empty names are refused, and so is an
   offer that came without a `transit` message, all before asking anything.
3. Refuses to overwrite: anything at the destination, a dangling symlink
   included, is answered with an `error`.
4. Asks `ok? (y/N)` unless `--accept-file`. Without a terminal to ask on, it
   refuses and tells the sender, rather than leaving it waiting.
5. Creates `.<name>.<unique>.wyrm-part` next to the destination before
   acking, so a directory it cannot write to is refused up front. Writes
   records into it, stops at the offered size and fails if the sender sends
   more, syncs, then hard-links it to the destination, which fails rather than
   overwrite a file that appeared meanwhile (a checked rename where hard links
   are not supported). The partial file is removed on every exit, Ctrl-C
   included: `wyrmyon::main` turns Ctrl-C into an error, which unwinds the
   transfer.
6. Sends the ack record with the SHA-256 of what it wrote.

## Sources

- [crates/cli/src/send.rs](../../crates/cli/src/send.rs)
- [crates/cli/src/receive.rs](../../crates/cli/src/receive.rs)
- [crates/cli/src/protocol.rs](../../crates/cli/src/protocol.rs)
- [crates/cli/src/transfer.rs](../../crates/cli/src/transfer.rs)
- [crates/cli/src/lib.rs](../../crates/cli/src/lib.rs)
