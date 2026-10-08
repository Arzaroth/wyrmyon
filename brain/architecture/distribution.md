# Distribution

## CI

The toolchain is pinned: `rust-toolchain.toml` for CI (rustup on the runners
honours it) and `.mise.toml` for local work, both on the same version with
clippy and rustfmt, so a lint new in one release never fails CI only. Bump
both together.

`.github/workflows/ci.yml` runs on pull requests and pushes to `master`, on
x86_64 and aarch64 Linux: `cargo fmt --all --check`, clippy with
`-D warnings` (pedantic, from `[workspace.lints]`), and `cargo test`, all
`--locked`.

The repository lives on Forgejo (`git.arzaroth.com/Arzaroth/wyrmyon`) and is
push-mirrored to GitHub (`github.com/Arzaroth/wyrmyon`). CI and releases run on
GitHub Actions; Forgejo Actions is off for this repository.

## Packaging (Planned, M7)

- **maturin** with `bindings = "bin"` publishes the native binary as wheels on
  PyPI, so `uvx wyrmyon` and `pipx install wyrmyon` work without a Python
  runtime, the way ruff and uv ship.
- **cargo-dist** builds the GitHub release archives, shell and PowerShell
  installers, and a Homebrew tap.

Every artefact carries both binaries, `wyrmyon` and `wyrm`. The package name is
always `wyrmyon`; `wyrm` is taken on crates.io and PyPI.

## Sources

- [.github/workflows/ci.yml](../../.github/workflows/ci.yml)
- [rust-toolchain.toml](../../rust-toolchain.toml)
- [.mise.toml](../../.mise.toml)
- [Cargo.toml](../../Cargo.toml)
- [crates/cli/Cargo.toml](../../crates/cli/Cargo.toml)
