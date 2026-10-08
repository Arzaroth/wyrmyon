# TODO

Small, concrete work: fixes, chores, decisions to make before a roadmap item
starts. Features go in [ROADMAP.md](ROADMAP.md). Delete an entry when it is
done.

## Decisions before roadmap work

- [ ] M1: write the mailbox, SPAKE2 and transit code ourselves, or build on the
      `magic-wormhole` crate (magic-wormhole.rs, EUPL-1.2). Its licence and
      its async runtime decide most of it.
- [ ] M1: how the test suite gets a mailbox and a transit relay: the Python
      `magic-wormhole-mailbox-server` and `magic-wormhole-transit-relay` in a
      container, or a minimal in-process fake.
- [ ] M4: pin the iroh and iroh-blobs versions, and check which NodeAddr
      serialisation is stable enough to put on the wire.
- [ ] M4: what the iroh side does when hole-punching and n0's relays both fail:
      fall back to classic transit on the same wormhole, or report and stop.
- [ ] Pronunciation: one line in the README once settled ("WURM-yon").
