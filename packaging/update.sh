#!/bin/sh
# Builds Sprite from this checkout and installs it as a tracked package.
#
#   packaging/update.sh
#
# The three steps the local recipe describes, in one command, because doing
# them by hand is easy to get half-right: a rebuild without a repackage
# installs the old binary, and a repackage without `-C` can install a binary
# you have since replaced.
#
# Deliberately not a distribution recipe. `PKGBUILD` beside this builds from a
# clean checkout, which is what a repository would use. This packages the tree
# as it stands, so what gets installed is what you just tested.
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"

echo "==> building"
cargo build --release -p sprite-app --locked --offline

echo "==> packaging"
cd packaging
makepkg -p PKGBUILD.local -fC >/dev/null

echo "==> installing (sudo)"
sudo pacman -U --noconfirm "sprite-$(. ./PKGBUILD.local; echo "$pkgver-$pkgrel")-$(uname -m).pkg.tar.zst"

echo "==> installed: $(command -v sprite) $(sprite --version 2>/dev/null | head -1)"
