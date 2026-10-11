# Prompt startup and throughput Technical Spec

> **For agentic workers:** REQUIRED SUB-SKILL: Use dmi-superpowers:subagent-driven-development or dmi-superpowers:executing-plans to implement this TSP task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Reduce first-prompt overhead and redundant work while preserving terminal behavior, backed by observed failing regressions and measured performance.

**Architecture:** Improve existing owners: app-scoped font discovery, coherent image capture, bounded socket housekeeping, cancellable observation requests, prospective pane geometry, and consumer-driven snapshot delivery. Preserve the independent lossless event stream and per-pane worker. Existing review recommendations were approved by the user's request to implement all improvements; no separate PRD was supplied.

**Tech Stack:** Rust/Cargo 1.97.1, vendored GPUI, pinned libghostty-vt 0.2.2, macOS and Linux, standard library and existing dependencies only.

## Global constraints

- Base `afece5293271b410d83c63e236938ff2a7e0a250`; branch `perf/prompt-throughput` in the existing isolated worktree.
- Read `CONTEXT.md`, `crates/CONTEXT.md`, applicable ADRs, and `docs/performance/prompt-startup-review.md` before editing.
- For each behavior: write regression first, run it and save the expected failing output, make the smallest repair, then run covering tests. Existing known-bad fixtures need a recorded failure before their change. Count-based tests protect algorithmic work; opt-in release measurements protect latency without flaky CI wall-clock assertions.
- Run all Rust commands with `PATH="$HOME/.cargo/bin:$PATH" cargo +1.97.1`; selecting Cargo alone is insufficient on this host.
- Preserve wire protocol v1, ticket correlation, lossless lifecycle/events, bounded memory, snapshot coherence, explicit command refusal, focus/clipboard/paste rules, and existing cleanup budgets.
- Preserve configured shell arguments/login semantics, cwd, environment, HiDPI metrics, native text behavior, and live reload. No personal shell file changes or warm spare shell pool.
- No arbitrary recurring per-pane timer or animation loop. Ground frame callbacks in vendored GPUI source and a real test. A blank generation-zero snapshot is not a prompt.
- Commands, first prompt, quiet output, final snapshots, hidden tabs, paused/closed consumers, and minimized windows are required cadence cases. Never make lifecycle completion depend on a paint callback.
- Main-worktree review files remain preserved. Parent owns commits, integration, whole-workspace gates and final review. Implementers edit only assigned files; do not push, merge, or modify global environment.
- Evidence directory: `/Users/davidlee/pr-reviews/sprite-prompt-improvements/`; task reports include exact commands, red/green results, changed files, limitations.

## Task 1: Reuse font discovery with explicit invalidation

**Blocked by:** None

**Files:** `crates/sprite-app/src/terminal_view/theme.rs`, font selection call sites in `terminal_view.rs`, `workspace/reload.rs`, focused tests.

**Interfaces:** Keep font/metric semantics; introduce one app-scoped catalog/resolver owned by font selection. The resolver takes a configured family, obtains one list lazily, and selects fallback from that same list. Explicit settings reload invalidates the catalog so newly installed fonts can be discovered. Runtime reference-font registration currently precedes first view; confirm every registration call before implementation.

- [ ] Add a counter at the actual enumeration seam; construct default, installed, missing-family, and second-pane cases. Assert one enumeration for a catalog epoch, repeat fallback diagnostics, and unchanged selection/metrics.
- [ ] Run `cargo test -p sprite-app font -- --nocapture`; save observed duplicate/repeated enumeration failure before repairing.
- [ ] Implement lazy catalog reuse and invalidate once before publishing reloaded ActiveSettings. Preserve default/missing-family warnings and re-resolve when the configured family changes. Avoid a global mutex/OS call on the hot path.
- [ ] Test reload discovers a changed catalog and successful/error reload semantics are unchanged. Run theme/settings/terminal view covering tests and record results.

## Task 2: Bound image conversion and make capture membership scalable

**Blocked by:** None

**Files:** `crates/sprite-app/src/graphics_cache.rs`, `crates/sprite-term/src/graphics.rs`, their tests/graphics fixtures.

**Interfaces:** `GraphicsCache::texture` and `capture_frame` retain existing signatures, ordering and pixel Arc identity.

- [ ] Add a conversion/allocation-count test calling `texture` twice with output size above budget; zero conversion is required. Include overflow/length mismatch, exact-fit, zero budget, changed generation, and collective cache pressure.
- [ ] Run the focused test and retain failing evidence before reordering work.
- [ ] Validate checked dimensions/source length and computed BGRA byte size before allocation/conversion. Preserve malformed-image refusal and LRU budget correctness.
- [ ] Add a count/scaling regression exercising production placement deduplication and retention with many distinct IDs and repeated placements. Observe excessive work before replacing linear membership scans with a set.
- [ ] Preserve sorted snapshots, deletion eviction and unchanged pixel sharing. Run app graphics-cache and terminal graphics suites plus the focused release scaling probe.

## Task 3: Remove unbounded socket sweep work from opening a pane

**Blocked by:** None

**Files:** `crates/sprite-app/src/local_socket.rs` and its tests; endpoint consumers only if required.

**Interfaces:** Endpoint credentials/binding still exist before children launch. Stale-file cleanup remains best-effort and must not unlink live listeners because their backlog is full.

- [ ] In a private temporary directory, create a listener with a saturated backlog. Exercise actual sweep/open in an isolated test process with an independent watchdog; record platform-specific refusal/blocking behavior. Also cover normal live and stale sockets.
- [ ] Make each connect probe nonblocking/bounded; treat in-progress/full backlog as potentially live. Move directory housekeeping off the UI creation path if traversal is otherwise unbounded. Do not create an unbounded number of housekeeping threads; reuse the listener's owner loop when possible.
- [ ] Assert the endpoint is usable promptly while a probe cannot complete, cleanup eventually removes genuinely stale files, and live paths survive. Run socket/observation/Surface endpoint suites. Record red-green and Linux-specific evidence honestly.

## Task 4: Cancel abandoned history work before traversal

**Blocked by:** None

**Files:** `crates/sprite-term/src/session.rs`, `worker/mod.rs`, a small history request module if needed, exports in `lib.rs`, `crates/sprite-app/src/observation/panes.rs`, related tests.

**Interfaces:** Preserve existing `TerminalCommand::CaptureHistory { ticket, lines }` for noncancellable callers. Add a bounded `CommandSender` request entry returning an owned cancellation handle; use a private worker message carrying ticket, lines, and shared cancellation state. Dropping/withdrawing the handle cancels queued work without needing queue admission. Successful events keep their ticket. No new wire fields.

- [ ] At the real worker handoff, hold progress, enqueue a request, withdraw it, then release progress. Assert no traversal/publication for the abandoned request and a following live request receives its own coherent answer. Preserve existing out-of-order/timed-out ticket tests.
- [ ] Observe the current unnecessary traversal/publication before repair. Introduce only the cancellation seam needed to exercise the production path.
- [ ] Skip cancelled work before foreground lookup/history traversal; ensure admission errors do not leak registry state. Keep successful accepted responses coherent and lifecycle delivery independent of render consumption.
- [ ] Run terminal history/event-pressure and app broker/observation tests; test receiver drop, window unregister, full queue, and successful completion races.

## Task 5: Start panes with their prospective geometry; reduce cosmetic workspace work

**Blocked by:** Task 1

**Files:** `workspace/pane_factory.rs`, `workspace/mod.rs`, `workspace/tab_strip.rs`, `tabs.rs`/`pane_tree.rs` only to reuse layout arithmetic, `terminal_view.rs`, `terminal_view/geometry.rs`, `terminal_view/render.rs`, GPUI tests.

**Interfaces:** Keep `TerminalView::new` compatibility for existing direct tests/callers where feasible; add a constructor/helper accepting known initial allocation. Workspace derives the prospective new-tab/split rectangle using the same layout arithmetic as final placement. Defaults remain a fallback when bounds are unavailable.

- [ ] Use a real child whose first command reports `stty size`; verify startup, a new tab (including first tab-strip appearance), horizontal/vertical nested splits, nondefault font/padding and scale. It must see the final initial grid before any later resize.
- [ ] Observe default-24×80 failures, then pass correct initial allocation/physical cell metrics before spawn. Preserve subsequent resize, docks, divider math, and sibling resizing.
- [ ] Add counter tests showing content-only snapshots do not repeatedly query the OS for an unchanged cosmetic title, while clock-driven titles and synchronous close warnings remain correct. Move repeated cosmetic probes to the existing window clock; explicit OSC title events remain immediate.
- [ ] Add work-count tests for workspace membership/placement reconciliation with inactive tabs, 4/8/16 panes, retained title subscriptions and changed/unchanged allocations. Replace nested membership scans with sets/keyed lookups, maintaining stable ordering and avoiding redundant callbacks.
- [ ] Run workspace/layout/title/foreground/terminal-view suites and the real startup-size regressions. Record red-green for each behavior slice.

## Task 6: Pace continuous snapshot consumption without delaying startup or completion

**Blocked by:** Tasks 4, 5

**Files:** `terminal_view.rs` snapshot consumer, render/measurement helper as necessary, worker/session files only if proven necessary, coalescing and GPUI regressions, focused performance fixture.

**Interfaces:** Independent lossless event delivery stays live. Snapshot transport stays bounded/coherent. Consumer demand may be tied to completed frames, but initial/quiet output must not wait on a recurring timer, and invisible/closed panes must not strand final lifecycle work.

- [ ] Build a release probe with a live child after a bounded output burst: immediate, frame-paced and paused consumers at 80×24 and 100×100. Count parsed bytes/captures and record input-to-snapshot latency. Include a final output marker; stream closure alone is not success.
- [ ] Reproduce excessive captures with a fast consumer and add a deterministic bounded-work regression plus first-content, quiet update, final dirty snapshot, hidden/minimized/closed consumer tests.
- [ ] Validate vendored `Window::on_next_frame` (after-current-render callback, not self-scheduling timer) and scheduling consumers before choosing the smallest pacing mechanism. Do not replace a measured throughput cost with a fixed first-prompt wait.
- [ ] Implement the validated demand policy. Re-run latency/throughput probe and regression suite; reject a design that reduces captures by suppressing output or stalls lifecycle cleanup.

## Task 7: Measure prompt delivery and first paint; integrated verification

**Blocked by:** Tasks 1–6

**Files:** existing `tests/prompt_latency.rs`, focused app benchmark/GPUI test helper, `docs/performance/prompt-startup-review.md`, new performance evidence report, ADR/context entries only for actual contract decisions.

**Interfaces:** Distinguish spawn-to-Ready, prompt-bearing snapshot, GPUI first paint, and native presentation. The existing opt-in prompt test is retained. Any diagnostic instrumentation must be opt-in, bounded, and free of personal shell text or global state mutation.

- [ ] Keep the delayed-child control and Ready-only mutation proof. Add first-prompt delivery coverage for startup/tab/split at the actual view/paint seam and busy existing panes. Observe any identified regression before a fix.
- [ ] Run controlled release before/after measurements of actual font selection/pane creation, prompt latency, captures and graphics scaling. Label native-paint proxies honestly; do not claim compositor or personal-shell timings without measuring them.
- [ ] Run pinned workspace tests, all-target Clippy -D warnings, fmt, build, Python policy/GPUI/release checks, and relevant emulated Linux tests. Existing ignored timing probes remain opt-in; correctness regressions run in CI.
- [ ] Request independent whole-change review covering every committed/uncommitted change and original findings. Fix substantive findings with covering tests before completion.
- [ ] Update the review disposition with evidence for each requested improvement, actual measured gains, limits, and commands. Parent commits the verified changes; no merge is authorized.

## Design stress-check decisions

- Font installation discovery is refreshed explicitly on configuration reload; changing font preference must never reuse a previous resolved family incorrectly.
- First paint is not compositor presentation. Reports use the narrower measured endpoint.
- Cosmetic titles can lag until the existing 530 ms clock; close/ownership safety probes remain fresh.
- Cancellable history is additive to the engine interface, preserving command Eq/Clone and existing callers. Cancel-before-start skips work; in-progress successful coherent capture may complete and be discarded.
- Native socket behavior differs; a refused backlog must not be mistaken for proof that a live path is stale. The test controls only its private directory.
- Pane geometry should reuse current tree split/layout rules; no parallel copy of divider arithmetic or deferred-shell timer.
- Snapshot cadence is the only implementation policy intentionally gated on new measurement. The required behavior is fixed; a source-grounded probe chooses a reversible implementation.
