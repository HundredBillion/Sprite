# Close every open finding

## Outcome

PR #76 left ten findings tracked by GitHub issues: six open, four deferred.
This PR closes all ten. Eight are fixed, each guarded by a regression test that
fails against `03b4929`. The other two are recorded as `wontfix` with a reason
that the existing measurements support. The ledger has no `open` or `deferred`
entry afterwards.

| ID | Issue | Defect | Disposition here |
|---|---|---|---|
| GAP-C01 | #61 | A Dock Quit, menu-bar Quit or logout skips pane cleanup. | Fixed: the quit goes through the in-app quit path (R-Q) |
| GAP-C03 | #63 | Small SVG filter documents block the UI thread for seconds. | Fixed: decoded off the UI thread, filter work capped (R-V) |
| GAP-C04 | #64 | The PKGBUILD labels a fixed version but fetches git HEAD. | Fixed: the source is pinned to the release tag (R-P) |
| GAP-C06 | #66 | When rows rotate instead of being evicted, the selection anchor drifts. | Fixed: a sentinel pin detects the rotation (R-S) |
| GAP-C07 | #67 | A slowly draining Surface writer keeps a connection slot. | Fixed: every batch has a drain deadline (R-D) |
| GAP-C08 | #68 | Placeholder tiles snap against the previous frame's origin. | Fixed: tiles are placed at paint time (R-G) |
| GAP-C10 | #57 | Arch CI is flaky on timing-sensitive tests. | Fixed: the remaining fixed waits are removed and the jobs are bounded (R-W) |
| GAP-C14 | #73 | Cached GPUI views per pane. | `wontfix`, with the measurements as the reason (R-X) |
| GAP-C15 | #74 | Divider-drag per-move layout work. | `wontfix`, with the count tests as the reason (R-X) |
| GAP-C16 | #75 | Kitty Unicode-placeholder row and column inheritance. | Fixed: implemented per the spec (R-K) |

## Constraints

- Rust 1.97.1, pinned dependencies, no new crates, no async runtime.
- No new threads. Background work runs on GPUI's existing background executor.
  The only dedicated threads remain the Surface writers and pane cleanups
  (ADR 0029).
- The vendored GPUI changes only through the patch that
  `scripts/check_gpui_patch.py` verifies, and `vendor/gpui.provenance.md`
  records the change.
- The Surface Channel and observation wire protocols are unchanged.
- `scrollback_bytes = 0` still means "this pane keeps no history". The tests
  `scrollback_is_bytes_and_is_bounded` and `a_zero_scrollback_budget_keeps_no_history`
  stay unchanged.
- Work happens on `fix/close-open-findings` in `.worktrees/close-open-findings`.
  No release or version bump.
- Every fix is test-first: the regression is written, seen to fail against
  `03b4929`, then made to pass. Tests cross the real seam: GPUI view, worker,
  socket or platform callback.

## R-Q. Every quit goes through the in-app quit path (GAP-C01, #61)

- R-Q1 The vendored GPUI's macOS application delegate implements
  `applicationShouldTerminate:`. When the app has registered a should-terminate
  callback, the delegate returns `NSTerminateLater` and calls the callback with
  a reply handle. The handle's single method answers allow or cancel through
  `replyToApplicationShouldTerminate:`. When no callback is registered, the
  delegate returns `NSTerminateNow`, which is today's behaviour.
- R-Q2 A quit the app itself requested after its cleanup finished (`cx.quit()`
  from `quit_app`) is not intercepted again. The delegate returns
  `NSTerminateNow` for it.
- R-Q3 The GPUI test platform can simulate the platform's should-terminate
  request, so a test crosses the same callback the Cocoa delegate calls.
- R-Q4 Sprite registers the callback at start-up. It runs the same decision as
  the quit key (`Workspace::quit` → `may_close(CloseScope::Quit, …)`):
  - When no pane is busy, or the request answers an open quit question, the
    answer is allow. It is sent after `shutdown_and_quit` has waited for every
    pending cleanup, with the HUP → TERM → KILL escalation.
  - When a pane is busy and no question is open, the question is put on screen
    as it is for the quit key, and the answer is cancel, sent at once, so a
    logout is never left waiting on an unanswered question. The next quit
    request, from the Dock or the key, answers the question.

  A logout, restart or shutdown is a Quit Request like any other: with a busy
  pane, it is cancelled and the question shown, as Terminal.app does. The quit
  reason is not inspected. Linux behaviour is unchanged.
- R-Q4b The quit question's instruction for repeating the request changes from
  "press the same keys again" to "quit again", because a Dock or menu Quit
  repeats it without keys.
- R-Q4a ADR 0032 records this decision and the rejected alternatives.
- R-Q5 Tests:
  - A GPUI test simulates the request with an idle pane and a cleanup gate
    held. It asserts the request has no answer while the gate is held, and is
    allowed once the gate is released.
  - A second GPUI test, with a busy pane, asserts that the first request is
    cancelled with the question on screen, and that a second request is allowed
    after cleanup.
  - Manual evidence on a macOS desktop: with
    `sh -c 'trap "" HUP; exec sleep 1000'` in a pane, Dock → Quit twice, then
    `pgrep -f "sleep 1000"` finds nothing. The child ignores the hangup on
    purpose: a plain `sleep` dies on the hangup even without this fix, so it
    proves nothing.

## R-V. SVG decoding leaves the UI thread, and filter work is capped (GAP-C03, #63)

- R-V1 Both SVG callers decode on the background executor: the element image
  cache (`surface/render.rs`) and the list view's asset cache
  (`terminal_view/list_view.rs`). Each entry is one of `Pending`, `Ready(image)`
  or `Refused(reason)`. A pending entry draws nothing. A finished decode
  notifies the view that asked for it. The UI thread never parses or
  rasterises SVG.
- R-V1a Each cache runs at most `MAX_CONCURRENT_SVG_DECODES` (2) decodes at a
  time. Further lookups queue in FIFO order. Peak memory for filter
  intermediates is therefore bounded per cache by two capped decodes, not by
  the number of images in a description.
- R-V2 The pixel budgets of ADR 0022 are charged when a decoded image is
  installed. A decode whose result no longer fits the budget is dropped, as
  today. A decode whose entry is no longer wanted (the description changed) is
  discarded when it finishes.
- R-V2a A document refused by the filter cap (R-V3), or one that does not parse,
  stays refused for as long as a description draws it, so it is not decoded
  again on every update. A decode refused only for lack of budget is retried
  when the next description arrives, as today.
- R-V3 The background task walks the parsed `usvg` tree before rasterising. It
  counts filter primitives once for every group that applies them, so a shared
  filter counts once per use. A document above `MAX_FILTER_PRIMITIVE_USES` is
  refused with a reason. The cap is chosen from measured release-build decode times so that a
  document at the cap decodes in under 250 ms on the development machine; the
  value and that measurement are recorded in the code and in ADR 0022, not
  asserted by a test.
- R-V4 The font database's first load happens in the background task too, not
  on the UI thread.
- R-V5 ADR 0022 is amended. Decoding is asynchronous, and filter work is
  bounded by counting the uses of filter primitives. Parse cost is bounded by
  the existing message and asset byte limits.
- R-V6 Tests. No test asserts on wall-clock time:
  - A GPUI test looks up a document rebuilt from round 10 (N named `feFlood`
    results merged by one `feMerge`). Before the test executor runs background
    work, the entry is `Pending` and nothing has been decoded. After
    `run_until_parked`, the image is `Ready` and the view was notified. This
    fails today, because the lookup decodes synchronously.
  - A 128-result document ends up `Refused`, and the test counter shows it was
    never rasterised.
  - The existing cache and list-view tests wait for decodes to finish.

## R-S. Two sentinel pins detect a rotated-out anchor (GAP-C06, #66)

- R-S1 When a gesture starts, `track_selection_anchor` also tracks up to two
  sentinel `TrackedGridRef`s: one on the screen row directly above the anchor,
  and one on the row directly below it. A sentinel is absent when the anchor
  is on the first or last screen row.
- R-S2 `apply_selection` treats the anchor as lost when any of these holds,
  and then clears the selection exactly as BCA-26 does:
  - The anchor's snapshot is `None`.
  - A present sentinel's snapshot is `None`.
  - The anchor and a present sentinel report the same row in screen
    coordinates.
  - The other screen is now active (today's check).
- R-S3 Why this works: when libghostty rotates rows out instead of evicting a
  page (no scrollback: `Terminal.index` → `Screen.cursorScrollRegionUp`), the
  pin on the erased row is left in place, or moved one row above the scroll
  region when the region's top margin is not row 0, and the pins below it
  shift up by one.
  - In a full-screen scroll the erased row is row 0, and the anchor's row is
    erased exactly when the sentinel below lands on it.
  - In a scroll region, the anchor's pin moves onto the static row above the
    region, where the sentinel above sits.
  - When the row below the anchor is erased, the gap stays one row and nothing
    is cleared.
- R-S3a Accepted imprecision. Each case clears a selection; none leaves one on
  the wrong text:
  - On a screen with no history, libghostty leaves an erased pin on row 0. A
    gesture anchored below row 0 is therefore cleared one scroll early: when
    its text reaches row 0, not when that text leaves the screen.
  - A one-row screen has no sentinels and keeps today's behaviour.
  - A widening reflow during a gesture can merge the anchor's row with an
    adjacent wrapped row, which clears the selection.
- R-S4 This covers the primary screen with zero scrollback, the alternate
  screen (always created without scrollback) and scroll regions. Scrollback
  configuration is unchanged.
- R-S5 Tests in `crates/sprite-term/tests/selection.rs`, next to
  `a_gesture_whose_anchor_was_evicted_selects_nothing`, all failing today:
  - `a_gesture_whose_anchor_was_rotated_out_with_zero_scrollback_selects_nothing`
  - `a_gesture_whose_anchor_was_rotated_out_on_the_alternate_screen_selects_nothing`
  - `a_gesture_anchored_on_the_last_row_is_cleared_before_it_drifts`
  - `a_gesture_anchored_mid_way_down_a_wrapped_line_is_cleared_when_its_row_rotates_out`
  - `a_gesture_inside_a_scroll_region_is_cleared_when_its_row_rotates_out`

## R-D. A slow reader cannot keep a writer slot indefinitely (GAP-C07, #67)

- R-D1 `write_events` gives each batch an overall deadline of
  `WRITE_TIMEOUT + bytes / MIN_DRAIN_RATE`, with `MIN_DRAIN_RATE` at 64 KiB/s.
  A full 4 MiB queue therefore has about 66 s. The constants live in
  `timeouts.rs`, and a test-scale rate applies under test, as `WRITE_TIMEOUT`
  already does.
- R-D2 The batch is written in chunks. Before each chunk the socket's write
  timeout is set to the smaller of `WRITE_TIMEOUT` and the time left. At the
  deadline the connection is dropped and the writer returns, releasing its
  slot.
- R-D3 ADR 0029 gains the drain-floor rule. The code comment that describes
  the gap is rewritten to describe the bound.
- R-D4 Test: `a_draining_writer_keeps_its_connection_slot_until_it_returns`
  becomes `a_reader_slower_than_the_drain_floor_loses_its_slot`. Its reader
  takes a few bytes at a time, each read inside the write timeout, and the test
  asserts the slot is free after the deadline. The stalled-writer test is
  unchanged.

## R-G. Placeholder tiles are placed at paint time (GAP-C08, #68)

- R-G1 Tile geometry (`placeholder::tile_geometry`) is computed in
  prepaint/paint against the current `bounds.origin`, the same way `GridPaint`
  snaps text. It is no longer computed in `Render::render` against the last
  frame's `content_origin`.
- R-G2 `content_origin` stays for its other reader (`input.rs`). The comment
  admitting the stale origin is removed.
- R-G3 The issue's trigger, a divider drag, cannot misalign tiles: layout
  rounds every position to whole device pixels, and snapping is unchanged by a
  whole-pixel shift. The trigger is a display-scale change, such as moving the
  window between a 2× and a 1× display, after which the previous frame's corner
  sits between device pixels.
- R-G4 The GPUI test window gains a settable scale factor, carried as a third
  patch in `patches/` and replayed by `check_gpui_patch.py`. Test: a GPUI test
  places a Kitty image in an unfocused pane and changes the window's scale from
  2.0 to 1.0. It draws the next frame without a forced `window.refresh()` and
  asserts the painted tiles lie on that frame's cell edges. It fails today.

## R-K. Kitty Unicode-placeholder inheritance (GAP-C16, #75)

- R-K1 In `placeholder.rs`'s `image_cells`, each placeholder cell may inherit
  from the placeholder cell immediately to its left on the same row. Ghostty's
  `IncompletePlacement` in the vendored source is the reference:
  - With no diacritics, the cell continues its left neighbour when the image
    and placement IDs match, taking the neighbour's row and the neighbour's
    column + 1.
  - With only a row diacritic, the cell continues its neighbour when the row
    matches.
  - The third diacritic, the high byte of the image ID, is decoded, and is
    inherited the same way.
- R-K2 A 256-colour (palette) foreground is accepted as an image ID, as in
  Ghostty's `colorToId`.
- R-K3 Tests: ports of Ghostty's continuation tests, a palette-ID test and a
  high-byte test. The existing test asserting that a cell with only a row
  diacritic is skipped is updated to the inheritance rule.

## R-W. The flaky tests wait on conditions, and CI jobs are bounded (GAP-C10, #57)

- R-W1 Each remaining fixed wait in the #57 tests becomes a condition wait
  bounded by `watchdog()` or `PROGRESS_CEILING`:
  - `natural_exit_deadline_is_not_renewed_by_continuous_output`: the 10 s exit
    wait becomes `GIVE_UP_AFTER + watchdog()`.
  - `probe()` in the sprite-term test support: its fixed 20 s loop.
  - `a_running_program_is_reported_by_name`: the child becomes `sleep 600` and
    the test kills it, so the condition cannot expire.
  - In `submission_regressions.rs`: the fixed `recv_timeout(5 s)` waits and the
    5 s retry deadline.
- R-W2 The Arch and macOS CI jobs get `timeout-minutes` set to about twice
  their recent successful run time, so a hang fails within that time instead
  of after six hours.
- R-W3 `completed_removed_cleanup_is_pruned_before_shutdown`, which hung CI
  for six hours before #76, is looped under load with the others. If it hangs,
  it is fixed here.
- R-W4 #57's "done when" is replaced by:
  - each listed test (with `a_refused_reload_is_kept_whole_and_reverted_whole`
    standing in for the deleted `accepted_colors_revert…`) passes 100 looped
    runs under CPU load (one `yes` per core) on Linux;
  - this PR's Arch job passes;
  - five `workflow_dispatch` Arch runs on the branch pass.

## R-P. The PKGBUILD builds the version it names (GAP-C04, #64)

- R-P1 `packaging/PKGBUILD` uses `source=("git+$url.git#tag=v$pkgver")`.
  `prepare_release.py` needs no change because the source expands from
  `pkgver`. A build that runs before the tag is pushed fails instead of
  building HEAD.
- R-P2 `scripts/test_packaging_automation.py` asserts the pin in
  `packaging/PKGBUILD`.

## R-X. Two deferred findings become `wontfix` (GAP-C14 #73, GAP-C15 #74)

- R-X1 GAP-C14's reason: an unchanged pane prepares in about 0.3 ms, with no
  allocations and no shaping (`docs/performance/bug-class-audit.md`), and
  `shaping_happens_only_for_cells_whose_drawn_text_changed` guards that.
  Caching views would also make a pane miss resizes, because `set_allocated`
  does not notify.
- R-X2 GAP-C15's reason: the work per move is linear and test-bounded
  (`placement_matching_scales_linearly`,
  `title_membership_reconciliation_scales_linearly`), and PTY resizes happen
  only when the drag crosses a cell boundary. The earlier claim that
  `docs/performance/` holds divider-drag numbers was wrong; this reason
  replaces it.

## R-L. Ledger and PR

- R-L1 `docs/reviews/findings.toml`:
  - GAP-C01, C03, C04, C06, C07, C08, C10 and C16 become `resolved`, each with
    `fixed_by` and the test(s) named above.
  - GAP-C14 and C15 become `wontfix` with the reasons in R-X.
  - `scripts/check_findings.py` passes.
- R-L2 The PR body carries the Findings table and
  `Closes #57, #61, #63, #64, #66, #67, #68, #73, #74, #75`.

## Explicitly out of scope

- Busy-pane confirmation or cleanup on Linux session logout. Only the macOS
  delegate path is in #61's scope.
- Bounding `usvg` parse time beyond the existing byte limits.
- A minimum scrollback, or any change to what `scrollback_bytes = 0` means.
- New performance benchmarks for multi-pane paint or divider drag.

## Evidence

- The gate #76 ran:
  - `cargo fmt --all -- --check`
  - `cargo clippy --workspace --all-targets -- -D warnings`
  - `cargo test --workspace --no-fail-fast`
  - the Python suites
  - `scripts/check_findings.py`
  - `scripts/check_gpui_patch.py`
- Each new regression test, seen to fail at `03b4929` before its fix.
- The R-W4 loops on Linux under load (Docker `rust:1.97.1-bookworm`).
- The manual Dock-quit check (R-Q5).
- The PR's hosted Arch and macOS runs, plus five `workflow_dispatch` Arch runs.
