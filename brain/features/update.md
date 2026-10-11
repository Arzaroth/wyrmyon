# Updating itself

```
wyrm --update                  # replace this copy with the latest release
wyrm --update --hide-progress  # without the download bar
```

`--update` takes no subcommand and no targets. It is for a copy installed
from a release archive or `install.sh` / `install.ps1`: anything a package
manager installed is refused, before anything is fetched, with how to upgrade
it instead.

## Who installed it

`selvedge::update::owner` looks at where the running binary resolves to:

| Where | Owner | Told to |
| --- | --- | --- |
| under `/usr`, outside `/usr/local` (the .deb, .rpm and Arch packages) | the distribution, named by the dpkg, rpm or pacman database found | upgrade through the package manager |
| a Homebrew `Cellar` | Homebrew | `brew upgrade wyrmyon` |
| `/nix/store`, `/snap` | Nix, snap | upgrade through them |
| a `.app/Contents/` | an app bundle | download the release |
| Windows, `HKCU\Software\wyrmyon\ProductCode` present | the MSI | download the new `.msi` |

The MSI and `install.ps1` install into the same directory, so the path cannot
tell them apart; the MSI writes `ProductCode` beside its PATH entry and takes
both away on uninstall. Anywhere else (`~/.local/bin`, a directory a tarball
was unpacked into, `cargo install`'s `~/.cargo/bin`) is the user's, and gets
updated.

## The update

1. The newest GitHub release of `Arzaroth/wyrmyon` (`WYRMYON_REPO` names a
   fork) carrying this platform's archive: the asset whose name holds the
   target triple the release workflow builds (`x86_64-unknown-linux-musl`,
   `aarch64-apple-darwin`, `x86_64-pc-windows-msvc`...). No newer release:
   `wyrmyon X.Y.Z is the latest release`, nothing touched.
2. Under a lock on `.wyrmyon-update.lock` in the install directory, the
   archive is downloaded and unpacked into `.wyrmyon-update.tmp` there, and
   the binaries are taken from its `wyrmyon-vX.Y.Z-<target>/` directory.
3. `wyrmyon` (and `wyrm.exe` on Windows) replace the installed ones, the old
   copy renamed aside so the running process keeps working; on Unix `wyrm`
   stays a symlink to `wyrmyon`.
4. `update.json` in the cache root (`~/.cache/wyrmyon`,
   `%LOCALAPPDATA%\wyrmyon`) records the version now installed.

Only the binaries change: the man pages and completions `install.sh` put
under `~/.local/share` stay at the version they came with until the script
runs again.

The download goes over rustls with ring, the provider iroh already links;
selvedge's `updater` feature leaves the provider to wyrmyon, so aws-lc is not
built. The archive is not checked against `SHA256SUMS` the way the install
scripts check it: it comes over TLS from GitHub's API.

## Sources

- [crates/cli/src/update.rs](../../crates/cli/src/update.rs) (`WYRMYON`, `run`)
- [crates/cli/src/lib.rs](../../crates/cli/src/lib.rs) (`Cli::update`, `cache_root_from`)
- [packaging/windows/wyrmyon.wxs](../../packaging/windows/wyrmyon.wxs)
- [selvedge](https://github.com/Arzaroth/selvedge) (`update::owner`, `update::apply`)
