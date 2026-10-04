# Design review paint baseline

`design-review-paint-baseline.json` records the terminal's existing allocations
before row/text sharing or paint caching. The benchmark is a separate process
with `System` behind its counting allocator. It creates no application, Window,
PTY or worker.

## Measured production seam

`TerminalView::render` and `PaintBenchmark` both call `grid::prepare_rows`, which
calls the real `lay_out_row` and applies generation-checked hyperlink styling.
Both then call `GridPaint::prepare` to copy the palette into a fresh Arc and build
the whole pass or background/text passes. Split preparation still clones all
positioned rows and their cell strings. No allocation optimization is included.

The narrow `GridPaint::benchmark_draw_decisions` wrapper calls `resolve_row`, the
same helper used by live painting, which calls the actual `GridPaint::draw`.
It includes the existing fresh scratch Vec for resolved cells, reused across rows
within a pass, and consumes every row's result with `black_box`. Prepared paint
elements are also consumed with `black_box` and dropped inside measurement.

The seam excludes GPUI Window/layout work, image element construction, geometry
submission, font selection/shaping/rasterization, glyph painting and GPU work.
Split mode forces the same split preparation/decisions that images between the
background and text trigger, without constructing images. These timings describe
headless CPU preparation/decisions, not frame rate or end-to-end paint latency.

## Fixture and transitions

Each fixture has 200 columns and 60 rows, with ASCII, spaces, accented characters,
a combining grapheme, wide characters and spacer tails, box/block glyphs,
selection, palette foreground/background colors and a visible blinking block
cursor overlapping selected wide text. Cell metrics are 8.4 × 18 logical pixels.

Each measured sample creates a fresh driver outside measurement. The driver owns
an original generation-1 snapshot and a generation-2 snapshot with one cell in
row 30 changed. Snapshot copying and mutation happen outside measurement. A fresh
driver resets any preparation state, so these transitions remain meaningful when
a per-driver cache is introduced:

| Scenario | State before measurement | Measured transition |
| --- | --- | --- |
| First frame | Fresh driver; no priming frame | Prepare/draw the initial frame |
| Same-generation blink | Prime generation 1 with cursor on | Same snapshot/generation, cursor phase off |
| Hover | Prime generation 1 without hover | Same snapshot/generation, hover span on row 12 |
| One-row change | Prime generation 1 | Generation 2 with only row 30 changed |

Every sample measures a transition; hover does not become repeated unchanged
hover, and one-row change does not become repeated unchanged generation 2. The
first-frame sample gets no cache warmup, though each metric has a separate process
warmup sample excluded from measurement. All four scenarios run in whole and
split modes, with stable metric names for direct before/after comparison.

## Allocation and report contract

The guard enables allocation counting only on the measuring thread during live
preparation, cell decisions and destruction of their temporary results. Fixture
construction, process warmup, per-sample priming, timing/report vector setup,
serialization and file I/O are excluded. The allocator counts every `alloc`,
`alloc_zeroed` and `realloc` call; `realloc` adds its entire requested new size.
Deallocation does not increment counts. Bytes are cumulative requested allocation
sizes, not net live memory, heap overhead or peak memory.

JSON schema 1 carries fixture dimensions, sample count, transition descriptions,
timing median/p95 in milliseconds and allocation/byte median/p95 with budgets.
Percentiles use the nearest index of `(sample_count - 1) × fraction` after sorting.
Allocation and byte budgets are p95 plus 10%, rounded up. `--check-budgets` validates
the schema and all eight metric/transition contracts, then checks measured p95
allocation and byte counts against the supplied budgets. Timing is contextual and
is not a pass/fail budget. The JSON has no timestamp or machine identity.

## Release measurements

Measured on Linux x86_64, Intel 11th Gen Core i7-11850H @ 2.50 GHz (8 cores,
16 threads), using Rust/Cargo 1.97.1 and Cargo's default optimized release profile
(no repository release-profile override), with 30 measured samples per metric.

```sh
TERM=xterm-ghostty cargo run -p sprite-app --bin sprite-paint-bench \
  --release --locked --offline -- --samples 30 \
  --output docs/performance/design-review-paint-baseline.json
```

| Scenario/pass | Allocations p95 | Requested bytes p95 | Median ms | p95 ms |
| --- | ---: | ---: | ---: | ---: |
| `whole_first_frame` | 11,463 | 702,424 | 0.423 | 0.460 |
| `whole_same_generation_blink` | 11,463 | 702,424 | 0.442 | 0.459 |
| `whole_hover` | 11,463 | 702,424 | 0.446 | 0.457 |
| `whole_one_row_change` | 11,463 | 702,424 | 0.444 | 0.457 |
| `split_first_frame` | 22,925 | 1,370,464 | 1.073 | 1.151 |
| `split_same_generation_blink` | 22,925 | 1,370,464 | 1.066 | 1.104 |
| `split_hover` | 22,925 | 1,370,464 | 1.065 | 1.102 |
| `split_one_row_change` | 22,925 | 1,370,464 | 1.062 | 1.106 |

All four transitions currently rebuild all rows. Split passes approximately double
the allocations because the second pass owns cloned rows and another scratch
buffer. The committed report is the before-optimization evidence; later reports
must retain the same transitions and measured seam, including scratch allocation.

## Executable verification

The report integration test runs the binary, validates its report, passes the
allocation/byte budget check, and observes failure with an allocation budget of
zero and with an unsupported schema. Allocator tests exercise allocation,
zero-filled allocation, reallocation, deallocation and guard teardown; fixture and
preparation tests cover the real blink, hover and one-row transitions in both
whole and split modes.

```sh
TERM=xterm-ghostty cargo test -p sprite-app --locked --offline --no-fail-fast
TERM=xterm-ghostty cargo clippy -p sprite-app --all-targets --locked --offline -- -D warnings
cargo fmt --all -- --check
TERM=xterm-ghostty cargo run -p sprite-app --bin sprite-paint-bench \
  --release --locked --offline -- --samples 30 \
  --check-budgets docs/performance/design-review-paint-baseline.json
```

A separate release run with `whole_same_generation_blink.allocations.budget = 0`
failed with `allocation budget exceeded (11463 > 0)`. The unmodified baseline
then passed. This confirms the executable gate can reject an actual over-budget
measurement rather than merely accepting its own report.
