# TODO

Small, concrete work: fixes, chores, decisions to make before a roadmap item
starts. Features go in [ROADMAP.md](ROADMAP.md). Delete an entry when it is
done.

## Small work

- [ ] Tests for out-of-order and duplicate phases: needs a way to make the fake
      mailbox reorder deliveries.
- [ ] Mailbox reconnection after a dropped connection, as the Python client
      does.
- [ ] A directory from Linux holding names that differ only by case
      (`Makefile`, `makefile`) or Unicode normalization is refused whole on
      macOS and Windows (`create_new` fails on the second one); say which
      names collide, or rename the second.
- [ ] macOS keeps partial transfers in `~/.cache/wyrmyon/partial`; decide
      whether `~/Library/Caches` is worth the second convention.

## Decisions before roadmap work


- [ ] Pronunciation: one line in the README once settled ("WURM-yon").
