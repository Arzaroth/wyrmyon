---
name: release
description: Cut a wyrmyon release, the whole shebang - docs sweep (CHANGELOG, README, brain, wire identifiers), then the version bump, gate, commit, annotated tag and push, then watch the Release workflow until the GitHub release, the PyPI wheels and the installers are out. Use when the user says "cut a release", "release", "tag vX.Y.Z", or "the whole release shebang".
---

# Release shebang

Everything between "the code is on master" and "users can install it". Pushing
the tag is what builds and publishes the release, so there is no undo once
step 3 runs: a bad tag means a new patch release, and a version on PyPI can
never be uploaded again.

## 0. Before the first release

The pipeline is described in `brain/architecture/distribution.md`. Two
things live outside the repository and must exist before the first tag, or the
Release workflow fails halfway through:

- **PyPI trusted publishers** for the `wyrmyon` project (pending publishers
  before the first upload): owner `Arzaroth`, repository `wyrmyon`,
  environment `pypi` (GitHub creates it on first use), and register it twice,
  once with workflow `publish-pypi.yml` and once with `release.yml`. The
  upload runs in the reusable `publish-pypi.yml` called from `release.yml`,
  and PyPI's support for reusable workflows is partial (it matches one of the
  two depending on the claim it reads), so both names keep it working.
  Without them the upload is refused after the GitHub release is out.
- **`Arzaroth/homebrew-tap`** on GitHub, and a `HOMEBREW_TAP_TOKEN` secret on
  `Arzaroth/wyrmyon` that can push to it.

Check them with the user before the first release; afterwards they stay.

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

```bash
scripts/release.sh <x.y.z> --dry-run   # shows the changelog section it will release
scripts/release.sh <x.y.z>
```

It refuses unless on a clean `master` level with `origin/master`, with
something under `[Unreleased]` and no `v<x.y.z>` tag yet. It moves
`[Unreleased]` into `## [x.y.z] - <UTC date>`, sets the workspace version
(and `Cargo.lock`), runs the gate, checks `wyrm --version` prints
`wyrmyon x.y.z`, then commits `[master] chore(release): x.y.z`, creates the
annotated tag `vx.y.z` and pushes both to `origin`; a failed gate undoes the
bump. The GitHub mirror syncs on commit; confirm the tag reached GitHub
(`gh api repos/Arzaroth/wyrmyon/git/refs/tags/vx.y.z`) before watching for the
workflow.

## 4. Watch the Release workflow

```bash
run=$(gh run list -R Arzaroth/wyrmyon --workflow Release --limit 1 --json databaseId --jq '.[0].databaseId')
gh run watch "$run" -R Arzaroth/wyrmyon --exit-status
gh release view v<x.y.z> -R Arzaroth/wyrmyon --json assets --jq '.assets[].name'
```

Read a failing job with `gh run view "$run" --log-failed`. A re-run executes
the workflow files and the code at the tag, so it only helps with a transient
failure (a runner, the network, PyPI being down): `gh run rerun "$run"
--failed`. The PyPI upload skips files already there, so re-running it
finishes a partial upload. Anything else is fixed on `master` and shipped as
the next patch release; never move or reuse a tag. If the build jobs failed,
nothing is public yet and the dead tag just stays unreleased; if `host` ran,
the GitHub release exists and the next patch supersedes it.

## 5. Verify it installs

```bash
uvx wyrmyon@<x.y.z> --version            # wyrmyon <x.y.z>
h=$(mktemp -d); curl -LsSf https://github.com/Arzaroth/wyrmyon/releases/download/v<x.y.z>/wyrmyon-installer.sh \
  | HOME=$h CARGO_HOME=$h/.cargo sh && $h/.cargo/bin/wyrm --version; rm -rf "$h"
```

and that `Arzaroth/homebrew-tap` received `Formula/wyrmyon.rb` for the new
version.

## Done when

The tag is pushed, the GitHub release has its archives and installers, PyPI
has the wheels, the tap has the formula, a
throwaway install reports the new version, and `CHANGELOG.md` has a dated
section for it. Report the version and the release URL.
