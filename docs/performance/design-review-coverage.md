# Issue 48 implementation and proof record

This record maps the three waves of issue 48 to the final implementation and
its executable evidence. Numbers such as 2.1 refer to the issue's type-move
identifiers, not continuation task numbers. It supplements the measured scopes
and original budgets in [design-review.md](design-review.md). It does not claim
native compositor or macOS acceptance from the Linux headless harness.

## Types, ownership and live behavior

| Issue item | Implementation | Interface proof |
| --- | --- | --- |
| 1a / 2.1: idle color/cursor generations | `Pending::mutated` in [worker/mod.rs](../../crates/sprite-term/src/worker/mod.rs) is the sole generation increment. SetColors and SetCursor both use it. | [colors.rs](../../crates/sprite-term/tests/colors.rs) `a_live_colour_reload_repaints_on_its_own`; [terminal_view/tests.rs](../../crates/sprite-app/src/terminal_view/tests.rs) `idle_view_accepts_colour_and_cursor_reloads` and `startup_and_reload_apply_identical_session_defaults`. |
| 1b / 2.2: automatic output permits and pooled buffers | [pty_unix.rs](../../crates/sprite-term/src/pty_unix.rs) `OutputChunk` owns `Permit`; Drop returns the buffer and wakes the pump without blocking. | `pump_delivers_more_than_forty_chunks_when_consumers_only_drop_messages` delivers 64 chunks; discarded inbox, failed sends, chunks outliving pump and full wake socket tests cover Drop. `steady_state_pump_delivers_sixty_four_chunks_without_allocating` measures zero allocations/bytes. The original normal worker returned permits; the confirmed failing case was drop-only consumption. |
| 2.3: one-time stream handoff | [session.rs](../../crates/sprite-term/src/session.rs) returns `Spawned { session, events, snapshots }`; no take-stream state remains. | Every terminal integration fixture destructures `Spawned`; [lifecycle.rs](../../crates/sprite-term/tests/lifecycle.rs) covers spawn, failure and idempotent shutdown. Repeated move of a stream is a Rust ownership error. |
| 2.4: paired base colors | [config.rs](../../crates/sprite-term/src/config.rs) `ColorDefaults.base: Option<BaseColors>`. | BaseColors positive/compile-fail doctests; [colors.rs](../../crates/sprite-term/tests/colors.rs) configured, program override and reset behavior. |
| 2.5: validated terminal dimensions | [config.rs](../../crates/sprite-term/src/config.rs) `ValidTerminalSize::new` is the only dimensional validation implementation; SessionConfig and Resize carry the validated type. | Constructor boundary/generated-dimension tests; [session_io.rs](../../crates/sprite-term/tests/session_io.rs) resize refusal and backend agreement; [type_invariants.rs](../../crates/sprite-app/tests/type_invariants.rs) extracts the actual relocated size doctests and verifies E0308. |
| 2.6: structural resource order | [worker/mod.rs](../../crates/sprite-term/src/worker/mod.rs) `Owned` declares projector, key encoder, mouse encoder, terminal in drop order. [pty_unix.rs](../../crates/sprite-term/src/pty_unix.rs) owns a duplicated `OwnedFd` and denies unsafe code except the audited duplication/process-name blocks. | Descriptor close-on-exec and caller-descriptor closure tests; [lifecycle.rs](../../crates/sprite-term/tests/lifecycle.rs) shutdown/reaping/stubborn-descendant tests. The new bounded full-queue/lost-Ready regression exercises production cleanup. |
| 2.7: bounded bell notices | [worker/mod.rs](../../crates/sprite-term/src/worker/mod.rs) `Notices` coalesces Bell at chunk end, keeping title/directory events in parser order. | `one_bell_per_chunk_and_no_bell_for_the_next_silent_chunk`; [session_output.rs](../../crates/sprite-term/tests/session_output.rs) `bell_burst_does_not_stall_snapshots_with_a_paused_event_consumer`. Mixed Bell/title/directory ordering has no additional dedicated integration test. |
| 2.8: tree-derived focus | [workspace/keymap.rs](../../crates/sprite-app/src/workspace/keymap.rs) derives focus from the active pane; there is no pending-focus copy. | `window_focus_follows_split_tab_switch_and_close`; [surface_routing.rs](../../crates/sprite-app/src/workspace/surface_routing.rs) `repaint_preserves_a_hosted_surfaces_keyboard_focus`. |
| 2.9: stable active-tab identity | [tabs.rs](../../crates/sprite-app/src/tabs.rs) owns optional active TabId and returns optional active content. | Tabs tests cover empty/last-tab closure and switching. The workspace focus test closes all tabs, renders, reopens and verifies window focus. |
| 2.10: owning pane tree | [pane_tree.rs](../../crates/sprite-app/src/pane_tree.rs) leaves own `(PaneId, T)`, including empty-tree/optional-focus behavior; no PaneRegistry. | Drop-spy tests, `every_pane_appears_in_the_layout_with_its_own_contents`, refusal/panic construction and generated transition sequences. [terminal_view/tests.rs](../../crates/sprite-app/src/terminal_view/tests.rs) proves immediate observation unregister at begin_shutdown with retained handles. [close_gate.rs](../../crates/sprite-app/src/workspace/close_gate.rs) proves window-wide identity-ordered cleanup once across background tabs. |
| 2.11: terminal lifecycle states | [terminal_view.rs](../../crates/sprite-app/src/terminal_view.rs) `SessionState` separates NeverStarted, Running and Ended; sending is a match. | Terminal view and [terminal_events.rs](../../crates/sprite-app/src/terminal_events.rs) lifecycle tests cover failed startup, normal/requested/signalled exit and effect delivery. |
| 2.12: exclusive workspace modes | [workspace/mod.rs](../../crates/sprite-app/src/workspace/mod.rs) owns Mode; [keymap.rs](../../crates/sprite-app/src/workspace/keymap.rs) has one exhaustive keyboard match. | `modal_keys_reach_only_the_intended_consumer` uses focused GPUI panes to check idle pass-through, rename editing/commit/cancel, close confirmation/cancel and divider keyboard behavior; `modes_cancel_and_close_confirmation_remains_scope_specific` checks actual close actions. Removing the rename arm made the new test fail. |
| 2.13: real PaneHandle Surface seam | [sprite-pane/lib.rs](../../crates/sprite-pane/src/lib.rs), TerminalView's implementation and [surface_routing.rs](../../crates/sprite-app/src/workspace/surface_routing.rs) route through the trait; no workspace downcasts. | `placeholder_surface_replies_complete_and_missing_panes_are_unknown`, hosted-focus regression and sprite-pane trait tests exercise terminal and placeholder adapters. |
| 2.14: one authenticated socket transport | [local_socket.rs](../../crates/sprite-app/src/local_socket.rs) owns bind/auth/caps/deadlines/cancel; observation and Surface remain separate adapters and grammars. [reload.rs](../../crates/sprite-app/src/workspace/reload.rs) owns the shared reply relay. | LocalSocket boundary/auth/buffered-followup/trickle/cap/cancel/panic tests, both endpoint test suites and wire tests. There is one literal UnixListener::bind including fixtures and one production key.matches call. |
| 2.15: typed Surface content/ownership/styles | [description.rs](../../crates/sprite-app/src/surface/description.rs) `Element`, [channel.rs](../../crates/sprite-app/src/surface/channel.rs) `Placement`/`Ownership`, [style.rs](../../crates/sprite-app/src/surface/style.rs) parsed style values and [host.rs](../../crates/sprite-app/src/surface/host.rs) own valid states. | Existing description/host/wire regressions plus [type_invariants.rs](../../crates/sprite-app/tests/type_invariants.rs) compile controls and exact-error refusals for private/public contracts; Fill cannot carry a return target. |
| 2.16: shared dock sizes | [channel.rs](../../crates/sprite-app/src/surface/channel.rs) `DockSize`; CLI and [wire.rs](../../crates/sprite-app/src/surface/wire.rs) share conversion. | `open_sizes_are_validated_consistently_with_the_cli` and `wire_and_cli_reject_nonfinite_sizes`. |
| 2.17: informative token refusal, no no-op repaint | [tokens.rs](../../crates/sprite-app/src/tokens.rs), [surface.rs](../../crates/sprite-app/src/surface.rs), [surface_routing.rs](../../crates/sprite-app/src/workspace/surface_routing.rs) preserve TokenConflict and repaint only for New. | `duplicate_token_registration_does_not_repaint_panes` checks notification counts for both adapters and exact name/standing-hex wire refusal. |
| 2.18: coherent measured cell metrics | [terminal_view/theme.rs](../../crates/sprite-app/src/terminal_view/theme.rs) `CellMetrics::measure`; font settings replace the aggregate. | `reload_replaces_the_measured_font_and_surface_metrics_together`; compile controls/refusals in type_invariants. |
| 2.19: one settings-to-session adapter | [terminal_view/theme.rs](../../crates/sprite-app/src/terminal_view/theme.rs) `session_defaults`, used by startup and live settings. | [terminal_view/tests.rs](../../crates/sprite-app/src/terminal_view/tests.rs) `session_defaults_pair_fallbacks_and_preserve_configured_preferences` and startup/reload equivalence. |
| 2.20: typed geometry edges | [grid.rs](../../crates/sprite-app/src/grid.rs) Col/Row and [box_drawing.rs](../../crates/sprite-app/src/box_drawing.rs) Snapped, consumed by grid_paint. | Existing tiling/geometry tests and type_invariants' raw-pixel/row-column compile refusals with compiling controls. |
| 2.21: canonical validated configuration | [config/raw.rs](../../crates/sprite-app/src/config/raw.rs), [validated.rs](../../crates/sprite-app/src/config/validated.rs), [changes.rs](../../crates/sprite-app/src/config/changes.rs) and [workspace/reload.rs](../../crates/sprite-app/src/workspace/reload.rs). | [properties.rs](../../crates/sprite-app/src/config/properties.rs) whole-settings roundtrips, drawable metrics, malformed siblings, escaped keys, normalized preferences, canonical collections and non-UTF8 CLI/session commands. Type-invariant tests refuse constructor bypass; theme tests compare reports with actual live effects. |

The actual authenticated self-disable test is now
`workspace::reload::tests::reload_reconciles_observation_endpoint_and_revokes_old_credentials`.
Its subprocess filter was updated and the parent also asserts that exactly one
test ran. It proves the initiating reply arrives before EOF, the old socket
disappears, and reenabling produces a new key. The separate
[local_socket.rs](../../crates/sprite-app/src/local_socket.rs) test
`closing_for_one_reply_cancels_other_clients_and_preserves_write_timeout` proves
other clients are cancelled while the initiating reply retains its write timeout.
The opaque ReplyConnection is preserved unchanged from transport through request and
relay. LocalSocket tests reject foreign, stale and reopened identities;
[type_invariants.rs](../../crates/sprite-app/tests/type_invariants.rs) preserves the
opaque-identity compile proof, positive controls, exact diagnostics and required
source-marker failures.

## Final module seams and lifecycle

The [terminal crate root](../../crates/sprite-term/src/lib.rs) is 50 lines of
module declarations, platform guards and stable reexports. Its public families
live in session, command, event, config, render and pane. HyperlinkSpan lives with
events. Hyperlink resolution and text fallback use the same scheme allow-list.
Input keys, mouse/selection/wheel and paste live in separate input modules.
Existing public integration tests still import the same crate-root names.

The worker uses `Session::handle(Message) -> Flow`, where Flow is the standard
`ControlFlow<()>` and `Break` is locally named `Stop`. `emit` is the sole fallible
worker event sender. Callback notices and clipboard writes are drained outside
the parser; losing their consumer now stops the session, including inside a
notice loop. Projection errors retain dirty state; Projector invalidates reuse
on failure, independently tested by both injected capture failure sites.

Worker termination publishes the final dirty snapshot, drops Owned (and thus
terminal callbacks), cancels the pump, drains its reports/output before joining,
then handles waiter/final events and master closure. The explicit Owned field
order remains the lifetime guarantee. [closing.rs](../../crates/sprite-term/src/worker/closing.rs)
keeps TERM at 2 seconds, KILL at 3 and give-up at 6, measured from the first
observed shutdown request. A natural close retains its separate deadline.
Latest-only snapshots, ordered events and pooled output are retained.

A new test, `closed_ready_consumer_drains_a_full_worker_queue_before_join`,
preloads the worker queue, closes the Ready receiver and runs the production
worker. Before the change it blocked in Pump::Drop; the parent test process
killed and reaped its child after 10 seconds and failed. After the change it
finishes. The same cleanup path now handles errors constructing terminal state
after pump startup. That constructor-error routing was checked in code; the new
regression specifically reproduces lost Ready with a full queue, not a forced
native allocation failure. Pre-pump missing-executable failure remains covered
by the public lifecycle test. Post-pump construction failures now report Error
only after cleanup, before any available child exit report; they never emit Ready.

The workspace wiring module is 495 lines. Keymap, divider (including overlay),
close gate, tab strip/title subscriptions, reload, pane factory and Surface routing
own their implementation and tests. Two wiring/layout tests live in a dedicated
layout_tests module; reusable GPUI fixtures live in test_support. All 51 original
workspace tests remain, plus pure close-consent, focused key-routing and retained-entity allocation tests.
CloseGate::decide takes values without a Window; keyboard nudges and mouse drags
use the same divider arithmetic. PaneServices remains the shared factory input.

## Documentation and retained contracts

[DEPENDENCIES.md](../../DEPENDENCIES.md) describes actual CI tree inspections,
serde_json preserve_order and the current dependency features, rather than
claiming unused-dependency/license/vulnerability gates. [ADR 0002](../adr/0002-use-toml-for-configuration.md)
describes actual config discovery, explicit reload, lack of schema/watchers and
normalized UTF8 preferences versus arbitrary OS command arguments. Config module
docs describe all implemented settings. MAX_CELLS has one exported definition;
the application imports it. Terminal Core explicitly supports Linux/macOS.

The earlier review's chart/score observations are historical corrections, not
new measurements: the omitted Surface grid and pane-tree files were larger than
the selected interface file, 67/8 = 8.375 was an unweighted mean, and a finding
count does not derive an 80/20 coverage bar. This record uses item-level proofs
instead of reproducing that score. Worker density is resolved by responsibility
seams, not a claim that a smaller line count alone improves correctness.

The existing deep modules, independent render/pane projections, image generation
identity, row batching, event notification batching and list virtualization remain.
No content-hashed graphics cache or unmeasured box-geometry cache was added.

## Performance proof map

| Issue proposal | Actual measured seam and proof |
| --- | --- |
| Paint baseline first | [paint baseline](design-review-paint-baseline.json), [measurement contract](design-review.md): real production preparation and draw decisions, 200×60 mixed cells, four actual transitions in whole/split modes; excludes font/GPU/window work. |
| Memoized row layout, blink and hover | [grid.rs](../../crates/sprite-app/src/grid.rs), [paint_benchmark.rs](../../crates/sprite-app/src/paint_benchmark.rs), [shared report](design-review-paint-shared.json): all measured blink samples have zero Rust allocations/bytes; hover rebuilds affected rows. |
| Shared cell text, rows and palette | [render.rs](../../crates/sprite-term/src/render.rs), [snapshot.rs](../../crates/sprite-term/src/snapshot.rs): scalar-inline/grapheme-shared CellText, row and palette identity reuse. [capture baseline](design-review-capture-baseline.json) versus [shared capture](design-review-capture-shared.json): 10,206 to 104 Rust allocations; native Ghostty allocations excluded. |
| Dirty-row correctness | Snapshot tests compare reused results with fresh projection after resize, viewport, selection, screen, image, OSC colors and semantic-only prompt changes; both failure/retry tests invalidate caches. Paint one-row transition allocates only the changed row in each pass mode. |
| Pushed titles and mutation-driven layout | [tab_strip.rs](../../crates/sprite-app/src/workspace/tab_strip.rs), [layout_tests.rs](../../crates/sprite-app/src/workspace/layout_tests.rs): idle render creates zero counted display strings and no title/geometry publications; geometry changes publish only changed normalized layout. [terminal_view/tests.rs](../../crates/sprite-app/src/terminal_view/tests.rs) retains live foreground checks for close consent. |
| Surface grid/list diet | [Surface report](design-review-surface-shared.md): 8-byte stored cell, bounded live text interner, immutable retained rows, real 100k-row GPUI virtualization at three scroll positions, one index per replacement and no index rebuild for state-only updates. Production allocation and generated transition tests enforce the budgets. |
| Batched event writes | Surface report and [trace checker](../../scripts/check_surface_wheel_trace.py): actual wheel handler on an empty Unix socket pair sends five JSON events in one syscall. 64 KiB chunks retain gesture-wide ordering; backpressure/partial writes and pre-open caps are tested separately. No one-syscall claim under arbitrary backpressure. |

Original reports and budgets are retained. The final rerun reports are separate;
none of their automatically generated budgets replaces an existing acceptance
threshold. A held historical immutable render frame may keep its own text alive;
the Surface text-pool bound describes the active grid, not arbitrary retained
GPUI frame histories. Zero paint allocations describes CPU preparation/decisions,
not a full rendered frame.

## Final functional and static gates

Implementation revision: `b3ab5a4`, following pre-split revision
`b0aae2d75a77f5efc19476f198cb64c8497c8818`. Linux x86_64, Intel i7-11850H,
Rust/Cargo 1.97.1, locked/offline dependencies. These checks passed:

- `cargo fmt --all -- --check`.
- `TERM=xterm-ghostty cargo clippy --workspace --all-targets --locked --offline -- -D warnings`.
- `TERM=xterm-ghostty cargo test --workspace --locked --offline --no-fail-fast`:
  771 top-level Rust tests passed, two existing tests ignored. The captured log
  also contains the shutdown subprocess's one passing result. After adding the
  retained-entity allocation fixture, the three affected layout tests passed;
  after strengthening subprocess-count verification, the shutdown test passed.
- `TERM=xterm-ghostty cargo build --workspace --locked --offline` and
  `cargo build --workspace --bins --release --locked --offline`.
- `cargo tree --locked --offline --duplicates` and
  `cargo tree --locked --offline --edges features`; these inspect the tree and
  do not constitute dependency vulnerability/license audits.
- `python3 -m unittest scripts.test_release_automation -v`: nine tests passed.
- All four CI forbidden-state scans passed. Source census found one generation
  increment, no manual return_permit, no Workspace downcasts **including test
  fixtures**, one literal UnixListener::bind including fixtures, one production
  key.matches, no PaneRegistry and one exported MAX_CELLS definition.

The existing proc-macro-error2 2.0.1 future-compatibility notice remains. No
new dependency was added by the final split. The allocation assertion formerly
using a test downcast now retains concrete TerminalView entities before trait
coercion; it still checks actual workspace-assigned terminal dimensions.

Surface verification used the current app library test executable obtained from
Cargo JSON, not a stale hash:

```sh
TERM=xterm-ghostty cargo test -p sprite-app --lib --locked --offline \
  --no-run --message-format=json > /tmp/sprite-task11-app-artifact.json
# Select compiler-artifact with target.name == sprite_app, profile.test == true,
# and a non-null executable; require exactly one result.
TERM=xterm-ghostty "$APP_TEST" surface_performance --nocapture --test-threads=1
TERM=xterm-ghostty "$APP_TEST" surface_list_100k_virtualization_probe --nocapture --test-threads=1
TERM=xterm-ghostty /tmp/sprite-syscall-tools.B9rhrh/usr/bin/strace \
  -f -e trace=write,writev,sendto,sendmsg -s 2048 \
  -o /tmp/sprite-task11-wheel.trace "$APP_TEST" \
  surface_wheel_syscall_probe --nocapture --test-threads=1
python3 scripts/check_surface_wheel_trace.py /tmp/sprite-task11-wheel.trace
```

Actual results match the [shared Surface report](design-review-surface-shared.md):
8-byte cells; idle grid render 1 allocation/632 bytes; one-row update 7/10,960;
100k list parse 300,018/40,160,012; replacement 300,019/40,160,018; state-only
15/792. The real list callback made 51 truncations for 47 visible rows at each
of three scroll positions. The traced wheel gesture made one syscall, 399 bytes,
five complete JSON lines. It uses a synthetic socket pair and no real session
credentials.

## Final release measurements and unresolved timing gates

Each normal metric used 30 samples; the existing graphics large-image cap of
10 and observation stalled/history caps of six remain. Runs were sequential,
without concurrent compilation or test commands. Original reports were not
replaced. Final artifact paths are:

- [paint final](design-review-paint-final.json): all eight allocation/byte
  maxima pass both original and shared budgets. Both blink modes remain zero
  allocations and zero bytes in every sample. First-frame counts remain 122;
  hover and one-row transitions remain three.
- [capture final](design-review-capture-final.json): 104 allocations and
  14,656 Rust bytes in every isolated sample; both original and shared count/
  byte budgets pass. Original isolated timing passes; shared isolated timing
  **fails**.
- [graphics final](design-review-graphics-final.json): all four original latency
  budgets pass; storage after 30 and 60 transmissions is 65,536 bytes.
- [observation final](design-review-observation-final.json): one of five carried
  timing gates passes.

The first capture miss triggered a pre-split comparison, not a search for a
passing result. A detached worktree at `b0aae2d75a77f5efc19476f198cb64c8497c8818`
was built with the same workspace release profile, dependency features and pinned
native library. [Pre-split capture](design-review-capture-before-final-split.json)
and [pre-split observation](design-review-observation-before-final-split.json)
were measured, followed by [paired final capture](design-review-capture-final-paired.json).
All reports are kept. The final binary was copied before building the comparison
revision. Rebuilding the final `b3ab5a4` implementation produced the exact same
SHA-256: both final capture runs therefore measure the final committed code.

Binary SHA-256 identities:

```text
pre-split terminal 0b36c424a71be7c74db80c2d34667c8cf0bfe8555c6771b8c3f852d8513aed22
final terminal     16769b60cbc2d6e8d000e680198cd06fd5b3eb8a925d97972fbef00813e6a9ee
pre-split observe  99973c48cdb4d17d2468149381e03d4cfca8c8d26a826ad2a5fdd93a4ac3dcd5
```

The following tables use original carried timing budgets, except the isolated
Projector row which uses the tighter shared capture budget. All values are
milliseconds, and bold values exceed the named budget.

| Terminal metric | Budget | Pre-split p95 | First final p95 | Paired final p95 |
| --- | ---: | ---: | ---: | ---: |
| `spawn_to_ready` | 0.869961 | **1.473555** | **1.116461** | **1.405551** |
| `input_to_snapshot_idle` | 0.162089 | 0.142758 | 0.065106 | 0.060084 |
| `input_to_snapshot_under_load` | 13.550327 | 5.977734 | 3.988617 | 3.719377 |
| `output_10mib_to_final_snapshot` | 1708.276404 | 414.815691 | 315.413914 | 347.313116 |
| `capture_100x100_grid` | 0.097317 | **0.353395** | **0.167543** | **0.200785** |
| `capture_with_full_scrollback` | 1.077974 | **1.440728** | 0.540695 | 0.741036 |
| `scroll_round_trip` | 0.722521 | **0.893021** | **0.896433** | **0.878827** |
| `select_full_screen` | 0.146845 | **0.333699** | **0.148316** | **0.216802** |
| `isolated_projector_capture` | 0.017937 | **0.027819** | **0.020908** | **0.038870** |

| Observation metric | Budget | Pre-split p95 | Final p95 |
| --- | ---: | ---: | ---: |
| `collect_four_panes` | 0.001044 | **0.001476** | **0.001185** |
| `collect_sixteen_panes` | 0.002956 | **0.004921** | **0.006174** |
| `collect_with_one_stalled_pane` | 550.225332 | 500.157782 | 500.192293 |
| `encode_default_request` | 1.416955 | **2.164648** | **1.991566** |
| `encode_maximum_history` | 24.815041 | **38.458949** | **33.233474** |

The three previously documented misses remain historical evidence:
`capture_100x100_grid` 0.150657 > 0.097317,
`collect_sixteen_panes` 0.003070 > 0.002956, and
`encode_default_request` 1.495205 > 1.416955.
The new runs add failures beyond those three; no threshold was increased and
no failing run was discarded.

For isolated Projector work, pre-split median/p95 was 0.015019/0.027819,
first final 0.016424/0.020908 and paired final 0.022221/0.038870. The paired final
slowdown is unresolved. Every original session p95 in that paired final is at or
below its contemporaneous pre-split counterpart, but that does not establish
that the tighter isolated miss is harmless. The synchronous capture source and
fixture are unchanged. An instruction-shape comparison found 1,499 instructions
in each Projector::capture and 19 in each CaptureBenchmark::capture, with equal
sequences after normalizing relocated addresses, module paths and symbol
comments. This is not a proof of equivalent machine code or timing: call targets,
layout, caches and scheduling still matter. The host was using the powersave CPU
governor; an observed load average was 3.75/3.35/2.85. Those observations do not
prove noise or justify waiving a budget. The shared timing miss and all carried
timing failures remain review risks.

A subsequent [predefined CPU-affinity comparison](design-review-capture-controlled/README.md)
ran the preserved binaries once in balanced ABBA BAAB order, after fixed A/B
warmups, with 30 samples per invocation. All ten reports and the protocol are
retained. The median of four paired final/baseline ratios was 0.982397 for medians
and **1.366733 for p95s**. Final p95 was higher in three of four pairs, in both
orders; three measured final runs still exceeded the shared 0.017937 ms budget.
The median of four run p95s was 0.0238615 ms final versus 0.0176810 ms baseline;
these are not pooled percentiles. The focused source/binary assessment found no
added direct capture work or actionable production defect, but does not establish
a cause or dismiss the tail slowdown. Timing acceptance remains unresolved;
there were no further benchmark runs or threshold changes.

Reproduction commands for the final measurements:

```sh
TERM=xterm-ghostty cargo build --workspace --bins --release --locked --offline
TERM=xterm-ghostty target/release/sprite-paint-bench --samples 30 \
  --output docs/performance/design-review-paint-final.json \
  --check-budgets docs/performance/design-review-paint-shared.json
TERM=xterm-ghostty target/release/sprite-term-bench --samples 30 \
  --output docs/performance/design-review-capture-final.json
python3 scripts/check_capture_budgets.py \
  docs/performance/design-review-capture-final.json \
  docs/performance/design-review-capture-baseline.json
# Expected failure on the recorded final report: shared timing is over budget.
python3 scripts/check_capture_budgets.py \
  docs/performance/design-review-capture-final.json \
  docs/performance/design-review-capture-shared.json
TERM=xterm-ghostty target/release/sprite-graphics-bench --samples 30 \
  --output docs/performance/design-review-graphics-final.json
TERM=xterm-ghostty target/release/sprite-observation-bench --samples 30 \
  --output docs/performance/design-review-observation-final.json
```

The paint report's allocation/byte maxima were additionally compared with every
original baseline threshold using the same metric and transition keys. Session,
graphics and observation p95 values were compared with checkpoint-2-arch,
checkpoint-4-arch and checkpoint-3-arch respectively. Automatically generated
budgets in new reports are descriptive only. Shared capture checks of both the
pre-split and paired-final reports also exit nonzero on timing, after passing
the allocation/byte checks.

Native Wayland/X11 compositor interactions, interactive real macOS, native
clipboard/window behavior and GPU/font rasterization performance remain outside
this host's validation.


## Whole-branch review and PR readiness

Independent review covered all 150 changed paths from `cee803a` through
`69bcea3`, including the final controlled measurement evidence. It found no
actionable Critical or Important code defect. The functional, static and scoped
allocation evidence supports opening a draft PR for all three waves.

At that review the branch was **not ready to merge**. Capture timing acceptance
was an Important unresolved gate: the controlled comparison reports a median paired
p95 ratio of 1.3667, and three of four final runs exceed the unchanged shared
budget. No source-level cause or accepted exception had been established.
Opening the draft PR did not waive that gate or claim native-platform acceptance.


## Capture follow-up after draft PR

The [observation-row sharing follow-up](design-review-capture-row-sharing/README.md)
resolves the isolated capture gate without raising its 0.017937 ms threshold.
All four corrected runs pass at p95 0.006758–0.007946 ms; each allocates four
Rust objects / 3,456 requested bytes. An allocation regression failed before
(104 / 14,656) and passes after. Cold-row costs, all controlled results, the
773-test workspace run, and independent bounded review are in that record.
The earlier reports remain historical evidence, not current gate failures.
Unrelated legacy timing limits and native-platform limitations remain disclosed.

The previous PR CI run additionally exposed two test portability defects:
a macOS test changed a socket timeout after peer shutdown, and the compile-proof
helper chose Serde/TOML artifacts that loaded independently but used different
Serde trait identities. The test now sets its deadline before shutdown; the
helper compiles a joint derive/parse probe before choosing Serde. The actual
mixed-artifact proof was reproduced locally (exit 101) and passes after the fix;
all three proof tests and eleven socket tests pass locally. The macOS run also
reported a five-second snapshot watchdog in the image steady-state test; no
speculative production fix or timeout increase was made for that observation.
Remote CI verification remains separate from the Linux capture result.
