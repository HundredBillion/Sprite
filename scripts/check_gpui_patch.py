#!/usr/bin/env python3
"""Verify the local GPUI tree is the published archive plus the recorded patch."""
import argparse
import hashlib
import re
import tarfile
import urllib.request
from pathlib import Path

CHECKSUM = "979b45cfa6ec723b6f42330915a1b3769b930d02b2d505f9697f8ca602bee707"
ROOT = Path(__file__).resolve().parents[1]


def archive_path(fetch=False):
    cached = list((Path.home() / ".cargo/registry/cache").glob("*/gpui-0.2.2.crate"))
    if cached:
        return cached[0]
    path = ROOT / "target/gpui-0.2.2.crate"
    if not path.exists() and fetch:
        path.parent.mkdir(parents=True, exist_ok=True)
        urllib.request.urlretrieve("https://static.crates.io/crates/gpui/gpui-0.2.2.crate", path)
    return path


def apply_exact_patch(files, patch):
    """Apply recorded unified hunks at their exact lines, without offset/fuzz."""
    lines = patch.splitlines(keepends=True)
    index = 0
    while index < len(lines):
        if not lines[index].startswith("--- ") or not lines[index + 1].startswith("+++ "):
            raise ValueError("invalid GPUI patch file header")
        old_path = lines[index][4:].strip()
        path = lines[index + 1][4:].strip().removeprefix("b/")
        if Path(path).is_absolute() or ".." in Path(path).parts:
            raise ValueError("unsafe GPUI patch path")
        original = files.get(path, b"").decode().splitlines(keepends=True)
        if old_path == "/dev/null" and path in files:
            raise ValueError("patch addition already exists")
        if old_path != "/dev/null" and old_path != "a/" + path:
            raise ValueError("patch must not rename source")
        output, cursor = [], 0
        index += 2
        while index < len(lines) and lines[index].startswith("@@ "):
            match = re.fullmatch(r"@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@.*\n", lines[index])
            if not match:
                raise ValueError("invalid GPUI patch hunk")
            old_line, old_count, new_line, new_count = (int(value) if value else 1 for value in match.groups())
            position = max(old_line - 1, 0)
            if position < cursor or position > len(original):
                raise ValueError("GPUI patch hunk offset mismatch")
            output.extend(original[cursor:position])
            cursor = position
            if len(output) != max(new_line - 1, 0):
                raise ValueError("GPUI patch new line mismatch")
            index += 1
            consumed = produced = 0
            while index < len(lines) and lines[index][:1] in (" ", "+", "-") and not lines[index].startswith("--- "):
                line = lines[index]
                if line[0] in " -":
                    if cursor >= len(original) or original[cursor] != line[1:]:
                        raise ValueError("GPUI patch source context mismatch")
                    cursor += 1
                    consumed += 1
                if line[0] in " +":
                    output.append(line[1:])
                    produced += 1
                index += 1
            if (consumed, produced) != (old_count, new_count):
                raise ValueError("GPUI patch hunk count mismatch")
        output.extend(original[cursor:])
        files[path] = "".join(output).encode()
    return files


def verify(tree, archive):
    if hashlib.sha256(archive.read_bytes()).hexdigest() != CHECKSUM:
        raise ValueError("published GPUI archive checksum mismatch")
    with tarfile.open(archive) as source:
        expected = {member.name.removeprefix("gpui-0.2.2/"): source.extractfile(member).read()
                    for member in source.getmembers() if member.isfile()}
    expected = apply_exact_patch(expected, (ROOT / "patches/gpui-0.2.2-native-text.patch").read_text())
    actual = {str(p.relative_to(tree)): p.read_bytes() for p in tree.rglob("*") if p.is_file()}
    changed = sorted(p for p in expected.keys() | actual.keys() if expected.get(p) != actual.get(p))
    if changed:
        raise ValueError("local GPUI differs from exact archive + patch: " + ", ".join(changed))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--fetch", action="store_true")
    parser.add_argument("--tree", type=Path, default=ROOT / "vendor/gpui")
    args = parser.parse_args()
    verify(args.tree, archive_path(args.fetch))
    print("GPUI archive checksum and exact patched source verified")


if __name__ == "__main__":
    main()
