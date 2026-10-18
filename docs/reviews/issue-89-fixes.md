# Four close-warning and rendering fixes

Issue: https://github.com/HundredBillion/Sprite/issues/89

Baseline: `d588758` (master including merged URL-click PR #98).
Host: macOS arm64; installed Rust/Cargo 1.98.1 (repository toolchain pin 1.97.1);
Zig 0.17.0; GPUI 0.2.2 from Cargo.lock and vendor/gpui;
libghostty source from the repository's pinned submodule.
Finding status and guarding tests live in `findings.toml` (SPR-024–SPR-027).

## Reproduction and acceptance

The original issue's four real-session/GPUI tests were applied to current
master and all four failed on their reported symptoms. After the fixes, all
four pass, plus a crop/offset geometry test at 1× and 2×.

| Finding | Before | After and coverage |
|---|---|---|
| Close classification | Direct `/bin/cat` gave no pane-close warning; direct and interactive-shell `exec` tests both reported `Idle`. | Application test warns. Two real PTY tests observe actual cat output and report `Busy(Some("cat"))`; the `exec` test first establishes an idle interactive shell. Existing idle-shell and completed-foreground-job tests pass. |
| Fixed image geometry | A ten-column image at 2× measured 170 logical pixels instead of 85. | It measures 85. Real Kitty crop/offset fixture checks outer dimensions, cell-relative offsets, source crop positioning, and full source image scaling at 1× and 2×, allowing physical-pixel rounding. |
| Composition anchor | The normal blink clock moved Japanese preedit from (78,76) to (11,12). | Both phases retain (78,76). Input delivery is unchanged; only composition positioning stops filtering out the hidden cursor. |
| Collective pressure | Text-only output replaced 2/2 retained textures with identical image IDs/generations. | Replaced 0/2. Cache tests additionally cover zero/shrinking/growing budgets, content invalidation, malformed/oversized refusal, disappearing images, and bounded storage across 128 changing snapshots. |

## Why these changes

Foreground group equality identifies the initial child, which may be either a
shell or an application. The watcher now checks the live group's current
kernel program name. The initial group is idle only for recognized shell names;
applications launched directly or through shell `exec` are busy. This reuses
the existing process-name implementation, without changing process cleanup or
propagating launch flags that would become stale after `exec`.

Shell names remain a heuristic: custom/renamed shells ask conservatively;
applications impersonating a recognized shell name may appear idle, and a
noninteractive script retaining a shell name is not distinguished from a prompt.
Unreadable metadata retains the existing `Unknown` policy. This is a close
warning, not a security identity check. Idle queries now perform live liveness
and name checks; that OS-query cost was not separately benchmarked.

The fixed-image path now converts physical dimensions and within-cell offsets
using the window scale exactly once at its rendering boundary. Source-image
crop coordinates remain image pixels; their scaling becomes logical too.
Cell-relative positions already use logical cell metrics and are not divided.
The separately painted placeholder-tile path is unchanged. The
[GPUI 0.2.2 scale contract](https://docs.rs/gpui/0.2.2/gpui/struct.ScaledPixels.html)
and the [Kitty graphics protocol](https://sw.kovidgoyal.net/kitty/graphics-protocol/)
agree with the installed source and the real geometry probes.

Composition needs the terminal cursor's location. Cursor-paint visibility is
independently controlled by `GridPaint::prepare`; removing the composition's
blink filter preserves ordinary cursor blinking without touching UTF-16 range
handling, native commits, or key-fallback suppression.

A per-image LRU request cannot know that the texture it evicts will be requested
later in the same snapshot. `GraphicsCache::refresh` instead reserves current
ID/generation entries first, drops obsolete ones, then admits missing images
into remaining capacity without evicting those survivors. No persistent state
or dependency is added. Previously retained images have priority; a cold cache
uses input-order first-fit. Refused images continue through the existing warning
and budget-reload paths.

## Cache workload measurement

Bounded sibling checks found seven fixed-image pixel casts (four in the fixed
renderer, three in tests), all now divided by scale. The four remaining
blink-filter expressions belong to cursor painters or their tests/benchmark;
hosted-grid composition separately reads its raw cursor snapshot. The five
remaining direct `textures.texture` calls are test fixtures; production
snapshot refresh uses the batch boundary.

Measured three samples each on the same host in debug builds. Each sample warms
the actual cache, then refreshes 100 identical snapshots of 256×256 RGBA images.
Timers cover the cache refresh/conversion work; they do not measure GPU frames.

| Workload | Before median | After median | Conversions before → after |
|---|---:|---:|---:|
| One image; room for one | 30 µs | 30 µs | 0 → 0 |
| Twelve images; room for twelve | 227 µs | 330 µs | 0 → 0 |
| Twelve images; room for eight | 2,839,142 µs | 278 µs | 1,200 → 0 |

The pressured cache retains 2 MiB in both versions. The fitting twelve-image
case adds about 1.03 µs per refresh in this synthetic workload. The experiment
supports eliminating conversions under pressure; it makes no frame-rate claim.
The temporary benchmark was removed from production tests; exact harnesses and
logs are retained under `/tmp/sprite-89-cache-*` for this session.

## Verification boundaries

`cargo test --workspace --locked --offline --no-fail-fast` passed 1,099 tests,
with zero failures and seven ignored, including doctests and excluding nested
child-test reports. All five earlier-fix guards named by the issue passed.
Workspace/all-target Clippy passed with warnings denied and the existing
`chunks_exact_to_as_chunks` lint in unchanged `png_decoder.rs` allowed.
Existing GPUI dependency warnings remain. Formatting, diff checks, ledger
validation, and 14 ledger-checker Python tests passed. Independent source
review found no unresolved findings.

The tests use real terminal child processes and GPUI's headless layout/input
platform. Native desktop pixel appearance, physical IME candidate panels, and
live Linux compositor execution have not been qualified by these checks.
The fixes add no vendor patches or dependency updates. The issue is closed by
the fixing PR's merge rather than by treating these source changes as already
installed in the user's application.
