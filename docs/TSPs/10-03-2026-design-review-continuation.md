# Complete Design Review Continuation Technical Spec

> **For agentic workers:** REQUIRED SUB-SKILL: Use dmi-superpowers:subagent-driven-development or dmi-superpowers:executing-plans to implement this TSP task-by-task. Steps use checkbox syntax for tracking.

**Goal:** Complete Waves 2 and 3 and remaining type-level moves of issue #48 in the
same PR as Wave 1.

**Architecture:** Shared ownership replaces paired mutable state; transport has
two protocol adapters; pane routing uses the existing trait seam; row-level
identity allows capture and paint work to be reused. Measure before optimising.

**Tech Stack:** Rust 1.97.1, GPUI 0.2.2, libghostty-vt 0.2.1, nix 0.28.0.

## Global Constraints

- Prerequisite: all six tasks in 10-03-2026-design-review-wave-1.md pass review.
- Source requirements: docs/PRDs/10-03-2026-design-review-complete.md and issue #48
  (full JSON at /tmp/sprite-issue-48.json). User explicitly selected all three waves.
- Use TERM=xterm-ghostty for terminal integration tests. Preserve protocol grammars,
  exact authentication, bounded resources, latest-only snapshots and ordered events.
- No editor dependency, no unsafe Send/Sync. Existing forbidden-state CI stays.
- serde is already locked at 1.0.229; make it direct when needed. proptest is a new
  test-only dependency explicitly needed for the requested properties; pin/fetch
  before running offline checks. No other new dependency planned.
- Keep public term re-exports; migrate all linked Rust consumers atomically.
  No persisted-data backfill or mixed-version process rollout applies here.
- Run meaningful interface tests, red→green for confirmed bugs, independent review
  per task, and record measurements instead of inferred allocation counts.
- Every compile-fail doctest needs a compiling positive control with the same
  imports; missing/private imports are not evidence of the intended invariant.
- Update docs/performance and relevant ADRs when contract changes need explanation.
- Do not open a partial PR. Final task opens the one complete PR requested.

### Task 1: Measure actual paint preparation and decision allocations

**Blocked by:** None (global Wave 1 prerequisite applies)

**Files:** create crates/sprite-app/src/bin/sprite-paint-bench.rs and a narrow
benchmark entry module; modify crates/sprite-app/{Cargo.toml,src/lib.rs},
src/{grid.rs,grid_paint.rs}; create docs/performance/design-review-paint-baseline.json
and docs/performance/design-review.md; add benchmark report test.

**Interfaces:** Expose a small benchmark driver from sprite_app; keep grid internals
private. The driver calls actual lay_out_row and GridPaint::draw, not copied logic.
Binary accepts `--samples N --output PATH` and `--check-budgets PATH`.

- [x] Construct a synthetic 200x60 RenderSnapshot with ASCII, blanks, Unicode,
  wide cells, selection, palette colours and a blinking cursor. Add first-frame,
  same-generation blink, hover, and one-row-change scenarios. Blink toggles phase
  after warmup; hover transitions on each sample; one-row-change presents a fresh
  row/generation per sample with fixture mutation outside measurement; first-frame
  resets preparation state. These semantics must remain valid after caching.
- [x] Use a counting allocator in the standalone bench process. Disable counting
  for fixture/report setup and warmup; scope actual measured work with a guard.

```rust
struct AllocationSample { allocations: u64, bytes: u64 }
// Count alloc/realloc calls while the measurement guard is alive; deallocation
// does not increment allocations. Use std::alloc::System behind GlobalAlloc.
```

- [x] Make the measured draw decision path shared with production. Consume results
  with black_box. Record timing median/p95 plus allocations/bytes in stable JSON.
  Clearly exclude GPUI Window/glyph submission from this headless seam.
- [x] Run a release benchmark with >=20 samples and commit the baseline BEFORE
  optimising. Add an executable budget/report-schema check that fails when a
  supplied allocation budget is deliberately too small, then passes with baseline.
- [x] Run sprite-app tests/fmt/clippy for changed targets; commit benchmark/evidence.

### Task 2: Return PTY permits and buffers through ownership

**Blocked by:** Task 1

**Files:** crates/sprite-term/src/{pty_unix.rs,worker.rs}; pump tests;
crates/sprite-term/src/bin/sprite-term-bench.rs if measurement support is needed.

**Interfaces:** `Message::PtyOutput(OutputChunk)` carries bytes and a Permit whose
Drop returns its buffer/token and wakes the pump. OutputChunk exposes bytes by
slice only. No manual return_permit call remains outside Drop.

- [x] Write a pump test consuming/dropping >=40 chunks without calling a return
  method; verify all bytes arrive and cancellation joins. Cover inbox rejection
  and unread queued messages dropped during shutdown. Observe old behavior fail.
- [x] Enforce the Unix pump safety boundary with `#![deny(unsafe_code)]` and
  narrowly scoped allowances for the descriptor-duplication and macOS process-name
  FFI boundaries; the pump otherwise uses owned safe descriptor APIs.
- [x] Pool the existing fixed number of 16 KiB buffers at startup. Read directly
  into a checked-out buffer; send ownership rather than to_vec().

```rust
struct OutputChunk { buffer: Vec<u8>, len: usize, permit: Permit }
// Design Drop so the same allocation returns to the pool before its wake;
// an Option-owned buffer or Permit-owned buffer avoids taking through a borrow.
```

- [x] Preserve available-permit backpressure and the reserved command queue slot.
  A lost consumer/cancelled pump must not block in Drop; use bounded nonblocking
  return with closed-channel cleanup. No cycles retain the pump thread.
- [x] Count steady-state buffer allocations across >40 deliveries; assert zero.
  Run pump, lifecycle, session_output, input_backpressure and graphics-transfer
  tests, plus term all-target clippy. Commit evidence and implementation.

### Task 3: Share local socket transport and extract Surface wire grammar

**Blocked by:** Task 2

**Files:** create crates/sprite-app/src/local_socket.rs and surface/wire.rs;
modify observation/{endpoint,client}.rs, surface/{channel,client}.rs, lib.rs and
workspace reply relay helpers; update ADR 0018.

**Interfaces:** A private LocalSocket listener owns path/key/listener thread and
close/unlink. Protocol adapters receive authenticated connections. Explicit
TransportPolicy supplies cap, handshake timeout, line bound and filename suffix.
ObservationKey re-export remains source compatible.

- [ ] Inventory current authentication paths and socket lifetimes. Preserve and
  test wrong/missing/oversized key lines, slow clients, cap exhaustion, socket
  0600 mode, private directory, cancellation, stale cleanup and path-length limits.
- [ ] Move runtime directory, random name/key generation, bind, accept accounting,
  capped first-line authentication and teardown into local_socket. Bind and
  key.matches should each have one production call site. Route socket test
  fixtures through the shared bind helper too, so the literal source census has
  one `UnixListener::bind` site without hiding calls behind aliases. Preserve justified
  observation/stream-specific policy differences and record them in ADR 0018.
- [ ] Move pure Surface request parsing and event serialisation to wire.rs;
  unify one-shot relay plumbing and version handling behind one Envelope parse.
  Existing version-less focus/token clients remain accepted through an explicit
  legacy default; emitted clients use the versioned envelope consistently.
- [ ] Keep read-only observation verbs structurally separate; never accept Surface
  verbs through the observation adapter. Run both endpoint/client/wire test suites
  and the full sprite-app suite. Check listener/key site census and commit.

### Task 4: Route Surface actions through the pane interface

**Blocked by:** Task 3

**Files:** crates/sprite-pane/src/lib.rs; app workspace Surface routing,
terminal_view/{mod or root,surfaces,placeholder}.rs; surface request enum.

**Interfaces:** Associate the application request type with Pane and PaneHandle
without importing sprite-app into sprite-pane. A request refusal contract lets a
placeholder answer unsupported requests, including Open, rather than drop replies.

```rust
pub trait PaneRequest { fn refuse(self); }
// Pane: type Request: PaneRequest; default surface_request refuses.
// PaneHandle<Request = ...> dispatches through Entity::update_in with Window.
```

- [ ] Test a placeholder pane's Open returns NotATerminal through the trait, and
  terminal pane open/update/focus/close/grid/list/capabilities retain behavior.
- [ ] Add surface_request at the existing handle seam. Move pane-specific matching
  into TerminalView; keep window token registration in the workspace. Route focus
  cycling through the same seam or an explicit default trait operation.
- [ ] Delete both workspace downcasts. Preserve failed/ended terminal refusals,
  owner checks, reply completion, close-on-disconnect and focus notifications.
- [ ] Run app Surface and GPUI tests, pane crate tests and all-target clippy.
  Confirm no `downcast` remains in workspace sources. Commit interface + adapters.

### Task 5: Encode parsed Surface variants and geometry invariants

**Blocked by:** Task 4

**Files:** surface/{description,style,render,host,channel,wire}.rs; terminal_view
{theme,render,surfaces,geometry}.rs; grid.rs; grid_paint.rs; box_drawing.rs;
sprite-term size/config/command definitions and consumers.

**Interfaces:** Element variants carry only valid payloads; shared style lives
once. Ownership distinguishes unowned from owned dock with return destination.
CellMetrics has one measuring constructor. ValidTerminalSize has one validating
constructor and getters; SessionConfig and Resize carry it. Snapped/Col/Row types
are used at the geometry edge.

- [ ] Add compile-fail docs and parser tests for contradictory Element/Ownership
  shapes (including Fill + return_target), preserving existing wire error text
  where clients rely on it. Match exhaustively in rendering and host management.
- [ ] Replace the option bag with variants for stack/text/image/grid/list and their
  valid attributes. Parse wire data once into valid values. Route all consumers
  through variants; do not recreate independent kind/payload pairs. Parse utility
  styles into valid values once, replacing validated strings that are parsed again
  during rendering; preserve the existing utility vocabulary and error messages.
- [ ] Replace independent font/cell fields with measured CellMetrics. Startup and
  reload assign one measured value; tests change font family/size/line height and
  verify terminal and Surface use matching metrics.
- [ ] Introduce ValidTerminalSize, moving size validation to its constructor.
  Migrate resize/session call sites and tests without clamping invalid sizes into
  acceptance. Expose unvalidated dimensions only as input/DTO where required.
- [ ] Use Snapped(Pixels), Col and Row in grid/box geometry so unsnapped box edges
  cannot compile. Retain tiling properties across fractional scale and wide cells.
- [ ] Run relevant parser/geometry/terminal lifecycle tests, compile-fail docs,
  workspace tests/clippy. Commit focused invariants together with their evidence.

### Task 6: Share terminal text/rows and cache paint preparation

**Blocked by:** Task 5

**Files:** sprite-term render DTOs and snapshot.rs; sprite-app grid.rs,
grid_paint.rs, terminal_view/{root,render}.rs; graphics placeholder consumers;
paint and terminal benchmark drivers and budgets.

**Interfaces:** Render rows and positioned rows have Arc identity. CellText is
blank/inline char/shared grapheme or a row text slice, selected by baseline data.
Palette is Arc<[Rgb;256]>. A private layout cache accepts generation, row identities
and hovered span and returns shared immutable positioned rows.

- [ ] Extend benchmark/checks for non-ASCII graphemes, wide spacers, blank fills,
  selection-only changes, hover transitions, palette/default reload and images.
  Add allocations_per_capture to the term benchmark with a bounded isolated
  measuring run (exclude unrelated worker setup and test allocation).
- [ ] Replace per-cell String allocations with shared/inline text across both
  snapshots and positioned cells. Keep PaneSnapshot text-only and independent.
- [ ] Retain previous Arc render rows in Projector and reuse genuinely clean rows.
  Read libghostty row dirty state before clearing it; invalidate on dimensions,
  viewport/alternate-screen changes, selection changes and relevant styles.
  Compare content as a correctness fallback where upstream dirty signals are
  insufficient; don't trust dirty flags without tests for each mutation class.
- [ ] Cache layout by row identity and hovered span; unchanged blink reuses the
  existing shared rows. Hover rebuilds only affected rows. Pass snapshot palette
  Arc directly; split background/text passes share row storage.
- [ ] Run original generation/input/selection/history/graphics tests and paint
  decision tests. Prove unchanged-generation blink preparation/draw decisions have
  zero allocations in every measured sample after warmup (a p95 of zero alone
  is insufficient), one-row change rebuilds one row, capture budgets
  improve, and old benchmarks pass. Commit measured before/after budgets and code.

### Task 7: Push pane title changes and publish layout only on mutation

**Blocked by:** Task 6

**Files:** workspace root/reload/tab strip; terminal_view root; terminal_events.rs;
observation/panes.rs; sprite-pane interface if title change events need it.

**Interfaces:** Pane title changes notify the workspace; observation layout has a
single publication owner triggered by tree/size/tab mutations. Render consumes
cached titles/layout and never re-polls foreground or publishes identical layout.

- [ ] Instrument test counters for set_layout and title/foreground queries.
  Assert idle render produces no layout publication or title String allocation.
- [ ] Push existing Effect::Title updates through pane notifications. Preserve
  foreground fallback for shells without OSC title; trigger updates on actual
  terminal foreground/activity changes, not a new idle polling timer.
- [ ] Publish layout from one mutation-driven path, including resize, split,
  divider drag, close, tab switch and observation re-enable. Cache render layout
  without forgetting pane allocated sizes or observation ordering.
- [ ] Test all mutation classes and title fallback; assert publication counts
  match actual changed layouts. Run observation and workspace/GPUI tests; commit.

### Task 8: Reduce Surface grid/list work and batch outgoing events

**Blocked by:** Task 7

**Files:** surface/{grid,list,render,channel or wire}.rs;
terminal_view/{surfaces,list_view}.rs; Surface benchmark/test fixtures.

**Interfaces:** Surface grid Cell uses <=8 bytes with interned/row-shared text and
highlight identity. Grid rendering exposes shared rows invalidated by operations
or theme changes. ListConfig/visible row strings are shared; list op index built
once. SurfaceConnection supports a single-buffer batch write.

- [ ] Record current size_of<Cell>, grid/list allocations and outgoing write count.
  Add regression checks for scroll/copy/clear/resize, graphemes and highlight/theme
  updates, large-list stable IDs and view virtualization.
- [ ] Compact grid cells, share immutable render rows, track dirty rows, and reuse
  unchanged rows without cloning the entire grid. Preserve operation ordering and
  bounds/refusal behavior. Reclaim overwritten interned text so repeated distinct
  writes cannot grow storage without bound; test this against the live grid size.
- [ ] Arc ListConfig and shared row text; construct id_index once per operation.
  On a 100k-row list, instrument truncate_line to prove <=visible+16 calls/frame.
- [ ] Batch one wheel gesture's events into a reusable buffer with one write;
  preserve JSON line boundaries and ordering. Dedupe resize with a tuple, not a
  newly serialized string. Test writing to a closed/slow peer and concurrent sends.
- [ ] Run Surface/list/grid/wire/GPUI suites and benchmark checks; prove the compact
  cell size, bounded list work and single write. Commit code + measured evidence.

### Task 9: Make the pane tree own its payloads

**Blocked by:** Task 8

**Files:** pane_tree.rs; remove pane_registry.rs; tabs.rs; workspace root;
terminal_view observation registration/cleanup; app lib.rs and tests.

**Interfaces:** `PaneTree<T>` has `Node::Leaf(PaneId, T)` and existing geometric
operations extended to return payload references. Layout cannot name an absent
payload. Closing/moving a leaf transfers ownership exactly once.

- [ ] Move existing PaneRegistry drop-spy tests unchanged in meaning to generic
  tree tests. Add layout().len()==len(), split/close/resize sequence properties,
  and exactly-once payload shutdown/drop assertions.
- [ ] Fold registry storage into leaves, migrate tabs/workspace access, remove
  defensive filter_map and parallel contents.clear/contains_key sync logic.
  Keep observation WindowPanes only for its authorization/command responsibilities;
  ensure closing a payload unregisters it rather than retaining a ghost command.
- [ ] Preserve stable Divider identity and geometry ordering. Run pane/tree/tab,
  observation, shutdown, and workspace tests; no orphan IDs/payloads may remain.
- [ ] Commit generic tree and remove the obsolete registry module.

### Task 10: Parse raw config into validated settings and typed differences

**Blocked by:** Task 9

**Files:** config.rs or config/{mod,raw,metrics}.rs; workspace reload/font actions;
terminal_view settings consumers; Cargo manifests/lock; DEPENDENCIES.md.

**Interfaces:** serde RawConfig represents the file; validated Settings holds
bounded finite metrics and canonical palette entries. Settings::diff produces a
typed change set consumed by reload and documented live/restart behavior.

- [ ] Capture existing config parse/to_toml behavior and error-recovery fixtures.
  Add proptest (test-only, pinned) and direct serde 1.0.229; fetch once, then offline.
- [ ] Deserialize raw fields with serde while preserving per-field diagnostics and
  fallback for malformed sections, unknown keys and values. Convert to validated
  newtypes through one constructor per invariant. No NaN/zero/negative metrics can
  enter drawable settings through CLI/font actions or tests.
- [ ] Canonicalize palette indices in the type; remove duplicate default values
  from parser/serializer/app paths. Preserve explicit ordering in serialized files.
- [ ] Replace classify with Settings::diff and use its typed live/restart effects
  during reload. One source defines both behavior and explanatory diagnostics.
- [ ] Add generated `parse(to_toml(settings)) == settings` and drawable-metrics
  properties with boundary/nonfinite cases and shrinking. Test malformed-input
  compatibility and round-trip all settings sections/highlights/shell preferences.
- [ ] Run config/property/reload tests and full locked offline suite/clippy; update
  dependency ledger and configuration docs. Commit parser + caller migration.

### Task 11: Split remaining large files by responsibility and complete PR

**Blocked by:** Task 10

**Files:** sprite-term src/lib.rs -> session.rs,command.rs,event.rs,config.rs,
render.rs,pane.rs; worker.rs -> worker/{mod,start,closing}.rs, input/{keys,mouse,paste}.rs,
hyperlink.rs; app workspace.rs -> workspace/{mod,keymap,divider,close_gate,tab_strip,
reload,pane_factory,surface_routing}.rs. Wire split is already Task 3.

**Interfaces:** Public re-exports stay stable. Worker Session::handle(Message)
returns Flow; event emission has one fallible helper. Pure close-gate and divider
logic remains Window-free. Avoid pass-through wrappers around moved code.

- [ ] Inventory remaining production/test responsibilities and move each with its
  tests. Share hyperlink scheme allow-list with its actual consumers. Keep the
  worker's shutdown escalation policy together in closing; preserve 2s/3s/6s timing.
- [ ] Move runtime worker state into Session; `handle` owns message transitions and
  returns Continue/Stop via Flow. Consolidate repeated send-or-break error handling
  without changing final-snapshot/Exited/Error ordering or callback lifetime.
- [ ] Move workspace keymap/divider/rename/title/reload/factory/routing logic to
  named files. Make CloseGate::decide pure and keep both mouse/keyboard divider
  adapters on shared arithmetic. Reuse PaneServices construction in the factory.
  Aim for the requested roughly 600-line workspace wiring module; move covering
  tests with their responsibilities rather than leaving a monolithic test tail.
  Dispatch modal keyboard behavior with one exhaustive `Mode` match, preserving
  rename, confirmation, idle and divider-drag key behavior with interface tests.
- [ ] Run all offline gates: fmt, clippy --workspace --all-targets -D warnings,
  test --workspace --no-fail-fast, build --workspace, cargo tree duplicates/features,
  plus CI forbidden-state checks. Run paint/capture/Surface budget checks.
- [ ] Confirm source census: one generation increment; no manual return_permit;
  no workspace downcast; one UnixListener::bind site and one production key.matches;
  lib.rs re-exports; no PaneRegistry; size validation only in validated constructor.
- [ ] Write issue-item-to-proof coverage record in docs/performance/design-review.md
  or a linked implementation record. Update all plan checkboxes from evidence.
- [ ] Obtain independent whole-branch review, resolve important findings, repeat
  affected checks, and commit final docs. Push branch and open one PR for #48 using
  creating-a-pull-request skill, with measured evidence and native-platform limits.
  Do not merge or publish a release.
