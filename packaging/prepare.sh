#!/bin/sh
# Generate package terminfo from the same pinned source as the terminal engine.
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"
export ZIG_GLOBAL_CACHE_DIR="$PWD/target/zig-global-cache"
export ZIG_LOCAL_CACHE_DIR="$PWD/target/zig-local-cache"
mkdir -p target
zig build-exe -lc -femit-bin=target/gen-terminfo \
    --dep ghostty_terminfo \
    -Mroot=scripts/gen-terminfo.zig \
    -Mghostty_terminfo=vendor/ghostty/src/terminfo/ghostty.zig
./target/gen-terminfo > target/ghostty.terminfo
