# Bug-class audit paint record

Before/after record for the paint work in the bug-class audit (PRD
`docs/PRDs/10-09-2026-bug-class-audit.md`, R-R1 and R-R3). Recorded, not
enforced: no budget in this file is a gate.

## What changed in the measured seam

- Cell text is a `SharedString` made once when a row is laid out. Printable
  ASCII borrows a static string; other text is copied once per layout of its
  row. At `a62247e` paint copied every glyph cell's text into a new string on
  every frame, outside this benchmark's measured seam.
- Glyphs are shaped through a per-row shape cache beside the layout cache: a
  cell is shaped again only when its drawn colour differs from the one it was
  shaped in, and the cache is dropped on any font, theme or scale change.
  Shapes are pooled per pane (at most 4,096 distinct shapes).
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
(8bab26f4f 2026-07-14); release profile; 30 samples per metric; no
compilation, tests or other benchmark running (one idle Sprite window, under 1%
CPU, was open). Recorded on 2026-10-09; the "after" run is at commit d1fceb0.

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
| `whole_first_frame` | 3,122 | 747,376 | 0.396334 | 0.716834 |
| `whole_same_generation_blink` | 0 | 0 | 0.317083 | 0.369292 |
| `whole_hover` | 53 | 12,936 | 0.323417 | 0.381542 |
| `whole_one_row_change` | 53 | 12,936 | 0.326625 | 0.380041 |
| `split_first_frame` | 3,122 | 747,376 | 0.562167 | 0.639500 |
| `split_same_generation_blink` | 0 | 0 | 0.480791 | 0.588916 |
| `split_hover` | 53 | 12,936 | 0.496833 | 0.553042 |
| `split_one_row_change` | 53 | 12,936 | 0.489000 | 0.527833 |

Same-generation blink allocates nothing before and after. First frame, hover
and one-row change now allocate once per non-ASCII cell in each row laid out
(the fixture has 50 per row), because that text is made at layout instead of
at paint: layout allocations rose inside this seam. Per-frame paint
allocations, outside it, fell: the paint path this replaces made a new string
for every one of the 6,600 glyph cells on every frame, idle ones included, and
paint now makes none.

Bytes rose by 2,800 per laid-out row (168,000 for the 60-row first frame, 2,800
for hover and one-row change). Each laid-out cell's paint-ready text is 8 bytes
wider than the terminal's compact cell text (200 cells, 1,600 bytes), and each
of the 50 non-ASCII cells' text is one 24-byte shared allocation (1,200 bytes).

Timing is recorded, not gated. Medians in this seam were 7–28% higher in the
"after" run. Same-generation blink allocates nothing in either run and still
rose 7–12%, so the per-cell draw decision's new work (faint and hidden text
are now resolved there) and run-to-run noise account for part of it. The
re-freeze run a few minutes later measured 0–19% above "before". The paint-side
saving is outside this seam and is not timed.

### Surface allocation probe

`surface_performance::surface_allocation_probe` measures the grid Surface's
one-row update and render. Its byte bound was raised from 12,000 to 13,000 for
the same reason as above: the relaid 200-cell row's buffer is 1,600 bytes
larger. That update now allocates 6 times instead of 7, because the changed
cell's text is interned once and shared between lookup and paint, and requests
12,528 bytes instead of 10,960. The probe's first render requests about
96,000 bytes more (60 rows of 200 cells, 8 bytes each). Its allocation bounds
are unchanged.

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
| `whole_first_frame` | 3,122 | 3,435 | 747,376 | 822,114 |
| `whole_same_generation_blink` | 0 | 0 | 0 | 0 |
| `whole_hover` | 53 | 59 | 12,936 | 14,230 |
| `whole_one_row_change` | 53 | 59 | 12,936 | 14,230 |
| `split_first_frame` | 3,122 | 3,435 | 747,376 | 822,114 |
| `split_same_generation_blink` | 0 | 0 | 0 | 0 |
| `split_hover` | 53 | 59 | 12,936 | 14,230 |
| `split_one_row_change` | 53 | 59 | 12,936 | 14,230 |

The whole first frame's 747,376 requested bytes are within 3% of the original
baseline byte budget of 772,667: little headroom remains against that file.

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
same colour share one pooled shape.

The same gate is enforced at Sprite's real `shape_line` call site by
`grid_paint::tests::shaping_happens_only_for_cells_whose_drawn_text_changed`
(unchanged frame 0; blink ≤ 1; one-row change only that row; font or theme
change equal to a cold cache).

## Verification gates

| Gate | Result |
| --- | --- |
| `TERM=dumb cargo test --workspace --locked --offline` | pass: 937 passed, 0 failed, 3 ignored (45 test binaries; run with `--no-fail-fast`) |
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
