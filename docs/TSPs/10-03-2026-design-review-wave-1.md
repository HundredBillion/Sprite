# Design Review Wave 1 Technical Spec

> **For agentic workers:** REQUIRED SUB-SKILL: Use dmi-superpowers:subagent-driven-development or dmi-superpowers:executing-plans to implement this TSP task-by-task. Steps use checkbox syntax for tracking.

**Goal:** Deliver the first independently shippable wave of issue #48.

**Architecture:** Retain terminal ownership in the worker and application state in
its existing owners. Encode duplicated invariants in types and keep tests at the
public terminal and GPUI interfaces. No Wave 2/3 work.

**Tech Stack:** Rust 1.97.1, pinned libghostty-vt 0.2.1, GPUI 0.2.2.

## Global Constraints

- No new dependencies or unsafe Send/Sync implementations.
- Preserve separate Surface Channel and Pane Observation grammars.
- Keep latest-only snapshot coalescing and ordered lifecycle delivery.
- Use the existing vocabulary in crates/CONTEXT.md.
- Run test commands with TERM=xterm-ghostty: inherited TERM=dumb caused the
  recorded baseline graphics_tmux failures, and both pass with terminal identity.
- Tests run with --locked --offline. Report every failure; do not mask it.
- Read source requirements from /tmp/sprite-issue-48.json when exact review detail
  is needed. Verify claims against code; the issue is explicitly an unrun review.

### Task 1: Make snapshot mutations advance generation

**Blocked by:** None

**Files:** crates/sprite-term/src/worker.rs;
crates/sprite-term/tests/colors.rs; crates/sprite-app/src/terminal_view.rs
(and a tests submodule if needed).

**Interfaces:** Consumes TerminalCommand::{SetColors,SetCursor} and
Projector::capture(generation,...). Produces private Pending and unchanged public
SnapshotBundle with strictly newer generation for reloads.

- [ ] Preserve the existing uncommitted failing regression in colors.rs. Add a
  GPUI regression driving the actual TerminalView snapshot task on an idle child;
  change ActiveSettings or send the reload command and assert its held bundle
  accepts the new colour/cursor without PTY output. Use a bounded condition wait.
- [ ] Run `cargo test -p sprite-term --locked --offline --test colors
  a_live_colour_reload_repaints_on_its_own -- --exact` and the new app regression;
  confirm generation failure, not startup or environment failure.
- [ ] Introduce the mutation owner:

```rust
struct Pending { generation: u64, dirty: bool }
impl Pending {
    fn mutated(&mut self) { self.generation += 1; self.dirty = true; }
}
```

  Initialise generation 0 and dirty from initial capture. Replace every paired
  increment/dirty assignment with mutated(), including SetColors and SetCursor.
  Capture requests alone mark dirty without inventing a mutation. Pass
  pending.generation to snapshots, hyperlink replies and history capture. Retain
  capture failure and full-slot dirty behaviour.
- [ ] Run terminal colors, snapshot_waiting, history, hyperlink, selection tests
  and the app regression. Assert only one `generation += 1` site remains.
- [ ] Commit with the confirmed root cause and test evidence.

### Task 2: Coalesce bells and own terminal resources

**Blocked by:** Task 1

**Files:** crates/sprite-term/src/worker.rs; crates/sprite-term/src/pty_unix.rs;
crates/sprite-term/tests/session_output.rs; crates/sprite-term/tests/lifecycle.rs.

**Interfaces:** Preserve Pump's external operations and TerminalEvent::Bell.
Produce per-chunk bell coalescing, owned PTY descriptor and structurally ordered
libghostty state destruction.

- [ ] Reproduce a chunk of 16 KiB BEL bytes stalling progress while lifecycle
  consumer is slow. Assert one Bell for a controlled chunk and progress to the next
  snapshot. Use an internal worker seam for chunk identity, public tests for liveness.
- [ ] Replace bell counting with a boolean pending signal, drained once per chunk:

```rust
if bell_pending.replace(false) {
    if events.send_blocking(TerminalEvent::Bell).is_err() { break; }
}
```

  Adapt the existing callback's ownership primitive rather than adding another.
- [ ] Make the pump thread own a duplicated OwnedFd for the PTY master. Prefer the
  pinned nix stdlib-compatible ownership interface; use AsFd for safe reads/writes
  and polls. Remove the borrowed-raw-fd lifetime contract. Preserve nonblocking
  operation, cancellation, EIO/EOF handling and shutdown joining.
- [ ] Inspect pinned libghostty wrapper handle lifetimes, then store projector,
  encoder and terminal in a private Owned struct in declaration/drop order. Avoid
  self-referential borrows; if the existing API prevents this, document evidence
  and resolve the ownership representation before proceeding.
- [ ] Run `cargo test -p sprite-term --locked --offline --test lifecycle
  --test session_output --test input_backpressure` and pump unit tests. Verify
  no unsafe operations remain in pty_unix except required proc-name/dup boundary.
- [ ] Commit resource and bell changes with evidence.

### Task 3: Typed terminal defaults, spawn result and application session state

**Blocked by:** Task 2

**Files:** crates/sprite-term/src/lib.rs; crates/sprite-term/src/worker.rs;
crates/sprite-term/tests/*.rs; crates/sprite-term/src/bin/*.rs;
crates/sprite-app/src/terminal_view.rs; crates/sprite-app/src/terminal_view/theme.rs;
other spawn consumers found by a full repo search.

**Interfaces:**

```rust
pub struct BaseColors { pub foreground: Rgb, pub background: Rgb }
pub struct Spawned {
    pub session: TerminalSession,
    pub events: EventStream,
    pub snapshots: SnapshotStream,
}
// TerminalSession::spawn(config) -> Result<Spawned, SessionError>
// ColorDefaults uses pub base: Option<BaseColors>, retains cursor and palette.
```

- [ ] Add a compile-fail doctest for incomplete BaseColors and verify existing
  colour precedence/reset tests will retain their coverage with paired colours.
- [ ] Change ColorDefaults and all consumers. Delete the independently optional
  foreground/background fields; use the pair in apply_color_defaults.
- [ ] Return Spawned, remove stream Options and take methods from TerminalSession,
  migrate every production/test/benchmark caller, delete take-once runtime tests.
  Destructure at callers: `let Spawned { session, events, snapshots } =
  TerminalSession::spawn(config)?;` with each caller's existing error handling.
- [ ] Introduce application SessionState with NeverStarted, Running(session), and
  Ended(session) variants so the ended handle remains owned until pane cleanup.
  Adapt send, foreground and shutdown consumers with explicit matches.
- [ ] Centralise Settings-to-colour/cursor mapping in one session_defaults adapter
  used at startup and reload. Test identical mappings and fallback colours.
- [ ] Run `cargo test --workspace --locked --offline --no-fail-fast`, recording the
  baseline tmux result separately. Run `cargo clippy --workspace --all-targets
  --locked --offline -- -D warnings` for migrated call sites.
- [ ] Commit the typed interface and its complete caller migration.

### Task 4: Tree-derived focus, safe empty tabs and exclusive workspace modes

**Blocked by:** Task 3

**Files:** crates/sprite-app/src/tabs.rs; crates/sprite-app/src/workspace.rs;
crates/sprite-app/src/terminal_view.rs if necessary for tests.

**Interfaces:** Tabs stores active identity instead of index. active() and
active_tab() represent absence. Workspace's interaction state is:

```rust
enum Mode {
    Idle,
    Renaming(TabRename),
    ConfirmingClose(PendingClose),
    DraggingDivider(DividerDrag),
}
```

- [ ] Add tests for closing the last tab then accessing/rendering it, closing tabs
  before and after active identity, and opening a new tab after empty.
- [ ] Replace active index with TabId (or Option<TabId> for explicit empty state),
  return Options from accessors, migrate all workspace consumers. Empty render
  returns an empty element, never indexing after quit. Preserve neighbour choice.
- [ ] Delete pending_focus and request_focus. In render derive the focused handle
  from the active tree; call the PaneHandle interface so a focused Surface stays
  focused. Only focus when the selected handle differs from window focus.
- [ ] Merge modal Options into Mode, handling cancellation/transition explicitly.
  Test rename to divider gesture, confirmation cancellation, and no simultaneous
  states. Preserve existing keyboard semantics and close safety.
- [ ] Add GPUI tests for pane focus after split/tab change/close and empty render.
  Run `cargo test -p sprite-app --locked --offline`.
- [ ] Commit the workspace state changes.

### Task 5: Precise token refusals and shared dock size validation

**Blocked by:** Task 4

**Files:** crates/sprite-app/src/tokens.rs; crates/sprite-app/src/surface.rs;
crates/sprite-app/src/workspace.rs; crates/sprite-app/src/surface/channel.rs;
crates/sprite-app/src/cli.rs; dock-size consumers.

**Interfaces:** Refusal::TokenConflict carries name and standing Rgb. One DockSize
newtype with TryFrom<f32> owns the existing CLI accepted range; CLI and wire reject
out-of-range/non-finite values identically.

- [ ] Test duplicate token registration as Same without repaint and conflicting
  registration preserving the first colour with a wire refusal naming it in hex.
- [ ] Propagate TokenConflict data through the request and serialisation path;
  call repaint_terminals only for Registration::New. Remove obsolete dead_code
  allowances only where the values now have consumers.
- [ ] Test a shared DockSize table at min/max, below/above, NaN and infinities;
  verify CLI and wire both reject size 1, preserving omission defaults and fill
  handling. Replace wire clamping and CLI independent range checks with TryFrom.
- [ ] Run `cargo test -p sprite-app --locked --offline` and commit.

### Task 6: Align documentation and shared limits; verify the whole PR

**Blocked by:** Task 5

**Files:** DEPENDENCIES.md; docs/adr/0002*; crates/sprite-app/src/config.rs;
crates/sprite-term/src/lib.rs; crates/sprite-app/src/terminal_view/geometry.rs.

**Interfaces:** One exported MAX_CELLS constant used by engine and geometry.
No new config discovery, schema version or file watcher.

- [ ] Correct dependency feature and CI claims to match Cargo and workflows.
  Amend ADR 0002 to describe implemented paths and explicit config reload; fix
  config module documentation. Share MAX_CELLS. Replace decorative Unix cfg with
  a clear supported-platform compile guard or consistently gated modules.
- [ ] Run `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets
  --locked --offline -- -D warnings`, `cargo test --workspace --locked --offline
  --no-fail-fast`, `cargo build --workspace --locked --offline`, and both cargo
  tree checks from CI. Execute the forbidden-states commands from ci.yml.
- [ ] Review the complete diff independently, resolve important findings, update
  this task checklist with commands/results, and commit final documentation.
- [ ] Push the branch and open one PR referencing #48 without a closing keyword.
  Explain Wave 1 coverage, evidence and outstanding baseline/environment failures.
