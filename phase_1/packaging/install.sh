#!/bin/sh
# Installs a built Sprite into a prefix.
#
# Used by the Arch recipe's package() and usable by hand, so a distribution
# package and a manual install put the same files in the same places. It builds
# nothing: everything it installs must already exist.
#
#   DESTDIR=/tmp/stage PREFIX=/usr packaging/install.sh
#
# The terminfo database is compiled here rather than copied, because `tic`
# writes a directory tree whose layout is the local ncurses implementation's
# business, not Sprite's.
set -eu

PREFIX=${PREFIX:-/usr}
DESTDIR=${DESTDIR:-}
BINARY=${BINARY:-target/release/sprite}
TERMINFO_SOURCE=${TERMINFO_SOURCE:-target/ghostty.terminfo}

for required in "$BINARY" "$TERMINFO_SOURCE"; do
    if [ ! -f "$required" ]; then
        echo "install.sh: $required does not exist; build it first" >&2
        exit 1
    fi
done

root="$DESTDIR$PREFIX"

install -Dm755 "$BINARY"                     "$root/bin/sprite"
install -Dm644 packaging/sprite.desktop      "$root/share/applications/sprite.desktop"
install -Dm644 packaging/sprite.svg          "$root/share/icons/hicolor/scalable/apps/sprite.svg"

# Both texts, because Sprite is offered under either.
install -Dm644 LICENSE-MIT                   "$root/share/licenses/sprite/LICENSE-MIT"
install -Dm644 LICENSE-APACHE                "$root/share/licenses/sprite/LICENSE-APACHE"
install -Dm644 THIRD-PARTY-NOTICES.md        "$root/share/licenses/sprite/THIRD-PARTY-NOTICES.md"

# Sprite's own database, beside Sprite, and **not** in the shared tree: on Arch,
# `ncurses` owns /usr/share/terminfo/g/ghostty and `ghostty-terminfo` owns
# x/xterm-ghostty, so writing there is a file conflict pacman refuses. Sprite
# adds this directory to the search instead of replacing anything, and this copy
# is the one built from the pinned Ghostty commit.
#
# `-x` keeps the extended capabilities: the entry uses them, and an ncurses
# built without them produces a terminal that quietly lacks features.
install -d "$root/share/sprite/terminfo"
tic -x -o "$root/share/sprite/terminfo" "$TERMINFO_SOURCE"

echo "installed sprite into $root"
