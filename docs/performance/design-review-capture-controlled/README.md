# Controlled capture comparison

This one predefined comparison retains the preserved pre-split binary A
(`b0aae2d75a77f5efc19476f198cb64c8497c8818`) and final binary B (the final
`b3ab5a4` implementation, reproduced byte-for-byte before the documentation
commits). [protocol.json](protocol.json) records SHA-256 identities, exact commands,
fixed order, elapsed times, exit codes, local log paths and environment observations.

All runs used CPU 2 affinity, `TERM=xterm-ghostty`, the original workspace cwd,
30 samples and default 10 MiB output. The preserved CLI has no capture-only filter:
all session workloads run before the isolated capture fixture. That fixture
performs exactly one unmeasured capture per fresh driver before each timed capture.
Whole-process warmups A then B were designated before execution and excluded
from summaries. Measured order was ABBA BAAB. No report was discarded or rerun.

The eight measured reports contain 120 isolated samples per binary. Every one
of the ten invocations exited 0. Allocation and Rust-byte median/p95/max
summaries remained 104 allocations and 14,656 bytes throughout. The original
shared timing budget remains **0.017937 ms**; generated report budgets are not
adopted. Bold p95 values below exceed that original budget.

| Phase / order | Binary | Median (ms) | p95 (ms) | Max (ms) | Shared checker exit |
| --- | --- | ---: | ---: | ---: | ---: |
| [warmup 1](warmup-01-A.json) | A | 0.013992 | **0.023214** | 0.025661 | 1 |
| [warmup 2](warmup-02-B.json) | B | 0.013682 | **0.020601** | 0.028541 | 1 |
| [measured 1](measured-01-A.json) | A | 0.014197 | 0.017916 | 0.026026 | 0 |
| [measured 2](measured-02-B.json) | B | 0.012824 | 0.017422 | 0.022530 | 0 |
| [measured 3](measured-03-B.json) | B | 0.013407 | **0.025291** | 0.035517 | 1 |
| [measured 4](measured-04-A.json) | A | 0.013609 | 0.014973 | 0.032797 | 0 |
| [measured 5](measured-05-B.json) | B | 0.013519 | **0.022432** | 0.022655 | 1 |
| [measured 6](measured-06-A.json) | A | 0.013800 | 0.017446 | 0.018627 | 0 |
| [measured 7](measured-07-A.json) | A | 0.014179 | **0.019941** | 0.020892 | 1 |
| [measured 8](measured-08-B.json) | B | 0.014382 | **0.028868** | 0.124903 | 1 |

Ratios use B divided by A in each adjacent pair; values above 1 mean B is slower.

| Measured indices | Order | Median ratio | p95 ratio | Max ratio |
| --- | --- | ---: | ---: | ---: |
| 1–2 | AB | 0.903289 | 0.972427 | 0.865673 |
| 3–4 | BA | 0.985157 | 1.689107 | 1.082934 |
| 5–6 | BA | 0.979638 | 1.285796 | 1.216245 |
| 7–8 | AB | 1.014317 | 1.447671 | 5.978509 |

| Summary statistic | A | B |
| --- | ---: | ---: |
| Median of four run median values (ms) | 0.0139895 | 0.0134630 |
| Range of four run median values (ms) | 0.013609–0.014197 | 0.012824–0.014382 |
| Median of four run p95 values (ms) | 0.0176810 | 0.0238615 |
| Range of four run p95 values (ms) | 0.014973–0.019941 | 0.017422–0.028868 |
| Median of four run max values (ms) | 0.0234590 | 0.0290860 |
| Range of four run max values (ms) | 0.018627–0.032797 | 0.022530–0.124903 |

These are medians of run summaries, **not pooled percentiles**: the preserved
harness emits no raw sample series. The median of the four paired B/A ratios
is 0.982397 for medians (−1.8%), 1.366733 for p95s (+36.7%), and 1.149590 for maxima.

The earlier typical-median slowdown did not recur, but the tail concern persists:
final p95 exceeds baseline in three of four pairs, including both AB and BA
orderings. Three of four measured B runs and one of four measured A runs exceed
the absolute shared budget. The last B maximum of 0.124903 ms remains in the record.
This is evidence requiring review; it does not isolate a code cause, dismiss the
regression as noise, or pass the timing gate. No further measurement was run.

Observed environment limits: powersave governor unchanged; CPU 2 shares a core
with CPU 10, whose activity was uncontrolled. Load averages changed from
1.49/1.37/1.74 to 4.21/2.91/2.30. Read-only frequency snapshots were 4.200 GHz
before and 0.853 GHz after; they are not measurements of frequency during capture.
No governor, SMT, scheduling-policy or system configuration was changed.
The full sequence took approximately 308 seconds. No concurrent builds or tests
were launched by this measurement task; unrelated machine activity was not controlled.

Runner: `/tmp/sprite-task11-controlled-capture.py`; combined log:
`/tmp/sprite-task11-controlled-capture.log`; per-run and checker logs:
`/tmp/sprite-task11-capture-controlled/`. All prior failures and reports remain
in [the issue proof record](../design-review-coverage.md).


## Focused capture-path assessment

Compared with pre-split `b0aae2d`, there is no source diff in
[snapshot.rs](../../../crates/sprite-term/src/snapshot.rs),
[capture_benchmark.rs](../../../crates/sprite-term/src/capture_benchmark.rs),
[graphics.rs](../../../crates/sprite-term/src/graphics.rs), the
[benchmark driver](../../../crates/sprite-term/src/bin/sprite-term-bench.rs), or
[counting allocator](../../../crates/sprite-term/src/test_allocations.rs).
Cargo manifests and lockfile are also unchanged over this task. The moved render
and pane definitions preserve their source bodies; the render module only adds
imports while omitting intervening unrelated old-root definitions.

The timed closure calls `CaptureBenchmark::capture`, then destroys its returned
bundle before reading elapsed time. It directly calls `Projector::capture` with
the same generation, dimensions and selection flag. There is no worker, PTY,
message dispatch, input encoder, hyperlink lookup, or session shutdown in this
synchronous call. The Task 11 worker restructuring and shutdown fix therefore do
not add direct work to this timed path. Constructor validation and the initial
warmup capture are outside the timer. The existing 104 Rust allocations/14,656
requested bytes show no added measured Rust allocation work; they do not count
native allocations or prove equal allocation/deallocation cost.

The earlier normalized disassembly comparison found matching instruction shapes
for the 1,499-instruction Projector capture body and 19-instruction capture driver.
The local empty normalized diffs remain at
`/tmp/sprite-task11-{projector,driver}-normalized.diff`. Normalization removes
addresses/module paths/relocation comments: it cannot establish identical linked
call targets, drop glue, native callees, binary layout, cache behavior, or timing.
Moving snapshot types changes symbol identities, and broader worker changes can
change the preceding full-harness workloads and process allocator/cache history.
These are plausible mechanisms, not demonstrated causes. This full-binary
comparison cannot isolate them, and the uncontrolled SMT sibling, scheduler and
frequency behavior remain possible contributors.

No extra capture traversal, invalidation, cloning, allocation or other actionable
production defect has been established by this assessment. The repeated p95
slowdown nevertheless remains unresolved evidence against timing acceptance.
No speculative production change or further benchmark was made to hide it.
