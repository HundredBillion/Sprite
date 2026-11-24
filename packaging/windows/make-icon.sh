#!/bin/sh
# Renders packaging/windows/sprite.ico from packaging/Sprite.png on macOS.
#
#   packaging/windows/make-icon.sh
#
# Windows builds do not have sips or an SVG rasterizer, so sprite.ico is
# committed beside sprite.rc rather than generated during Cargo builds.
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$root"

src=packaging/Sprite.png
out=packaging/windows/sprite.ico

if [ ! -f "$src" ]; then
    echo "make-icon.sh: $src does not exist" >&2
    exit 1
fi
if [ ! -x /usr/bin/sips ]; then
    echo "make-icon.sh: /usr/bin/sips is required (macOS)" >&2
    exit 1
fi
if ! command -v python3 >/dev/null 2>&1; then
    echo "make-icon.sh: python3 is required" >&2
    exit 1
fi

work=$(mktemp -d "${TMPDIR:-/tmp}/sprite-ico.XXXXXX")
trap 'rm -rf "$work"' EXIT

for size in 16 32 48; do
    /usr/bin/sips -s format ico -z "$size" "$size" "$src" --out "$work/${size}.ico" >/dev/null
done
/usr/bin/sips -s format png -z 256 256 "$src" --out "$work/256.png" >/dev/null

python3 - "$work" "$out" <<'PY'
import pathlib
import struct
import sys

work = pathlib.Path(sys.argv[1])
out = pathlib.Path(sys.argv[2])

entries = []
for size in (16, 32, 48):
    raw = (work / f"{size}.ico").read_bytes()
    reserved, icon_type, count = struct.unpack_from("<HHH", raw, 0)
    if (reserved, icon_type, count) != (0, 1, 1):
        raise SystemExit(f"unexpected single-icon header in {size}.ico: {(reserved, icon_type, count)}")
    w, h, _cc, _rsv, planes, bpp, byte_size, offset = struct.unpack_from("<BBBBHHII", raw, 6)
    payload = raw[offset : offset + byte_size]
    if len(payload) != byte_size:
        raise SystemExit(f"truncated payload in {size}.ico")
    entries.append((w, h, planes, bpp, payload))

png_payload = (work / "256.png").read_bytes()
if not png_payload.startswith(b"\x89PNG\r\n\x1a\n"):
    raise SystemExit("256.png is not a valid PNG")
entries.append((0, 0, 1, 32, png_payload))

header = struct.pack("<HHH", 0, 1, len(entries))
offset = 6 + 16 * len(entries)
dir_bytes = bytearray()
data_bytes = bytearray()
for w, h, planes, bpp, payload in entries:
    dir_bytes.extend(struct.pack("<BBBBHHII", w, h, 0, 0, planes, bpp, len(payload), offset))
    data_bytes.extend(payload)
    offset += len(payload)

out.parent.mkdir(parents=True, exist_ok=True)
out.write_bytes(header + dir_bytes + data_bytes)
PY

echo "wrote $out"
