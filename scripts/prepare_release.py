#!/usr/bin/env python3
"""Update Sprite's checked-in release version references."""

import argparse
import re
import sys
import tomllib
from pathlib import Path


VERSION_RE = re.compile(
    r"(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\Z"
)


def replace_once(
    content: str, path: Path, pattern: re.Pattern[str], replacement: str
) -> str:
    updated, count = pattern.subn(replacement, content)
    if count != 1:
        raise ValueError(f"expected exactly one version reference in {path}")
    return updated


def prepare_release(root: Path, version: str) -> None:
    if not VERSION_RE.fullmatch(version):
        raise ValueError("version must be a stable X.Y.Z version without a v prefix")

    cargo_toml = root / "Cargo.toml"
    cargo_content = cargo_toml.read_text()
    current = tomllib.loads(cargo_content)["workspace"]["package"]["version"]
    section = re.compile(r"(?ms)(^\[workspace\.package\]\s*\n)(.*?)(?=^\[|\Z)")
    match = section.search(cargo_content)
    if match is None:
        raise ValueError("Cargo.toml has no [workspace.package] section")
    version_line = re.compile(r'(?m)^version = "' + re.escape(current) + r'"$')
    section_body, count = version_line.subn(
        f'version = "{version}"', match.group(2), count=1
    )
    if count != 1:
        raise ValueError("could not find the workspace version in Cargo.toml")
    updated_cargo = (
        cargo_content[: match.start(2)]
        + section_body
        + cargo_content[match.end(2) :]
    )

    pkgbuild = root / "packaging" / "PKGBUILD"
    updated_pkgbuild = replace_once(
        pkgbuild.read_text(),
        pkgbuild,
        re.compile(r"(?m)^pkgver=" + re.escape(current) + r"$"),
        f"pkgver={version}",
    )
    readme = root / "README.md"
    updated_readme = replace_once(
        readme.read_text(),
        readme,
        re.compile(
            r"sprite-" + re.escape(current) + r"(-1-x86_64\.pkg\.tar\.zst)"
        ),
        f"sprite-{version}\\1",
    )

    cargo_toml.write_text(updated_cargo)
    pkgbuild.write_text(updated_pkgbuild)
    readme.write_text(updated_readme)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("version", help="stable release version, for example 0.2.1")
    parser.add_argument("--root", type=Path, default=Path.cwd())
    args = parser.parse_args()
    try:
        prepare_release(args.root, args.version)
    except (OSError, KeyError, tomllib.TOMLDecodeError, ValueError) as error:
        print(f"prepare_release: {error}", file=sys.stderr)
        return 1
    print(f"prepared Sprite release {args.version}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
