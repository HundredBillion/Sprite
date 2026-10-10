# Bug-class audit paint record

Before/after record for the paint work in the bug-class audit (PRD
`docs/PRDs/10-09-2026-bug-class-audit.md`, R-R1 and R-R3). Recorded, not
enforced: no budget in this file is a gate.

## What changed in the measured seam

- A laid-out cell keeps the terminal's compact `CellText`: empty and
  single-scalar text is stored inline, and longer text is shared through an
  `Arc<str>`, so the laid-out cell shares the terminal cell's string. The text
  system's `SharedString` is made only when a shape is made, on a shape-cache
  miss. At `a62247e` paint copied every glyph cell's
  text into a new string on every frame, outside this benchmark's measured
  seam.
- Glyphs are shaped through a per-row shape cache beside the layout cache: a
  cell is shaped again only when its drawn colour differs from the one it was
  shaped in, and the cache is dropped on any font, theme or scale change.
  Shapes are pooled per pane (at most 4,096 distinct shapes). Hidden cells are
  never shaped and carry no decorations.
- Box-drawing strokes and outlines are fixed arrays; drawing one allocates
  nothing.

## Platform text system: timing not measured

GPUI shapes text only through a window's text system
(`WindowTextSystem::shape_line`, `vendor/gpui/src/text_system.rs:365`). Its
constructor and the platform text systems (`MacTextSystem`, `CosmicTextSystem`)
are crate-private, and `Application::headless()` opens no window. Shaping cost
was therefore **not timed**, and no shaping speed-up is claimed. The
`--shaping` mode counts, through the live shape cache, how many cells reach the
text system and how many the cache had to shape.

## Commands

```sh
# before: master a62247e in a throwaway worktree (vendor/ghostty at the same pinned commit)
TERM=xterm-ghostty <worktree>/target/release/sprite-paint-bench --samples 30 \
  --output docs/performance/bug-class-audit-paint-before.json
# after: this branch
TERM=xterm-ghostty target/release/sprite-paint-bench --samples 30 --shaping \
  --output docs/performance/bug-class-audit-paint-after.json
```

Machine: Darwin arm64, Apple M5 Pro, 18 cores (18 logical); rustc 1.97.1
(8bab26f4f 2026-07-14); release profile; 30 samples per metric. The "before"
run was recorded on 2026-10-09 with no compilation, tests or other benchmark
running and one idle Sprite window (under 1% CPU) open. The "after" run was
recorded on 2026-10-09 at 23:08 at commit 52cb4cf, whose code is that of
0fdbc7c, with no compilation, tests or other benchmark running, on a loaded
machine:

- OrbStack Helper used 550–700% CPU, about six to seven of the 18 cores,
  before and during every run.
- Microsoft Defender used 11–31%, and WindowServer about 11%.
- Seven `llm-wiki` Python processes were at 86–112% CPU each just before the
  re-freeze run started and had fallen below 10% by the two runs after it.
- The user's Sprite app (`/Applications/Sprite.app`) and a release `sprite`
  built from another checkout were open and idle, each under 2% CPU.

## Preparation and decisions (existing seam)

| Scenario/pass | Allocations max (before) | Bytes max (before) | Median ms (before) | p95 ms (before) |
| --- | ---: | ---: | ---: | ---: |
| `whole_first_frame` | 122 | 579,376 | 0.308459 | 0.323625 |
| `whole_same_generation_blink` | 0 | 0 | 0.296417 | 0.343125 |
| `whole_hover` | 3 | 10,136 | 0.282459 | 0.302750 |
| `whole_one_row_change` | 3 | 10,136 | 0.280334 | 0.285250 |
| `split_first_frame` | 122 | 579,376 | 0.457458 | 0.477125 |
| `split_same_generation_blink` | 0 | 0 | 0.428542 | 0.457000 |
| `split_hover` | 3 | 10,136 | 0.433208 | 0.451834 |
| `split_one_row_change` | 3 | 10,136 | 0.429375 | 0.446541 |

| Scenario/pass | Allocations max (after) | Bytes max (after) | Median ms (after) | p95 ms (after) |
| --- | ---: | ---: | ---: | ---: |
| `whole_first_frame` | 122 | 579,376 | 0.323083 | 0.404375 |
| `whole_same_generation_blink` | 0 | 0 | 0.295250 | 0.326792 |
| `whole_hover` | 3 | 10,136 | 0.282708 | 0.297417 |
| `whole_one_row_change` | 3 | 10,136 | 0.291917 | 0.294875 |
| `split_first_frame` | 122 | 579,376 | 0.472667 | 0.508416 |
| `split_same_generation_blink` | 0 | 0 | 0.432250 | 0.484375 |
| `split_hover` | 3 | 10,136 | 0.454667 | 0.500875 |
| `split_one_row_change` | 3 | 10,136 | 0.450791 | 0.520959 |

Allocations and requested bytes are identical before and after in every
transition: 122 allocations and 579,376 bytes for a first frame, 3 and 10,136
for hover and a one-row change, and nothing for a same-generation blink. Laying
out a row clones each cell's compact text, which allocates nothing, so the
measured preparation allocates exactly as it did at `a62247e`. Per-frame
paint, outside this seam, now allocates less: `a62247e` made a new string for
every one of the 6,600 glyph cells on every frame, idle frames included, and
paint now makes one only when a shape is made.

A first draft of this work built the text system's string at layout instead.
That raised first-frame allocations from 122 to 3,122, and hover and one-row
allocations from 3 to 53, because every non-ASCII cell's text was copied once
per layout of its row. It was reverted before merge, and a test now holds the
fixture's first frame at 122 allocations and hover and one-row change at 3.

Timing is recorded, not gated. "After" medians were within 0–5% of "before"
(−0.4% for whole same-generation blink, +5.0% for split hover and split
one-row change). The "after" run shared the machine with OrbStack's load
described above and the "before" run did not, so part of that difference may
be load. The re-freeze run a few seconds earlier, which overlapped the end of
the Python burst, measured 17–33% above "before"; the budget-check run after it
measured 1–17% above. The paint-side saving is outside this seam and is not
timed.

### Surface allocation probe

`surface_performance::surface_allocation_probe` measures the grid Surface's
one-row update and render. Its bounds are those of `a62247e`, including the
12,000-byte bound on that update. The update now allocates 6 times and
requests 10,928 bytes, against 7 and 10,960 at `a62247e`, because the changed
cell's text is interned once and the laid-out cell shares that string.

## Budgets re-frozen

The budgets in `design-review-paint-shared.json` were re-frozen with the full
30-sample release benchmark on the machine above, per `checkpoint-1.md`'s
regression policy. The new run also passed the original
`design-review-paint-baseline.json` budgets. A second run then passed
`--check-budgets design-review-paint-shared.json`. The new budgets and the
reason they moved are recorded in `design-review.md` under "Paint budgets
re-frozen after the bug-class audit".

| Scenario/pass | Allocations max | Allocation budget | Requested bytes max | Byte budget |
| --- | ---: | ---: | ---: | ---: |
| `whole_first_frame` | 122 | 135 | 579,376 | 637,314 |
| `whole_same_generation_blink` | 0 | 0 | 0 | 0 |
| `whole_hover` | 3 | 4 | 10,136 | 11,150 |
| `whole_one_row_change` | 3 | 4 | 10,136 | 11,150 |
| `split_first_frame` | 122 | 135 | 579,376 | 637,314 |
| `split_same_generation_blink` | 0 | 0 | 0 | 0 |
| `split_hover` | 3 | 4 | 10,136 | 11,150 |
| `split_one_row_change` | 3 | 4 | 10,136 | 11,150 |

These are the budgets the file held at `a62247e`. They replace the 3,435 /
822,114 and 59 / 14,230 budgets frozen for the reverted first draft. The whole
first frame's 579,376 requested bytes are 25% below the original baseline byte
budget of 772,667.

## Shaping (counts; not timed)

`glyph_cells` is what `a62247e` shaped on every frame of that transition (it
shaped every glyph cell every frame). `shape_calls` is what the shape cache
shapes now.

| Scenario/pass | Glyph cells (= shapes per frame at a62247e) | Shape calls now |
| --- | ---: | ---: |
| `whole_first_frame` | 6,600 | 2,335 |
| `whole_same_generation_blink` | 6,600 | 0 |
| `whole_hover` | 6,600 | 6 |
| `whole_one_row_change` | 6,601 | 1 |
| `split_first_frame` | 6,600 | 2,335 |
| `split_same_generation_blink` | 6,600 | 0 |
| `split_hover` | 6,600 | 6 |
| `split_one_row_change` | 6,601 | 1 |

The first frame shapes fewer than 6,600 because identical cells drawn in the
same colour share one pooled shape. The fixture has no hidden cells, so never
shaping them leaves these counts unchanged. The counts match those of the
first draft: building the string later changed what a shape costs to make, not
how many are made.

The same gate is enforced at Sprite's real `shape_line` call site by
`grid_paint::tests::shaping_happens_only_for_cells_whose_drawn_text_changed`
(unchanged frame 0; blink ≤ 1; one-row change only that row; font or theme
change equal to a cold cache).

## Verification gates

| Gate | Result |
| --- | --- |
| `TERM=dumb cargo test --workspace --locked --offline` | pass at 0fdbc7c: 950 passed, 0 failed, 3 ignored (45 test binaries; run with `--no-fail-fast`) |
| `cargo clippy --workspace --all-targets --locked --offline -- -D warnings` | pass |
| `cargo fmt --all --check` | pass |
| `sprite-paint-bench --samples 30 --check-budgets docs/performance/design-review-paint-shared.json` (release) | pass (exit 0) |

## Native macOS behaviour not executed

These need a real desktop session (a key window, a display, an input method or
the system pasteboard) that the test platform and CI do not provide. They are
covered by tests against GPUI's test platform and the real terminal worker,
but were not exercised on a live macOS desktop:

- Pane Focus following real window activation: switching to another app sends
  focus-out to a program with DECSET 1004 and denies OSC 52 clipboard writes;
  returning sends focus-in.
- The unfocused pane's steady hollow block cursor, and only the focused pane
  blinking, as seen on screen.
- Holding Ctrl+Shift+W, close-tab, quit or split with real keyboard
  auto-repeat: the confirmation is not answered by the repeat, and one split
  is made.
- A real input method commit (for example Kotoeri) disarming a pending
  confirmation.
- Unsafe-paste confirmation against the real macOS pasteboard, including the
  clipboard changing between the two pastes.
- On a Retina (scale 2) display: faint text at half strength, selected hidden
  text showing the selection, a cursor on a wide character's second column
  drawn over both columns, and Kitty Unicode-placeholder images without seams
  between tiles.
- Shaping cost through CoreText (not timed; see above).
- Quitting with many busy panes: cleanup runs on per-pane threads and the
  window closes without stalling other background work.
