# Files

```
wyrm send report.pdf
wyrm receive 7-guitarist-revenge              # asks before accepting
wyrm receive --accept-file -o ~/in/r.pdf CODE # no question, chosen path
```

One regular file per transfer, with any magic-wormhole client. Directories
are refused for now (M3), and so are pipes, devices and anything else whose
size the sender cannot know.

## Sender

1. Pairs, then sends `{"transit": <hints>}` and
   `{"offer": {"file": {"filename", "filesize"}}}`.
2. Waits for the receiver's `transit` and `{"answer": {"file_ack": "ok"}}`.
   An `error` or any other answer fails the transfer.
3. Connects ([transit-classic.md](../architecture/transit-classic.md)) and
   streams the file in 64 KiB records, hashing it with SHA-256. A file that
   grows or shrinks during the send fails it.
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
