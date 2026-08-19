# Checkpoint 2 performance baselines and budgets

**Status: Arch Linux only.** Under the Linux-first posture, macOS baselines join
the macOS acceptance milestone. See the PRD's platform posture and
[checkpoint-1.md](checkpoint-1.md), whose machine description still applies —
same hardware, same toolchain, same pinned Ghostty commit.

## Budgets

Thirty samples, release build, on an otherwise idle machine. Budgets are 110% of
this run's p95. Machine-readable copy: `checkpoint-2-arch.json`.

| Metric | Median (ms) | p95 (ms) | Budget (ms) |
|---|---:|---:|---:|
| `spawn_to_ready` | 0.637 | 0.791 | 0.870 |
| `input_to_snapshot_idle` | 0.127 | 0.147 | 0.162 |
| `input_to_snapshot_under_load` | 9.462 | 12.318 | 13.550 |
| `output_10mib_to_final_snapshot` | 1187.958 | 1552.979 | 1708.276 |
| `capture_100x100_grid` | 0.042 | 0.088 | 0.097 |
| `capture_with_full_scrollback` | 0.254 | 0.980 | 1.078 |
| `scroll_round_trip` | 0.259 | 0.657 | 0.723 |
| `select_full_screen` | 0.126 | 0.133 | 0.147 |

The last three are new in Checkpoint 2. `capture_with_full_scrollback` exists to
prove capture stays proportional to the visible screen rather than to retained
history: it runs against more than 5,000 rows of scrollback and stays within
about a factor of ten of `capture_100x100_grid`, which itself captures four
times as many cells.

## A regression, found and mostly fixed

The first Checkpoint 2 run put **three** metrics above Checkpoint 1's budgets:

| Metric | C1 budget | C2 first run | After the fix |
|---|---:|---:|---:|
| `spawn_to_ready` | 0.850 | 0.897 | 0.791 |
| `input_to_snapshot_idle` | 0.142 | 0.168 | 0.147 |
| `input_to_snapshot_under_load` | 14.302 | 16.259 | 12.318 |

The cause was specific. Checkpoint 2 added an `is_selected()` call **per cell**
to snapshot capture — about 1,900 extra FFI calls on a default grid, every
capture. At roughly 20 ns each that is ~0.038 ms, which matched the observed
+0.039 ms on `input_to_snapshot_idle` almost exactly.

The fix was to skip the query entirely when nothing is selected, which is the
overwhelmingly common case. The worker already knows whether it installed a
selection, so no extra state was needed. That recovered idle latency from 0.168
to 0.147 ms and brought the other two back under their old budgets.

**Two metrics remain marginally over Checkpoint 1's budgets, and are accepted:**

- `input_to_snapshot_idle` at 0.147 against a 0.142 budget — 3.5% over. The
  residue is the per-row `semantic_prompt()` query and four per-capture queries
  (`scrollbar`, `is_mouse_tracking`, `title`, `pwd`) that deliver scrollback
  position, mouse routing, and observation metadata. Real cost for real
  features.
- `output_10mib_to_final_snapshot` at 1553.0 against a 1543.9 budget — 0.6%
  over, which is inside this benchmark's run-to-run spread and not a signal.

## A measurement error worth recording

The middle run in the table above was taken while a Croft build was compiling in
the background, and it is not comparable to the others: it reported
`input_to_snapshot_idle` at 0.180 ms and `output_10mib` at 1625 ms, both worse
than the unoptimised code, which briefly made the fix look like a regression.

Budgets must be measured on an idle machine. The numbers in the budget table
were re-taken after the build finished. This is recorded because the mistake was
invisible in the output — the harness cannot tell that the machine was busy, and
nothing in the JSON would have revealed it later.

## Gates

- **Croft capability gate: passed** on Arch against unmodified upstream `main` at
  `cf805f2901155bc1144fa31b78e8061ce2f76d3e`. Worth noting that the rendering
  path changed substantially this checkpoint — cell positioning, selection
  marking, scrollback — and Croft still behaves.
- **Forbidden states: clean.** No `unsafe impl Send/Sync`, no async runtime, no
  `gpui-ghostty` or `tty7`. The only `unsafe` in the workspace remains the two
  audited descriptor borrows in `pty_unix.rs`. `sprite-app` reaches past the seam
  nowhere and contains no polling or timers.
- **Provenance: clean.** The Ghostty submodule is still pinned to
  `ab0b9da9e88fcb4b0533a1854e84628f663930af`.
- **Human review: not yet requested.** Still owed, as it was for Checkpoint 1.

## Regression policy

Unchanged from Checkpoint 1. A metric above its budget column blocks merge until
explained or fixed, and budgets are re-frozen only by rerunning the full
30-sample release benchmark on a recorded, idle machine and updating this file
together with the JSON.
