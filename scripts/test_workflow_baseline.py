import json
import tempfile
import unittest
from pathlib import Path

from scripts.capture_workflow_baseline import capture


class CaptureTests(unittest.TestCase):
    def fixture(self, directory, body):
        binary = directory / 'fixture'
        binary.write_text('#!/usr/bin/env python3\n' + body)
        binary.chmod(0o700)
        return binary

    def test_keeps_raw_report_and_binary_identity(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            binary = self.fixture(root, "import json,sys\nfrom pathlib import Path\nPath(sys.argv[sys.argv.index('--output')+1]).write_text(json.dumps({'sample_count':30,'metrics':{'capture':{'median':1}}}))\n")
            result = capture(binary, root / 'report.json', 30)
            self.assertEqual(json.loads((root / 'report.json').read_text())['sample_count'], 30)
            self.assertEqual(len(result['sha256']), 64)
            self.assertEqual(result['command'][1:3], ['--samples', '30'])
            self.assertTrue((root / 'report.log').is_file())

    def test_refuses_process_failure_and_nonfinite_json(self):
        cases = [
            'raise SystemExit(7)\n',
            "import sys\nfrom pathlib import Path\nPath(sys.argv[sys.argv.index('--output')+1]).write_text('{\"sample_count\":30,\"metrics\":{\"x\":NaN}}')\n",
        ]
        for body in cases:
            with self.subTest(body=body), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                with self.assertRaises((RuntimeError, ValueError)):
                    capture(self.fixture(root, body), root / 'report.json', 30)

    def test_refuses_empty_or_wrong_sample_report(self):
        for report in ({'sample_count': 30, 'metrics': {}}, {'sample_count': 1, 'metrics': {'x': 1}}):
            with self.subTest(report=report), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                body = "import sys\nfrom pathlib import Path\nPath(sys.argv[sys.argv.index('--output')+1]).write_text(" + repr(json.dumps(report)) + ")\n"
                with self.assertRaises(ValueError):
                    capture(self.fixture(root, body), root / 'report.json', 30)

    def test_refuses_stale_report_without_running_binary(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            report = root / 'report.json'
            original = '{"sample_count":30,"metrics":{"x":1}}'
            report.write_text(original)
            binary = self.fixture(root, "print('must not run')\n")
            with self.assertRaises(FileExistsError):
                capture(binary, report, 30)
            self.assertEqual(report.read_text(), original)
            self.assertFalse(report.with_suffix('.log').exists())

    def test_timeout_retains_process_log(self):
        import subprocess
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            binary = self.fixture(root, "import time\nprint('started', flush=True)\ntime.sleep(30)\n")
            with self.assertRaises(subprocess.TimeoutExpired):
                capture(binary, root / 'report.json', 30, timeout=1)
            self.assertIn('started', (root / 'report.log').read_text())


class CliTests(unittest.TestCase):
    def run_cli(self, root, *args):
        import subprocess
        import sys
        return subprocess.run([sys.executable, str(root / 'scripts/capture_workflow_baseline.py'),
                               '--bin-dir', str(root / 'bin'), '--output', str(root / 'result'),
                               *args], capture_output=True, text=True)

    def prepare_repo(self, root, failing=False):
        import shutil
        import subprocess
        (root / 'scripts').mkdir()
        shutil.copy(Path(__file__).with_name('capture_workflow_baseline.py'), root / 'scripts')
        (root / 'bin').mkdir()
        for name in ('sprite-term-bench', 'sprite-observation-bench', 'sprite-paint-bench'):
            binary = root / 'bin' / name
            body = "import json,sys\nfrom pathlib import Path\nPath(sys.argv[sys.argv.index('--output')+1]).write_text(json.dumps({'sample_count':30,'metrics':{'x':1}}))\n"
            if failing and name == 'sprite-observation-bench':
                body = "print('deliberate failure')\nraise SystemExit(7)\n"
            binary.write_text('#!/usr/bin/env python3\n' + body)
            binary.chmod(0o700)
        for args in (['init', '-q'], ['add', '.'],
                     ['-c', 'user.name=Test', '-c', 'user.email=test@example.invalid',
                      '-c', 'commit.gpgsign=false', '-c', 'core.hooksPath=/dev/null',
                      'commit', '-qm', 'fixture']):
            subprocess.run(['git', '-C', str(root), *args], check=True, capture_output=True)

    def test_cli_retains_partial_manifest_on_failure(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.prepare_repo(root, failing=True)
            result = self.run_cli(root)
            self.assertNotEqual(result.returncode, 0)
            manifest = json.loads((root / 'result/manifest.json').read_text())
            self.assertEqual(manifest['status'], 'incomplete')
            self.assertEqual(len(manifest['runs']), 1)
            self.assertIn('sprite-observation-bench', manifest['error'])
            self.assertIn('deliberate failure', (root / 'result/sprite-observation-bench.log').read_text())
            self.assertFalse((root / 'result/sprite-paint-bench.json').exists())

    def test_cli_success_records_provenance_and_refuses_overwrite(self):
        import subprocess
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.prepare_repo(root)
            result = self.run_cli(root)
            self.assertEqual(result.returncode, 0, result.stderr)
            path = root / 'result/manifest.json'
            original = path.read_text()
            manifest = json.loads(original)
            commit = subprocess.check_output(['git', '-C', str(root), 'rev-parse', 'HEAD'], text=True).strip()
            self.assertEqual(manifest['source_commit'], commit)
            self.assertEqual(manifest['status'], 'component-capture-complete')
            self.assertEqual(len(manifest['runs']), 3)
            self.assertTrue(manifest['unmeasured'])
            # Ignore the result to exercise the fresh-directory guard on a clean repo.
            (root / '.git/info/exclude').write_text('result/\n')
            self.assertNotEqual(self.run_cli(root).returncode, 0)
            self.assertEqual(path.read_text(), original)

    def test_cli_rejects_dirty_checkout_and_invalid_samples(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.prepare_repo(root)
            for samples in ('0', '29', '100001'):
                self.assertNotEqual(self.run_cli(root, '--samples', samples).returncode, 0)
                self.assertFalse((root / 'result').exists())
            (root / 'untracked').write_text('dirty')
            result = self.run_cli(root)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn('commit the measured checkout', result.stderr)
            self.assertFalse((root / 'result').exists())
