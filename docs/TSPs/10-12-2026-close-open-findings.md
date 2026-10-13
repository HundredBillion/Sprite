# Close Open Findings Technical Spec

> **For agentic workers:** REQUIRED SUB-SKILL: Use dmi-superpowers:subagent-driven-development (recommended) or dmi-superpowers:executing-plans to implement this TSP task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close all ten findings that PR #76 left tracked by issues (#57, #61, #63, #64, #66, #67, #68, #73, #74, #75), as the PRD `docs/PRDs/10-12-2026-close-open-findings.md` requires. Eight are fixed under a regression test that fails at `03b4929`; two become `wontfix` with a reason.

**Architecture:** Each finding is fixed at the seam where it lives:
- GPUI's macOS delegate gains a should-terminate hook that Sprite answers through its existing quit gate.
- SVG decoding moves to the background executor behind a two-at-a-time queue with a filter-use cap.
- The selection anchor gets a sentinel pin in sprite-term.
- Surface writers get a per-batch drain deadline.
- Placeholder tiles are placed at paint time.
- Kitty placeholder inheritance is added in `placeholder.rs`.
- The flaky tests' remaining fixed waits become condition waits.

The ledger, the PKGBUILD pin and the evidence close it out.

**Tech Stack:** Rust 1.97.1 (edition 2024), GPUI 0.2.2 (vendored under `vendor/gpui`, patched via `patches/`), libghostty-vt 0.2.1 / libghostty-vt-sys 0.2.2 (built from `vendor/ghostty` @ ab0b9da), resvg/usvg 0.45.1, Python 3 `unittest`, GitHub Actions, Objective-C runtime via the `objc`/`cocoa` crates GPUI already uses.

## Global Constraints

- Toolchain 1.97.1. On this Mac, Homebrew's `cargo-clippy` (0.1.98) shadows the pinned one and reports a false `chunks_exact_mut` error, so every cargo command puts the pinned toolchain first explicitly. Written below in full; it always reads exactly:
  `env PATH=/Users/davidlee/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin:/Users/davidlee/.cargo/bin:/opt/homebrew/bin:/usr/bin:/bin cargo`
  followed by the subcommand and `--locked --offline`. Do not use a shell variable: spell the full command out. The session's sandbox refuses commands whose program comes from a variable.
- No new crates. No async runtime. No new threads: background work runs on GPUI's existing background executor. The only dedicated threads remain the Surface writers and pane cleanups (ADR 0029).
- One libghostty-owning worker thread per Terminal Session (ADR 0008).
- The vendored GPUI changes only through the patches that `scripts/check_gpui_patch.py` replays (its `PATCHES` list, added in Task 6); `vendor/gpui.provenance.md` records each change.
- Surface Channel and observation wire protocols unchanged.
- `scrollback_bytes = 0` still means "this pane keeps no history". `scrollback_is_bytes_and_is_bounded` and `a_zero_scrollback_budget_keeps_no_history` stay unchanged.
- Linux and macOS source compatibility; the Linux-only code paths must still compile (`cargo check -p sprite-term --all-targets --target x86_64-unknown-linux-musl` where possible).
- Workspace lints are strict: `unwrap_used`, `expect_used`, `let_underscore_must_use`, `wildcard_enum_match_arm`, `significant_drop_tightening` are denied. `thread::sleep` is a disallowed method in sprite-app outside the two named sites; tests use `test_blocking_wait::{pause, watchdog}` (sprite-app) and `tests/support` `watchdog()`/`wait_until`/`wait_within` (sprite-term).
- Work happens on branch `fix/close-open-findings` in `/Users/davidlee/Projects/Sprite/.worktrees/close-open-findings`. Never `cd` out of it; use absolute paths. No release, no version bump.
- Every fix is test-first: write the regression, run it and see it fail for the stated reason, make it pass. Tests cross the real seam (GPUI view, worker, socket, platform callback).
- No test asserts on wall-clock duration.
- The shell is zsh: quote variables; a failing glob aborts the whole command line.
- Commit messages end with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- Code comments follow `dmi-superpowers:writing-code-comments`: no finding IDs, issue or PR numbers in code comments (test names describe behaviour; the ledger maps IDs to tests).
- Glossary: "Quit Request" (crates/CONTEXT.md) is any request to end Sprite; use it in docs and comments.

---

## File Structure

New files, each with one responsibility:

| Path | Responsibility |
|---|---|
| `patches/gpui-0.2.2-should-terminate.patch` | GPUI's should-terminate hook and its test-platform simulation (Task 6). |
| `patches/gpui-0.2.2-test-scale-factor.patch` | A settable scale factor on GPUI's test window (Task 8). |
| `crates/sprite-app/tests/platform_quit_request.rs` | GPUI-level contract tests for the should-terminate hook (Task 6). |
| `crates/sprite-app/src/surface/svg_decode.rs` | Parsing, the filter cap and rasterising off the UI thread, plus the two-at-a-time decode queue (Tasks V1-V3). |
| `docs/adr/0032-answer-every-quit-request-through-the-in-app-quit-path.md` | Already committed. |

Modified files are named per task. Order matters in three places:
- Task 8 needs Task 6's `PATCHES` list in `scripts/check_gpui_patch.py`.
- Tasks K1 and G1 both touch `terminal_view/placeholder.rs`. Task 3 changes only decoding and leaves `image_cells`, `ImageCell`, `fit_image`, `ImageFit`, `tile_geometry` and `TileGeometry` unchanged.
- Task 12 cites test names defined by Tasks P1-V3, and its Step 1 catches any test that was renamed.

Notes from drafting, kept for the executor:

#### From the quit draft

##### R-Q: Every Quit Request goes through the in-app quit path (GAP-C01, #61)

Two tasks. Q1 gives the vendored GPUI a should-terminate hook, a test-platform double for it, and a second recorded patch. Q2 makes Sprite answer the hook with the quit key's decision and adds the R-Q5 tests and the manual Dock check. A reviewer can reject either one without the other: Q1 changes only GPUI and its provenance, and Q2 changes only Sprite.

Design decisions (all of them are in the code below):

- **One-shot reply, allow on drop.** `gpui::TerminateReply` is answered once with `allow(self)` or `cancel(self)`. If it is dropped unanswered it **allows**. Cancel-on-drop would be the dangerous choice. A lost reply (a released window, a panicking callback, a task dropped at shutdown) would then refuse every Dock Quit forever and block logout until the user force-quits. Allow-on-drop falls back to today's behaviour: shutdown without cleanup. The reply holds a `Box<dyn FnOnce(bool)>`, so it is `!Send` and can only be answered on the main thread.
- **The answer is sent from the main queue.** `replyToApplicationShouldTerminate:` is dispatched with `dispatch_async_f` on the main queue, the same way `MacPlatform::quit` already sends `terminate:`. Two reasons:
  1. Allowing termination runs `applicationWillTerminate:` → `App::shutdown`, which mutably borrows the app state. Sprite answers from inside `cx.update`, so a synchronous reply would double-borrow.
  2. A `cancel()` given inside the callback must not reach AppKit before the delegate has returned `NSTerminateLater`.
  AppKit runs `NSModalPanelRunLoopMode` while it waits. That mode is a common mode, so the main queue drains during the wait. The manual check (Q2 Step 7) exercises this.
- **Re-entry (R-Q2): `Platform::quit` sets `quit_requested`.** When the app asks to quit itself (`App::quit` → `Platform::quit`), the decision has already been made. The delegate answers `NSTerminateNow` without calling the callback. Sprite never calls `cx.quit()` while it holds a reply: its quit path answers the held reply instead (`quit_app(Some(reply))` → `reply.allow()`). So there are exactly two exits: a platform request is answered through its reply, and a key or close-button quit goes through `cx.quit()`.
- **One reply outstanding at a time.** A should-terminate request that arrives while a reply is unanswered gets `NSTerminateCancel` at once and does not reach the callback. The outstanding reply still decides. Sprite therefore holds at most one reply.
- **Registration follows the `on_reopen` pattern.** `App::on_should_terminate` captures `self.this` (the `Weak<AppCell>`), upgrades it inside the platform callback and calls `callback(reply, &mut app.borrow_mut())`. This is the same as `Application::on_reopen` (`vendor/gpui/src/app.rs:198-209`). If the app is gone, the reply drops, which allows the quit.
- **Patch management: a second patch file, applied after the first.** `scripts/check_gpui_patch.py` gains `PATCHES = (native-text, should-terminate)` and replays them in order. It was a three-line change. The alternative was to regenerate the one patch file, which would rewrite the reviewed native-text hunks byte for byte. It would also need its own `/dev/null` handling for the added `key_text_scope.rs`. The new patch is generated against "archive + native-text patch". The generator below builds that tree with the checker's own `apply_exact_patch`, so no system `patch` is involved. It emits `difflib` unified hunks in the format the checker parses.

The design was checked before it was written down, in a scratch copy of this worktree, on macOS with toolchain 1.97.1:

- the five contract tests and the two R-Q5 tests pass, and the R-Q5 tests fail against the stub in Q2 Step 3 for the stated reasons;
- `cargo clippy -p sprite-app --all-targets -- -D warnings` is clean, and `cargo fmt --all -- --check` and `rustfmt --check` on every touched vendor file are clean;
- the generated patch passes `check_gpui_patch.py`, and the Python suite (4 tests) passes.

---

#### From the svg draft

##### R-V. SVG decoding leaves the UI thread, and filter work is capped (GAP-C03, #63)

**What is wrong today (verified at `e235320`, same code as `03b4929`):**
- `ElementImageCache::lookup` (`crates/sprite-app/src/surface/render.rs:75-97`) calls `render_svg` / `render_svg_with_budget` (`:318-388`) inline. `ElementBody::render` reaches it from `surface_element` (`crates/sprite-app/src/terminal_view/surfaces.rs:1128-1131`), which `TerminalView::render` calls. So an uncached SVG is parsed and rasterised inside the frame.
- `sync_image_cache` (`crates/sprite-app/src/terminal_view/list_view.rs:50-86`) does the same for list assets from `VirtualListView::render` (`:445`).
- `FONT_DB` (`render.rs:327-334`) runs `load_system_fonts` on first use, so the first SVG with text loads the system font database on the UI thread.
- Nothing bounds filter work. A 512×512 document with 32 `feFlood` results merged by one `feMerge` took 3.4 s in the round-10 probe.

**Owners.** `ElementImageCache` lives in `ElementBody`, inside `Body::Elements` (`surfaces.rs:34-35`), inside `HostedSurface`, inside `TerminalView.surfaces`. Its only frame caller, `surface_element`, holds `cx: &mut Context<TerminalView>`. The list cache is a field of `VirtualListView`, which is its own entity (`surfaces.rs:69`). Its `Render::render` holds `cx: &mut Context<VirtualListView>`.

**Design.**
- A new module `crates/sprite-app/src/surface/svg_decode.rs` takes over the decoder. It parses, counts filter-primitive uses, refuses documents over `MAX_FILTER_PRIMITIVE_USES`, and rasterises.
- The module also holds `SvgDecodes<K>`, one per cache. It is a FIFO queue that keeps a `SvgState` for each key: `Pending`, `Ready(Arc<RenderImage>)` or `Refused(RefusalKind)`.
- At most `MAX_CONCURRENT_SVG_DECODES` (2) decodes run on the background executor at once.
- When a decode completes, a foreground task installs the result and charges the 64 MiB budget at that point. It then notifies the entity that started the decode and starts the next queued decode.
- The queue state is an `Rc<RefCell<…>>`. The completion task holds only a `Weak`, so a dropped cache (closed Surface) discards its results.
- Each entry carries a ticket, so a result whose entry was dropped, or dropped and re-asked, is discarded.
- A frame never decodes. It only *asks*: `render` records wants and draws `Pending` as nothing. Then the owner calls `start_decodes(cx)` with its own `Context<T>`, the same `cx.background_executor().spawn` + `cx.spawn(async move |this, cx| … this.update(…))` pattern that `crates/sprite-app/src/workspace/close_gate.rs:103-139` uses.
- `RefusalKind` has a fourth variant, `Unrasterisable`, beyond the PRD's three. It covers empty, non-finite or over-4096/16 MiB rasters. Such a refusal does not depend on the remaining budget, so it sticks like `Unparsable` and `TooComplex`. Only `OverBudget` is forgotten on the next description (R-V2a).
- The list view keeps its current rule: every outcome sticks until the icon raster width changes.
- After Task 11, `decode` is private to `svg_decode`, so the compiler stops any UI path from calling it. Its statics `OPTIONS` and `FONT_DB` are therefore first touched on the background executor (R-V4).

**Test determinism.** GPUI's `TestDispatcher` (`vendor/gpui/src/platform/test/dispatcher.rs:122-182`) runs background runnables on the test thread, in seeded random order, only when ticked. This has two consequences:
- A thread-local `RASTERISED` counter in `svg_decode` sees background rasterisations. Nothing runs until `run_until_parked`.
- Completion order among the two in-flight decodes is random. So the budget tests below assert "exactly one of five is refused", not which one.

---

---

### Task 1: The PKGBUILD builds the release tag it names (R-P; GAP-C04, #64)

**Files:**
- Modify: `packaging/PKGBUILD:30` (`source=("git+$url.git")`)
- Test: `scripts/test_packaging_automation.py`, a new `DistributionRecipeTests` class inserted between `LocalPackagingTests` (ends line 49) and `class MacosInstallTests` (line 52)

**Interfaces:**
- Consumes: `packaging/PKGBUILD`'s `url`, `pkgver` and `source`, read by sourcing the file in `bash` (the way `LocalPackagingTests` reads `PKGBUILD.local`).
- Produces: the Python test `test_distribution_pkgbuild_pins_the_release_tag`, which Task 12 cites for GAP-C04. `scripts/prepare_release.py` is unchanged: it rewrites only the `^pkgver=` line, and the source expands from `pkgver`.

- [ ] **Step 1: Write the failing test**

In `scripts/test_packaging_automation.py`, insert after `LocalPackagingTests` (after line 49, before the two blank lines and `class MacosInstallTests`):

```python
class DistributionRecipeTests(unittest.TestCase):
    def test_distribution_pkgbuild_pins_the_release_tag(self):
        # Sourced, not grepped: the assertion is about what makepkg will fetch
        # once `url` and `pkgver` are expanded.
        result = subprocess.run(
            ['bash', '-c', '. ./PKGBUILD; printf "%s\\n" "$url" "$pkgver" "${source[@]}"'],
            cwd=REPO / 'packaging', capture_output=True, text=True, check=True,
        )
        url, pkgver, *sources = result.stdout.splitlines()
        self.assertEqual(sources, [f'git+{url}.git#tag=v{pkgver}'])
```

The file already imports `subprocess`, `unittest` and defines `REPO`; no import changes.

- [ ] **Step 2: Run it and see it fail**

Run: `cd /Users/davidlee/Projects/Sprite/.worktrees/close-open-findings && python3 -m unittest scripts.test_packaging_automation -v`
Expected: `test_distribution_pkgbuild_pins_the_release_tag ... FAIL` with
`AssertionError: Lists differ: ['git+https://github.com/HundredBillion/Sprite.git'] != ['git+https://github.com/HundredBillion/Sprite.git#tag=v0.2.4']`. Every other test in the module passes. Keep this output for the PR's Evidence.

- [ ] **Step 3: Pin the source**

In `packaging/PKGBUILD` replace line 30:

```bash
source=("git+$url.git")
```

with:

```bash
# The release tag, not the default branch: a build started before the tag is
# pushed fails instead of packaging whatever HEAD happens to be.
source=("git+$url.git#tag=v$pkgver")
```

`_root="$srcdir/Sprite"` is unchanged: makepkg names a git source's checkout after the repository, with or without a fragment.

- [ ] **Step 4: Run it and see it pass**

Run: `cd /Users/davidlee/Projects/Sprite/.worktrees/close-open-findings && python3 -m unittest scripts.test_packaging_automation scripts.test_release_automation -v`
Expected: `OK` (the release suite writes its own one-line PKGBUILD fixtures and does not read this file). `git -C /Users/davidlee/Projects/Sprite/.worktrees/close-open-findings tag --list v0.2.4` prints `v0.2.4`, so the pinned source exists for the current `pkgver`.

- [ ] **Step 5: Commit**

```bash
git -C /Users/davidlee/Projects/Sprite/.worktrees/close-open-findings add packaging/PKGBUILD scripts/test_packaging_automation.py
git -C /Users/davidlee/Projects/Sprite/.worktrees/close-open-findings commit -m "$(cat <<'EOF'
Build the release tag the PKGBUILD names, not git HEAD

GAP-C04 (#64): the distribution PKGBUILD labelled pkgver but fetched
the default branch. The source now carries #tag=v$pkgver, so a build
before the tag is pushed fails rather than packaging HEAD. The test
sources the recipe and asserts the expanded source.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
EOF
)"
```

---

---

### Task 2: The flaky tests wait on conditions, and CI jobs are bounded (R-W1..R-W3; GAP-C10, #57)

**Files:**
- Modify: `crates/sprite-term/tests/lifecycle.rs:408,468-471` (the exit wait in `retained_descendant_exits`, used by `natural_exit_deadline_is_not_renewed_by_continuous_output`)
- Modify: `crates/sprite-term/tests/support/mod.rs:374-398` (`GraphicsSession::probe`)
- Modify: `crates/sprite-term/tests/foreground.rs:94-105` (`a_running_program_is_reported_by_name`)
- Modify: `crates/sprite-app/src/terminal_view/submission_regressions.rs:8,87,96,117,348,362,375,472`
- Modify: `.github/workflows/ci.yml:27,37,163` (one `timeout-minutes` per job)
- Test: `scripts/test_ci_policy.py` (new `test_every_job_has_a_time_limit`)
- Evidence only, not committed: `$S/linux-loop.sh`, `$S/linux-loop.out`, `$S/loop-logs/`, where `S=/private/tmp/claude-502/-Users-davidlee/cccdda8c-2bbf-4621-b81d-1fb67fee46c7/scratchpad` (or the executing session's own scratchpad; use the same `S` in Task 12)

**Interfaces:**
- Consumes: sprite-term `tests/support` `watchdog()` (5 s, or `SPRITE_TEST_WATCHDOG_SECS`), `PROGRESS_CEILING` (120 s), `EventPump::next`/`next_with_timeout`; sprite-app `crate::test_blocking_wait::{pause, watchdog}`; `ci.yml`'s existing `SPRITE_TEST_WATCHDOG_SECS: "20"` on the arch job.
- Produces: no new helpers. One test-local constant, `NATURAL_EXIT_GIVE_UP`, in `lifecycle.rs`. `GIVE_UP_AFTER` stays `pub(crate)` in `crates/sprite-term/src/worker/closing.rs:14`: integration tests cannot see it, and widening a worker constant to the public API for one test is a larger change than mirroring it with a comment that names it. A drift either way is caught: a larger production value fails the test, and a smaller one only makes the wait more generous.
- Produces for Task 12: the loop counts (Step 8) and the measured CI durations (Step 6).

There is no failing-first run for Steps 1–5: they change how a test waits, not what it asserts, and the flake they remove does not reproduce on demand. Step 8's loops are their evidence. Step 6 (the CI policy test) is failing-first.

- [ ] **Step 1: The natural-exit wait is the give-up period plus the watchdog**

In `crates/sprite-term/tests/lifecycle.rs`, directly above `fn retained_descendant_exits(writes: bool) {` (line 408), add:

```rust
/// How long the worker drains output after a natural exit before it publishes
/// the exit anyway: the crate-private `GIVE_UP_AFTER` in
/// `src/worker/closing.rs`. Change both together.
const NATURAL_EXIT_GIVE_UP: std::time::Duration = std::time::Duration::from_secs(6);
```

Replace lines 468-471:

```rust
    events.expect_ready();
    // Natural exit permits six seconds of output draining without another event.
    // Allow scheduling overhead while still detecting a deadline renewed by output.
    match events.next_with_timeout(std::time::Duration::from_secs(10)) {
```

with:

```rust
    events.expect_ready();
    // A deadline that output renewed would never publish the exit, so the
    // wait is the drain period plus the watchdog's allowance for a slow runner.
    match events.next_with_timeout(NATURAL_EXIT_GIVE_UP + support::watchdog()) {
```

Locally the bound becomes 11 s (was 10 s); on Arch, where `SPRITE_TEST_WATCHDOG_SECS=20`, 26 s.

- [ ] **Step 2: `probe()` is bounded by progress, not by 20 s**

In `crates/sprite-term/tests/support/mod.rs` replace the whole of `probe` (lines 374-398, from `/// Asks what the pane is holding, without any image data.` to the closing `}` of the fn) with:

```rust
    /// Asks what the pane is holding, without any image data.
    ///
    /// Each event has the watchdog; `PROGRESS_CEILING` bounds a pane that keeps
    /// sending other events and never answers.
    pub fn probe(&mut self) -> GraphicsSnapshot {
        self.session
            .send(TerminalCommand::CaptureGraphics)
            .expect("request graphics");
        let started = Instant::now();
        loop {
            match self.events.next() {
                TerminalEvent::Graphics(graphics) => return (*graphics).clone(),
                TerminalEvent::Error(error) => panic!("graphics probe failed: {error}"),
                TerminalEvent::Ready
                | TerminalEvent::Exited(_)
                | TerminalEvent::SelectionCopied(_)
                | TerminalEvent::UnsafePaste(_)
                | TerminalEvent::Bell
                | TerminalEvent::TitleChanged(_)
                | TerminalEvent::History { .. }
                | TerminalEvent::HistoryFailed { .. }
                | TerminalEvent::WorkingDirectoryChanged(_)
                | TerminalEvent::Hyperlink { .. }
                | TerminalEvent::ClipboardWrite(_) => {}
            }
            assert!(
                started.elapsed() < PROGRESS_CEILING,
                "watchdog: no graphics answer arrived over {PROGRESS_CEILING:?}"
            );
        }
    }
```

`Instant` and `Duration` stay imported (`wait_within` and `watchdog` use them).

- [ ] **Step 3: The foreground child cannot finish first, and the test ends it**

In `crates/sprite-term/tests/foreground.rs` replace `a_running_program_is_reported_by_name` (lines 94-105, from `#[test]` to its closing `}`) with:

```rust
#[test]
fn a_running_program_is_reported_by_name() {
    let (mut session, _snapshots) = interactive_shell();

    // Far longer than any wait below, so the program cannot finish before it
    // is sampled; the test interrupts it instead of waiting it out.
    session
        .send(TerminalCommand::Input(b"sleep 600\n".to_vec()))
        .expect("send input");

    let state = wait_for(&session, "sleep to be in the foreground", |state| {
        state.program() == Some("sleep")
    });
    assert_eq!(state.program(), Some("sleep"));

    session
        .send(TerminalCommand::Input(vec![0x03]))
        .expect("interrupt the program");
    wait_for(&session, "the interrupted program to leave", |state| {
        *state == ForegroundState::Idle
    });
}
```

`0x03` reaches `sleep` as SIGINT because `interactive_shell` leaves the line discipline's signal keys on (the suspend test at line 149 relies on `0x1a` the same way). If an assertion fails before the interrupt, dropping `session` requests shutdown, whose hang-up and TERM/KILL escalation ends the job, as it already does for `owner_matches_only_the_live_foreground_job`'s `sleep 30`.

- [ ] **Step 4: sprite-app's fixed five-second waits read the watchdog**

In `crates/sprite-app/src/terminal_view/submission_regressions.rs`:

1. Line 8 (`submit_probe`), lines 87, 96 and 117 (`rejected_link_requests_recover_after_event_pressure`), and lines 348, 362 and 375 (`a_refused_reload_is_kept_whole_and_reverted_whole`): each
   ```rust
   .recv_timeout(std::time::Duration::from_secs(5))
   ```
   becomes
   ```rust
   .recv_timeout(crate::test_blocking_wait::watchdog())
   ```
   (Line 8's receiver is written `rx.recv_timeout(...)` on one line; keep that shape.) Lines 87/96/117 are not in the R-W1 list by line, but they are the same fixed wait in the same file, and R-W1 names "the fixed `recv_timeout(5 s)` waits" in `submission_regressions.rs` as a set.
2. Line 472 (`refused_resize_and_font_reload_retry_the_identical_layout`):
   ```rust
   let retry_deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
   ```
   becomes
   ```rust
   let retry_deadline = std::time::Instant::now() + crate::test_blocking_wait::watchdog();
   ```
   The loop body (`pause(5 ms)` then `cx.run_until_parked()`) is unchanged; `pause` is the sanctioned blocking site.

Then confirm no fixed wait remains:

Run: `grep -n "from_secs(5)" /Users/davidlee/Projects/Sprite/.worktrees/close-open-findings/crates/sprite-app/src/terminal_view/submission_regressions.rs`
Expected: no output (exit status 1). The remaining `pause(from_secs(1))` at line 541 is `saturated_ui_submission_and_reload_are_visible_refusals`' guard thread, not a wait on a condition, and is not a #57 test.

- [ ] **Step 5: Run the touched suites**

```bash
W=/Users/davidlee/Projects/Sprite/.worktrees/close-open-findings
env PATH=/Users/davidlee/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin:/Users/davidlee/.cargo/bin:/opt/homebrew/bin:/usr/bin:/bin cargo test --manifest-path "$W/Cargo.toml" -p sprite-term --locked --offline --test foreground --test lifecycle --test graphics_identity --test graphics_fixtures
env PATH=/Users/davidlee/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin:/Users/davidlee/.cargo/bin:/opt/homebrew/bin:/usr/bin:/bin cargo test --manifest-path "$W/Cargo.toml" -p sprite-app --locked --offline --lib -- terminal_view::submission_regressions
env PATH=/Users/davidlee/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin:/Users/davidlee/.cargo/bin:/opt/homebrew/bin:/usr/bin:/bin cargo clippy --manifest-path "$W/Cargo.toml" -p sprite-term -p sprite-app --all-targets --locked --offline -- -D warnings
```

Expected: every test passes (`foreground` 4, `lifecycle` 11, `graphics_identity` 8, `graphics_fixtures` 6 at `03b4929`, plus any a sibling task added; `submission_regressions` 5); clippy clean.

- [ ] **Step 6: Bound every CI job, test first**

Measured on 2026-10-12 from the ten most recent successful `ci.yml` runs (`gh run list -R HundredBillion/Sprite --workflow ci.yml --status success --limit 10`: 38323811720, 38322372720, 38167314514, 38167101497, 38139733757, 38110306163, 38024563753, 37884753506, 37878315524, 37414377306), job `startedAt` → `completedAt`:

| Job | Range | Longest | ≈2×, rounded up to 5 min |
|---|---|---|---|
| `arch linux` | 7.1–11.8 min | 11.8 min (38323811720, master push of #76) | **25** |
| `macos` | 10.3–17.8 min | 17.8 min (38139733757) | **40** |
| `release version automation` | 0.1–0.3 min | 0.3 min | **5** |

R-W2 requires the arch and macOS limits. The release job gets one too so the policy test can say "every job" and a future job cannot be added unbounded.

First the test. In `scripts/test_ci_policy.py` add `import re` after `import subprocess` (keep the imports in their current order: `from pathlib import Path`, `import re`, `import subprocess`, `import tempfile`, `import unittest`), and add this method to `CiPolicyTests` after `test_event_driven_rule_lives_in_clippy_config`:

```python
    def test_every_job_has_a_time_limit(self):
        # Without one a hung test holds a hosted runner for six hours.
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        jobs = workflow.split("\njobs:\n", 1)[1]
        headers = list(re.finditer(r"(?m)^  ([a-z][a-z0-9-]*):$", jobs))
        self.assertTrue(headers, "no jobs found under jobs:")
        ends = [header.start() for header in headers[1:]] + [len(jobs)]
        for header, end in zip(headers, ends):
            with self.subTest(job=header.group(1)):
                self.assertRegex(jobs[header.end():end], r"(?m)^    timeout-minutes: [1-9][0-9]*$")
```

Run: `cd /Users/davidlee/Projects/Sprite/.worktrees/close-open-findings && python3 -m unittest scripts.test_ci_policy -v`
Expected: `test_every_job_has_a_time_limit` FAILS with three subtest failures, `(job='release-automation')`, `(job='arch')`, `(job='macos')`, each `AssertionError: Regex didn't match: '(?m)^    timeout-minutes: [1-9][0-9]*$'`. The other four tests pass.

Then in `.github/workflows/ci.yml` add one line after each job's `runs-on:`:

- after line 27 (`    runs-on: ubuntu-latest` in `release-automation`):
  ```yaml
      timeout-minutes: 5
  ```
- after line 37 (`    runs-on: ubuntu-latest` in `arch`), before `    container:`:
  ```yaml
      # About twice the longest recent successful run (11.8 min), so a hang
      # fails in minutes instead of holding the runner for six hours.
      timeout-minutes: 25
  ```
- after line 163 (`    runs-on: macos-latest` in `macos`):
  ```yaml
      # About twice the longest recent successful run (17.8 min).
      timeout-minutes: 40
  ```

All three lines are indented four spaces (job level). `CiPolicyTests.setUpClass` slices the Forbidden-states script up to `"\n  macos:"`, which is unchanged.

Run: `cd /Users/davidlee/Projects/Sprite/.worktrees/close-open-findings && python3 -m unittest scripts.test_ci_policy -v`
Expected: `Ran 5 tests ... OK`.

- [ ] **Step 7: Commit**

```bash
W=/Users/davidlee/Projects/Sprite/.worktrees/close-open-findings
git -C "$W" add crates/sprite-term/tests/lifecycle.rs crates/sprite-term/tests/support/mod.rs crates/sprite-term/tests/foreground.rs crates/sprite-app/src/terminal_view/submission_regressions.rs .github/workflows/ci.yml scripts/test_ci_policy.py
git -C "$W" commit -m "$(cat <<'EOF'
Wait on conditions in the last flaky tests, and bound every CI job

GAP-C10 (#57): the natural-exit wait is the worker's six-second drain
period plus the watchdog; the graphics probe is bounded by progress;
the foreground test runs sleep 600 and interrupts it; sprite-app's
fixed five-second receives and retry deadline read the watchdog, so
Arch's SPRITE_TEST_WATCHDOG_SECS=20 reaches them. Every ci.yml job has
timeout-minutes at about twice its longest recent success (arch 25,
macos 40, release 5), asserted by the CI policy test.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
EOF
)"
```

- [ ] **Step 8: Loop each #57 test 100 times under load on Linux (R-W3, R-W4)**

Environment choice: the Arch container CI uses (`archlinux@sha256:714acd1e…`, `--platform linux/amd64`), not `rust:1.97.1-bookworm`. Last time bookworm was enough for sprite-term, but sprite-app was never built on Linux there (PR #76's Evidence says so), and its GPUI Linux dependencies are known only as `ci.yml`'s pacman list. Running in CI's own image avoids translating that list and is where #57 flaked. It needs network for pacman, rustup, `cargo fetch` and Zig's package fetch, the same stages CI runs with network; the tests themselves run `--offline`. Cargo and rustup state live in two named Docker volumes so a rerun skips the downloads; the host's `~/.cargo` is not mounted, so nothing is written there. `CARGO_TARGET_DIR` keeps Linux artefacts out of the macOS `target/debug`. `SPRITE_TEST_WATCHDOG_SECS=20` is what the Arch job sets.

Write the loop script to the scratchpad (not the repo):

```bash
S=/private/tmp/claude-502/-Users-davidlee/cccdda8c-2bbf-4621-b81d-1fb67fee46c7/scratchpad
mkdir -p "$S/loop-logs"
cat > "$S/linux-loop.sh" <<'SCRIPT'
#!/usr/bin/env bash
# Each #57 test, 100 runs under one `yes` per core, in CI's Arch image.
set -euo pipefail
cd /work

pacman -Syu --noconfirm --needed \
  base-devel git curl ncurses python less "zig=0.16.0" \
  wayland libxkbcommon libx11 libxcb libxkbcommon-x11 \
  mesa vulkan-icd-loader vulkan-headers \
  fontconfig freetype2 openssl alsa-lib > /dev/null
zig version | grep -qx 0.16.0
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
  | sh -s -- -y --profile minimal --default-toolchain 1.97.1 > /dev/null
export PATH="$HOME/.cargo/bin:$PATH"

cargo fetch --locked
(cd vendor/ghostty && \
  ZIG_GLOBAL_CACHE_DIR=/work/target/zig-global-cache \
  ZIG_LOCAL_CACHE_DIR=/work/target/zig-local-cache \
  zig build --fetch=all -Demit-lib-vt=true -Demit-xcframework=false -Dapp-runtime=none)

# ncurses ignores TERMINFO for root, so the generated entry is installed
# system-wide, as ci.yml does for its root container.
ZIG_GLOBAL_CACHE_DIR=/tmp/zig-global ZIG_LOCAL_CACHE_DIR=/tmp/zig-local \
  zig build-exe -lc -femit-bin=/tmp/gen-terminfo \
  --dep ghostty_terminfo \
  -Mroot=scripts/gen-terminfo.zig \
  -Mghostty_terminfo=vendor/ghostty/src/terminfo/ghostty.zig
/tmp/gen-terminfo > /tmp/ghostty.terminfo
tic -x /tmp/ghostty.terminfo
infocmp xterm-ghostty > /dev/null

cargo test -p sprite-term --locked --offline --no-run \
  --test graphics_fixtures --test lifecycle --test graphics_identity --test foreground
cargo test -p sprite-app --locked --offline --lib --no-run

for _core in $(seq "$(nproc)"); do yes > /dev/null & done
trap 'kill $(jobs -p) 2> /dev/null || true' EXIT
echo "load: $(nproc) x yes; SPRITE_TEST_WATCHDOG_SECS=${SPRITE_TEST_WATCHDOG_SECS}"

# A run that has not finished in 15 minutes is a hang (exit 124), counted
# apart from a failure. Logs are kept only for runs that did not pass.
loop() {
  local label=$1
  shift
  local failed=0 hung=0 status
  for i in $(seq 100); do
    status=0
    timeout 900 "$@" > "/scratch/loop-logs/$label-$i.log" 2>&1 || status=$?
    if [ "$status" -eq 0 ]; then
      rm "/scratch/loop-logs/$label-$i.log"
    elif [ "$status" -eq 124 ]; then
      hung=$((hung + 1))
    else
      failed=$((failed + 1))
    fi
  done
  echo "$label: $((100 - failed - hung))/100 passed, $failed failed, $hung hung"
}

term() { cargo test -p sprite-term --locked --offline --test "$1" -- --exact "$2"; }
app() { cargo test -p sprite-app --locked --offline --lib -- --exact "$1"; }

loop a_crop_selects_a_region_of_the_image term graphics_fixtures a_crop_selects_a_region_of_the_image
loop natural_exit_deadline_is_not_renewed_by_continuous_output term lifecycle natural_exit_deadline_is_not_renewed_by_continuous_output
loop storage_eviction_removes_an_image_from_the_projection term graphics_identity storage_eviction_removes_an_image_from_the_projection
loop a_running_program_is_reported_by_name term foreground a_running_program_is_reported_by_name
loop a_refused_reload_is_kept_whole_and_reverted_whole app terminal_view::submission_regressions::a_refused_reload_is_kept_whole_and_reverted_whole
loop refused_resize_and_font_reload_retry_the_identical_layout app terminal_view::submission_regressions::refused_resize_and_font_reload_retry_the_identical_layout
loop completed_removed_cleanup_is_pruned_before_shutdown app workspace::close_gate::tests::completed_removed_cleanup_is_pruned_before_shutdown
SCRIPT
```

Each `cargo test` call is one test run, so a loop of 100 is 100 fresh processes and 100 fresh Terminal Sessions. The names are the ones at `03b4929`; if a sibling task renamed one of them (Task Q edits `close_gate.rs`), use its new name here and in the PR body.

Run it (in the background; under amd64 emulation with every core loaded, expect one to three hours, most of it the first sprite-app build and `natural_exit…`'s six-second drain × 100):

```bash
W=/Users/davidlee/Projects/Sprite/.worktrees/close-open-findings
S=/private/tmp/claude-502/-Users-davidlee/cccdda8c-2bbf-4621-b81d-1fb67fee46c7/scratchpad
docker run --rm --platform linux/amd64 \
  -v "$W":/work -v "$S":/scratch \
  -v sprite-arch-cargo:/root/.cargo -v sprite-arch-rustup:/root/.rustup \
  -e CARGO_TARGET_DIR=/work/target/linux-amd64-arch \
  -e SPRITE_TEST_WATCHDOG_SECS=20 \
  archlinux@sha256:714acd1eef9ae997d95691b1c5220ada0076185b77857c1813f02de0fa83cf7b \
  bash /scratch/linux-loop.sh > "$S/linux-loop.out" 2>&1
tail -n 9 "$S/linux-loop.out"
```

Expected: the `load:` line, then seven lines of the form `<test>: 100/100 passed, 0 failed, 0 hung`. `ls "$S/loop-logs"` is empty.

If a line shows a failure or a hang:
- Read the kept log (`$S/loop-logs/<test>-<i>.log`), then use `dmi-superpowers:systematic-debugging`. A hang in `completed_removed_cleanup_is_pruned_before_shutdown` is R-W3's case and is fixed in this PR, with a regression test, before Task 12.
- If `a_refused_reload_is_kept_whole_and_reverted_whole` or `refused_resize_and_font_reload_retry_the_identical_layout` fails at `snapshot deadline` / `terminal output deadline`, the cause is their `wait_for_text` closures (lines 317-327 and 459-470), which hold every snapshot to one overall `watchdog()` while the child streams output. Replace each closure body with a per-snapshot wait; the children stop writing once they print the awaited text and block on `read`, so silence after it is what fails the wait:
  ```rust
  let wait_for_text = |text: &str| loop {
      let bundle = rx
          .recv_timeout(crate::test_blocking_wait::watchdog())
          .expect("snapshot deadline");
      if bundle.pane.rows.iter().any(|row| row.text.contains(text)) {
          break bundle;
      }
  };
  ```
  (use `.expect("terminal output deadline")` in the second), commit it as `Wait per snapshot, not per stream, in the reload regressions`, and rerun the loop.
- If the container route itself fails (pacman mirror, emulation, Zig fetch), record the exact failing command and its output, then rerun on `rust:1.97.1-bookworm` with `apt-get install -y -qq xz-utils python3 ncurses-bin pkg-config libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev libx11-dev libx11-xcb-dev libxcb1-dev libvulkan-dev libfontconfig-dev libfreetype-dev libssl-dev libasound2-dev` in place of the pacman line and Zig 0.16.0 from `https://ziglang.org/download/0.16.0/zig-x86_64-linux-0.16.0.tar.xz` (Task 14's recipe in `docs/TSPs/10-12-2026-findings-ledger-and-admission.md`). The Debian package list is a translation of ci.yml's pacman list and has not been run; if it does not link sprite-app, loop the four sprite-term tests there and say in the PR body that the three sprite-app tests were looped only by the hosted runs in Task 12.

Keep `$S/linux-loop.out` for Task 12.

---

---

### Task 3: Kitty placeholder cells inherit from their left neighbour (R-K; GAP-C16)

Today `image_cells` drops every placeholder cell that does not spell out both a row and a column diacritic (`coordinates`, `placeholder.rs:311-321`, returns `None` on the second `?`). It drops every cell whose foreground is a 256-colour palette index (`rgb_id`, `:409-414`). It ignores the third diacritic. Kitty and Ghostty fill a missing mark from the placeholder cell to the left. The reference is `IncompletePlacement` in `vendor/ghostty/src/terminal/kitty/graphics_unicode.zig:405-530`, with its tests at `:927-1180`.

**The rule, as Ghostty does it.** Ghostty's `PlacementIterator.next` (`:36-97`) and `IncompletePlacement.canAppend` (`:497-504`) define the rule:

- A **run** is a stretch of placeholder cells on one row. Ghostty runs never cross rows. Any non-placeholder cell ends the current run, and so does a wide character's spacer cell, which has codepoint 0.
- Each cell is decoded on its own (`init`, `:441-488`):
  - The low 24 bits of the image ID come from `colorToId(fg)`.
  - The placement ID comes from `colorToId(underline)`, with 0 meaning "none".
  - The first diacritic is the row, the second the column and the third the image ID's high byte. Each is optional, and a mark that is not in the diacritic table counts as absent. A third-mark index above 255 also counts as absent: `std.math.cast(u8, …) orelse null`. Marks after the third are ignored.
- The first cell of a run takes its own row and column, and a missing one becomes 0 (`:83-85`). Its high byte stays as decoded. An absent high byte reads as 0 when the ID is assembled (`complete`, `:510-525`).
- A later cell continues the run only if all of these hold (`canAppend`):
  - its image-ID low bits equal the run's;
  - its placement ID equals the run's;
  - its row is absent, or equals the run's row;
  - its column is absent, or equals the run's start column + the run's width so far;
  - its high byte is absent, or equals the run's high byte. This is an `Option` comparison, so a cell that says `Some(0)` does not continue a run whose high byte is `None`.
- A continuing cell takes the run's row, the column `start + width`, the run's image ID (low bits plus high byte) and the run's placement ID. Then the width grows by one.
- On a mismatch, the cell starts a new run, as a first cell does.
- `colorToId` (`:528-539`) maps `.none` to 0, `.palette => |v| v` (the index itself is the ID, so palette 42 is image 42) and `.rgb` to `r<<16 | g<<8 | b`. This is verified against Ghostty's tests "specifying image id as palette" (`:1102`, ID 42) and "specifying image id with high bits" (`:1128`, palette 42 plus mark `U+030E`, index 2, giving `2<<24 | 42 = 33554474`). The same function decodes the underline. "Specifying placement id as palette" (`:1154`) gives placement 21 from palette underline 21, so this task accepts a palette underline too.

**How the rule maps onto Sprite's cells.** Sprite sees laid-out `PositionedCell`s, not Ghostty pins. `lay_out_row` (`crates/sprite-app/src/grid.rs:85-112`) keeps every narrow and wide cell, blanks included, and skips spacer cells. So the run ends:

- at a non-placeholder cell, and
- whenever the next placeholder is not at the grid column right after the previous one. Only a wide placeholder's skipped spacer can cause that gap.

Runs are formed before the placement lookup, so a cell whose placement is unknown still carries the run on, as in Ghostty.

**Kept out of the paint-time work.** The GAP-C08 task changes how tiles are positioned in `render.rs`. This task touches only decoding inside `placeholder.rs`, and these items stay byte-for-byte unchanged:

- `pub(super) fn image_cells<'a>(rows: &[std::sync::Arc<Vec<PositionedCell>>], placements: &'a [Placement]) -> Vec<ImageCell<'a>>`
- `pub(super) struct ImageCell<'a> { pub placement: &'a Placement, pub column: u16, pub row: usize, pub image_column: u32, pub image_row: u32 }`
- `pub(super) fn fit_image(image_width: u32, image_height: u32, box_width: f32, box_height: f32) -> Option<ImageFit>`, `pub(super) struct ImageFit`
- `pub(super) fn tile_geometry(cell: &ImageCell<'_>, fit: &ImageFit, grid_origin: gpui::Point<Pixels>, cell_width: Pixels, cell_height: Pixels, scale: f32) -> TileGeometry`, `pub(super) struct TileGeometry`

`pub(super) fn coordinates` is removed. `grep -rn "coordinates(" crates` shows its only callers are `placeholder.rs:436` and its own test (`:530-532`). The private `rgb_id` is replaced.

**Files:**
- Modify: `crates/sprite-app/src/terminal_view/placeholder.rs`:
  - `:311-321`: `coordinates` becomes `Marks` plus `placeholder_marks`.
  - `:409-414`: `rgb_id` becomes `color_id`, and a new `Run` is added.
  - `:416-461`: the body of `image_cells`.
  - Tests at `:463-677`: new helpers after `row` (`:498-526`), `row` refactored onto them, the decode test at `:528-533` updated, and 15 new tests.

**Interfaces:**
- Consumes: `crate::grid::{PositionedCell, lay_out_row}`, `sprite_term::{Placement, SnapshotColor::{Default, Palette(u8), Rgb(Rgb)}}`.
- Produces: no new cross-module items. The unchanged signatures are listed above. The new items are private to `placeholder.rs`:
  - `struct Marks { row: Option<u32>, column: Option<u32>, image_high: Option<u8> }`
  - `fn placeholder_marks(text: &str) -> Option<Marks>`
  - `fn color_id(color: SnapshotColor) -> u32`
  - `struct Run`, with `Run::start`, `Run::continues`, `Run::extend`, `Run::image_column` and `Run::image_id`

- [ ] **Step 1: Write the failing tests**

In `crates/sprite-app/src/terminal_view/placeholder.rs`, replace the test helper `row` (`:498-526`) with these helpers. `row` keeps its signature and output:

```rust
    use std::sync::Arc;

    // Diacritic indexes used below: U+0305 is 0, U+030D is 1, U+030E is 2,
    // U+0310 is 3.
    const BARE: &str = "\u{10eeee}";

    fn style(foreground: SnapshotColor, underline_color: SnapshotColor) -> CellStyle {
        CellStyle {
            foreground,
            background: SnapshotColor::Default,
            underline_color,
            bold: false,
            italic: false,
            faint: false,
            blink: false,
            inverse: false,
            invisible: false,
            strikethrough: false,
            overline: false,
            underline: UnderlineStyle::None,
        }
    }

    fn render_cell(
        text: &str,
        width: CellWidth,
        foreground: SnapshotColor,
        underline_color: SnapshotColor,
    ) -> RenderCell {
        RenderCell {
            text: text.into(),
            width,
            selected: false,
            style: style(foreground, underline_color),
        }
    }

    /// A narrow cell with no placement colour.
    fn narrow(text: &str, foreground: SnapshotColor) -> RenderCell {
        render_cell(text, CellWidth::Narrow, foreground, SnapshotColor::Default)
    }

    fn laid_out(cells: Vec<RenderCell>) -> Arc<Vec<PositionedCell>> {
        lay_out_row(&RenderRow {
            cells,
            wrapped: false,
        })
        .into()
    }

    /// Each decoded cell as (grid column, image row, image column, image ID,
    /// placement ID).
    fn decoded(
        rows: &[Arc<Vec<PositionedCell>>],
        placements: &[Placement],
    ) -> Vec<(u16, u32, u32, u32, u32)> {
        image_cells(rows, placements)
            .iter()
            .map(|cell| {
                (
                    cell.column,
                    cell.image_row,
                    cell.image_column,
                    cell.placement.image,
                    cell.placement.placement,
                )
            })
            .collect()
    }

    fn row(fg: u32, placement_id: Option<u32>) -> RenderRow {
        let underline = placement_id.map(color).unwrap_or(SnapshotColor::Default);
        let cell = |text: &str| render_cell(text, CellWidth::Narrow, color(fg), underline);
        RenderRow {
            cells: vec![
                cell("text"),
                cell("\u{10eeee}\u{0305}\u{0305}"),
                cell("\u{10eeee}\u{0305}\u{030d}"),
            ],
            wrapped: false,
        }
    }
```

Then add these tests after `explicit_placement_requires_its_exact_id` (`:545-556`):

```rust
    #[test]
    fn bare_placeholders_start_at_the_images_corner_and_run_on() {
        let fg = color(1001);
        let cells = laid_out(vec![narrow(BARE, fg), narrow(BARE, fg), narrow(BARE, fg)]);
        assert_eq!(
            decoded(&[cells], &[placement(1001, 3)]),
            vec![
                (0, 0, 0, 1001, 3),
                (1, 0, 1, 1001, 3),
                (2, 0, 2, 1001, 3)
            ]
        );
    }

    #[test]
    fn a_placeholder_without_diacritics_continues_its_left_neighbour() {
        let fg = color(1001);
        let cells = laid_out(vec![
            narrow("\u{10eeee}\u{030d}\u{0310}", fg),
            narrow(BARE, fg),
            narrow(BARE, fg),
        ]);
        assert_eq!(
            decoded(&[cells], &[placement(1001, 3)]),
            vec![
                (0, 1, 3, 1001, 3),
                (1, 1, 4, 1001, 3),
                (2, 1, 5, 1001, 3)
            ]
        );
    }

    #[test]
    fn a_placeholder_with_only_a_row_diacritic_continues_its_left_neighbour() {
        let fg = color(1001);
        let cells = laid_out(vec![
            narrow("\u{10eeee}\u{030d}\u{030e}", fg),
            narrow("\u{10eeee}\u{030d}", fg),
            narrow("\u{10eeee}\u{030d}", fg),
        ]);
        assert_eq!(
            decoded(&[cells], &[placement(1001, 3)]),
            vec![
                (0, 1, 2, 1001, 3),
                (1, 1, 3, 1001, 3),
                (2, 1, 4, 1001, 3)
            ]
        );
    }

    #[test]
    fn a_row_diacritic_that_differs_from_its_neighbours_starts_a_new_run() {
        let fg = color(1001);
        let cells = laid_out(vec![
            narrow("\u{10eeee}\u{0305}\u{030e}", fg),
            narrow("\u{10eeee}\u{030d}", fg),
        ]);
        assert_eq!(
            decoded(&[cells], &[placement(1001, 3)]),
            vec![(0, 0, 2, 1001, 3), (1, 1, 0, 1001, 3)]
        );
    }

    #[test]
    fn explicit_neighbours_with_contiguous_columns_form_one_run() {
        let fg = color(1001);
        let cells = laid_out(vec![
            narrow("\u{10eeee}\u{0305}\u{0305}", fg),
            narrow("\u{10eeee}\u{0305}\u{030d}", fg),
            narrow("\u{10eeee}\u{0305}\u{030e}", fg),
            narrow(BARE, fg),
        ]);
        assert_eq!(
            decoded(&[cells], &[placement(1001, 3)]),
            vec![
                (0, 0, 0, 1001, 3),
                (1, 0, 1, 1001, 3),
                (2, 0, 2, 1001, 3),
                (3, 0, 3, 1001, 3)
            ]
        );
    }

    /// The second cell jumps from column 0 to column 2, so it starts its own
    /// run; the bare cell after it continues that run, not the first.
    #[test]
    fn a_column_jump_starts_a_new_run() {
        let fg = color(1001);
        let cells = laid_out(vec![
            narrow("\u{10eeee}\u{0305}\u{0305}", fg),
            narrow("\u{10eeee}\u{0305}\u{030e}", fg),
            narrow(BARE, fg),
        ]);
        assert_eq!(
            decoded(&[cells], &[placement(1001, 3)]),
            vec![
                (0, 0, 0, 1001, 3),
                (1, 0, 2, 1001, 3),
                (2, 0, 3, 1001, 3)
            ]
        );
    }

    #[test]
    fn text_between_placeholders_ends_the_run() {
        let fg = color(1001);
        let cells = laid_out(vec![
            narrow("\u{10eeee}\u{0305}\u{030e}", fg),
            narrow("A", fg),
            narrow(BARE, fg),
        ]);
        assert_eq!(
            decoded(&[cells], &[placement(1001, 3)]),
            vec![(0, 0, 2, 1001, 3), (2, 0, 0, 1001, 3)]
        );
    }

    #[test]
    fn a_run_can_start_after_text() {
        let fg = color(1001);
        let cells = laid_out(vec![
            narrow("A", fg),
            narrow("B", fg),
            narrow(BARE, fg),
            narrow(BARE, fg),
        ]);
        assert_eq!(
            decoded(&[cells], &[placement(1001, 3)]),
            vec![(2, 0, 0, 1001, 3), (3, 0, 1, 1001, 3)]
        );
    }

    /// A wide cell's second column is a spacer, which is not a placeholder,
    /// so the cell after it starts afresh.
    #[test]
    fn a_wide_placeholder_ends_its_run_at_its_spacer() {
        let fg = color(1001);
        let cells = laid_out(vec![
            render_cell(
                "\u{10eeee}\u{0305}\u{0305}",
                CellWidth::Wide,
                fg,
                SnapshotColor::Default,
            ),
            render_cell("", CellWidth::SpacerTail, fg, SnapshotColor::Default),
            narrow(BARE, fg),
        ]);
        assert_eq!(
            decoded(&[cells], &[placement(1001, 3)]),
            vec![(0, 0, 0, 1001, 3), (2, 0, 0, 1001, 3)]
        );
    }

    #[test]
    fn a_placeholder_for_a_different_image_starts_a_new_run() {
        let cells = laid_out(vec![
            narrow("\u{10eeee}\u{030d}\u{030e}", color(1001)),
            narrow(BARE, color(2002)),
        ]);
        assert_eq!(
            decoded(&[cells], &[placement(1001, 3), placement(2002, 5)]),
            vec![(0, 1, 2, 1001, 3), (1, 0, 0, 2002, 5)]
        );
    }

    #[test]
    fn a_placeholder_for_a_different_placement_starts_a_new_run() {
        let fg = color(1001);
        let cells = laid_out(vec![
            render_cell("\u{10eeee}\u{030d}\u{030e}", CellWidth::Narrow, fg, color(3)),
            render_cell(BARE, CellWidth::Narrow, fg, color(4)),
        ]);
        assert_eq!(
            decoded(&[cells], &[placement(1001, 3), placement(1001, 4)]),
            vec![(0, 1, 2, 1001, 3), (1, 0, 0, 1001, 4)]
        );
    }

    #[test]
    fn a_palette_foreground_names_the_image() {
        let cells = laid_out(vec![narrow(
            "\u{10eeee}\u{0305}\u{0305}",
            SnapshotColor::Palette(42),
        )]);
        assert_eq!(
            decoded(&[cells], &[placement(42, 7)]),
            vec![(0, 0, 0, 42, 7)]
        );
    }

    /// Placement 7 is listed last, so it is the image's default; only a
    /// decoded underline reaches placement 21.
    #[test]
    fn a_palette_underline_names_the_placement() {
        let cells = laid_out(vec![render_cell(
            "\u{10eeee}\u{0305}\u{0305}",
            CellWidth::Narrow,
            SnapshotColor::Palette(42),
            SnapshotColor::Palette(21),
        )]);
        assert_eq!(
            decoded(&[cells], &[placement(42, 21), placement(42, 7)]),
            vec![(0, 0, 0, 42, 21)]
        );
    }

    #[test]
    fn the_third_diacritic_is_the_image_ids_high_byte() {
        let cells = laid_out(vec![narrow(
            "\u{10eeee}\u{0305}\u{0305}\u{030e}",
            SnapshotColor::Palette(42),
        )]);
        assert_eq!(
            decoded(&[cells], &[placement(42, 7), placement(33_554_474, 9)]),
            vec![(0, 0, 0, 33_554_474, 9)]
        );
    }

    /// The bare cell and the column-only cell take the first cell's high
    /// byte; the last cell names high byte 0, which differs, so it starts a
    /// run of its own and names the plain image.
    #[test]
    fn a_placeholder_inherits_its_neighbours_high_byte() {
        const HIGH: u32 = (2 << 24) | 1001;
        let fg = color(1001);
        let cells = laid_out(vec![
            narrow("\u{10eeee}\u{0305}\u{0305}\u{030e}", fg),
            narrow(BARE, fg),
            narrow("\u{10eeee}\u{0305}\u{030e}", fg),
            narrow("\u{10eeee}\u{0305}\u{0310}\u{0305}", fg),
        ]);
        assert_eq!(
            decoded(&[cells], &[placement(1001, 3), placement(HIGH, 9)]),
            vec![
                (0, 0, 0, HIGH, 9),
                (1, 0, 1, HIGH, 9),
                (2, 0, 2, HIGH, 9),
                (3, 0, 3, 1001, 3)
            ]
        );
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `env PATH=/Users/davidlee/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin:/Users/davidlee/.cargo/bin:/opt/homebrew/bin:/usr/bin:/bin cargo test -p sprite-app --locked --offline placeholder`

Expected: the 15 new tests FAIL on `assert_eq!`, each with a `left` vector shorter than `right`. The reasons:

- Cells without both coordinates are dropped, so for example `bare_placeholders_start_at_the_images_corner_and_run_on` gets `left: []`.
- Palette cells are dropped, so `a_palette_foreground_names_the_image` gets `left: []`.
- The high byte is ignored. `the_third_diacritic_is_the_image_ids_high_byte` gets `left: []` because its palette cell is dropped. In `a_placeholder_inherits_its_neighbours_high_byte`, `left` has 3 entries, all naming image 1001: the bare cell is dropped, and the high byte is never applied.

Other tests the `placeholder` filter matches outside this module are unaffected. The existing 6 placeholder tests still pass, including the refactored `row` callers.

- [ ] **Step 3: Implement**

Replace `coordinates` (`:311-321`) with:

```rust
const PLACEHOLDER: char = '\u{10eeee}';

/// What one placeholder cell's diacritics say. Each mark is optional: one that
/// is missing, or is not in the table, is taken from the cell to the left.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Marks {
    row: Option<u32>,
    column: Option<u32>,
    /// The top eight bits of the image ID.
    image_high: Option<u8>,
}

fn diacritic_index(mark: char) -> Option<u32> {
    DIACRITICS
        .iter()
        .position(|candidate| *candidate == mark)
        .and_then(|index| u32::try_from(index).ok())
}

/// Reads a placeholder cell's row, column and image-ID high byte marks, in
/// that order, or `None` when the cell is not a placeholder. Marks after the
/// third are ignored, and a high-byte mark whose index does not fit in a byte
/// counts as missing.
fn placeholder_marks(text: &str) -> Option<Marks> {
    let mut chars = text.chars();
    if chars.next()? != PLACEHOLDER {
        return None;
    }
    let row = chars.next().and_then(diacritic_index);
    let column = chars.next().and_then(diacritic_index);
    let image_high = chars
        .next()
        .and_then(diacritic_index)
        .and_then(|high| u8::try_from(high).ok());
    Some(Marks {
        row,
        column,
        image_high,
    })
}
```

Replace `rgb_id` (`:409-414`) with:

```rust
/// The ID a placeholder colour carries: an RGB colour's 24 bits, or a
/// 256-colour palette index as it is. The default colour carries 0, which as a
/// placement ID means "none named".
fn color_id(color: SnapshotColor) -> u32 {
    match color {
        SnapshotColor::Default => 0,
        SnapshotColor::Palette(index) => u32::from(index),
        SnapshotColor::Rgb(color) => {
            (u32::from(color.r) << 16) | (u32::from(color.g) << 8) | u32::from(color.b)
        }
    }
}

/// A stretch of placeholder cells on one row, each continuing the one before
/// it: Kitty lets a cell leave out marks its left neighbour implies.
struct Run {
    /// The image ID's low 24 bits, from the foreground colour.
    image_low: u32,
    image_high: Option<u8>,
    /// From the underline colour; 0 when none is named.
    placement: u32,
    row: u32,
    /// The image column of the run's first cell.
    column: u32,
    /// How many cells the run holds so far.
    width: u32,
    /// The grid column a cell must sit in to continue the run.
    next_grid_column: u32,
}

impl Run {
    fn start(grid_column: u32, image_low: u32, placement: u32, marks: Marks) -> Self {
        Self {
            image_low,
            image_high: marks.image_high,
            placement,
            row: marks.row.unwrap_or(0),
            column: marks.column.unwrap_or(0),
            width: 1,
            next_grid_column: grid_column + 1,
        }
    }

    /// Whether a cell continues this run: it must sit right after the run's
    /// last cell, name the same image and placement, and give no mark that
    /// disagrees with where the run has reached.
    fn continues(&self, grid_column: u32, image_low: u32, placement: u32, marks: Marks) -> bool {
        grid_column == self.next_grid_column
            && image_low == self.image_low
            && placement == self.placement
            && marks.row.is_none_or(|row| row == self.row)
            && marks
                .column
                .is_none_or(|column| column == self.column + self.width)
            && marks
                .image_high
                .is_none_or(|high| Some(high) == self.image_high)
    }

    fn extend(&mut self) {
        self.width += 1;
        self.next_grid_column += 1;
    }

    /// The image column of the run's newest cell.
    fn image_column(&self) -> u32 {
        self.column + self.width - 1
    }

    fn image_id(&self) -> u32 {
        self.image_low | (u32::from(self.image_high.unwrap_or(0)) << 24)
    }
}
```

Replace the per-row loop of `image_cells` (`:433-460`, from `let mut result` to the final `result`). The signature and the `exact` and `defaults` maps above it stay as they are:

```rust
    let mut result = Vec::new();
    for (row, cells) in rows.iter().enumerate() {
        // A run never crosses a row.
        let mut run: Option<Run> = None;
        for cell in cells.iter() {
            let Some(marks) = placeholder_marks(&cell.text) else {
                run = None;
                continue;
            };
            let grid_column = u32::from(cell.column);
            let image_low = color_id(cell.style.foreground);
            let placement_id = color_id(cell.style.underline_color);
            let current = match run
                .take()
                .filter(|previous| previous.continues(grid_column, image_low, placement_id, marks))
            {
                Some(mut continued) => {
                    continued.extend();
                    continued
                }
                None => Run::start(grid_column, image_low, placement_id, marks),
            };
            let image_id = current.image_id();
            let placement = exact
                .get(&(image_id, current.placement))
                .copied()
                .or_else(|| {
                    (current.placement == 0)
                        .then(|| defaults.get(&image_id).copied())
                        .flatten()
                });
            if let Some(placement) = placement {
                result.push(ImageCell {
                    placement,
                    column: cell.column,
                    row,
                    image_column: current.image_column(),
                    image_row: current.row,
                });
            }
            run = Some(current);
        }
    }
    result
```

Update the existing decode test (`:528-533`), which tested the removed `coordinates`, to the inheritance rule. A row-only cell now decodes with its column left open instead of being rejected:

```rust
    #[test]
    fn decodes_whichever_placeholder_diacritics_are_present() {
        let marks = |row, column, image_high| {
            Some(Marks {
                row,
                column,
                image_high,
            })
        };
        assert_eq!(
            placeholder_marks("\u{10eeee}\u{0305}\u{030d}"),
            marks(Some(0), Some(1), None)
        );
        assert_eq!(placeholder_marks("ordinary text"), None);
        assert_eq!(
            placeholder_marks("\u{10eeee}\u{0305}"),
            marks(Some(0), None, None)
        );
        assert_eq!(placeholder_marks("\u{10eeee}"), Some(Marks::default()));
        assert_eq!(
            placeholder_marks("\u{10eeee}\u{0305}\u{0305}\u{030e}"),
            marks(Some(0), Some(0), Some(2))
        );
        // A mark outside the table counts as missing, not as the end.
        assert_eq!(
            placeholder_marks("\u{10eeee}x\u{030d}"),
            marks(None, Some(1), None)
        );
        // Index 296 does not fit the high byte.
        assert_eq!(
            placeholder_marks("\u{10eeee}\u{0305}\u{0305}\u{1d244}"),
            marks(Some(0), Some(0), None)
        );
    }
```

- [ ] **Step 4: Run the tests, clippy and the crate suite**

Run: `env PATH=/Users/davidlee/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin:/Users/davidlee/.cargo/bin:/opt/homebrew/bin:/usr/bin:/bin cargo test -p sprite-app --locked --offline placeholder`
Expected: all 21 placeholder tests pass: 15 new, the 5 untouched ones, and `decodes_whichever_placeholder_diacritics_are_present`.

Run: `env PATH=/Users/davidlee/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin:/Users/davidlee/.cargo/bin:/opt/homebrew/bin:/usr/bin:/bin cargo clippy -p sprite-app --all-targets --locked --offline -- -D warnings`
Expected: no warnings. `color_id`'s match lists all three `SnapshotColor` variants, and `run.take().filter(..)` matches `Some`/`None` with no wildcard arm.

Run: `env PATH=/Users/davidlee/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin:/Users/davidlee/.cargo/bin:/opt/homebrew/bin:/usr/bin:/bin cargo test -p sprite-app --locked --offline`
Expected: all pass. `render.rs:220` still compiles against the unchanged `image_cells` and `ImageCell`.

- [ ] **Step 5: Commit**

```bash
git -C /Users/davidlee/Projects/Sprite/.worktrees/close-open-findings add crates/sprite-app/src/terminal_view/placeholder.rs
git -C /Users/davidlee/Projects/Sprite/.worktrees/close-open-findings commit -m "$(cat <<'EOF'
Let Kitty placeholder cells inherit marks from their left neighbour

A placeholder cell may now omit its row, column or image-ID high byte
and continue the run to its left, as Kitty and Ghostty do. Palette
colours are accepted as image and placement IDs, and the third
diacritic supplies the image ID's high byte.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 4: Two sentinel pins detect a rotated-out anchor (R-S; GAP-C06, #66)

Today a gesture's press is one `TrackedGridRef` (`PinnedAnchor`, `crates/sprite-term/src/input/mouse.rs:232-235`). `apply_selection` (`:276-329`) treats the anchor as lost only when that pin's snapshot is `None` or the other screen is active (`:296-311`). That catches eviction, meaning pruned scrollback pages. It misses rotation.

**What libghostty does (vendor/ghostty @ ab0b9da, checked).** A screen keeps no history when its `max_scrollback` is 0 (`Screen.zig:313`, `no_scrollback = opts.max_scrollback == 0`). That is true for:
- a pane with `scrollback_bytes = 0` (`crates/sprite-term/src/worker/start.rs:34` passes it straight through as `max_scrollback`);
- the alternate screen, always (`Terminal.zig:4290-4293`: `.alternate => 0`).

A line feed on the bottom row of a scroll region goes through `Terminal.index` (`Terminal.zig:2236-2261`). It calls `cursorScrollAbove`, which grows scrollback, only when the region starts at row 0 and the screen keeps history (`:2238-2245`). In every other case, a region with a top margin or a full-screen region on a screen without history, it calls `Screen.cursorScrollRegionUp` (`Screen.zig:1213-1303`). That function rotates the region's rows in place. Its tracked-pin fixup (`:1287-1293`) skips pins above the region (`p.y < pin.y - limit`, which is the region's top row) and below it. For each pin inside the region, it runs `if (p.y == 0) p.x = 0 else p.y -= 1`. So:

- **Full-screen region, no history.** The region starts at page row 0. The pin on the erased row 0 stays on row 0 and now names whatever rotated into it. Every pin below it moves up one row.
- **Region with top margin `t > 0`.** The pin on the erased row `t` moves to `t - 1`, outside the region. A pin already on `t - 1` is skipped, so it stays put. Pins below `t` inside the region move up one row.

The PRD names `PageList.eraseRow` (`PageList.zig:4427-4456`, which shifts only pins with `p.y > pn.y` and leaves the erased-row pin in place). At ab0b9da, `index` reaches it only through `cursorDownScroll` (`Screen.zig:893-920`) for a one-row screen. The path a line feed actually takes is `cursorScrollRegionUp`, as above. `eraseRowBounded`, the slow path for a region that spans pages (`PageList.zig:4591-4603`, `:4638-4646`), uses the same `p.y == 0` rule.

**The design.** `PinnedAnchor` gains two sentinels, tracked at column 0 in screen coordinates (scrollback included) when the press is pinned. There is no logical-line walking.
- `above` sits on the row directly above the anchor's row. It is `None` when the anchor is on screen row 0.
- `below` sits on the row directly below. It is `None` when the anchor is on the last screen row.

`apply_selection` treats a tracked anchor as lost when any of these holds, and then clears the selection exactly as today:
- the other screen is active;
- the anchor's snapshot or its `point(PointSpace::Screen)` is `None`;
- a sentinel that was placed now has a `point(PointSpace::Screen)` of `None`;
- the anchor's screen row equals the screen row of a sentinel that was placed.

A `point` of `None` is the documented "no meaningful value" result, the same as a `None` snapshot. Screen space covers every row, so `None` never means "unrepresentable" here.

Each case below was worked through against the fixup above.

| Case | Anchor | Sentinel | Result |
| --- | --- | --- | --- |
| Full screen, no history, anchor on row 0 | Row 0 is erased; the anchor stays at 0 | `below` moves from 1 to 0 | They meet exactly when the anchor's row is erased |
| Region with top margin `t > 0`, anchor on row `t` | Row `t` is erased; the anchor moves from `t` to `t - 1` | `above` sits on `t - 1`, outside the region, and does not move | They meet exactly then |
| Region with top margin `t > 0`, anchor on row `t + 1` | Moves to `t` | `above`'s own row `t` is erased and it moves to `t - 1`; `below` moves up one row too | Gap stays 1; nothing is cleared, which is correct |
| Full screen, no history, anchor on row 1 | Moves to 0, still naming its own content | `above` sits on row 0. Its row is the one erased, and the `p.y == 0` rule keeps it there | They meet one scroll early (see the limits) |
| Anchor pressed on the last row | Moves up as the screen scrolls | Only `above` exists | Cleared one scroll early, the same way |

The coordinator's note says that when a sentinel's own row is erased, the gap stays 1. That is true for `below`, and for `above` inside a region with a top margin. It is not true for `above` on page row 0 of a full-screen region, because of the `p.y == 0` rule. That is the "one scroll early" row in the table, and it is accepted.

**libghostty-vt 0.2.1 API used (all checked in `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/libghostty-vt-0.2.1/src`):**
- `Terminal::track_grid_ref(&self, Point) -> Result<TrackedGridRef>` (`terminal.rs:402`).
- `Terminal::total_rows(&self) -> Result<usize>`: the active screen's rows, scrollback included (`terminal.rs:638`).
- `Terminal::active_screen(&self) -> Result<Screen>` (`terminal.rs:602`).
- `TrackedGridRef::snapshot<'t>(&self, &'t Terminal) -> Result<Option<GridRef<'t>>>` (`screen.rs:193`).
- `TrackedGridRef::point(&self, PointSpace) -> Result<Option<PointCoordinate>>` (`screen.rs:231`).
- `Point::{Active, Viewport, Screen, History}(PointCoordinate { x: u16, y: u32 })` (`terminal.rs:752-763,820-825`).
- `PointSpace::{Active, Viewport, Screen, History}` (`terminal.rs:796-806`).
- Every fallible call returns `libghostty_vt::Error`, which is re-exported at the crate root (`lib.rs:115-117`).

**Known limits (accepted under the PRD rule that a cleared selection is never a wrong one):**
- **Cleared one scroll early.** On a screen without history, an anchor pressed below row 0 is cleared one scroll early. When its content reaches row 0, the next scroll leaves `above` on row 0 and moves the anchor onto it, while the anchored content is still the top row. An anchor pressed on the last row has only `above`, so it behaves the same way.
- **Widening reflow.** A widening reflow during a gesture can merge the anchor's row with an adjacent row of the same wrapped line. The selection is then cleared, not misplaced. No test pins this down: it is an accepted loss, not a fix.
- **Pruned sentinel page.** A sentinel's scrollback page can be pruned while the anchor's page survives. The selection is then cleared early.
- **Loss is not latched.** It is re-evaluated on every `Select`. Two pins that met on a row in the scroll-region case can sit in different columns of that row. If a later narrowing reflow wraps that row, they can land on different rows again, and the gesture would extend from the reused row. Pins that meet on row 0 of a full-screen region both have `x` reset to 0, so they share a cell and never separate.
- **One-row screen without history.** No sentinel can be placed, so the gesture keeps today's behaviour (R-S2a). A code comment records this. Such a screen clears its single row in place rather than rotating it (`Screen.zig:897-904`).

**Files:**
- Modify: `crates/sprite-term/src/input/mouse.rs`:
  - `:6-7` (imports);
  - `:229-235` (`PinnedAnchor`);
  - `:254-267` (`track_selection_anchor`, plus the new `track_row`, `sentinel_error` and `pinned_anchor`);
  - `:269-275` (doc of `apply_selection`);
  - `:295-313` (the `Tracked` arm).
- Modify: `crates/sprite-term/src/worker/mod.rs:657-658` (comment only). The call sites at `:638` and `:652-656` need no change: `track_selection_anchor` and `SelectionAnchor::Tracked(&PinnedAnchor)` keep their signatures. Replacing or clearing `selection_anchor` (`:630`, `:672`) drops all three pins.
- Test: `crates/sprite-term/tests/selection.rs`:
  - one helper and six tests after `a_gesture_whose_anchor_was_evicted_selects_nothing` (`:455-504`);
  - the comment at `:464-467`.

**Interfaces:**
- Consumes:
  - the libghostty-vt 0.2.1 API listed above;
  - `SessionError::new(&'static str, impl Display)` (`crates/sprite-term/src/event.rs:98`);
  - the test helpers `EventPump`, `SnapshotPump`, `row_of`, `copied`, `at`, `args` and `session`, all already in `selection.rs` or `tests/support/mod.rs`.
- Produces (crate-private or private; no change to the public API):
  - `pub(crate) struct PinnedAnchor { tracked: TrackedGridRef, above: Option<TrackedGridRef>, below: Option<TrackedGridRef>, screen: Screen }`
  - `pub(crate) fn track_selection_anchor(terminal: &Terminal<'_, '_>, anchor: CellPosition) -> Result<PinnedAnchor, SessionError>` (signature unchanged)
  - `fn track_row(terminal: &Terminal<'_, '_>, y: Option<u32>) -> Result<Option<TrackedGridRef>, SessionError>`
  - `fn sentinel_error(error: libghostty_vt::Error) -> SessionError`
  - `fn pinned_anchor<'t>(terminal: &'t Terminal<'_, '_>, pin: &PinnedAnchor) -> Result<Option<GridRef<'t>>, SessionError>`
  - `pub(crate) fn apply_selection(...) -> Result<bool, SessionError>` (signature unchanged)

- [ ] **Step 1: Write the failing tests**

In `crates/sprite-term/tests/selection.rs`, replace the comment at `:464-467`, which describes the behaviour this task fixes:

```rust
    // The smallest nonzero budget: libghostty keeps scrollback in whole pages
    // and prunes the oldest, which is what evicts the anchored line. A zero
    // budget rotates rows in place instead; the rotation tests below cover it.
```

Then insert the following after the closing `}` of `a_gesture_whose_anchor_was_evicted_selects_nothing` (`:504`), before the doc comment of `a_gesture_does_not_extend_across_a_screen_switch`:

```rust
/// Drives one gesture across a stream of output. It presses on `ANCHOR-TEXT`
/// once `ready` is on screen, releases the child to stream until
/// `STREAM-DONE`, then extends the gesture to `head` and copies.
/// Returns the row the press landed on and what the copy carried.
fn press_then_stream(config: SessionConfig, ready: &str, head: CellPosition) -> (u16, String) {
    let sprite_term::Spawned {
        mut session,
        events,
        snapshots,
    } = TerminalSession::spawn(config).expect("spawn session");
    let events = EventPump::new(events);
    let snapshots = SnapshotPump::new(snapshots);
    events.expect_ready();

    let before = snapshots.wait_for("the anchor line", |bundle| {
        row_of(bundle, ready).is_some()
    });
    let pressed = at(row_of(&before, "ANCHOR-TEXT").expect("anchor row"), 0);
    session
        .send(TerminalCommand::BeginSelection { anchor: pressed })
        .expect("press");
    session
        .send(TerminalCommand::Input(b"go\n".to_vec()))
        .expect("release the child");
    let after = snapshots.wait_for("the end of the stream", |bundle| {
        row_of(bundle, "STREAM-DONE").is_some()
    });
    assert_eq!(
        row_of(&after, "ANCHOR-TEXT"),
        None,
        "the anchor line scrolled out of the pane"
    );

    session
        .send(TerminalCommand::Select {
            anchor: pressed,
            head,
            mode: SelectionMode::Character,
            rectangle: false,
        })
        .expect("extend the gesture");
    session
        .send(TerminalCommand::CopySelection)
        .expect("copy the selection");
    (pressed.row, copied(&events))
}

/// A pane that keeps no history scrolls by rotating rows in place: the
/// anchored row is reused, not evicted, so its pin keeps a value. The gesture
/// still selects nothing rather than extending from whatever rotated in.
#[test]
fn a_gesture_whose_anchor_was_rotated_out_with_zero_scrollback_selects_nothing() {
    let mut config = SessionConfig::command(
        "/bin/sh",
        args(&[
            "-c",
            "stty -echo; printf 'ANCHOR-TEXT\\nREADY'; IFS= read -r go; \
             seq 1 100; printf 'STREAM-DONE'; sleep 30",
        ]),
    );
    config.scrollback_bytes = 0;
    let (_, text) = press_then_stream(config, "READY", at(0, 3));
    assert_eq!(text, "", "a rotated-out anchor selects nothing");
}

/// The alternate screen never keeps history, so it always scrolls by rotation;
/// a gesture there must notice its row was reused just as a primary screen
/// without scrollback does.
#[test]
fn a_gesture_whose_anchor_was_rotated_out_on_the_alternate_screen_selects_nothing() {
    let config = SessionConfig::command(
        "/bin/sh",
        args(&[
            "-c",
            "stty -echo; printf '\\033[?1049h\\033[HANCHOR-TEXT\\nREADY'; IFS= read -r go; \
             seq 1 100; printf 'STREAM-DONE'; sleep 30",
        ]),
    );
    let (_, text) = press_then_stream(config, "READY", at(0, 3));
    assert_eq!(text, "", "a rotated-out anchor selects nothing");
}

/// A press on the bottom row has no row below it to watch, so its rotation is
/// detected from the row above. That clears the selection one scroll early,
/// while the anchored content is still on the top row, and never lets the
/// gesture extend from a reused row.
#[test]
fn a_gesture_anchored_on_the_last_row_is_cleared_before_it_drifts() {
    let mut config = SessionConfig::command(
        "/bin/sh",
        args(&[
            "-c",
            "stty -echo; seq 1 30; printf 'ANCHOR-TEXT'; IFS= read -r go; \
             echo; seq 1 100; printf 'STREAM-DONE'; sleep 30",
        ]),
    );
    config.scrollback_bytes = 0;
    let (pressed, text) = press_then_stream(config, "ANCHOR-TEXT", at(0, 3));
    assert_eq!(
        pressed,
        sprite_term::ValidTerminalSize::DEFAULT.rows() - 1,
        "the press landed on the bottom row"
    );
    assert_eq!(text, "", "a rotated-out anchor selects nothing");
}

/// A press on the middle row of a wrapped line, on a pane without history.
/// The stream stops one scroll after the anchored row was rotated out, while
/// the line's last row sits where the anchor's row was. The gesture must not
/// extend from that row, even though it belongs to the same line.
#[test]
fn a_gesture_anchored_mid_way_down_a_wrapped_line_is_cleared_when_its_row_rotates_out() {
    // Three rows at 80 columns: 80 `x`s, then `ANCHOR-TEXT` padded to a full
    // row so it wraps, then `TAIL-OF-THE-LINE`.
    let line = format!(
        "{}ANCHOR-TEXT{}TAIL-OF-THE-LINE",
        "x".repeat(80),
        " ".repeat(69)
    );
    // 30 numbers push the line to rows 20-22 with READY on row 23. `echo`
    // plus `seq 1 21` is 22 line feeds: 21 bring the anchor's row to row 0,
    // the 22nd rotates it out and leaves the line's last row there.
    let mut config = SessionConfig::command(
        "/bin/sh",
        args(&[
            "-c",
            &format!(
                "stty -echo; seq 1 30; printf '{line}\\nREADY'; IFS= read -r go; \
                 echo; seq 1 21; printf 'STREAM-DONE'; sleep 30"
            ),
        ]),
    );
    config.scrollback_bytes = 0;
    let (pressed, text) = press_then_stream(config, "READY", at(0, 3));
    assert_eq!(pressed, 21, "the press landed on the line's middle row");
    assert_eq!(text, "", "a rotated-out anchor selects nothing");
}

/// A scroll region with a top margin erases its top row by moving that row's
/// pin onto the row above the region, so a pressed anchor there would land on
/// content outside the region. The gesture selects nothing instead.
#[test]
fn a_gesture_inside_a_scroll_region_is_cleared_when_its_row_rotates_out() {
    // Rows 0-1 stay outside the region; the region is rows 2-19 (DECSTBM
    // 3;20). The cursor waits on the region's bottom row, so the one line
    // feed after `go` scrolls the region once and erases ANCHOR-TEXT's row.
    let config = SessionConfig::command(
        "/bin/sh",
        args(&[
            "-c",
            "stty -echo; printf '\\033[1;1HHEADER\\033[2;1HABOVE-REGION\\033[3;1HANCHOR-TEXT\
             \\033[4;1HREADY\\033[3;20r\\033[20;1H'; IFS= read -r go; \
             printf '\\nSTREAM-DONE'; sleep 30",
        ]),
    );
    let (pressed, text) = press_then_stream(config, "READY", at(1, 11));
    assert_eq!(pressed, 2, "the press landed on the region's top row");
    assert_eq!(text, "", "a rotated-out anchor selects nothing");
}
```

Why each test fails at `03b4929`. At master the only checks are "snapshot `None`" and "other screen".
- **Zero scrollback.**
  - The press pins row 0 (`ANCHOR-TEXT`). Each scroll is a `cursorScrollRegionUp` rotation that leaves that pin on row 0, so its snapshot never becomes `None`.
  - The copy is row 0's text. The 102 logical lines are `ANCHOR-TEXT`, `READY1`, then `2` to `100`, then `STREAM-DONE`. The last 24 start at `78`.
  - With the fix, `below` moves onto row 0 in the first scroll.
- **Alternate screen.**
  - The mechanics are the same, because the alternate screen's `max_scrollback` is 0. The active screen is still the alternate one at `Select`, so master's screen check passes. The copy is `78`.
  - With the fix, `below` meets the anchor in the first scroll.
- **Last row.**
  - `ANCHOR-TEXT` is on row 23, with numbers 8 to 30 above it. At master the pin rides up to row 0 and stays there. The copy is row 0's final text, `78`.
  - With the fix, `above` (row 22) reaches row 0 and the anchor joins it one scroll later.
- **Mid-way down a wrapped line.**
  - Rows 20, 21 and 22 hold the line, and row 23 holds `READY`. The press is on row 21.
  - After 21 line feeds the anchor is on row 0. The 22nd erases it, and the pin stays on row 0, which now holds `TAIL-OF-THE-LINE`. The master copy from `(0,0)` to `(0,3)` is `TAIL`.
  - With the fix, `above` (row 20) reached row 0 after 20 scrolls, and the anchor joined it on the 21st. The earlier logical-line design kept its sentinel on `READY`, which was still on row 1, and copied `TAIL` too.
- **Scroll region.**
  - One line feed scrolls rows 2 to 19. The fixup moves the anchor from row 2 to row 1 (`ABOVE-REGION`), with `x` unchanged at 0.
  - The master copy from `(1,0)` to `(1,11)` is `ABOVE-REGION`.
  - With the fix, `above` is on row 1, outside the region and not moved, so the anchor lands on its row.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `env PATH=/Users/davidlee/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin:/Users/davidlee/.cargo/bin:/opt/homebrew/bin:/usr/bin:/bin cargo test -p sprite-term --locked --offline --test selection -- rotated_out anchored_on_the_last_row anchored_mid_way inside_a_scroll_region`

Expected: all five FAIL.
- `a_gesture_whose_anchor_was_rotated_out_with_zero_scrollback_selects_nothing`, `a_gesture_whose_anchor_was_rotated_out_on_the_alternate_screen_selects_nothing` and `a_gesture_anchored_on_the_last_row_is_cleared_before_it_drifts` fail with:

  ```
  assertion `left == right` failed: a rotated-out anchor selects nothing
    left: "78"
   right: ""
  ```

- `a_gesture_anchored_mid_way_down_a_wrapped_line_is_cleared_when_its_row_rotates_out` fails with the same message and `left: "TAIL"`.
- `a_gesture_inside_a_scroll_region_is_cleared_when_its_row_rotates_out` fails with the same message and `left: "ABOVE-REGION"`.

The `left` text in these failures was worked out by hand, not observed. Any non-empty `left` is the failure that matters. If a test instead fails on one of its setup assertions, the shell script is wrong; fix the script, not the code. The setup assertions are `the anchor line scrolled out of the pane`, `the press landed on the bottom row`, `the press landed on the line's middle row` and `the press landed on the region's top row`.

- [ ] **Step 3: Implement**

In `crates/sprite-term/src/input/mouse.rs`, replace the imports at `:6-7`:

```rust
use libghostty_vt::screen::{GridRef, Screen, TrackedGridRef};
use libghostty_vt::terminal::{Point, PointCoordinate, PointSpace};
```

Replace `PinnedAnchor` (`:229-235`) with:

```rust
/// A gesture's press pinned to the content under it, with the screen that was
/// active when it was pinned: a pin belongs to that screen, so it means
/// nothing once the other one is showing.
///
/// A pin alone cannot always tell that its content is gone. A scroll that
/// keeps no history (the alternate screen, a pane with no scrollback budget,
/// or any scroll region with a top margin) rotates rows in place. The pin on
/// the erased row is not dropped. It stays put, or moves to the row above the
/// region, and silently names whatever is there now. The sentinels are pins
/// on the rows directly above and below the anchor. Rotating the anchor's row
/// away is exactly what lands one of them on the anchor's row, so a shared
/// row means the anchor is lost.
pub(crate) struct PinnedAnchor {
    tracked: TrackedGridRef,
    /// The row directly above the anchor's; `None` on the top screen row.
    above: Option<TrackedGridRef>,
    /// The row directly below the anchor's; `None` on the bottom screen row.
    /// With neither sentinel (a one-row screen without history) the anchor
    /// is checked only for eviction and screen switches.
    below: Option<TrackedGridRef>,
    screen: Screen,
}
```

Replace `track_selection_anchor` (`:254-267`) with:

```rust
/// Pins a gesture's anchor to the content under a viewport cell, so it follows
/// that content through scrolling, scrollback pruning and reflow.
pub(crate) fn track_selection_anchor(
    terminal: &Terminal<'_, '_>,
    anchor: CellPosition,
) -> Result<PinnedAnchor, SessionError> {
    let screen = terminal
        .active_screen()
        .map_err(|error| SessionError::new("selection_anchor", error))?;
    let tracked = terminal
        .track_grid_ref(viewport_point(anchor))
        .map_err(|error| SessionError::new("selection_anchor", error))?;
    let row = tracked.point(PointSpace::Screen).map_err(sentinel_error)?;
    let total = u32::try_from(terminal.total_rows().map_err(sentinel_error)?).unwrap_or(u32::MAX);
    let above = track_row(terminal, row.and_then(|at| at.y.checked_sub(1)))?;
    let below = track_row(
        terminal,
        row.and_then(|at| at.y.checked_add(1)).filter(|&y| y < total),
    )?;
    Ok(PinnedAnchor {
        tracked,
        above,
        below,
        screen,
    })
}

/// Pins column 0 of screen-space row `y`, where row 0 is the oldest line of
/// scrollback, when there is such a row.
fn track_row(
    terminal: &Terminal<'_, '_>,
    y: Option<u32>,
) -> Result<Option<TrackedGridRef>, SessionError> {
    y.map(|y| {
        terminal
            .track_grid_ref(Point::Screen(PointCoordinate { x: 0, y }))
            .map_err(sentinel_error)
    })
    .transpose()
}

fn sentinel_error(error: libghostty_vt::Error) -> SessionError {
    SessionError::new("selection_sentinel", error)
}

/// The cell a pinned press still names. `None` once it names something else:
/// the other screen is showing, its content was evicted, or its row was
/// rotated away, which shows as a sentinel lost or on the anchor's row.
fn pinned_anchor<'t>(
    terminal: &'t Terminal<'_, '_>,
    pin: &PinnedAnchor,
) -> Result<Option<GridRef<'t>>, SessionError> {
    let lost = |error: libghostty_vt::Error| SessionError::new("selection_anchor", error);
    if terminal.active_screen().map_err(lost)? != pin.screen {
        return Ok(None);
    }
    let Some(anchor_ref) = pin.tracked.snapshot(terminal).map_err(lost)? else {
        return Ok(None);
    };
    let Some(anchor_at) = pin.tracked.point(PointSpace::Screen).map_err(lost)? else {
        return Ok(None);
    };
    for sentinel in pin.above.iter().chain(pin.below.iter()) {
        let Some(sentinel_at) = sentinel.point(PointSpace::Screen).map_err(lost)? else {
            return Ok(None);
        };
        if sentinel_at.y == anchor_at.y {
            return Ok(None);
        }
    }
    Ok(Some(anchor_ref))
}
```

Replace the doc comment of `apply_selection` (`:269-275`) with:

```rust
/// Installs a selection whose head is a viewport cell.
///
/// Word and line modes delegate to libghostty so Sprite agrees with Ghostty on
/// what a word or a wrapped line is, rather than inventing its own boundaries.
///
/// Returns `false` when a tracked anchor no longer names the content it was
/// pressed on (evicted, or its row rotated away) or the other screen is now
/// active; the selection is then cleared rather than re-anchored on whatever
/// took its place.
```

Replace the `SelectionAnchor::Tracked(pin)` arm (`:295-313`) with:

```rust
                SelectionAnchor::Tracked(pin) => {
                    let Some(anchor_ref) = pinned_anchor(terminal, pin)? else {
                        terminal
                            .set_selection(None)
                            .map_err(|error| SessionError::new("clear_selection", error))?;
                        return Ok(false);
                    };
                    anchor_ref
                }
```

In `crates/sprite-term/src/worker/mod.rs`, replace the comment at `:657-658`:

```rust
                        // `false`: the anchored content is gone (evicted, or
                        // its row rotated away) or the other screen is showing,
                        // and the selection was cleared rather than moved.
```

- [ ] **Step 4: Format, then run the tests, clippy and the crate suite**

Run: `env PATH=/Users/davidlee/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin:/Users/davidlee/.cargo/bin:/opt/homebrew/bin:/usr/bin:/bin cargo fmt --all` then `env PATH=/Users/davidlee/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin:/Users/davidlee/.cargo/bin:/opt/homebrew/bin:/usr/bin:/bin cargo fmt --all -- --check`
Expected: the check prints nothing.

Run: `env PATH=/Users/davidlee/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin:/Users/davidlee/.cargo/bin:/opt/homebrew/bin:/usr/bin:/bin cargo test -p sprite-term --locked --offline --test selection`
Expected: everything passes. That includes:
- the six new tests;
- `a_gesture_anchor_follows_its_content_while_output_scrolls`, where the scrollback-keeping primary screen moves all three pins together;
- `a_gesture_whose_anchor_was_evicted_selects_nothing`;
- `a_gesture_does_not_extend_across_a_screen_switch`.

Run: `env PATH=/Users/davidlee/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin:/Users/davidlee/.cargo/bin:/opt/homebrew/bin:/usr/bin:/bin cargo clippy -p sprite-term --all-targets --locked --offline -- -D warnings`
Expected: no warnings.

Run: `env PATH=/Users/davidlee/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin:/Users/davidlee/.cargo/bin:/opt/homebrew/bin:/usr/bin:/bin cargo test -p sprite-term --locked --offline --no-fail-fast`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git -C /Users/davidlee/Projects/Sprite/.worktrees/close-open-findings add crates/sprite-term/src/input/mouse.rs crates/sprite-term/src/worker/mod.rs crates/sprite-term/tests/selection.rs
git -C /Users/davidlee/Projects/Sprite/.worktrees/close-open-findings commit -m "$(cat <<'EOF'
Detect a selection anchor whose row was rotated away

A scroll without history rotates rows in place, which leaves the
anchor's pin on a reused row. Pins on the rows directly above and below
the anchor catch that: one of them lands on the anchor's row exactly
when the anchor's row is rotated out.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
EOF
)"
```

---

---

### Task 5: Each Surface batch has a drain deadline (R-D; GAP-C07, #67)

**Files:**
- Modify: `crates/sprite-app/src/timeouts.rs:27-32` (add `MIN_DRAIN_RATE` and `drain_deadline` after `WRITE_TIMEOUT`), `:79-90` (one more const assert), `:92-107` (one more unit test)
- Modify: `crates/sprite-app/src/surface/channel.rs:13` (import), `:22-23` (`Duration` import is no longer test-only), `:204-206` (add `WRITE_CHUNK_BYTES`), `:307-309` (doc on `SurfaceConnection`), `:339-349` (writer-thread comment), `:474-519` (`write_events`, plus new `write_within`)
- Modify: `docs/adr/0029-run-blocking-socket-writes-and-pane-cleanup-on-dedicated-threads.md` (new paragraph between the paragraphs at `:3` and `:5`)
- Test: `crates/sprite-app/src/surface/channel.rs:2011-2098`. `a_draining_writer_keeps_its_connection_slot_until_it_returns` is replaced by `a_reader_slower_than_the_drain_floor_loses_its_slot`. `dropping_the_last_handle_never_waits_for_a_stalled_writer` (`:1965-1987`) is unchanged.

**Interfaces:**
- Consumes: `crate::timeouts::WRITE_TIMEOUT` (`timeouts.rs:29-32`); `LocalSocket::live_connections(&self) -> usize` (`local_socket.rs:364`, `#[cfg(test)]`); `crate::test_blocking_wait::{pause, watchdog}`; the channel test helpers `Scratch::new`, `endpoint`, `window`, `connect`, `open_message` (`channel.rs:990-1081`).
- Produces: `pub(crate) const MIN_DRAIN_RATE: u64` (bytes per second; 65 536 in production, 1 310 720 under `cfg(test)`); `pub(crate) const fn drain_deadline(bytes: usize) -> Duration` in `crate::timeouts`; private `const WRITE_CHUNK_BYTES: usize = 64 * 1024` and `fn write_within(stream: &UnixStream, bytes: &[u8], budget: Duration) -> std::io::Result<()>` in `surface/channel.rs`.

Why the test rate is 1 310 720 B/s (1.25 MiB/s): the test's batch is the 1 MiB event. Its deadline is then `200 ms + 1 048 576 × 1000 / 1 310 720 ms = 200 + 800 ms = 1 s`. Every other channel test that moves megabytes reads at full speed, so 1.25 MiB/s does not affect them. The largest such test is `a_single_event_larger_than_the_bound_is_delivered_whole` at 5 MiB, which gets 4.2 s. In production, a full 4 MiB queue gets `2 s + 4 194 304 × 1000 / 65 536 ms = 2 + 64 s = 66 s`, as R-D1 requires.

- [ ] **Step 1: Write the failing test**

In `crates/sprite-app/src/surface/channel.rs`, replace the whole of `:2011-2098`, from the doc comment `/// The connection cap is also the cap on writer threads: a program that` through the closing `}` of `a_draining_writer_keeps_its_connection_slot_until_it_returns`, with:

```rust
    /// The connection cap is also the cap on writer threads, so a writer
    /// must not keep its slot for as long as a reader cares to trickle. A
    /// program closes its Surface with a megabyte queued, then reads it a few
    /// kilobytes at a time. Each read comes well inside the write timeout, so
    /// no single write ever times out. The writer gives up at the batch's
    /// drain deadline anyway, the slot comes back, and the rest of the
    /// megabyte is never delivered.
    #[test]
    fn a_reader_slower_than_the_drain_floor_loses_its_slot() {
        use std::io::Read;
        let scratch = Scratch::new();
        let (endpoint, rx) = endpoint(&scratch);
        let (connections, connection) = mpsc::channel();
        let (closes, closed) = mpsc::channel();
        let _window = window(rx, move |request| {
            match request {
                SurfaceRequest::Open {
                    connection, reply, ..
                } => {
                    reply.send(Ok(())).expect("reply");
                    connections.send(connection).expect("connection");
                }
                SurfaceRequest::Close { .. } => {
                    closes.send(()).expect("close");
                    return false;
                }
                other @ (SurfaceRequest::Capabilities { .. }
                | SurfaceRequest::Update { .. }
                | SurfaceRequest::Focus { .. }
                | SurfaceRequest::Closed { .. }
                | SurfaceRequest::FocusPane { .. }
                | SurfaceRequest::Grid { .. }
                | SurfaceRequest::List { .. }
                | SurfaceRequest::RegisterToken { .. }) => panic!("unexpected request: {other:?}"),
            }
            true
        });
        let (mut stream, _reader) = connect(&endpoint);
        writeln!(stream, "{} {}", endpoint.key_hex(), open_message(3)).expect("open");
        let connection = connection
            .recv_timeout(Duration::from_secs(2))
            .expect("the window saw the open");
        // At most 4 KiB every WRITE_TIMEOUT / 5: about 100 KiB/s, a
        // thirteenth of the test-scale drain floor. Every read is far inside
        // the write timeout, so the writer always makes progress.
        let reader = std::thread::spawn({
            let mut stream = stream.try_clone().expect("clone");
            move || {
                let mut received = 0;
                let mut chunk = [0; 4096];
                loop {
                    match stream.read(&mut chunk) {
                        Ok(0) | Err(_) => return received,
                        Ok(count) => received += count,
                    }
                    crate::test_blocking_wait::pause(WRITE_TIMEOUT / 5);
                }
            }
        });
        let event = "x".repeat(1024 * 1024);
        assert!(connection.send(&event));
        writeln!(stream, "{}", json!({"type":"close"})).expect("close");
        closed
            .recv_timeout(Duration::from_secs(2))
            .expect("the window saw the close");
        drop(connection);
        // Every handle is gone and the connection thread has returned, so
        // only the writer holds the slot now. At this reader's pace the
        // megabyte takes over ten seconds, far beyond its drain deadline.
        let deadline = Instant::now() + crate::test_blocking_wait::watchdog();
        while endpoint.transport.live_connections() != 0 {
            assert!(
                Instant::now() < deadline,
                "a reader slower than the drain floor kept the writer's slot"
            );
            crate::test_blocking_wait::pause(Duration::from_millis(5));
        }
        let received = reader.join().expect("reader");
        assert!(
            received < event.len(),
            "the writer gave up before delivering the megabyte ({received} bytes arrived)"
        );
    }
```

- [ ] **Step 2: Run it and watch it fail**

Run: `env PATH=/Users/davidlee/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin:/Users/davidlee/.cargo/bin:/opt/homebrew/bin:/usr/bin:/bin cargo test -p sprite-app --locked --offline a_reader_slower_than_the_drain_floor_loses_its_slot`

Expected: FAIL, panicking with `a reader slower than the drain floor kept the writer's slot` after the 5 s watchdog. Today the writer keeps writing for as long as the reader makes progress, and at about 100 KiB/s it needs at least 10.24 s for the megabyte. With `SPRITE_TEST_WATCHDOG_SECS` raised above that, the test fails instead on `the writer gave up before delivering the megabyte`, because every byte arrives.

- [ ] **Step 3: Add the drain floor to `timeouts.rs`**

Insert directly after `WRITE_TIMEOUT` (after `timeouts.rs:32`):

```rust

/// The slowest a program may read its Surface events, in bytes per second.
/// A write timeout only catches a reader that stops completely. A reader
/// that takes a few bytes at a time, each sooner than the timeout, would
/// otherwise keep a writer thread and its connection slot for as long as it
/// kept reading. Fast under test, so the drain-floor test's one-megabyte
/// batch is due in a second.
#[cfg(not(test))]
pub(crate) const MIN_DRAIN_RATE: u64 = 64 * 1024;
#[cfg(test)]
pub(crate) const MIN_DRAIN_RATE: u64 = 1280 * 1024;

/// How long a Surface writer may spend on a batch of `bytes`: the write
/// timeout, plus the time the batch takes at `MIN_DRAIN_RATE`. A full 4 MiB
/// queue gets about 66 s.
pub(crate) const fn drain_deadline(bytes: usize) -> Duration {
    let millis = (bytes as u64).saturating_mul(1000) / MIN_DRAIN_RATE;
    WRITE_TIMEOUT.saturating_add(Duration::from_millis(millis))
}
```

After `const _: () = assert!(longer(REPLY_TIMEOUT, SILENT_CLIENT_TIMEOUT));` (`:90`) add:

```rust
const _: () = assert!(MIN_DRAIN_RATE > 0, "the drain deadline divides by the floor");
```

In the `tests` module (`:92-107`) add after the existing test:

```rust

    /// An empty batch gets only the write timeout. Each further floor's
    /// worth of bytes adds one second.
    #[test]
    fn a_batch_gets_the_write_timeout_plus_its_time_at_the_drain_floor() {
        assert_eq!(drain_deadline(0), WRITE_TIMEOUT);
        let one_second = usize::try_from(MIN_DRAIN_RATE).unwrap();
        assert_eq!(
            drain_deadline(3 * one_second),
            WRITE_TIMEOUT + Duration::from_secs(3)
        );
    }
```

- [ ] **Step 4: Write each batch in chunks against its deadline**

In `crates/sprite-app/src/surface/channel.rs`:

`:13` becomes:

```rust
use crate::timeouts::{HANDSHAKE_TIMEOUT, REPLY_TIMEOUT, WRITE_TIMEOUT, drain_deadline};
```

`:22-23` (`#[cfg(test)]` / `use std::time::Duration;`) become one line:

```rust
use std::time::{Duration, Instant};
```

After `const KEPT_WRITE_BUFFER_BYTES: usize = 64 * 1024;` (`:206`) add:

```rust
/// The most a writer hands the socket in one write, so it checks its drain
/// deadline at least this often.
const WRITE_CHUNK_BYTES: usize = 64 * 1024;
```

In the `SurfaceConnection` doc, replace `:308-309`:

```rust
/// pending. Sending with the bound already reached, a write that times out,
/// or a failed write marks the connection dead and shuts the socket down;
```

with:

```rust
/// pending. Sending with the bound already reached, a write that times out,
/// a batch that misses its drain deadline, or a failed write marks the
/// connection dead and shuts the socket down;
```

Replace the writer-thread comment at `:339-349` (from `// Detached on purpose, and nothing ever waits to join it, the GPUI` through `// user can connect at all.`) with:

```rust
        // Detached on purpose, and nothing ever waits to join it, the GPUI
        // thread least of all. It returns as soon as the connection is dead.
        // Once the last handle is gone nothing more can be queued, so what is
        // left to drain is at most the pending bound plus one event. Each
        // batch has a drain deadline: `WRITE_TIMEOUT` plus the time the batch
        // takes at `MIN_DRAIN_RATE`. A reader slower than that loses the
        // connection, however steadily it reads, so the thread and its slot
        // are bounded in time as well as in bytes. It holds one of the
        // endpoint's slots and no more, so the cap on connections still caps
        // writer threads, and only a program running as the same user can
        // connect at all.
```

In `write_events`, replace:

```rust
        let mut stream = &shared.stream;
        let written = stream.write_all(&outgoing).and_then(|()| stream.flush());
```

with:

```rust
        let written = write_within(&shared.stream, &outgoing, drain_deadline(outgoing.len()));
```

and replace the error comment:

```rust
            // The write timed out or failed: the program is not reading, or
            // is gone. Shutting the socket down ends the connection thread's
            // read, and the window hears the Surface closed.
```

with:

```rust
            // The write timed out, the batch missed its drain deadline, or
            // the write failed: the program is not reading fast enough, or
            // is gone. Shutting the socket down ends the connection thread's
            // read, and the window hears the Surface closed.
```

Directly after `write_events` (after its closing `}` at `:519`) add:

```rust

/// Writes `bytes` within `budget`, one chunk at a time. Before each chunk
/// the socket's write timeout is set to the smaller of `WRITE_TIMEOUT` and
/// the time left. A write that makes no progress still fails after
/// `WRITE_TIMEOUT`, and a reader that makes a little progress on every
/// write still runs out of time. Running out is a timeout like any other.
///
/// The timeout is a property of the socket, shared by every clone of it.
/// Once a connection is opened only its writer writes to it, so narrowing
/// the timeout here does not affect any other writer.
fn write_within(mut stream: &UnixStream, bytes: &[u8], budget: Duration) -> std::io::Result<()> {
    let started = Instant::now();
    let mut rest = bytes;
    while !rest.is_empty() {
        let left = budget.saturating_sub(started.elapsed());
        // Checked before the socket is told: it refuses a zero timeout.
        if left.is_zero() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "the reader fell below the drain floor",
            ));
        }
        stream.set_write_timeout(Some(left.min(WRITE_TIMEOUT)))?;
        let (chunk, _) = rest.split_at(rest.len().min(WRITE_CHUNK_BYTES));
        match stream.write(chunk) {
            Ok(0) => return Err(std::io::ErrorKind::WriteZero.into()),
            Ok(written) => rest = rest.get(written..).unwrap_or_default(),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    stream.flush()
}
```

Only the deadline check above runs between writes. One write can therefore overrun the deadline by as long as the kernel takes to accept at most `WRITE_CHUNK_BYTES`. During that write each kernel wait is still capped at `left`. `holding` still calls `stream.set_write_timeout(Some(WRITE_TIMEOUT))` at `:332`, so a connection's socket has a bounded timeout before its first batch.

- [ ] **Step 5: Record the rule in ADR 0029**

In `docs/adr/0029-run-blocking-socket-writes-and-pane-cleanup-on-dedicated-threads.md`, insert this paragraph after the first paragraph (`:3`), with a blank line on each side:

```markdown
A write timeout bounds a write that makes no progress, not one that makes a little. A reader that took a few bytes at a time, each sooner than the timeout, kept a writer thread and its connection slot for as long as it kept reading. Each batch a writer takes from its queue therefore has a drain deadline: the write timeout plus the batch's size at a floor of 64 KiB/s (`MIN_DRAIN_RATE` and `drain_deadline` in `timeouts.rs`), so a full 4 MiB queue has about 66 s. The writer hands the batch to the socket in writes of at most 64 KiB. Before each write it sets the socket's write timeout to the smaller of the write timeout and the time left. A batch that misses its deadline counts as a failed write: the connection is shut down and the writer returns, releasing its slot. A reader below the floor is treated as gone, as a stalled one already was. The deadline is checked between writes, so one write can overrun it by as long as the kernel takes to accept 64 KiB. Rejected: a fixed per-batch deadline (a large paste to a healthy but busy reader would fail) and resetting a timer on progress (the trickling reader makes progress on every write).
```

- [ ] **Step 6: Run the tests and see them pass**

Run: `env PATH=/Users/davidlee/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin:/Users/davidlee/.cargo/bin:/opt/homebrew/bin:/usr/bin:/bin cargo test -p sprite-app --locked --offline a_reader_slower_than_the_drain_floor_loses_its_slot`
Expected: PASS. It takes about 1 to 2 s: the deadline is 1 s, and the write in progress can run on by up to one 64 KiB chunk at the reader's pace.

Run: `env PATH=/Users/davidlee/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin:/Users/davidlee/.cargo/bin:/opt/homebrew/bin:/usr/bin:/bin cargo test -p sprite-app --locked --offline surface::channel`
Expected: PASS, including `dropping_the_last_handle_never_waits_for_a_stalled_writer` (unchanged), `a_backpressured_batch_keeps_its_written_prefix_and_shuts_down_on_timeout`, `a_large_batch_delivers_every_byte_while_the_peer_drains_in_small_chunks` and `a_single_event_larger_than_the_bound_is_delivered_whole`.

Run: `env PATH=/Users/davidlee/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin:/Users/davidlee/.cargo/bin:/opt/homebrew/bin:/usr/bin:/bin cargo test -p sprite-app --locked --offline timeouts`
Expected: PASS, including `a_batch_gets_the_write_timeout_plus_its_time_at_the_drain_floor`.

Run: `env PATH=/Users/davidlee/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin:/Users/davidlee/.cargo/bin:/opt/homebrew/bin:/usr/bin:/bin cargo clippy -p sprite-app --all-targets --locked --offline -- -D warnings`
Expected: no warnings.

- [ ] **Step 7: Commit**

```bash
git -C /Users/davidlee/Projects/Sprite/.worktrees/close-open-findings add crates/sprite-app/src/timeouts.rs crates/sprite-app/src/surface/channel.rs docs/adr/0029-run-blocking-socket-writes-and-pane-cleanup-on-dedicated-threads.md
git -C /Users/davidlee/Projects/Sprite/.worktrees/close-open-findings commit -m "$(cat <<'EOF'
Give every Surface batch a drain deadline

GAP-C07 (#67): a reader that took a few bytes at a time, each inside the
write timeout, kept a writer thread and its connection slot indefinitely.
Each batch is now written in 64 KiB chunks against WRITE_TIMEOUT plus its
size at a 64 KiB/s floor, and a batch that misses the deadline kills the
connection like any failed write. ADR 0029 records the floor.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
EOF
)"
```

---

---

### Task 6: GPUI asks the app before it terminates for a quit the app did not request (R-Q1, R-Q2, R-Q3)

**Files:**
- Modify: `vendor/gpui/src/platform.rs:88` (test re-exports), insert before `:165` (`pub(crate) trait Platform`), insert after `:234` (`fn on_reopen`)
- Modify: `vendor/gpui/src/platform/test.rs:11`
- Modify: `vendor/gpui/src/platform/test/platform.rs:13,41-42,122-124,251,364`
- Modify: `vendor/gpui/src/platform/mac/platform.rs:97-100,170,211,503-504,878-880`, insert before `:1431` (`extern "C" fn on_keyboard_layout_change`)
- Modify: `vendor/gpui/src/app.rs`, insert after `:751` (end of `App::quit`)
- Modify: `vendor/gpui/src/app/test_context.rs`, insert after `:338` (end of `set_screen_capture_sources`)
- Create: `patches/gpui-0.2.2-should-terminate.patch` (generated in Step 7)
- Modify: `scripts/check_gpui_patch.py:2,10-11,81`
- Modify: `scripts/test_gpui_patch.py:7`, insert before `:30`
- Modify: `.gitattributes:3`
- Modify: `vendor/gpui.provenance.md:11-12`, append a section after `:46` (end of "Caller inventory")
- Modify: `DEPENDENCIES.md:250-253`
- Create: `crates/sprite-app/tests/platform_quit_request.rs`

**Interfaces:**
- Consumes: `App::this: Weak<AppCell>` (`vendor/gpui/src/app.rs:539`), `MacPlatform` delegate ivar and `get_mac_platform` (`mac/platform.rs:1362`), `dispatch_async_f`/`dispatch_get_main_queue` (`mac/dispatcher`), `TestAppContext::test_platform`.
- Produces (crate root `gpui::`):
  ```rust
  pub struct TerminateReply { /* private */ }
  impl TerminateReply {
      pub(crate) fn new(send: impl FnOnce(bool) + 'static) -> Self;
      pub fn allow(self);
      pub fn cancel(self);
  }
  impl Drop for TerminateReply; // unanswered = allow
  // pub(crate) trait Platform:
  fn on_should_terminate(&self, _callback: Box<dyn FnMut(TerminateReply)>) {} // default: never asked
  impl App {
      pub fn on_should_terminate(&self, callback: impl FnMut(TerminateReply, &mut App) + 'static);
  }
  // test-support only:
  pub enum TerminateAnswer { Pending, Allowed, Cancelled }
  pub struct TestTerminateRequest; // Clone, Debug, Default
  impl TestTerminateRequest { pub fn answer(&self) -> TerminateAnswer; }
  impl TestAppContext { pub fn simulate_should_terminate(&self) -> TestTerminateRequest; }
  ```
- Produces (`scripts/check_gpui_patch.py`): `PATCHES: tuple[str, ...]`, applied in order by `verify`.

- [ ] **Step 1: Write the failing contract tests**

Create `crates/sprite-app/tests/platform_quit_request.rs`:

```rust
//! GPUI's should-terminate hook, driven through its test platform: the
//! question macOS asks before a Quit from the Dock or the menu bar, or the
//! end of the session, ends the application.

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{TerminateAnswer, TerminateReply};

/// The replies the callback was handed, kept so a test can answer them later.
type Held = Rc<RefCell<Vec<TerminateReply>>>;

fn hold_replies(cx: &gpui::TestAppContext) -> Held {
    let held: Held = Rc::default();
    let keep = Rc::clone(&held);
    cx.update(|cx| cx.on_should_terminate(move |reply, _| keep.borrow_mut().push(reply)));
    held
}

fn take(held: &Held) -> TerminateReply {
    let reply = held.borrow_mut().pop();
    let Some(reply) = reply else {
        panic!("the callback was not asked");
    };
    reply
}

#[gpui::test]
fn without_a_callback_the_platform_terminates_at_once(cx: &mut gpui::TestAppContext) {
    assert_eq!(
        cx.simulate_should_terminate().answer(),
        TerminateAnswer::Allowed
    );
}

#[gpui::test]
fn the_callback_answers_now_or_later(cx: &mut gpui::TestAppContext) {
    let held = hold_replies(cx);
    let cancelled = cx.simulate_should_terminate();
    assert_eq!(cancelled.answer(), TerminateAnswer::Pending);
    take(&held).cancel();
    assert_eq!(cancelled.answer(), TerminateAnswer::Cancelled);

    let allowed = cx.simulate_should_terminate();
    assert_eq!(allowed.answer(), TerminateAnswer::Pending);
    take(&held).allow();
    assert_eq!(allowed.answer(), TerminateAnswer::Allowed);
}

/// One reply is outstanding at a time: a request made while the application
/// still holds one is refused without asking, and the held reply decides.
#[gpui::test]
fn a_request_while_one_is_unanswered_is_refused_without_asking(cx: &mut gpui::TestAppContext) {
    let held = hold_replies(cx);
    let first = cx.simulate_should_terminate();
    let second = cx.simulate_should_terminate();
    assert_eq!(second.answer(), TerminateAnswer::Cancelled);
    assert_eq!(held.borrow().len(), 1, "the callback was asked twice");
    take(&held).allow();
    assert_eq!(first.answer(), TerminateAnswer::Allowed);
}

/// A reply nobody answers lets the application end, as it did before the
/// hook existed, rather than leaving it impossible to quit.
#[gpui::test]
fn a_dropped_reply_allows(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| cx.on_should_terminate(|reply, _| drop(reply)));
    assert_eq!(
        cx.simulate_should_terminate().answer(),
        TerminateAnswer::Allowed
    );
}

/// A quit the application requested itself is already decided, and is not
/// put to the callback a second time.
#[gpui::test]
fn a_quit_the_app_requested_is_not_asked_about(cx: &mut gpui::TestAppContext) {
    let held = hold_replies(cx);
    cx.update(|cx| cx.quit());
    assert_eq!(
        cx.simulate_should_terminate().answer(),
        TerminateAnswer::Allowed
    );
    assert!(held.borrow().is_empty(), "the callback was asked");
}
```

- [ ] **Step 2: Run to verify failure**

Run: `env PATH=/Users/davidlee/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin:/Users/davidlee/.cargo/bin:/opt/homebrew/bin:/usr/bin:/bin cargo test -p sprite-app --locked --offline --test platform_quit_request`
Expected: fails to compile with `error[E0432]: unresolved imports gpui::TerminateAnswer, gpui::TerminateReply` and `no method named on_should_terminate` / `simulate_should_terminate`. At `03b4929` GPUI has no way to ask the app, which is the bug.

- [ ] **Step 3: Implement the platform-independent half**

In `vendor/gpui/src/platform.rs`, replace line 88:

```rust
pub use test::{TestDispatcher, TestScreenCaptureSource, TestScreenCaptureStream};
```

with:

```rust
pub use test::{
    TerminateAnswer, TestDispatcher, TestScreenCaptureSource, TestScreenCaptureStream,
    TestTerminateRequest,
};
```

Insert immediately before `pub(crate) trait Platform: 'static {` (line 165):

```rust
/// The application's answer when the platform asks whether it may terminate
/// now, for a quit the application did not request through `App::quit`: on
/// macOS, Quit from the Dock or the menu bar, or the session ending at logout,
/// restart or shutdown.
///
/// Answer it once, with [`TerminateReply::allow`] or [`TerminateReply::cancel`],
/// at once or after work of the application's own. It is answered on the main
/// thread, so it is not `Send`. A reply dropped unanswered allows termination:
/// nothing is left to answer it, and refusing would leave the application
/// impossible to quit and hold up logout.
pub struct TerminateReply {
    send: Option<Box<dyn FnOnce(bool)>>,
}

impl TerminateReply {
    #[cfg_attr(
        not(any(target_os = "macos", test, feature = "test-support")),
        allow(dead_code)
    )]
    pub(crate) fn new(send: impl FnOnce(bool) + 'static) -> Self {
        Self {
            send: Some(Box::new(send)),
        }
    }

    /// Lets the platform terminate the application.
    pub fn allow(mut self) {
        self.send(true);
    }

    /// Keeps the application running.
    pub fn cancel(mut self) {
        self.send(false);
    }

    fn send(&mut self, allow: bool) {
        if let Some(send) = self.send.take() {
            send(allow);
        }
    }
}

impl Drop for TerminateReply {
    fn drop(&mut self) {
        self.send(true);
    }
}

```

Insert after `    fn on_reopen(&self, callback: Box<dyn FnMut()>);` (line 234):

```rust
    /// Registers the callback asked before the platform terminates the
    /// application for a quit it did not request through [`Platform::quit`].
    /// Platforms that never ask keep this default and never call it.
    fn on_should_terminate(&self, _callback: Box<dyn FnMut(TerminateReply)>) {}
```

The `cfg_attr` keeps a Linux build without `test-support` free of a dead-code warning: the macOS delegate and the test platform are the only callers of `new`. The Linux and Windows platforms keep the trait default.

In `vendor/gpui/src/app.rs`, insert after the closing `}` of `pub fn quit(&self)` (line 751):

```rust

    /// Registers the answer to the platform asking whether the application may
    /// terminate now, for a quit it did not request through [`App::quit`]: on
    /// macOS, Quit from the Dock or the menu bar, or the session ending at
    /// logout, restart or shutdown. Without a callback the platform terminates
    /// at once.
    ///
    /// The callback answers the [`TerminateReply`], at once or later. While
    /// one reply is unanswered, a further request is refused without calling
    /// the callback. A quit requested through [`App::quit`] is never put to
    /// it. Registering again replaces the previous callback.
    pub fn on_should_terminate(
        &self,
        mut callback: impl FnMut(crate::TerminateReply, &mut App) + 'static,
    ) {
        let this = self.this.clone();
        self.platform.on_should_terminate(Box::new(move |reply| {
            if let Some(app) = this.upgrade() {
                callback(reply, &mut app.borrow_mut());
            }
        }));
    }
```

- [ ] **Step 4: Implement the test platform**

In `vendor/gpui/src/platform/test.rs`, replace line 11:

```rust
pub use platform::{TestScreenCaptureSource, TestScreenCaptureStream};
```

with:

```rust
pub use platform::{
    TerminateAnswer, TestScreenCaptureSource, TestScreenCaptureStream, TestTerminateRequest,
};
```

If only `platform.rs` is changed, the crate fails with `E0365` ("re-export of crate public `TestTerminateRequest`"), because `test.rs` glob-imports `platform::*` at `pub(crate)`.

In `vendor/gpui/src/platform/test/platform.rs`:

Line 13: `    cell::RefCell,` → `    cell::{Cell, RefCell},`

Replace lines 41-42 (`    weak: Weak<Self>,` and the struct's closing `}`) with:

```rust
    weak: Weak<Self>,
    should_terminate: RefCell<Option<Box<dyn FnMut(crate::TerminateReply)>>>,
    terminate_reply_outstanding: Rc<Cell<bool>>,
    quit_requested: Cell<bool>,
}

/// How the application answered a should-terminate request from the test
/// platform.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TerminateAnswer {
    /// Not answered yet.
    #[default]
    Pending,
    /// The application may terminate.
    Allowed,
    /// The application keeps running.
    Cancelled,
}

/// A should-terminate request made by
/// [`TestAppContext::simulate_should_terminate`](crate::TestAppContext::simulate_should_terminate).
#[derive(Clone, Debug, Default)]
pub struct TestTerminateRequest {
    answer: Rc<Cell<TerminateAnswer>>,
}

impl TestTerminateRequest {
    /// The application's answer so far.
    pub fn answer(&self) -> TerminateAnswer {
        self.answer.get()
    }
}
```

Replace lines 122-124 (`            text_system,`, `        })`, `    }`, the end of `TestPlatform::new`) with:

```rust
            text_system,
            should_terminate: Default::default(),
            terminate_reply_outstanding: Default::default(),
            quit_requested: Default::default(),
        })
    }

    /// Asks the application whether it may terminate, deciding as the macOS
    /// delegate's `applicationShouldTerminate:` does.
    pub(crate) fn simulate_should_terminate(&self) -> TestTerminateRequest {
        let request = TestTerminateRequest::default();
        if self.quit_requested.get() {
            request.answer.set(TerminateAnswer::Allowed);
            return request;
        }
        if self.terminate_reply_outstanding.get() {
            request.answer.set(TerminateAnswer::Cancelled);
            return request;
        }
        let callback = self.should_terminate.borrow_mut().take();
        let Some(mut callback) = callback else {
            request.answer.set(TerminateAnswer::Allowed);
            return request;
        };
        self.terminate_reply_outstanding.set(true);
        let outstanding = self.terminate_reply_outstanding.clone();
        let answer = request.answer.clone();
        callback(crate::TerminateReply::new(move |allow| {
            outstanding.set(false);
            answer.set(if allow {
                TerminateAnswer::Allowed
            } else {
                TerminateAnswer::Cancelled
            });
        }));
        self.should_terminate.borrow_mut().get_or_insert(callback);
        request
    }
```

Line 251: `    fn quit(&self) {}` → replace with:

```rust
    fn quit(&self) {
        self.quit_requested.set(true);
    }
```

After line 364 (`    fn on_quit(&self, _callback: Box<dyn FnMut()>) {}`) insert:

```rust

    fn on_should_terminate(&self, callback: Box<dyn FnMut(crate::TerminateReply)>) {
        *self.should_terminate.borrow_mut() = Some(callback);
    }
```

In `vendor/gpui/src/app/test_context.rs`, after the closing `}` of `set_screen_capture_sources` (line 338) insert:

```rust

    /// Simulates the platform asking whether the application may terminate,
    /// as macOS does for Quit from the Dock or the menu bar, or at logout.
    /// The returned request records the application's answer.
    pub fn simulate_should_terminate(&self) -> crate::TestTerminateRequest {
        self.test_platform.simulate_should_terminate()
    }
```

- [ ] **Step 5: Implement the macOS delegate**

In `vendor/gpui/src/platform/mac/platform.rs`:

After the `applicationWillTerminate:` registration (lines 97-100) insert:

```rust
            decl.add_method(
                sel!(applicationShouldTerminate:),
                should_terminate as extern "C" fn(&mut Object, Sel, id) -> NSUInteger,
            );
```

After line 170 (`    quit: Option<Box<dyn FnMut()>>,` in `MacPlatformState`) insert:

```rust
    should_terminate: Option<Box<dyn FnMut(crate::TerminateReply)>>,
    /// Whether a reply handed to `should_terminate` is still unanswered.
    terminate_reply_outstanding: Rc<Cell<bool>>,
    /// Whether the application asked to quit through `Platform::quit`.
    quit_requested: bool,
```

After line 211 (`            quit: None,` in `MacPlatform::new`) insert:

```rust
            should_terminate: None,
            terminate_reply_outstanding: Rc::default(),
            quit_requested: false,
```

In `fn quit(&self)` (line 503), insert as its first statements, before the existing `// Quitting the app causes us…` comment:

```rust
        // The application has decided; `applicationShouldTerminate:` lets
        // this quit through without asking it again.
        self.0.lock().quit_requested = true;

```

After `fn on_reopen` (lines 878-880) insert:

```rust

    fn on_should_terminate(&self, callback: Box<dyn FnMut(crate::TerminateReply)>) {
        self.0.lock().should_terminate = Some(callback);
    }
```

Immediately before `extern "C" fn on_keyboard_layout_change` (line 1431) insert:

```rust
// `NSApplicationTerminateReply` values.
const NS_TERMINATE_CANCEL: NSUInteger = 0;
const NS_TERMINATE_NOW: NSUInteger = 1;
const NS_TERMINATE_LATER: NSUInteger = 2;

extern "C" fn should_terminate(this: &mut Object, _: Sel, _: id) -> NSUInteger {
    let platform = unsafe { get_mac_platform(this) };
    let mut lock = platform.0.lock();
    if lock.quit_requested {
        return NS_TERMINATE_NOW;
    }
    if lock.terminate_reply_outstanding.get() {
        // The reply already handed out answers AppKit's pending request.
        return NS_TERMINATE_CANCEL;
    }
    let Some(mut callback) = lock.should_terminate.take() else {
        return NS_TERMINATE_NOW;
    };
    let outstanding = lock.terminate_reply_outstanding.clone();
    outstanding.set(true);
    drop(lock);
    callback(crate::TerminateReply::new(move |allow| {
        outstanding.set(false);
        reply_to_terminate(allow);
    }));
    platform.0.lock().should_terminate.get_or_insert(callback);
    NS_TERMINATE_LATER
}

/// Sends the answer from the main queue, as `MacPlatform::quit` sends
/// `terminate:`. Allowing termination closes the windows and runs the quit
/// callbacks synchronously, which must not happen while the caller holds a
/// borrow of the app state. It also keeps an answer given inside the callback
/// from arriving before `NSTerminateLater` has been returned.
fn reply_to_terminate(allow: bool) {
    use super::dispatcher::{dispatch_get_main_queue, dispatch_sys::dispatch_async_f};

    unsafe extern "C" fn allow_termination(_: *mut c_void) {
        unsafe {
            let app = NSApplication::sharedApplication(nil);
            let _: () = msg_send![app, replyToApplicationShouldTerminate: YES];
        }
    }

    unsafe extern "C" fn cancel_termination(_: *mut c_void) {
        unsafe {
            let app = NSApplication::sharedApplication(nil);
            let _: () = msg_send![app, replyToApplicationShouldTerminate: NO];
        }
    }

    let answer: unsafe extern "C" fn(*mut c_void) = if allow {
        allow_termination
    } else {
        cancel_termination
    };
    unsafe {
        dispatch_async_f(dispatch_get_main_queue(), ptr::null_mut(), Some(answer));
    }
}

```

`NSUInteger`, `YES`, `NO`, `Cell`, `Rc`, `c_void` and `ptr` are already imported at the top of this file. The `let _: () = msg_send![…]` form is the file's existing idiom for void Objective-C sends. It is not a discarded `must_use` value, and vendored GPUI is outside the workspace lints.

- [ ] **Step 6: Run the contract tests until they pass**

Run: `env PATH=/Users/davidlee/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin:/Users/davidlee/.cargo/bin:/opt/homebrew/bin:/usr/bin:/bin cargo test -p sprite-app --locked --offline --test platform_quit_request`
Expected: `test result: ok. 5 passed`. The only warnings are GPUI's existing two `float_literal_f32_fallback` warnings in `taffy.rs`. On macOS this build also compiles the delegate.

Run: `python3 scripts/check_gpui_patch.py`
Expected: FAIL, `ValueError: local GPUI differs from exact archive + patch: src/app.rs, src/app/test_context.rs, src/platform.rs, src/platform/mac/platform.rs, src/platform/test.rs, src/platform/test/platform.rs`. This is the gate refusing an unrecorded change.

- [ ] **Step 7: Record the change as a second patch**

In `scripts/check_gpui_patch.py`:

Line 2: `"""Verify the local GPUI tree is the published archive plus the recorded patch."""` → `"""Verify the local GPUI tree is the published archive plus the recorded patches."""`

After line 11 (`ROOT = Path(__file__).resolve().parents[1]`) insert:

```python
# Applied in this order; each patch's line numbers are those of the tree the
# patches before it produce.
PATCHES = ("gpui-0.2.2-native-text.patch", "gpui-0.2.2-should-terminate.patch")
```

Replace line 81:

```python
    expected = apply_exact_patch(expected, (ROOT / "patches/gpui-0.2.2-native-text.patch").read_text())
```

with:

```python
    for name in PATCHES:
        expected = apply_exact_patch(expected, (ROOT / "patches" / name).read_text())
```

In `scripts/test_gpui_patch.py`, replace line 7 (`from scripts.check_gpui_patch import ROOT, archive_path, verify`) with:

```python
from unittest import mock
from scripts.check_gpui_patch import PATCHES, ROOT, archive_path, verify
```

and insert before `    def test_cocoa_scope_policy_from_production_source(self):`:

```python
    def test_every_recorded_patch_is_required(self):
        # Without the should-terminate patch, the files it changes no longer match.
        with mock.patch("scripts.check_gpui_patch.PATCHES", PATCHES[:1]):
            with self.assertRaisesRegex(ValueError, "src/platform/mac/platform.rs"):
                verify(ROOT / "vendor/gpui", archive_path())

```

Generate the patch. The generator is a throw-away file, not committed:

```sh
work=$(mktemp -d)
cat > "$work/gen_patch.py" <<'EOF'
"""Write the last recorded GPUI patch: vendor/gpui against the archive plus every earlier patch."""
import difflib
import hashlib
import sys
import tarfile
from pathlib import Path

ROOT = Path(sys.argv[1]).resolve()
sys.path.insert(0, str(ROOT))
from scripts.check_gpui_patch import CHECKSUM, PATCHES, apply_exact_patch, archive_path  # noqa: E402

archive = archive_path()
if hashlib.sha256(archive.read_bytes()).hexdigest() != CHECKSUM:
    raise SystemExit("published GPUI archive checksum mismatch")
with tarfile.open(archive) as source:
    base = {member.name.removeprefix("gpui-0.2.2/"): source.extractfile(member).read()
            for member in source.getmembers() if member.isfile()}
for name in PATCHES[:-1]:
    base = apply_exact_patch(base, (ROOT / "patches" / name).read_text())
tree = ROOT / "vendor/gpui"
lines = []
for path in sorted(str(p.relative_to(tree)) for p in tree.rglob("*") if p.is_file()):
    new = (tree / path).read_bytes()
    old = base.get(path)
    if old == new:
        continue
    if old is None:
        raise SystemExit(f"{path} is new; this patch only edits published files")
    if not old.endswith(b"\n") or not new.endswith(b"\n"):
        raise SystemExit(f"{path} lacks a final newline")
    lines.extend(difflib.unified_diff(old.decode().splitlines(keepends=True),
                                      new.decode().splitlines(keepends=True),
                                      "a/" + path, "b/" + path))
out = ROOT / "patches" / PATCHES[-1]
out.write_text("".join(lines))
print(f"wrote {out.relative_to(ROOT)}: {len(lines)} lines")
EOF
python3 -I "$work/gen_patch.py" /Users/davidlee/Projects/Sprite/.worktrees/close-open-findings
rm -rf "$work"
```

Expected: `wrote patches/gpui-0.2.2-should-terminate.patch: 375 lines` (±a few if comment wrapping differs). The file headers are `--- a/src/…` / `+++ b/src/…` for the six files listed in Step 6.

In `.gitattributes`, after line 3 (`patches/gpui-0.2.2-native-text.patch -whitespace`) add:

```
patches/gpui-0.2.2-should-terminate.patch -whitespace
```

The rule is needed because the unified diff's blank context lines are a single space, and `git diff --check` would otherwise report them.

- [ ] **Step 8: Verify the gate and its tests**

Run: `python3 scripts/check_gpui_patch.py`
Expected: `GPUI archive checksum and exact patched source verified`

Run: `python3 -m unittest scripts.test_gpui_patch -v`
Expected: 4 tests OK, including `test_every_recorded_patch_is_required`.

Run: `/Users/davidlee/.cargo/bin/env PATH=/Users/davidlee/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin:/usr/bin:/bin rustfmt --edition 2024 --check vendor/gpui/src/platform.rs vendor/gpui/src/platform/test.rs vendor/gpui/src/platform/test/platform.rs vendor/gpui/src/platform/mac/platform.rs vendor/gpui/src/app/test_context.rs vendor/gpui/src/app.rs`
Expected: no output.

Run: `env PATH=/Users/davidlee/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin:/Users/davidlee/.cargo/bin:/opt/homebrew/bin:/usr/bin:/bin cargo clippy -p sprite-app --all-targets --locked --offline -- -D warnings`
Expected: clean. The two `taffy.rs` warnings come from GPUI, which the workspace lints do not cover.

- [ ] **Step 9: Provenance and dependency notes**

In `vendor/gpui.provenance.md`, replace lines 11-12:

```
`patches/gpui-0.2.2-native-text.patch` is the complete additive input-origin
patch. `python3 scripts/check_gpui_patch.py` verifies every byte of the local
```

with:

```
`patches/gpui-0.2.2-native-text.patch` is the complete additive input-origin
patch. `patches/gpui-0.2.2-should-terminate.patch` follows it (see "Should-terminate
hook" below); `PATCHES` in the checker lists both in application order.
`python3 scripts/check_gpui_patch.py` verifies every byte of the local
```

After the "Caller inventory" section (before `## Execution limits`) insert:

```
## Should-terminate hook

`patches/gpui-0.2.2-should-terminate.patch` applies to the tree the native-text
patch produces; its line numbers are that tree's. It lets the application
answer macOS before a quit it did not request ends it: Quit from the Dock or
the menu bar, or the session ending at logout, restart or shutdown (ADR 0032).
It edits published files only and adds none.

- `platform.rs`: `TerminateReply`, a one-shot allow/cancel answer. It is not
  `Send`, so it is answered on the main thread. Dropped unanswered, it allows
  termination, which is the behaviour without the hook. `Platform::on_should_terminate`
  defaults to doing nothing, so Linux and Windows are unchanged.
- `app.rs`: `App::on_should_terminate` reaches the app through its weak handle,
  as `Application::on_reopen` does.
- macOS `platform.rs`: the delegate implements `applicationShouldTerminate:`.
  It answers `NSTerminateNow` when no callback is registered or when the app
  asked to quit through `Platform::quit`, `NSTerminateCancel` while a reply is
  still unanswered, and otherwise calls the callback and answers
  `NSTerminateLater`. The reply reaches `replyToApplicationShouldTerminate:`
  from the main queue, as `quit` sends `terminate:`, so termination never runs
  inside a borrow of the app state and never precedes `NSTerminateLater`.
- test `test.rs`, `platform.rs` and `test_context.rs`:
  `TestAppContext::simulate_should_terminate` makes the delegate's decision
  and returns a `TestTerminateRequest` whose `answer()` is `Pending`, `Allowed`
  or `Cancelled`. `TestPlatform::quit` records that the app asked to quit.

The delegate method runs only on a macOS desktop. Hosted macOS CI compiles it;
the manual Dock-quit check recorded in the PR evidence executes it. Tests drive
the test platform, not AppKit.
```

In `DEPENDENCIES.md`, replace lines 250-253:

```
**Local input patch.** The exact published 0.2.2 archive is retained in
`vendor/gpui`, with its Apache license and a narrow recorded native-commit/key-
fallback patch. `scripts/check_gpui_patch.py` verifies the complete source
against the SHA256-pinned archive plus `patches/gpui-0.2.2-native-text.patch`;
```

with:

```
**Local patches.** The exact published 0.2.2 archive is retained in
`vendor/gpui`, with its Apache license, a narrow recorded native-commit/key-
fallback patch, and a macOS should-terminate hook (ADR 0032).
`scripts/check_gpui_patch.py` verifies the complete source against the
SHA256-pinned archive plus `patches/gpui-0.2.2-native-text.patch` and then
`patches/gpui-0.2.2-should-terminate.patch`;
```

- [ ] **Step 10: Commit**

```sh
git add vendor/gpui/src/platform.rs vendor/gpui/src/platform/test.rs vendor/gpui/src/platform/test/platform.rs \
  vendor/gpui/src/platform/mac/platform.rs vendor/gpui/src/app.rs vendor/gpui/src/app/test_context.rs \
  patches/gpui-0.2.2-should-terminate.patch scripts/check_gpui_patch.py scripts/test_gpui_patch.py \
  .gitattributes vendor/gpui.provenance.md DEPENDENCIES.md crates/sprite-app/tests/platform_quit_request.rs
git commit -F - <<'EOF'
Let GPUI ask the app before macOS ends it

The macOS delegate now implements applicationShouldTerminate:. With a
callback registered it answers NSTerminateLater and hands the app a
one-shot TerminateReply; the answer reaches AppKit from the main queue.
A quit the app requested through App::quit is not asked about, a request
while a reply is unanswered is refused, and a dropped reply allows.

The test platform simulates the same request. The change is recorded as a
second patch that check_gpui_patch.py replays after the native-text one.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
EOF
```

---

---

### Task 7: Sprite answers every Quit Request with the quit key's decision (R-Q4, R-Q5)

**Files:**
- Modify: `crates/sprite-app/src/workspace/close_gate.rs:9-13` (`quit_app`), `:129-143` (`shutdown_and_quit`), insert after `:207` (end of `quit`), tests inserted after `:735` (end of `shortcut_quit_keeps_window_until_prior_and_current_cleanup_finish`)
- Modify: `crates/sprite-app/src/workspace/mod.rs:118` (field), `:325` (initialiser)
- Modify: `crates/sprite-app/src/main.rs:122-129`

**Interfaces:**
- Consumes (Q1): `gpui::TerminateReply::{allow, cancel}`, `App::on_should_terminate`, `TestAppContext::simulate_should_terminate`, `gpui::TerminateAnswer`. Also, from `close_gate.rs`: `may_close(CloseScope::Quit, …)`, `stopping`, `begin_shutdown`, and the test helpers `gated_panes`, `release`, `BusyPane` and `QUIT_REQUESTS`.
- Produces:
  ```rust
  impl Workspace {
      pub fn answer_platform_quit_requests(workspace: &gpui::Entity<Self>, cx: &mut gpui::App);
      fn answer_quit_request(&mut self, reply: gpui::TerminateReply, cx: &mut Context<Self>);
  }
  fn quit_app(reply: Option<gpui::TerminateReply>, cx: &mut gpui::App); // was quit_app(cx)
  // Workspace field:
  quit_request: Option<gpui::TerminateReply>,
  ```

- [ ] **Step 1: Write the failing tests**

In `close_gate.rs`'s `mod tests`, after `shortcut_quit_keeps_window_until_prior_and_current_cleanup_finish` (ends line 735), insert:

```rust

    /// A Quit from the Dock, the menu bar or logout, with nothing running,
    /// lets the platform end Sprite only once every pane cleanup has finished.
    #[gpui::test]
    fn a_platform_quit_request_is_allowed_only_after_pane_cleanup(cx: &mut gpui::TestAppContext) {
        let (workspace, cx) = test_workspace(cx);
        let (completed, gates) = gated_panes(&workspace, cx);
        cx.update(|_, cx| Workspace::answer_platform_quit_requests(&workspace, cx));
        let request = cx.simulate_should_terminate();
        cx.run_until_parked();
        assert_eq!(
            request.answer(),
            gpui::TerminateAnswer::Pending,
            "allowed before cleanup"
        );
        release(&workspace, cx, &gates, 0);
        assert_eq!(
            request.answer(),
            gpui::TerminateAnswer::Pending,
            "allowed with a cleanup still running"
        );
        release(&workspace, cx, &gates, 1);
        assert_eq!(completed.lock().unwrap().len(), 2);
        assert_eq!(request.answer(), gpui::TerminateAnswer::Allowed);
        assert_eq!(QUIT_REQUESTS.with(|requests| requests.get()), 1);
    }

    /// With a program running, a Quit Request from the platform puts the quit
    /// question on screen and is refused at once, so a logout never waits on
    /// a question nobody has seen. The next request answers the question and
    /// is allowed once pane cleanup has finished.
    #[gpui::test]
    fn a_platform_quit_request_with_a_busy_pane_asks_then_the_next_one_quits(
        cx: &mut gpui::TestAppContext,
    ) {
        use gpui::AppContext;
        let (workspace, cx) = test_workspace(cx);
        let (completed, gates) = gated_panes(&workspace, cx);
        workspace.update(cx, |workspace, cx| {
            workspace.tabs.split(Orientation::Horizontal, |_, _| {
                Rc::new(cx.new(|cx| BusyPane {
                    focus: cx.focus_handle(),
                })) as Rc<dyn PaneHandle<Request = SurfaceRequest>>
            });
            workspace.refresh_layout(cx);
        });
        cx.update(|_, cx| Workspace::answer_platform_quit_requests(&workspace, cx));
        let first = cx.simulate_should_terminate();
        cx.run_until_parked();
        assert_eq!(
            first.answer(),
            gpui::TerminateAnswer::Cancelled,
            "a busy pane must refuse the first request"
        );
        workspace.read_with(cx, |workspace, _| {
            assert!(
                matches!(workspace.mode, Mode::ConfirmingClose(_)),
                "the quit question is not on screen"
            );
            assert!(!workspace.stopping, "cleanup began before the answer");
        });
        let second = cx.simulate_should_terminate();
        cx.run_until_parked();
        assert_eq!(
            second.answer(),
            gpui::TerminateAnswer::Pending,
            "allowed before cleanup"
        );
        release(&workspace, cx, &gates, 0);
        release(&workspace, cx, &gates, 1);
        assert_eq!(completed.lock().unwrap().len(), 2);
        assert_eq!(second.answer(), gpui::TerminateAnswer::Allowed);
        assert_eq!(QUIT_REQUESTS.with(|requests| requests.get()), 1);
    }
```

The busy test uses the two gated `ShutdownPane`s from `gated_panes` plus one `BusyPane`. `BusyPane::begin_shutdown` returns `None`, and `shut_down` returns before it pops a gate, so gate and thread index 0 and 1 still belong to panes 0 and 1. The second request is therefore genuinely held by cleanup.

- [ ] **Step 2: Run to verify failure (compile)**

Run: `env PATH=/Users/davidlee/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin:/Users/davidlee/.cargo/bin:/opt/homebrew/bin:/usr/bin:/bin cargo test -p sprite-app --locked --offline --lib a_platform_quit_request`
Expected: `error[E0599]: no associated function or constant named answer_platform_quit_requests found for struct workspace::Workspace`.

- [ ] **Step 3: Add the seam with today's behaviour and see the real red**

Temporarily add, after `quit` (line 207), a registration that does what the platform does at `03b4929`, which is to let the quit go ahead at once:

```rust

    pub fn answer_platform_quit_requests(_workspace: &gpui::Entity<Self>, cx: &mut gpui::App) {
        cx.on_should_terminate(|reply, _| drop(reply));
    }
```

Run the Step 2 command again. Expected: both tests FAIL for the bug's reason:
- `a_platform_quit_request_is_allowed_only_after_pane_cleanup`: `assertion left == right failed: allowed before cleanup`, `left: Allowed`, `right: Pending`.
- `a_platform_quit_request_with_a_busy_pane_asks_then_the_next_one_quits`: `assertion left == right failed: a busy pane must refuse the first request`, `left: Allowed`, `right: Cancelled`.

Delete the temporary function before Step 4.

- [ ] **Step 4: Implement**

`crates/sprite-app/src/workspace/mod.rs`. After line 118 (`    stopping: bool,`) insert:

```rust
    /// The platform's Quit Request this window answers when its pane cleanup
    /// finishes: a Quit from the Dock or the menu bar, or the session ending.
    quit_request: Option<gpui::TerminateReply>,
```

After line 325 (`            stopping: false,`) insert:

```rust
            quit_request: None,
```

`crates/sprite-app/src/workspace/close_gate.rs`. Replace lines 9-13 (`fn quit_app`) with:

```rust
/// Ends Sprite once pane cleanup has finished: by allowing the platform's
/// Quit Request when one is waiting, otherwise by asking the platform to quit.
fn quit_app(reply: Option<gpui::TerminateReply>, cx: &mut gpui::App) {
    #[cfg(test)]
    QUIT_REQUESTS.with(|requests| requests.set(requests.get() + 1));
    match reply {
        Some(reply) => reply.allow(),
        None => cx.quit(),
    }
}
```

In `shutdown_and_quit`, replace lines 129-143 (from `let cleanups = self.begin_shutdown(cx);` through `.detach();`) with:

```rust
        let cleanups = self.begin_shutdown(cx);
        if cleanups.is_empty() {
            let reply = self.quit_request.take();
            quit_app(reply, cx);
            return;
        }
        let finished = cx.background_executor().spawn(async move {
            for cleanup in cleanups {
                cleanup.await;
            }
        });
        cx.spawn(async move |workspace, cx| {
            finished.await;
            // Taken only now: a Quit Request that arrived while cleanup ran
            // is answered by this quit.
            let reply = workspace
                .update(cx, |workspace, _| workspace.quit_request.take())
                .ok()
                .flatten();
            cx.update(|cx| quit_app(reply, cx)).ok();
        })
        .detach();
```

After `quit` (ends line 207) insert:

```rust

    /// Sends every Quit Request the platform asks about through the quit
    /// key's decision, so a Quit from the Dock or the menu bar, or the session
    /// ending, asks about running programs and waits for pane cleanup.
    ///
    /// Public because start-up lives in the `sprite` binary rather than in
    /// this library.
    pub fn answer_platform_quit_requests(workspace: &gpui::Entity<Self>, cx: &mut gpui::App) {
        let workspace = workspace.downgrade();
        cx.on_should_terminate(move |reply, cx| {
            // A window already gone has no panes left to clean up. Its reply
            // is dropped unanswered, which lets the quit go ahead.
            workspace
                .update(cx, |workspace, cx| workspace.answer_quit_request(reply, cx))
                .ok();
        });
    }

    /// Answers one Quit Request from the platform.
    ///
    /// With nothing running, or when this request answers the quit question
    /// already on screen, the reply allows the quit once every pane cleanup
    /// has finished. With a program running and no question yet, the
    /// question goes on screen and the reply refuses at once: a logout is
    /// never left waiting on a question nobody has seen, and the next Quit
    /// Request answers it.
    fn answer_quit_request(&mut self, reply: gpui::TerminateReply, cx: &mut Context<Self>) {
        if self.stopping {
            // Cleanup is already under way and ends in a quit, which answers
            // this request. GPUI hands out one reply at a time; were there an
            // earlier one, refusing it keeps it from ending Sprite first.
            if let Some(earlier) = self.quit_request.replace(reply) {
                earlier.cancel();
            }
            return;
        }
        // A click in the Dock or the menu bar, or the system, is never a held key.
        if self.may_close(CloseScope::Quit, false, cx) {
            self.quit_request = Some(reply);
            self.shutdown_and_quit(cx);
        } else {
            reply.cancel();
        }
    }
```

How each path ends:

- **Idle.** `may_close` → `true`, the reply is stored, and `shutdown_and_quit` starts cleanup. When cleanup completes, the stored reply is taken and `reply.allow()` sends `YES`. `cx.quit()` is not called, so the delegate is not entered twice.
- **Busy, no question on screen.** `may_close` → `false` and puts up `ConfirmingClose(Quit)`, then `reply.cancel()` sends `NO` at once.
- **Busy, question on screen.** `Confirmation::answer(&Quit, false)` → `true`, then the same as the idle path. A Dock Quit therefore answers a quit-key question, and the other way round, because both use `CloseScope::Quit`.
- **Already stopping** (the quit key or close button started cleanup first). The reply is stored, and the in-flight completion answers it instead of calling `cx.quit()`.
- **Quit key or close button.** `shutdown_and_quit` with no stored reply ends in `cx.quit()`. The delegate sees `quit_requested` and answers `NSTerminateNow` (R-Q2).

`crates/sprite-app/src/main.rs`, replace lines 122-129 with:

```rust
        window
            .update(cx, |_view, window, cx| {
                let view = cx.entity();
                Workspace::answer_platform_quit_requests(&view, cx);
                window.on_window_should_close(cx, move |_window, cx| {
                    view.update(cx, |view, cx| view.close_window(cx))
                });
            })
            .expect("install the close and quit handlers");
```

On Linux, `on_should_terminate` keeps the trait default, so registering the callback does nothing and Linux behaviour is unchanged.

- [ ] **Step 4a: The quit question says "quit again"**

A Dock or menu Quit repeats the request without keys, so the instruction for `CloseScope::Quit` must not name keys (PRD R-Q4b). In `crates/sprite-app/src/workspace/close_gate.rs`, `CloseScope::again` (:343-349) becomes:

```rust
    pub(super) fn again(self) -> &'static str {
        match self {
            Self::Pane | Self::Tab => "press the same keys again",
            Self::Window => "click close again",
            Self::Quit => "quit again",
        }
    }
```

In the same file's tests, line 560 becomes:

```rust
        assert_eq!(CloseScope::Quit.again(), "quit again");
```

Then run `grep -rn "press the same keys again" crates/sprite-app/src` and update any other assertion on the Quit text. Only the Pane and Tab uses may remain.

- [ ] **Step 5: Run the tests until they pass**

Run: `env PATH=/Users/davidlee/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin:/Users/davidlee/.cargo/bin:/opt/homebrew/bin:/usr/bin:/bin cargo test -p sprite-app --locked --offline --lib close_gate`
Expected: `test result: ok. 17 passed`. That is the two new tests plus the 15 existing ones, so the quit key and close-button paths are unchanged.

Run: `env PATH=/Users/davidlee/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin:/Users/davidlee/.cargo/bin:/opt/homebrew/bin:/usr/bin:/bin cargo test -p sprite-app --locked --offline --test platform_quit_request`
Expected: 5 passed.

Run: `env PATH=/Users/davidlee/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin:/Users/davidlee/.cargo/bin:/opt/homebrew/bin:/usr/bin:/bin cargo fmt --all -- --check && env PATH=/Users/davidlee/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin:/Users/davidlee/.cargo/bin:/opt/homebrew/bin:/usr/bin:/bin cargo clippy --workspace --all-targets --locked --offline -- -D warnings`
Expected: clean for everything this task touches. A `chunks_exact_mut` error in `sprite-term` means Homebrew's clippy 0.1.98 ran instead of the pinned 0.1.97: check the command's `PATH` (see Global Constraints). The pinned clippy is clean at `03b4929`.

Run: `env PATH=/Users/davidlee/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin:/Users/davidlee/.cargo/bin:/opt/homebrew/bin:/usr/bin:/bin cargo test -p sprite-app --locked --offline --no-fail-fast`
Expected: all pass (707 lib tests in the scratch run, plus the integration tests). The change to `TestPlatform::quit` affects no existing test.

- [ ] **Step 6: Commit**

```sh
git add crates/sprite-app/src/workspace/close_gate.rs crates/sprite-app/src/workspace/mod.rs crates/sprite-app/src/main.rs
git commit -F - <<'EOF'
Answer Dock, menu and logout quits through the in-app quit path

Sprite registers GPUI's should-terminate callback at start-up and gives a
platform Quit Request the quit key's decision. With a program running and
no question yet, the question goes on screen and the request is refused
at once, so a logout never waits on an unseen question. Otherwise the
request is allowed only after every pane cleanup has run its hangup,
terminate and kill escalation.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
EOF
```

- [ ] **Step 7: Manual evidence on a macOS desktop (R-Q5)**

This is the only step that runs the real `applicationShouldTerminate:` and the main-queue reply under AppKit's modal wait. Build a bundle from this checkout. It goes into `target/` and does not replace `/Applications/Sprite.app`.

```sh
cd /Users/davidlee/Projects/Sprite/.worktrees/close-open-findings
git rev-parse HEAD
env PATH=/Users/davidlee/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin:/Users/davidlee/.cargo/bin:/opt/homebrew/bin:/usr/bin:/bin cargo build --release -p sprite-app --locked --offline
packaging/prepare.sh                       # target/ghostty.terminfo (needs zig)
OUT=target/SpriteQuitCheck.app packaging/macos/bundle.sh
pgrep -fl 'MacOS/sprite' || echo "no Sprite running"   # quit any other Sprite first: same bundle id
open -n target/SpriteQuitCheck.app
```

Run A, a busy pane through the Dock (R-Q4 busy path):

1. In the Sprite pane run a child that ignores the hangup, so only the TERM/KILL escalation can end it:
   `sh -c 'trap "" HUP; exec sleep 1000'`
2. In Terminal.app: `pgrep -fl "sleep 1000"`. Expected: one line.
3. Right-click Sprite in the Dock → Quit. Expected: Sprite stays open, the window stays responsive, and the red banner reads `sleep is running — quit again to close this window, Esc to keep it`.
4. Right-click Sprite in the Dock → Quit again. Expected: Sprite exits within about 6 s (`GIVE_UP_AFTER`).
5. `pgrep -fl "sleep 1000"; echo "exit=$?"` → no lines, `exit=1`. Then `pgrep -fl SpriteQuitCheck; echo "exit=$?"` → `exit=1`.

Run B, the quit key is not intercepted a second time (R-Q2): `open -n target/SpriteQuitCheck.app`, repeat step 1, then press ⌘Q twice. Expected: Sprite exits, and `pgrep -fl "sleep 1000"` is empty. A Sprite that stays open after cleanup would mean the delegate asked about its own quit again.

Run C, idle through the Dock: `open -n target/SpriteQuitCheck.app` and, with no program running, Dock → Quit once. Expected: Sprite exits.

Afterwards `rm -rf target/SpriteQuitCheck.app`. Record for the evidence task the commit hash, the commands, and the `pgrep` output with exit codes for runs A to C. Logout was not exercised by hand. It uses the same delegate method and Sprite's same decision (ADR 0032).

---

### Task 8: Placeholder tiles are placed at paint time (R-G; GAP-C08, #68)

**Depends on Task 6.** Q1 adds the `PATCHES` list to `scripts/check_gpui_patch.py`, the second patch `patches/gpui-0.2.2-should-terminate.patch`, the generator, and the provenance and DEPENDENCIES text that this task appends to. Do Q1 first. This task adds a third GPUI patch on top of Q1's.

**The seam the test crosses: a display-scale change.** Taffy rounds every layout position to whole device pixels (`vendor/gpui/src/taffy.rs:41` `enable_rounding`; taffy 0.9.0 `round_layout_inner` rounds each node's location with `f32::round`, `util/sys.rs:59`). So at a fixed scale, the grid's corner always moves between frames by whole device pixels. `tile_geometry`'s offsets (`snap(origin + n·cell) − origin`) do not change under such a move, so a drag alone cannot misplace a tile. The seam appears when the scale changes. A corner on a half logical pixel is a whole device pixel at 2x, but it is not a whole pixel at 1x. At 1x, render at `03b4929` still snaps against that 2x corner, and the frame after the change paints tiles off their cells. Nothing in an unfocused pane redraws it.

**What a real scale change does, and what the test does.** On macOS, a backing-properties change (`vendor/gpui/src/platform/mac/window.rs:2096-2119` `view_did_change_backing_properties`) calls the window's resize callback with the content size and the new scale. GPUI's callback (`window.rs:1067-1074`) runs `Window::bounds_changed` (`window.rs:1675-1684`). That reads the platform's scale factor and viewport, calls `self.refresh()`, and runs the bounds observers. The platform then draws one frame. The GPUI test window has no such path: `TestWindow::scale_factor` returns the constant `2.0` (`platform/test/window.rs:136-138`). This task's GPUI patch stores the scale in the test window's state, and adds `TestWindow::simulate_scale_factor`, which calls the same resize callback with (current size, new scale), as `simulate_resize` does (`platform/test/window.rs:80-90`). It is exposed as `TestAppContext::simulate_window_scale_factor` and `VisualTestContext::simulate_scale_factor`, like `simulate_window_resize` and `simulate_resize` (`app/test_context.rs:329-332, 803-806`).

The test therefore makes no draw call and no `window.refresh()` of its own after the scale change. Under `test-support`, GPUI ends every effect flush by drawing each window its invalidator marks dirty (`vendor/gpui/src/app.rs:1236-1249`). `bounds_changed`'s own `refresh()` marks the window dirty, so the callback's update draws exactly the frame the platform would draw. The test then reads what that frame painted. `Entity::read_with` on a `VisualTestContext` only borrows the app (`TestAppContext::read_entity`). It does not flush, so it cannot draw a second frame.

**Arithmetic: why the default layout crosses the seam, with no divider drag needed.** These are the actual values in a test window:
- **Panes.** `simulate_resize(1000 × 700)`, one tab: `pane_area` reserves no tab strip when there is only one tab (`workspace/mod.rs:424-431`). After one horizontal split at 0.5, both panes sit at y = 0 (`rect.y · 700`). Each is allocated `0.5 · 1000 − 1 = 499` × `1 · 700 − 1 = 699` (`pane_allocation`, `DIVIDER_PX = 1`).
- **Cells.** Cell height is `Font::cell_height(14, 8/7) = round(16.0) = 16`. Cell width under the test platform's `NoopTextSystem` (`platform/test/platform.rs:105`; `platform.rs:648-676`) is `14 · 600 / 1000 = 8.4`. Neither depends on the scale.
- **Grid size and corner within the pane.** Padding is 8: rows `= floor((699 − 16) / 16) = floor(42.69) = 42`, and columns `= floor((499 − 16) / 8.4) = floor(57.5) = 57`. `grid_origin` centres the grid in the pane (`grid.rs:513-534`): y `= (699 − 42·16) / 2 = 27 / 2 = 13.5`, and x `= (499 − 57·8.4) / 2 ≈ 10.1`.
- **At 2x.** GPUI hands taffy device pixels (`taffy.rs:357-362`, `pixels · scale_factor`). The grid box's `top(13.5)` is 27 device pixels, which rounds to 27, so the corner is at y = 13.5 logical. `left(10.1)` is 20.2, which rounds to 20, so x = 10.0 logical (510.0 for the right-hand pane). The input canvas's prepaint stores this as `content_origin`.
- **At 1x, the next frame.** `top(13.5)` is 13.5 device pixels, which rounds to 14, so the corner is at y = 14. `left(10.1)` rounds to x = 10 (510 for the right pane).
- **At `03b4929`,** that frame's render snaps against the stale corner y = 13.5. `row_edge(13.5, 16, Row(0), 1.0) = round(13.5) = 14`, so `tile.top = 14 − 13.5 = 0.5`. Taffy rounds the tile `div`'s location 0.5 to 1, so row 0 is painted at y = 15 while its cell is at y = 14. On x the stale and new corners are both whole pixels, so the columns agree. The test fails by one pixel on the y axis.

**Files:**
- Modify (GPUI, recorded as the third patch): `vendor/gpui/src/platform/test/window.rs:15-32` (state), `:60-77` (`TestWindow::new`), after `:90` (`simulate_resize`), `:136-138` (`scale_factor`); `vendor/gpui/src/app/test_context.rs`, after `simulate_window_resize` (`:329-332` at `03b4929`; Q1 inserts only after `:338`) and after `VisualTestContext::simulate_resize` (`:803-806` at `03b4929`)
- Create: `patches/gpui-0.2.2-test-scale-factor.patch` (generated in Step 2)
- Modify: `scripts/check_gpui_patch.py` (Q1's `PATCHES` gains a third entry), `scripts/test_gpui_patch.py` (one more test after Q1's `test_every_recorded_patch_is_required`), `.gitattributes` (after Q1's line for the should-terminate patch), `vendor/gpui.provenance.md` (Q1's patch list sentence; a new section after Q1's "Should-terminate hook"), `DEPENDENCIES.md` (Q1's "Local patches" paragraph)
- Modify: `crates/sprite-app/src/terminal_view.rs`: `TileLog` alias above `pub struct TerminalView` (`:72`); new field after `content_origin` (`:157`); field initialised next to `content_origin: None,` at `:544` and `:633`; three test accessors after `bundle_for_test` (`:958-961`)
- Modify: `crates/sprite-app/src/terminal_view/render.rs:80-121` (`placeholder_element` is replaced by `PlaceholderTile` and `placeholder_tiles`), `:173-240` (`image_layers`), `:341-358` (`Render::render`: image layers and the stale-origin comment), `:400-404` (`grid_box` children)
- Test: append to `crates/sprite-app/src/workspace/layout_tests.rs` (after `:479`)
- Unchanged: `terminal_view/placeholder.rs` (`tile_geometry`, `ImageCell`, `fit_image` keep their signatures); `terminal_view/input.rs:141` still reads `content_origin`, which the input canvas's prepaint (`render.rs:429-434`) still writes.

**Interfaces:**
- Produces (GPUI, `test-support`): `TestWindow::simulate_scale_factor(&mut self, scale_factor: f32)`; `TestAppContext::simulate_window_scale_factor(&self, window_handle: AnyWindowHandle, scale_factor: f32)`; `VisualTestContext::simulate_scale_factor(&self, scale_factor: f32)`. `TestWindow::scale_factor` returns the stored value, which starts at `2.0`, so every existing test sees what it sees today.
- Consumes (GPUI 0.2.2, verified in `vendor/gpui`): `gpui::canvas<T>(prepaint: impl 'static + FnOnce(Bounds<Pixels>, &mut Window, &mut App) -> T, paint: impl 'static + FnOnce(Bounds<Pixels>, T, &mut Window, &mut App)) -> Canvas<T>`, where `Canvas<T>: Styled` (`elements/canvas.rs:10-19, 116-120`); `Window::paint_image(&mut self, bounds: Bounds<Pixels>, corner_radii: Corners<Pixels>, data: Arc<RenderImage>, frame_index: usize, grayscale: bool) -> anyhow::Result<()>` (`window.rs:3129`; paint phase only); `Window::with_content_mask<R>(&mut self, mask: Option<ContentMask<Pixels>>, f: impl FnOnce(&mut Self) -> R) -> R` (`window.rs:2384`); `Window::scale_factor(&self) -> f32`. Also `super::placeholder::{ImageCell, ImageFit, fit_image, tile_geometry}` and `crate::grid::{Col, Row, column_edge, row_edge}`. From Q1: `PATCHES` and `apply_exact_patch` in `scripts/check_gpui_patch.py`.
- Produces (Sprite): `pub(super) struct PlaceholderTile` (render.rs); `TerminalView::image_layers(&self, rows: &[Arc<Vec<PositionedCell>>], cell_width: Pixels, cell_height: Pixels) -> ([Vec<gpui::Div>; 3], Vec<PlaceholderTile>)`, which drops the `grid_origin` and `scale` parameters; private `fn placeholder_tiles(tiles: Vec<PlaceholderTile>, cell_width: Pixels, cell_height: Pixels, #[cfg(test)] painted: TileLog) -> impl IntoElement`. Test-only, `#[cfg(test)]`: `pub(crate) type TileLog = Rc<RefCell<Vec<(u16, usize, gpui::Bounds<Pixels>)>>>`; `TerminalView::{painted_tiles_for_test, content_origin_for_test, cell_bounds_for_test}`.

**How painted tiles are observed.** A `debug_selector` cannot be used. GPUI records debug bounds only from a `div`'s own paint (`elements/div.rs:1801-1804`, into the crate-private `next_frame.debug_bounds`), and after this task the tiles are no longer divs. Instead the view gets a `#[cfg(test)]` log, `painted_tiles: TileLog`. Render clears it each frame, and every tile appends `(column, row, bounds)` with the window bounds it was painted at. Before the fix, Step 5 records a tile's bounds from a probe canvas inside each tile `div`. After the fix the tile canvas records the tile's clip bounds as it paints.

- [ ] **Step 1: Give GPUI's test window a settable scale factor**

In `vendor/gpui/src/platform/test/window.rs`:

In `TestWindowState`, after `    is_fullscreen: bool,` (`:31`) insert:

```rust
    scale_factor: f32,
```

In `TestWindow::new`, after `            is_fullscreen: false,` (`:76`) insert:

```rust
            scale_factor: 2.0,
```

After the closing `}` of `pub fn simulate_resize` (`:90`) insert:

```rust

    /// Moves the window to a display with another scale factor. As macOS does
    /// when the window's backing properties change, the resize callback hears
    /// the unchanged content size with the new scale.
    pub fn simulate_scale_factor(&mut self, scale_factor: f32) {
        let mut lock = self.0.lock();
        lock.scale_factor = scale_factor;
        let Some(mut callback) = lock.resize_callback.take() else {
            return;
        };
        let size = lock.bounds.size;
        drop(lock);
        callback(size, scale_factor);
        self.0.lock().resize_callback = Some(callback);
    }
```

Replace `:136-138`:

```rust
    fn scale_factor(&self) -> f32 {
        2.0
    }
```

with:

```rust
    fn scale_factor(&self) -> f32 {
        self.0.lock().scale_factor
    }
```

`simulate_resize` reads `self.scale_factor()` before it takes the lock, so it does not deadlock, and it now reports the stored scale.

In `vendor/gpui/src/app/test_context.rs`, after the closing `}` of `pub fn simulate_window_resize` (in `impl TestAppContext`) insert:

```rust

    /// Simulates the window moving to a display with another scale factor.
    pub fn simulate_window_scale_factor(&self, window_handle: AnyWindowHandle, scale_factor: f32) {
        self.test_window(window_handle).simulate_scale_factor(scale_factor);
    }
```

and after the closing `}` of `pub fn simulate_resize` (in `impl VisualTestContext`) insert:

```rust

    /// Simulates the window moving to a display with another scale factor.
    pub fn simulate_scale_factor(&self, scale_factor: f32) {
        self.simulate_window_scale_factor(self.window, scale_factor)
    }
```

Run: `/Users/davidlee/.cargo/bin/env PATH=/Users/davidlee/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin:/usr/bin:/bin rustfmt --edition 2024 --check vendor/gpui/src/platform/test/window.rs vendor/gpui/src/app/test_context.rs`
Expected: no output. If rustfmt reports anything, apply its layout before you generate the patch: the patch must record formatted source.

Run: `python3 scripts/check_gpui_patch.py`
Expected: FAIL, `ValueError: local GPUI differs from exact archive + patch: src/app/test_context.rs, src/platform/test/window.rs`.

- [ ] **Step 2: Record the change as the third patch**

In `scripts/check_gpui_patch.py`, replace Q1's

```python
PATCHES = ("gpui-0.2.2-native-text.patch", "gpui-0.2.2-should-terminate.patch")
```

with:

```python
PATCHES = (
    "gpui-0.2.2-native-text.patch",
    "gpui-0.2.2-should-terminate.patch",
    "gpui-0.2.2-test-scale-factor.patch",
)
```

In `scripts/test_gpui_patch.py`, insert after Q1's `test_every_recorded_patch_is_required`:

```python
    def test_the_test_scale_factor_patch_is_required(self):
        # Without the third patch, the test window's files no longer match.
        with mock.patch("scripts.check_gpui_patch.PATCHES", PATCHES[:2]):
            with self.assertRaisesRegex(ValueError, "src/platform/test/window.rs"):
                verify(ROOT / "vendor/gpui", archive_path())

```

Generate the patch with the same throw-away generator as Q1 Step 7, not committed. It diffs `vendor/gpui` against the archive plus every patch except the last, and writes the last patch named in `PATCHES`:

```sh
work=$(mktemp -d)
cat > "$work/gen_patch.py" <<'EOF'
"""Write the last recorded GPUI patch: vendor/gpui against the archive plus every earlier patch."""
import difflib
import hashlib
import sys
import tarfile
from pathlib import Path

ROOT = Path(sys.argv[1]).resolve()
sys.path.insert(0, str(ROOT))
from scripts.check_gpui_patch import CHECKSUM, PATCHES, apply_exact_patch, archive_path  # noqa: E402

archive = archive_path()
if hashlib.sha256(archive.read_bytes()).hexdigest() != CHECKSUM:
    raise SystemExit("published GPUI archive checksum mismatch")
with tarfile.open(archive) as source:
    base = {member.name.removeprefix("gpui-0.2.2/"): source.extractfile(member).read()
            for member in source.getmembers() if member.isfile()}
for name in PATCHES[:-1]:
    base = apply_exact_patch(base, (ROOT / "patches" / name).read_text())
tree = ROOT / "vendor/gpui"
lines = []
for path in sorted(str(p.relative_to(tree)) for p in tree.rglob("*") if p.is_file()):
    new = (tree / path).read_bytes()
    old = base.get(path)
    if old == new:
        continue
    if old is None:
        raise SystemExit(f"{path} is new; this patch only edits published files")
    if not old.endswith(b"\n") or not new.endswith(b"\n"):
        raise SystemExit(f"{path} lacks a final newline")
    lines.extend(difflib.unified_diff(old.decode().splitlines(keepends=True),
                                      new.decode().splitlines(keepends=True),
                                      "a/" + path, "b/" + path))
out = ROOT / "patches" / PATCHES[-1]
out.write_text("".join(lines))
print(f"wrote {out.relative_to(ROOT)}: {len(lines)} lines")
EOF
python3 -I "$work/gen_patch.py" /Users/davidlee/Projects/Sprite/.worktrees/close-open-findings
rm -rf "$work"
```

Expected: `wrote patches/gpui-0.2.2-test-scale-factor.patch: N lines`, with N about 70. The file has exactly two `--- a/…` headers: `src/app/test_context.rs` and `src/platform/test/window.rs`. If it names any of Q1's files, Q1's patch does not match `vendor/gpui`. Stop and fix that first.

In `.gitattributes`, after Q1's `patches/gpui-0.2.2-should-terminate.patch -whitespace` add:

```
patches/gpui-0.2.2-test-scale-factor.patch -whitespace
```

Run: `python3 scripts/check_gpui_patch.py`
Expected: `GPUI archive checksum and exact patched source verified`

Run: `python3 -m unittest scripts.test_gpui_patch -v`
Expected: 5 tests OK, including `test_every_recorded_patch_is_required` and `test_the_test_scale_factor_patch_is_required`.

- [ ] **Step 3: Provenance and dependency notes**

In `vendor/gpui.provenance.md`, replace Q1's sentence

```
patch. `patches/gpui-0.2.2-should-terminate.patch` follows it (see "Should-terminate
hook" below); `PATCHES` in the checker lists both in application order.
```

with:

```
patch. `patches/gpui-0.2.2-should-terminate.patch` follows it (see "Should-terminate
hook" below), and `patches/gpui-0.2.2-test-scale-factor.patch` follows that (see
"Test window scale factor" below); `PATCHES` in the checker lists all three in
application order.
```

After Q1's "Should-terminate hook" section (before `## Execution limits`) insert:

```
## Test window scale factor

`patches/gpui-0.2.2-test-scale-factor.patch` applies to the tree the
should-terminate patch produces; its line numbers are that tree's. It changes
the test platform only, so no shipped code path changes.

- test `window.rs`: the test window keeps its scale factor in its state,
  starting at the 2.0 it always reported, and
  `TestWindow::simulate_scale_factor` sets it and calls the resize callback
  with the unchanged content size and the new scale. That is what the macOS
  window does when its backing properties change
  (`view_did_change_backing_properties`), and GPUI answers both with
  `Window::bounds_changed`, which reads the new scale and refreshes the window.
- `test_context.rs`: `TestAppContext::simulate_window_scale_factor` and
  `VisualTestContext::simulate_scale_factor`, beside the existing resize
  simulations.

Sprite uses it to test that Kitty placeholder tiles stay on their cells when a
window moves between displays of different scale.
```

In `DEPENDENCIES.md`, in Q1's "Local patches" paragraph, replace

```
fallback patch, and a macOS should-terminate hook (ADR 0032).
`scripts/check_gpui_patch.py` verifies the complete source against the
SHA256-pinned archive plus `patches/gpui-0.2.2-native-text.patch` and then
`patches/gpui-0.2.2-should-terminate.patch`;
```

with:

```
fallback patch, a macOS should-terminate hook (ADR 0032), and a test-platform
scale-factor simulation. `scripts/check_gpui_patch.py` verifies the complete
source against the SHA256-pinned archive plus, in order,
`patches/gpui-0.2.2-native-text.patch`,
`patches/gpui-0.2.2-should-terminate.patch` and
`patches/gpui-0.2.2-test-scale-factor.patch`;
```

- [ ] **Step 4: Commit the GPUI test hook**

```bash
git -C /Users/davidlee/Projects/Sprite/.worktrees/close-open-findings add vendor/gpui/src/platform/test/window.rs vendor/gpui/src/app/test_context.rs patches/gpui-0.2.2-test-scale-factor.patch scripts/check_gpui_patch.py scripts/test_gpui_patch.py .gitattributes vendor/gpui.provenance.md DEPENDENCIES.md
git -C /Users/davidlee/Projects/Sprite/.worktrees/close-open-findings commit -m "$(cat <<'EOF'
Let GPUI's test window change its scale factor

The test window reported a constant 2.0, so no test could move a window
between displays. It now keeps its scale in its state and
simulate_scale_factor calls the resize callback with the new scale, as
the macOS window does on a backing change. Recorded as a third patch
that check_gpui_patch.py replays after the should-terminate one.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
EOF
)"
```

- [ ] **Step 5: Add the test hooks and the failing test**

**5a.** In `crates/sprite-app/src/terminal_view.rs`, directly above `pub struct TerminalView {` (`:72`):

```rust
/// Each placeholder tile painted in the last frame: its cell, and its bounds
/// in the window.
#[cfg(test)]
pub(crate) type TileLog =
    std::rc::Rc<std::cell::RefCell<Vec<(u16, usize, gpui::Bounds<Pixels>)>>>;
```

After the `content_origin` field (`:157`):

```rust
    /// Where this frame painted each placeholder tile, for tests that check
    /// it against the cells.
    #[cfg(test)]
    painted_tiles: TileLog,
```

Next to each `content_origin: None,` (`:544` and `:633`):

```rust
            #[cfg(test)]
            painted_tiles: Default::default(),
```

After `bundle_for_test` (`:958-961`):

```rust

    #[cfg(test)]
    pub(crate) fn painted_tiles_for_test(&self) -> Vec<(u16, usize, gpui::Bounds<Pixels>)> {
        self.painted_tiles.borrow().clone()
    }

    #[cfg(test)]
    pub(crate) fn content_origin_for_test(&self) -> Option<gpui::Point<Pixels>> {
        self.content_origin
    }

    /// One cell's bounds in the window, snapped as the painter snaps them,
    /// against the grid's corner as the last frame's layout placed it.
    #[cfg(test)]
    pub(crate) fn cell_bounds_for_test(
        &self,
        column: u16,
        row: usize,
        scale: f32,
    ) -> Option<gpui::Bounds<Pixels>> {
        use crate::grid::{Col, Row, column_edge, row_edge};
        let origin = self.content_origin?;
        let (width, height) = (self.metrics.width(), self.metrics.height());
        let column = u32::from(column);
        let left = column_edge(origin.x, width, Col(column), scale).pixels();
        let right = column_edge(origin.x, width, Col(column + 1), scale).pixels();
        let top = row_edge(origin.y, height, Row(row), scale).pixels();
        let bottom = row_edge(origin.y, height, Row(row + 1), scale).pixels();
        Some(gpui::Bounds::new(
            gpui::point(left, top),
            gpui::size(right - left, bottom - top),
        ))
    }
```

**5b.** This temporary probe records where today's tile `div`s land. Step 7 deletes it. In `crates/sprite-app/src/terminal_view/render.rs`, add after `placeholder_element` (after `:121`):

```rust

/// Records where layout put one placeholder tile: a canvas covering the
/// tile's `div`, so its bounds are the tile's.
#[cfg(test)]
fn tile_probe(painted: &TileLog, column: u16, row: usize) -> impl IntoElement {
    let painted = std::rc::Rc::clone(painted);
    canvas(
        move |bounds, _window, _cx| painted.borrow_mut().push((column, row, bounds)),
        |_bounds, (), _window, _cx| {},
    )
    .absolute()
    .left(px(0.0))
    .top(px(0.0))
    .size_full()
}
```

In `image_layers`, replace:

```rust
            ) {
                layers[1].push(element);
            }
```

with:

```rust
            ) {
                #[cfg(test)]
                let element = element.child(tile_probe(&self.painted_tiles, cell.column, cell.row));
                layers[1].push(element);
            }
```

In `Render::render`, insert directly before `let grid_origin = self.content_origin.unwrap_or(origin);` (`:347`):

```rust
        #[cfg(test)]
        self.painted_tiles.borrow_mut().clear();
```

**5c.** Append to `crates/sprite-app/src/workspace/layout_tests.rs`:

```rust

/// A workspace in which every pane draws one Kitty image through two Unicode
/// placeholder cells, row 0 columns 0 and 1, and then waits.
fn image_workspace(
    cx: &mut gpui::TestAppContext,
) -> (gpui::Entity<super::Workspace>, &mut gpui::VisualTestContext) {
    // A 1x1 RGBA image as a virtual placement two cells wide. Then its two
    // placeholder cells: U+10EEEE followed by the row and column diacritics
    // (U+0305 is 0, U+030D is 1), in the foreground colour that names
    // image 1.
    let script = r"printf '\033_Ga=T,U=1,f=32,s=1,v=1,i=1,c=2,r=1,q=2;/////w==\033\\\033[38;2;0;0;1m\364\216\273\256\314\205\314\205\364\216\273\256\314\205\314\215\033[39m'; exec sleep 30";
    cx.add_window_view(|window, cx| {
        let mut settings = crate::config::Settings::default();
        settings.pane_observation.enabled = false;
        super::Workspace::new(
            Some(vec!["/bin/sh".into(), "-c".into(), script.into()]),
            settings,
            None,
            window,
            cx,
        )
    })
}

/// Placeholder tiles land on the cells the frame after a display-scale
/// change paints. In a 1000x700 window split once, each pane's grid sits
/// (699 - 42 rows of 16) / 2 = 13.5 logical pixels down. That is a whole
/// device pixel at 2x but not at 1x, where layout rounds it to 14. The
/// window moves to a 1x display, and GPUI draws once, as it does when macOS
/// reports the change. The pane without Pane Focus gets no blink to repaint
/// it, so that one frame is what stays on screen.
#[gpui::test]
fn placeholder_tiles_land_on_this_frames_cells_after_a_scale_change(
    cx: &mut gpui::TestAppContext,
) {
    let (workspace, cx) = image_workspace(cx);
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.split(Orientation::Horizontal, window, cx)
    });
    cx.simulate_resize(gpui::size(px(1000.0), px(700.0)));
    draw_workspace(cx);
    let (focused, panes) = workspace.read_with(cx, |workspace, _| {
        (
            workspace.tabs.active().and_then(|tree| tree.focus()).unwrap(),
            workspace
                .tabs
                .all_panes()
                .into_iter()
                .map(|(_, pane, _)| pane)
                .collect::<Vec<_>>(),
        )
    });
    let unfocused = panes.into_iter().find(|pane| *pane != focused).unwrap();
    let view = terminal_view(&workspace, unfocused, cx);
    let executor = cx.executor();
    executor.allow_parking();
    executor.block_test(view.condition::<()>(cx, |view, _| {
        view.bundle_for_test().is_some_and(|bundle| {
            bundle.graphics.as_ref().is_some_and(|frame| {
                frame
                    .placements
                    .iter()
                    .any(|placement| placement.image == 1 && placement.is_virtual)
            }) && bundle.render.rows.iter().any(|row| {
                row.cells
                    .iter()
                    .any(|cell| cell.text.starts_with('\u{10eeee}'))
            })
        })
    }));
    draw_workspace(cx);
    assert!(
        !view
            .read_with(cx, |view, _| view.painted_tiles_for_test())
            .is_empty(),
        "the unfocused pane painted no placeholder tiles"
    );
    let at_two = view
        .read_with(cx, |view, _| view.content_origin_for_test())
        .unwrap();
    assert_eq!(
        f32::from(at_two.y).fract(),
        0.5,
        "the layout no longer puts the grid's corner on a half pixel at 2x: {at_two:?}"
    );

    // The scale change draws its own frame; the test draws none.
    cx.simulate_scale_factor(1.0);
    let (at_one, painted, cells) = view.read_with(cx, |view, _| {
        let painted = view.painted_tiles_for_test();
        let cells = painted
            .iter()
            .map(|(column, row, _)| view.cell_bounds_for_test(*column, *row, 1.0))
            .collect::<Vec<_>>();
        (view.content_origin_for_test(), painted, cells)
    });
    let at_one = at_one.unwrap();
    assert_eq!(
        f32::from(at_one.y).fract(),
        0.0,
        "no frame was drawn at 1x: the grid's corner is still {at_one:?}"
    );
    assert!(
        painted.iter().any(|(column, row, _)| (*column, *row) == (0, 0)),
        "the tile at the grid's corner was painted: {painted:?}"
    );
    let close = |a: gpui::Pixels, b: gpui::Pixels| (f32::from(a) - f32::from(b)).abs() < 1e-3;
    for ((column, row, tile), cell) in painted.iter().zip(cells) {
        let cell = cell.unwrap();
        assert!(
            close(tile.origin.x, cell.origin.x)
                && close(tile.origin.y, cell.origin.y)
                && close(tile.size.width, cell.size.width)
                && close(tile.size.height, cell.size.height),
            "tile ({column}, {row}) painted at {tile:?}, but this frame's cell is {cell:?}"
        );
    }
}
```

After the scale-change frame, `content_origin` is that frame's corner, because the input canvas's prepaint wrote it. So `cell_bounds_for_test(…, 1.0)` gives that frame's cell edges, the same ones `GridPaint` snaps against, since both sit at the grid box's (0, 0). The two `fract()` assertions guard the premise. The first fails if a layout change stops putting the corner on a half pixel at 2x. The second fails if no frame was drawn at 1x.

- [ ] **Step 6: Run it and watch it fail**

Run: `env PATH=/Users/davidlee/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin:/Users/davidlee/.cargo/bin:/opt/homebrew/bin:/usr/bin:/bin cargo test -p sprite-app --locked --offline placeholder_tiles_land_on_this_frames_cells`

Expected: FAIL on `tile (0, 0) painted at Bounds { origin: Point { x: 10.0 px, y: 15.0 px }, … }, but this frame's cell is Bounds { origin: Point { x: 10.0 px, y: 14.0 px }, … }`. Both x values are 510.0 if the unfocused pane is the right-hand one. This is the arithmetic above: the frame snaps against y = 13.5 left over from 2x.

If the test passes here, a second frame was drawn after the scale change, and it snapped against the corner the first one stored. Do not go on. Find what notified during `simulate_scale_factor`'s effect flush, and report it.

- [ ] **Step 7: Place tiles at paint time**

In `crates/sprite-app/src/terminal_view/render.rs`:

**7a.** Replace `placeholder_element` (`:80-121`) and the `tile_probe` added in Step 5b with:

```rust
/// One placeholder tile as render finds it: the image, the cell it fills and
/// which part of the picture shows there. Where that lands in the window is
/// decided when it is painted, against the grid's corner in that frame.
pub(super) struct PlaceholderTile {
    texture: Arc<gpui::RenderImage>,
    placement: sprite_term::Placement,
    column: u16,
    row: usize,
    image_column: u32,
    image_row: u32,
    fit: super::placeholder::ImageFit,
}

impl PlaceholderTile {
    /// The tile's bounds, which clip the picture, and the picture's own, in
    /// window coordinates, for a grid whose corner is at `grid_origin`.
    fn bounds(
        &self,
        grid_origin: gpui::Point<Pixels>,
        cell_width: Pixels,
        cell_height: Pixels,
        scale: f32,
    ) -> (gpui::Bounds<Pixels>, gpui::Bounds<Pixels>) {
        let cell = super::placeholder::ImageCell {
            placement: &self.placement,
            column: self.column,
            row: self.row,
            image_column: self.image_column,
            image_row: self.image_row,
        };
        let tile = super::placeholder::tile_geometry(
            &cell,
            &self.fit,
            grid_origin,
            cell_width,
            cell_height,
            scale,
        );
        let corner = grid_origin + gpui::point(tile.left, tile.top);
        (
            gpui::Bounds::new(corner, gpui::size(tile.width, tile.height)),
            gpui::Bounds::new(
                corner + gpui::point(tile.image_left, tile.image_top),
                gpui::size(px(self.fit.width), px(self.fit.height)),
            ),
        )
    }
}

/// Paints placeholder tiles against the grid's corner as this frame laid it
/// out, the way `GridPaint` snaps the cells beside them. The element covers
/// the grid box from its corner, so its bounds' origin is that corner, at
/// whatever scale this frame is drawn. Whole device pixels throughout: a tile
/// can neither leave a seam against its neighbour nor sit half a pixel off
/// its cell.
fn placeholder_tiles(
    tiles: Vec<PlaceholderTile>,
    cell_width: Pixels,
    cell_height: Pixels,
    #[cfg(test)] painted: TileLog,
) -> impl IntoElement {
    canvas(
        |_bounds, _window, _cx| {},
        move |bounds, (), window, _cx| {
            let scale = window.scale_factor();
            for tile in &tiles {
                let (clip, picture) = tile.bounds(bounds.origin, cell_width, cell_height, scale);
                #[cfg(test)]
                painted.borrow_mut().push((tile.column, tile.row, clip));
                window.with_content_mask(Some(gpui::ContentMask { bounds: clip }), |window| {
                    // A texture the sprite atlas cannot take leaves its tile
                    // blank, as GPUI's own image element does.
                    window
                        .paint_image(
                            picture,
                            gpui::Corners::default(),
                            Arc::clone(&tile.texture),
                            0,
                            false,
                        )
                        .ok();
                });
            }
        },
    )
    .absolute()
    .left(px(0.0))
    .top(px(0.0))
    .size_full()
}
```

**7b.** Replace `image_layers` (`:173-240`) with:

```rust
    /// The images to draw, grouped by the band they belong to, and the
    /// placeholder tiles, which sit below the text after that band's fixed
    /// placements.
    ///
    /// Virtual placements draw only where a matching Unicode placeholder cell
    /// appears; fixed placements retain their own viewport coordinates. A
    /// tile is described in cells here and placed in the window when it is
    /// painted, because only layout knows where the grid's corner is.
    pub(super) fn image_layers(
        &self,
        rows: &[std::sync::Arc<Vec<PositionedCell>>],
        cell_width: Pixels,
        cell_height: Pixels,
    ) -> ([Vec<gpui::Div>; 3], Vec<PlaceholderTile>) {
        let mut layers = [Vec::new(), Vec::new(), Vec::new()];
        let mut tiles = Vec::new();
        let Some(frame) = self.bundle.as_ref().and_then(|b| b.graphics.as_ref()) else {
            return (layers, tiles);
        };

        for placement in &frame.placements {
            if placement.is_virtual || !placement.visible {
                continue;
            }
            let Some(image) = frame.image(placement.image) else {
                continue;
            };
            let Some(texture) = self.textures.get(image.id, image.generation) else {
                // No texture means the image was refused — too large, or its
                // pixels disagreed with its size. The rest of the pane still
                // draws.
                continue;
            };
            let Some(element) = placement_element(
                placement,
                texture,
                image.width,
                image.height,
                cell_width,
                cell_height,
            ) else {
                continue;
            };
            let band = match placement.layer {
                sprite_term::Layer::BelowBackground => 0,
                sprite_term::Layer::BelowText => 1,
                sprite_term::Layer::AboveText => 2,
            };
            layers[band].push(element);
        }
        for cell in super::placeholder::image_cells(rows, &frame.placements) {
            let placement = cell.placement;
            if cell.image_column >= placement.columns || cell.image_row >= placement.rows {
                continue;
            }
            let Some(image) = frame.image(placement.image) else {
                continue;
            };
            let Some(texture) = self.textures.get(image.id, image.generation) else {
                continue;
            };
            let Some(fit) = super::placeholder::fit_image(
                image.width,
                image.height,
                placement.columns as f32 * f32::from(cell_width),
                placement.rows as f32 * f32::from(cell_height),
            ) else {
                continue;
            };
            tiles.push(PlaceholderTile {
                texture,
                placement: *placement,
                column: cell.column,
                row: cell.row,
                image_column: cell.image_column,
                image_row: cell.image_row,
                fit,
            });
        }
        (layers, tiles)
    }
```

**7c.** In `Render::render`, replace `:341-358`, from `// Images first, because whether any belong below the text decides how` through `let split = !below_background.is_empty() || !below_text.is_empty();`, including the Step 5b `#[cfg(test)]` clear. The new code is:

```rust
        // Images first, because whether any belong below the text decides how
        // the rows themselves are drawn.
        let ([below_background, below_text, above_text], tiles) =
            self.image_layers(&rows, cell_width, cell_height);
        // The split costs an extra pass over the cells, so it is taken only
        // when something actually needs to sit between them. The Kitty default
        // is above the text, so the common case never pays for it.
        let split = !below_background.is_empty() || !below_text.is_empty() || !tiles.is_empty();
        #[cfg(test)]
        self.painted_tiles.borrow_mut().clear();
        #[cfg(test)]
        let tiles = (!tiles.is_empty()).then(|| {
            placeholder_tiles(
                tiles,
                cell_width,
                cell_height,
                std::rc::Rc::clone(&self.painted_tiles),
            )
        });
        #[cfg(not(test))]
        let tiles = (!tiles.is_empty()).then(|| placeholder_tiles(tiles, cell_width, cell_height));
```

This removes the comment that admitted tiles snap against the last frame's origin (`:343-346`), along with `grid_origin` and the `window.scale_factor()` argument. `content_origin` is still written by the input canvas's prepaint (`:429-434`) for `input.rs:141`. Its field doc (`terminal_view.rs:150-156`) already describes only that use.

**7d.** In `grid_box` (`:400-404`), the tiles go after the band's fixed placements and before the text, where `layers[1]` used to end:

```rust
            .children(below_background)
            .child(background_grid)
            .children(below_text)
            .children(tiles)
            .children(text_grid)
            .children(above_text)
```

`Canvas<T>` is `Styled` and `gpui::prelude::*` is already imported, so `.absolute()`, `.left()`, `.top()` and `.size_full()` resolve. `TileLog` is in scope through `use super::*;`.

- [ ] **Step 8: Run the tests and see them pass**

Run: `env PATH=/Users/davidlee/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin:/Users/davidlee/.cargo/bin:/opt/homebrew/bin:/usr/bin:/bin cargo test -p sprite-app --locked --offline placeholder_tiles_land_on_this_frames_cells`
Expected: PASS. Row 0 is now painted at y = 14, the 1x frame's own corner.

Run: `env PATH=/Users/davidlee/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin:/Users/davidlee/.cargo/bin:/opt/homebrew/bin:/usr/bin:/bin cargo test -p sprite-app --locked --offline placeholder`
Expected: PASS, including `placeholder_tiles_snap_to_the_cell_grid_and_share_one_image_origin` and `placeholder_tiles_snap_against_where_the_grid_sits_in_the_window`. They are unchanged because `tile_geometry` is unchanged.

Run: `env PATH=/Users/davidlee/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin:/Users/davidlee/.cargo/bin:/opt/homebrew/bin:/usr/bin:/bin cargo test -p sprite-app --locked --offline terminal_view`
Expected: PASS, including `texture_budget_growth_restores_a_quiet_terminal_image`.

Run: `env PATH=/Users/davidlee/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin:/Users/davidlee/.cargo/bin:/opt/homebrew/bin:/usr/bin:/bin cargo test -p sprite-app --locked --offline workspace`
Expected: PASS. The test window still starts at 2.0, so every existing layout test sees the same scale.

Run: `env PATH=/Users/davidlee/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin:/Users/davidlee/.cargo/bin:/opt/homebrew/bin:/usr/bin:/bin cargo clippy -p sprite-app --all-targets --locked --offline -- -D warnings`
Expected: no warnings. Check in particular that nothing is left unused: `placeholder_element`, `tile_probe` and `grid_origin` are all gone.

Run: `python3 scripts/check_gpui_patch.py`
Expected: `GPUI archive checksum and exact patched source verified`. This task's Sprite changes do not touch `vendor/gpui`.

- [ ] **Step 9: Commit**

```bash
git -C /Users/davidlee/Projects/Sprite/.worktrees/close-open-findings add crates/sprite-app/src/terminal_view.rs crates/sprite-app/src/terminal_view/render.rs crates/sprite-app/src/workspace/layout_tests.rs
git -C /Users/davidlee/Projects/Sprite/.worktrees/close-open-findings commit -m "$(cat <<'EOF'
Place Kitty placeholder tiles when they are painted

GAP-C08 (#68): render snapped placeholder tiles against the grid corner
the last frame's prepaint reported. After a move to a display of another
scale, that corner was off the new device-pixel grid, so the first frame
painted tiles a pixel off their cells, and an unfocused pane kept the seam
until its next output. Render now describes each tile in cells; a canvas
covering the grid box places and clips them against its own bounds when it
paints, as GridPaint does for text.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
EOF
)"
```

**Ordering note:** Task R-K changes `placeholder::image_cells` and may add inheritance rules, but it keeps the `ImageCell` fields this task builds from `PlaceholderTile`. If R-K changes those fields, update `PlaceholderTile::bounds` to match. The two tasks touch different functions. Run them in either order, then rebase.

---

### Task 9: Move the decoder into `svg_decode` and cap filter-primitive uses (R-V3)

**Files:**
- Create: `crates/sprite-app/src/surface/svg_decode.rs`
- Modify: `crates/sprite-app/src/surface.rs:11` (add `pub(crate) mod svg_decode;` after `pub mod render;`)
- Modify: `crates/sprite-app/src/surface/render.rs:10` (imports), `:75-97` (`lookup`), delete `:309-388` (`raster_bytes`, the three consts, `render_svg`, `render_svg_with_budget`), `:639-647` (test imports)
- Modify: `crates/sprite-app/src/terminal_view/list_view.rs:22-24` (imports), `:73-77` (decode call)
- Test: unit tests inside `svg_decode.rs`

**Interfaces:**
- Consumes: `resvg::usvg::{Tree, Group, Node, Options}` 0.45.1 (`Tree::root() -> &Group` mod.rs:1577, `Group::filters() -> &[Arc<filter::Filter>]` :1099, `Group::children() -> &[Node]` :1156, `Node::subroots(&self, FnMut(&Group))` :978, which covers clip paths, masks, `feImage` roots, patterns, nested SVG images and flattened text, and `filter::Filter::primitives() -> &[Primitive]` filter.rs:37).
- Produces:
  - `pub(crate) const MAX_SURFACE_IMAGE_BYTES: usize`
  - `pub(crate) const MAX_FILTER_PRIMITIVE_USES: usize`
  - `pub(crate) enum RefusalKind { Unparsable, TooComplex, Unrasterisable, OverBudget }`
  - `pub(crate) fn decode(svg: &str, target_width: Option<f32>, available_bytes: usize) -> Result<Arc<RenderImage>, RefusalKind>`
  - `pub(crate) fn raster_bytes(picture: &RenderImage) -> usize`
  - `#[cfg(test)] pub(crate) fn render_svg(svg: &str, target_width: Option<f32>) -> Option<Arc<RenderImage>>`
  - `#[cfg(test)] pub(crate) fn rasterised() -> usize`
  - `#[cfg(test)] pub(crate) fn merged_floods(results: usize, side: u32) -> String`

- [ ] **Step 1: Move the decoder (no behaviour change) and write the failing tests**

Create `crates/sprite-app/src/surface/svg_decode.rs`. At this step `count_filter_primitive_uses` is a `todo!()` stub and `decode` does not call it yet. Both are completed in Step 3.

```rust
//! Turns SVG text into a picture: parses it, refuses a document whose
//! filters would cost too much, and rasterises the rest within the Surface
//! raster bounds. Element images and list assets both come through here.

use std::sync::{Arc, LazyLock};

use gpui::RenderImage;
use resvg::usvg;

/// Decoded pixels one Surface cache keeps at once.
pub(crate) const MAX_SURFACE_IMAGE_BYTES: usize = 64 * 1024 * 1024;
/// Pixels one decoded SVG may hold.
const MAX_SVG_RASTER_BYTES: usize = 16 * 1024 * 1024;
/// The longest side, in pixels, of one decoded SVG.
const MAX_SVG_RASTER_DIMENSION: f32 = 4096.0;

/// Filter primitives one document may apply, counted once for every group
/// that applies them.
///
/// Each primitive's result is a canvas-sized intermediate raster kept until
/// its filter finishes, so a few hundred encoded bytes can describe seconds
/// of work. The encoded-size limits cannot bound that; this does. Chosen as
/// the largest measured count whose document decodes in under 250 ms in a
/// release build (`measure_filter_cap`).
///
/// Provisional: replaced by the value `measure_filter_cap` prints.
pub(crate) const MAX_FILTER_PRIMITIVE_USES: usize = 16;

/// Why an SVG was not drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RefusalKind {
    /// usvg could not read the text.
    Unparsable,
    /// The document applies more than `MAX_FILTER_PRIMITIVE_USES` filter
    /// primitives.
    TooComplex,
    /// The raster would be empty, not finite, or larger than one image may
    /// be, however much of the budget is free.
    Unrasterisable,
    /// The raster fits one image but not what the cache's budget has left.
    OverBudget,
}

/// The picture `svg` draws, `target_width` pixels wide or at its own size,
/// holding no more than `available_bytes` of pixels.
pub(crate) fn decode(
    svg: &str,
    target_width: Option<f32>,
    available_bytes: usize,
) -> Result<Arc<RenderImage>, RefusalKind> {
    let tree = parse(svg)?;
    rasterise(&tree, target_width, available_bytes)
}

/// The parsed document. The font database is loaded the first time a
/// document with text is parsed, on whichever thread parses it.
fn parse(svg: &str) -> Result<usvg::Tree, RefusalKind> {
    static FONT_DB: LazyLock<Arc<usvg::fontdb::Database>> = LazyLock::new(|| {
        let mut db = usvg::fontdb::Database::new();
        db.load_system_fonts();
        db.load_font_data(include_bytes!("../../assets/fonts/AdwaitaSans-Regular.ttf").to_vec());
        db.load_font_data(include_bytes!("../../assets/fonts/AdwaitaSans-Bold.ttf").to_vec());
        db.set_sans_serif_family("Adwaita Sans");
        Arc::new(db)
    });
    static OPTIONS: LazyLock<usvg::Options<'static>> = LazyLock::new(|| {
        let select = usvg::FontResolver::default_font_selector();
        usvg::Options {
            font_resolver: usvg::FontResolver {
                select_font: Box::new(move |font, db| {
                    if db.is_empty() {
                        *db = FONT_DB.clone();
                    }
                    select(font, db)
                }),
                select_fallback: usvg::FontResolver::default_fallback_selector(),
            },
            ..Default::default()
        }
    });
    usvg::Tree::from_data(svg.as_bytes(), &OPTIONS).map_err(|_| RefusalKind::Unparsable)
}

/// How many filter primitives `tree` applies. Each group adds the primitives
/// of every filter it applies, so a filter two groups share counts twice.
/// Clip paths, masks, patterns, nested SVG images and text outlines are
/// walked too, because they are rendered too.
fn count_filter_primitive_uses(tree: &usvg::Tree) -> usize {
    todo!()
}

/// Draws `tree`. Refuses a raster that is empty, not finite, past one
/// image's dimension or byte limit, or past `available_bytes`.
fn rasterise(
    tree: &usvg::Tree,
    target_width: Option<f32>,
    available_bytes: usize,
) -> Result<Arc<RenderImage>, RefusalKind> {
    let size = tree.size();
    let scale = target_width.map_or(1.0, |target| target / size.width());
    let width = (size.width() * scale).ceil();
    let height = (size.height() * scale).ceil();
    if [width, height].iter().any(|dimension| {
        !dimension.is_finite() || *dimension <= 0.0 || *dimension > MAX_SVG_RASTER_DIMENSION
    }) {
        return Err(RefusalKind::Unrasterisable);
    }
    let width = width as u32;
    let height = height as u32;
    let bytes = (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or(RefusalKind::Unrasterisable)?;
    if bytes > MAX_SVG_RASTER_BYTES {
        return Err(RefusalKind::Unrasterisable);
    }
    if bytes > available_bytes {
        return Err(RefusalKind::OverBudget);
    }
    let mut pixmap =
        resvg::tiny_skia::Pixmap::new(width, height).ok_or(RefusalKind::Unrasterisable)?;
    #[cfg(test)]
    RASTERISED.with(|count| count.set(count.get() + 1));
    resvg::render(
        tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    let width = pixmap.width();
    let height = pixmap.height();
    let mut pixels = pixmap.take();
    // tiny-skia holds premultiplied RGBA; GPUI draws straight BGRA.
    for pixel in pixels.chunks_exact_mut(4) {
        let alpha = pixel[3] as u16;
        if alpha > 0 && alpha < 255 {
            for channel in &mut pixel[..3] {
                *channel = ((*channel as u16 * 255 + alpha / 2) / alpha).min(255) as u8;
            }
        }
        pixel.swap(0, 2);
    }
    let buffer =
        image::RgbaImage::from_raw(width, height, pixels).ok_or(RefusalKind::Unrasterisable)?;
    Ok(Arc::new(RenderImage::new(vec![image::Frame::new(buffer)])))
}

/// What a decoded picture holds against the image budget.
pub(crate) fn raster_bytes(picture: &RenderImage) -> usize {
    picture.as_bytes(0).map_or(0, <[u8]>::len)
}

/// `decode` with a whole image's budget, for tests that only care whether
/// and how an SVG draws.
#[cfg(test)]
pub(crate) fn render_svg(svg: &str, target_width: Option<f32>) -> Option<Arc<RenderImage>> {
    decode(svg, target_width, MAX_SVG_RASTER_BYTES).ok()
}

#[cfg(test)]
thread_local! {
    /// How many rasters this thread has drawn, counted just before resvg
    /// renders, so a test can see that a refused document cost no raster.
    static RASTERISED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// How many rasters this thread has drawn so far.
#[cfg(test)]
pub(crate) fn rasterised() -> usize {
    RASTERISED.with(std::cell::Cell::get)
}

/// A `side`-pixel square whose one filter floods `results` differently named
/// results and merges them all: `results + 1` primitive uses in a few hundred
/// bytes, the shape that once took seconds to decode.
#[cfg(test)]
pub(crate) fn merged_floods(results: usize, side: u32) -> String {
    let floods: String = (0..results)
        .map(|n| {
            format!(
                "<feFlood flood-color='#{:06x}' flood-opacity='0.5' result='r{n}'/>",
                n * 0x01_0101
            )
        })
        .collect();
    let merges: String = (0..results)
        .map(|n| format!("<feMergeNode in='r{n}'/>"))
        .collect();
    format!(
        "<svg xmlns='http://www.w3.org/2000/svg' width='{side}' height='{side}'>\
         <filter id='f' x='0' y='0' width='1' height='1'>{floods}<feMerge>{merges}</feMerge></filter>\
         <rect width='{side}' height='{side}' filter='url(#f)'/></svg>"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(svg: &str) -> usvg::Tree {
        usvg::Tree::from_data(svg.as_bytes(), &usvg::Options::default()).unwrap()
    }

    #[test]
    fn filter_primitives_are_counted_once_for_every_group_that_applies_them() {
        let filter = "<filter id='f'><feFlood result='a'/><feFlood result='b'/>\
                      <feMerge><feMergeNode in='a'/><feMergeNode in='b'/></feMerge></filter>";
        let svg = |body: &str| {
            format!("<svg xmlns='http://www.w3.org/2000/svg' width='8' height='8'>{filter}{body}</svg>")
        };
        let uses = |body: &str| count_filter_primitive_uses(&tree(&svg(body)));
        assert_eq!(uses(""), 0, "a filter nothing applies costs nothing");
        assert_eq!(uses("<rect width='8' height='8' filter='url(#f)'/>"), 3);
        assert_eq!(
            uses("<rect width='8' height='8' filter='url(#f)'/><rect width='4' height='4' filter='url(#f)'/>"),
            6,
            "a shared filter counts once per use"
        );
        assert_eq!(
            uses("<g filter='url(#f)'><g filter='url(#f)'><rect width='8' height='8' filter='url(#f)'/></g></g>"),
            9,
            "nested groups each count"
        );
        assert_eq!(
            uses("<pattern id='p' width='8' height='8' patternUnits='userSpaceOnUse'>\
                  <rect width='8' height='8' filter='url(#f)'/></pattern>\
                  <rect width='8' height='8' fill='url(#p)'/>"),
            3,
            "a filter inside a pattern is rendered, so it counts"
        );
        assert_eq!(count_filter_primitive_uses(&tree(&merged_floods(4, 64))), 5);
    }

    #[test]
    fn a_document_over_the_filter_cap_is_refused_before_it_is_rasterised() {
        let before = rasterised();
        assert_eq!(
            decode(&merged_floods(128, 64), None, MAX_SVG_RASTER_BYTES).map(|_| ()),
            Err(RefusalKind::TooComplex)
        );
        assert_eq!(rasterised(), before, "a refused document is never rasterised");
        let at_cap = merged_floods(MAX_FILTER_PRIMITIVE_USES - 1, 64);
        assert!(
            decode(&at_cap, None, MAX_SVG_RASTER_BYTES).is_ok(),
            "a document at the cap is drawn"
        );
        assert_eq!(rasterised(), before + 1);
    }

    #[test]
    fn a_refusal_says_why_the_svg_was_not_drawn() {
        let refused = |svg: &str, available| decode(svg, None, available).map(|_| ()).err();
        assert_eq!(
            refused("<svg invalid", MAX_SVG_RASTER_BYTES),
            Some(RefusalKind::Unparsable)
        );
        assert_eq!(
            refused(
                "<svg xmlns='http://www.w3.org/2000/svg' width='4097' height='1'/>",
                MAX_SVG_RASTER_BYTES
            ),
            Some(RefusalKind::Unrasterisable)
        );
        assert_eq!(
            refused(
                "<svg xmlns='http://www.w3.org/2000/svg' width='2048' height='2048'/>",
                1024
            ),
            Some(RefusalKind::OverBudget)
        );
        assert_eq!(
            refused(
                "<svg xmlns='http://www.w3.org/2000/svg' width='2' height='2'/>",
                MAX_SVG_RASTER_BYTES
            ),
            None
        );
    }

    /// Prints release-build decode times for 512×512 documents of merged
    /// flood results, to choose `MAX_FILTER_PRIMITIVE_USES`. Asserts nothing
    /// about time.
    #[test]
    #[ignore = "a measurement: run by hand in a release build"]
    fn measure_filter_cap() {
        const BUDGET_MS: f64 = 250.0;
        assert!(
            decode(&merged_floods(1, 512), None, MAX_SVG_RASTER_BYTES).is_ok(),
            "the warm-up decode builds the parser options"
        );
        let mut chosen = None;
        for results in [1usize, 2, 4, 8, 16, 32] {
            let svg = merged_floods(results, 512);
            let uses = count_filter_primitive_uses(&parse(&svg).unwrap());
            let mut times: Vec<f64> = (0..5)
                .map(|_| {
                    let started = std::time::Instant::now();
                    let tree = parse(&svg).unwrap();
                    assert!(rasterise(&tree, None, MAX_SVG_RASTER_BYTES).is_ok());
                    started.elapsed().as_secs_f64() * 1000.0
                })
                .collect();
            times.sort_by(f64::total_cmp);
            let median = times[times.len() / 2];
            println!("{results:>3} results, {uses:>3} primitive uses: median {median:.0} ms of 5");
            if median < BUDGET_MS {
                chosen = Some((results, uses, median));
            }
        }
        match chosen {
            Some((results, uses, median)) => println!(
                "cap: MAX_FILTER_PRIMITIVE_USES = {uses}; a 512x512 document of {results} merged \
                 flood results ({uses} uses) decodes in {median:.0} ms (median of 5, release \
                 build, {}/{})",
                std::env::consts::OS,
                std::env::consts::ARCH
            ),
            None => println!("no measured document decodes in under {BUDGET_MS} ms"),
        }
    }
}
```

Add the module in `crates/sprite-app/src/surface.rs` after `pub mod render;`:

```rust
pub(crate) mod svg_decode;
```

Rewire `crates/sprite-app/src/surface/render.rs`:
- Line 10: `use std::sync::{Arc, LazyLock};` becomes `use std::sync::Arc;`.
- After `use crate::surface::style;` (line 23) add:

```rust
use crate::surface::svg_decode::{self, MAX_SURFACE_IMAGE_BYTES, raster_bytes};
```

- In `lookup` (`:86-91`), replace:

```rust
        let available = MAX_SURFACE_IMAGE_BYTES - self.retained_bytes;
        let picture = if available >= MAX_SVG_RASTER_BYTES {
            render_svg(svg, None)
        } else {
            render_svg_with_budget(svg, None, available)
        };
```

with:

```rust
        let available = MAX_SURFACE_IMAGE_BYTES - self.retained_bytes;
        let picture = svg_decode::decode(svg, None, available).ok();
```

- Delete lines 309-388: the `raster_bytes` doc and fn, `MAX_SURFACE_IMAGE_BYTES`, `MAX_SVG_RASTER_BYTES`, `MAX_SVG_RASTER_DIMENSION`, `render_svg` and `render_svg_with_budget`, through the closing `}` before `/// What a grid borrows from the pane…`.
- In `mod tests` (`:639-647`), after `use crate::surface::description;` add:

```rust
    use crate::surface::svg_decode::render_svg;
```

Rewire `crates/sprite-app/src/terminal_view/list_view.rs`:
- Lines 22-24:

```rust
#[cfg(test)]
use crate::surface::render::render_svg;
use crate::surface::render::{MAX_SURFACE_IMAGE_BYTES, raster_bytes, render_svg_with_budget};
```

become:

```rust
#[cfg(test)]
use crate::surface::svg_decode::render_svg;
use crate::surface::svg_decode::{MAX_SURFACE_IMAGE_BYTES, decode, raster_bytes};
```

- Lines 73-77:

```rust
            let picture = render_svg_with_budget(
                svg,
                Some(raster_width),
                MAX_SURFACE_IMAGE_BYTES - retained_bytes,
            );
```

become:

```rust
            let picture = decode(
                svg,
                Some(raster_width),
                MAX_SURFACE_IMAGE_BYTES - retained_bytes,
            )
            .ok();
```

- [ ] **Step 2: Run the tests to verify the new ones fail and the moved code still passes**

Run: `env PATH=/Users/davidlee/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin:/Users/davidlee/.cargo/bin:/opt/homebrew/bin:/usr/bin:/bin cargo test -p sprite-app --locked --offline svg`
Expected:
- `svg_decode::tests::filter_primitives_are_counted_once_for_every_group_that_applies_them` FAILS with `not yet implemented`.
- `svg_decode::tests::a_document_over_the_filter_cap_is_refused_before_it_is_rasterised` FAILS on `assertion left == right failed`, `left: Ok(())`, `right: Err(TooComplex)`.
- `a_refusal_says_why_the_svg_was_not_drawn` passes, as do the existing `svg_raster_*`, `text_only_svg_renders_visible_pixels`, `*_svg_*` and list `svg_cache_*` tests.

- [ ] **Step 3: Implement the count and the cap**

In `svg_decode.rs`, replace the body of `count_filter_primitive_uses`:

```rust
fn count_filter_primitive_uses(tree: &usvg::Tree) -> usize {
    fn walk(group: &usvg::Group, uses: &mut usize) {
        for filter in group.filters() {
            *uses = uses.saturating_add(filter.primitives().len());
        }
        for node in group.children() {
            if let usvg::Node::Group(child) = node {
                walk(child, uses);
            }
            node.subroots(|subroot| walk(subroot, uses));
        }
    }
    let mut uses = 0;
    walk(tree.root(), &mut uses);
    uses
}
```

and make `decode` check the cap before rasterising:

```rust
pub(crate) fn decode(
    svg: &str,
    target_width: Option<f32>,
    available_bytes: usize,
) -> Result<Arc<RenderImage>, RefusalKind> {
    let tree = parse(svg)?;
    if count_filter_primitive_uses(&tree) > MAX_FILTER_PRIMITIVE_USES {
        return Err(RefusalKind::TooComplex);
    }
    rasterise(&tree, target_width, available_bytes)
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `env PATH=/Users/davidlee/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin:/Users/davidlee/.cargo/bin:/opt/homebrew/bin:/usr/bin:/bin cargo test -p sprite-app --locked --offline svg`
Expected: all pass, with `measure_filter_cap` reported as ignored.

If a count in `filter_primitives_are_counted_once_for_every_group_that_applies_them` differs, print the tree and check how usvg kept or dropped that group. Use `tree(&svg(body)).root().children()` in a scratch `dbg!`. Fix the test's expectation only if usvg legitimately removed a group that renders nothing. The shared-filter and merged-floods counts must hold as written.

- [ ] **Step 5: Measure the cap in a release build**

Run: `env PATH=/Users/davidlee/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin:/Users/davidlee/.cargo/bin:/opt/homebrew/bin:/usr/bin:/bin cargo test -p sprite-app --locked --offline --release --lib measure_filter_cap -- --ignored --nocapture`

Expected: six lines of the form ` 16 results,  17 primitive uses: median N ms of 5`, then one line starting `cap: MAX_FILTER_PRIMITIVE_USES = …`.
- If it prints `no measured document decodes in under 250 ms`, stop and escalate. Even a one-flood document is too slow, so a cap cannot meet the 250 ms target on this machine.
- Keep the printed `cap:` line. Task 11 copies it into ADR 0022.

- [ ] **Step 6: Record the measured cap**

In `svg_decode.rs`:
- Set `MAX_FILTER_PRIMITIVE_USES` to the number after `MAX_FILTER_PRIMITIVE_USES = ` in the `cap:` line.
- Replace the line `/// Provisional: replaced by the value \`measure_filter_cap\` prints.` with `/// Measured: ` followed by the `cap:` line's text after `cap: `, wrapped at 79 columns as further `///` lines.

Then re-run Step 4's command. Expected: all pass. `a_document_over_the_filter_cap_is_refused_before_it_is_rasterised` uses the constant, and 128 results stays above any cap the sweep can print (at most 33).

- [ ] **Step 7: Commit**

```bash
git -C /Users/davidlee/Projects/Sprite/.worktrees/close-open-findings add crates/sprite-app/src/surface.rs crates/sprite-app/src/surface/svg_decode.rs crates/sprite-app/src/surface/render.rs crates/sprite-app/src/terminal_view/list_view.rs
git -C /Users/davidlee/Projects/Sprite/.worktrees/close-open-findings commit -m "$(cat <<'EOF'
Move SVG decoding into one module and cap filter-primitive uses

A document applying more filter primitives than the measured cap is
refused before it is rasterised. The cap is the largest count whose
512x512 document decodes in under 250 ms in a release build.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
EOF
)"
```

---

---

### Task 10: Element images decode on the background executor, two at a time (R-V1, R-V1a, R-V2, R-V2a)

**Files:**
- Modify: `crates/sprite-app/src/surface/svg_decode.rs`. Imports, module doc, and new `SvgState`, `SvgDecodes`, `Queue`, `Entry`, `Job`, `start_waiting`, plus test accessors.
- Modify: `crates/sprite-app/src/surface/render.rs`:
  - `:1-26` (module doc and imports)
  - `:28-117` (`ElementImageCache` and its impl)
  - `:137-163` (test accessors)
  - `:165-222` (`ElementBody`: add `start_decodes`)
  - tests `:639-1048`, rewriting four existing tests and adding four new ones
- Modify: `crates/sprite-app/src/terminal_view/surfaces.rs`:
  - `:1128-1131` (`surface_element` starts decodes)
  - tests `:1629-1808` (two tests rewritten)
  - new helpers next to `draw_test_window` (`:1875`)
  - one new test

**Interfaces:**
- Consumes: `svg_decode::{decode, raster_bytes, MAX_SURFACE_IMAGE_BYTES, RefusalKind}` from Task 9. `gpui::Context<T>::{background_executor, spawn}`, `WeakEntity<T>::update`, `Task::detach`.
- Produces:
  - `pub(crate) const MAX_CONCURRENT_SVG_DECODES: usize = 2;`
  - `#[derive(Clone, Debug)] pub(crate) enum SvgState { Pending, Ready(Arc<RenderImage>), Refused(RefusalKind) }` with `pub(crate) fn picture(&self) -> Option<Arc<RenderImage>>`
  - `pub(crate) struct SvgDecodes<K>`, which implements `Default`. It has:
    - `pub(crate) fn new(target_width: Option<f32>) -> Self`
    - `pub(crate) fn settled(&self) -> u64`
    - `pub(crate) fn want<Q>(&self, key: &Q, job: impl FnOnce() -> (K, Arc<str>)) -> SvgState where K: Borrow<Q>, Q: Hash + Eq + ?Sized`
    - `pub(crate) fn start<T: 'static>(&self, cx: &mut Context<T>)`
    - `pub(crate) fn retain(&self, keep: impl Fn(&K) -> bool)`
    - test-only `state`, `keys`, `started`, `in_flight`, `peak_in_flight` and `retained_bytes`
  - `ElementImageCache::start_decodes<T: 'static>(&self, cx: &mut Context<T>)`
  - `ElementBody::start_decodes<T: 'static>(&self, cx: &mut Context<T>)`
  - test-only `ElementImageCache::{state_for, queue}`, with `cached_texts` now returning `Vec<String>`

- [ ] **Step 1: Write the failing seam test (compiles against the current API)**

In `crates/sprite-app/src/terminal_view/surfaces.rs` `mod tests`, add after `an_update_keeps_the_decode_of_an_unchanged_svg_and_decodes_a_changed_one_once`:

```rust
    #[gpui::test]
    fn an_svg_is_decoded_off_the_ui_thread_and_its_view_told_when_it_is_ready(
        cx: &mut gpui::TestAppContext,
    ) {
        let settings = crate::config::Settings::default();
        cx.set_global(crate::config::ActiveSettings(settings.clone()));
        cx.set_global(TokenRegistry::new(&settings.colors));
        let (host, cx) = cx.add_window_view(|window, cx| {
            TerminalView::failed(
                "test".to_owned(),
                SharedString::from(".SystemUIFont"),
                window,
                cx,
            )
        });
        let (stream, _peer) = std::os::unix::net::UnixStream::pair().unwrap();
        let connection = SurfaceConnection::new(&stream).unwrap();
        // The shape that once took seconds to decode: differently named flood
        // results, all merged by one feMerge.
        let floods: String = (0..4)
            .map(|n| format!("<feFlood flood-color='#{:06x}' result='r{n}'/>", n * 0x40))
            .collect();
        let merges: String = (0..4)
            .map(|n| format!("<feMergeNode in='r{n}'/>"))
            .collect();
        let svg = format!(
            "<svg xmlns='http://www.w3.org/2000/svg' width='64' height='64'>\
             <filter id='f' x='0' y='0' width='1' height='1'>{floods}<feMerge>{merges}</feMerge></filter>\
             <rect width='64' height='64' filter='url(#f)'/></svg>"
        );
        host.update_in(cx, |view, window, cx| {
            view.open_surface(
                SurfaceId(5),
                open_description(serde_json::json!({"version":1,"root":{
                    "kind":"image","style":"w_16 h_16","svg":svg
                }})),
                connection,
                window,
                cx,
            )
            .unwrap();
        });
        let notified = std::rc::Rc::new(std::cell::Cell::new(0usize));
        let counter = notified.clone();
        let _observer =
            cx.update(|_, cx| cx.observe(&host, move |_, _| counter.set(counter.get() + 1)));
        let drawn = |cx: &mut gpui::VisualTestContext| {
            host.read_with(cx, |view, _| {
                let Body::Elements(elements) = &view.surfaces.fill.as_ref().unwrap().body else {
                    panic!("expected elements")
                };
                elements.images().image_for(&svg)
            })
        };

        draw_test_window(cx);
        assert!(
            drawn(cx).is_none(),
            "the frame decoded the SVG on the UI thread"
        );
        let before = notified.get();
        cx.run_until_parked();
        assert!(
            notified.get() > before,
            "the finished decode told the view to draw again"
        );
        assert!(
            drawn(cx).is_some(),
            "the decode finished on the background executor"
        );
        cx.update(|window, _| window.remove_window());
        drop(host);
    }
```

- [ ] **Step 2: Run it to verify it fails for the stated reason**

Run: `env PATH=/Users/davidlee/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin:/Users/davidlee/.cargo/bin:/opt/homebrew/bin:/usr/bin:/bin cargo test -p sprite-app --locked --offline an_svg_is_decoded_off_the_ui_thread`
Expected: FAIL with `the frame decoded the SVG on the UI thread`. The frame's `lookup` decoded synchronously, so the picture exists before any background work ran.

- [ ] **Step 3: Write the failing queue tests**

In `crates/sprite-app/src/surface/render.rs` `mod tests`:
- Replace the line `use crate::surface::svg_decode::render_svg;` (added in V1) with:

```rust
    use crate::surface::svg_decode::{
        self, MAX_CONCURRENT_SVG_DECODES, RefusalKind, SvgState, merged_floods, render_svg,
    };
    use gpui::AppContext as _;

    /// Stands in for the view that draws a Surface: a finished decode tells
    /// this entity to draw again.
    struct Owner;

    /// One frame of `description`, as a view draws it: builds the elements,
    /// then starts the decodes that frame asked for.
    fn draw_frame(
        cx: &mut gpui::TestAppContext,
        owner: &Entity<Owner>,
        cache: &mut ElementImageCache,
        description: &Description,
        registry: &TokenRegistry,
        connection: &SurfaceConnection,
    ) {
        owner.update(cx, |_, cx| {
            let _ = render(
                description,
                &[],
                SurfaceId(1),
                registry,
                connection,
                None,
                cache,
            );
            cache.start_decodes(cx);
        });
    }
```

- Add these four tests at the end of the module:

```rust
    #[gpui::test]
    fn a_pending_image_draws_nothing_and_nothing_is_rasterised_until_the_executor_runs(
        cx: &mut gpui::TestAppContext,
    ) {
        let (ours, _peer) = UnixStream::pair().unwrap();
        let connection = SurfaceConnection::new(&ours).unwrap();
        let registry = TokenRegistry::new(&Colors::default());
        let svg = merged_floods(4, 64);
        let document = description::parse(
            &json!({"version":1,"root":{"kind":"image","svg":svg}}),
            &registry,
        )
        .unwrap()
        .description;
        let owner = cx.new(|_| Owner);
        let notified = Rc::new(std::cell::Cell::new(0usize));
        let counter = Rc::clone(&notified);
        let _observer =
            cx.update(|cx| cx.observe(&owner, move |_, _| counter.set(counter.get() + 1)));
        let rasterised = svg_decode::rasterised();
        let mut cache = ElementImageCache::default();

        draw_frame(cx, &owner, &mut cache, &document, &registry, &connection);
        assert!(matches!(cache.state_for(&svg), Some(SvgState::Pending)));
        assert!(cache.image_for(&svg).is_none(), "a pending image draws nothing");
        assert_eq!(cache.queue().in_flight(), 1);
        assert_eq!(
            svg_decode::rasterised(),
            rasterised,
            "the frame rasterised nothing"
        );

        cx.run_until_parked();
        assert!(matches!(cache.state_for(&svg), Some(SvgState::Ready(_))));
        assert_eq!(svg_decode::rasterised(), rasterised + 1);
        assert!(
            notified.get() > 0,
            "the view that drew the image was told it is ready"
        );
    }

    #[gpui::test]
    fn a_document_over_the_filter_cap_is_refused_never_rasterised_and_not_tried_again(
        cx: &mut gpui::TestAppContext,
    ) {
        let (ours, _peer) = UnixStream::pair().unwrap();
        let connection = SurfaceConnection::new(&ours).unwrap();
        let registry = TokenRegistry::new(&Colors::default());
        let svg = merged_floods(128, 64);
        let document = description::parse(
            &json!({"version":1,"root":{"kind":"image","svg":svg}}),
            &registry,
        )
        .unwrap()
        .description;
        let owner = cx.new(|_| Owner);
        let rasterised = svg_decode::rasterised();
        let mut cache = ElementImageCache::default();

        draw_frame(cx, &owner, &mut cache, &document, &registry, &connection);
        cx.run_until_parked();
        assert!(matches!(
            cache.state_for(&svg),
            Some(SvgState::Refused(RefusalKind::TooComplex))
        ));
        assert_eq!(
            svg_decode::rasterised(),
            rasterised,
            "the refused document was never rasterised"
        );

        // An update that still draws it keeps the refusal instead of
        // decoding it again.
        cache.retain_drawn_by(&document);
        draw_frame(cx, &owner, &mut cache, &document, &registry, &connection);
        cx.run_until_parked();
        assert!(matches!(
            cache.state_for(&svg),
            Some(SvgState::Refused(RefusalKind::TooComplex))
        ));
        assert_eq!(cache.decodes(), 1, "a refused document is decoded once");
    }

    #[gpui::test]
    fn a_cache_decodes_at_most_two_images_at_a_time_in_the_order_they_were_drawn(
        cx: &mut gpui::TestAppContext,
    ) {
        let (ours, _peer) = UnixStream::pair().unwrap();
        let connection = SurfaceConnection::new(&ours).unwrap();
        let registry = TokenRegistry::new(&Colors::default());
        let svg = |n: usize| {
            format!(
                "<svg xmlns='http://www.w3.org/2000/svg' width='4' height='4'><desc>{n}</desc></svg>"
            )
        };
        let document = description::parse(
            &json!({"version":1,"root":{"kind":"box","children":(0..5)
                .map(|n| json!({"kind":"image","svg":svg(n)}))
                .collect::<Vec<_>>()}}),
            &registry,
        )
        .unwrap()
        .description;
        let in_order = |count: usize| (0..count).map(|n| Arc::<str>::from(svg(n))).collect::<Vec<_>>();
        let owner = cx.new(|_| Owner);
        let mut cache = ElementImageCache::default();

        draw_frame(cx, &owner, &mut cache, &document, &registry, &connection);
        assert_eq!(cache.queue().in_flight(), MAX_CONCURRENT_SVG_DECODES);
        assert_eq!(cache.queue().started(), in_order(2), "the first two drawn start first");
        assert!((0..5).all(|n| matches!(cache.state_for(&svg(n)), Some(SvgState::Pending))));

        cx.run_until_parked();
        assert_eq!(cache.queue().started(), in_order(5), "the rest started oldest first");
        assert_eq!(cache.queue().peak_in_flight(), MAX_CONCURRENT_SVG_DECODES);
        assert_eq!(cache.queue().in_flight(), 0);
        assert!((0..5).all(|n| cache.image_for(&svg(n)).is_some()));
    }

    #[gpui::test]
    fn a_decode_that_finishes_after_its_image_left_the_description_is_discarded(
        cx: &mut gpui::TestAppContext,
    ) {
        let (ours, _peer) = UnixStream::pair().unwrap();
        let connection = SurfaceConnection::new(&ours).unwrap();
        let registry = TokenRegistry::new(&Colors::default());
        let svg = |n: usize| {
            format!(
                "<svg xmlns='http://www.w3.org/2000/svg' width='4' height='4'><desc>{n}</desc></svg>"
            )
        };
        let document = |n: usize| {
            description::parse(
                &json!({"version":1,"root":{"kind":"image","svg":svg(n)}}),
                &registry,
            )
            .unwrap()
            .description
        };
        let owner = cx.new(|_| Owner);
        let mut cache = ElementImageCache::default();

        draw_frame(cx, &owner, &mut cache, &document(0), &registry, &connection);
        assert_eq!(cache.queue().in_flight(), 1);
        let next = document(1);
        cache.retain_drawn_by(&next);
        cx.run_until_parked();
        assert!(
            cache.state_for(&svg(0)).is_none(),
            "the finished decode of an image no longer drawn was discarded"
        );
        assert_eq!(cache.retained_bytes(), 0, "and charged nothing");

        draw_frame(cx, &owner, &mut cache, &next, &registry, &connection);
        cx.run_until_parked();
        assert!(cache.image_for(&svg(1)).is_some());
        assert_eq!(cache.retained_bytes(), 4 * 4 * 4);
    }
```

- [ ] **Step 4: Run them to verify they fail**

Run: `env PATH=/Users/davidlee/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin:/Users/davidlee/.cargo/bin:/opt/homebrew/bin:/usr/bin:/bin cargo test -p sprite-app --locked --offline surface::render`
Expected: FAIL to compile. You get `E0432` (unresolved imports `MAX_CONCURRENT_SVG_DECODES`, `SvgState`) and `E0599` (no method named `start_decodes`, `state_for`, `queue` on `ElementImageCache`).

- [ ] **Step 5: Implement the queue**

In `crates/sprite-app/src/surface/svg_decode.rs`, replace the module doc and the `use` block with:

```rust
//! Turns SVG text into a picture, off the UI thread.
//!
//! A cache asks for an SVG and gets `Pending` until a decode on GPUI's
//! background executor finishes. The decode parses the text, refuses a
//! document whose filters would cost too much, and rasterises the rest
//! within the Surface raster bounds. Each cache runs at most
//! `MAX_CONCURRENT_SVG_DECODES` at once, oldest request first. A finished
//! picture is charged to the cache's budget when it is installed, and the
//! view that asked is told to draw again.

use std::borrow::Borrow;
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::hash::Hash;
use std::rc::Rc;
use std::sync::{Arc, LazyLock};

use gpui::{Context, RenderImage};
use resvg::usvg;
```

After `MAX_FILTER_PRIMITIVE_USES`, add:

```rust
/// Decodes one cache runs at the same time. The rest wait in the order they
/// were asked for, so the memory filter intermediates take is bounded per
/// cache by this many capped decodes, not by how many images a description
/// holds.
pub(crate) const MAX_CONCURRENT_SVG_DECODES: usize = 2;
```

After the `RefusalKind` enum, add:

```rust
/// What one SVG a cache asked for has come to.
#[derive(Clone, Debug)]
pub(crate) enum SvgState {
    /// Waiting for, or in, its decode. Draws nothing.
    Pending,
    Ready(Arc<RenderImage>),
    Refused(RefusalKind),
}

impl SvgState {
    /// The picture to draw, if the decode is ready.
    pub(crate) fn picture(&self) -> Option<Arc<RenderImage>> {
        match self {
            Self::Ready(picture) => Some(Arc::clone(picture)),
            Self::Pending | Self::Refused(_) => None,
        }
    }
}

/// The decodes of one cache: what each SVG it asked for has come to, and the
/// decodes still waiting for a turn.
///
/// Shared with the tasks running its decodes. A task holds it weakly, so a
/// cache that is dropped (its Surface closed) drops whatever finishes after.
pub(crate) struct SvgDecodes<K> {
    queue: Rc<RefCell<Queue<K>>>,
}

struct Queue<K> {
    entries: HashMap<K, Entry>,
    /// Asked for and not yet started, oldest first.
    waiting: VecDeque<Job<K>>,
    in_flight: usize,
    /// Pixels the ready pictures hold, at most `MAX_SURFACE_IMAGE_BYTES`.
    retained_bytes: usize,
    /// How many entries have settled. A cache that remembers what it found
    /// compares this to know when to look again.
    settled: u64,
    /// Stamped on each entry, so a decode finishing for an entry that has
    /// been dropped, or dropped and asked for again, is told apart.
    next_ticket: u64,
    target_width: Option<f32>,
    #[cfg(test)]
    started: Vec<K>,
    #[cfg(test)]
    peak_in_flight: usize,
}

struct Entry {
    state: SvgState,
    ticket: u64,
}

struct Job<K> {
    key: K,
    ticket: u64,
    svg: Arc<str>,
}

impl<K> Default for SvgDecodes<K> {
    fn default() -> Self {
        Self::new(None)
    }
}

impl<K> SvgDecodes<K> {
    /// The decodes of a cache that draws each SVG `target_width` pixels wide,
    /// or at its own size.
    pub(crate) fn new(target_width: Option<f32>) -> Self {
        Self {
            queue: Rc::new(RefCell::new(Queue {
                entries: HashMap::new(),
                waiting: VecDeque::new(),
                in_flight: 0,
                retained_bytes: 0,
                settled: 0,
                next_ticket: 0,
                target_width,
                #[cfg(test)]
                started: Vec::new(),
                #[cfg(test)]
                peak_in_flight: 0,
            })),
        }
    }

    /// Goes up each time a decode is installed or refused.
    pub(crate) fn settled(&self) -> u64 {
        self.queue.borrow().settled
    }
}

impl<K: Clone + Eq + Hash + 'static> SvgDecodes<K> {
    /// What `key` has come to. The first time it is asked for, `job` names it
    /// and its SVG text, its decode joins the end of the queue, and it is
    /// `Pending`. Nothing is decoded here: `start` does that.
    pub(crate) fn want<Q>(&self, key: &Q, job: impl FnOnce() -> (K, Arc<str>)) -> SvgState
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        let mut queue = self.queue.borrow_mut();
        if let Some(entry) = queue.entries.get(key) {
            return entry.state.clone();
        }
        let (key, svg) = job();
        let ticket = queue.next_ticket;
        queue.next_ticket += 1;
        queue.entries.insert(
            key.clone(),
            Entry {
                state: SvgState::Pending,
                ticket,
            },
        );
        queue.waiting.push_back(Job { key, ticket, svg });
        SvgState::Pending
    }

    /// Starts waiting decodes, oldest first, until `MAX_CONCURRENT_SVG_DECODES`
    /// are running. `cx` is the view that draws this cache: as each decode
    /// finishes, the view is told to draw again.
    pub(crate) fn start<T: 'static>(&self, cx: &mut Context<T>) {
        start_waiting(&self.queue, cx);
    }

    /// Keeps the entries `keep` accepts and drops the rest. A decode still
    /// running for a dropped entry is discarded when it finishes. A refusal
    /// for lack of budget is dropped too, so the next frame asks again
    /// against whatever this freed. Every other refusal stays for as long as
    /// it is kept.
    pub(crate) fn retain(&self, keep: impl Fn(&K) -> bool) {
        let mut queue = self.queue.borrow_mut();
        let Queue {
            entries,
            waiting,
            retained_bytes,
            ..
        } = &mut *queue;
        entries.retain(|key, entry| {
            keep(key) && !matches!(entry.state, SvgState::Refused(RefusalKind::OverBudget))
        });
        waiting.retain(|job| {
            entries
                .get(&job.key)
                .is_some_and(|entry| entry.ticket == job.ticket)
        });
        *retained_bytes = entries
            .values()
            .filter_map(|entry| entry.state.picture())
            .map(|picture| raster_bytes(&picture))
            .sum();
    }
}

impl<K: Clone + Eq + Hash> Queue<K> {
    /// The next decode to start, with the width to draw it at and the budget
    /// it may use, if a slot is free and one is waiting.
    fn next_job(&mut self) -> Option<(Job<K>, Option<f32>, usize)> {
        while self.in_flight < MAX_CONCURRENT_SVG_DECODES {
            let job = self.waiting.pop_front()?;
            if !self
                .entries
                .get(&job.key)
                .is_some_and(|entry| entry.ticket == job.ticket)
            {
                continue;
            }
            self.in_flight += 1;
            #[cfg(test)]
            {
                self.started.push(job.key.clone());
                self.peak_in_flight = self.peak_in_flight.max(self.in_flight);
            }
            let available = MAX_SURFACE_IMAGE_BYTES.saturating_sub(self.retained_bytes);
            return Some((job, self.target_width, available));
        }
        None
    }

    /// Installs a finished decode, charging its pixels to the budget, unless
    /// its entry was dropped since it started. A picture the budget can no
    /// longer hold is refused for lack of budget.
    fn finish(&mut self, key: &K, ticket: u64, outcome: Result<Arc<RenderImage>, RefusalKind>) {
        self.in_flight = self.in_flight.saturating_sub(1);
        let Some(entry) = self.entries.get_mut(key) else {
            return;
        };
        if entry.ticket != ticket {
            return;
        }
        entry.state = match outcome {
            Ok(picture) => {
                let bytes = raster_bytes(&picture);
                if bytes > MAX_SURFACE_IMAGE_BYTES.saturating_sub(self.retained_bytes) {
                    SvgState::Refused(RefusalKind::OverBudget)
                } else {
                    self.retained_bytes += bytes;
                    SvgState::Ready(picture)
                }
            }
            Err(refusal) => SvgState::Refused(refusal),
        };
        self.settled += 1;
    }
}

/// Starts waiting decodes until the queue is full or empty. Each one parses
/// and rasterises on the background executor. Then, back on the view's
/// thread, its result is installed, the next decode starts, and the view is
/// told to draw again.
fn start_waiting<K, T>(queue: &Rc<RefCell<Queue<K>>>, cx: &mut Context<T>)
where
    K: Clone + Eq + Hash + 'static,
    T: 'static,
{
    loop {
        let next = queue.borrow_mut().next_job();
        let Some((Job { key, ticket, svg }, target_width, available_bytes)) = next else {
            return;
        };
        let decoding = cx
            .background_executor()
            .spawn(async move { decode(&svg, target_width, available_bytes) });
        let weak = Rc::downgrade(queue);
        cx.spawn(async move |view, cx| {
            let outcome = decoding.await;
            let Some(queue) = weak.upgrade() else {
                return;
            };
            queue.borrow_mut().finish(&key, ticket, outcome);
            view.update(cx, |_, cx| {
                start_waiting(&queue, cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

#[cfg(test)]
impl<K: Clone + Eq + Hash> SvgDecodes<K> {
    /// What `key` has come to, without asking for it.
    pub(crate) fn state<Q>(&self, key: &Q) -> Option<SvgState>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self.queue
            .borrow()
            .entries
            .get(key)
            .map(|entry| entry.state.clone())
    }

    pub(crate) fn keys(&self) -> Vec<K> {
        self.queue.borrow().entries.keys().cloned().collect()
    }

    /// Every key whose decode has started, in the order they started.
    pub(crate) fn started(&self) -> Vec<K> {
        self.queue.borrow().started.clone()
    }

    pub(crate) fn in_flight(&self) -> usize {
        self.queue.borrow().in_flight
    }

    pub(crate) fn peak_in_flight(&self) -> usize {
        self.queue.borrow().peak_in_flight
    }

    pub(crate) fn retained_bytes(&self) -> usize {
        self.queue.borrow().retained_bytes
    }
}
```

In `crates/sprite-app/src/surface/render.rs`:

Module doc (`:1-5`) becomes:

```rust
//! Draws a Surface Description: one GPUI element per described element,
//! built fresh on every frame the way GPUI's own views are. Decoded SVGs are
//! kept by their SVG text, so updates that keep an SVG reuse its decode, and
//! an update releases the ones it no longer draws. A frame only asks for a
//! decode; it runs on the background executor, and until it is ready the
//! image draws nothing. Redraws find each decode by its place in the tree,
//! without touching the text.
```

In the imports:
- Add `Context` to the `gpui::{…}` list.
- Replace `use crate::surface::svg_decode::{self, MAX_SURFACE_IMAGE_BYTES, raster_bytes};` with:

```rust
use crate::surface::svg_decode::{SvgDecodes, SvgState};
```

Replace the `ElementImageCache` struct and its first `impl` (`:28-117`, from the struct's doc comment through the end of `retain_drawn_by`) with:

```rust
/// The decoded images of one element Surface, kept across frames and across
/// updates for as long as its description draws them.
///
/// Keyed by the SVG text itself: the map hashes it to find an entry and then
/// compares the stored text, so two different SVGs can never share a
/// picture. Elements that draw the same SVG share one decode, and an update
/// that keeps an SVG keeps its decode wherever in the tree it moved.
///
/// The text is looked up once per description, and once more after any
/// decode finishes. A description may carry megabytes of SVG, so in between,
/// each image's state is kept by its tree-order index and every frame finds
/// it there.
#[derive(Default)]
pub(crate) struct ElementImageCache {
    decodes: SvgDecodes<Arc<str>>,
    /// What each image of the current description had come to when it was
    /// last looked up, by its index in tree order. `None` until that image is
    /// drawn. Emptied by `retain_drawn_by`, which `ElementBody::replace`
    /// calls with every new description, and whenever a decode has settled
    /// since `placed_at`.
    placed: Vec<Option<SvgState>>,
    placed_at: u64,
    #[cfg(test)]
    text_lookups: usize,
}

impl ElementImageCache {
    /// The picture for the image at tree-order `index`, drawing `svg`. `None`
    /// while its decode is pending or once it was refused.
    fn picture(&mut self, index: u64, svg: &str) -> Option<Arc<RenderImage>> {
        let settled = self.decodes.settled();
        if self.placed_at != settled {
            self.placed.clear();
            self.placed_at = settled;
        }
        let index = index as usize;
        if let Some(Some(state)) = self.placed.get(index) {
            return state.picture();
        }
        let state = self.lookup(svg);
        if self.placed.len() <= index {
            self.placed.resize(index + 1, None);
        }
        let picture = state.picture();
        self.placed[index] = Some(state);
        picture
    }

    /// What one SVG has come to. Its decode is queued the first time any
    /// frame or description draws it.
    fn lookup(&mut self, svg: &str) -> SvgState {
        #[cfg(test)]
        {
            self.text_lookups += 1;
        }
        self.decodes.want(svg, || {
            let text: Arc<str> = Arc::from(svg);
            (Arc::clone(&text), text)
        })
    }

    /// Starts the decodes the frames so far asked for. `cx` is the view that
    /// draws this cache; it is told to draw again as each one finishes.
    pub(crate) fn start_decodes<T: 'static>(&self, cx: &mut Context<T>) {
        self.decodes.start(cx);
    }

    /// Keeps the decodes `description` still draws and releases the rest, so
    /// the budget counts only images on screen. A decode refused for lack of
    /// budget is forgotten too, and the next frame tries it again against
    /// whatever the update freed. A document that does not parse, or has too
    /// many filters, stays refused while the description draws it.
    fn retain_drawn_by(&mut self, description: &Description) {
        self.placed.clear();
        let mut drawn = HashSet::new();
        drawn_images(&description.root, &mut drawn);
        self.decodes.retain(|svg| drawn.contains(&**svg));
    }
}
```

Replace the test-only `impl ElementImageCache` (`:137-163`) with:

```rust
#[cfg(test)]
impl ElementImageCache {
    pub(crate) fn image_for(&self, svg: &str) -> Option<Arc<RenderImage>> {
        self.decodes.state(svg).and_then(|state| state.picture())
    }

    pub(crate) fn state_for(&self, svg: &str) -> Option<SvgState> {
        self.decodes.state(svg)
    }

    /// The SVG texts the cache holds entries for, sorted.
    pub(crate) fn cached_texts(&self) -> Vec<String> {
        let mut texts: Vec<String> = self
            .decodes
            .keys()
            .iter()
            .map(|svg| svg.to_string())
            .collect();
        texts.sort_unstable();
        texts
    }

    /// How many decodes this cache has started.
    pub(crate) fn decodes(&self) -> usize {
        self.decodes.started().len()
    }

    pub(crate) fn text_lookups(&self) -> usize {
        self.text_lookups
    }

    pub(crate) fn retained_bytes(&self) -> usize {
        self.decodes.retained_bytes()
    }

    pub(crate) fn queue(&self) -> &SvgDecodes<Arc<str>> {
        &self.decodes
    }
}
```

In `impl ElementBody`, after `render` (`:193-209`), add:

```rust
    /// Starts the decodes the last `render` asked for, on the background
    /// executor. `cx` is the view that draws this body; it is told to draw
    /// again as each one finishes.
    pub(crate) fn start_decodes<T: 'static>(&self, cx: &mut Context<T>) {
        self.images.start_decodes(cx);
    }
```

In `crates/sprite-app/src/terminal_view/surfaces.rs`, `surface_element` (`:1129-1131`):

```rust
            Body::Elements(elements) => {
                elements.render(registry, &surface.connection, Some(cx.entity()))
            }
```

becomes:

```rust
            Body::Elements(elements) => {
                let body = elements.render(registry, &surface.connection, Some(cx.entity()));
                // After the frame is built, since the frame decides which
                // images are wanted.
                elements.start_decodes(cx);
                body
            }
```

Rewrite the existing tests that read decodes straight after a frame. They now start the decodes and let them finish.

`render.rs` — replace `element_image_cache_bounds_retained_pixels_and_resets` (`:676-741`):

```rust
    #[gpui::test]
    fn element_image_cache_bounds_retained_pixels_and_resets(cx: &mut gpui::TestAppContext) {
        let (ours, _peer) = UnixStream::pair().unwrap();
        let connection = SurfaceConnection::new(&ours).unwrap();
        let registry = TokenRegistry::new(&Colors::default());
        // Distinct texts: identical SVGs would share a single decode.
        let svg = |n: usize| {
            format!(
                "<svg xmlns='http://www.w3.org/2000/svg' width='2048' height='2048'><desc>{n}</desc></svg>"
            )
        };
        let nodes = (0..5)
            .map(|n| json!({"kind":"image","svg":svg(n)}))
            .collect::<Vec<_>>();
        let document = description::parse(
            &json!({"version":1,"root":{"kind":"box","children":nodes}}),
            &registry,
        )
        .unwrap()
        .description;
        let owner = cx.new(|_| Owner);
        let mut cache = ElementImageCache::default();
        draw_frame(cx, &owner, &mut cache, &document, &registry, &connection);
        cx.run_until_parked();
        let retained: usize = (0..5)
            .filter_map(|n| cache.image_for(&svg(n)))
            .map(|image| image.as_bytes(0).unwrap().len())
            .sum();
        assert_eq!(retained, 64 * 1024 * 1024);
        assert_eq!(cache.retained_bytes(), 64 * 1024 * 1024);
        // Two decodes run at once and either may finish first, so which of
        // the five misses out is not fixed; that exactly one does is.
        let refused: Vec<usize> = (0..5)
            .filter(|n| {
                matches!(
                    cache.state_for(&svg(*n)),
                    Some(SvgState::Refused(RefusalKind::OverBudget))
                )
            })
            .collect();
        assert_eq!(refused.len(), 1, "exactly one image is over the budget");
        let kept = (0..5).find(|n| !refused.contains(n)).unwrap();
        let first = cache.image_for(&svg(kept)).unwrap().id;
        draw_frame(cx, &owner, &mut cache, &document, &registry, &connection);
        cx.run_until_parked();
        assert_eq!(cache.image_for(&svg(kept)).unwrap().id, first);
        assert!(
            cache.image_for(&svg(refused[0])).is_none(),
            "redrawing the same description does not retry it"
        );
        cache = ElementImageCache::default();
        let fresh = description::parse(
            &json!({"version":1,"root":{"kind":"image","svg":svg(4)}}),
            &registry,
        )
        .unwrap()
        .description;
        draw_frame(cx, &owner, &mut cache, &fresh, &registry, &connection);
        cx.run_until_parked();
        assert!(cache.image_for(&svg(4)).is_some());
    }
```

`render.rs` — replace `retaining_a_new_description_releases_what_it_does_not_draw_and_retries_what_did_not_fit` (`:743-808`):

```rust
    #[gpui::test]
    fn retaining_a_new_description_releases_what_it_does_not_draw_and_retries_what_did_not_fit(
        cx: &mut gpui::TestAppContext,
    ) {
        let (ours, _peer) = UnixStream::pair().unwrap();
        let connection = SurfaceConnection::new(&ours).unwrap();
        let registry = TokenRegistry::new(&Colors::default());
        // Each one is a distinct 16 MiB raster: four fill the 64 MiB budget.
        let svg = |n: usize| {
            format!(
                "<svg xmlns='http://www.w3.org/2000/svg' width='2048' height='2048'><desc>{n}</desc></svg>"
            )
        };
        let document = |numbers: &[usize]| {
            description::parse(
                &json!({"version":1,"root":{"kind":"box","children":numbers
                    .iter()
                    .map(|n| json!({"kind":"image","svg":svg(*n)}))
                    .collect::<Vec<_>>()}}),
                &registry,
            )
            .unwrap()
            .description
        };
        let owner = cx.new(|_| Owner);
        let mut cache = ElementImageCache::default();
        draw_frame(cx, &owner, &mut cache, &document(&[0, 1, 2, 3, 4]), &registry, &connection);
        cx.run_until_parked();
        let refused = (0..5)
            .find(|n| cache.image_for(&svg(*n)).is_none())
            .expect("one image is over the budget");
        let kept_number = (0..5).find(|n| *n != refused).unwrap();
        let kept = cache.image_for(&svg(kept_number)).unwrap();
        let decodes = cache.decodes();

        let next = document(&[kept_number, refused]);
        cache.retain_drawn_by(&next);
        assert_eq!(cache.retained_bytes(), 16 * 1024 * 1024);
        assert!(
            cache.state_for(&svg(refused)).is_none(),
            "a refusal for lack of budget is forgotten with the old description"
        );
        draw_frame(cx, &owner, &mut cache, &next, &registry, &connection);
        cx.run_until_parked();
        // Entries hold the full SVG text and a lookup compares it, so the
        // cache holds exactly the two texts drawn, each under its own text.
        let mut drawn = vec![svg(kept_number), svg(refused)];
        drawn.sort_unstable();
        assert_eq!(cache.cached_texts(), drawn);
        assert!(Arc::ptr_eq(&kept, &cache.image_for(&svg(kept_number)).unwrap()));
        assert!(
            cache.image_for(&svg(refused)).is_some(),
            "the update freed room for it"
        );
        assert_eq!(cache.decodes(), decodes + 1);
        assert_eq!(cache.retained_bytes(), 32 * 1024 * 1024);
    }
```

`render.rs` — replace `an_unchanged_description_never_looks_its_svg_text_up_again` (`:807-869`):

```rust
    /// A description may carry megabytes of SVG text. It is looked up by that
    /// text once per description and once more after decodes finish; every
    /// other frame finds each image by its place in the tree, so a repaint
    /// never hashes or compares the text.
    #[gpui::test]
    fn an_unchanged_description_never_looks_its_svg_text_up_again(
        cx: &mut gpui::TestAppContext,
    ) {
        let (ours, _peer) = UnixStream::pair().unwrap();
        let connection = SurfaceConnection::new(&ours).unwrap();
        let registry = TokenRegistry::new(&Colors::default());
        let svg = |n: usize| {
            format!(
                "<svg xmlns='http://www.w3.org/2000/svg' width='4' height='4'><desc>{n}</desc></svg>"
            )
        };
        let document = |numbers: &[usize]| {
            description::parse(
                &json!({"version":1,"root":{"kind":"box","children":numbers
                    .iter()
                    .map(|n| json!({"kind":"box","children":[{"kind":"image","svg":svg(*n)}]}))
                    .collect::<Vec<_>>()}}),
                &registry,
            )
            .unwrap()
            .description
        };
        let owner = cx.new(|_| Owner);
        let mut cache = ElementImageCache::default();
        let first = document(&[0, 1, 0]);
        draw_frame(cx, &owner, &mut cache, &first, &registry, &connection);
        assert_eq!(cache.text_lookups(), 3, "the first frame finds each image");
        assert_eq!(cache.decodes(), 2, "the repeated SVG shares one decode");
        cx.run_until_parked();
        draw_frame(cx, &owner, &mut cache, &first, &registry, &connection);
        assert_eq!(
            cache.text_lookups(),
            6,
            "the frame after the decodes finished finds each image once more"
        );
        let shown = cache.image_for(&svg(1)).unwrap();
        for _ in 0..5 {
            draw_frame(cx, &owner, &mut cache, &first, &registry, &connection);
        }
        assert_eq!(
            cache.text_lookups(),
            6,
            "later frames find every picture by position"
        );
        assert!(Arc::ptr_eq(&shown, &cache.image_for(&svg(1)).unwrap()));

        // An update is looked up afresh, once, and keeps the decodes it still
        // draws wherever they moved in the tree.
        let moved = document(&[1, 2]);
        cache.retain_drawn_by(&moved);
        draw_frame(cx, &owner, &mut cache, &moved, &registry, &connection);
        assert_eq!(cache.text_lookups(), 8);
        cx.run_until_parked();
        draw_frame(cx, &owner, &mut cache, &moved, &registry, &connection);
        draw_frame(cx, &owner, &mut cache, &moved, &registry, &connection);
        assert_eq!(cache.text_lookups(), 10);
        assert_eq!(cache.decodes(), 3, "only the new SVG is decoded");
        assert!(Arc::ptr_eq(&shown, &cache.image_for(&svg(1)).unwrap()));
    }
```

`render.rs` — replace `legacy_image_redraw_reuses_decode_and_new_cache_decodes_changed_svg` (`:961-1017`):

```rust
    #[gpui::test]
    fn legacy_image_redraw_reuses_decode_and_new_cache_decodes_changed_svg(
        cx: &mut gpui::TestAppContext,
    ) {
        let (ours, _theirs) = UnixStream::pair().unwrap();
        let connection = SurfaceConnection::new(&ours).unwrap();
        let registry = TokenRegistry::new(&Colors::default());
        let svg = |color: &str| {
            format!(
                "<svg xmlns='http://www.w3.org/2000/svg' width='4' height='4'><rect width='4' height='4' fill='{color}'/></svg>"
            )
        };
        let document = |color: &str| {
            description::parse(
                &json!({"version":1,"root":{"kind":"image","style":"w_4 h_4","svg":svg(color)}}),
                &registry,
            )
            .unwrap()
            .description
        };
        let owner = cx.new(|_| Owner);
        let blue = document("blue");
        let mut cache = ElementImageCache::default();
        draw_frame(cx, &owner, &mut cache, &blue, &registry, &connection);
        cx.run_until_parked();
        let first = cache.image_for(&svg("blue")).unwrap();
        draw_frame(cx, &owner, &mut cache, &blue, &registry, &connection);
        cx.run_until_parked();
        assert!(Arc::ptr_eq(&first, &cache.image_for(&svg("blue")).unwrap()));
        let first_id = first.id;
        drop(first);
        let red = document("red");
        cache = ElementImageCache::default();
        draw_frame(cx, &owner, &mut cache, &red, &registry, &connection);
        cx.run_until_parked();
        let second = cache.image_for(&svg("red")).unwrap();
        assert_eq!(&second.as_bytes(0).unwrap()[..4], &[0, 0, 255, 255]);
        assert_ne!(second.id, first_id);
    }
```

The other `render.rs` tests stay as they are. They never read a decode:
- `an_unchanged_description_builds_its_click_ids_once`
- `click_ids_and_element_walk_the_tree_in_the_same_order`
- `every_kind_becomes_an_element_without_a_window`. Its image now draws as a pending empty `div`, which is still built without a window.
- the `svg_raster_*` and `text_only_svg_renders_visible_pixels` tests.

`surfaces.rs` — next to `draw_test_window` (`:1875`), add:

```rust
    /// Builds the fill Surface's elements as a frame does, starts the
    /// decodes that frame asked for, and lets them all finish.
    fn settle_fill_images(
        host: &Entity<TerminalView>,
        cx: &mut gpui::VisualTestContext,
        registry: &TokenRegistry,
    ) {
        host.update(cx, |view, cx| {
            let surface = view.surfaces.fill.as_mut().unwrap();
            let connection = surface.connection.clone();
            let Body::Elements(elements) = &mut surface.body else {
                panic!("expected elements")
            };
            let _ = elements.render(registry, &connection, None);
            elements.start_decodes(cx);
        });
        cx.run_until_parked();
    }

    /// The picture the fill Surface's cache holds for `svg`, once its decode
    /// is ready.
    fn fill_image(view: &TerminalView, svg: &str) -> Option<std::sync::Arc<gpui::RenderImage>> {
        let Body::Elements(elements) = &view.surfaces.fill.as_ref().unwrap().body else {
            panic!("expected elements")
        };
        elements.images().image_for(svg)
    }
```

`surfaces.rs` — in `legacy_svg_cache_follows_description_update_and_surface_close` (`:1629-1705`), replace everything from `let (first, again) = host.update(cx, |view, _| {` through the `assert_ne!(first, replacement);` line with:

```rust
        let registry = TokenRegistry::new(&settings.colors);
        settle_fill_images(&host, cx, &registry);
        let first = host.read_with(cx, |view, _| fill_image(view, &svg("blue")).unwrap().id);
        settle_fill_images(&host, cx, &registry);
        let again = host.read_with(cx, |view, _| fill_image(view, &svg("blue")).unwrap().id);
        assert_eq!(first, again);
        host.update(cx, |view, cx| {
            view.update_surface(SurfaceId(2), document("red"), cx);
            assert!(
                fill_image(view, &svg("blue")).is_none(),
                "blue is no longer drawn, so its decode was released"
            );
        });
        settle_fill_images(&host, cx, &registry);
        let replacement =
            host.read_with(cx, |view, _| fill_image(view, &svg("red")).unwrap().id);
        assert_ne!(first, replacement);
```

`surfaces.rs` — in `an_update_keeps_the_decode_of_an_unchanged_svg_and_decodes_a_changed_one_once` (`:1707-1808`):
- Replace the `frame` closure (from its `// Builds the Surface's elements as a frame does…` comment through `};`) with:

```rust
        // Draws the Surface as a frame does and lets its decodes finish, then
        // reports how many SVGs its cache has decoded in all and what it holds
        // for each colour.
        let frame = |cx: &mut gpui::VisualTestContext, colors: &[&str]| {
            settle_fill_images(&host, cx, &registry);
            host.read_with(cx, |view, _| {
                let Body::Elements(elements) = &view.surfaces.fill.as_ref().unwrap().body else {
                    panic!("expected elements")
                };
                let images = elements.images();
                (
                    images.decodes(),
                    colors
                        .iter()
                        .map(|color| images.image_for(&svg(color)))
                        .collect::<Vec<_>>(),
                )
            })
        };
```

- Replace its three calls:
  - `host.update(cx, |view, _| frame(view, &["blue", "green"]))` (twice) becomes `frame(cx, &["blue", "green"])`.
  - `host.update(cx, |view, _| frame(view, &["blue", "green", "red"]))` becomes `frame(cx, &["blue", "green", "red"])`.

  The assertions after each call stay unchanged.

- [ ] **Step 6: Run the tests to verify they pass, then the suite and clippy**

Run: `env PATH=/Users/davidlee/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin:/Users/davidlee/.cargo/bin:/opt/homebrew/bin:/usr/bin:/bin cargo test -p sprite-app --locked --offline svg`
Expected: all pass, including `an_svg_is_decoded_off_the_ui_thread_and_its_view_told_when_it_is_ready`.

Run: `env PATH=/Users/davidlee/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin:/Users/davidlee/.cargo/bin:/opt/homebrew/bin:/usr/bin:/bin cargo test -p sprite-app --locked --offline surface::render`
Expected: all pass. That includes the four new tests and the rewritten `element_image_cache_bounds_retained_pixels_and_resets`, `retaining_a_new_description_releases_what_it_does_not_draw_and_retries_what_did_not_fit`, `an_unchanged_description_never_looks_its_svg_text_up_again` and `legacy_image_redraw_reuses_decode_and_new_cache_decodes_changed_svg`.

Run: `env PATH=/Users/davidlee/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin:/Users/davidlee/.cargo/bin:/opt/homebrew/bin:/usr/bin:/bin cargo test -p sprite-app --locked --offline terminal_view::surfaces`
Expected: all pass.

Run: `env PATH=/Users/davidlee/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin:/Users/davidlee/.cargo/bin:/opt/homebrew/bin:/usr/bin:/bin cargo clippy -p sprite-app --all-targets --locked --offline -- -D warnings`
Expected: no warnings.

- [ ] **Step 7: Commit**

```bash
git -C /Users/davidlee/Projects/Sprite/.worktrees/close-open-findings add crates/sprite-app/src/surface/svg_decode.rs crates/sprite-app/src/surface/render.rs crates/sprite-app/src/terminal_view/surfaces.rs
git -C /Users/davidlee/Projects/Sprite/.worktrees/close-open-findings commit -m "$(cat <<'EOF'
Decode element-Surface SVGs on the background executor

A frame only asks for an image; a pending one draws nothing. Each cache
runs two decodes at a time, oldest first, charges a finished picture to
its budget when installing it, discards one its description stopped
drawing, and tells the view to draw again.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
EOF
)"
```

---

---

### Task 11: List assets decode on the background executor; amend ADR 0022 (R-V1, R-V4, R-V5, R-V6)

**Files:**
- Modify: `crates/sprite-app/src/terminal_view/list_view.rs`:
  - `:22-24` (imports)
  - `:50-86` (`sync_image_cache`)
  - `:155-170` (struct)
  - `:193-212` (`new`)
  - `:409-445` (`render`)
  - tests `:929-1069`: three tests rewritten, one new test
- Modify: `crates/sprite-app/src/surface/svg_decode.rs` (`reset`; `decode`, `raster_bytes`, `MAX_SURFACE_IMAGE_BYTES` become private)
- Modify: `docs/adr/0022-bound-surface-raster-memory-at-the-decoder.md` (append the amendment)

**Interfaces:**
- Consumes: `SvgDecodes<K>::{new, want, start}`, `SvgState`, `RefusalKind` (Task 10). `crate::surface::list::parse_op(&Value) -> Result<ListOp, Refusal>`. `TerminalView::list_operation(&mut self, SurfaceId, ListOp, &mut Context<Self>)`.
- Produces:
  - `pub(crate) fn SvgDecodes::reset(&self, target_width: Option<f32>)`
  - `fn sync_image_cache(cache: &mut Arc<BTreeMap<String, Option<Arc<RenderImage>>>>, decodes: &SvgDecodes<String>, assets: &BTreeMap<String, String>, ids: impl IntoIterator<Item = String>)`
  - `svg_decode::decode` is no longer visible outside its module.

- [ ] **Step 1: Write the failing seam test (compiles against the current API)**

In `crates/sprite-app/src/terminal_view/list_view.rs` `mod tests`, add:

```rust
    #[gpui::test]
    fn a_list_icon_is_decoded_off_the_ui_thread_and_its_list_told_when_it_is_ready(
        cx: &mut gpui::TestAppContext,
    ) {
        let settings = crate::config::Settings::default();
        cx.set_global(crate::config::ActiveSettings(settings.clone()));
        cx.set_global(TokenRegistry::new(&settings.colors));
        let (host, cx) = cx.add_window_view(|window, cx| {
            TerminalView::failed(
                "test".to_owned(),
                SharedString::from(".SystemUIFont"),
                window,
                cx,
            )
        });
        let (stream, _peer) = std::os::unix::net::UnixStream::pair().unwrap();
        let connection = crate::surface::channel::SurfaceConnection::new(&stream).unwrap();
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/surface-list-v1.json"
        ))
        .unwrap();
        host.update_in(cx, |view, window, cx| {
            view.open_surface(
                SurfaceId(1),
                crate::surface::channel::Open {
                    placement: crate::surface::channel::Placement::Fill { owner_pid: None },
                    focus: false,
                    description: fixture["description"].clone(),
                },
                connection,
                window,
                cx,
            )
            .unwrap();
        });
        // The fixture's section header draws the "chevron-down" icon on every
        // frame. Make that icon the shape that once took seconds to decode:
        // differently named flood results, all merged by one feMerge.
        let floods: String = (0..4)
            .map(|n| format!("<feFlood flood-color='#{:06x}' result='r{n}'/>", n * 0x40))
            .collect();
        let merges: String = (0..4)
            .map(|n| format!("<feMergeNode in='r{n}'/>"))
            .collect();
        let svg = format!(
            "<svg xmlns='http://www.w3.org/2000/svg' width='16' height='16'>\
             <filter id='f' x='0' y='0' width='1' height='1'>{floods}<feMerge>{merges}</feMerge></filter>\
             <rect width='16' height='16' filter='url(#f)'/></svg>"
        );
        let assets = crate::surface::list::parse_op(
            &serde_json::json!({"type":"assets","entries":{"chevron-down":svg}}),
        )
        .unwrap();
        host.update(cx, |view, cx| view.list_operation(SurfaceId(1), assets, cx));
        let list = host.read_with(cx, |view, _| match &view.surfaces.fill.as_ref().unwrap().body {
            super::super::surfaces::Body::List { view, .. } => view.clone(),
            super::super::surfaces::Body::Elements(_)
            | super::super::surfaces::Body::Grid { .. } => panic!("expected a list"),
        });
        let notified = std::rc::Rc::new(std::cell::Cell::new(0usize));
        let counter = notified.clone();
        let _observer =
            cx.update(|_, cx| cx.observe(&list, move |_, _| counter.set(counter.get() + 1)));
        let draw = |cx: &mut gpui::VisualTestContext| {
            cx.update(|window, cx| {
                window.refresh();
                window.draw(cx).clear();
            });
        };
        let icon = |cx: &mut gpui::VisualTestContext| {
            list.read_with(cx, |list, _| list.images.get("chevron-down").cloned())
        };

        draw(cx);
        assert!(
            icon(cx).is_none(),
            "the frame decoded the icon on the UI thread"
        );
        let before = notified.get();
        cx.run_until_parked();
        assert!(
            notified.get() > before,
            "the finished decode told the list to draw again"
        );
        draw(cx);
        assert!(
            icon(cx).flatten().is_some(),
            "the next frame draws the decoded icon"
        );
        cx.update(|window, _| window.remove_window());
        drop(host);
    }
```

- [ ] **Step 2: Run it to verify it fails for the stated reason**

Run: `env PATH=/Users/davidlee/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin:/Users/davidlee/.cargo/bin:/opt/homebrew/bin:/usr/bin:/bin cargo test -p sprite-app --locked --offline a_list_icon_is_decoded_off_the_ui_thread`
Expected: FAIL with `the frame decoded the icon on the UI thread`. `sync_image_cache` decoded the section icon inside `VirtualListView::render`.

- [ ] **Step 3: Implement**

In `crates/sprite-app/src/surface/svg_decode.rs`:
- `pub(crate) const MAX_SURFACE_IMAGE_BYTES` becomes `const MAX_SURFACE_IMAGE_BYTES`.
- `pub(crate) fn decode(` becomes `fn decode(`, with its doc comment extended by one line: `/// Only a decode task calls this, never a frame.`
- `pub(crate) fn raster_bytes(` becomes `fn raster_bytes(`.
- In `impl<K: Clone + Eq + Hash + 'static> SvgDecodes<K>`, after `retain`, add:

```rust
    /// Forgets every entry and waiting decode, to draw from now on at
    /// `target_width`. A decode still running is discarded when it finishes.
    pub(crate) fn reset(&self, target_width: Option<f32>) {
        let mut queue = self.queue.borrow_mut();
        queue.entries.clear();
        queue.waiting.clear();
        queue.retained_bytes = 0;
        queue.target_width = target_width;
    }
```

In `crates/sprite-app/src/terminal_view/list_view.rs`:
- Lines 22-24 become:

```rust
#[cfg(test)]
use crate::surface::svg_decode::render_svg;
use crate::surface::svg_decode::{SvgDecodes, SvgState};
```

- Replace `sync_image_cache` (`:50-86`) with:

```rust
/// Asks for the pictures of the visible assets and keeps what rows draw: a
/// picture once its decode is ready, an empty slot once it was refused. A
/// pending asset is left out, so its row draws an empty slot until the
/// decode finishes and the list is told to draw again. Every outcome is kept
/// until the raster width changes, as assets cannot be redefined.
fn sync_image_cache(
    cache: &mut Arc<BTreeMap<String, Option<Arc<RenderImage>>>>,
    decodes: &SvgDecodes<String>,
    assets: &BTreeMap<String, String>,
    ids: impl IntoIterator<Item = String>,
) {
    let mut settled = Vec::new();
    for id in ids {
        if cache.contains_key(&id) {
            continue;
        }
        let Some(svg) = assets.get(&id) else {
            continue;
        };
        match decodes.want(id.as_str(), || (id.clone(), Arc::from(svg.as_str()))) {
            SvgState::Pending => {}
            SvgState::Ready(picture) => settled.push((id, Some(picture))),
            SvgState::Refused(_) => settled.push((id, None)),
        }
    }
    if settled.is_empty() {
        return;
    }
    let mut images = (**cache).clone();
    images.extend(settled);
    *cache = Arc::new(images);
}
```

- In `struct VirtualListView` (`:155-170`), after `images: Arc<BTreeMap<String, Option<Arc<RenderImage>>>>,` add:

```rust
    /// The decodes behind `images`, drawn `raster_width` pixels wide.
    decodes: SvgDecodes<String>,
```

- In `VirtualListView::new` (`:193-212`), after `images: Arc::new(BTreeMap::new()),` add:

```rust
            decodes: SvgDecodes::new(None),
```

- In `impl Render for VirtualListView::render`, the width check (`:414-417`):

```rust
        if self.raster_width != raster_width {
            self.raster_width = raster_width;
            self.images = Arc::new(BTreeMap::new());
        }
```

  becomes:

```rust
        if self.raster_width != raster_width {
            self.raster_width = raster_width;
            self.images = Arc::new(BTreeMap::new());
            self.decodes.reset(Some(raster_width));
        }
```

- In the same function, the call `sync_image_cache(&mut self.images, &self.model.assets, icons, raster_width);` (`:445`) becomes:

```rust
        sync_image_cache(&mut self.images, &self.decodes, &self.model.assets, icons);
        self.decodes.start(cx);
```

In `list_view.rs` `mod tests` (`:929`), after `use super::*;` add:

```rust
    use crate::surface::svg_decode::RefusalKind;
    use gpui::AppContext as _;

    /// Stands in for the list view: a finished decode tells this entity to
    /// draw again.
    struct Owner;

    /// Asks for `ids` as a frame does, lets every decode finish, then takes
    /// up what they came to, as the next frame does.
    fn settle(
        cx: &mut gpui::TestAppContext,
        owner: &gpui::Entity<Owner>,
        cache: &mut Arc<BTreeMap<String, Option<Arc<RenderImage>>>>,
        decodes: &SvgDecodes<String>,
        assets: &BTreeMap<String, String>,
        ids: &[&str],
    ) {
        let wanted = || ids.iter().map(|id| (*id).to_owned());
        sync_image_cache(cache, decodes, assets, wanted());
        owner.update(cx, |_, cx| decodes.start(cx));
        cx.run_until_parked();
        sync_image_cache(cache, decodes, assets, wanted());
    }
```

Replace `visible_list_assets_bound_retained_pixels_and_reset` (`:984-1016`):

```rust
    #[gpui::test]
    fn visible_list_assets_bound_retained_pixels_and_reset(cx: &mut gpui::TestAppContext) {
        let owner = cx.new(|_| Owner);
        let mut cache = Arc::new(BTreeMap::new());
        let decodes = SvgDecodes::new(Some(2048.0));
        let assets: BTreeMap<String, String> = (0..5)
            .map(|i| {
                (
                    i.to_string(),
                    "<svg xmlns='http://www.w3.org/2000/svg' width='2048' height='2048'/>"
                        .to_owned(),
                )
            })
            .collect();
        settle(cx, &owner, &mut cache, &decodes, &assets, &["0", "1", "2", "3"]);
        assert_eq!(cache.values().flatten().count(), 4);
        settle(cx, &owner, &mut cache, &decodes, &assets, &["4"]);
        let retained: usize = cache
            .values()
            .flatten()
            .map(|image| image.as_bytes(0).unwrap().len())
            .sum();
        assert_eq!(retained, 64 * 1024 * 1024);
        assert!(cache["4"].is_none());
        assert!(matches!(
            decodes.state("4"),
            Some(SvgState::Refused(RefusalKind::OverBudget))
        ));
        let first = cache.clone();
        settle(cx, &owner, &mut cache, &decodes, &assets, &["4"]);
        assert!(Arc::ptr_eq(&cache, &first));
        assert_eq!(
            decodes.started().len(),
            5,
            "a list keeps a refusal rather than decoding again"
        );
        drop(first);
        cache = Arc::new(BTreeMap::new());
        decodes.reset(Some(1024.0));
        settle(cx, &owner, &mut cache, &decodes, &assets, &["4"]);
        assert_eq!(
            cache["4"].as_ref().unwrap().as_bytes(0).unwrap().len(),
            4 * 1024 * 1024
        );
    }
```

Replace `accepted_duplicate_assets_reuse_cache_and_image_arcs` (`:1018-1038`):

```rust
    #[gpui::test]
    fn accepted_duplicate_assets_reuse_cache_and_image_arcs(cx: &mut gpui::TestAppContext) {
        let owner = cx.new(|_| Owner);
        let mut cache = Arc::new(BTreeMap::new());
        let decodes = SvgDecodes::new(Some(2.0));
        let mut assets = BTreeMap::new();
        assets.insert(
            "first".to_owned(),
            "<svg xmlns='http://www.w3.org/2000/svg' width='2' height='2'/>".to_owned(),
        );
        settle(cx, &owner, &mut cache, &decodes, &assets, &["first"]);
        let old_cache = cache.clone();
        let old_image = cache["first"].as_ref().unwrap().clone();
        settle(cx, &owner, &mut cache, &decodes, &assets, &["first"]);
        assert!(Arc::ptr_eq(&cache, &old_cache));
        assets.insert(
            "second".to_owned(),
            "<svg xmlns='http://www.w3.org/2000/svg' width='2' height='2'/>".to_owned(),
        );
        settle(cx, &owner, &mut cache, &decodes, &assets, &["second"]);
        assert!(Arc::ptr_eq(cache["first"].as_ref().unwrap(), &old_image));
        assert_eq!(cache.len(), 2);
    }
```

Replace `invalid_visible_svg_is_cached_as_an_empty_slot` (`:1039-1048`):

```rust
    #[gpui::test]
    fn invalid_visible_svg_is_cached_as_an_empty_slot(cx: &mut gpui::TestAppContext) {
        let owner = cx.new(|_| Owner);
        let mut cache = Arc::new(BTreeMap::new());
        let decodes = SvgDecodes::new(Some(32.0));
        let assets = BTreeMap::from([("broken".to_owned(), "<svg invalid".to_owned())]);
        settle(cx, &owner, &mut cache, &decodes, &assets, &["broken"]);
        assert!(cache["broken"].is_none());
        assert!(matches!(
            decodes.state("broken"),
            Some(SvgState::Refused(RefusalKind::Unparsable))
        ));
        let first = cache.clone();
        settle(cx, &owner, &mut cache, &decodes, &assets, &["broken"]);
        assert!(Arc::ptr_eq(&first, &cache));
        assert_eq!(decodes.started().len(), 1, "an unparsable asset is decoded once");
    }
```

`svg_cache_keeps_blue_red_and_alpha_channels_in_renderer_order` stays as it is. `render_svg` now comes from `svg_decode` through the `#[cfg(test)]` import above.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `env PATH=/Users/davidlee/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin:/Users/davidlee/.cargo/bin:/opt/homebrew/bin:/usr/bin:/bin cargo test -p sprite-app --locked --offline list_view`
Expected: all pass, including `a_list_icon_is_decoded_off_the_ui_thread_and_its_list_told_when_it_is_ready` and the three rewritten tests.

- [ ] **Step 5: Amend ADR 0022**

Append to `docs/adr/0022-bound-surface-raster-memory-at-the-decoder.md`. Replace the bracketed sentence with the `cap:` line printed in Task 9 Step 5, copied verbatim after `cap: `. It is also on `MAX_FILTER_PRIMITIVE_USES` after `Measured:`.

```markdown

## Amendment, October 2026

Decoding is asynchronous, and filter work is bounded by counting the uses of filter primitives.

Neither cache parses or rasterises SVG on the UI thread. An image is `Pending` from the first frame that draws it until its decode finishes on GPUI's background executor. A pending image draws nothing, and the finished decode tells the view that drew it to draw again. Each cache runs at most `MAX_CONCURRENT_SVG_DECODES` (2) decodes at a time, oldest request first. The memory that filter intermediates take is therefore bounded per cache by two capped decodes, not by how many images a description holds. The font database is first loaded by a decode, so it too is loaded on the background executor.

The 64MiB cache budget is charged when a finished decode is installed. A result that no longer fits is dropped, and a result whose image the description no longer draws is discarded. An element Surface forgets a refusal for lack of budget when its next description arrives, and tries again. A document that does not parse, applies too many filter primitives, or cannot be one raster stays refused for as long as a description draws it. A list keeps every outcome until its icon raster width changes, as before.

Before rasterising, a decode walks the parsed tree, including clip paths, masks, patterns, nested images and text. It adds up the primitives of every filter each group applies, so a filter shared by two groups counts twice. A document with more than `MAX_FILTER_PRIMITIVE_USES` uses is refused. The cap is the largest measured count whose 512x512 document, made of differently named `feFlood` results merged by one `feMerge`, decodes in under 250 ms in a release build: [the measured `cap:` line]. The measurement is a manual run of the ignored test `measure_filter_cap`. No test asserts on time.

Parse cost is bounded by the existing Surface Channel message limit (16MiB) and list asset limit (64MiB), not by the cap. CPU time spent on content other than filters, and memory across Surfaces, remain outside this decision.
```

- [ ] **Step 6: Run the suite, clippy, and the UI-thread checks**

Run: `env PATH=/Users/davidlee/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin:/Users/davidlee/.cargo/bin:/opt/homebrew/bin:/usr/bin:/bin cargo test -p sprite-app --locked --offline`
Expected: all pass.

Run: `env PATH=/Users/davidlee/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin:/Users/davidlee/.cargo/bin:/opt/homebrew/bin:/usr/bin:/bin cargo clippy -p sprite-app --all-targets --locked --offline -- -D warnings`
Expected: no warnings.

Run: `grep -rn "load_system_fonts\|FONT_DB\|fn decode\|decode(&svg" crates/sprite-app/src`
Expected: every hit is in `crates/sprite-app/src/surface/svg_decode.rs`. `decode` is private there, and its one non-test caller is the background closure in `start_waiting`. No UI path can reach the font database or the rasteriser (R-V1, R-V4).

- [ ] **Step 7: Commit**

```bash
git -C /Users/davidlee/Projects/Sprite/.worktrees/close-open-findings add crates/sprite-app/src/surface/svg_decode.rs crates/sprite-app/src/terminal_view/list_view.rs docs/adr/0022-bound-surface-raster-memory-at-the-decoder.md
git -C /Users/davidlee/Projects/Sprite/.worktrees/close-open-findings commit -m "$(cat <<'EOF'
Decode list assets on the background executor and amend ADR 0022

List icons now go through the same two-at-a-time decode queue as element
images, and the decoder is private to it, so no frame can parse or
rasterise SVG. ADR 0022 records the asynchronous decode, the
filter-primitive cap and the measurement it was chosen from.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 12: Ledger, gate, hosted runs and the PR (R-X, R-L, R-W4, Evidence)

**Files:**
- Modify: `docs/reviews/findings.toml:462-580` (the ten GAP-C entries below)
- Not committed: `$S/pr-body.md`, `$S/gate-tests.log`, `$S/gate.out` (`S` as in Task 2)
- Nothing under `vendor/` changes here. The GPUI patch and `vendor/gpui.provenance.md` belong to the quit task; this task only runs `scripts/check_gpui_patch.py` to show they agree.

**Interfaces:**
- Consumes: every earlier task's regression tests (names below), their recorded failures at `03b4929`, Task 2's loop output and CI measurements, the quit task's manual Dock-quit record (R-Q5).
- Produces: a ledger with no `open` or `deferred` entry; the PR; five `workflow_dispatch` Arch runs on the PR head.

`fixed_by` must be a string (`check_findings.py` rejects anything else). The PR number does not exist until the PR does, so this task does what #76 did (`e233fa3`, then `9943c5a`): the ledger first says `fixed_by = "fix/close-open-findings"`, the PR is opened as a draft, and one commit replaces the branch name with `"#N"`. The five dispatched runs come after that commit so they test the PR's final head.

- [ ] **Step 1: Confirm every cited test exists**

The names are fixed by the earlier tasks; adjust a row if a task renamed one.

| ID | `test` |
|---|---|
| GAP-C01 | `a_platform_quit_request_is_allowed_only_after_pane_cleanup`, `a_platform_quit_request_with_a_busy_pane_asks_then_the_next_one_quits` |
| GAP-C03 | `an_svg_is_decoded_off_the_ui_thread_and_its_view_told_when_it_is_ready`, `a_document_over_the_filter_cap_is_refused_never_rasterised_and_not_tried_again` |
| GAP-C04 | `test_distribution_pkgbuild_pins_the_release_tag` (Python, Task 1) |
| GAP-C06 | `a_gesture_whose_anchor_was_rotated_out_with_zero_scrollback_selects_nothing`, `a_gesture_whose_anchor_was_rotated_out_on_the_alternate_screen_selects_nothing` |
| GAP-C07 | `a_reader_slower_than_the_drain_floor_loses_its_slot` |
| GAP-C08 | `placeholder_tiles_land_on_this_frames_cells_after_a_scale_change` |
| GAP-C10 | `a_running_program_is_reported_by_name`, `natural_exit_deadline_is_not_renewed_by_continuous_output` |
| GAP-C16 | `a_placeholder_without_diacritics_continues_its_left_neighbour` |

Run: `cd /Users/davidlee/Projects/Sprite/.worktrees/close-open-findings && grep -rnE "fn (a_platform_quit_request_is_allowed_only_after_pane_cleanup|a_platform_quit_request_with_a_busy_pane_asks_then_the_next_one_quits|an_svg_is_decoded_off_the_ui_thread_and_its_view_told_when_it_is_ready|a_document_over_the_filter_cap_is_refused_never_rasterised_and_not_tried_again|a_gesture_whose_anchor_was_rotated_out_with_zero_scrollback_selects_nothing|a_gesture_whose_anchor_was_rotated_out_on_the_alternate_screen_selects_nothing|a_reader_slower_than_the_drain_floor_loses_its_slot|placeholder_tiles_land_on_this_frames_cells_after_a_scale_change|a_running_program_is_reported_by_name|natural_exit_deadline_is_not_renewed_by_continuous_output|a_placeholder_without_diacritics_continues_its_left_neighbour)\b" crates; grep -n "def test_distribution_pkgbuild_pins_the_release_tag" scripts/test_packaging_automation.py`
Expected: exactly twelve lines, one per name. `check_findings.py` in Step 2 also proves each Rust one carries `#[test]` or `#[gpui::test]`.

- [ ] **Step 2: Update the ledger**

In `docs/reviews/findings.toml` replace each of the ten entries from `id = "GAP-C01"` (line 462) to the end of the file, keeping the five resolved entries between them (GAP-C02, C05, C09, C11, C12, C13) byte-for-byte. Each replacement drops the `issue` line. The new text for each entry, in file order:

```toml
[[finding]]
id = "GAP-C01"
title = "Dock or system termination may bypass the window cleanup gate"
status = "resolved"
source = "docs/reviews/sprite-gap-audit.md"
fixed_by = "fix/close-open-findings"
test = [
  "a_platform_quit_request_is_allowed_only_after_pane_cleanup",
  "a_platform_quit_request_with_a_busy_pane_asks_then_the_next_one_quits",
]
```

```toml
[[finding]]
id = "GAP-C03"
title = "Small SVG filter documents cost seconds of synchronous decoder work on the UI thread"
status = "resolved"
source = "docs/reviews/sprite-gap-audit.md"
fixed_by = "fix/close-open-findings"
test = [
  "an_svg_is_decoded_off_the_ui_thread_and_its_view_told_when_it_is_ready",
  "a_document_over_the_filter_cap_is_refused_never_rasterised_and_not_tried_again",
]
```

```toml
[[finding]]
id = "GAP-C04"
title = "The distribution PKGBUILD labels a fixed version but fetches unpinned git HEAD"
status = "resolved"
source = "docs/reviews/sprite-gap-audit.md"
fixed_by = "fix/close-open-findings"
test = "test_distribution_pkgbuild_pins_the_release_tag"
```

```toml
[[finding]]
id = "GAP-C06"
title = "BCA-26's zero-scrollback anchor drift is reachable: ScrollbackBytes accepts 0"
status = "resolved"
source = "docs/PRDs/10-09-2026-bug-class-audit.md"
fixed_by = "fix/close-open-findings"
test = [
  "a_gesture_whose_anchor_was_rotated_out_with_zero_scrollback_selects_nothing",
  "a_gesture_whose_anchor_was_rotated_out_on_the_alternate_screen_selects_nothing",
]
```

```toml
[[finding]]
id = "GAP-C07"
title = "A slowly draining Surface writer keeps one of the 64 connection slots"
status = "resolved"
source = "docs/reviews/pr-58-follow-up.md"
fixed_by = "fix/close-open-findings"
test = "a_reader_slower_than_the_drain_floor_loses_its_slot"
```

```toml
[[finding]]
id = "GAP-C08"
title = "Placeholder tiles snap against the previous frame's origin"
status = "resolved"
source = "docs/reviews/pr-58-follow-up.md"
fixed_by = "fix/close-open-findings"
test = "placeholder_tiles_land_on_this_frames_cells_after_a_scale_change"
```

```toml
[[finding]]
id = "GAP-C10"
title = "Arch CI fails intermittently on timing-sensitive tests"
status = "resolved"
source = "docs/PRDs/10-12-2026-findings-ledger-and-admission.md"
fixed_by = "fix/close-open-findings"
test = [
  "a_running_program_is_reported_by_name",
  "natural_exit_deadline_is_not_renewed_by_continuous_output",
]
```

```toml
[[finding]]
id = "GAP-C14"
title = "Cached GPUI views per pane"
status = "wontfix"
source = "docs/PRDs/10-09-2026-bug-class-audit.md"
reason = "An unchanged pane prepares in about 0.3 ms with no allocations and no shaping (docs/performance/bug-class-audit.md), and shaping_happens_only_for_cells_whose_drawn_text_changed guards that. Caching views would also make a pane miss resizes, because set_allocated does not notify."
```

```toml
[[finding]]
id = "GAP-C15"
title = "Divider-drag per-move layout work"
status = "wontfix"
source = "docs/PRDs/10-09-2026-bug-class-audit.md"
reason = "The work per move is linear and test-bounded (placement_matching_scales_linearly, title_membership_reconciliation_scales_linearly), and PTY resizes happen only when the drag crosses a cell boundary. The earlier claim that docs/performance/ holds divider-drag numbers was wrong; this reason replaces it."
```

```toml
[[finding]]
id = "GAP-C16"
title = "Kitty Unicode-placeholder implicit row and column inheritance"
status = "resolved"
source = "docs/PRDs/10-09-2026-bug-class-audit.md"
fixed_by = "fix/close-open-findings"
test = "a_placeholder_without_diacritics_continues_its_left_neighbour"
```

The comment above GAP-C01 (`# issue = 0 marks an issue not yet filed; the checker fails until it is.`) stays; it describes the format.

Run:

```bash
cd /Users/davidlee/Projects/Sprite/.worktrees/close-open-findings
python3 scripts/check_findings.py
python3 -m unittest scripts.test_findings -v
grep -cE '^status = "(open|deferred)"' docs/reviews/findings.toml
```

Expected: `every finding is guarded by a test or tracked by an issue`; `OK`; `0` (grep exits 1 on a zero count; that is the expected result).

Commit:

```bash
git -C /Users/davidlee/Projects/Sprite/.worktrees/close-open-findings add docs/reviews/findings.toml
git -C /Users/davidlee/Projects/Sprite/.worktrees/close-open-findings commit -m "$(cat <<'EOF'
docs: the ledger's last ten findings: eight resolved, two wontfix

GAP-C01, C03, C04, C06, C07, C08, C10 and C16 cite their regression
tests; GAP-C14 and C15 record why they are not worth doing. No entry
is open or deferred. fixed_by names the branch until the PR exists.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
EOF
)"
```

- [ ] **Step 3: The gate**

```bash
W=/Users/davidlee/Projects/Sprite/.worktrees/close-open-findings
S=/private/tmp/claude-502/-Users-davidlee/cccdda8c-2bbf-4621-b81d-1fb67fee46c7/scratchpad
cd "$W" || exit 1
set -o pipefail
{
env PATH=/Users/davidlee/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin:/Users/davidlee/.cargo/bin:/opt/homebrew/bin:/usr/bin:/bin cargo fmt --all -- --check &&
env PATH=/Users/davidlee/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin:/usr/bin:/bin rustfmt --edition 2024 --check vendor/gpui/src/platform/key_text_scope.rs &&
env PATH=/Users/davidlee/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin:/Users/davidlee/.cargo/bin:/opt/homebrew/bin:/usr/bin:/bin cargo clippy --workspace --all-targets --locked --offline -- -D warnings &&
env PATH=/Users/davidlee/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin:/Users/davidlee/.cargo/bin:/opt/homebrew/bin:/usr/bin:/bin cargo test --workspace --locked --offline --no-fail-fast 2>&1 | tee "$S/gate-tests.log" &&
env PATH=/Users/davidlee/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin:/Users/davidlee/.cargo/bin:/opt/homebrew/bin:/usr/bin:/bin cargo build --workspace --locked --offline &&
env PATH=/Users/davidlee/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin:/Users/davidlee/.cargo/bin:/opt/homebrew/bin:/usr/bin:/bin cargo check -p sprite-term --all-targets --target x86_64-unknown-linux-musl --locked --offline &&
git -C "$W" diff --quiet 03b4929 -- Cargo.lock && echo "Cargo.lock unchanged since 03b4929" &&
python3 -m unittest scripts.test_release_automation scripts.test_packaging_automation scripts.test_shell_integration scripts.test_ci_policy scripts.test_findings scripts.test_gpui_patch -v &&
python3 scripts/check_findings.py &&
python3 scripts/check_gpui_patch.py &&
echo "GATE PASSED"
} 2>&1 | tee "$S/gate.out"
grep -E "^test result" "$S/gate-tests.log" | awk '{p+=$4; f+=$6; i+=$8} END {print "passed="p" failed="f" ignored="i}'
```

(The `cd` is the worktree itself, so the shell does not leave it.) Expected: the last line of `$S/gate.out` is `GATE PASSED`; the awk line prints `failed=0`; the Python suites end `OK` (#76 had 49 tests and 4 skipped; this PR adds `test_distribution_pkgbuild_pins_the_release_tag` and `test_every_job_has_a_time_limit`, plus any the quit task added to `scripts.test_gpui_patch`); `check_gpui_patch.py` reports the vendored tree matches the patch. Any red line stops the task: fix it in its own commit and rerun the whole gate.

- [ ] **Step 4: Push and open the PR as a draft**

Use `dmi-superpowers:creating-a-pull-request` with `.github/pull_request_template.md`. Write the body to `$S/pr-body.md` (not in the repo). Its sections, in order:

1. `## Summary` (plain language). **Problem:** after the last round of fixes ten known problems were still open: quitting from the Dock or logging out could skip the cleanup that stops programs left running in panes; an image description with SVG filters could freeze the window for seconds; the Arch package build named a version but built whatever the main branch held; a text selection could drift onto the wrong line in a pane that keeps no history; a program reading Sprite's output very slowly could hold a connection slot indefinitely; image tiles could land a frame behind when a divider moved; images drawn with Kitty's Unicode placeholders lost rows and columns that the format lets a terminal infer; and the Arch CI job failed now and then on timing, and could hang for six hours. **Solution:** each is fixed where it lives, with a test that failed before the fix, or, for two performance ideas the measurements show are not needed, recorded as won't-fix with the reason. The findings ledger now has no open entry.
2. `## TLDR for developers`: one bullet per finding, from the commits on this branch (`git -C "$W" log --oneline 03b4929..HEAD`), each naming the mechanism and the reason it was chosen: GPUI's macOS delegate `applicationShouldTerminate:` → Sprite's existing quit gate (ADR 0032; a busy pane cancels and asks, as Terminal.app does); SVG decode on the background executor, two at a time per cache, with a filter-use cap (ADR 0022 amended); a sentinel `TrackedGridRef` below the anchor detects row rotation; a per-batch drain deadline `WRITE_TIMEOUT + bytes / MIN_DRAIN_RATE` (ADR 0029); placeholder tile geometry at paint time; Kitty placeholder left-neighbour inheritance per Ghostty's `IncompletePlacement`; `#tag=v$pkgver`; the remaining fixed waits become condition waits and every CI job has `timeout-minutes`. Check each bullet against the diff before writing it; if a task's implementation differs from this list, write what the diff shows.
3. `## What changed`: one paragraph naming what the diff cannot show: the GPUI patch is regenerated and `vendor/gpui.provenance.md` records it; ADR 0032 is new; the two wontfix reasons replace an earlier claim that `docs/performance/` held divider-drag numbers.
4. `## Findings`, the template's text, then this table:

   ```markdown
   | ID | Status after this PR | Guarding test or issue |
   |---|---|---|
   | GAP-C01 | resolved | `a_platform_quit_request_is_allowed_only_after_pane_cleanup`, `a_platform_quit_request_with_a_busy_pane_asks_then_the_next_one_quits` |
   | GAP-C03 | resolved | `an_svg_is_decoded_off_the_ui_thread_and_its_view_told_when_it_is_ready`, `a_document_over_the_filter_cap_is_refused_never_rasterised_and_not_tried_again` |
   | GAP-C04 | resolved | `test_distribution_pkgbuild_pins_the_release_tag` |
   | GAP-C06 | resolved | `a_gesture_whose_anchor_was_rotated_out_with_zero_scrollback_selects_nothing`, `a_gesture_whose_anchor_was_rotated_out_on_the_alternate_screen_selects_nothing` |
   | GAP-C07 | resolved | `a_reader_slower_than_the_drain_floor_loses_its_slot` |
   | GAP-C08 | resolved | `placeholder_tiles_land_on_this_frames_cells_after_a_scale_change` |
   | GAP-C10 | resolved | `a_running_program_is_reported_by_name`, `natural_exit_deadline_is_not_renewed_by_continuous_output` |
   | GAP-C14 | wontfix | an unchanged pane prepares in about 0.3 ms with no shaping, guarded by `shaping_happens_only_for_cells_whose_drawn_text_changed`; cached views would miss resizes |
   | GAP-C15 | wontfix | per-move work is linear and test-bounded (`placement_matching_scales_linearly`, `title_membership_reconciliation_scales_linearly`); PTY resizes only on a cell boundary |
   | GAP-C16 | resolved | `a_placeholder_without_diacritics_continues_its_left_neighbour` |

   Closes #57, #61, #63, #64, #66, #67, #68, #73, #74, #75
   ```

5. `### #57: done when` (R-W4), verbatim:

   ```markdown
   ### #57: done when

   This PR replaces #57's "done when" (100 looped runs per test, and 10
   consecutive master runs) with:

   - each listed test passes 100 looped runs under CPU load (one `yes` per
     core) on Linux, with `a_refused_reload_is_kept_whole_and_reverted_whole`
     standing in for the deleted `accepted_colors_revert_after_the_other_reload_groups_refuse`;
   - this PR's Arch job passes;
   - five `workflow_dispatch` Arch runs on this branch pass.

   The ten-master-runs condition is dropped: it measures merges after this
   one, which this PR cannot show. The jobs now carry `timeout-minutes`, so a
   hang after merge fails in minutes and is visible.

   ### #61 and #68: done when

   #61 asked for a test that crosses the real quit hook. The GPUI tests cross
   the same callback the Cocoa delegate calls, and the manual Dock check below
   runs the delegate itself.

   #68 asked for a test that drags a divider. A divider drag cannot misalign
   tiles: layout rounds every position to whole device pixels, and snapping
   does not change under a whole-pixel shift. The real trigger is a
   display-scale change, so the regression test changes the window's scale
   from 2.0 to 1.0 instead.
   ```

6. `## Evidence`:
   - The gate (Step 3): each command and its result; the `passed= failed= ignored=` line; the Python suite count; the musl check; `Cargo.lock unchanged since 03b4929`.
   - Failing first: a table with one row per regression test in the Findings table, giving the failure message that test produced at `03b4929` as each task recorded it (GAP-C04's is Task 1 Step 2's `Lists differ` line). GAP-C10's two tests are the exception: they change how a test waits, and their evidence is the loop below.
   - The Linux loops: the seven lines from `tail -n 9 "$S/linux-loop.out"`, with the image (`archlinux@sha256:714acd1e…`, `linux/amd64` under emulation), the load (`nproc` × `yes`) and `SPRITE_TEST_WATCHDOG_SECS=20`. If Task 2 fell back to bookworm or looped only sprite-term, say exactly that.
   - The CI time limits: the measured table from Task 2 Step 6.
   - The manual Dock-quit check (R-Q5) as the quit task recorded it: `sleep 1000` in a pane, Dock → Quit twice, the `ps` output.
   - Hosted runs: filled in Step 7.
   - Unexecuted: native desktop behaviour CI cannot run (window activation, real keyboard focus, a physical display); Linux session logout (out of scope, PRD); the Linux-only sprite-app paths beyond what the hosted Arch job compiles.
7. End with `🤖 Generated with [Claude Code](https://claude.com/claude-code)`.

Title (the outcome, not the mechanism): `Quitting from the Dock cleans up, SVG filters no longer freeze the window, and every open finding is closed`.

```bash
W=/Users/davidlee/Projects/Sprite/.worktrees/close-open-findings
S=/private/tmp/claude-502/-Users-davidlee/cccdda8c-2bbf-4621-b81d-1fb67fee46c7/scratchpad
git -C "$W" push -u origin fix/close-open-findings
gh pr create --repo HundredBillion/Sprite --base master --head fix/close-open-findings --draft \
  --title "Quitting from the Dock cleans up, SVG filters no longer freeze the window, and every open finding is closed" \
  --body-file "$S/pr-body.md"
```

Expected: `gh pr create` prints `https://github.com/HundredBillion/Sprite/pull/N`. Call that number `N` below.

- [ ] **Step 5: Record the PR number in the ledger**

In `docs/reviews/findings.toml` replace every `fixed_by = "fix/close-open-findings"` (eight lines) with `fixed_by = "#N"` using the real number, matching `fixed_by = "#76"` above them.

```bash
W=/Users/davidlee/Projects/Sprite/.worktrees/close-open-findings
cd "$W" || exit 1
grep -c 'fixed_by = "fix/close-open-findings"' docs/reviews/findings.toml
python3 scripts/check_findings.py
git -C "$W" add docs/reviews/findings.toml
git -C "$W" commit -m "$(cat <<'EOF'
Record PR #N in the ledger

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
EOF
)"
git -C "$W" push
```

(Write the real number in the commit subject.) Expected: the `grep -c` prints `0`; the checker passes; the push updates the PR, which starts its `pull_request` run.

- [ ] **Step 6: Five dispatched Arch runs on the final head**

`ci.yml`'s `workflow_dispatch` has no inputs and no job conditions, so each dispatch runs all three jobs (release automation, arch, macOS), not Arch alone; only the `arch linux` job's result is the R-W4 evidence. Five dispatches also cost five macOS runs.

```bash
W=/Users/davidlee/Projects/Sprite/.worktrees/close-open-findings
HEAD_SHA=$(git -C "$W" rev-parse HEAD)
echo "$HEAD_SHA"
gh workflow run ci.yml -R HundredBillion/Sprite --ref fix/close-open-findings
gh workflow run ci.yml -R HundredBillion/Sprite --ref fix/close-open-findings
gh workflow run ci.yml -R HundredBillion/Sprite --ref fix/close-open-findings
gh workflow run ci.yml -R HundredBillion/Sprite --ref fix/close-open-findings
gh workflow run ci.yml -R HundredBillion/Sprite --ref fix/close-open-findings
```

`ci.yml` has no `concurrency` group, so the five run side by side and none cancels another. After about a minute, list them:

```bash
gh run list -R HundredBillion/Sprite --workflow ci.yml --branch fix/close-open-findings --event workflow_dispatch --limit 5 --json databaseId,headSha,status,conclusion
```

Expected: five entries, each `headSha` equal to `$HEAD_SHA`. Wait for them (one per run ID, in the background or with Monitor, since each blocks until its run ends; a run is bounded by the new limits at about 40 min):

```bash
gh run watch <run id> -R HundredBillion/Sprite --exit-status
```

Then, for each run ID:

```bash
gh run view <run id> -R HundredBillion/Sprite --json jobs --jq '.jobs[] | select(.name == "arch linux") | [.conclusion, .startedAt, .completedAt] | @tsv'
```

Expected: `success` for all five. Also check the PR's own run:

```bash
gh pr checks N -R HundredBillion/Sprite
```

Expected: `arch linux`, `macos` and `release version automation` all `pass`.

If any Arch job fails: do not rerun it to get green. Read its log (`gh run view <run id> -R HundredBillion/Sprite --log-failed`), fix the cause in a commit, push, and start Step 6 again from the new head: R-W4 counts five passing runs on one head, not five passes out of more attempts.

- [ ] **Step 7: Finish the PR and #57**

Add to the body's Evidence a `Hosted runs` list: the PR run's ID and each job's result, and the five dispatched run IDs with their `arch linux` conclusion and duration (from Step 6). Then:

```bash
S=/private/tmp/claude-502/-Users-davidlee/cccdda8c-2bbf-4621-b81d-1fb67fee46c7/scratchpad
gh pr edit N -R HundredBillion/Sprite --body-file "$S/pr-body.md"
gh pr ready N -R HundredBillion/Sprite
```

Do not comment on #57 or any other issue: the PR body records the replaced "done when", and the merge closes the issues.

(Write the real PR number for `N`.)

- [ ] **Step 8: Report**

Tell the user: the PR URL; the gate numbers; the seven loop lines; the PR run and the five dispatched Arch results with their run IDs; the manual Dock-quit result; and everything listed as unexecuted. Issues #57, #61, #63, #64, #66, #67, #68, #73, #74 and #75 close when the PR merges.

---

