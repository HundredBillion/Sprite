# Authorized Pane Control Technical Spec

> **For agentic workers:** REQUIRED SUB-SKILL: Use dmi-superpowers:executing-plans to implement this TSP task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** Deliver issue #106's public pane CRUD workflow with explicit, revocable owner grants.

**Architecture:** A control module owns the public grammar, grant endpoint and client. The Workspace approves grants and executes addressed operations on GPUI through a bounded lane; existing local transport, claim/abandon replies, pane handles and cleanup remain authoritative.

**Tech Stack:** Rust 1.97.1, vendored GPUI 0.2.2, serde/serde_json, async-channel, UnixStream; no new dependencies.

## Global Constraints

- Source: `docs/PRDs/10-26-2026-authorized-pane-control.md`, owner-approved 2026-10-26.
- 16 active grants/window, 16 connections/grant, 16 queued control operations/window.
- 8 KiB request lines, 256-byte labels, 4 KiB input payloads.
- 128 live panes and 16 pending cleanups/window bound agent mutation admission.
- All-panes grants include existing and future panes; fixed scope includes its created panes.
- Preserve observation/Surface credentials, snapshots and keyboard behavior.
- Input is independent of manage; busy close never accepts unattended confirmation.
- Move preserves handle/session identity within a window; cross-window move is unsupported.
- Recheck authorization on GPUI and confirmation. No late application after abandonment.
- No TCP, new dependencies, arbitrary-PID signals or managed-command/event subsystem.

## File map

- `control/mod.rs`: public request/reply and capabilities, endpoint and lane ownership.
- `control/client.rs`: public CLI parsing, descriptors, exchanges and exit outcomes.
- `workspace/control.rs`: owner approval, grant execution, confirmations and access UI.
- `workspace/control_tests.rs`: public client/protocol acceptance on real Workspace.
- `tabs.rs`, `pane_tree.rs`: addressed insert/move/divider operations without recreation.
- `observation/panes.rs`: publish current tab membership while retaining capture tickets.
- `workspace/close_gate.rs`: preserve cleanup and resolve moved exit membership.
- `terminal_view.rs`: terminal control request handling and content capabilities.
- `cli.rs`, `main.rs`, `lib.rs`: route new commands without changing old invocations.
- `docs/pane-control.md`: public workflow, trust/permission model and protocol reference.

## Task 1: Address panes without changing sessions

**Blocked by:** None

**Files:** Modify `crates/sprite-app/src/pane_tree.rs`, `tabs.rs`, `observation/panes.rs`, `workspace/mod.rs`, `workspace/close_gate.rs`.

**Interfaces:**
- Consumes existing `PaneTree::close`, `Node::split_leaf`, `Tabs::all_panes`.
- Produces addressed `Tabs::tab_of(PaneId) -> Option<TabId>`, `Tabs::get(PaneId) -> Option<&T>`, `Tabs::insert_beside(PaneId, Direction, impl FnOnce(TabId, PaneId) -> T) -> Option<PaneId>`, `Tabs::move_pane(PaneId, Option<(PaneId, Direction)>) -> bool`.
- Produces `Tabs::tree(TabId) -> Option<&PaneTree<T>>`, `tree_mut`, and observation membership publication `WindowPanes::set_tab(PaneId, TabId)`.

- [x] Add one failing ownership test before implementing movement:

```rust
#[test]
fn moving_between_background_tabs_retains_the_payload() {
    let value = std::rc::Rc::new(());
    let mut tabs = Tabs::new(|_, _| value.clone());
    let source = PaneId(0);
    tabs.open(|_, _| std::rc::Rc::new(()));
    let destination = PaneId(1);
    assert!(tabs.move_pane(source, Some((destination, Direction::Left))));
    assert!(std::rc::Rc::ptr_eq(tabs.get(source).unwrap(), &value));
    assert_eq!(tabs.tab_of(source), tabs.tab_of(destination));
}
```

- [x] Run `cargo test -p sprite-app --lib moving_between_background_tabs --locked --offline`; expect missing addressed methods before implementation, then one passing test.
- [x] Add leaf insertion that preserves current focus; validate both IDs before removing a move source. Insert transferred payload with the same ID; allocate a tab only for new-tab moves. Make existing split delegate to the same leaf insertion mechanism.
- [x] Publish current tab membership before layout, retaining command sender and capture waiters. Change `close_exited_pane` to resolve the current tab by PaneId, ignoring the original captured tab.
- [x] Run existing tabs/tree/observation and workspace close tests. Commit `feat: address and move panes without replacing sessions`.

## Task 2: Public protocol, grant endpoint and client

**Blocked by:** Task 1

**Files:** Create `crates/sprite-app/src/control/mod.rs`, `control/client.rs`; modify `cli.rs`, `main.rs`, `lib.rs`, `workspace/mod.rs`, `workspace/reload.rs`.

**Interfaces:**
- `Request { protocol: String, id: String, window: String, operation: Operation }`.
- `Operation`: List, Read, Create, Rename, Move, Focus, Resize, Close, Input.
- `Response { protocol: String, id: String, outcome: String, result: serde_json::Value }`.
- `AccessRequest { all_panes: bool, panes: Vec<u64>, manage: bool, input: bool, from: Option<u64> }`.
- `Descriptor { window: String, grant: String, socket: PathBuf, key: String }` (no Debug).
- `ControlMessage { grant: u64, request: Request, reply: Relayed<Response>, connection: Option<ReplyConnection> }`.
- `Invocation::PaneControl(control::client::Args)`; parser and client share typed operations.

- [x] Add a public CLI test that parses `panes list --access /tmp/access.json` and rejects unknown/duplicate flags.
- [x] Run `cargo test -p sprite-app --lib control --locked --offline` before implementing; observe failure.
- [x] Implement serde-tagged operations with denied unknown fields, size/label/key checks and strict version/window validation. Render stable JSON outcomes; do not log tokens. Use `LocalSocket::open_in` with a fresh key per grant and existing transport caps/timeouts.
- [x] Handle `panes access request` on the authenticated observation adapter before snapshot parsing. Queue at most one pending approval; use a 60-second reply deadline. Read-only credentials can request but cannot approve.
- [x] Implement `--access` descriptor loading, repeated descriptors plus explicit `--window`, exclusive owner-only output file creation, bounded replies, nonzero refusal exits, and outcome-unknown transport reporting after a sent mutation.
- [x] Run public grammar/client tests and existing CLI tests. Commit `feat: add public pane control protocol and client`.

## Task 3: Owner approval and addressed CRUD

**Blocked by:** Task 2

**Files:** Create `crates/sprite-app/src/workspace/control.rs`; modify `workspace/mod.rs`, `workspace/reload.rs`, `workspace/close_gate.rs`, `workspace/tab_strip.rs`.

**Interfaces:**
- `Workspace::serve_control(ControlMessage, &mut Window, &mut Context<Self>)`.
- `Workspace::answer_access(bool, &mut Context<Self>)` approves or denies the pending request.
- `Workspace::revoke_control(u64, &mut Context<Self>)` removes endpoint and grant.
- `Workspace::control_elements(&mut Context<Self>) -> Vec<gpui::AnyElement>` renders pending approval, grant controls and operation confirmations.

- [x] Add public request test: observation credential alone cannot label a pane; pending access leaves it unchanged; explicit owner approval permits the same operation.
- [x] Run control acceptance test red, then implement pending access and visible Allow/Deny actions. Keep a Pane Access button visible while grants exist, offering revoke and remove-manage/remove-input. Display claimed requester identity as unverified.
- [x] Drain a bounded operation lane with existing `lane_drain`. Claim each request, recheck live grant and permissions, then execute on GPUI. Use window-incarnation/pane addresses and never temporary focus switches for background mutations.
- [x] Implement inventory/read metadata, separate pane labels, configured-shell creation, identity-preserving move, explicit focus and persistent divider resize. Admission checks count live panes and unfinished cleanups. Return exact clamped allocations.
- [x] Implement target-specific busy close confirmation with explicit UI decision and expiry. A repeated request cannot confirm. Recheck target/permission at approval, then use existing shutdown and after-close behavior. Preserve final reply connection while canceling endpoints.
- [x] Test all-panes/fixed scopes, future panes, wrong/stale addresses, revoke-before-execute, close races and redraw persistence. Commit `feat: grant and execute authorized pane management`.

## Task 4: Supported content and distinct terminal input

**Blocked by:** Task 3

**Files:** Modify `terminal_view.rs`, `workspace/control.rs`, `control/mod.rs`; create `terminal_view/control.rs` if needed for locality.

**Interfaces:**
- `TerminalView::control_input(&Operation, &mut Context<Self>) -> Result<(), &'static str>` validates terminal input and uses existing session command admission.
- Generic panes advertise unsupported terminal read/input; TerminalView advertises terminal state plus hosted Surface presentation, without leaking editor/native documents.

- [x] Add public test: manage without input cannot type, input without manage cannot close; unsupported pane content/input returns `unsupported`.
- [x] Run red, then route text through `TerminalCommand::Paste`, honoring unsafe-paste confirmation; map documented keys through terminal key encoding rather than hard-coded escape guesses. Never route keys to Surface ownership/event channels.
- [x] Make unsafe input confirmation target/grant-specific and recheck at approval. Existing terminal pending-paste state must not let a later agent request confirm itself.
- [x] Verify Surface-backed panes retain CRUD and underlying terminal text while declaring native content unavailable. Commit `feat: expose separately authorized terminal input`.

## Task 5: Public workflow evidence, docs and branch review

**Blocked by:** Task 4

**Files:** Create `workspace/control_tests.rs`, `docs/pane-control.md`; modify `docs/technical-reference.md`, `README.md` and the execution ledger.

**Interfaces:** Consumes the documented client/protocol and real Workspace. Produces recorded acceptance evidence and user documentation.

- [ ] Build a test harness that runs the public client on a worker while the GPUI test executor drains Workspace messages. Approve only via the same owner action used by the UI. Assert protocol results and observable pane content/identity, without mocking a second mutation engine.
- [ ] Exercise the PRD's 12 acceptance scenarios, including human/background panes, all Surface presentations, two windows, revoked access, session-preserving move, stale IDs, close/control races and resource saturation.
- [ ] Write runnable CLI examples for grant, create/read/rename/move/focus/resize/close, per-kind capabilities, separate input, revocation, unknown outcomes and cleanup semantics.
- [ ] Run `cargo fmt --all -- --check`, `cargo test --workspace --locked --offline --no-fail-fast`, and `cargo clippy --workspace --all-targets --locked --offline -- -D warnings`. Record failures explicitly if platform/environment prevents a gate.
- [ ] Commit `test: verify and document the public pane control workflow`.
- [ ] Request an independent whole-branch review using the PRD, TSP and ledger; fix substantive findings and rerun affected checks. Keep issue open until implementation and verification are complete.

## Source chain and stress-test record

GPUI is declared as 0.2.2 and patched to `vendor/gpui`; vendored source is authoritative for the patch. Existing `workspace/lane_drain.rs`, GPUI `Window::activate_window` and `Div::on_click` are the local interfaces used for draining, activation and owner buttons. Compile and GPUI acceptance tests verify compatibility before any completion claim.

Grilling found no requirement requiring another owner decision. The approved scope settles the alternatives: separate endpoint, per-window grants, no cross-window moves, distinct input. Specific probes are assigned above for forged requester IDs, move/exit membership, revocation at confirmation, background resize and final-pane teardown. Public protocol tests must demonstrate behavior; pure policy/parser tests alone do not establish completion.

Execution: inline in the existing isolated worktree; use one independent final reviewer as required by executing-plans. No additional approval gate is needed for routine implementation details within the approved PRD.
