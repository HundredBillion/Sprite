#!/bin/sh
# Turns a built Sprite into Sprite.app.
#
# The macOS counterpart of ../install.sh: it builds nothing, and everything it
# bundles must already exist. A bundle is a folder layout, a property list, an
# icon, and a signature around the same binary and terminfo the Linux package
# installs.
#
#   packaging/macos/bundle.sh                       -> target/Sprite.app
#   BINARY=target/debug/sprite packaging/macos/bundle.sh
#
# The icon is rendered from ../sprite.svg here rather than kept as a committed
# .icns, for the reason the terminfo is compiled rather than copied: a generated
# artifact in the tree drifts from its source. sips and iconutil ship with
# macOS.
#
# The signature is ad-hoc. An app built on the machine that runs it is never
# quarantined, and a Developer ID protects a download Sprite does not have.
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$root"

BINARY=${BINARY:-target/release/sprite}
TERMINFO_SOURCE=${TERMINFO_SOURCE:-target/ghostty.terminfo}
OUT=${OUT:-target/Sprite.app}

for required in "$BINARY" "$TERMINFO_SOURCE" packaging/sprite.svg packaging/macos/Info.plist; do
    if [ ! -f "$required" ]; then
        echo "bundle.sh: $required does not exist; build it first" >&2
        exit 1
    fi
done
for tool in sips iconutil tic codesign plutil; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        echo "bundle.sh: $tool is not on PATH; this script runs on macOS" >&2
        exit 1
    fi
done

version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
if [ -z "$version" ]; then
    echo "bundle.sh: no workspace version in Cargo.toml" >&2
    exit 1
fi

contents="$OUT/Contents"
# Fresh every time: a stale bundle can keep a file this run no longer writes,
# and the signature would then cover the wrong set.
rm -rf "$OUT"
mkdir -p "$contents/MacOS" "$contents/Resources"

install -m755 "$BINARY" "$contents/MacOS/sprite"
sed "s/@VERSION@/$version/g" packaging/macos/Info.plist > "$contents/Info.plist"
plutil -lint -s "$contents/Info.plist"

# Every size Finder and the Dock ask for, from one vector source. The @2x names
# are how iconutil pairs a size with its Retina double.
iconset=$(mktemp -d "${TMPDIR:-/tmp}/sprite-iconset.XXXXXX")
trap 'rm -rf "$iconset"' EXIT
set_dir="$iconset/sprite.iconset"
mkdir "$set_dir"
for size in 16 32 128 256 512; do
    double=$((size * 2))
    sips -s format png -z "$size" "$size" packaging/sprite.svg \
        --out "$set_dir/icon_${size}x${size}.png" >/dev/null
    sips -s format png -z "$double" "$double" packaging/sprite.svg \
        --out "$set_dir/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$set_dir" -o "$contents/Resources/sprite.icns"

# Sprite's own terminfo, inside the bundle where codesign seals resources. `-x`
# keeps the extended capabilities the entry uses. The binary looks here through
# its canonical path, so the symlinked command finds it too.
mkdir -p "$contents/Resources/terminfo"
tic -x -o "$contents/Resources/terminfo" "$TERMINFO_SOURCE"

install -m644 LICENSE-MIT LICENSE-APACHE THIRD-PARTY-NOTICES.md "$contents/Resources/"

codesign --force --deep --sign - "$OUT"
codesign --verify --deep --strict "$OUT"

echo "bundled $OUT (version $version)"
