#!/usr/bin/env bash
# Usage: scripts/arch.sh bin <x.y.z>
#        scripts/arch.sh source <x.y.z> <source tarball>
#
# Runs on Arch Linux as a regular user (makepkg refuses root), with
# base-devel and pacman-contrib installed, plus cargo for `source`.
#
# bin: renders packaging/arch/wyrmyon-bin for the musl tarballs that
# scripts/package.sh left in target/dist, and builds the x86_64 and aarch64
# packages from it. source: renders packaging/arch/wyrmyon for the given
# source tarball (the release's, or a `git archive` of the tree being tested)
# and builds it for this machine. Either way, the packages and the rendered
# PKGBUILD and .SRCINFO land in target/dist.

set -euo pipefail

mode="${1:-}"
version="${2:-}"
if [[ ! $version =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] ||
  [[ $mode == bin && $# -ne 2 ]] || [[ $mode == source && $# -ne 3 ]] ||
  [[ $mode != bin && $mode != source ]]; then
  echo "usage: scripts/arch.sh bin <x.y.z> | source <x.y.z> <tarball>" >&2
  exit 2
fi
tarball=""
[[ $mode == source ]] && tarball="$(readlink -f "$3")"

cd "$(dirname "$(readlink -f "$0")")/.."
dist="$PWD/target/dist"
pkgname=wyrmyon
[[ $mode == bin ]] && pkgname=wyrmyon-bin
work="$PWD/target/arch/$pkgname"
rendered="$dist/arch/$pkgname"

rm -rf "$work" "$rendered"
mkdir -p "$work" "$rendered"
sed "s/^pkgver=.*/pkgver=$version/" "packaging/arch/$pkgname/PKGBUILD" >"$work/PKGBUILD"

if [[ $mode == bin ]]; then
  for arch in x86_64 aarch64; do
    cp "$dist/wyrmyon-v$version-$arch-unknown-linux-musl.tar.gz" "$work/"
  done
else
  cp "$tarball" "$work/wyrmyon-$version.tar.gz"
fi

cd "$work"
updpkgsums
makepkg --printsrcinfo >.SRCINFO
cp PKGBUILD .SRCINFO "$rendered/"

PKGDEST="$dist" makepkg --force --noconfirm
if [[ $mode == bin ]]; then
  sed -e 's/^CARCH=.*/CARCH="aarch64"/' -e 's/^CHOST=.*/CHOST="aarch64-unknown-linux-gnu"/' \
    /etc/makepkg.conf >makepkg-aarch64.conf
  PKGDEST="$dist" makepkg --force --noconfirm --config makepkg-aarch64.conf
fi
ls -l "$dist"/*.pkg.tar.zst
