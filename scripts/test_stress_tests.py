"""A failed or stuck stress run must not be reported as passing."""
import os
from pathlib import Path
import tempfile
import shutil
import time
import unittest

from scripts.stress_tests import run_once


class StressTests(unittest.TestCase):
    def run_command(self, command, timeout=5):
        with tempfile.TemporaryDirectory() as directory:
            log = Path(directory) / "run.log"
            status = run_once(command, directory, log, timeout, os.environ.copy())
            return status, log.read_text()

    def test_preserves_failure_status_and_output(self):
        status, output = self.run_command(["sh", "-c", "echo failure >&2; exit 7"])
        self.assertEqual(status, 7)
        self.assertEqual(output, "failure\n")

    def test_a_stalled_binary_is_killed_and_reported(self):
        status, _ = self.run_command(["sh", "-c", "sleep 30 & wait"], timeout=0.1)
        self.assertEqual(status, 124)

    def test_success_is_preserved(self):
        status, output = self.run_command(["sh", "-c", "echo passed"])
        self.assertEqual(status, 0)
        self.assertEqual(output, "passed\n")

    @unittest.skipUnless(shutil.which("tmux"), "tmux is not installed")
    def test_a_stalled_test_does_not_leave_its_tmux_server_running(self):
        status, output = self.run_command(
            ["sh", "-c", 'tmux -f /dev/null -L fixture new-session -d "sleep 30"; '
             'tmux -L fixture display-message -p "#{pid}"; sleep 30'], timeout=5)
        self.assertEqual(status, 124)
        pid = int(output.splitlines()[0])
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            try:
                os.kill(pid, 0)
            except ProcessLookupError:
                return
            time.sleep(0.01)
        self.fail(f"tmux server {pid} survived the timed-out test")
