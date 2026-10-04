# Task 7 report — pane titles and mutation-owned layout

Task brief: `task-7-brief.md` in this directory. Worktree: `/home/hundredbillion/Projects/Sprite/.worktrees/issue-48-design`. Branch: `fix/issue-48-design`. Base: `5a895a3`.

Implementation commit: `7d2748a2fa653b3c98b3444354bc5bcb6606d36e` (`perf(workspace): cache pushed titles and publish layout on mutations`). This report is committed separately; the report commit is returned in the handoff and can be identified with `git log -1 --format=%H -- <this-report-path>`.

## Requirements and implemented behavior

- Explicit OSC titles already arrived through `terminal_events::Effect::Title` and `TerminalView::apply`. That path remains the source of explicit titles. The costly old fallback was `TerminalView::title -> foreground -> kernel/process-name lookup`, called by Workspace on every render.
- `TerminalView::title` now returns a cached display title. OSC effects refresh it immediately. New terminal snapshots, session-end transitions, and the existing half-second blink task refresh fallback titles. No new timer, sleep, dependency, thread, or unsafe Send/Sync implementation was introduced.
- The existing blink wake refreshes titles before checking whether the cursor blinks, so a silent job is discoverable with a steady cursor too. Explicit titles bypass foreground queries entirely on these display refreshes.
- `sprite-pane` now defines `TitleChanged`, its GPUI callback type, and an object-safe subscription method. The blanket handle implementation subscribes through GPUI. The crate remains GPUI-only. Static/unknown-title placeholders supply the marker `EventEmitter` implementation and otherwise retain their previous behavior.
- Workspace reads a pane's title once when inserting it into the title cache, then consumes title event payloads. Notifications rebuild cached tab labels and the desired window title; render never queries the pane title, even through the cheap getter. Explicit tab names still take precedence, unknown tab titles still show their index, and an unknown window title still shows `Sprite`.
- `refresh_layout` is the single application caller of `WindowPanes::set_layout`. It runs at startup and on split/open, pane/tab close (including background exits), focus changes, divider drag/reset/nudge, tab switch, observation toggle, and actual viewport changes reported by GPUI's bounds observer. It is not called from render.
- The mutation owner calculates pixel placements, updates changed visible pane allocations before child rendering, caches divider geometry/group names, and compares the new normalized observation layout with the previously published layout. Unchanged observation data is never published. Rendering consumes cached pane geometry, divider names, and tab labels.
- Resize and tab-strip appearance/removal update pixel allocations. Observation geometry is normalized and its `focused` flag belongs to each tab independently. Thus pixel-only resize and active-tab switches correctly cause **zero** publication when the observation representation is unchanged. Observation re-enable retains the current `WindowPanes` registry, so re-enabling likewise does not require publishing identical data.
- Rename caret labels and close-confirmation text are formatted when mode/input state changes, not on each frame. Divider/group formatting is also outside render.
- Title subscriptions capture a weak Workspace handle. Closed panes lose their cache/subscription entries and cached geometry handles immediately on the close mutation, including the last pane. Events from a still externally retained closed pane are ignored; subscriptions do not retain it.
- Close consent and Surface ownership checks remain live. `close_warning` still calls `foreground`, and `foreground_owner_group` still queries the session directly. Focus restoration remains the existing deferred GPUI path with the hosted-Surface subtree check.
- Row `LayoutCache`, `CellMetrics`, generic Surface handle/request types, observation ordering, configuration design, and tree ownership were not redesigned. Later continuation tasks were not implemented. Parent owns the TSP checkboxes and independent review. No push or PR was performed.

## Source grounding and decisions

| Field | Evidence |
| --- | --- |
| Version chain | Workspace `Cargo.toml`: `gpui = "=0.2.2"`; `Cargo.lock:2103` resolves GPUI 0.2.2; `rustc --version` returned `rustc 1.97.1 (8bab26f4f 2026-07-14)`. |
| Primary API reference | [GPUI 0.2.2 Context](https://docs.rs/gpui/0.2.2/gpui/struct.Context.html): `observe_window_bounds` registers resize callbacks, `subscribe` registers typed entity event listeners, `emit` delivers typed events. |
| Installed source | `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/gpui-0.2.2/src/app/context.rs:429` binds the bounds observer using a weak entity; `src/window.rs:1675` updates viewport state before invoking bounds observers. |
| Subscription lifetime | Same installed GPUI source, `src/app.rs:865` (`subscribe`), `:884` (subscription activation), and `:898` (`subscribe_internal`) store a downgraded emitting entity. Workspace callback also captures only `WeakEntity`, avoiding an application-side cycle. |
| Local precedent | Existing event/snapshot delivery, GPUI subscriptions, deferred focus, and Surface subtree focus protection remain compatible. Previous render-time layout publication was unnecessary for the observed contract; synchronous mutation publication makes normalized geometry current before the next render. |
| Verification | Actual GPUI visual tests exercise resize callbacks, pane event delivery, keyboard focus, mutation entry points, and release of a closed entity. A live PTY test exercises foreground/process-name lookup, OSC reset, existing blink execution, and close consent. |

Code anchors at the implementation commit:

- `crates/sprite-pane/src/lib.rs:25,66,107,137`: event contract and blanket subscription.
- `crates/sprite-app/src/terminal_view.rs:258,309,466,575,586,660`: session-end/snapshot/OSC refresh, cached getter, display-title owner, live close warning.
- `crates/sprite-app/src/terminal_view/render.rs:124`: title refresh on the pre-existing blink task's tick before steady-cursor early return.
- `crates/sprite-app/src/workspace.rs:244,284,367,1599`: viewport observation, single layout owner, cached labels, cache-consuming render.
- `crates/sprite-app/src/observation/panes.rs:100,111`: test publication count and publication method.
- `crates/sprite-app/src/workspace.rs:1966,2006,2108`: idle-render, mutation/publication, and generic title/lifetime GPUI regressions.
- `crates/sprite-app/src/terminal_view/tests.rs:216`: live fallback/OSC/steady-cursor/close-consent GPUI regression.

## Feedback loop and counter scope

The initial GPUI regression was added before the production fix. Exact command:

```sh
TERM=xterm-ghostty cargo test --locked --offline -p sprite-app --lib idle_workspace_does_not_refresh_titles_or_publish_layout -- --nocapture
```

It failed with four redraws changing `(workspace display Strings, title queries, foreground queries, layout publications)` from `(9, 15, 15, 6)` to `(17, 27, 27, 10)`: **+8, +12, +12, +4**. This was the actual Workspace render path, with two tabs and a split, not a synthetic helper benchmark.

After the fix the regression asserts unchanged counters over four forced GPUI redraws. Final fixture also leaves tab rename active, covering the cached caret label while the split's divider group is rendered. Counters now include `TITLE_STRINGS` as well; all five deltas are zero.

Counters mean:

- `WindowPanes::layout_publications`: calls reaching `set_layout`, per registry; test-only atomic counter because the registry is shared with observation threads.
- `TerminalView::TITLE_QUERIES`: cached getter invocations, thread-local and test-only.
- `TerminalView::FOREGROUND_QUERIES`: invocations of the live foreground-state query, including close-warning queries; a failed session may return Idle without entering the kernel.
- `TerminalView::TITLE_STRINGS`: Sprite's owned fallback-display String creation, after comparing the returned process name with the cached title.
- Workspace `DISPLAY_STRINGS`: its display-string construction helper, covering tab labels/index labels, divider group identifiers, rename caret labels, and close-banner formatting.

**Scope:** these are operation/construction counters, not global allocator measurements. Zero new Sprite title/display String construction and zero layout publication are proven for the measured idle Workspace redraw path. GPUI elements, event listeners, vectors, platform title APIs, terminal/Surface rendering, and the whole frame are not claimed allocation-free. An existing blink wake without an OSC title still asks the kernel about the foreground; its returned process-name String may allocate in `sprite-term`. That bounded existing-activity refresh is intentional and is outside the idle-render counter window. Unchanged fallback names avoid a second display String construction in Sprite. No claim is made that elapsed idle time means zero foreground queries.

A second mutation probe removed the production `split` call to `refresh_layout` temporarily and ran:

```sh
TERM=xterm-ghostty cargo test --locked --offline -p sprite-app --lib layout_mutations_publish_only_changed_observation_geometry -- --nocapture
```

The test failed at publication count **1 versus expected 2**. The production call was restored in a `finally` block; subsequent focused/full tests passed. This establishes that the mutation test catches a missing production invalidation rather than merely testing a helper called by the fixture.

The mutation regression checks these cumulative publication counts, before depending on a render to publish anything:

| Mutation | Count |
| --- | ---: |
| Startup | 1 |
| Split | 2 |
| Divider drag | 3 |
| Divider reset | 4 |
| Identical divider reset | 4 |
| Keyboard nudge | 5 |
| Focus change | 6 |
| Resize to 1000 × 700 | 6 |
| New tab / strip appears | 7 |
| Switch tabs | 7 |
| Background pane exit | 8 |
| Close tab / strip disappears | 9 |
| Observation enable/disable | 9 |
| Close final pane | 10 |

It also verifies allocations delivered to real TerminalView entities, tab-strip height changes, and empty caches after the final pane closes. Existing workspace tests cover mouse/keyboard focus, tab switching/closing, close-scope consent, and preservation of a hosted Surface's keyboard focus.

The live PTY test uses `/bin/sh -i` and `cat`. It sets a steady cursor, asserts no foreground queries on existing blink wakes with an explicit OSC title, asserts live close consent reports `cat`, resets OSC and observes a `cat` title event, proves unchanged fallback names reuse their display String, observes return to unknown at the shell, then discovers a new silent `cat` via the existing blink task.

A first full run caught an overly strong test assertion that expected no delayed shell snapshot to arrive after recording its generation (499 passed, 1 test failed: generation 9 versus 6). The deterministic test isolates the timer by dropping **only the test's snapshot-consumer task** before starting the final silent job; it then advances GPUI's existing blink clock and asserts the title changes while snapshot generation remains unchanged. Production behavior was not changed to accommodate the test. No new application sleeps/timers were added; bounded test setup polls a kernel predicate with `yield_now`.

## Exact verification commands and results

All cargo commands used `TERM=xterm-ghostty`, `--locked`, and `--offline`; no manifest or lockfile changed.

| Command | Result |
| --- | --- |
| `TERM=xterm-ghostty cargo test --locked --offline -p sprite-app --lib idle_workspace_does_not_refresh_titles_or_publish_layout -- --nocapture` before fix | Expected red: +8 display Strings, +12 title queries, +12 foreground queries, +4 publications. |
| `TERM=xterm-ghostty cargo test --locked --offline -p sprite-app --lib workspace::tests -- --nocapture` | 49 passed after adapting old fixtures that directly replace tab registries to refresh once during setup. Assertions use production entry points. |
| `TERM=xterm-ghostty cargo test --locked --offline -p sprite-app --lib observation::` | 73 passed. |
| `TERM=xterm-ghostty cargo test --locked --offline -p sprite-app --lib terminal_view::` | 47 passed, including Surface ownership and ordered-event coverage. |
| `TERM=xterm-ghostty cargo test --locked --offline -p sprite-app --lib layout_mutations_publish_only_changed_observation_geometry -- --nocapture` with the split refresh temporarily removed | Expected red: 1 publication, expected 2. Restored before further tests. |
| `TERM=xterm-ghostty cargo test --locked --offline -p sprite-app -p sprite-pane` | Final full run passed: 500 app library tests plus app binary/integration suites, 3 pane tests, and app/pane doctests; **532 total passed**, 0 failed. |
| `TERM=xterm-ghostty cargo test --locked --offline -p sprite-app --lib fallback_titles_use_existing_blink_activity_and_close_checks_stay_live -- --nocapture` | Final stronger steady-cursor test passed (1). This final test-only strengthening followed the full run. |
| `TERM=xterm-ghostty cargo test --locked --offline -p sprite-app --lib workspace::tests` | Final run with rename-active idle fixture: 49 passed. |
| `TERM=xterm-ghostty cargo clippy --locked --offline -p sprite-app -p sprite-pane --all-targets -- -D warnings` | Passed after final test changes. |
| `cargo fmt --all -- --check` | Passed. |
| `git diff --check` | Passed. |
| `rg -n 'set_layout\(' crates/sprite-app/src` | Exactly one application call, in `Workspace::refresh_layout`, plus the method definition. |
| `git diff -U0 \| rg '^\+.*\b(above\|below\|here\|these\|those)\b'` | No added ambiguous comment referents. |

Cargo reports the pre-existing future-incompatibility warning for dependency `proc-macro-error2 v2.0.1`; no new lint warning was accepted. The full app suite naturally runs its benchmark-report integration test; no standalone terminal/paint benchmark sweep was requested or rerun, and no performance budgets or historical timing-breach records were changed.

Temporary command logs are under `/tmp/sprite-task7-{red,workspace2,observation,gpui,mutation-red,full,full2,steady,workspace-final,clippy-final}.log` for this session. They are not committed artifacts.
