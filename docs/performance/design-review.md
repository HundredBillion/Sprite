# Design review paint baseline

Latest follow-up: [observation-row sharing resolves the capture timing gate](design-review-capture-row-sharing/README.md).
The original measurements below are retained as historical evidence.


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

## PTY output allocation measurement

The pump allocates sixteen 16 KiB output buffers at startup (262,144 payload
bytes). Each output message owns its buffer until it is parsed or discarded;
Drop returns that allocation and its read permit together. Poll descriptors use
a stack array, removing the polling loop's previous temporary vector allocations.

`steady_state_pump_delivers_sixty_four_chunks_without_allocating` measures actual
`System` allocator calls around the production pump's `run` loop, including its
polling, reading, and message sends. Socket/channel/thread and pool setup happen
before measurement. A separate consumer writes and drops 64 messages, with one
15-byte write per delivery; its allocations are excluded by thread-local counting.
No terminal parser, snapshot work, or application rendering is included.
The measured pump loop performs **0 allocations and requests 0 allocated bytes**
over all 64 deliveries. This is an allocator measurement, independent of the
separate pointer-reuse assertion. Temporarily restoring the old per-read `to_vec`
copy made the check fail at **64 allocations / 960 bytes**; removing that copy
restored the zero-allocation result.

```sh
TERM=xterm-ghostty cargo test -p sprite-term --lib \
  steady_state_pump_delivers_sixty_four_chunks_without_allocating -- --nocapture
```

The drop-only ownership regression previously stopped after 16 of 64 deliveries
because dropping a plain byte vector did not return a permit. The ordinary
worker already returned permits explicitly; this test does not demonstrate that
normal worker consumption previously stalled. The new ownership contract makes
both normal consumption and discarded messages return permits automatically.

## Isolated capture baseline

`design-review-capture-baseline.json` adds a direct synchronous call to the actual
`Projector::capture`, through `capture_benchmark::CaptureBenchmark`. Each sample
creates and primes a fresh 100×100 mixed-text terminal outside measurement, then
counts the second capture, including bundle destruction, on the measuring thread.
There is no PTY or worker. Rust `GlobalAlloc` counts exclude libghostty's native
allocations. The fixture includes ASCII, blanks, accented scalars, combining
marks, wide characters/spacers and background fills. Thirty release samples before
row optimization measured 10,206 allocations / 527,832 requested Rust bytes on
every sample, with 0.732 ms median / 1.188 ms p95 capture time.

The original eight session timings remain separate. In particular,
`capture_100x100_grid` is a public session roundtrip that can consume a pending
same-generation snapshot; it does not isolate projection work and is not used to
claim capture allocation savings.

```sh
TERM=xterm-ghostty cargo run -p sprite-term --bin sprite-term-bench \
  --release --locked --offline -- --samples 30 \
  --output docs/performance/design-review-capture-baseline.json
```

## Shared rows and paint preparation

`design-review-paint-shared.json` and `design-review-capture-shared.json` record
30 release samples after row sharing. Both used Rust/Cargo 1.97.1 on the machine
specified above. Measurement ran after the workspace tests and release build
completed; neither run overlapped compilation, tests, or the other benchmark.
Fixture setup, transitions, priming, and allocation scopes remain the same.

| Scenario/pass | Allocations max | Requested bytes max | Median ms | p95 ms |
| --- | ---: | ---: | ---: | ---: |
| `whole_first_frame` | 122 | 579,376 | 0.414037 | 0.481569 |
| `whole_same_generation_blink` | 0 | 0 | 0.370313 | 0.384785 |
| `whole_hover` | 3 | 10,136 | 0.382118 | 0.399413 |
| `whole_one_row_change` | 3 | 10,136 | 0.385735 | 0.404405 |
| `split_first_frame` | 122 | 579,376 | 0.672180 | 0.716478 |
| `split_same_generation_blink` | 0 | 0 | 0.594845 | 0.634608 |
| `split_hover` | 3 | 10,136 | 0.605731 | 0.635041 |
| `split_one_row_change` | 3 | 10,136 | 0.599389 | 0.643119 |

Allocation counts and bytes were identical across all samples of each metric.
The benchmark asserts zero allocations and bytes for **every** measured blink
sample, reports maxima, and checks maxima against the allocation/byte budgets.
Both the original paint budgets and the new budgets pass. A zero allocation
budget on the genuinely nonzero `whole_first_frame` metric fails at `122 > 0`.
No original budget was relaxed.

Isolated capture decreased from 10,206 allocations / 527,832 requested Rust bytes
to **104 / 14,656** in every sample. Median/p95 decreased from
0.731974/1.187771 ms to 0.010386/0.016306 ms. The remaining allocations include
independent pane text strings: PaneSnapshot retains its own text-only storage.
These are Rust projection counts, excluding native libghostty allocations.

`CellText` stores empty strings and one Unicode scalar inline, and shares longer
UTF-8 text through `Arc<str>`. Render rows use `Arc<RenderRow>` and the palette uses
`Arc<[Rgb; 256]>`. Projector reads both Ghostty dirty layers before clearing them.
Clean visual rows reuse their identity; selection, full redraws and dirty rows
compare content before retaining an old row. Size, viewport and screen changes
invalidate row reuse. Live row metadata is read separately because Ghostty permits
stale nonvisual metadata in its render cache. A prompt-only OSC 133 transition
with a stationary cursor is a regression test for this distinction. History
capture updates the same render state without clearing pending render dirtiness.

The per-view layout cache accepts row identity and generation-checked hover;
hover rebuilds only the affected row, and a one-row update rebuilds one row.
Positioned rows use `Arc<Vec<PositionedCell>>`, so moving a prepared vector into
shared storage does not copy its allocation. Whole and split passes share the
outer row collection and snapshot palette directly. Live painting no longer
allocates a `Vec<Drawn>` per ephemeral element: it resolves colors lazily for the
background and text traversals. This repeats inexpensive decisions for a whole
or text pass; the benchmark executes those same traversals inside measurement.
Glyph shaping/painting, GPUI string conversion, image element construction,
geometry submission, and GPU work remain outside the benchmark. Zero allocations
therefore describes the measured CPU preparation/decision seam, not an entire
GPUI frame.

```sh
TERM=xterm-ghostty cargo build --workspace --bins --release --locked --offline
TERM=xterm-ghostty target/release/sprite-paint-bench --samples 30 \
  --output docs/performance/design-review-paint-shared.json \
  --check-budgets docs/performance/design-review-paint-baseline.json
TERM=xterm-ghostty target/release/sprite-term-bench --samples 30 \
  --output docs/performance/design-review-capture-shared.json
python3 scripts/check_capture_budgets.py \
  docs/performance/design-review-capture-shared.json \
  docs/performance/design-review-capture-baseline.json
TERM=xterm-ghostty target/release/sprite-paint-bench --samples 30 \
  --check-budgets docs/performance/design-review-paint-shared.json
```

The capture checker checks maximum counts/bytes and p95 isolated timing. A copy
of the new report with `allocations_per_capture.budget = 0` fails at `104 > 0`.
The actual report passes against both the original and new capture budgets.

### Paint budgets re-frozen after the bug-class audit

`design-review-paint-shared.json` was re-frozen on 2026-10-09 at commit d1fceb0
by rerunning the full 30-sample release benchmark, per the regression policy in
`checkpoint-1.md`. Machine: Darwin arm64, Apple M5 Pro, 18/18 cores;
rustc 1.97.1 (8bab26f4f 2026-07-14); Cargo's default release profile; no
compilation, tests or other benchmark running (one idle Sprite window, under
1% CPU, was open). The run passed `--check-budgets design-review-paint-baseline.json` (the original
pre-optimisation budgets), and a second 30-sample run passed
`--check-budgets design-review-paint-shared.json`.

| Scenario/pass | Allocations max | Allocation budget | Requested bytes max | Byte budget | Median ms | p95 ms |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| `whole_first_frame` | 3,122 | 3,435 | 747,376 | 822,114 | 0.343792 | 0.365375 |
| `whole_same_generation_blink` | 0 | 0 | 0 | 0 | 0.352958 | 0.428875 |
| `whole_hover` | 53 | 59 | 12,936 | 14,230 | 0.330458 | 0.364208 |
| `whole_one_row_change` | 53 | 59 | 12,936 | 14,230 | 0.294417 | 0.322375 |
| `split_first_frame` | 3,122 | 3,435 | 747,376 | 822,114 | 0.491125 | 0.507500 |
| `split_same_generation_blink` | 0 | 0 | 0 | 0 | 0.430625 | 0.477458 |
| `split_hover` | 53 | 59 | 12,936 | 14,230 | 0.431167 | 0.450167 |
| `split_one_row_change` | 53 | 59 | 12,936 | 14,230 | 0.431500 | 0.440917 |

Why the budgets moved: cell text is now a `SharedString` made when a row is
laid out, not a `String` made each time a cell is painted. Laying out a row now
allocates once per non-ASCII cell (printable ASCII borrows a static string;
the fixture has 50 non-ASCII cells per row), so first frame, hover and one-row
change rose inside this seam. Per-frame paint, which is outside this seam,
fell: `a62247e` made a new string for every one of the 6,600 glyph cells on
every frame, idle frames included, and paint now makes none. Same-generation
blink still allocates nothing. See `bug-class-audit.md`.

### Carried timing gates

The original session, observation and graphics harnesses were also run, without
concurrent compilation/tests. Graphics used 30 samples (its existing large-image
cap remains 10); observation used 30 (its existing stalled/history caps remain 6).
Reports are `design-review-graphics-shared.json` and
`design-review-observation-shared.json`. All four graphics latency gates pass;
image storage remains 65,536 bytes after both 30 and 60 transmissions. Seven of
eight Checkpoint 2 session gates and three of five Checkpoint 3 observation gates
pass. The following carried gates **do not pass**:

| Metric | Measured median ms | Measured p95 ms | Carried budget ms |
| --- | ---: | ---: | ---: |
| `capture_100x100_grid` | 0.041324 | 0.150657 | 0.097317 |
| `collect_sixteen_panes` | 0.002873 | 0.003070 | 0.002956 |
| `encode_default_request` | 1.327925 | 1.495205 | 1.416955 |

`checkpoint-5.md` already recorded breaches of all three carried gates. That
history does not prove the present differences are noise. In this task's
contemporaneous pre-optimization report, `capture_100x100_grid` measured
0.022249 ms median / 0.096404 ms p95, versus 0.041324 / 0.150657 after sharing.
The session metric waits only for the first changed generation while filling,
and accepts `generation >=` after requesting capture. A pending snapshot can
therefore satisfy it before the requested capture is processed. Capture commands
do not themselves increment generation. The full-scrollback metric, conversely,
waits for a newer generation while output continues. These timings cannot
attribute a change to isolated projection work. The separate synchronous seam
shows the actual primed projection is faster with fewer Rust allocations.

The observation harness uses stand-in pane data and never invokes Projector or
the terminal painter; those code paths were unchanged by this task. Its two
breaches remain unresolved carried gates, not evidence of a measured improvement
from row caching. No timing budget was changed and no repeated run was selected
to make these gates green. Parent review tracks this limited historical-budget
exception separately from the passing new allocation gates.

## Complete issue coverage

The [issue-item proof record](design-review-coverage.md) maps all three waves to
source, executable tests, final gate results and unresolved measurement limits.
