#!/usr/bin/env python3
"""Require a vX.Y.Z release tag to match the Cargo workspace version."""

import argparse
import re
import sys
import tomllib
from pathlib import Path


TAG_RE = re.compile(
    r"v((?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*))\Z"
)


def check_release_version(root: Path, tag: str) -> None:
    match = TAG_RE.fullmatch(tag)
    if match is None:
        raise ValueError(f"release tag must be vX.Y.Z; got {tag!r}")
    cargo_version = tomllib.loads((root / "Cargo.toml").read_text())["workspace"]
    cargo_version = cargo_version["package"]["version"]
    if match.group(1) != cargo_version:
        raise ValueError(
            f"release tag {tag} does not match Cargo workspace version {cargo_version}"
        )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("tag", help="release tag, for example v0.2.1")
    parser.add_argument("--root", type=Path, default=Path.cwd())
    args = parser.parse_args()
    try:
        check_release_version(args.root, args.tag)
    except (OSError, KeyError, tomllib.TOMLDecodeError, ValueError) as error:
        print(f"check_release_version: {error}", file=sys.stderr)
        return 1
    print(f"release tag {args.tag} matches Cargo workspace version")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
