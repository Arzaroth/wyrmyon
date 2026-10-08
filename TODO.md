# TODO

Small, concrete work: fixes, chores, decisions to make before a roadmap item
starts. Features go in [ROADMAP.md](ROADMAP.md). Delete an entry when it is
done.

## Decisions before roadmap work

- [ ] M3: the test transit relay: extend `crates/testkit` with an in-process
      fake, as for the mailbox, and use the Python
      `magic-wormhole-transit-relay` in the interop suite.
- [ ] M4: pin the iroh and iroh-blobs versions, and check which NodeAddr
      serialisation is stable enough to put on the wire.
- [ ] M4: what the iroh side does when hole-punching and n0's relays both fail:
      fall back to classic transit on the same wormhole, or report and stop.
- [ ] Pronunciation: one line in the README once settled ("WURM-yon").
