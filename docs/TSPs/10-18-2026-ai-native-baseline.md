# AI-native workflow baseline Technical Spec

> **For agentic workers:** REQUIRED SUB-SKILL: Use dmi-superpowers:subagent-driven-development (recommended) or dmi-superpowers:executing-plans to implement this TSP task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** Give issue #102 reproducible baseline evidence and an explicit inventory of measurements that still require native platform or agent-runtime access.

**Architecture:** Reuse existing release benchmark binaries and their report formats. A small standard-library capture driver records provenance and raw reports; native display and agent-task experiments remain separately named workloads so their absence cannot be mistaken for a passing component benchmark.

**Tech Stack:** Rust 1.97.1, existing benchmark binaries, Python 3.11+ standard library, Git, native macOS/Linux.

**Status:** PRD and scheduling decision approved; capture driver, repeated component baselines and workload recipes implemented; independent review in progress. No numerical budget is accepted merely by writing this plan.

**References:** [PRD](../PRDs/10-18-2026-ai-native-workflows.md), [source inventory](../reviews/10-18-2026-ai-native-source-audit.md), [#102](https://github.com/HundredBillion/Sprite/issues/102).

## Global Constraints

- Support macOS and Linux.
- Use Rust 1.97.1 and the repository's pinned dependency versions.
- Sprite Terminal must remain fully functional without an agent, inference engine, replacement shell, or CPR tooling.
- Sprite Terminal should target the lowest practical CPU, RAM, and energy use during idle and active workloads, with measured improvements that preserve correctness, responsiveness, and required functionality.
- Reuse the existing Terminal Session, pane layout, local Unix-socket transport, bounded channels, and Surface modules unless measured evidence establishes a deficiency.
- Keep model execution, model weights, inference kernels, context assembly, and compiler semantics outside Sprite Terminal's rendering hot path.
- Preserve supported observation CLI/JSON and Surface Channel compatibility; new authority must not be inferred from legacy credentials.
- Never block the GPUI thread on socket I/O, process cleanup, model execution, or subscriber consumption.
- Bound retained output, subscriptions, queues, audit records, and connection resources; expose saturation, truncation, and replay gaps explicitly.
- Retain one libghostty-owning worker per Terminal Session and the existing safe ownership rules.
- Do not add HTTP or binary serialization without evidence that the existing Unix-socket and JSON approach is insufficient.

---

## Design review decision

The owner approved collecting the macOS component baseline now and proceeding
with development while unavailable native Linux/display/energy and model-backed
task evidence remains open. Neither #102 nor #112 is fully accepted until its
required evidence exists. This changes scheduling, not measurement requirements.

Source review confirms all three binaries accept the planned arguments and expose
`sample_count` and nonempty `metrics`; paint nests timing/allocation metrics.
Capture validation checks that common envelope without duplicating each binary's
schema tests. Use vertical executable-contract tests, retain timeout/failure logs,
and refuse pre-existing reports so a successful process cannot validate stale data.
The manifest records failure details as well as successful runs. Preserve both
repetitions so variability claims are independently inspectable. These are routine
reliability refinements to the approved capture interface.

## File structure and testing seam

- Create `scripts/capture_workflow_baseline.py`: run existing binaries sequentially, retain their raw JSON/logs and record provenance; no new benchmark engine.
- Create `scripts/test_workflow_baseline.py`: exercise the capture driver's process/report contract using a temporary executable, including failure and invalid report cases.
- Create `docs/performance/ai-native-workflow-baseline.md`: repeatable native/headless workloads, limitations, hardware identity and the evidence matrix.
- Create `docs/performance/ai-native-workflow-baseline/manifest.json` and raw report files only after a valid measured run; copy only the deliberate report artifacts, never build output or credentials.
- Modify `docs/reviews/10-18-2026-ai-native-source-audit.md` only if the measurement checkout exposes a factual inventory change.

The capture-driver test seam is the executable/report interface. Product acceptance
remains the public Sprite CLI/protocol; this driver does not replace that seam.

### Task 1: Capture trustworthy component reports

**Blocked by:** None.

**Files:**
- Create: `scripts/capture_workflow_baseline.py`
- Test: `scripts/test_workflow_baseline.py`

**Interfaces:**
- Consumes: existing `sprite-term-bench`, `sprite-observation-bench`, `sprite-paint-bench` executables accepting `--samples N --output PATH`.
- Produces: `capture(binary: Path, output: Path, samples: int) -> dict`, which writes the raw JSON and adjacent process log, validates a nonempty report, and returns binary/command/time provenance. CLI: `--bin-dir PATH --output PATH --samples N`.

- [x] **Step 1: Write the failing executable-contract tests.**

Create `scripts/test_workflow_baseline.py`:

```python
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
```

- [x] **Step 2: Run the new tests before implementation.**

Run: `python3 -m unittest scripts.test_workflow_baseline -v`.
Expected: import failure because the capture driver does not exist. Record it;
this is a test setup red, followed by behavioral negative cases in the completed
suite. No production behavior is being modified.

- [x] **Step 3: Implement the minimal capture driver.**

Create `scripts/capture_workflow_baseline.py`:

```python
import argparse
import hashlib
import json
import platform
import subprocess
from datetime import datetime, timezone
from pathlib import Path


def reject_constant(value):
    raise ValueError('nonfinite JSON value: ' + value)


def capture(binary: Path, output: Path, samples: int) -> dict:
    binary = binary.resolve(strict=True)
    command = [str(binary), '--samples', str(samples), '--output', str(output.resolve())]
    result = subprocess.run(command, capture_output=True, text=True, timeout=1800)
    output.with_suffix('.log').write_text(result.stdout + result.stderr)
    if result.returncode:
        raise RuntimeError(str(binary) + ' exited ' + str(result.returncode))
    report = json.loads(output.read_text(), parse_constant=reject_constant)
    json.dumps(report, allow_nan=False)
    if not isinstance(report, dict) or report.get('sample_count') != samples or not isinstance(report.get('metrics'), dict) or not report['metrics']:
        raise ValueError('missing metrics or incompatible sample count')
    with binary.open('rb') as binary_file:
        digest = hashlib.file_digest(binary_file, 'sha256').hexdigest()
    return {'binary': str(binary), 'sha256': digest,
            'command': command, 'report': output.name,
            'captured_at': datetime.now(timezone.utc).isoformat()}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--bin-dir', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--samples', type=int, default=30)
    args = parser.parse_args()
    if not 30 <= args.samples <= 100000:
        parser.error('--samples must be between 30 and 100000')
    repo = Path(__file__).resolve().parents[1]
    def git(*arguments):
        return subprocess.check_output(['git', '-C', str(repo), *arguments], text=True).strip()
    if git('status', '--porcelain'):
        parser.error('commit the measured checkout before capture')
    args.output.mkdir(parents=True, exist_ok=False)
    manifest = {'schema': 1, 'source_commit': git('rev-parse', 'HEAD'),
                'platform': platform.platform(), 'machine': platform.machine(),
                'python': platform.python_version(), 'samples': args.samples,
                'profile': 'caller-built; see recorded release build commands', 'runs': [], 'status': 'incomplete',
                'unmeasured': ['native GPU presentation', 'live IPC round trip',
                               'eight live pane CPU/RAM', 'energy', 'model-backed task time/tokens']}
    try:
        for name in ('sprite-term-bench', 'sprite-observation-bench', 'sprite-paint-bench'):
            manifest['runs'].append(capture(args.bin_dir / name, args.output / (name + '.json'), args.samples))
        manifest['status'] = 'component-capture-complete'
    finally:
        (args.output / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')


if __name__ == '__main__':
    main()
```

- [x] **Step 4: Verify positive and negative process/report behavior.**

Run: `python3 -m unittest scripts.test_workflow_baseline -v`.
Expected: eight tests pass after the design-review refinements, including subprocess failure, nonfinite JSON, missing
metrics and mismatched sample count. Then run `git diff --check`.

- [x] **Step 5: Commit the capture driver and its tests.**

Run: `git add scripts/capture_workflow_baseline.py scripts/test_workflow_baseline.py`.
Run: `git commit -m "test: capture reproducible workflow baseline reports"`.

### Task 2: Run the existing release benchmarks and preserve provenance

**Blocked by:** Task 1.

**Files:**
- Create: `docs/performance/ai-native-workflow-baseline/manifest.json`
- Create: `docs/performance/ai-native-workflow-baseline/sprite-term-bench.json`
- Create: `docs/performance/ai-native-workflow-baseline/sprite-observation-bench.json`
- Create: `docs/performance/ai-native-workflow-baseline/sprite-paint-bench.json`
- Create: `docs/performance/ai-native-workflow-baseline.md`

**Interfaces:**
- Consumes: Task 1 CLI and existing release benchmark JSON.
- Produces: measured component baseline and evidence matrix; no claim about unmeasured live/UI/agent outcomes.

- [x] **Step 1: Build the measured checkout without reusing unverified binaries.**

Run from the isolated checkout:

```sh
git submodule update --init vendor/ghostty
PATH="$HOME/.cargo/bin:$PATH" cargo build --release --locked --offline -p sprite-term --bin sprite-term-bench
PATH="$HOME/.cargo/bin:$PATH" cargo build --release --locked --offline -p sprite-app --bin sprite-observation-bench --bin sprite-paint-bench
```

Expected: successful release builds at the recorded source commit. If offline
dependencies are missing, report that condition before choosing an online fetch;
do not silently measure an older executable from another checkout.

- [x] **Step 2: Capture at least 30 samples with the exact driver command.**

```sh
python3 scripts/capture_workflow_baseline.py --bin-dir target/release --output /tmp/sprite-workflow-baseline-native-run-1 --samples 30
```

Expected: three nonempty reports and a manifest with
`status: component-capture-complete`. The output directory must be fresh; retain
an earlier run rather than overwrite it. Repeat under a new output directory to
check gross variability before accepting timings as a baseline.

- [x] **Step 3: Record hardware/tool identity and bounded resource observations.**

On macOS run `sw_vers`, `sysctl -n machdep.cpu.brand_string`,
`sysctl -n hw.memsize`, `cargo --version`, `rustc --version`, and `zig version`.
On Linux use `uname -a`, `lscpu`, `cat /proc/meminfo`, and the same tool commands.
Record display availability, competing load and exact binaries/commands. Do not
call whole-command resource use steady-state terminal CPU or GPU frame timing.

- [x] **Step 4: Preserve reports and write the evidence matrix.**

Copy only the manifest and three raw reports to the listed repository directory.
In the Markdown report use these exact evidence categories:

| Measurement | Source | Claim allowed |
|---|---|---|
| Terminal Session timing/capture | term report | Component timing at recorded hardware/commit |
| Broker collection/encoding | observation report | Stand-in panes; excludes live socket/terminal capture |
| CPU paint preparation | paint report | Excludes GPU presentation and native window layout |
| Live IPC / eight live panes | Separate native workload | Unverified until that workload is executed |
| Energy / model tasks | Platform/runtime producer | Unverified until producer-backed measurement exists |
| Other native OS | Actual native host | Unverified on this host |

Populate values directly from reports, include actual hardware/commit, and list
failed or missing runs. Retain individual report metric sample counts: some
existing expensive scenarios cap their sample count even when the run asks for
30. Their p95 is descriptive, not a robust tail estimate.

- [x] **Step 5: Commit only inspected report artifacts and reproduction notes.**

Run: `git diff --check`.
Run: `git add docs/performance/ai-native-workflow-baseline.md docs/performance/ai-native-workflow-baseline`.
Run: `git commit -m "docs: record native workflow component baseline"`.

### Task 3: Fix the public observation workload and remaining acceptance gates

**Blocked by:** Task 2.

**Files:**
- Modify: `docs/performance/ai-native-workflow-baseline.md`
- Modify: `docs/TSPs/10-18-2026-ai-native-delivery-map.md`

**Interfaces:**
- Consumes: baseline artifacts and existing `sprite panes snapshot --window --lines 500` public CLI.
- Produces: named reproducible workload recipe, budget acceptance decision, and explicit open evidence for #102/#112.

- [x] **Step 1: Specify the existing-observation workload exactly.**

Use a dedicated Sprite Window, a controlled child emitting numbered 79-character
lines, 80×24 cells, 500 requested history rows, 1/4/8 panes, 30 snapshot calls per
scenario, and the same default configuration and Surface state per comparison.
Record actual emitted bytes, complete/truncated snapshot flags, response bytes,
wall time, CPU/RAM method and binary/source identity. The snapshot client runs
inside the target window and uses its actual inherited credentials. A plain host
shell without those credentials is not a successful IPC measurement.

Use this exact producer command in each measurement pane:

```sh
python3 -c 'import sys,time; [(sys.stdout.write(str(i).zfill(8)+"x"*71+"\n"),sys.stdout.flush(),time.sleep(0.01)) for i in range(3000)]'
```

Use `sprite panes snapshot --window --lines 500` from a pane in that window.
Retain raw outputs outside the terminal so observing the result does not itself
inflate the next snapshot. Record pane creation and window configuration. Mark
the workload unexecuted if native launch, repeatable geometry, or identity cannot
be established; do not substitute broker stand-ins.

- [x] **Step 2: Define fair future comparisons and freeze justified budgets.**

Use identical input/output/task fixtures across snapshots, incremental events,
and structured results. Record the actual tokenizer/model revision before
counting tokens; bytes alone are not token savings. Choose an available coding
agent/backend and fixed repository fixture before recording code-fix task time.
Record numeric accepted budgets only from repeated measured scenarios, with
sample-count caveats and provenance; leave unavailable scenario budgets explicitly
unaccepted. Do not automatically adopt every benchmark's generated 1.10×p95
suggestion, particularly for capped six-sample scenarios.

- [ ] **Step 3: Review evidence and update issue readiness.**

Run `python3 -m unittest scripts.test_workflow_baseline -v` and `git diff --check`.
Obtain independent review of the driver, measured provenance and report claims.
Link the PR, TSP, and artifacts from #102. Keep every missing native/platform/task
measurement visibly open and apply the design-review scheduling decision before
starting #103. Only close #102 when its acceptance criteria are actually met.

Run: `git add docs/performance/ai-native-workflow-baseline.md docs/TSPs/10-18-2026-ai-native-delivery-map.md`.
Run: `git commit -m "docs: define workflow comparison and acceptance gates"`.

## Self-review

- AN-01 is covered by source inventory plus Tasks 1–3. AN-11 is prepared here;
  final comparisons belong to #112, not this baseline capture.
- No terminal, IPC, authorization, Surface or model behavior changes in this TSP.
- Future unavailable hardware/model evidence is an explicit open acceptance item,
  not a fabricated implementation detail or a silently passing check.
- One functional helper and one CLI are sufficient; no framework, new runtime,
  dependency, or pluggable benchmark abstraction is introduced.
- The scheduling decision above is approved; missing evidence remains an acceptance gate.
