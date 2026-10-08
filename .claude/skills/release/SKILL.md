---
name: release
description: Cut a wyrmyon release, the whole shebang - docs sweep (CHANGELOG, README, brain, wire identifiers), then the version bump, gate, commit, annotated tag and push, then watch the Release workflow until the GitHub release, the PyPI wheels and the installers are out. Use when the user says "cut a release", "release", "tag vX.Y.Z", or "the whole release shebang".
---

# Release shebang

Everything between "the code is on master" and "users can install it". Pushing
the tag is what builds and publishes the release, so there is no undo once
step 3 runs: a bad tag means a new patch release, and a version on PyPI can
never be uploaded again.

## 0. Is there a release pipeline yet?

Packaging is roadmap milestone M7 (maturin wheels, cargo-dist archives and
installers). If `dist-workspace.toml`, `pyproject.toml` or
`.github/workflows/release.yml` is missing, there is nothing to release: stop
and say so. When M7 lands, it fills in steps 3 to 5 below with the real
commands and adds `scripts/release.sh`.

## 1. Pre-flight

- On `master`, working tree clean, level with `origin/master` (Forgejo).
  Stash unrelated dirty files and restore them afterwards.
- Anything still on a feature branch is not in this release: finish it first
  (the **feature-treatment** skill) or leave it out on purpose.
- Last version: `git describe --tags --abbrev=0` (tags are `vX.Y.Z`; before the
  first release there is none). Pick the next semver: minor for a new command,
  transport or user-visible behaviour, patch for fix-only. While on 0.x, a
  change that breaks talking to older wyrmyon peers is a minor bump that says
  so in the changelog.

## 2. Docs sweep (review against `git diff v<last>..HEAD`)

- **CHANGELOG.md `[Unreleased]`** covers every user-visible change since the
  last tag. Entries are added per branch, so this is a completeness check
  against `git log v<last>..HEAD --oneline`. That section becomes the release
  notes verbatim.
- **README.md**: usage, install commands, the status line.
- **brain/**: every doc a change made wrong is fixed (the **brain** skill has
  the checklist). The features and architecture indexes list what shipped.
- **Wire identifiers**: `git diff v<last>..HEAD` touching the `app_versions`
  key, `iroh-v1`, the ALPN or the HKDF labels needs a changelog line on
  compatibility with older peers.
- **Dependencies**: `git diff v<last>..HEAD -- '*Cargo.toml'`. A new dependency
  must build on every release target (anything linking C is the risk).
- Commit the sweep as `[master] docs(release): ...`.

## 3. Cut it

Move `[Unreleased]` into `## [x.y.z] - <UTC date>`, set `version` in the
workspace `Cargo.toml` (and `Cargo.lock`), run the gate (`cargo fmt --all
--check`, clippy `-D warnings`, `cargo test --locked`), check
`cargo run -q --bin wyrm -- --version` prints `wyrmyon x.y.z`, then commit
`[master] chore(release): x.y.z`, create annotated tag `vx.y.z`, and push both
to `origin`. The GitHub mirror syncs on commit; confirm the tag reached GitHub
(`gh api repos/Arzaroth/wyrmyon/git/refs/tags/vx.y.z`) before watching for the
workflow.

## 4. Watch the Release workflow

```bash
run=$(gh run list -R Arzaroth/wyrmyon --workflow Release --limit 1 --json databaseId --jq '.[0].databaseId')
gh run watch "$run" -R Arzaroth/wyrmyon --exit-status
gh release view v<x.y.z> -R Arzaroth/wyrmyon --json assets --jq '.assets[].name'
```

A failed run leaves a pushed tag without a release: read the failing job
(`gh run view "$run" --log-failed`), fix on `master`, and re-run the workflow
for the same tag rather than moving the tag.

## 5. Verify it installs

```bash
uvx wyrmyon@<x.y.z> --version            # wyrmyon <x.y.z>
```

and the shell installer into a throwaway `HOME`.

## Done when

The tag is pushed, the GitHub release has its archives, PyPI has the wheels, a
throwaway install reports the new version, and `CHANGELOG.md` has a dated
section for it. Report the version and the release URL.
