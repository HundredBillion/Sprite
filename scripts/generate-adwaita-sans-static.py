#!/usr/bin/env python3
"""Rebuild Sprite's pinned Adwaita Sans static Linux UI faces."""

import hashlib
import io
import sys
import tarfile
from pathlib import Path

from fontTools.ttLib import TTFont
from fontTools.varLib.instancer import instantiateVariableFont

ARCHIVE_SHA256 = "4c927fbfeec1c503801ba510c2c94e0054c82c522cf7ba0d3be5d4d41fcf5c86"
SOURCE_SHA256 = "8381c33b9a44f066f2b99dba3d416a2342891e28c956a35dfd8d16ee2987e6d4"
LICENSE_SHA256 = "459687971d21c53923c1d1c9c062ec273a7ea03226b36195b79ec6af7d98dc81"
OUTPUT_SHA256 = {
    "Regular": "e2e3e6aede5c4245c7c3260e9d2ef4437a2ab0dc7a446ee14127ad03db55dff3",
    "Bold": "fd309de15fe71c3b9cd68ee59266ede17a8af00a25ac39852b4b0ad7bad61080",
}
ROOT = Path(__file__).resolve().parents[1]
OUTPUT = ROOT / "crates/sprite-app/assets/fonts"


def sha256(data):
    return hashlib.sha256(data).hexdigest()


def main(archive_path):
    archive_data = archive_path.read_bytes()
    if sha256(archive_data) != ARCHIVE_SHA256:
        raise ValueError("Adwaita Fonts 50.0 archive hash mismatch")
    with tarfile.open(fileobj=io.BytesIO(archive_data), mode="r:xz") as archive:
        source = archive.extractfile("adwaita-fonts-50.0/sans/AdwaitaSans-Regular.ttf").read()
        license_text = archive.extractfile("adwaita-fonts-50.0/LICENSE").read()
    if sha256(source) != SOURCE_SHA256 or sha256(license_text) != LICENSE_SHA256:
        raise ValueError("Adwaita source font or license hash mismatch")
    if (OUTPUT / "OFL-1.1.txt").read_bytes() != license_text:
        raise ValueError("bundled OFL license differs from source archive")
    for label, weight in (("Regular", 400), ("Bold", 700)):
        font = TTFont(io.BytesIO(source), recalcTimestamp=False)
        font = instantiateVariableFont(font, {"opsz": 14, "wght": weight}, updateFontNames=True)
        output = io.BytesIO()
        font.save(output, reorderTables=True)
        digest = sha256(output.getvalue())
        if digest != OUTPUT_SHA256[label]:
            raise ValueError(f"{label} instance hash mismatch: {digest}")
        path = OUTPUT / f"AdwaitaSans-{label}.ttf"
        path.write_bytes(output.getvalue())
        print(f"{path.relative_to(ROOT)} {digest}")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        sys.exit("usage: uv run --with fonttools==4.65.0 python scripts/generate-adwaita-sans-static.py ARCHIVE")
    main(Path(sys.argv[1]))
