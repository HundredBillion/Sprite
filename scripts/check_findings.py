#!/usr/bin/env python3
"""Require every reviewed finding to be either guarded by a test or tracked.

`docs/reviews/findings.toml` is the one ledger of review findings. A finding
is closed only when a named regression test exists in `crates/`; a finding
that is still open is visible only through a GitHub issue. Any finding ID that
appears anywhere under `docs/` must be registered here, so an audit cannot
leave a bug as a sentence in a report.
"""

import argparse
import re
import sys
import tomllib
from pathlib import Path


LEDGER = Path("docs/reviews/findings.toml")
ID_RE = re.compile(r"\b(?:SPR-\d{3}|BCA-\d{2}|GAP-C\d{2})\b")
FN_RE = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+([A-Za-z_][A-Za-z0-9_]*)\s*[<(]", re.MULTILINE)
PY_TEST_RE = re.compile(r"^\s*def\s+(test_[A-Za-z0-9_]*)\s*\(", re.MULTILINE)
STATUSES = {"open", "resolved", "deferred", "wontfix"}
TRACKED = {"open", "deferred"}


def test_names(root: Path) -> set[str]:
    """Every Rust `fn` under crates/ and every Python test under scripts/.

    Packaging, release and shell-integration findings are guarded by the
    Python suites, so those names count too.
    """
    names: set[str] = set()
    for path in (root / "crates").rglob("*.rs"):
        names.update(FN_RE.findall(path.read_text(errors="replace")))
    for path in (root / "scripts").glob("test_*.py"):
        names.update(PY_TEST_RE.findall(path.read_text(errors="replace")))
    return names


def ids_in_docs(docs: Path) -> set[str]:
    found: set[str] = set()
    for path in docs.rglob("*.md"):
        found.update(ID_RE.findall(path.read_text(errors="replace")))
    return found


def check_findings(root: Path) -> list[str]:
    """Returns every complaint, so a report lists all of them at once."""
    ledger_path = root / LEDGER
    ledger = tomllib.loads(ledger_path.read_text())
    findings = ledger.get("finding", [])
    complaints: list[str] = []
    seen: dict[str, int] = {}
    known_tests = test_names(root)

    for index, finding in enumerate(findings, start=1):
        where = f"{LEDGER} finding #{index}"
        identifier = finding.get("id")
        if not isinstance(identifier, str) or ID_RE.fullmatch(identifier) is None:
            complaints.append(f"{where}: id must match SPR-nnn, BCA-nn or GAP-Cnn; got {identifier!r}")
            continue
        where = f"{LEDGER} {identifier}"
        if identifier in seen:
            complaints.append(f"{where}: listed twice (also finding #{seen[identifier]})")
        seen[identifier] = index

        for field in ("title", "source"):
            if not isinstance(finding.get(field), str) or not finding[field].strip():
                complaints.append(f"{where}: {field} is required")

        status = finding.get("status")
        if status not in STATUSES:
            complaints.append(f"{where}: status must be one of {sorted(STATUSES)}; got {status!r}")
            continue

        if status == "resolved":
            tests = finding.get("test")
            if isinstance(tests, str):
                tests = [tests]
            if not tests or not all(isinstance(test, str) and test for test in tests):
                complaints.append(f"{where}: resolved without a test; name the regression that guards it")
            else:
                for test in tests:
                    if test not in known_tests:
                        complaints.append(
                            f"{where}: test {test!r} is not a Rust fn under crates/ "
                            "or a Python test under scripts/"
                        )
            if not isinstance(finding.get("fixed_by"), str):
                complaints.append(f"{where}: resolved without fixed_by (a PR or commit)")

        if status in TRACKED:
            issue = finding.get("issue")
            if not isinstance(issue, int) or issue <= 0:
                complaints.append(f"{where}: {status} findings need a GitHub issue number")

        if status == "wontfix" and not isinstance(finding.get("reason"), str):
            complaints.append(f"{where}: wontfix needs a reason")

    unregistered = ids_in_docs(root / "docs") - set(seen)
    for identifier in sorted(unregistered):
        complaints.append(f"{identifier} appears under docs/ but is not registered in {LEDGER}")

    return complaints


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path.cwd())
    args = parser.parse_args()
    try:
        complaints = check_findings(args.root)
    except (OSError, tomllib.TOMLDecodeError) as error:
        print(f"check_findings: {error}", file=sys.stderr)
        return 1
    for complaint in complaints:
        print(f"check_findings: {complaint}", file=sys.stderr)
    if complaints:
        print(f"check_findings: {len(complaints)} problem(s)", file=sys.stderr)
        return 1
    print("every finding is guarded by a test or tracked by an issue")
    return 0


if __name__ == "__main__":
    sys.exit(main())
