#!/usr/bin/env bash
# Usage: scripts/package.sh <target> <x.y.z>
#
# Packages the binaries cargo built for <target> (target/<target>/release)
# into target/dist: an archive for every target, and on Linux a .deb and an
# .rpm (needs nfpm on PATH). The archive holds both binaries, the man pages,
# the shell completions, the README, the changelog and the licence.

set -euo pipefail

target="${1:-}"
version="${2:-}"
if [[ -z $target || ! $version =~ ^[0-9]+\.[0-9]+\.[0-9]+([-+].*)?$ ]]; then
  echo "usage: scripts/package.sh <target> <x.y.z>" >&2
  exit 2
fi

cd "$(dirname "$(readlink -f "$0")")/.."

bin="target/$target/release"
name="wyrmyon-v$version-$target"
stage="target/stage/$name"
dist="target/dist"

exe=""
[[ $target == *windows* ]] && exe=".exe"
for b in wyrmyon wyrm; do
  if [[ ! -f $bin/$b$exe ]]; then
    echo "package: $bin/$b$exe is missing: build it first" >&2
    exit 1
  fi
done

rm -rf "$stage"
mkdir -p "$stage" "$dist"
cargo run -q --locked -p wyrmyon-xtask -- assets "$stage"
cp README.md CHANGELOG.md LICENSE "$stage/"
cp "$bin/wyrmyon$exe" "$stage/"
if [[ -n $exe ]]; then
  cp "$bin/wyrm$exe" "$stage/"
else
  ln -s wyrmyon "$stage/wyrm"
fi

case "$target" in
*windows*)
  rm -f "$dist/$name.zip"
  (cd target/stage && 7z a -tzip -bso0 -bsp0 "../dist/$name.zip" "$name")
  ;;
*)
  tar -czf "$dist/$name.tar.gz" -C target/stage "$name"
  ;;
esac

if [[ $target == *linux* ]]; then
  case "$target" in
  x86_64-*) arch=amd64 ;;
  aarch64-*) arch=arm64 ;;
  *)
    echo "package: no package architecture for $target" >&2
    exit 1
    ;;
  esac
  root="$PWD"
  for format in deb rpm; do
    (cd "$stage" && ARCH="$arch" VERSION="$version" \
      nfpm package --config "$root/packaging/nfpm.yaml" --packager "$format" --target "$root/$dist/")
  done
fi

ls -l "$dist"
