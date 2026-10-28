"""Exercise the CI source guards, including their failure exit status."""

from pathlib import Path
import re
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]


class CiPolicyTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        step = workflow.split("      - name: Forbidden states\n", 1)[1]
        script = step.split("        run: |\n", 1)[1].split("\n  macos:", 1)[0]
        cls.script = "\n".join(line[10:] for line in script.splitlines())

    def run_gate(self, directory):
        return subprocess.run(
            ["bash", "-c", self.script],
            cwd=directory,
            capture_output=True,
            text=True,
            timeout=10,
        )

    def test_repository_passes_the_guards(self):
        result = self.run_gate(ROOT)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_every_forbidden_pattern_fails_even_before_a_later_clean_guard(self):
        cases = [
            ("src/ui.rs", "unsafe impl Send for Pane {}"),
            ("src/ui.rs", "use tokio::runtime;"),
            ("src/ui.rs", "use libghostty_vt::Terminal;"),
            ("src/ui.rs", "portable_pty::openpty();"),
            ("Cargo.toml", 'libghostty-vt = "1"'),
            ("Cargo.toml", 'portable-pty = "1"'),
        ]
        for name, source in cases:
            with self.subTest(source=source), tempfile.TemporaryDirectory() as directory:
                app = Path(directory) / "crates/sprite-app"
                (app / "src").mkdir(parents=True)
                (app / "Cargo.toml").write_text("")
                (app / name).write_text(source)
                result = self.run_gate(directory)
                self.assertEqual(result.returncode, 1, result.stdout + result.stderr)

    def test_event_driven_rule_lives_in_clippy_config(self):
        config = (ROOT / "crates/sprite-app/clippy.toml").read_text()
        for path in ["std::thread::sleep", "gpui::Timer::after", "request_animation_frame"]:
            self.assertIn(path, config)
        self.assertIn("allow-unwrap-in-tests = true", config)

    def test_every_job_has_a_time_limit(self):
        # Without one a hung test holds a hosted runner for six hours.
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        jobs = workflow.split("\njobs:\n", 1)[1]
        headers = list(re.finditer(r"(?m)^  ([a-z][a-z0-9-]*):$", jobs))
        self.assertTrue(headers, "no jobs found under jobs:")
        ends = [header.start() for header in headers[1:]] + [len(jobs)]
        for header, end in zip(headers, ends):
            with self.subTest(job=header.group(1)):
                self.assertRegex(jobs[header.end():end], r"(?m)^    timeout-minutes: (?:[1-9][0-9]*|\$\{\{ inputs\.stress && 360 \|\| [1-9][0-9]* \}\})$")

    def test_every_native_test_job_raises_the_test_watchdog(self):
        # Hosted runners can starve a busy test past the five-second default
        # without anything being stuck; macOS ran without this and flaked.
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        jobs = workflow.split("\njobs:\n", 1)[1]
        for job in ["arch", "macos"]:
            with self.subTest(job=job):
                body = re.split(r"(?m)^  [a-z][a-z0-9-]*:$", jobs.split(f"\n  {job}:\n", 1)[1], 1)[0]
                header = body.split("\n    steps:\n", 1)[0]
                self.assertRegex(header, r'(?m)^      SPRITE_TEST_WATCHDOG_SECS: "20"$')


if __name__ == "__main__":
    unittest.main()
