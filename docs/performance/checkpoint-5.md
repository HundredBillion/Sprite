# Checkpoint 5 performance run — measured, not frozen

**Status: the gate does not pass, and no budget is frozen from this run.**
Arch Linux only, under the Linux-first posture; see
[checkpoint-1.md](checkpoint-1.md), whose machine description still applies.

This is the Checkpoint 5 Task 10 budget re-run, taken 2026-09-05 on an idle
machine. It is recorded rather than adopted: six metrics exceed budgets carried
from Checkpoints 2 to 4, and the medians say those six are not one story. Until
they are explained, freezing a new budget would only move the goalposts to
wherever the code happens to be.

Machine-readable: `checkpoint-5-arch-term.json`,
`checkpoint-5-arch-observation.json`, `checkpoint-5-arch-graphics.json`.

## Why this run was needed

The last freeze was Checkpoint 4. Since then: the whole of Checkpoint 5, the
block-element and box-drawing rewrite from shaped glyphs to geometry, the wheel
routing change, and divider resize. The render path changed underneath the last
recorded numbers, and nobody had measured what that cost.

## Against the carried budgets

Median is the robust figure and is what the "change" column compares.
`p95` is the budgeted one. Where `n` is 6 or 10, **p95 is simply the maximum**,
and should be read as one bad sample rather than a tail estimate.

| Metric | From | n | Median was | Median now | Change | p95 / budget |
|---|---|---:|---:|---:|---:|---|
| `spawn_to_ready` | cp2 | 30 | 0.637 | 0.642 | +1% | 0.726 / 0.870 |
| `input_to_snapshot_idle` | cp2 | 30 | 0.127 | 0.138 | +9% | 0.162 / 0.162 |
| `input_to_snapshot_under_load` | cp2 | 30 | 9.462 | 9.793 | +4% | 12.359 / 13.550 |
| `output_10mib_to_final_snapshot` | cp2 | 30 | 1187.958 | 1190.088 | +0% | 1385.130 / 1708.276 |
| `capture_100x100_grid` | cp2 | 30 | 0.042 | 0.019 | **−54%** | 0.227 / 0.097 ⚠ |
| `capture_with_full_scrollback` | cp2 | 30 | 0.254 | 0.494 | **+95%** | 0.931 / 1.078 |
| `scroll_round_trip` | cp2 | 30 | 0.259 | 0.295 | +14% | 0.684 / 0.723 |
| `select_full_screen` | cp2 | 30 | 0.126 | 0.135 | +7% | 0.160 / 0.147 ⚠ |
| `collect_four_panes` | cp3 | 30 | 0.001 | 0.001 | +15% | 0.001 / 0.001 |
| `collect_sixteen_panes` | cp3 | 30 | 0.003 | 0.003 | +16% | 0.005 / 0.003 ⚠ |
| `collect_with_one_stalled_pane` | cp3 | 6 | 500.196 | 500.108 | −0% | 500.222 / 550.225 |
| `encode_default_request` | cp3 | 30 | 1.239 | 1.423 | +15% | 1.550 / 1.417 ⚠ |
| `encode_maximum_history` | cp3 | 6 | 15.472 | 18.417 | **+19%** | 26.307 / 24.815 ⚠ |
| `transmit_to_placement` | cp4 | 20 | 3.294 | 3.533 | +7% | 3.878 / 3.926 |
| `transmit_to_placement_large` | cp4 | 10 | 11.262 | 15.370 | **+36%** | 23.841 / 19.032 ⚠ |
| `text_capture_without_images` | cp4 | 20 | 0.143 | 0.156 | +9% | 0.868 / 0.990 |
| `text_capture_with_an_image` | cp4 | 20 | 0.204 | 0.201 | −1% | 0.236 / 0.244 |

⚠ = p95 over its carried budget. All units are milliseconds.

## Reading it

**The six breaches are not one story, and the budget column is the wrong place
to look first.**

- **`capture_100x100_grid` breaches its budget while its median improved by
  54%** — 0.042 ms to 0.019 ms. The typical capture got faster; one sample in
  thirty got slower. At a 19-microsecond median, a p95 budget of 0.097 ms is
  measuring the scheduler as much as the code. This is the clearest case for
  *not* treating a breach as a regression.
- **`select_full_screen`, `collect_sixteen_panes`, `encode_default_request`**
  are over by 9%, 85% and 9% at p95, on medians that moved 7%, 16% and 15%.
  Modest, and all three are sub-2 ms.
- **`encode_maximum_history` (+19%) and `transmit_to_placement_large` (+36%)**
  are the two worth explaining. Both moved substantially at the *median*, which
  noise does not do, and both are at a millisecond scale where the measurement
  is trustworthy. `transmit_to_placement_large` is the largest single movement
  in the run.

**And the metric the budget gate missed entirely: `capture_with_full_scrollback`
almost doubled at the median — 0.254 ms to 0.494 ms — and still passed**,
because its budget had room. A gate that only watches p95 against a generous
budget will not see a 95% median regression. That is worth fixing in the
harness, independently of this run.

## What has to happen before a budget is frozen

1. Explain `transmit_to_placement_large` (+36%) and `encode_maximum_history`
   (+19%). Both are in `sprite-term`/observation paths, so the geometry
   rendering rewrite is **not** an obvious cause and should not be assumed to be
   one.
2. Explain `capture_with_full_scrollback` (+95%), which no budget caught.
3. Re-run both small-sample metrics (`n` = 6 and 10) with enough samples for p95
   to mean something.
4. Only then freeze, and record the budgets with the same 110%-of-p95 rule the
   earlier checkpoints used.

## A note on the second run

A second run was taken immediately afterwards and is **not** recorded here,
because Croft had restarted and was taking ~42% CPU: every metric was slower and
several more breached. It is mentioned only to say that it is not evidence
either way, and that the numbers above are the idle-machine ones.

## Reproducing

~~~bash
cargo build --release -p sprite-term -p sprite-app --bins --locked --offline
./target/release/sprite-term-bench        --samples 30 \
  --output docs/performance/checkpoint-5-arch-term.json
./target/release/sprite-observation-bench --samples 30 \
  --output docs/performance/checkpoint-5-arch-observation.json
./target/release/sprite-graphics-bench    --samples 20 \
  --output docs/performance/checkpoint-5-arch-graphics.json
~~~

Run on an idle machine — check `/proc/loadavg` first, not just your impression
of it. This run began at a load of 1.44, which is already not ideal, and is one
more reason the numbers above are a measurement to explain rather than a budget
to adopt.
