#!/usr/bin/env python3
"""Require every promised behavior to be held by a test that exists.

`docs/capabilities.toml` lists what Sprite promises a person using it. A kept
capability must name at least one test, and every named test must exist. An
area may not lose its last kept capability, and with `--base` a capability
present on the base branch may not disappear: retiring one is a status change
with a reason and a decision, never a deletion. A behavior on the list
therefore cannot be dropped by renaming or inverting its test without the
change showing up in this file.
"""

import argparse
import re
import subprocess
import sys
import tomllib
from pathlib import Path

try:
    from scripts.check_findings import test_names
except ModuleNotFoundError:  # run as a script from the repository root
    from check_findings import test_names


LEDGER = Path("docs/capabilities.toml")
NAME = r"[a-z0-9]+(?:-[a-z0-9]+)*"
ID_RE = re.compile(rf"{NAME}\.{NAME}")
STATUSES = {"kept", "retired"}


def check_capabilities(root: Path, base: str | None = None) -> list[str]:
    """Returns every complaint, so a report lists all of them at once."""
    ledger = tomllib.loads((root / LEDGER).read_text())
    complaints: list[str] = []
    areas = ledger.get("areas")
    if not isinstance(areas, list) or not areas or not all(isinstance(area, str) for area in areas):
        return [f"{LEDGER}: areas must be a non-empty list of names"]
    known_tests, plain_fns = test_names(root)
    seen: dict[str, int] = {}
    kept_in: dict[str, int] = dict.fromkeys(areas, 0)

    for index, capability in enumerate(ledger.get("capability", []), start=1):
        where = f"{LEDGER} capability #{index}"
        identifier = capability.get("id")
        if not isinstance(identifier, str) or ID_RE.fullmatch(identifier) is None:
            complaints.append(f"{where}: id must look like <area>.<name> in lower-case words; got {identifier!r}")
            continue
        where = f"{LEDGER} {identifier}"
        if identifier in seen:
            complaints.append(f"{where}: listed twice (also capability #{seen[identifier]})")
        seen[identifier] = index

        area = capability.get("area")
        if area not in kept_in:
            complaints.append(f"{where}: area {area!r} is not in the areas list")
        elif identifier.split(".", 1)[0] != area:
            complaints.append(f"{where}: id must start with its area, {area!r}")

        if not isinstance(capability.get("promise"), str) or not capability["promise"].strip():
            complaints.append(f"{where}: promise is required; say what a person can rely on")

        status = capability.get("status")
        if status not in STATUSES:
            complaints.append(f"{where}: status must be one of {sorted(STATUSES)}; got {status!r}")
            continue

        if status == "retired":
            for field in ("reason", "decision"):
                if not isinstance(capability.get(field), str) or not capability[field].strip():
                    complaints.append(f"{where}: a retired capability needs a {field}")
            continue

        tests = capability.get("tests")
        if not isinstance(tests, list) or not tests or not all(isinstance(test, str) and test for test in tests):
            complaints.append(f"{where}: kept without a test; name the test that holds this promise")
            continue
        for test in tests:
            if test in known_tests:
                continue
            if test in plain_fns:
                complaints.append(
                    f"{where}: {test!r} is a Rust fn under crates/ but not a test; "
                    "it needs #[test] or #[<path>::test] above it"
                )
            else:
                complaints.append(
                    f"{where}: test {test!r} no longer exists. If the behavior changed on purpose, "
                    "retire this capability with a reason and a decision; otherwise restore the test"
                )
        if area in kept_in:
            kept_in[area] += 1

    for area, count in kept_in.items():
        if count == 0:
            complaints.append(f"{LEDGER}: area {area!r} has no kept capability; nothing would notice it breaking")

    if base is not None:
        for identifier in sorted(ids_on(root, base) - set(seen)):
            complaints.append(
                f"{LEDGER}: {identifier} is on {base} but missing here; "
                "retire it with a reason and a decision instead of deleting it"
            )
    return complaints


def ids_on(root: Path, ref: str) -> set[str]:
    """Capability ids in the ledger at `ref`; none when the ledger is not there yet."""
    shown = subprocess.run(
        ["git", "-C", str(root), "show", f"{ref}:{LEDGER.as_posix()}"],
        capture_output=True,
        text=True,
        check=False,
    )
    if shown.returncode != 0:
        present = subprocess.run(
            ["git", "-C", str(root), "rev-parse", "--verify", "--quiet", f"{ref}^{{commit}}"],
            capture_output=True,
            check=False,
        )
        if present.returncode != 0:
            raise OSError(f"base {ref!r} is not a commit this checkout has")
        return set()
    return {
        capability["id"]
        for capability in tomllib.loads(shown.stdout).get("capability", [])
        if isinstance(capability.get("id"), str)
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path.cwd())
    parser.add_argument("--base", help="a git ref whose capabilities must all still be listed")
    args = parser.parse_args()
    try:
        complaints = check_capabilities(args.root, args.base)
    except (OSError, tomllib.TOMLDecodeError) as error:
        print(f"check_capabilities: {error}", file=sys.stderr)
        return 1
    for complaint in complaints:
        print(f"check_capabilities: {complaint}", file=sys.stderr)
    if complaints:
        print(f"check_capabilities: {len(complaints)} problem(s)", file=sys.stderr)
        return 1
    print("every kept capability is held by a test that exists")
    return 0


if __name__ == "__main__":
    sys.exit(main())
