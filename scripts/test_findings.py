"""Exercise the findings ledger check against fixtures and the repository."""

from pathlib import Path
import tempfile
import textwrap
import unittest

from scripts.check_findings import check_findings


ROOT = Path(__file__).resolve().parents[1]


def fixture(ledger: str, rust: str = "#[test]\nfn guards_it() {}", doc: str = "") -> Path:
    directory = Path(tempfile.mkdtemp())
    (directory / "docs/reviews").mkdir(parents=True)
    (directory / "docs/reviews/findings.toml").write_text(textwrap.dedent(ledger))
    (directory / "docs/reviews/audit.md").write_text(doc)
    (directory / "crates/x/src").mkdir(parents=True)
    (directory / "crates/x/src/lib.rs").write_text(rust)
    return directory


RESOLVED = """
    [[finding]]
    id = "BCA-01"
    title = "Something"
    source = "docs/reviews/audit.md"
    status = "resolved"
    fixed_by = "#58"
    test = "guards_it"
"""


class FindingsLedgerTests(unittest.TestCase):
    def test_the_repository_ledger_is_complete(self):
        self.assertEqual(check_findings(ROOT), [])

    def test_a_resolved_finding_with_an_existing_test_passes(self):
        self.assertEqual(check_findings(fixture(RESOLVED)), [])

    def test_a_resolved_finding_needs_a_test_that_exists(self):
        missing = RESOLVED.replace('test = "guards_it"', 'test = "nope"')
        self.assertIn("is not a Rust fn", "\n".join(check_findings(fixture(missing))))
        absent = RESOLVED.replace('test = "guards_it"\n', "")
        self.assertIn("without a test", "\n".join(check_findings(fixture(absent))))

    def test_a_resolved_finding_cannot_cite_a_plain_fn(self):
        complaints = check_findings(fixture(RESOLVED, rust="fn guards_it() {}"))
        self.assertEqual(len(complaints), 1, complaints)
        self.assertIn("BCA-01", complaints[0])
        self.assertIn("not a test", complaints[0])

    def test_a_python_test_under_scripts_also_guards(self):
        directory = fixture(RESOLVED.replace('test = "guards_it"', 'test = "test_recipe"'))
        (directory / "scripts").mkdir()
        (directory / "scripts/test_packaging.py").write_text("def test_recipe(self):\n    pass\n")
        self.assertEqual(check_findings(directory), [])

    def test_a_resolved_finding_names_what_fixed_it(self):
        anonymous = RESOLVED.replace('fixed_by = "#58"\n', "")
        self.assertIn("fixed_by", "\n".join(check_findings(fixture(anonymous))))

    def test_an_open_finding_needs_an_issue(self):
        untracked = """
            [[finding]]
            id = "GAP-C03"
            title = "Slow SVG filters"
            source = "docs/reviews/audit.md"
            status = "open"
        """
        self.assertIn("GitHub issue", "\n".join(check_findings(fixture(untracked))))
        tracked = untracked + "            issue = 61\n"
        self.assertEqual(check_findings(fixture(tracked)), [])

    def test_wontfix_needs_a_reason(self):
        ledger = RESOLVED.replace('status = "resolved"', 'status = "wontfix"')
        self.assertIn("reason", "\n".join(check_findings(fixture(ledger))))

    def test_an_id_mentioned_in_docs_must_be_registered(self):
        doc = "The review found SPR-099 and left it in prose."
        complaints = check_findings(fixture(RESOLVED, doc=doc))
        self.assertEqual(len(complaints), 1, complaints)
        self.assertIn("SPR-099", complaints[0])

    def test_duplicate_and_malformed_ids_are_rejected(self):
        twice = RESOLVED + RESOLVED
        self.assertIn("listed twice", "\n".join(check_findings(fixture(twice))))
        malformed = RESOLVED.replace('id = "BCA-01"', 'id = "bug-1"')
        self.assertIn("must match", "\n".join(check_findings(fixture(malformed))))

    def test_every_complaint_is_reported_in_one_run(self):
        ledger = RESOLVED.replace('test = "guards_it"', 'test = "nope"') + """
            [[finding]]
            id = "GAP-C01"
            title = "Dock quit"
            source = "docs/reviews/audit.md"
            status = "open"
        """
        self.assertEqual(len(check_findings(fixture(ledger))), 2)

    def test_ci_runs_the_findings_suite(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        line = next(l for l in workflow.splitlines() if "python3 -m unittest scripts.test_release_automation" in l)
        self.assertIn("scripts.test_findings", line)

    def test_the_pull_request_template_carries_a_findings_table(self):
        template = (ROOT / ".github/pull_request_template.md").read_text()
        self.assertIn("| ID | Status after this PR | Guarding test or issue |", template)
        self.assertIn("docs/reviews/findings.toml", template)


if __name__ == "__main__":
    unittest.main()
