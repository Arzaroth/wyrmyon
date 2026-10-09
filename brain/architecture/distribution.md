# Distribution

## CI

The toolchain is pinned: `rust-toolchain.toml` for CI (rustup on the runners
honours it) and `.mise.toml` for local work, both on the same version with
clippy and rustfmt, so a lint new in one release never fails CI only. Bump
both together.

`.github/workflows/ci.yml` runs on pull requests and pushes to `master`, on
x86_64 and aarch64 Linux, aarch64 macOS and x86_64 Windows: `cargo fmt --all
--check`, clippy with `-D warnings` (pedantic, from `[workspace.lints]`), and
`cargo test`, all `--locked`. A `coverage` job shellchecks `scripts/*.sh` and
runs `scripts/coverage.sh --check`: line coverage under 98% fails the build
([testing.md](testing.md)). Coverage is measured on Linux, so code behind
`#[cfg(not(unix))]` is built and tested by the Windows job but never counted.

The repository lives on Forgejo (`git.arzaroth.com/Arzaroth/wyrmyon`) and is
push-mirrored to GitHub (`github.com/Arzaroth/wyrmyon`). CI and releases run on
GitHub Actions; Forgejo Actions is off for this repository.

## Platforms

Releases target x86_64 and aarch64 on Linux (static musl), macOS and Windows.
The Linux binaries are statically linked against musl, so one binary runs on
any distribution, old glibc ones and Alpine included. The code is the same
everywhere except:

- File modes: Unix keeps the permission bits of sent files and applies the
  received ones; elsewhere a file is sent as `0o644` (`0o444` if read-only)
  and received with the platform's defaults.
- The partial-transfer cache: `%LOCALAPPDATA%\wyrmyon\partial` on Windows
  ([features/resume.md](../features/resume.md)).
- Received names: on Windows a file or zip entry name Windows cannot hold is
  refused ([features/files.md](../features/files.md)).
- Tests that need a pty, Unix permissions, symlinks or the Python servers
  (the interop suite) run on Unix only ([testing.md](testing.md)).

## Release artefacts

| Artefact | For | Built by |
| --- | --- | --- |
| `wyrmyon-vX.Y.Z-<target>.tar.gz` (`.zip` on Windows) | every target | `scripts/package.sh` |
| `wyrmyon_X.Y.Z_{amd64,arm64}.deb`, `wyrmyon-X.Y.Z-1.{x86_64,aarch64}.rpm` | Debian, Ubuntu, Fedora, RHEL, openSUSE... | nfpm, `packaging/nfpm.yaml` |
| `wyrmyon-bin-X.Y.Z-1-{x86_64,aarch64}.pkg.tar.zst` | Arch (`pacman -U`) | makepkg, `packaging/arch/wyrmyon-bin` |
| `wyrmyon-vX.Y.Z-pkgbuild.tar.gz` | the AUR, later | `scripts/arch.sh` |
| `wyrmyon-vX.Y.Z-<target>.msi` | Windows x64 and arm64 | WiX 5, `packaging/windows/wyrmyon.wxs` |
| `SHA256SUMS` | the install scripts, anyone checking | the workflow |

Every archive and package carries both binaries, a man page per binary and
subcommand, and bash, zsh and fish completions (PowerShell and elvish in the
archives too), generated from the clap definition by
`cargo run -p wyrmyon-xtask -- assets DIR`. `wyrm` is a symlink to `wyrmyon`
everywhere but Windows, where it is a second copy. The packages put the man
pages and completions where each distribution looks for them (zsh's are in
`vendor-completions` on Debian, `site-functions` elsewhere).

The Arch side has two PKGBUILDs: `wyrmyon` builds from the release's source
tarball (with `!lto`: makepkg's LTO flags turn ring's C objects into GCC
bitcode that Rust's lld cannot link), `wyrmyon-bin` repackages the musl
tarballs. The committed copies keep `sha256sums=('SKIP')`, since a PKGBUILD
cannot checksum the tarball it lives in; the release attaches rendered copies
with real sums and their `.SRCINFO`, ready for the AUR, and the `-bin`
packages built from them.

The MSI is per-user and unsigned (SmartScreen warns): it installs into
`%LOCALAPPDATA%\Programs\wyrmyon`, the directory `install.ps1` uses, and adds
it to the user PATH, both undone on uninstall.

macOS gets the tarballs and `install.sh`. A `.pkg` or `.dmg` would need a
Developer ID signature and notarization to open without a Gatekeeper
warning; a tarball fetched with curl carries no quarantine flag, so the
install script works unsigned.

## Install scripts

`scripts/install.sh` (Linux, macOS) and `scripts/install.ps1` (Windows) are
fetched from the repository and download the archive for the machine from the
latest release (or `--version` / `WYRMYON_VERSION`), checked against
`SHA256SUMS`. `install.sh` installs into `~/.local/bin` (or `--dir`), with the
man pages and completions under `~/.local/share` and `~/.config/fish`;
`install.ps1` into `%LOCALAPPDATA%\Programs\wyrmyon`, added to the user PATH.
`WYRMYON_DOWNLOAD_BASE` points both at another location, a directory or a
`file://` URL, which is how the workflow tests them.

## The release workflow

`.github/workflows/release.yml`, on a `vX.Y.Z` tag (or `workflow_dispatch`
for an existing tag):

1. **build**, per target on a native runner (x86_64 macOS is cross-built on
   the arm64 one and not tested): checks the tag matches the workspace
   version, runs the tests for the target, builds, checks `--version`, and
   runs `scripts/package.sh`, then WiX on Windows.
2. **arch**, in an `archlinux` container: `scripts/arch.sh bin` and
   `scripts/arch.sh source` against GitHub's tarball of the tag.
3. **checksums**: `SHA256SUMS` over everything.
4. **verify**: the `.deb` on Debian 11, the `.rpm` on Fedora, the Arch
   package (x86_64, there is no arm64 Arch image), `install.sh` on Linux and
   macOS, `install.ps1` and an MSI install and uninstall on Windows, on both
   architectures where a runner exists.
5. **release**: the GitHub release, its notes taken from the version's
   `CHANGELOG.md` section, refused if there is none.

Pull requests that touch packaging run steps 1 to 4 (with the source PKGBUILD
built from `git archive HEAD`), so the pipeline is tested before a tag needs
it. A failed tag run is fixed on `master` and shipped as the next patch: a
re-run builds the tag's own files.

`scripts/release.sh X.Y.Z` cuts a release: it dates the `[Unreleased]`
changelog section, bumps the version in the workspace and the PKGBUILDs, runs
the gate, checks `wyrm --version`, commits, tags and pushes master and the
tag atomically ([the release skill](../../.claude/skills/release/SKILL.md)).

## Not yet published

PyPI wheels, a Homebrew formula and the AUR. The PyPI groundwork stays:
`pyproject.toml` builds the wheels with **maturin** (`bindings = "bin"`), and
CI builds the x86_64 manylinux wheel on every pull request and checks it
carries both binaries; the publishing workflows were removed in commit
`7a0a02a` and need the trusted publisher registered under both workflow
names. The crates are not published to crates.io (`publish = false`) and
depend on each other by path only, so the version lives once, in the
workspace `Cargo.toml`. The package name is always `wyrmyon`; `wyrm` is taken
on crates.io and PyPI.

## Sources

- [.github/workflows/ci.yml](../../.github/workflows/ci.yml)
- [.github/workflows/release.yml](../../.github/workflows/release.yml)
- [scripts/package.sh](../../scripts/package.sh)
- [scripts/arch.sh](../../scripts/arch.sh)
- [scripts/install.sh](../../scripts/install.sh)
- [scripts/install.ps1](../../scripts/install.ps1)
- [packaging/nfpm.yaml](../../packaging/nfpm.yaml)
- [packaging/arch/wyrmyon/PKGBUILD](../../packaging/arch/wyrmyon/PKGBUILD)
- [packaging/arch/wyrmyon-bin/PKGBUILD](../../packaging/arch/wyrmyon-bin/PKGBUILD)
- [packaging/windows/wyrmyon.wxs](../../packaging/windows/wyrmyon.wxs)
- [crates/xtask/src/main.rs](../../crates/xtask/src/main.rs)
- [pyproject.toml](../../pyproject.toml)
- [scripts/release.sh](../../scripts/release.sh)
- [crates/cli/src/zipdir.rs](../../crates/cli/src/zipdir.rs)
- [rust-toolchain.toml](../../rust-toolchain.toml)
- [.mise.toml](../../.mise.toml)
- [Cargo.toml](../../Cargo.toml)
- [crates/cli/Cargo.toml](../../crates/cli/Cargo.toml)
