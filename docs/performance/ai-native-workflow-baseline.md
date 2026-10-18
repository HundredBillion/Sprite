# AI-native workflow component baseline

Issue [#102](https://github.com/HundredBillion/Sprite/issues/102), supporting
[PRD AN-01/AN-11](../PRDs/10-18-2026-ai-native-workflows.md).

Two component captures completed on 18 October 2026. These establish reproducible
local reference data, not an agent-workflow speedup or full acceptance of #102.
No terminal, authorization, observation or Surface behavior changed.

## Provenance and reproduction

- Source: `fa0131aba995b3ced1c07fb304d0bc43d7b1c28f`, clean checkout for both captures.
  Production sources, Cargo manifests/lockfile, vendor pins and build configuration
  match audited v0.2.8 (`3fbdfedf30a5ee752375c2d94c8a556ba50e706c`).
- Host: Apple M5 Pro, 18 logical CPUs, 68,719,476,736 bytes (64 GiB) RAM;
  macOS 26.5.2 build 25F84, arm64. Online built-in 3024×1964 Retina display.
- Rust/cargo 1.97.1; Zig 0.17.0; Python 3.15.0. Ghostty commit
  `92fb5d0b67b762211728975865d00eb2503df6da`.
- [First manifest](ai-native-workflow-baseline/manifest.json),
  [repeat manifest](ai-native-workflow-baseline/repeat/manifest.json), and
  [host/tool/load record](ai-native-workflow-baseline/host.json).
  Each manifest contains exact command, binary SHA-256 and UTC capture time.
- Binaries were built locally from this checkout with the locked dependencies.
  Both measured builds used the pinned Rust toolchain. The shell initially selected
  Homebrew Rust 1.98.1; that unmeasured build was superseded before capture.
  Builds succeeded with two existing GPUI numeric-fallback warnings and future
  compatibility notices for `block` and `proc-macro-error2`.
- Captures ran sequentially after builds completed, on a normal desktop rather
  than an idle laboratory host. No background application was stopped. Load
  samples are retained; process load and thermal conditions were not controlled.

```sh
git submodule update --init vendor/ghostty
PATH="$HOME/.cargo/bin:$PATH" cargo --version
PATH="$HOME/.cargo/bin:$PATH" rustc --version
zig version
PATH="$HOME/.cargo/bin:$PATH" cargo build --release --locked --offline -p sprite-term --bin sprite-term-bench
PATH="$HOME/.cargo/bin:$PATH" cargo build --release --locked --offline -p sprite-app --bin sprite-observation-bench --bin sprite-paint-bench
/usr/bin/time -l python3 scripts/capture_workflow_baseline.py --bin-dir target/release --output /tmp/sprite-workflow-baseline-native-run-1 --samples 30
/usr/bin/time -l python3 scripts/capture_workflow_baseline.py --bin-dir target/release --output /tmp/sprite-workflow-baseline-native-run-2 --samples 30
```

Use fresh output directories on subsequent runs. Python 3.11+ is required.
The driver refuses a dirty source checkout, preserves raw reports and logs,
rejects nonfinite JSON/incompatible sample counts/empty metrics, and leaves an
incomplete manifest on failure. It records executable hashes but does **not**
prove that arbitrary `--bin-dir` executables came from the current source or were
optimized: the caller must establish that with the recorded build commands.
Do not rebuild or modify the checkout while capturing. No failed measured run
preceded the two successful captures.

## Component results

Numbers below come directly from the committed JSON. R1 and R2 are consecutive
repetitions, not different implementations. Millisecond values are rounded to six
decimal places. Allocation bytes mean requested Rust allocation bytes, not RSS.
Raw JSON retains the binaries' generated `budget` fields; those are suggestions,
not accepted timing limits.

| Component / metric | Samples per run | Unit | R1 median | R1 p95 | R2 median | R2 p95 |
|---|---:|---|---:|---:|---:|---:|
| term: `spawn_to_ready` | 30 | ms | 2.132125 | 3.461584 | 1.751250 | 1.998042 |
| term: `input_to_snapshot_idle` | 30 | ms | 0.037958 | 0.072958 | 0.030750 | 0.054042 |
| term: `input_to_snapshot_under_load` | 30 | ms | 0.428500 | 0.572500 | 0.391500 | 0.485583 |
| term: `output_10mib_to_final_snapshot` | 30 | ms | 131.948000 | 137.159583 | 122.622541 | 130.887875 |
| term: `capture_100x100_grid` | 30 | ms | 0.007416 | 0.016166 | 0.005458 | 0.010875 |
| term: `capture_with_full_scrollback` | 30 | ms | 0.058417 | 0.064875 | 0.056750 | 0.059125 |
| term: `scroll_round_trip` | 30 | ms | 0.058916 | 0.064709 | 0.058666 | 0.064375 |
| term: `select_full_screen` | 30 | ms | 0.065417 | 0.075417 | 0.063666 | 0.070208 |
| term: `allocations_per_capture` | 30 | Rust allocations | 4.000000 | 4.000000 | 4.000000 | 4.000000 |
| term: `rust_bytes_per_capture` | 30 | requested Rust bytes | 3456.000000 | 3456.000000 | 3456.000000 | 3456.000000 |
| term: `isolated_projector_capture` | 30 | ms | 0.002375 | 0.003000 | 0.002416 | 0.002625 |
| observation: `collect_four_panes` | 30 | ms | 0.002208 | 0.006334 | 0.000666 | 0.006791 |
| observation: `collect_sixteen_panes` | 30 | ms | 0.006083 | 0.010833 | 0.002209 | 0.003416 |
| observation: `collect_with_one_stalled_pane` | 6 | ms | 508.268333 | 509.911667 | 506.763041 | 510.037917 |
| observation: `encode_default_request` | 30 | ms | 1.403500 | 3.631083 | 1.570958 | 3.695209 |
| observation: `encode_maximum_history` | 6 | ms | 10.071375 | 14.116541 | 9.939583 | 14.352875 |
| paint: `whole_first_frame` | 30 | ms | 0.635167 | 0.917125 | 0.285750 | 0.292459 |
| paint: `whole_same_generation_blink` | 30 | ms | 0.427083 | 0.484792 | 0.256458 | 0.274291 |
| paint: `whole_hover` | 30 | ms | 0.306000 | 0.329625 | 0.260667 | 0.277209 |
| paint: `whole_one_row_change` | 30 | ms | 0.258250 | 0.274958 | 0.259250 | 0.267958 |
| paint: `split_first_frame` | 30 | ms | 0.426959 | 0.436459 | 0.423833 | 0.439084 |
| paint: `split_same_generation_blink` | 30 | ms | 0.397042 | 0.404750 | 0.397917 | 0.409417 |
| paint: `split_hover` | 30 | ms | 0.400459 | 0.421416 | 0.399041 | 0.415250 |
| paint: `split_one_row_change` | 30 | ms | 0.397417 | 0.413166 | 0.395791 | 0.402541 |

The observation stalled-pane and maximum-history scenarios cap at six samples;
their reported p95 is a descriptive upper sample, not a robust tail estimate.
The stalled pane intentionally exercises an approximately 500 ms deadline.
Observation uses stand-in panes (4/16) and serialization fixtures; it does not
measure real terminal capture plus socket round trip. Paint uses the existing
200×60 fixture (whole/split, eight scenarios), without optional shaping mode;
it excludes native layout, text shaping, GPU submission and presentation.
Terminal throughput uses the benchmark's default 10 MiB output workload.
The legacy `capture_100x100_grid` measurement may consume an already pending
snapshot of the same generation; `capture_with_full_scrollback` may receive a
new generation from ongoing output. Both include session scheduling and do not
isolate capture work. Use `isolated_projector_capture` and its allocation metrics
for projection comparisons; do not interpret the legacy timings as pure capture
latency.

Run-to-run variation is material: whole-first-frame paint median is 0.635167 vs
0.285750 ms and four-pane broker collection is 0.002208 vs 0.000666 ms. Neither
change is an optimization result. Further controlled repetitions are needed
before choosing timing regression budgets. Existing project budgets are unchanged.

## Resource observations and evidence matrix

`/usr/bin/time -l` wrapped the entire Python capture command. Its raw outputs are
[run 1](ai-native-workflow-baseline/whole-command-resources.txt) and
[run 2](ai-native-workflow-baseline/repeat/whole-command-resources.txt).

| Whole command | Run 1 | Run 2 |
|---|---:|---:|
| Wall time (s) | 9.59 | 8.34 |
| User CPU time (s) | 5.43 | 5.08 |
| System CPU time (s) | 6.90 | 6.27 |
| Reported maximum resident set size (bytes) | 59,260,928 | 59,293,696 |

These are OS-reported whole-command resource statistics, including the capture
process and benchmark workload accounting. They are neither steady-state Sprite
CPU/RAM nor a simultaneous process-tree memory sum. They do not measure eight
live panes, GPU frame time, energy, or model costs. The raw `peak memory footprint`
field is retained without interpreting it as terminal RAM.

| Measurement | Source | Claim allowed |
|---|---|---|
| Terminal Session timing/capture | [term report](ai-native-workflow-baseline/sprite-term-bench.json) and repeat | Component timing at recorded hardware/commit |
| Broker collection/encoding | [observation report](ai-native-workflow-baseline/sprite-observation-bench.json) and repeat | Stand-in panes; excludes live socket/terminal capture |
| CPU paint preparation | [paint report](ai-native-workflow-baseline/sprite-paint-bench.json) and repeat | Excludes GPU presentation and native window layout |
| Live IPC / eight live panes | Separate native workload | Unverified until that workload is executed |
| Energy / model tasks | Platform/runtime producer | Unverified until producer-backed measurement exists |
| Other native OS | Actual native host | Unverified on this host |

## Budget decision

Freeze only the repeatable allocation invariants for the unchanged fixtures:
isolated projector capture maximum **4 Rust allocations / 3,456 requested bytes**;
whole/split same-generation paint blink maximum **0 allocations / 0 requested
bytes**. Both reports show identical maxima across all 30 samples in each run.
These are fixture-specific ceilings, not a product memory target. The existing
paint binary itself asserts zero allocation on every blink sample.

Timing, steady-state CPU/RAM, GPU, energy, token and end-to-end task budgets remain
**unaccepted** pending reliable scenario evidence. Do not wire the raw generated
1.10×p95 values into a new regression gate or silently replace existing budgets.
Allocation limits can be checked directly against the named metrics' `max` fields;
full regression-budget acceptance remains open in #102/#112.

## Public observation workload (not executed)

Use a dedicated Sprite Window built from the measured commit. Keep Surfaces off,
default configuration fixed, window focused and display refresh settings recorded.
Create 1, 4, then 8 terminal panes with the native UI; record the creation/layout
steps. Verify **each pane** is 80 columns × 24 rows using `stty size` and confirm
`size` in the returned JSON. A window pixel size alone is insufficient. If the
host display/layout cannot fit those panes, mark that scenario unexecuted.

For the fixed-content baseline, run this producer in every pane and wait for it
to finish before collecting snapshots. It emits 3,000 × 80 = 240,000 bytes per pane
before PTY newline processing. Record the actual output counters; these numbers
are fixture bytes, not total PTY traffic (shell echoes/prompts are additional).

```sh
python3 -c 'import sys,time; [(sys.stdout.write(str(i).zfill(8)+"x"*71+"\n"),sys.stdout.flush(),time.sleep(0.01)) for i in range(3000)]'
```

Run 30 sequential `sprite panes snapshot --window --lines 500` calls from one of
those same panes, using the real inherited observation environment. Do not add an
extra measurement pane to a scenario. Write each response and stderr to files
outside the terminal, without printing reports into the observed panes. Retain
UTC start, monotonic wall duration, exit code, response bytes, `complete`,
`errors`, each pane's `truncated`, `history`, `size`, and pane address. Shell
prompt/command lines must be held consistent between comparisons. Verify the
expected pane count and fail the scenario on incomplete/invalid responses.

For subsequent incremental-output comparisons, add a separately named active
variant: run the same producers while polling once per second for 30 calls;
record actual producer start/end and call timestamps. Compare equal delivered
content and completion outcomes, including replay gaps, rather than faster
responses that silently omit output. Never compare a static full-snapshot run
to an active delta run as a speedup.

This session has inherited observation credentials and an active macOS display,
but did not establish the dedicated controlled window, per-pane geometry and
running app identity. Both public workloads remain **unexecuted**. No human-pane
output or credentials are part of the committed artifacts.

## Resource and agent-task gates

- Native CPU/RAM: capture the Sprite process and its child-process identities;
  measure 60 seconds idle after warmup, then the same 1/4/8-pane producers.
  Record the OS tool, cadence, CPU normalization, RSS/physical-footprint meaning,
  and sample distribution. A sum of RSS can double-count shared memory. Python
  producers and model runtimes must be reported separately from Sprite Terminal.
- Native display: measure presented frames and input-to-present latency with the
  platform's real display tooling. Headless paint preparation cannot supply it.
  Record refresh rate, display scale, Surface state and foreground/background state.
- Energy: a real platform counter or instrument must identify measured processes,
  sample period, power source and units. No energy value is inferred from CPU time.
- Model-backed code-fix trials: select an available coding agent/backend, exact
  model revision, tokenizer revision, fixed repository commit, change prompt,
  test oracle, tool permissions, cache policy and repeat count **before** collecting
  results. Run the same task against full snapshots, delta output and structured
  results; retain pass/fail, wall time, input/output tokens and retries. No backend
  or task fixture was selected/executed in this component baseline; that acceptance
  item remains open. Bytes are never reported as tokens.
- Native Linux (owner: David, on his separate Linux machine): repeat on an actual
  Linux host and record Wayland/X11 display
  evidence. macOS data, a headless CI job or an emulated display cannot satisfy it.

The owner approved continuing development with these gaps visible. #102 and #112
remain open until their required evidence and justified budgets are accepted.
