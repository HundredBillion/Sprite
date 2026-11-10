"""Exercise the capability ledger check against fixtures and the repository."""

from pathlib import Path
import subprocess
import tempfile
import textwrap
import unittest

from scripts.check_capabilities import check_capabilities


ROOT = Path(__file__).resolve().parents[1]

KEPT = """
    areas = ["links"]

    [[capability]]
    id = "links.url-opens"
    area = "links"
    promise = "Clicking a URL opens it."
    status = "kept"
    tests = ["a_url_opens"]
"""


def fixture(ledger: str = KEPT, rust: str = "#[test]\nfn a_url_opens() {}") -> Path:
    directory = Path(tempfile.mkdtemp())
    (directory / "docs").mkdir()
    (directory / "docs/capabilities.toml").write_text(textwrap.dedent(ledger))
    (directory / "crates/x/src").mkdir(parents=True)
    (directory / "crates/x/src/lib.rs").write_text(rust)
    (directory / "scripts").mkdir()
    return directory


def complaints(*args, **kwargs) -> str:
    return "\n".join(check_capabilities(*args, **kwargs))


def git(directory: Path, *args: str) -> None:
    subprocess.run(
        ["git", "-C", str(directory), "-c", "user.name=t", "-c", "user.email=t@example.com", *args],
        check=True,
        capture_output=True,
    )


class CapabilityLedgerTests(unittest.TestCase):
    def test_the_repository_ledger_is_held_by_existing_tests(self):
        self.assertEqual(check_capabilities(ROOT), [])

    def test_a_kept_capability_with_an_existing_test_passes(self):
        self.assertEqual(check_capabilities(fixture()), [])

    def test_renaming_the_test_that_holds_a_promise_is_refused(self):
        # The shape of PR #98: the guarding test was replaced by one asserting
        # the opposite, under a new name.
        renamed = fixture(rust="#[test]\nfn a_url_is_not_clickable() {}")
        self.assertIn("'a_url_opens' no longer exists", complaints(renamed))
        self.assertIn("retire this capability", complaints(renamed))

    def test_a_kept_capability_needs_a_test(self):
        untested = KEPT.replace('tests = ["a_url_opens"]', "tests = []")
        self.assertIn("kept without a test", complaints(fixture(untested)))
        absent = KEPT.replace('    tests = ["a_url_opens"]\n', "")
        self.assertIn("kept without a test", complaints(fixture(absent)))

    def test_a_plain_function_does_not_hold_a_promise(self):
        self.assertIn("but not a test", complaints(fixture(rust="fn a_url_opens() {}")))

    def test_retiring_needs_a_reason_and_a_decision(self):
        retired = KEPT.replace('status = "kept"', 'status = "retired"')
        found = complaints(fixture(retired))
        self.assertIn("needs a reason", found)
        self.assertIn("needs a decision", found)

    def test_an_area_may_not_lose_its_last_kept_capability(self):
        retired = KEPT.replace('status = "kept"', 'status = "retired"\n    reason = "r"\n    decision = "ADR 1"')
        self.assertIn("area 'links' has no kept capability", complaints(fixture(retired)))

    def test_a_retired_capability_beside_a_kept_one_passes(self):
        both = KEPT + """
            [[capability]]
            id = "links.label-opens"
            area = "links"
            promise = "Clicking a label opens it."
            status = "retired"
            reason = "Only visible URLs are links."
            decision = "#98"
        """
        self.assertEqual(check_capabilities(fixture(textwrap.dedent(both))), [])

    def test_identifiers_are_unique_and_named_for_their_area(self):
        twice = KEPT + textwrap.dedent(KEPT).split("\n\n", 1)[1]
        self.assertIn("listed twice", complaints(fixture(twice)))
        misplaced = KEPT.replace('id = "links.url-opens"', 'id = "keys.url-opens"')
        self.assertIn("must start with its area", complaints(fixture(misplaced)))
        unknown = KEPT.replace('area = "links"', 'area = "nowhere"')
        self.assertIn("is not in the areas list", complaints(fixture(unknown)))
        shouting = KEPT.replace('id = "links.url-opens"', 'id = "Links URL"')
        self.assertIn("id must look like", complaints(fixture(shouting)))

    def test_a_promise_and_a_known_status_are_required(self):
        silent = KEPT.replace('promise = "Clicking a URL opens it."', 'promise = " "')
        self.assertIn("promise is required", complaints(fixture(silent)))
        vague = KEPT.replace('status = "kept"', 'status = "maybe"')
        self.assertIn("status must be one of", complaints(fixture(vague)))

    def test_a_capability_on_the_base_branch_may_not_be_deleted(self):
        directory = fixture()
        git(directory, "init", "-q", "-b", "base")
        git(directory, "add", "-A")
        git(directory, "commit", "-q", "-m", "base")
        replaced = KEPT.replace("links.url-opens", "links.other").replace("a_url_opens", "another")
        (directory / "docs/capabilities.toml").write_text(textwrap.dedent(replaced))
        (directory / "crates/x/src/lib.rs").write_text("#[test]\nfn another() {}")
        self.assertIn("links.url-opens is on base but missing here", complaints(directory, base="base"))
        self.assertEqual(check_capabilities(directory), [])

    def test_a_base_without_a_ledger_constrains_nothing(self):
        directory = fixture()
        git(directory, "init", "-q", "-b", "base")
        git(directory, "commit", "-q", "--allow-empty", "-m", "before the ledger")
        self.assertEqual(check_capabilities(directory, base="base"), [])

    def test_an_unknown_base_is_an_error_not_a_pass(self):
        directory = fixture()
        git(directory, "init", "-q")
        with self.assertRaises(OSError):
            check_capabilities(directory, base="no-such-ref")


if __name__ == "__main__":
    unittest.main()
