## Handoff: reduce Sprite CPU during frequent small updates

Continue the investigation in #159. The goal is lower terminal-process CPU during agent sessions without reducing update frequency, responsiveness, or rendering correctness. This handoff records work paused on November 7, 2026; it does not claim a completed fix.

### Repository and saved work

- Repository: https://github.com/HundredBillion/Sprite
- Branch: `fix/159-small-update-cpu`, based on `39c6ffc`.
- Local worktree: `~/Projects/Sprite/.worktrees/issue-159-small-updates`.
- Design and pause notes: `docs/plans/2026-11-07-small-update-cpu.md` on that branch.
- No fix PR exists yet. The branch contains an experimental test scaffold, not an implemented optimization. Neither compilation nor RED/GREEN verification has completed.

### Findings and limits

The laptop is a ThinkPad P1 Gen 4, i7-11850H, Linux/Hyprland, with a history of thermal shutdowns. Terminal CPU is separate from the Codex/Claude child processes running inside it.

Synthetic output used 28 static rows of 79 ASCII characters and one animated status line, hidden cursor, alternate screen, no scrolling. Both terminals received identical bytes in 900×700 windows using JetBrainsMono Nerd Font (Sprite 12px, Alacritty 9pt). Actual grids differed: Sprite 122×48, Alacritty 128×42; the populated region fits both.

Each phase lasted three seconds. Values are percent of ONE logical core, measured for the terminal process over complete 250ms sample intervals.

| Workload | Sprite 1 | Sprite 2 | Alacritty 1 | Alacritty 2 |
| --- | ---: | ---: | ---: | ---: |
| Quiet, populated | 0.79 | 1.19 | 0.00 | 0.00 |
| Populated, one line at 40 Hz | 25.78 | 11.12 | 5.71 | 5.37 |
| Populated, one line at 5 Hz | 3.22 | 1.44 | 0.71 | 0.36 |
| Cleared, one line at 40 Hz | 7.50 | 6.44 | 5.35 | 3.57 |
| Quiet, cleared | 0.71 | 0.71 | 0.00 | 0.36 |

Unchanged text contributes substantially to Sprite's small-update cost. Quiet windows do not show the same cost. A separate ten-second live sample attributed about 4.9% to the UI/main thread and 0.2% to terminal/PTY workers.

These are short exploratory measurements with substantial variability, background activity, and imperfect font/chrome normalization. They do not establish a universal performance ratio, a sustained thermal improvement, or a symbolized Linux root cause.

Installed Sprite SHA256: `4a86799c797ade8fffb1801b4be17a05bdaa1188f49fad60940617e3aed7ad9a`. Its exact source revision remains unverified. Alacritty was 0.17.0, SHA256 `0cf523596ad06112922350a3d7c0eb0875c7c45b7c05b0dcc405a40afc50e624`.

**Do not use the existing local `target/release/sprite` as a clean baseline.** It emits `[SPRITE141_FRAME]` and `[SPRITE141_PRESENT]` instrumentation. No installed binary was replaced.

### Relevant code and previous fix

At base `39c6ffc`:

- `terminal_view.rs` notifies on newer snapshots.
- `terminal_view/render.rs` prepares visible grid elements.
- `grid_paint.rs::paint_rows` traverses rows/cells and prepares paint work even when shapes are cached.
- `ShapeCache` avoids repeated shaping; it does not retain all row paint/scene preparation.
- Current source already uses `window.paint_layer`. #152 and merged #153 addressed a GPUI draw-order bottleneck. Do not reapply that change or treat the macOS profile in #152 as a profile of this Linux binary.

Alacritty uses glyph-instance batching in its v0.17.0 GL renderer. This investigation did NOT establish that it visits only damaged rows. Both terminals use GPU rendering; CPU preparation can still dominate.

### What the branch actually contains

- A test-only `PAINTED_ROWS` counter in `grid_paint.rs`.
- A private `RetainedGrid` adapter in `grid_paint/retained.rs`. It currently just returns `grid.into_any_element()`; there is no caching.
- A `RetainedGrid` field initialized in both `TerminalView` constructors and wired into the background-grid rendering path.
- Regression scaffold `a_small_update_repaints_only_its_row_in_the_same_frame` in `grid_paint/tests.rs`.

The test starts with three glyph rows, empties the middle row, and expects two glyphs to remain visible in the same frame while only row 1 repaints. It uses `ink_test_glyphs(true)` and notification/draw, not `window.refresh()`, because refresh disables reuse. It is intended to fail against the no-op adapter, but has NOT yet compiled or demonstrated that failure. Fix any test setup errors before treating it as evidence.

### Candidate design, reviewed but not implemented

Try retaining one cached GPUI child view per visible row through public `AnyView::cached`; keep the outer TerminalView uncached. Reuse unchanged row entities and replace changed entities during parent rendering. Notifying an existing child during parent rendering is too late for the current invalidation pass.

Cache identity must cover positioned-row identity/index, effective cursor and color, palette/default colors, font/cell metrics, scale, and render pass. Bound memory by visible rows; do not retain an entire historical grid per row.

Preserve the original full-grid origin and absolute row indices. Independent row layout rounding or tighter clipping can introduce seams. Initially fall back to the existing whole-grid path for split-image rendering and fractional scales/row heights. Preserve image, cursor, decoration, and Surface composition.

Reviewer findings in vendored GPUI:

- `view.rs`: `AnyView::cached` provides the public cache boundary. A cache miss on a cached parent can disable descendant reuse during painting; do not add a cached intermediate grid parent.
- `window.rs`: invalidation is processed before rendering. Dirty children mark ancestors, not all siblings.
- `paint_index`/`reuse_paint` are crate-private; avoid depending on them from Sprite.
- `scene.rs` still replays cached primitives. Fewer prepared rows alone is NOT proof of lower total CPU.

### Next steps and acceptance criteria

1. Inspect current instructions, branch, source, and local evidence. Obtain a clean source-identified baseline; do not assume the installed executable matches the checkout.
2. Complete baseline tests. Existing tests can initially exclude the intentional new regression, but a clean-source baseline is also needed for comparisons.
3. Compile and run the new regression; establish the expected failure before implementing retention.
4. Implement the smallest justified optimization. Verify same-frame content and unchanged-row preservation, not just cache identity.
5. Cover cursor motion/blink/focus, selection/hover, palette/font/cell changes, resize, full refresh, scrolling, fallback transitions, images, decorations, and Surface draw order.
6. Profile an owned test process with symbols to distinguish grid traversal, scene insertion, driver work, and presentation. Existing stripped binaries were insufficient for function-level Linux attribution.
7. Compare clean before/after binaries using identical output/config/geometry, repeated trials and controlled ordering. Include populated updates, scrolling/output, idle, CPU per update, and responsiveness. Keep thermal failures and variability in the report.
8. Request review and open a PR linked to #159 with reproducible workloads and evidence. Do not claim a fix based only on a row-counter test or reduced animation frequency.

### Thermal constraints and paused build

Do not change Turbo policy, weaken guards, or replace the user's running terminal to obtain favorable measurements. Coordinate with other active GPU/LLM work.

Prior builds used one Cargo job, systemd CPUQuota 20% (5ms period), MemoryMax 5G, RuntimeMaxSec 1800, CPU below 65°C stable for three seconds before starting, and abort at 85°C or AC disconnect. The Python guard's exit code alone is insufficient: inspect its JSON receipt.

The first build stopped at 85°C. A second build with a temporary authenticated fan session was explicitly stopped when the user paused. It had not completed dependency compilation. The test scaffold was added during that compilation, so it was no longer an untouched-baseline build.

The fan session used normal level 7, a 30-second hardware watchdog, a 20-minute maximum, and automatic restoration. Its receipt records `restored`, errors `[]`, automatic fan control, and restoration of the LAS fan service. Its early-stop marker exists; do not blindly restart the old session. Future privileged sessions need current authorization and bounded cleanup.

The previous build reused `~/Projects/Sprite/target` and passed Cargo config overrides for `GHOSTTY_SOURCE_DIR` pointing to `~/Projects/Sprite/vendor/ghostty`, plus `ZIG_GLOBAL_CACHE_DIR` and `ZIG_LOCAL_CACHE_DIR` under that target. The worktree's Ghostty submodule was empty. Account for that before building; do not mistake a missing submodule for a regression.

### Local evidence availability

These files are local to the original laptop, not GitHub artifacts:

- `~/.local/state/sprite-cpu-20261107/REPORT.md`: full exploratory report.
- Same directory: `workload.py`, comparison/config scripts, `provenance.json`, `installed-repeat-results.json`, `alacritty-results.json`, `second-thermal-stop-results.json`, and retained thermal-stop records.
- `~/.local/state/sprite-159/`: `guard.py`, build logs/receipts, fan helper and restoration receipt.

A different-machine agent can use this issue and the branch to continue, but must obtain those local artifacts or regenerate measurements. Publish only synthetic evidence; private terminal contents are unnecessary.
