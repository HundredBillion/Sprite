#!/bin/sh
# Builds Sprite from this checkout and installs it as Sprite.app.
#
#   packaging/macos/update.sh
#
# The macOS counterpart of ../update.sh, and for the same reason: a rebuild
# without a re-bundle installs the old binary. Four steps in one command —
# build, generate the terminfo source from the pinned Ghostty, bundle, install
# — then one symlink so the `sprite` command keeps working for the agents and
# scripts that read panes through it. One binary, reached two ways; two copies
# would drift apart after the next rebuild.
#
# /Applications is writable by an administrator without sudo. /usr/local/bin is
# not, so the link is the one step that asks for a password.
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$root"

app=/Applications/Sprite.app
link=/usr/local/bin/sprite

echo "==> building"
cargo build --release -p sprite-app --locked --offline

echo "==> generating terminfo from the pinned Ghostty"
export ZIG_GLOBAL_CACHE_DIR="$PWD/target/zig-global-cache"
export ZIG_LOCAL_CACHE_DIR="$PWD/target/zig-local-cache"
zig build-exe -lc -femit-bin=target/gen-terminfo \
    --dep ghostty_terminfo \
    -Mroot=scripts/gen-terminfo.zig \
    -Mghostty_terminfo=vendor/ghostty/src/terminfo/ghostty.zig
./target/gen-terminfo > target/ghostty.terminfo

echo "==> bundling"
packaging/macos/bundle.sh

echo "==> installing to $app"
rm -rf "$app"
ditto target/Sprite.app "$app"

echo "==> linking $link (sudo)"
if [ -e "$link" ] && [ ! -L "$link" ]; then
    echo "    replacing the regular file at $link with a link into the bundle"
fi
sudo ln -sfn "$app/Contents/MacOS/sprite" "$link"

echo "==> installed: $app; $(sprite --version 2>/dev/null | head -1) at $link"
