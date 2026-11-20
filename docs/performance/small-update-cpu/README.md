# Small updates on a populated screen (#159)

Measured on 2026-11-19 and 2026-11-20 on the physical Omarchy laptop (ThinkPad
P1 Gen 4, i7-11850H, Hyprland 0.56.2, Linux 7.2.15) for issue #159.
Baseline: `7496b77` (`master`, after #170). Both Sprite builds are release
builds of the locked dependencies with symbols kept for profiling
(`CARGO_PROFILE_RELEASE_STRIP=none`, `CARGO_PROFILE_RELEASE_DEBUG=line-tables-only`);
the shipped profile strips them, which does not change the code.

## Method

`workload.py` writes the same bytes as the issue's workload: 28 static rows of
79 characters, then one status line rewritten in place, on the alternate
screen with the cursor hidden. Each phase lasts three seconds:

| Phase | What changes |
|---|---|
| `quiet_full` | nothing, populated screen |
| `full_40hz` | the status line, 40 times a second, under the populated screen |
| `full_5hz` | the same at 5 Hz |
| `sparse_40hz` | the status line at 40 Hz on a cleared screen |
| `quiet_sparse` | nothing, cleared screen |
| `scroll_40hz` | a new line at the bottom 40 times a second: the whole screen scrolls (added for this change, the worst case for it) |

`measure.py OUTDIR ROUNDS label=/path/to/sprite ...` opens each build in its
own floating 900×700 window (122×48 cells, JetBrainsMono Nerd Font 12px),
alternating the order every round, and reads the terminal process's CPU from
`/proc/PID/stat` every 250 ms. Values are percent of one logical core, averaged
per phase over complete intervals; the child's CPU is not included. It leaves
every other window alone and restores focus. Before each run it waits for the
CPU package to stay below 78 °C for 3 s, and stops at 95 °C held for three
checks, any 98 °C, or off AC. `alacritty-wrap.sh` runs Alacritty 0.17.0 under
the same harness (9pt, 128×42 cells; the populated region fits both).

`--sample N` also takes `N` main-thread stacks with `eu-stack` during the run
(`perf` is not installed on this machine; the test process opts in to tracing
with `prctl(PR_SET_PTRACER)`). `fold.py` drops idle samples, those parked in the
event loop's poll, and counts the rest by function.

## Baseline, and where its time went

`baseline-vs-alacritty.json`, four alternating rounds:

| Phase | Sprite `7496b77` | Alacritty 0.17.0 |
|---|---|---|
| populated, 40 Hz | 6.34 / 6.13 / 6.48 / 5.76 | 2.52 / 2.52 / 2.77 / 1.98 |
| populated, 5 Hz | 1.08 / 0.72 / 1.08 / 1.08 | 0.36 / 0.36 / 0 / 0 |
| cleared, 40 Hz | 3.96 / 4.32 / 4.68 / 4.33 | 1.80 / 1.44 / 2.16 / 2.16 |

#170 already brought the issue's 11–26 % down to about 6 %. Unchanged text
still added about 1.9 points to every 40 Hz second (populated minus cleared),
against about 0.5 for Alacritty.

99 busy main-thread samples over 200 s of the populated 40 Hz phase:

| Where | Share of busy samples |
|---|---|
| `GridPaint::paint`, every row each frame | 45 % |
| — `paint_line`, one glyph at a time into the scene | 17 % |
| — `GridPaint::draw`, resolving each cell's colours | 11 % |
| Wayland/Blade present | 22 % |
| sorting the frame's glyph sprites | 6 % |
| layout and prepaint | 8 % |

Every frame re-resolved, re-looked-up and re-inserted every cell of every row,
although one row had changed.

## The change

An unchanged row is replayed from what it painted in an earlier frame instead
of being painted cell by cell (`RowPaints` in `grid_paint.rs`, on top of
`Window::record_paint`/`replay_paint` from `patches/gpui-0.2.2-recorded-paint.patch`).

`first-attempt.json` kept every row it painted. The populated 40 Hz phase fell
from 6.60 to 5.16 (mean of 3 and 4 runs), but a whole-screen scroll rose from
9.24 to 9.91: when every row changes, keeping a copy of each costs more and
nothing is ever replayed. The final version keeps rows only from a frame where
at most half the screen changed since the last one.

## Before and after

`before-after.json`, five alternating rounds:

| Phase | `master` | row replay | Median change |
|---|---|---|---|
| populated, 40 Hz | 5.94 / 5.76 / 6.13 / 7.58 / 6.13 | 4.69 / 4.68 / 4.68 / 4.68 / 4.32 | 6.13 → 4.68 (−24 %) |
| populated, 5 Hz | 1.08 / 1.08 / 1.08 / 1.08 / 1.08 | 0.72 / 0.72 / 1.08 / 1.08 / 0.72 | 1.08 → 0.72 |
| cleared, 40 Hz | 4.69 / 4.32 / 4.68 / 5.05 / 4.32 | 3.96 / 3.24 / 3.60 / 3.60 / 3.61 | 4.68 → 3.60 (−23 %) |
| whole-screen scroll, 40 Hz | 9.01 / 8.29 / 9.00 / 9.00 / 8.64 | 9.00 / 8.28 / 9.01 / 9.01 / 9.00 | 9.00 → 9.00 |
| quiet | 0–0.79 | 0–0.40 | — |

The cleared screen improves too: its blank rows were resolved cell by cell
every frame although they draw nothing.

## What is left, against Alacritty

75 busy samples of the populated 40 Hz phase with row replay:

| Where | Share of busy samples |
|---|---|
| replaying kept rows (`Scene::replay_operations`) | 28 % |
| Wayland/Blade present | 27 % |
| sorting the frame's glyph sprites | 9 % |
| layout and prepaint | 10 % |

Populated 40 Hz is now 4.68 against Alacritty's 2.0–2.8, and a cleared screen
3.60 against 1.4–2.2. Two costs remain, and neither is per-cell work in Sprite:

- **A fixed cost per frame** of about 1.5–2 points, present on a cleared
  screen: GPUI lays out, prepaints and presents the whole window, and builds a
  new scene, every frame.
- **A cost per glyph shown**, about 1 point here: each frame copies every
  glyph's primitives into the new scene (each glyph also opens a nested layer
  through `ShapedLine::paint`), sorts them, and uploads them. Alacritty keeps
  glyph instances in GPU batches. Closing this needs a grid that reaches the
  renderer without going through the scene each frame.

## Limits

- Exploratory measurements on one laptop with other work running (an idle
  LAS model server, two Claude sessions). The 250 ms intervals and 10 ms clock
  ticks make a single 3 s phase coarse; medians of five alternating runs are
  reported, and every run is in the JSON.
- In `first-attempt.json` the thermal guard stopped the fourth `master` run
  while it waited for a cool start, so that file has three `master` runs and
  four of the other.
- Responsiveness and input latency were not measured separately. The change
  does not alter when a frame is drawn, only what painting it costs.
- Only Linux/Wayland was measured. macOS uses the same paint path.
