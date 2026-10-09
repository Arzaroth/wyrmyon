---
name: feature-treatment
description: Ship a finished wyrmyon feature branch the full way - rebase onto master, run a max-effort multi-agent code review, fix everything confirmed, update the brain/ knowledge base, pass the gate and CI, merge keeping the layered commits, and cut a release when a milestone closes. Use when the user says "feature treatment", "treat this branch", "review and merge this feature", or names a worktree/branch to ship.
---

# Feature treatment

The pipeline a finished feature branch goes through before it lands: **rebase ->
max review -> fix -> update brain -> gate -> merge -> release -> clean up**.
Nothing merges without an adversarial review and a green gate.

The branch comes from the argument (a branch or worktree name). If none is
given, find it: `wt list` and `git branch` - it is the non-`master` one;
worktrees live in `~/repos/wyrmyon.worktrees/<branch>`. Confirm which one if
ambiguous. Branch names follow `feature/...`, `fix/...`; every commit subject
starts with `[<branch>]`.

## 1. Rebase onto master

```bash
cd ~/repos/wyrmyon.worktrees/<branch>     # or the primary checkout if the branch is there
git fetch origin && git rebase origin/master
```

Resolve conflicts if any (the **resolving-merge-conflicts** skill). The branch
must sit directly on `master` so the review and the merge see only this feature.
Check the size: `git diff --name-only origin/master..HEAD | wc -l` must stay under
149; split along a seam if it does not.

## 2. Max-effort review (parallel finders)

The diff: `git diff origin/master...HEAD` (exclude `Cargo.lock` from reading; a
dependency change is reviewed from the `Cargo.toml` files).

Spawn **independent finder subagents in parallel** (one Agent tool call with
several invocations), each over the same diff with a different lens. Scale the
count to the feature (4-6 is typical):

- **Correctness** - line by line: inverted conditions, off-by-one, `unwrap` on
  data from the network or disk, error paths that leave a half-written file,
  state machine transitions (mailbox, transit handshake, offer and answer) that
  can deadlock or accept a message out of order, clap flag defaults and
  conflicts.
- **Crypto and protocol security** - the rules in `CLAUDE.md`: is the transport
  decided only from the PAKE-encrypted version message? Is the peer's node ID
  pinned and the channel binding checked before any data is trusted? Are keys
  derived with the right HKDF labels and never reused across purposes? Any code,
  key or nonce in a log line, error message or spawned command line? Nonces
  never repeated; comparisons of MACs constant-time.
- **Interop and mailbox etiquette** - does every message the mailbox or a legacy
  peer can see match the magic-wormhole spec and what the Python client sends?
  Nothing nonstandard before the peer advertises `iroh-v1`? Official appid,
  welcome message surfaced, no file data through the mailbox? Wire identifiers
  (`app_versions` key, `iroh-v1`, ALPN, HKDF labels) unchanged, or changed with
  a plan for older peers?
- **Resource safety** - received paths cannot escape the target directory
  (`..`, absolute paths, symlinks in a zip), sizes from the peer are capped
  before allocation, partial files are not left looking complete, connections
  and tasks are cancelled on every exit path.
- **Reuse / simplify** - does new code re-implement something in
  `wormhole-core`? Transport-specific logic leaking into the CLI instead of
  staying behind the `Transport` trait? Crypto done outside `wormhole-core`?
- **Tests** - do the tests cover the new branches: a wrong code, a peer that
  disconnects mid-transfer, a legacy peer, a refused offer, a malformed
  message? Does any test reach a public server (it must not; see
  `brain/architecture/testing.md`)?

Each finder returns findings as JSON objects `{file, line, severity, summary,
failure_scenario}`, verified (quote the line), most-severe first. Tell them NOT
to fix anything. Then optionally run one **sweep** finder that has the merged
list and hunts only for gaps.

## 3. Fix what's confirmed

Triage the findings. Re-verify a claim before fixing it - finders surface
plausible-but-wrong items too. Fix every confirmed correctness, security and
interop issue, and the worthwhile quality ones, as new commits on top
(`[<branch>] fix(<area>): ...`), never by rewriting the reviewed layers. For
anything real but out of scope, add it to `TODO.md` or open a GitHub issue
(`gh issue create -R Arzaroth/wyrmyon`) rather than dropping it.

## 4. Update the brain

Bring `brain/` in line with what the feature changed - it ships in the SAME
merge, not as a follow-up. Apply the **brain** skill (it has the checklist); the
essentials:

- A **Planned** doc the feature built now describes the code, with `## Sources`.
- New command or behaviour -> `brain/features/<name>.md` plus the rows in
  `brain/features/index.md` and `brain/BRAIN.md`.
- New non-obvious decision -> `brain/decisions.md` with its why. New term ->
  `brain/glossary.md`.
- `ROADMAP.md`: tick the items the branch delivered. `TODO.md`: drop the
  decisions it settled.
- `CHANGELOG.md` `[Unreleased]` has an entry for every user-facing change;
  README's usage matches `wyrm --help`.

Commit as `[<branch>] docs: ...`, after checking the brain's relative links
resolve.

## 5. Gate (must be green before merge)

```bash
cargo fmt --all --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
scripts/coverage.sh                  # at or above the 98% floor, and look for a logic file drifting down the list
```

Every commit must pass on its own, not just the tip; check from clean exports
(a stale `target/` hides a broken layer):

```bash
w=$(mktemp -d); export CARGO_TARGET_DIR="$w-target"
for c in $(git rev-list --reverse origin/master..HEAD); do
  rm -rf "$w"/* && git archive "$c" | tar -x -m -C "$w"
  (cd "$w" && cargo clippy -q --all-targets --locked -- -D warnings && cargo test -q --locked) \
    >/dev/null 2>&1 && echo "ok   $(git log -1 --format=%s "$c")" || echo "FAIL $(git log -1 --format=%s "$c")"
done
rm -rf "$w" "$w-target"; unset CARGO_TARGET_DIR
```

If the feature touched the wire, run the interop suite against the Python
`wormhole` CLI (`brain/architecture/testing.md`), and transfer something
between two `wyrm` processes by hand.

## 6. Push and let CI run

`origin` is Forgejo; GitHub is a push mirror of it, and CI runs on GitHub.
Push the branch to Forgejo, then open the PR on GitHub once the mirror has it
(sync on commit, usually seconds):

```bash
git push --force-with-lease origin <branch>   # the rebase rewrote it
gh pr create -R Arzaroth/wyrmyon --head <branch> ... || gh pr edit ...
gh pr checks --watch -R Arzaroth/wyrmyon
```

CI runs fmt, clippy and tests on Linux (x86_64, aarch64), macOS (aarch64) and
Windows (x86_64), builds the manylinux wheel, and checks the coverage floor.
It must be green.

## 7. Merge (keep the layers)

Fast-forward `master` onto the branch rather than squashing, so the layers survive:

```bash
cd ~/repos/wyrmyon
git switch master && git pull --ff-only
git merge --ff-only <branch>
git push origin master                         # the mirror carries it; GitHub marks the PR merged
```

## 8. Release

When the branch closes a roadmap milestone, run the
**release** skill: minor for a user-facing feature, patch for fix-only.
Otherwise leave the changes in `[Unreleased]`.

## 9. Clean up

```bash
wt remove <branch>                            # worktree and branch
git push origin --delete <branch>
```

## Done when

The feature is on `master`, the brain reflects it, every layer and CI were green,
the branch and worktree are gone, and a release is cut if a milestone closed.
Report a one-line summary of what the review fixed.
