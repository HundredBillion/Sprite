# Capture tail-latency follow-up

The isolated capture check times one warmed 100×100 terminal snapshot plus the
returned bundle’s drop. It excludes terminal/fixture construction and native GUI
drawing. Its p95 limit remains **0.017937 ms (17.937 µs)**: no threshold changed.

## Finding and bounded fix

The preserved PR binary reproduced the failure at p95 **0.021263 ms**, with
104 Rust allocations and 14,656 requested bytes per capture. The complete
[reproduction report](original-reproduction.json) remains alongside prior failures.

The clean-row path reused render rows but cloned every observation-row `String`.
A separate production-fixture probe measured cloning its 100 observation rows at
5.133–5.203 µs median, versus 12.745–13.015 µs for full capture in the adjacent
threaded probes. The allocation regression failed with 104 allocations before
the change and passes after it. `PaneRow.text` now uses immutable `Arc<str>`;
unchanged generations share observation text, while changed rows build new text.
Prompt/wrap metadata is refreshed independently, old snapshots remain immutable,
and observation JSON still emits the same strings. Rendering and observation
remain independently projected from the terminal.

This changes the internal workspace Rust API from `String` to `Arc<str>`; all
workspace constructors and consumers were migrated. It is not a wire change.

The probes also found scheduler interruptions in some large samples and active
CPU cost in many others. Bare capture could fail while full-harness runs passed.
The removed copy cost is established; it is not a complete causal explanation
for every historical tail or the earlier module-extraction comparison.

## Predeclared verification

[protocol.json](protocol.json) records exact binary hashes, commands, exit codes,
environment observations, and all ten reports. A is the preserved prior PR
binary (`bd7f6eb` production code); B is the corrected `e47049e` binary. CPU 2,
TERM, working directory, 10 MiB output workload, 30 samples, and fixture warmup
are unchanged. Whole-process warmups A/B precede measured ABBA BAAB. All four
measured B runs were required to pass the original shared checker; none was
retried, discarded, or replaced. Generated per-run budgets are not adopted.

| Phase / order | Binary | Median (µs) | p95 (µs) | Max (µs) | Shared checker exit |
| --- | --- | ---: | ---: | ---: | ---: |
| [warmup 1](warmup-01-A.json) | A | 8.810 | 10.185 | 13.913 | 0 |
| [warmup 2](warmup-02-B.json) | B | 6.079 | 7.854 | 9.706 | 0 |
| [measured 1](measured-01-A.json) | A | 8.984 | 10.745 | 14.015 | 0 |
| [measured 2](measured-02-B.json) | B | 6.051 | 6.758 | 7.392 | 0 |
| [measured 3](measured-03-B.json) | B | 6.119 | 7.615 | 8.119 | 0 |
| [measured 4](measured-04-A.json) | A | 9.512 | 13.239 | 14.357 | 0 |
| [measured 5](measured-05-B.json) | B | 6.544 | 7.946 | 13.212 | 0 |
| [measured 6](measured-06-A.json) | A | 9.039 | 10.161 | 11.959 | 0 |
| [measured 7](measured-07-A.json) | A | 9.359 | 10.464 | 12.890 | 0 |
| [measured 8](measured-08-B.json) | B | 6.141 | 7.603 | 8.052 | 0 |

**All four corrected runs pass** timing, allocation, and byte limits. Each has
**4 Rust allocations / 3,456 requested bytes**, versus 104 / 14,656 before.
The median of four paired B/A p95 ratios is **0.677765 (32.2% lower)**; the
median-time ratio is 0.664845. These are medians of run-pair ratios, not pooled
percentiles. All four A runs also passed this time: earlier failures remain
valid observations of variability and have not been relabeled.

## Rebuilt-row tradeoff

Converting a newly constructed observation string to `Arc<str>` adds one
allocation and copy per rebuilt viewport row. Historical nonempty rows replace
their previous final owned-string copy with the Arc copy; an empty Arc can still
allocate where an empty String did not. This is not a claim that every path
allocates less.

A separate bounded [cold comparison](cold/protocol.json) used the first capture
of each fresh, filled 100×100 production fixture, timed with bundle drop. Both
diagnostic binaries used the same harness source with the warmup capture removed
and one thread spawn/join before sampling. CPU 2, 300 samples per run, and ABBA
BAAB were fixed; all eight JSON reports and raw-sample logs are retained.

| Order | Binary | Median (µs) | p95 (µs) | Rust allocations | Requested bytes |
| --- | --- | ---: | ---: | ---: | ---: |
| [1](cold/01-A.json) | A | 439.995 | 467.702 | 506 | 442248 |
| [2](cold/02-B.json) | B | 461.060 | 484.960 | 606 | 453448 |
| [3](cold/03-B.json) | B | 463.815 | 485.569 | 606 | 453448 |
| [4](cold/04-A.json) | A | 471.706 | 558.146 | 506 | 442248 |
| [5](cold/05-B.json) | B | 474.756 | 536.898 | 606 | 453448 |
| [6](cold/06-A.json) | A | 473.506 | 502.651 | 506 | 442248 |
| [7](cold/07-A.json) | A | 464.677 | 488.970 | 506 | 442248 |
| [8](cold/08-B.json) | B | 461.555 | 487.651 | 606 | 453448 |

Cold captures add 100 allocations and 11,200 requested bytes. The median paired
median ratio is 0.997961 (−0.2%), and the median paired p95 ratio is 1.017101
(+1.7%). This fixture demonstrates no material cold timing regression; it does
not prove equivalence for every repeated-dirty workload or explain throughput
differences in the full session harness. All full-harness metrics remain in the
reports; unrelated legacy timing limits are not claimed to pass.

The shared Cargo target initially reused the before diagnostic binary when
building after. Identical hashes revealed this **before measurement**; rebuilding
the changed inputs produced distinct hashes and the expected allocation delta.
No measured cold run was replaced. Production release binaries were restored
after diagnostic builds.

## Verification and review

- Full locked/offline workspace run: **773 passed, 2 ignored**.
- A final history-copy adjustment: 10 focused tests passed, 1 ignored.
- Formatting, all-target Clippy, and release builds passed; the existing
  `proc-macro-error2` future-compatibility notice remains.
- Independent bounded review found no outstanding correctness or performance
  finding after inspecting the clean and cold evidence.
- Temporary probes were removed from the repository; no diagnostic instrumentation
  or threshold relaxation enters production.

Local diagnostic sources and raw samples are preserved under
`/tmp/sprite-capture-investigation-probes/` and `/tmp/sprite-capture-*.csv`.
The red/green allocation logs are `/tmp/sprite-capture-row-sharing-{red,green}.log`.
The complete final verification runner is `/tmp/sprite-capture-row-sharing-verify.py`.
These local paths aid this investigation; committed JSON/protocol records provide
the portable measurement evidence. Native compositor/macOS performance was not
measured. CI test portability follow-ups are separate from this capture gate.
