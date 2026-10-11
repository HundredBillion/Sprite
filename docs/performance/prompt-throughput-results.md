# Prompt startup and throughput results

Implemented on `perf/prompt-throughput`, based on master `afece5293271b410d83c63e236938ff2a7e0a250`.
The changes remove repeated work before starting and displaying a shell. They do
not change shell initialization, login arguments, or the user's startup files.
A shell still has to emit its real directory prompt before Sprite can draw it.

## What changed

| Original review finding | Result | Regression evidence |
| --- | --- | --- |
| Repeated font discovery | One lazy catalog per app, refreshed on successful explicit settings reload. Warm panes reuse it. | Selection sequence fell from 18 enumerations to 1; unchanged-settings reload refreshes existing panes; failed reload and equivalent resolved fonts preserve metrics. |
| `Ready` benchmark misses prompt delay | Retained controlled directory-prompt test; added actual grid CPU-paint marker tests and opt-in native benchmark. | Blank/Ready cannot satisfy prompt detection; delayed-child and incorrect-marker negative controls fail. |
| Too much snapshot projection | Measured a demand-driven candidate, then removed it. Existing scheduling remains. | Retained first/quiet/input/final marker and paused-consumer coverage, plus hidden-view installation without painting. |
| Refused images converted repeatedly | Validate dimensions, source length, checked BGRA byte size, and budget before conversion. | Repeated oversized requests perform zero conversions; malformed, overflow, exact-fit, generation replacement, and collective LRU cases covered. |
| Quadratic image membership | Use sets for placement deduplication and cache retention; retain sorted output and shared pixels. | Production work-count regression, repeated placements, deletion, and pixel identity checks. |
| Socket cleanup delays opening | Existing listener loop sweeps at most one entry per turn; nonblocking datagram probes avoid admitting connections to live stream backlogs. | Real saturated-backlog tests protect live paths; stale paths removed; caller performs no directory traversal. |
| New panes start at 24×80 | Compute prospective startup/tab/split allocation before spawning the child. | Real `stty size` plus immutable spawn-config assertion covers fonts, padding, nested splits, moved dividers, zoom, and resized windows. |
| Content updates probe foreground titles | Use the existing 530 ms window clock; OSC titles remain immediate and close warnings still query fresh state. | Four content updates added zero foreground queries after repair; title and close-safety tests retained. |
| Abandoned history requests still traverse | Owned cancellation handle skips queued work before lookup/traversal. Already-started coherent work may finish. | Real held worker and app abandonment tests cover withdrawal, window removal, failures, pressure, and live ticket correlation. |
| Quadratic workspace reconciliation | Sets/maps replace repeated membership scans while preserving order, subscriptions, and allocation suppression. | At 4/8/16 active plus inactive panes, title work 36/136/528 → 16/32/64 and placement work 10/36/136 → 8/16/32. |

Each repair followed an observed failing regression before the production change.
Tests count actual work where possible; CI does not assert shared-host millisecond
budgets. Public wire protocol v1, bounded queues, lossless lifecycle events, and
the existing uncancellable history command remain compatible.

Linux integration also exposed long graphics commands truncated by the canonical
TTY input limit. Three test-shell initializers now disable canonical input and
echo before acknowledging READY. Assertions, payloads, and watchdogs are unchanged;
production terminal modes are unchanged. Direct PTY reproduction and a rerun with
master's graphics implementation confirmed the fixture problem. The container's
missing `less` dependency was installed there; CI already declares it.

## Measured performance

On native macOS, the controlled new-tab/split marker reached CPU paint in a
median **12.67 ms instead of 79.33 ms** with idle existing panes, an approximately
**84% reduction**. The first workspace still performs one cold font discovery;
its median fell from **101.72 ms to 65.74 ms** in these samples.

| Native CPU-paint scenario | Samples per version | Before median / p95 | After median / p95 |
| --- | ---: | ---: | ---: |
| First workspace, idle child | 6 | 101.72 / 158.26 ms | 65.74 / 70.33 ms |
| New tab or split, idle existing panes | 36 | 79.33 / 81.60 ms | 12.67 / 13.53 ms |
| First workspace, periodic-output child | 6 | 99.29 / 115.38 ms | 66.18 / 71.24 ms |
| New tab or split, periodic-output existing panes | 36 | 79.55 / 90.61 ms | 12.62 / 13.60 ms |

The comparison alternated before/after order over six fresh processes per version
and scenario. Each process opened the first workspace, then six tabs/splits through
real action dispatch, using the same controlled child and identical measurement
module. All 24 processes reported seven markers and exited successfully. The
before binary is original master plus only the benchmark instrumentation; the
after binary is production commit `74bce48` plus that same instrumentation. Later
changes are test fixtures/documentation only. Binary identities and raw samples
are retained in `native-binaries.json` and `native-measurements.json`.

Periodic output means one small child write every 10 ms in previously opened
panes (up to six); it is not a saturated-output stress test. There are no existing
panes in the first-workspace sample. These are small descriptive distributions:
startup p95 is the maximum of six samples, and tab/split samples share processes.
The 5 ms manual draw scheduling and native text system are included. Process
launch, compositor presentation, and personal shell initialization are excluded.
The combined change is measured; the table does not isolate font caching from
the other repairs.

The separate engine-only clean Bash probe (20 samples per scenario) measured
directory-prompt snapshot p95 **5.21 ms idle / 7.15 ms with four output-producing
sessions**; Ready p95 was 2.44 / 4.17 ms. The 250 ms delayed-child control produced
Ready at 4.78 ms and its prompt at 296.00 ms, confirming the endpoint includes
shell initialization. The original review's engine p95 was 4.76 / 7.80 ms;
these nonpaired runs do not establish an engine speedup or regression. This
probe excludes font/layout/paint work and pauses the background snapshot/event
receivers, unlike the native GUI experiment.

The image capture probe took **41.669 ms before and 12.814 ms after** for 100
captures with 512 distinct images and four placements per image. This is one
release-mode probe of graphics capture, not a prompt-latency distribution.

Snapshot pacing was deliberately **not shipped**. The corrected candidate saved
about 3% projection CPU at 80×24 and 17% at 100×100 in the controlled workload;
small-grid no-frame results were worse. That did not justify new scheduling and
delivery contracts. These percentages describe a rejected experiment, not gains
in this branch. The retained opt-in probe compares immediate, simulated 16 ms,
and paused consumption with exact bytes and final-marker checks.

## Verification and reproduction

The macOS workspace test suite, workspace all-target Clippy with warnings denied,
benchmark-feature all-target Clippy, formatting, and workspace build passed.
Python release/packaging/shell/CI/GPUI policy checks passed (35 tests, four skipped
for optional host dependencies/platform). After the test-only Linux fixture
follow-up, native covering graphics tests and terminal all-target Clippy/formatting
passed again. The broad workspace gates preceded that fixture-only follow-up;
production code did not change afterward.

The emulated x86_64 Ubuntu 24.04 terminal suite passed every target except one
intermittently hung palette process. That process was killed after inspection;
an immediate bounded retry of its unchanged binary passed all five tests in
0.16 seconds. The full invocation therefore exited 101, not zero. No cause was
established for that hang. All graphics/history targeted tests passed after the
fixture repair. Linux socket-module coverage passed serially; an earlier parallel
run hit an existing handshake timing assertion. This is emulated Linux coverage,
not a native Linux GUI verification.

Use Rust/Cargo 1.97.1 and the pinned submodule/dependencies. Native measurements
use release builds. The feature-gated benchmark and worker instrumentation add no
default-build tracing or recurring timer.

```sh
PATH="$HOME/.cargo/bin:$PATH" cargo +1.97.1 test --workspace --locked --offline --no-fail-fast
PATH="$HOME/.cargo/bin:$PATH" cargo +1.97.1 clippy --workspace --all-targets --locked --offline -- -D warnings
PATH="$HOME/.cargo/bin:$PATH" cargo +1.97.1 clippy -p sprite-app --all-targets --features startup-benchmark --locked --offline -- -D warnings
PATH="$HOME/.cargo/bin:$PATH" cargo +1.97.1 fmt --all -- --check
PATH="$HOME/.cargo/bin:$PATH" cargo +1.97.1 build --workspace --locked --offline
python3 -m unittest scripts.test_release_automation scripts.test_packaging_automation scripts.test_shell_integration scripts.test_ci_policy scripts.test_gpui_patch -v

PATH="$HOME/.cargo/bin:$PATH" cargo +1.97.1 test -p sprite-term --test prompt_latency --release --locked --offline -- --include-ignored --nocapture
PATH="$HOME/.cargo/bin:$PATH" cargo +1.97.1 test -p sprite-term --release --lib worker::cadence_tests::bounded_release_probe --locked --offline -- --ignored --exact --nocapture
PATH="$HOME/.cargo/bin:$PATH" cargo +1.97.1 run -p sprite-app --bin sprite-startup-bench --features startup-benchmark --release --locked --offline -- --manual-draw
# Add --busy after --manual-draw for periodic output in previously opened panes.
```

The native paint harness uses a controlled marker child, bounded records, and a
supervisor watchdog. Automatic frame delivery stalled in this environment after
the initial blank draw even though snapshots were installed. `--manual-draw`
explicitly drives native GPUI drawing, polling every 5 ms. Its endpoint is the
actual terminal-grid CPU paint, not GPU completion, automatic frame scheduling,
OS presentation, or the user's personal-shell prompt. Startup timing starts at
workspace construction and excludes earlier process/window creation.

Linux font discovery retains GPUI's existing in-memory database: installing new
OS fonts can still require restart. Explicit reload refreshes the application
catalog against what the platform currently exposes.

Raw reports, red/green logs, benchmark binaries with SHA-256 identities, and the
archived rejected pacing experiment are in the local evidence directory
`/Users/davidlee/pr-reviews/sprite-prompt-improvements/`. Independent task reviews
and the whole-change review found no remaining actionable correctness defect;
the native-presentation limitation above remains unmeasured.
