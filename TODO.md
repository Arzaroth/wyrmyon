# TODO

Small, concrete work: fixes, chores, decisions to make before a roadmap item
starts. Features go in [ROADMAP.md](ROADMAP.md). Delete an entry when it is
done.

## Small work

- [ ] Tests for out-of-order and duplicate phases: needs a way to make the fake
      mailbox reorder deliveries.
- [ ] Mailbox reconnection after a dropped connection, as the Python client
      does.
- [ ] Ctrl-C drops the send without releasing the nameplate or closing the
      mailbox. A `wyrm send --times` run spends most of its life waiting
      with a claimed nameplate, so a late receiver pairs with the dead
      sender and hangs. Release and close on Ctrl-C.
- [ ] A `--times` turn whose peer pairs and then goes silent stalls the
      run until Ctrl-C. Decide on a deadline that does not cut off a
      receiver still deciding whether to accept.
- [ ] A `--times` turn joins the code again right after pairing, which is
      `crowded` if the receiver releases only later; the Python client and
      wyrm release first, other clients are unchecked. Retry briefly on
      `crowded` if one does not.
- [ ] The interop test `one_wyrm_code_serves_python_receivers_back_to_back`
      fails now and then in a full run: a Python receiver gets `Connection
      refused` from its mailbox. It passes alone and touches nothing the
      branches around it changed; find out whether the mailbox server is
      still starting.
- [ ] `scripts/coverage.sh --lcov` fails: it writes to `target/llvm-cov/`
      without creating it.
- [ ] `a_closed_stdout_is_reported_to_the_sender` in `tests/peers.rs` hangs
      now and then (once for 35 minutes on macOS CI), waiting on a `wyrm`
      child that seems to have exited; it passes alone in seconds. Its waits
      now time out after 30 s, so it fails instead of hanging. Find out why.
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
- [ ] A transfer's iroh endpoint asks the gateway for a UPnP/PCP/NAT-PMP
      mapping with a two-hour lease, and `Endpoint::close` does not release
      it, so every transfer leaves one on the router. Check whether iroh
      can release it on close, or release it ourselves.
- [ ] macOS keeps partial transfers in `~/.cache/wyrmyon/partial`; decide
      whether `~/Library/Caches` is worth the second convention.

## Decisions before roadmap work


- [ ] Pronunciation: one line in the README once settled ("WURM-yon").
