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
- [ ] `wyrm --update` takes a copy installed from a PyPI wheel (pip, pipx,
      uvx) for a hand-installed one and replaces it in place; detect the
      wheel's install before PyPI publishing starts.
- [ ] `wyrm --update` on Windows replaces `wyrmyon.exe` then `wyrm.exe`
      with no rollback: if the second fails, the two stay a version apart
      and the next run says up to date. Roll back in selvedge.
- [ ] `wyrm --update` trusts the archive on TLS alone; check it against
      `SHA256SUMS`, or sign releases, as the install scripts check them.
- [ ] `wyrm --update` would take a prerelease as the latest release; none is
      published yet.
- [ ] macOS keeps partial transfers in `~/.cache/wyrmyon/partial`; decide
      whether `~/Library/Caches` is worth the second convention.

## Decisions before roadmap work


- [ ] Pronunciation: one line in the README once settled ("WURM-yon").
