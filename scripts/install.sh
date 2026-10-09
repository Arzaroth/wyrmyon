#!/bin/sh
# Usage: curl -fsSL https://raw.githubusercontent.com/Arzaroth/wyrmyon/master/scripts/install.sh | sh
#        ... | sh -s -- [--version vX.Y.Z] [--dir DIR]
#
# Installs wyrmyon and wyrm from a GitHub release into ~/.local/bin (or DIR),
# with their man pages and bash, zsh and fish completions under ~/.local/share
# and ~/.config/fish. Linux and macOS, x86_64 and arm64.

set -eu

repo="${WYRMYON_REPO:-Arzaroth/wyrmyon}"
base="${WYRMYON_DOWNLOAD_BASE:-}"
bindir="$HOME/.local/bin"
data="${XDG_DATA_HOME:-$HOME/.local/share}"
config="${XDG_CONFIG_HOME:-$HOME/.config}"
tag=""

die() {
  echo "install: $*" >&2
  exit 1
}

while [ $# -gt 0 ]; do
  case "$1" in
  --version)
    [ $# -ge 2 ] || die "--version needs a tag"
    tag="$2"
    case "$tag" in v*) ;; *) tag="v$tag" ;; esac
    shift
    ;;
  --dir)
    [ $# -ge 2 ] || die "--dir needs a directory"
    bindir="$2"
    shift
    ;;
  -h | --help)
    echo "usage: install.sh [--version vX.Y.Z] [--dir DIR]"
    exit 0
    ;;
  *) die "unknown option $1" ;;
  esac
  shift
done

case "$(uname -s)" in
Linux) os=unknown-linux-musl ;;
Darwin) os=apple-darwin ;;
*) die "no release for $(uname -s); on Windows use install.ps1" ;;
esac
case "$(uname -m)" in
x86_64 | amd64) arch=x86_64 ;;
aarch64 | arm64) arch=aarch64 ;;
*) die "no release for $(uname -m)" ;;
esac

if [ -z "$tag" ] && [ -z "$base" ]; then
  latest=$(curl -fsSLI -o /dev/null -w '%{url_effective}' "https://github.com/$repo/releases/latest")
  tag="${latest##*/}"
  case "$tag" in v*) ;; *) die "could not find the latest release of $repo" ;; esac
fi
[ -n "$base" ] || base="https://github.com/$repo/releases/download/$tag"

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT INT TERM

curl -fsSL "$base/SHA256SUMS" -o "$tmp/SHA256SUMS"
name=$(sed -n "s/^[0-9a-f]*  \(wyrmyon-v.*-$arch-$os\)\.tar\.gz$/\1/p" "$tmp/SHA256SUMS")
[ -n "$name" ] || die "no $arch-$os archive in $base"
archive="$name.tar.gz"

echo "Downloading $archive"
curl -fsSL "$base/$archive" -o "$tmp/$archive"

expected=$(grep "  $archive\$" "$tmp/SHA256SUMS" | cut -d' ' -f1)
if command -v sha256sum >/dev/null 2>&1; then
  actual=$(sha256sum "$tmp/$archive" | cut -d' ' -f1)
else
  actual=$(shasum -a 256 "$tmp/$archive" | cut -d' ' -f1)
fi
if [ -z "$expected" ] || [ "$expected" != "$actual" ]; then
  die "$archive does not match its SHA256SUMS entry"
fi

tar -xzf "$tmp/$archive" -C "$tmp"
src="$tmp/$name"
"$src/wyrmyon" --version >/dev/null 2>&1 || die "the downloaded binary does not run on this system"

mkdir -p "$bindir" "$data/man/man1" "$data/bash-completion/completions" \
  "$data/zsh/site-functions" "$config/fish/completions"
install -m 755 "$src/wyrmyon" "$bindir/wyrmyon"
ln -sf wyrmyon "$bindir/wyrm"
install -m 644 "$src"/man/*.1 "$data/man/man1/"
for bin in wyrmyon wyrm; do
  install -m 644 "$src/completions/$bin.bash" "$data/bash-completion/completions/$bin"
  install -m 644 "$src/completions/_$bin" "$data/zsh/site-functions/_$bin"
  install -m 644 "$src/completions/$bin.fish" "$config/fish/completions/$bin.fish"
done

echo "Installed $("$bindir/wyrmyon" --version) into $bindir"
case ":$PATH:" in
*":$bindir:"*) ;;
*) echo "Add $bindir to your PATH to run wyrm." ;;
esac
echo "zsh completions are in $data/zsh/site-functions: add it to fpath if it is not there."
