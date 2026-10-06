#!/usr/bin/env python3
"""Update Sprite's checked-in release version references."""

import argparse
import os
import tempfile
import re
import sys
import tomllib
from pathlib import Path


VERSION_RE = re.compile(
    r"(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\Z"
)
WORKSPACE_PACKAGES = ("sprite-app", "sprite-pane", "sprite-term")


def replace_once(
    content: str, path: Path, pattern: re.Pattern[str], replacement: str
) -> str:
    updated, count = pattern.subn(replacement, content)
    if count != 1:
        raise ValueError(f"expected exactly one version reference in {path}")
    return updated


def update_cargo_lock(content: str, current: str, version: str) -> str:
    updated = content
    for package_name in WORKSPACE_PACKAGES:
        package = re.compile(
            r"(?ms)(^\[\[package\]\]\n(?:(?!^\[\[package\]\]).)*?^name = \""
            + re.escape(package_name)
            + r"\"\n)(.*?)(?=^\[\[package\]\]|\Z)"
        )
        match = package.search(updated)
        if match is None:
            raise ValueError(f"Cargo.lock has no package entry for {package_name}")
        version_line = re.compile(r'(?m)^version = "' + re.escape(current) + r'"$')
        body, count = version_line.subn(
            f'version = "{version}"', match.group(2), count=1
        )
        if count != 1:
            raise ValueError(f"Cargo.lock has no unique version for {package_name}")
        updated = updated[: match.start(2)] + body + updated[match.end(2) :]
    return updated


def replace_release_files(updates: list[tuple[Path, str]]) -> None:
    staged: list[tuple[Path, Path, Path]] = []
    temporary: list[Path] = []
    replaced: list[tuple[Path, Path]] = []
    preserve: set[Path] = set()
    try:
        for path, content in updates:
            original = path.read_bytes()
            mode = path.stat().st_mode
            copies = []
            for data in (content.encode("utf-8"), original):
                with tempfile.NamedTemporaryFile(dir=path.parent, prefix=".sprite-release-", delete=False) as file:
                    copy = Path(file.name)
                    temporary.append(copy)
                    file.write(data)
                    file.flush()
                    os.fsync(file.fileno())
                copy.chmod(mode)
                copies.append(copy)
            staged.append((path, copies[0], copies[1]))
        for path, replacement, backup in staged:
            os.replace(replacement, path)
            replaced.append((path, backup))
    except OSError as error:
        failures = []
        for path, backup in reversed(replaced):
            try:
                os.replace(backup, path)
            except OSError as recovery_error:
                preserve.add(backup)
                failures.append(f"{path}: {recovery_error}; original retained at {backup}")
        if failures:
            raise OSError("release rollback failed: " + "; ".join(failures)) from error
        raise
    finally:
        for path in temporary:
            if path not in preserve:
                path.unlink(missing_ok=True)


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
    cargo_lock = root / "Cargo.lock"
    updated_lock = update_cargo_lock(cargo_lock.read_text(), current, version)

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

    replace_release_files([
        (cargo_toml, updated_cargo),
        (cargo_lock, updated_lock),
        (pkgbuild, updated_pkgbuild),
        (readme, updated_readme),
    ])


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
