# Bug-Class Audit Technical Spec

> **For agentic workers:** REQUIRED SUB-SKILL: Use dmi-superpowers:subagent-driven-development (recommended) or dmi-superpowers:executing-plans to implement this TSP task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Repair all 32 findings (BCA-01–BCA-32) from the October whole-repository review, removing each recurring failure class at its owning structure, test-first.

**Architecture:** Eight structural changes remove whole bug classes: answers correlated by ticket and relayed requests claimed before applying (ADR 0028); one `Confirmation<T>` for close and paste; Pane Focus as real terminal state; no Surface socket I/O or unbounded per-message work on the GPUI thread (ADR 0029); per-row shaped-text, SVG and snapshot reuse; panes always registered; an accept loop that only cancellation ends; pane cleanup on dedicated threads. The remaining findings are local fixes, each with its own regression test.

**Tech Stack:** Rust 1.97.1 workspace (`sprite-app`, `sprite-term`, `sprite-pane`), vendored GPUI 0.2.2, libghostty-vt 0.2.2, nix 0.28.0, png 0.18.1, async-channel 2.5.0.

**PRD:** `docs/PRDs/10-09-2026-bug-class-audit.md`. **ADRs:** 0028, 0029. **Glossary:** `crates/CONTEXT.md` (Pane Focus added).

## Global Constraints

- Rust 1.97.1, pinned dependencies, no new crates, no async runtime.
- One libghostty-owning worker thread per Terminal Session (ADR 0008).
- Surface Channel wire protocol version 1, its authentication, event ordering and existing refusal semantics are unchanged on the wire.
- New threads only: one writer thread per live Surface connection (R-S1), and one short-lived thread per pane cleanup in progress (R-W6).
- Linux and macOS source compatibility.
- Work on `fix/bug-class-audit` in `.worktrees/bug-class-audit`; the original checkout stays unchanged. No release, version bump, publishing or external messages.
- Every fix is test-first: the regression test fails against `a62247e` and passes after the change. Tests cross the real failing seam. Tests labelled **GUARD** intentionally pass before and after and protect behaviour a neighbouring change could break; every task still has at least one test that fails first.

## Working in this worktree

- Build environment is ready: `vendor/ghostty` submodule is checked out at `ab0b9da9`, and `target/` was cloned from the main checkout.
- Baseline at `5e2ed14` (docs-only over `a62247e`): `TERM=dumb cargo test --workspace --locked --offline --no-fail-fast` → **838 passed, 0 failed, 3 ignored**, about 4.5 minutes including build.
- Always run cargo with `TERM=dumb … --locked --offline`.
- Never use bare `git stash`; the stash stack is shared with other worktrees.

## Execution order and shared code

Tasks run strictly in numeric order, 1 → 21. Hunks quote code as it is at `a62247e`. When an earlier task in this TSP has already changed the surrounding code, apply the same change to the current code and keep the earlier task's edits — never revert them to make a hunk match. The places where tasks meet:

| Code | Tasks (in order) | What to keep in mind |
|---|---|---|
| `sprite-term/src/worker/mod.rs` `handle` match | 1 (CaptureHistory arm), 7 (Focus), 9 (moves every arm into `Session::apply`), 11 (Select/BeginSelection) | Task 9 moves arms; Tasks 1/7 edits travel with them. |
| `TerminalView` struct and both constructors (`terminal_view.rs`) | 6, 7, 8, 16, 17 | Each adds fields; keep all. |
| `TerminalView::apply` / event task (`terminal_view.rs`) | 1 (history arms, `fail_all` on stop), 6, 17 (returns `bool`) | After Task 17 every arm returns whether something visible changed; Task 1's arms end in `true`. |
| `spawn_retry` / `begin_shutdown` admission (`terminal_view.rs`) | 7 (`pending_focus`) | Any later pending admission value joins the same retry condition. |
| `GridPaintSpec` literals, `GridPaint::prepare/draw/paint` (`grid_paint.rs`) | 8 (`focused`), 16 (`shapes`), 18 (faint/hidden in `draw`) | Task 18's hunk starts at `let fill =` and leaves Task 8's cursor lines alone. |
| `terminal_view/render.rs` `render` and left-press handler | 5/6 (disarm), 8, 11 (`BeginSelection` on press), 16, 18 | |
| `Workspace` struct and initializer (`workspace/mod.rs`) | 3, 5, 8 (clock), 19 (`font_zoom`), 20 (`cleanup_threads`) | After Task 19, any code that builds a pane or publishes settings must use `active_settings()`. |
| `workspace/close_gate.rs` | 5 (Confirmation), 20 (cleanup threads) | |
| `workspace/reload.rs` | 2 (claim), 3 (registration), 19 (zoom diff) | Task 2's GPUI test reads font size; after Task 19 read it via `font_size()`. |
| `surface/channel.rs` `serve_surface` / `one_shot`, surface test helpers | 2 (claim), 13 (writer thread, `Peer` helper) | Task 15 uses Task 13's test helpers. |
| Event-pressure test fixtures | 9 (move title floods to OSC 52) | View-spawned fixtures gain real Pane Focus through Task 7's wiring. |
| `docs/performance/design-review-paint-shared.json` | 21 only | Anything that reruns the paint bench lands before Task 21 Step 7, or Step 7 is repeated. |

---

### Task 1: History capture is answered by ticket, never by arrival order (BCA-06, R-C1.1, R-C1.2, R-C1.3)

**Files:**
- Modify: `crates/sprite-term/src/command.rs:168-177` (`CaptureHistory` variant) and insert `Ticket` before line 195 (`/// How many lines of history ...`)
- Modify: `crates/sprite-term/src/event.rs:1` (import) and `:27-28` (`History` variant)
- Modify: `crates/sprite-term/src/worker/mod.rs:564-582` (`CaptureHistory` arm)
- Modify: `crates/sprite-app/src/observation/broker.rs:53-57` (`Pending`) plus the test `FakeWindow` at `:369-443`
- Modify: `crates/sprite-app/src/observation/panes.rs:1-232` (registry) and `:234-506` (tests)
- Modify: `crates/sprite-app/src/terminal_events.rs:8-25, 50-141` (effects, `decide`) and tests `:248-262, 306-313`
- Modify: `crates/sprite-app/src/terminal_view.rs:549-562` (`apply` arms) and `:283-297` (event task stop block, second cycle)
- Modify: `crates/sprite-app/src/lib.rs:31-34` (export `Withdraw`)
- Modify: `crates/sprite-app/src/bin/sprite-observation-bench.rs:22-25, 110`
- Modify (call sites of the changed variants): `crates/sprite-term/tests/history.rs`, `crates/sprite-app/tests/graphics_observation.rs:14-16, 99-107`, `crates/sprite-app/src/terminal_view/submission_regressions.rs:263, 321-334`
- Test: `crates/sprite-app/src/terminal_view/tests.rs` (GPUI, appended at the end of the file), `crates/sprite-app/src/observation/panes.rs` (`mod tests`), `crates/sprite-app/src/observation/broker.rs` (`mod tests`), `crates/sprite-app/src/terminal_events.rs` (`mod tests`), `crates/sprite-term/tests/history.rs` (integration test file)

**Interfaces:**
- Consumes: nothing from other tasks.
- Produces:
  - `sprite_term::Ticket` — `#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)] pub struct Ticket(u64)`, with `pub const fn new(value: u64) -> Self` and `pub const fn get(self) -> u64`.
  - `sprite_term::TerminalCommand::CaptureHistory { ticket: Ticket, lines: HistoryLines }`
  - `sprite_term::TerminalEvent::History { ticket: Ticket, snapshot: Arc<HistorySnapshot> }`
  - `sprite_term::TerminalEvent::HistoryFailed { ticket: Ticket, error: SessionError }`
  - `crate::observation::broker::Withdraw` (`Default`, `Withdraw::new(impl FnOnce() + Send + 'static)`, runs on drop), exported as `sprite_app::Withdraw`; `Pending` gains `pub withdraw: Withdraw`.
  - `WindowPanes::deliver(&self, pane: PaneId, ticket: Ticket, snapshot: Arc<HistorySnapshot>)` and `WindowPanes::deliver_failure(&self, pane: PaneId, ticket: Ticket, reason: String)`.
  - `WindowPanes::fail_all(&self, pane: PaneId, reason: String)`: fails every outstanding ticket for the pane. The view calls it when its event stream stops (`Exited`, or a sealed stream after a fatal error) with `"the pane's session ended before it answered"` (second cycle).
  - `terminal_events::Effect::DeliverHistory { ticket: Ticket, snapshot: Arc<HistorySnapshot> }` and `Effect::FailRequest { ticket: Ticket, reason: String }`. `TerminalEvent::Error` now yields only `Effect::Status`.

- [ ] **Step 1a: Write the real-seam regression test first (it compiles at a62247e).**

Append to the end of `crates/sprite-app/src/terminal_view/tests.rs`. It follows `closing_an_owned_terminal_unregisters_before_retained_handles_drop` in the same file: a real `TerminalView` with a real worker, linked to a real `WindowPanes`.

```rust
/// An error that belongs to no request must not answer one.
///
/// History answers used to be paired with waiting requests by arrival order,
/// and every error failed the oldest waiter. A selection that could not be
/// resolved, made while two observers waited, therefore failed the second
/// observer with the selection's error and threw its real answer away. Each
/// request now carries a ticket, so the error reaches only the status line
/// and each observer receives the answer to its own question.
#[gpui::test]
fn an_unrelated_error_neither_fails_nor_shifts_waiting_history_requests(
    cx: &mut gpui::TestAppContext,
) {
    use crate::observation::broker::{PaneSource, Pending};
    use crate::observation::panes::{PaneLink, WindowPanes};
    use crate::pane_tree::PaneId;
    use crate::tabs::TabId;

    /// Runs the view until the request is answered, for as long as the real
    /// worker takes.
    fn answer_to(
        pending: &Pending,
        cx: &mut gpui::VisualTestContext,
    ) -> Result<Arc<sprite_term::HistorySnapshot>, String> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            cx.run_until_parked();
            match pending.answer.try_recv() {
                Ok(answer) => return answer,
                Err(std::sync::mpsc::TryRecvError::Empty)
                    if std::time::Instant::now() < deadline =>
                {
                    crate::test_blocking_wait::pause(std::time::Duration::from_millis(10));
                }
                Err(error) => panic!("no answer arrived: {error:?}"),
            }
        }
    }

    let settings = crate::config::Settings::default();
    cx.set_global(crate::config::ActiveSettings(settings.clone()));
    cx.set_global(crate::tokens::TokenRegistry::new(&settings.colors));
    let panes = WindowPanes::new();
    let (sender, _exits) = async_channel::unbounded();
    let (view, cx) = cx.add_window_view(|window, cx| {
        TerminalView::new(
            Some(vec!["/bin/sh".into(), "-c".into(), "exec sleep 30".into()]),
            settings,
            Vec::new(),
            Some(PaneLink {
                pane: PaneId(0),
                tab: TabId(0),
                panes: Arc::clone(&panes),
            }),
            PaneExit {
                sender,
                identity: (TabId(0), PaneId(0)),
            },
            window,
            cx,
        )
    });
    cx.executor().allow_parking();

    // Queued in one turn of the GPUI thread, so the view cannot forward
    // anything until all three are with the worker, which answers them in
    // order: the first capture, the selection's error, the second capture.
    let (first, second) = view.update(cx, |view, _| {
        let first = panes
            .begin(PaneId(0), sprite_term::HistoryLines::new(3))
            .expect("the first request is sent");
        // Far below a 24-row screen, so the worker cannot resolve the cell
        // and reports an error that has nothing to do with either request.
        let nowhere = sprite_term::CellPosition {
            row: 500,
            column: 0,
        };
        assert!(view.submit(sprite_term::TerminalCommand::Select {
            anchor: nowhere,
            head: nowhere,
            mode: sprite_term::SelectionMode::Character,
            rectangle: false,
        }));
        let second = panes
            .begin(PaneId(0), sprite_term::HistoryLines::new(7))
            .expect("the second request is sent");
        (first, second)
    });

    let first = answer_to(&first, cx).expect("the first request is answered with a capture");
    let second =
        answer_to(&second, cx).expect("the selection error does not fail the second request");
    assert_eq!(first.requested, 3, "the first request gets its own capture");
    assert_eq!(
        second.requested, 7,
        "each request receives the answer to its own question"
    );
    view.read_with(cx, |view, _| {
        assert!(
            view.status
                .as_ref()
                .is_some_and(|status| status.contains("selection_grid_ref")),
            "the unrelated error is still reported on the status line: {:?}",
            view.status
        );
    });
}
```

- [ ] **Step 2a: Run it and confirm it fails at a62247e**

```
TERM=dumb cargo test -p sprite-app --locked --offline --lib terminal_view::tests::an_unrelated_error_neither_fails_nor_shifts_waiting_history_requests -- --exact
```

Expected: panic `the selection error does not fail the second request: "selection_grid_ref: ..."` (the `Error` effect popped the second waiter). If instead the panic is `the unrelated error is still reported on the status line: ...` or the first `expect` fails, the out-of-range `Select` did not produce an error on this libghostty build: stop and pick another deterministic worker error before continuing (see Drafter notes).

- [ ] **Step 1b: Write the remaining failing tests (these fail to compile until Step 3).**

1. `crates/sprite-term/tests/history.rs`. Change the import block at lines 9-12 to:

```rust
use sprite_term::{
    HistoryLines, HistorySnapshot, ScreenKind, SessionConfig, TerminalCommand, TerminalEvent,
    TerminalSession, Ticket,
};
```

Replace `wait_for_history` (lines 36-49) with the following, and add the `capture` helper directly after it:

```rust
/// Waits for the answer to one history request, ignoring unrelated events.
///
/// A live shell also reports its title and working directory, so the answer is
/// rarely the very next event.
fn wait_for_history(events: &EventPump) -> HistorySnapshot {
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        match events.next() {
            TerminalEvent::History { snapshot, .. } => return (*snapshot).clone(),
            TerminalEvent::HistoryFailed { error, .. } => {
                panic!("history request failed: {error}")
            }
            TerminalEvent::Error(error) => panic!("the session reported an error: {error}"),
            _ => {}
        }
    }
    panic!("watchdog: no history answer arrived");
}

/// One history request. These tests ask one question at a time, so every
/// request can use the same ticket.
fn capture(lines: HistoryLines) -> TerminalCommand {
    TerminalCommand::CaptureHistory {
        ticket: Ticket::new(0),
        lines,
    }
}
```

Then replace every `TerminalCommand::CaptureHistory(` in the file with `capture(` (the closing parentheses already match, including the multi-line ones at lines 121, 206 and 261):

```
sed -i.bak 's/TerminalCommand::CaptureHistory(/capture(/g' crates/sprite-term/tests/history.rs && rm crates/sprite-term/tests/history.rs.bak
```

Append this test at the end of the file:

```rust
/// The ticket a request is sent with comes back on its answer, so a caller
/// can match answers to requests without relying on their order.
#[test]
fn every_answer_carries_the_ticket_it_was_asked_with() {
    let (mut session, events, _snapshots) = counting_session(20);
    for (ticket, lines) in [(41, 2), (42, 5)] {
        session
            .send(TerminalCommand::CaptureHistory {
                ticket: Ticket::new(ticket),
                lines: HistoryLines::new(lines),
            })
            .expect("request history");
    }

    let mut answered = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(20);
    while answered.len() < 2 && Instant::now() < deadline {
        match events.next() {
            TerminalEvent::History { ticket, snapshot } => {
                answered.push((ticket.get(), snapshot.requested));
            }
            TerminalEvent::HistoryFailed { ticket, error } => {
                panic!("request {} failed: {error}", ticket.get())
            }
            _ => {}
        }
    }
    assert_eq!(
        answered,
        vec![(41, 2), (42, 5)],
        "each answer names the request it answers"
    );
}
```

2. `crates/sprite-app/src/terminal_events.rs` tests. Replace `an_error_both_reports_and_fails_the_waiter` (lines 248-262) with:

```rust
    /// A general error names no request, so it can only be reported. Failing
    /// a waiter with it would hand one request the answer to nothing, and,
    /// while answers were paired by arrival order, shift every later answer
    /// onto the wrong request.
    #[test]
    fn an_error_is_reported_and_answers_no_request() {
        let raised = effects(TerminalEvent::Error(error("select", "broke")));
        assert_eq!(raised.len(), 1, "a status line and nothing else: {raised:?}");
        assert!(
            matches!(&raised[0], Effect::Status(line) if line.contains("broke")),
            "the reason is still shown"
        );
    }

    /// A capture that failed answers its own request, with the reason, rather
    /// than leaving it to wait out the observation deadline.
    #[test]
    fn a_failed_capture_fails_only_its_own_request() {
        let ticket = sprite_term::Ticket::new(9);
        assert_eq!(
            effects(TerminalEvent::HistoryFailed {
                ticket,
                error: error("capture_history", "broke"),
            }),
            vec![Effect::FailRequest {
                ticket,
                reason: "capture_history: broke".to_owned(),
            }]
        );
    }
```

Replace `a_history_answer_is_forwarded_whole` (lines 306-313) with:

```rust
    #[test]
    fn a_history_answer_is_forwarded_whole_with_its_ticket() {
        let captured = snapshot();
        let ticket = sprite_term::Ticket::new(4);
        assert!(matches!(
            effects(TerminalEvent::History {
                ticket,
                snapshot: Arc::clone(&captured),
            })
            .as_slice(),
            [Effect::DeliverHistory { ticket: delivered_ticket, snapshot: delivered }]
                if *delivered_ticket == ticket && delivered == &captured
        ));
    }
```

3. `crates/sprite-app/src/observation/broker.rs` tests. In `struct FakeWindow` (line 369) add a field after `asked`:

```rust
        /// Every pane whose request has been withdrawn, in the order it was.
        withdrawn: Arc<Mutex<Vec<PaneId>>>,
```

In `FakeWindow::new` (line 379) add `withdrawn: Arc::default(),` after `asked: Mutex::default(),`. In `FakeWindow::begin` replace the final `Ok(Pending { address, answer })` (line 442) with:

```rust
            let withdrawn = Arc::clone(&self.withdrawn);
            Ok(Pending {
                address,
                answer,
                withdraw: Withdraw::new(move || {
                    withdrawn.lock().expect("lock").push(pane);
                }),
            })
```

Append to `mod tests`:

```rust
    /// A pane that misses the deadline has its request withdrawn when the
    /// collection ends, so a source keeps nothing for an answer nobody will
    /// read. Answered requests are withdrawn the same way, harmlessly.
    #[test]
    fn every_request_is_withdrawn_once_the_collection_is_over() {
        let window = FakeWindow::new(&[(0, 0), (0, 1)]).with(1, Behaviour::Stalls);
        let report =
            collect(&query("panes snapshot --window"), &window, TEST_DEADLINE).expect("allowed");
        assert!(!report.complete, "the stalled pane timed out");

        let mut withdrawn = window.withdrawn.lock().expect("lock").clone();
        withdrawn.sort();
        assert_eq!(
            withdrawn,
            vec![PaneId(0), PaneId(1)],
            "the timed-out request is taken back, not left registered"
        );
    }
```

4. `crates/sprite-app/src/observation/panes.rs`: replace the whole `#[cfg(test)] mod tests { ... }` (lines 234-506) with the module below. Existing tests keep their names and intent; the ones that delivered "to whoever asked first" now deliver to a ticket, and `concurrent_requests_for_one_pane_are_answered_in_order` becomes `concurrent_requests_for_one_pane_each_get_their_own_answer`, answered out of order.

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::observation::endpoint::Endpoint;
    use sprite_term::{
        PaneRow, PromptKind, ScreenKind, SessionConfig, Spawned, TerminalEvent, TerminalSession,
    };
    use std::time::Duration;

    fn snapshot(text: &str) -> Arc<HistorySnapshot> {
        Arc::new(HistorySnapshot {
            generation: 1,
            size: sprite_term::ValidTerminalSize::DEFAULT,
            screen: ScreenKind::Primary,
            rows: vec![PaneRow {
                text: text.into(),
                wrapped: false,
                prompt: PromptKind::None,
            }],
            history_rows: 0,
            requested: 0,
            available: 0,
            cursor: sprite_term::CursorSnapshot {
                row: 0,
                column: 0,
                visible: true,
                blinking: false,
                style: Default::default(),
            },
            viewport: sprite_term::Viewport {
                total_rows: 24,
                offset: 0,
                visible_rows: 24,
            },
            title: None,
            working_directory: None,
            placements: Vec::new(),
            captured_at_unix_ms: 1_800_000_000_000,
            foreground: None,
        })
    }

    /// Keep both receivers alive so the worker can serve registry commands.
    fn session() -> Spawned {
        let mut spawned = TerminalSession::spawn(SessionConfig::command(
            "/bin/sh",
            vec!["-c".into(), "sleep 30".into()],
        ))
        .expect("spawn a session");
        assert!(matches!(
            spawned.events.next_blocking().expect("worker started"),
            TerminalEvent::Ready
        ));
        spawned.snapshots.next_blocking().expect("initial snapshot");
        spawned
    }

    /// The tickets a pane is still waiting on, oldest first.
    fn waiting(panes: &WindowPanes, pane: PaneId) -> Vec<Ticket> {
        let entries = panes.entries.lock().unwrap();
        let mut tickets: Vec<Ticket> = entries[&pane].waiting.keys().copied().collect();
        tickets.sort_unstable();
        tickets
    }

    /// The one ticket a pane is waiting on.
    fn only_ticket(panes: &WindowPanes, pane: PaneId) -> Ticket {
        let tickets = waiting(panes, pane);
        assert_eq!(tickets.len(), 1, "exactly one request is outstanding");
        tickets[0]
    }

    fn text(answer: Answer) -> String {
        answer.expect("a snapshot").rows[0].text.to_string()
    }

    #[test]
    fn saturated_observation_refuses_without_holding_the_ui_registry() {
        let mut spawned = TerminalSession::spawn(SessionConfig::command("/bin/sh", vec!["-c".into(), "i=0; while [ $i -lt 100 ]; do printf '\\033]2;TITLE%s\\007' $i; i=$((i+1)); done; head -c 1048576 /dev/zero; sleep 30".into()])).unwrap();
        crate::test_blocking_wait::pause(Duration::from_millis(300));
        spawned.snapshots.next_blocking().unwrap();
        let panes = WindowPanes::new();
        panes.register(PaneId(0), TabId(0), spawned.session.commands());
        let (tx, rx) = std::sync::mpsc::channel();
        let registry = Arc::clone(&panes);
        std::thread::spawn(move || {
            let result = registry
                .begin(PaneId(0), HistoryLines::default())
                .map(|_| ());
            let _ = tx.send(result);
        });
        let result = rx.recv_timeout(Duration::from_secs(1));
        spawned
            .session
            .begin_shutdown()
            .unwrap()
            .unwrap()
            .wait()
            .unwrap();
        let refusal = result
            .expect("observation submission must release the UI registry promptly")
            .expect_err("saturated queue refuses observation");
        assert!(refusal.contains("queue is full"));
        let entries = panes.entries.lock().unwrap();
        assert!(
            entries[&PaneId(0)].waiting.is_empty(),
            "a refused request leaves no waiter"
        );
    }

    #[test]
    fn a_registered_pane_is_listed_in_a_stable_order() {
        let panes = WindowPanes::new();
        let first = session();
        let second = session();
        panes.register(PaneId(5), TabId(1), first.session.commands());
        panes.register(PaneId(2), TabId(0), second.session.commands());

        let listed = panes.panes();
        let order: Vec<(u64, u64)> = listed
            .iter()
            .map(|address| (address.tab.0, address.pane.0))
            .collect();
        assert_eq!(
            order,
            vec![(0, 2), (1, 5)],
            "ordered by the window's layout, never by hash order"
        );
    }

    #[test]
    fn an_unregistered_pane_cannot_be_asked() {
        let panes = WindowPanes::new();
        let outcome = panes.begin(PaneId(1), HistoryLines::default());
        assert!(outcome.is_err(), "a pane the window does not have");
    }

    #[test]
    fn an_answer_reaches_the_caller_that_asked_for_it() {
        let panes = WindowPanes::new();
        let session = session();
        panes.register(PaneId(0), TabId(0), session.session.commands());

        let pending = panes
            .begin(PaneId(0), HistoryLines::default())
            .expect("asked");
        panes.deliver(PaneId(0), only_ticket(&panes, PaneId(0)), snapshot("answer"));

        let answer = pending
            .answer
            .recv_timeout(Duration::from_secs(1))
            .expect("an answer arrived");
        assert_eq!(text(answer), "answer");
    }

    /// A capture that fails must tell its waiter why, not leave it to time
    /// out and be reported for the wrong reason.
    #[test]
    fn a_failure_reaches_the_caller_that_asked_for_it() {
        let panes = WindowPanes::new();
        let session = session();
        panes.register(PaneId(0), TabId(0), session.session.commands());

        let pending = panes
            .begin(PaneId(0), HistoryLines::default())
            .expect("asked");
        panes.deliver_failure(
            PaneId(0),
            only_ticket(&panes, PaneId(0)),
            "the child exited".to_owned(),
        );

        let answer = pending
            .answer
            .recv_timeout(Duration::from_secs(1))
            .expect("a failure arrived");
        assert_eq!(
            answer.expect_err("the pane failed"),
            "the child exited",
            "the waiter learns the actual reason, not a timeout"
        );
    }

    /// Two callers asking one pane at once each get the answer to their own
    /// request, whatever order the answers arrive in.
    #[test]
    fn concurrent_requests_for_one_pane_each_get_their_own_answer() {
        let panes = WindowPanes::new();
        let session = session();
        panes.register(PaneId(0), TabId(0), session.session.commands());

        let first = panes
            .begin(PaneId(0), HistoryLines::default())
            .expect("asked");
        let second = panes
            .begin(PaneId(0), HistoryLines::default())
            .expect("asked");
        let tickets = waiting(&panes, PaneId(0));
        assert_eq!(tickets.len(), 2, "two requests, two tickets");

        // Newest first: arrival order must not decide who receives what.
        panes.deliver(PaneId(0), tickets[1], snapshot("second"));
        panes.deliver(PaneId(0), tickets[0], snapshot("first"));

        let answer = |pending: &Pending| {
            pending
                .answer
                .recv_timeout(Duration::from_secs(1))
                .expect("answered")
        };
        assert_eq!(text(answer(&first)), "first");
        assert_eq!(text(answer(&second)), "second");
    }

    /// A failure names the request it belongs to and reaches no other.
    #[test]
    fn a_failure_reaches_only_the_request_it_belongs_to() {
        let panes = WindowPanes::new();
        let session = session();
        panes.register(PaneId(0), TabId(0), session.session.commands());

        let first = panes
            .begin(PaneId(0), HistoryLines::default())
            .expect("asked");
        let second = panes
            .begin(PaneId(0), HistoryLines::default())
            .expect("asked");
        let tickets = waiting(&panes, PaneId(0));

        panes.deliver_failure(PaneId(0), tickets[1], "capture failed".to_owned());

        assert!(
            first.answer.try_recv().is_err(),
            "the other request is still waiting for its own answer"
        );
        assert_eq!(
            second
                .answer
                .recv_timeout(Duration::from_secs(1))
                .expect("answered")
                .expect_err("failed"),
            "capture failed"
        );
        assert_eq!(waiting(&panes, PaneId(0)), vec![tickets[0]]);
    }

    /// A caller that stops waiting, as one does when its deadline passes,
    /// takes its ticket back: nothing is kept for a request nobody will read.
    #[test]
    fn a_request_that_stops_waiting_takes_its_ticket_back() {
        let panes = WindowPanes::new();
        let session = session();
        panes.register(PaneId(0), TabId(0), session.session.commands());

        let pending = panes
            .begin(PaneId(0), HistoryLines::default())
            .expect("asked");
        assert_eq!(waiting(&panes, PaneId(0)).len(), 1);

        drop(pending);

        assert!(
            waiting(&panes, PaneId(0)).is_empty(),
            "the request withdrew its own ticket"
        );
    }

    /// The answer to a request that gave up must not be handed to the request
    /// that came after it.
    #[test]
    fn an_answer_for_an_expired_ticket_reaches_nobody() {
        let panes = WindowPanes::new();
        let session = session();
        panes.register(PaneId(0), TabId(0), session.session.commands());

        let expired = panes
            .begin(PaneId(0), HistoryLines::default())
            .expect("asked");
        let late = only_ticket(&panes, PaneId(0));
        drop(expired);
        let current = panes
            .begin(PaneId(0), HistoryLines::default())
            .expect("asked again");

        // The worker's answer to the request that gave up arrives now.
        panes.deliver(PaneId(0), late, snapshot("late"));
        assert!(
            current.answer.try_recv().is_err(),
            "a late answer is dropped, not given to the next request"
        );

        panes.deliver(PaneId(0), only_ticket(&panes, PaneId(0)), snapshot("current"));
        assert_eq!(
            text(
                current
                    .answer
                    .recv_timeout(Duration::from_secs(1))
                    .expect("its own answer")
            ),
            "current"
        );
    }

    /// A pane that closes releases its waiters immediately. Making them wait
    /// out the deadline would report "did not answer in time" for something
    /// already known to be gone.
    #[test]
    fn forgetting_a_pane_releases_whoever_was_waiting() {
        let panes = WindowPanes::new();
        let session = session();
        panes.register(PaneId(0), TabId(0), session.session.commands());
        let pending = panes
            .begin(PaneId(0), HistoryLines::default())
            .expect("asked");

        panes.forget(PaneId(0));

        let answer = pending
            .answer
            .recv_timeout(Duration::from_secs(1))
            .expect("released rather than left waiting");
        assert!(answer.unwrap_err().contains("closed"));
        assert!(panes.panes().is_empty());
    }

    /// Turning observation off must not disturb a session that is running. The
    /// pane keeps its child and its output; it simply becomes unreachable.
    #[test]
    fn a_session_keeps_running_when_observation_is_switched_off() {
        let panes = WindowPanes::new();
        let mut session = session();
        panes.register(PaneId(0), TabId(0), session.session.commands());

        // What switching observation off does to a window: the endpoint is
        // destroyed. Nothing here touches the session.
        let directory = std::env::temp_dir().join(format!("sprite-panes-{}", std::process::id()));
        drop(Endpoint::open_in(directory.clone(), |_| String::new()).expect("an endpoint"));
        let _ = std::fs::remove_dir_all(&directory);

        // The child is still there, and the session still takes commands.
        assert!(
            session
                .session
                .send(TerminalCommand::Resize(
                    sprite_term::ValidTerminalSize::DEFAULT
                ))
                .is_ok(),
            "the session is alive and accepting commands"
        );
        session
            .session
            .send(TerminalCommand::CaptureHistory {
                ticket: Ticket::new(0),
                lines: HistoryLines::default(),
            })
            .expect("request a fresh answer after disabling observation");
        assert!(matches!(
            session
                .events
                .next_blocking()
                .expect("worker still answers"),
            TerminalEvent::History { .. }
        ));
        assert_eq!(panes.panes().len(), 1, "and the pane is still a pane");
    }

    #[test]
    fn an_answer_nobody_is_waiting_for_is_discarded() {
        let panes = WindowPanes::new();
        let session = session();
        panes.register(PaneId(0), TabId(0), session.session.commands());

        // No request outstanding: this must not panic, grow a map, or be
        // handed to the next caller as a stale answer.
        panes.deliver(PaneId(0), Ticket::new(u64::MAX), snapshot("stale"));

        let pending = panes
            .begin(PaneId(0), HistoryLines::default())
            .expect("asked");
        assert!(
            pending.answer.try_recv().is_err(),
            "a later request does not receive an earlier abandoned answer"
        );
    }
}
```

- [ ] **Step 2b: Confirm the new tests fail to compile**

```
TERM=dumb cargo test -p sprite-term --locked --offline --test history every_answer_carries_the_ticket_it_was_asked_with
```
Expected: `error[E0432]: unresolved import sprite_term::Ticket` and `no variant named HistoryFailed`.

```
TERM=dumb cargo test -p sprite-app --locked --offline --lib observation::
```
Expected: compile errors naming `Ticket`, `Withdraw`, `HistoryFailed`, `Effect::FailRequest { .. }` (struct variant) and `keys` on `VecDeque`.

- [ ] **Step 3: Implement**

1. `crates/sprite-term/src/command.rs`. Replace lines 168-177 (the doc comment and `CaptureHistory(HistoryLines),`) with:

```rust
    /// Ask for the active screen plus up to `lines` of history, answered once
    /// with [`crate::TerminalEvent::History`] or
    /// [`crate::TerminalEvent::HistoryFailed`], either of which carries this
    /// `ticket`.
    ///
    /// Deliberately not part of the render bundle. Snapshots carry no history
    /// because rebuilding a full scrollback on every capture would cost
    /// thousands of allocations a second for rows the renderer never draws;
    /// observation has the opposite need, so it asks separately and pays only
    /// when it asks.
    CaptureHistory { ticket: Ticket, lines: HistoryLines },
```

Insert directly above `/// How many lines of history an observation request wants.` (line 195):

```rust
/// Names one request so that its answer can find whoever asked.
///
/// Chosen by the asker and echoed back unchanged. Terminal Core attaches no
/// meaning to the value; it promises only that the answer to a request carries
/// that request's ticket, so an asker never has to rely on the order answers
/// arrive in, which any unrelated event emitted in between would shift.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Ticket(u64);

impl Ticket {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

```

2. `crates/sprite-term/src/event.rs`. Line 1 becomes:

```rust
use crate::{CellPosition, GraphicsSnapshot, HistorySnapshot, Ticket};
```

Replace lines 27-28 (`/// The answer to one ...CaptureHistory` and `History(Arc<HistorySnapshot>),`) with:

```rust
    /// The answer to one [`crate::TerminalCommand::CaptureHistory`], carrying
    /// the ticket it was asked with.
    History {
        ticket: Ticket,
        snapshot: Arc<HistorySnapshot>,
    },
    /// A [`crate::TerminalCommand::CaptureHistory`] that could not be
    /// answered, carrying the ticket it was asked with.
    ///
    /// Separate from [`TerminalEvent::Error`] so that a failed capture reaches
    /// only the request it belongs to: a general error says that something
    /// went wrong, never whose request it was.
    HistoryFailed { ticket: Ticket, error: SessionError },
```

3. `crates/sprite-term/src/worker/mod.rs`. Replace lines 564-582 (the `TerminalCommand::CaptureHistory(lines) => { ... }` arm) with:

```rust
                TerminalCommand::CaptureHistory { ticket, lines } => {
                    // Answered once, from this thread, against the same
                    // terminal the snapshots come from — so the rows returned
                    // belong to one generation rather than a moving target.
                    // Both outcomes carry the ticket, so the answer can only
                    // reach the request that asked.
                    let foreground = foreground_executable(master.as_ref());
                    match projector.capture_history(
                        pending.generation,
                        *size,
                        lines.get(),
                        foreground,
                        terminal,
                    ) {
                        Ok(history) => {
                            emit(
                                events,
                                TerminalEvent::History {
                                    ticket,
                                    snapshot: Arc::new(history),
                                },
                            )?;
                        }
                        Err(error) => {
                            emit(events, TerminalEvent::HistoryFailed { ticket, error })?;
                        }
                    }
                }
```

4. `crates/sprite-app/src/observation/broker.rs`. Replace lines 53-57 (`/// A capture that has been asked for ...` through the closing `}` of `Pending`) with:

```rust
/// A capture that has been asked for and not yet answered.
pub struct Pending {
    pub address: PaneAddress,
    pub answer: Receiver<Result<Arc<HistorySnapshot>, String>>,
    /// Runs when this is dropped, so a request whose caller has stopped
    /// waiting, answered or not, leaves nothing behind in its source.
    pub withdraw: Withdraw,
}

/// Takes a request back when its caller stops waiting for it.
///
/// Runs once, when the [`Pending`] it belongs to is dropped: after an answer,
/// after a failure, or when the request's deadline has passed. A source that
/// keeps nothing per request leaves it empty.
#[derive(Default)]
pub struct Withdraw(Option<Box<dyn FnOnce() + Send>>);

impl Withdraw {
    pub fn new(withdraw: impl FnOnce() + Send + 'static) -> Self {
        Self(Some(Box::new(withdraw)))
    }
}

impl Drop for Withdraw {
    fn drop(&mut self) {
        if let Some(withdraw) = self.0.take() {
            withdraw();
        }
    }
}
```

5. `crates/sprite-app/src/lib.rs` lines 31-34 become:

```rust
pub use observation::broker::{
    Failure, FailureKind, PaneAddress, PaneReport, PaneSource, Pending, Report, Withdraw,
    collect as collect_panes, parse as parse_request,
};
```

6. `crates/sprite-app/src/bin/sprite-observation-bench.rs`: add `Withdraw` to the `use sprite_app::{...}` list at lines 22-25 (after `TabId`), and change line 110 to:

```rust
        Ok(Pending {
            address,
            answer,
            withdraw: Withdraw::default(),
        })
```

7. `crates/sprite-app/src/observation/panes.rs`. Replace lines 9-232 (from `use std::collections::{HashMap, VecDeque};` to the end of `impl PaneSource for WindowPanes`) with:

```rust
use std::collections::HashMap;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};

use sprite_term::{CommandSender, HistoryLines, HistorySnapshot, TerminalCommand, Ticket};

use crate::observation::broker::{PaneAddress, PaneSource, Pending, Withdraw};

/// What a pane sends back when it answers.
type Answer = Result<Arc<HistorySnapshot>, String>;
use crate::pane_tree::{PaneId, Rect};
use crate::tabs::TabId;

/// Every registered pane, shared with each outstanding request so that the
/// request can withdraw itself.
type Entries = Arc<Mutex<HashMap<PaneId, Entry>>>;

/// What a pane needs in order to be observable.
///
/// Held by the pane's view, which is the single consumer of its session's
/// events and therefore the only thing able to forward an answer.
#[derive(Clone)]
pub struct PaneLink {
    pub pane: PaneId,
    pub tab: TabId,
    pub panes: Arc<WindowPanes>,
}

/// One registered pane.
struct Entry {
    tab: TabId,
    /// Where the pane sits, refreshed from the window as the layout changes.
    ///
    /// Held here rather than asked for at request time because the layout lives
    /// on the GPUI thread, and a request must never have to wait for a frame.
    placement: Placement,
    commands: CommandSender,
    /// Requests submitted and not yet answered, by the ticket each was sent
    /// with.
    ///
    /// Keyed rather than queued: an answer carries the ticket of the request
    /// it answers, so it can reach only that request. Arrival order, which
    /// any unrelated event emitted in between would shift, plays no part.
    waiting: HashMap<Ticket, std::sync::mpsc::Sender<Answer>>,
}

/// Where a pane sits in the window, as the schema reports it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub tab_order: usize,
    pub rect: Rect,
    pub focused: bool,
}

impl Default for Placement {
    fn default() -> Self {
        Self {
            tab_order: 0,
            rect: Rect::FULL,
            focused: false,
        }
    }
}

/// Every pane in one window that observation may reach.
#[derive(Default)]
pub struct WindowPanes {
    /// Shared with each outstanding request's [`Withdraw`], so a caller that
    /// stops waiting can take its own ticket back.
    entries: Entries,
    /// The next ticket to hand out. One counter for the whole window, so a
    /// ticket names one request whichever pane it went to.
    next_ticket: AtomicU64,
    #[cfg(test)]
    layout_publications: std::sync::atomic::AtomicUsize,
}

impl WindowPanes {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Records a pane, so requests can reach it.
    pub fn register(&self, pane: PaneId, tab: TabId, commands: CommandSender) {
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        entries.insert(
            pane,
            Entry {
                tab,
                // Corrected by the next layout the window publishes; a pane
                // that has not been laid out yet still has to be answerable.
                placement: Placement::default(),
                commands,
                waiting: HashMap::new(),
            },
        );
    }

    #[cfg(test)]
    pub(crate) fn layout_publications(&self) -> usize {
        self.layout_publications
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Records where the window's panes currently sit.
    ///
    /// Published by the window as the layout changes. Panes the window no
    /// longer has are ignored rather than added back: this reports placement,
    /// not membership.
    pub fn set_layout(&self, placements: &[(PaneId, Placement)]) {
        #[cfg(test)]
        self.layout_publications
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        for (pane, placement) in placements {
            if let Some(entry) = entries.get_mut(pane) {
                entry.placement = *placement;
            }
        }
    }

    /// Forgets a pane that has closed.
    ///
    /// Anyone still waiting on it is released rather than left to time out: the
    /// pane is known to be gone, so making a caller wait out the deadline for
    /// it would be a lie about what is happening.
    pub fn forget(&self, pane: PaneId) {
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(entry) = entries.remove(&pane) {
            for waiter in entry.waiting.into_values() {
                let _ = waiter.send(Err("the pane closed before it answered".to_owned()));
            }
        }
    }

    /// Hands one pane's answer to the request holding `ticket`.
    ///
    /// Called from the view, which is the single consumer of a session's
    /// events. An answer whose ticket nobody holds is dropped: its request has
    /// already given up, and handing it to anyone else would answer a question
    /// they did not ask.
    pub fn deliver(&self, pane: PaneId, ticket: Ticket, snapshot: Arc<HistorySnapshot>) {
        self.answer(pane, ticket, Ok(snapshot));
    }

    /// Reports that one request's capture failed, to that request only.
    pub fn deliver_failure(&self, pane: PaneId, ticket: Ticket, reason: String) {
        self.answer(pane, ticket, Err(reason));
    }

    fn answer(&self, pane: PaneId, ticket: Ticket, answer: Answer) {
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(entry) = entries.get_mut(&pane)
            && let Some(waiter) = entry.waiting.remove(&ticket)
        {
            let _ = waiter.send(answer);
        }
    }
}

impl PaneSource for WindowPanes {
    fn panes(&self) -> Vec<PaneAddress> {
        let entries = self
            .entries
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let mut addresses: Vec<PaneAddress> = entries
            .iter()
            .map(|(pane, entry)| PaneAddress {
                tab: entry.tab,
                tab_order: entry.placement.tab_order,
                pane: *pane,
                rect: entry.placement.rect,
                focused: entry.placement.focused,
            })
            .collect();
        // A map has no order, and a caller must not see panes shuffle between
        // requests. This is the schema's order: tabs by window order, then
        // panes by top edge, then left edge, then identity.
        addresses.sort_by(|left, right| {
            left.tab_order
                .cmp(&right.tab_order)
                .then(left.rect.y.total_cmp(&right.rect.y))
                .then(left.rect.x.total_cmp(&right.rect.x))
                .then(left.pane.cmp(&right.pane))
        });
        addresses
    }

    fn begin(&self, pane: PaneId, lines: HistoryLines) -> Result<Pending, String> {
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let entry = entries
            .get_mut(&pane)
            .ok_or_else(|| "the pane closed before it could be asked".to_owned())?;

        let ticket = Ticket::new(
            self.next_ticket
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        );
        let (sender, answer) = std::sync::mpsc::channel();
        // Recorded before the command is sent, so an answer cannot arrive
        // before there is anyone recorded to receive it.
        entry.waiting.insert(ticket, sender);
        let address = PaneAddress {
            tab: entry.tab,
            tab_order: entry.placement.tab_order,
            pane,
            rect: entry.placement.rect,
            focused: entry.placement.focused,
        };
        if let Err(error) = entry
            .commands
            .try_send(TerminalCommand::CaptureHistory { ticket, lines })
        {
            entry.waiting.remove(&ticket);
            return Err(error.to_string());
        }
        // A caller that stops waiting takes its ticket back, so the registry
        // holds nothing for a request nobody will read. Weak, so an
        // outstanding request does not keep a closed window's registry alive.
        let registry = Arc::downgrade(&self.entries);
        Ok(Pending {
            address,
            answer,
            withdraw: Withdraw::new(move || {
                if let Some(entries) = registry.upgrade() {
                    let mut entries = entries.lock().unwrap_or_else(|error| error.into_inner());
                    if let Some(entry) = entries.get_mut(&pane) {
                        entry.waiting.remove(&ticket);
                    }
                }
            }),
        })
    }
}
```

8. `crates/sprite-app/src/terminal_events.rs`. Line 6 becomes `use sprite_term::{HistorySnapshot, SessionError, TerminalEvent, Ticket};`. Replace lines 23-24 (`DeliverHistory(...)`, `FailRequest(String),`) with:

```rust
    /// One request's capture, for the request holding `ticket`.
    DeliverHistory {
        ticket: Ticket,
        snapshot: Arc<HistorySnapshot>,
    },
    /// One request's capture failed, and why.
    FailRequest { ticket: Ticket, reason: String },
```

In the doc comment of `decide` (line 54) change `all thirteen arms` to `all fourteen arms`. Replace lines 101-106 (the `History` arm and its comment) with:

```rust
        // Belongs to whoever holds the ticket. The view forwards because it is
        // the single consumer of this session's events; the ticket, not the
        // order answers arrive in, decides who receives it.
        Ok(TerminalEvent::History { ticket, snapshot }) => {
            effects.push(Effect::DeliverHistory { ticket, snapshot });
        }

        // A capture that failed answers its own request with the reason,
        // rather than leaving it to wait out the observation deadline.
        Ok(TerminalEvent::HistoryFailed { ticket, error }) => {
            effects.push(Effect::FailRequest {
                ticket,
                reason: error.to_string(),
            });
        }
```

Replace lines 116-121 (the `Error` arm) with:

```rust
        // Reported, and nothing more. A general error names no request, so it
        // cannot answer one: a selection or key failure that arrives while an
        // observer waits must leave that observer waiting for its own answer.
        Ok(TerminalEvent::Error(error)) => {
            effects.push(Effect::Status(error.to_string().into()));
        }
```

9. `crates/sprite-app/src/terminal_view.rs`. Replace lines 551-561 (the `Effect::DeliverHistory` and `Effect::FailRequest` arms) with:

```rust
            Effect::DeliverHistory { ticket, snapshot } => {
                if let Some(link) = &self.observation {
                    link.panes.deliver(link.pane, ticket, snapshot);
                }
            }
            // A capture that failed answers the one request it belongs to,
            // with the reason, rather than leaving it to wait out the deadline.
            Effect::FailRequest { ticket, reason } => {
                if let Some(link) = &self.observation {
                    link.panes.deliver_failure(link.pane, ticket, reason);
                }
            }
```

10. Remaining call sites of the changed variants:
- `crates/sprite-app/tests/graphics_observation.rs`: add `Ticket` to the `use sprite_term::{...}` list at lines 14-16. Replace lines 98-107 with:

```rust
        session
            .send(TerminalCommand::CaptureHistory {
                ticket: Ticket::new(0),
                lines: HistoryLines::new(500),
            })
            .expect("request history");
        while let Ok(event) = events.next_blocking() {
            match event {
                TerminalEvent::History {
                    snapshot: history, ..
                } if !history.placements.is_empty() => {
                    snapshot = Some((*history).clone());
                    break;
                }
                TerminalEvent::History { .. } => break,
                _ => {}
            }
        }
```

- `crates/sprite-app/src/terminal_view/submission_regressions.rs` line 263: `if let sprite_term::TerminalEvent::History { snapshot: history, .. } = event`. Lines 323 and 333: `TerminalCommand::CaptureHistory { ticket: sprite_term::Ticket::new(0), lines: sprite_term::HistoryLines::new(0) },` (let `cargo fmt` lay it out).

Then `cargo fmt --all`.

- [ ] **Step 4: Run tests, confirm pass**

```
TERM=dumb cargo test -p sprite-term --locked --offline --test history
TERM=dumb cargo test -p sprite-app --locked --offline --lib observation:: terminal_events:: terminal_view::
TERM=dumb cargo test -p sprite-app --locked --offline --test graphics_observation
TERM=dumb cargo build -p sprite-app --locked --offline --bins
```

All pass, including `an_unrelated_error_neither_fails_nor_shifts_waiting_history_requests`, `every_answer_carries_the_ticket_it_was_asked_with`, `an_answer_for_an_expired_ticket_reaches_nobody`, `a_request_that_stops_waiting_takes_its_ticket_back`, `every_request_is_withdrawn_once_the_collection_is_over`. (If `cargo test` refuses more than one filter, run the three `--lib` filters separately.)

- [ ] **Step 5: Commit**

```
git add crates/sprite-term/src/command.rs crates/sprite-term/src/event.rs crates/sprite-term/src/worker/mod.rs crates/sprite-term/tests/history.rs crates/sprite-app/src/observation/broker.rs crates/sprite-app/src/observation/panes.rs crates/sprite-app/src/terminal_events.rs crates/sprite-app/src/terminal_view.rs crates/sprite-app/src/terminal_view/tests.rs crates/sprite-app/src/terminal_view/submission_regressions.rs crates/sprite-app/src/lib.rs crates/sprite-app/src/bin/sprite-observation-bench.rs crates/sprite-app/tests/graphics_observation.rs
git commit -m "fix(observation): answer history captures by ticket, not arrival order

CaptureHistory carries a Ticket that History and HistoryFailed echo back.
WindowPanes keys waiters by ticket, drops answers for unknown or expired
tickets, and a request that stops waiting withdraws its own ticket. A
general TerminalEvent::Error now only updates the status line, so an
unrelated selection or input error can no longer fail or shift a waiting
observation request. (BCA-06)

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

#### Second cycle: an ended session fails its waiting captures at once

- [ ] **Step 6: Write the failing tests**

1. Append to `crates/sprite-app/src/terminal_view/tests.rs` (after the test from Step 1a). A second, idle session stands in for the pane's command queue, so the capture is accepted but its answer can never come back through this view. Only the view's own session ending can resolve the request. The test compiles against Step 3's code.

```rust
/// A capture waiting on a session that has ended fails at once, with the
/// reason, instead of sitting out the observation deadline: an ended session
/// answers nothing more.
#[gpui::test]
fn a_capture_waiting_on_a_session_that_ends_fails_at_once(cx: &mut gpui::TestAppContext) {
    use crate::observation::broker::PaneSource;
    use crate::observation::panes::{PaneLink, WindowPanes};
    use crate::pane_tree::PaneId;
    use crate::tabs::TabId;
    let settings = crate::config::Settings::default();
    cx.set_global(crate::config::ActiveSettings(settings.clone()));
    cx.set_global(crate::tokens::TokenRegistry::new(&settings.colors));
    let panes = WindowPanes::new();
    let (sender, _exits) = async_channel::unbounded();
    let (_view, cx) = cx.add_window_view(|window, cx| {
        TerminalView::new(
            Some(vec!["/bin/sh".into(), "-c".into(), "sleep 1".into()]),
            settings,
            Vec::new(),
            Some(PaneLink {
                pane: PaneId(0),
                tab: TabId(0),
                panes: Arc::clone(&panes),
            }),
            PaneExit {
                sender,
                identity: (TabId(0), PaneId(0)),
            },
            window,
            cx,
        )
    });
    cx.executor().allow_parking();

    // Its events are never read, so whatever it answers never reaches the
    // registry; the request can only be resolved by the pane's own session
    // ending.
    let stand_in = TerminalSession::spawn(SessionConfig::command(
        "/bin/sh",
        vec!["-c".into(), "exec sleep 30".into()],
    ))
    .expect("spawn the stand-in session");
    panes.register(PaneId(0), TabId(0), stand_in.session.commands());
    let pending = panes
        .begin(PaneId(0), sprite_term::HistoryLines::default())
        .expect("asked");

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let reason = loop {
        cx.run_until_parked();
        match pending.answer.try_recv() {
            Ok(answer) => break answer.expect_err("an ended session cannot answer"),
            Err(std::sync::mpsc::TryRecvError::Empty) if std::time::Instant::now() < deadline => {
                crate::test_blocking_wait::pause(std::time::Duration::from_millis(10));
            }
            Err(error) => panic!("the waiter was never released: {error:?}"),
        }
    };
    assert!(
        reason.contains("session ended"),
        "the waiter is told why: {reason}"
    );
    drop(stand_in);
}
```

2. Append to `mod tests` in `crates/sprite-app/src/observation/panes.rs`:

```rust
    /// Every request waiting on a pane whose session ended is failed with the
    /// reason, at once. The pane itself stays listed: it is still on screen.
    #[test]
    fn a_session_that_ends_fails_every_waiting_request_at_once() {
        let panes = WindowPanes::new();
        let session = session();
        panes.register(PaneId(0), TabId(0), session.session.commands());
        let first = panes
            .begin(PaneId(0), HistoryLines::default())
            .expect("asked");
        let second = panes
            .begin(PaneId(0), HistoryLines::default())
            .expect("asked");

        panes.fail_all(
            PaneId(0),
            "the pane's session ended before it answered".to_owned(),
        );

        for pending in [first, second] {
            assert_eq!(
                pending
                    .answer
                    .try_recv()
                    .expect("released at once")
                    .expect_err("failed"),
                "the pane's session ended before it answered"
            );
        }
        assert!(waiting(&panes, PaneId(0)).is_empty());
        assert_eq!(panes.panes().len(), 1, "the pane is still listed");
    }
```

- [ ] **Step 7: Run them and confirm they fail**

```
TERM=dumb cargo test -p sprite-app --locked --offline --lib terminal_view::tests::a_capture_waiting_on_a_session_that_ends_fails_at_once -- --exact
```

Run this before adding the `panes.rs` test from item 2. Expected: the GPUI test panics `the waiter was never released: Empty` after 10 s. Then add the `panes.rs` test and run `TERM=dumb cargo test -p sprite-app --locked --offline --lib observation::panes::tests`. Expected: compile error `no method named fail_all found for ... WindowPanes`.

- [ ] **Step 8: Implement**

1. `crates/sprite-app/src/observation/panes.rs`: in `impl WindowPanes`, directly after `forget`, add:

```rust
    /// Fails every request still waiting on a pane whose session has ended.
    ///
    /// An ended session answers nothing more, so its waiters learn why now
    /// rather than at the deadline. The pane stays listed, because it is still
    /// on screen; asking it again is refused when the request cannot be sent.
    pub fn fail_all(&self, pane: PaneId, reason: String) {
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(entry) = entries.get_mut(&pane) {
            for (_, waiter) in entry.waiting.drain() {
                let _ = waiter.send(Err(reason.clone()));
            }
        }
    }
```

2. `crates/sprite-app/src/terminal_view.rs`: in the event task, replace the `if decision.stop { ... }` block at lines 283-297 with the block below. Both ways a session ends reach it: `Exited`, and a sealed stream after a fatal error (`decide(Err(_))`).

```rust
                if decision.stop {
                    let _ = view.update(cx, |view, cx| {
                        view.session = match std::mem::replace(
                            &mut view.session,
                            SessionState::NeverStarted,
                        ) {
                            SessionState::Running(session) | SessionState::Ended(session) => {
                                SessionState::Ended(session)
                            }
                            SessionState::NeverStarted => SessionState::NeverStarted,
                        };
                        // An ended session answers nothing more, so a capture
                        // still waiting on it fails now, with the reason,
                        // rather than at the observation deadline.
                        if let Some(link) = &view.observation {
                            link.panes.fail_all(
                                link.pane,
                                "the pane's session ended before it answered".to_owned(),
                            );
                        }
                        let _ = view.retry_wake.force_send(());
                        view.refresh_display_title(cx);
                    });
                }
```

- [ ] **Step 9: Run tests, confirm pass**

```
TERM=dumb cargo test -p sprite-app --locked --offline --lib observation::panes::tests
TERM=dumb cargo test -p sprite-app --locked --offline --lib terminal_view::tests
```

- [ ] **Step 10: Commit**

```
git add crates/sprite-app/src/observation/panes.rs crates/sprite-app/src/terminal_view.rs crates/sprite-app/src/terminal_view/tests.rs
git commit -m "fix(observation): fail waiting captures as soon as a pane's session ends

A general error no longer resolves waiters, so captures waiting on a session
that ended (by exit or a fatal error) sat out the observation deadline.
When the view's event stream stops, every outstanding ticket for that pane
now fails at once with the reason. (BCA-06)

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 2: A relayed request is claimed before it is applied, or abandoned and never applied (BCA-08, R-C2.1, R-C2.2, ADR 0028)

**Files:**
- Modify: `crates/sprite-app/src/workspace/reload.rs:104-133` (`ReloadRequest`, `RelayError`, `relay`; new `Claim`, `Relayed`, `Patience`, `AFTER_CLAIM`)
- Modify: `crates/sprite-app/src/workspace/mod.rs:22` (re-exports) and `:239-255` (reload task)
- Modify: `crates/sprite-app/src/observation/request.rs:23, 211-212, 243, 309-327` (`ask_window`)
- Modify: `crates/sprite-app/src/surface/channel.rs:19, 52-54, 399-400, 480-497, 653-683, 730-746` (relay callers, reply aliases, `SurfaceRequest::claim`)
- Modify: `crates/sprite-app/src/workspace/surface_routing.rs:5-14` (claim before serving)
- Modify (requests built by hand in tests): `crates/sprite-app/src/workspace/close_gate.rs:578`, `crates/sprite-app/src/workspace/surface_routing.rs:120, 177, 199, 260, 273, 287`, `crates/sprite-app/src/terminal_view/surfaces.rs:1752, 2187, 2268, 2424, 2483`
- Test: `crates/sprite-app/src/workspace/reload.rs` (`mod tests`, GPUI + plain), `crates/sprite-app/src/observation/request.rs` (`mod tests`), `crates/sprite-app/src/workspace/surface_routing.rs` (`mod tests`, GPUI)

**Interfaces:**
- Consumes: nothing from Task 1.
- Produces (all in `crate::workspace`, re-exported from `reload`):
  - `pub struct Relayed<Answer>` with `pub(crate) fn waiting(reply: SyncSender<Answer>) -> (Self, Claim)`, `pub(crate) fn claim(&self) -> bool` (Waiting → Claimed; idempotent), `pub fn send(&self, answer: Answer) -> Result<(), SendError<Answer>>`, `impl From<SyncSender<Answer>>` (a fresh waiting claim), manual `Debug`.
  - `pub(crate) struct Claim` with `pub(crate) fn abandon(&self) -> bool` (Waiting → Abandoned).
  - `pub(crate) struct Patience { pub(crate) answer: Duration, pub(crate) after_claim: Duration }`, `pub(crate) const fn Patience::new(answer: Duration) -> Self` (after_claim = `AFTER_CLAIM` = 10 s).
  - `pub(crate) fn relay<Request, Answer>(sender: &async_channel::Sender<Request>, patience: Patience, request: impl FnOnce(Relayed<Answer>) -> Request) -> Result<Answer, RelayError>`
  - `#[derive(Debug)] pub(crate) enum RelayError { Disconnected, Timeout, Applying }`
  - `ReloadRequest.reply: Relayed<String>`; `surface::channel::Reply = Relayed<Result<(), Refusal>>`, `JsonReply = Relayed<Result<Value, Refusal>>`; `SurfaceRequest::claim(&self) -> bool`.

- [ ] **Step 1: Write the failing tests**

1. `crates/sprite-app/src/workspace/reload.rs`, inside `mod tests` (after `use super::*;`, line 138), append these four tests at the end of the module:

```rust
    /// A reload the endpoint has given up on was reported as "nothing was
    /// changed", so the window must never apply it afterwards.
    ///
    /// The GPUI thread is not run while the endpoint waits, which is exactly
    /// what a busy window looks like from an endpoint thread; the window then
    /// reaches the queued request only after the endpoint has answered.
    #[gpui::test]
    fn a_reload_the_endpoint_gave_up_on_is_never_applied(cx: &mut gpui::TestAppContext) {
        let (workspace, cx) = test_workspace(cx);
        let path = std::env::temp_dir().join(format!(
            "sprite-abandoned-reload-{}.toml",
            std::process::id()
        ));
        std::fs::write(&path, "[font]\nsize = 21.0\n").unwrap();
        let (sender, size_before) = workspace.update(cx, |workspace, _| {
            workspace.config_path = Some(path.clone());
            (
                workspace.reload_sender.clone(),
                workspace.settings.font.size,
            )
        });
        assert_ne!(size_before, 21.0, "the file must ask for a change");

        let endpoint = std::thread::spawn(move || {
            relay(
                &sender,
                Patience {
                    answer: std::time::Duration::from_millis(50),
                    after_claim: std::time::Duration::from_secs(5),
                },
                |reply| ReloadRequest {
                    what: ConfigVerb::Reload,
                    reply,
                    reply_connection: None,
                },
            )
        });
        let outcome = endpoint.join().unwrap();
        assert!(
            matches!(outcome, Err(RelayError::Timeout)),
            "the endpoint gave up and may say nothing changed: {outcome:?}"
        );

        cx.run_until_parked();
        workspace.read_with(cx, |workspace, _| {
            assert!(
                workspace.reload_sender.is_empty(),
                "the window took the request off its queue"
            );
            assert_eq!(
                workspace.settings.font.size, size_before,
                "an abandoned reload must never be applied"
            );
        });
        std::fs::remove_file(&path).unwrap();
    }

    /// Once the window has claimed a request, its real answer is what the
    /// asker reports, even when it arrives after the first wait.
    ///
    /// Claimed by the asker's own closure, before the request is sent, so the
    /// claim is certain to precede the asker's attempt to abandon it.
    #[test]
    fn a_claimed_request_returns_the_real_answer_after_the_first_wait() {
        let (sender, requests) = async_channel::bounded::<Relayed<String>>(1);
        let window = std::thread::spawn(move || {
            let reply = requests.recv_blocking().expect("a request");
            crate::test_blocking_wait::pause(std::time::Duration::from_millis(200));
            reply
                .send("applied".to_owned())
                .expect("the asker is still listening");
        });
        let started = std::time::Instant::now();
        let answer = relay(
            &sender,
            Patience {
                answer: std::time::Duration::from_millis(40),
                after_claim: std::time::Duration::from_secs(5),
            },
            |reply: Relayed<String>| {
                assert!(reply.claim(), "nobody has given up yet");
                reply
            },
        );
        window.join().unwrap();
        assert_eq!(answer.expect("the window's real answer"), "applied");
        assert!(started.elapsed() >= std::time::Duration::from_millis(200));
    }

    /// The wait after a claim is bounded, and what it ends with never says
    /// nothing was changed.
    #[test]
    fn a_claimed_request_that_is_not_answered_in_time_is_still_applying() {
        let (sender, requests) = async_channel::bounded::<Relayed<String>>(1);
        let (finished, done) = std::sync::mpsc::channel::<()>();
        let window = std::thread::spawn(move || {
            // Held open without an answer, as a window still applying would.
            let reply = requests.recv_blocking().expect("a request");
            let _ = done.recv();
            drop(reply);
        });
        let started = std::time::Instant::now();
        let answer = relay(
            &sender,
            Patience {
                answer: std::time::Duration::from_millis(30),
                after_claim: std::time::Duration::from_millis(120),
            },
            |reply: Relayed<String>| {
                assert!(reply.claim());
                reply
            },
        );
        let waited = started.elapsed();
        finished.send(()).unwrap();
        window.join().unwrap();
        assert!(
            matches!(answer, Err(RelayError::Applying)),
            "a claimed request is never reported as a timeout: {answer:?}"
        );
        assert!(waited >= std::time::Duration::from_millis(150), "{waited:?}");
        assert!(waited < std::time::Duration::from_secs(2), "{waited:?}");
    }

    /// Abandoning and claiming exclude each other: a request the asker gave
    /// up on can no longer be claimed by the window.
    #[test]
    fn an_abandoned_request_can_no_longer_be_claimed() {
        let (sender, requests) = async_channel::bounded::<Relayed<String>>(1);
        let answer = relay(
            &sender,
            Patience {
                answer: std::time::Duration::from_millis(20),
                after_claim: std::time::Duration::from_secs(5),
            },
            |reply| reply,
        );
        assert!(matches!(answer, Err(RelayError::Timeout)), "{answer:?}");
        let late = requests.try_recv().expect("the request is still queued");
        assert!(!late.claim(), "the window must drop it unapplied");
    }
```

2. `crates/sprite-app/src/observation/request.rs`, `mod tests`. Change the `use super::{...}` at lines 331-333 to:

```rust
    use super::{
        ConfigVerb, DENIED, Query, ReloadRequest, Scope, ask_window, config_request, parse,
        render, respond,
    };
    use crate::workspace::Patience;
    use std::time::Duration;
```

and append:

```rust
    /// "Nothing was changed" is said only about a request the window can no
    /// longer apply.
    #[test]
    fn a_reload_the_window_never_took_says_nothing_was_changed() {
        let (reload, requests) = async_channel::bounded::<ReloadRequest>(1);
        let answer = ask_window(
            &reload,
            ConfigVerb::Reload,
            None,
            Patience {
                answer: Duration::from_millis(30),
                after_claim: Duration::from_secs(5),
            },
        );
        assert_eq!(
            answer,
            "this window did not answer in time; nothing was changed"
        );
        let late = requests.try_recv().expect("the request is still queued");
        assert!(!late.reply.claim(), "and the window can no longer apply it");
    }

    /// A reload the window has taken but not finished is reported as under
    /// way, never as unchanged.
    #[test]
    fn a_reload_the_window_took_but_has_not_finished_is_still_applying() {
        let (reload, requests) = async_channel::bounded::<ReloadRequest>(1);
        let (finished, done) = std::sync::mpsc::channel::<()>();
        let window = std::thread::spawn(move || {
            let request = requests
                .recv_blocking()
                .expect("a request reached the window");
            assert!(request.reply.claim(), "taken before the endpoint gave up");
            // Held open without an answer, as a window still applying would.
            let _ = done.recv();
            drop(request);
        });
        // A first wait long enough that the stand-in window certainly claims
        // the request inside it.
        let answer = ask_window(
            &reload,
            ConfigVerb::Reload,
            None,
            Patience {
                answer: Duration::from_secs(1),
                after_claim: Duration::from_millis(100),
            },
        );
        finished.send(()).unwrap();
        window.join().unwrap();
        assert_eq!(
            answer,
            "the window accepted this request and is still applying it"
        );
    }
```

3. `crates/sprite-app/src/workspace/surface_routing.rs`, append to `mod tests`:

```rust
    /// A connection that gave up has already told its program the request
    /// failed, so the window must not carry it out afterwards.
    #[gpui::test]
    fn a_surface_request_its_connection_gave_up_on_is_not_applied(
        cx: &mut gpui::TestAppContext,
    ) {
        use crate::surface::channel::SurfaceRequest;
        let (workspace, cx) = test_workspace(cx);
        let (reply, receiver) = std::sync::mpsc::sync_channel(1);
        let (reply, claim) = crate::workspace::Relayed::waiting(reply);
        assert!(claim.abandon(), "the connection gives up first");
        workspace.update_in(cx, |workspace, window, cx| {
            workspace.serve_surface_request(
                SurfaceRequest::RegisterToken {
                    name: "demo.abandoned".into(),
                    default: crate::tokens::unpack(0x12ab03),
                    description: "Abandoned".into(),
                    reply,
                },
                window,
                cx,
            );
        });
        cx.update(|_, cx| {
            assert!(
                !cx.global::<crate::tokens::TokenRegistry>()
                    .is_known("demo.abandoned"),
                "an abandoned registration must not take effect"
            );
        });
        assert!(
            matches!(
                receiver.try_recv(),
                Err(std::sync::mpsc::TryRecvError::Disconnected)
            ),
            "nobody is answered, because nobody is listening"
        );
    }
```

- [ ] **Step 2: Run them and confirm they fail**

```
TERM=dumb cargo test -p sprite-app --locked --offline --lib workspace::reload::tests
```

Expected: compile errors `cannot find type Patience`, `cannot find type Relayed`, `no variant Applying`, `no function ask_window with 4 arguments` / `cannot find ... Relayed in workspace`. (Behaviourally, at a62247e the window applies the queued reload after the endpoint's timeout, so `a_reload_the_endpoint_gave_up_on_is_never_applied` would fail its last assertion with size 21.)

- [ ] **Step 3: Implement**

1. `crates/sprite-app/src/workspace/reload.rs`: replace lines 104-133 (from `/// A reload asked for from an endpoint thread` to the end of `relay`) with:

```rust
/// A reload asked for from an endpoint thread, and where to put the answer.
///
/// The reply travels on a `std::sync::mpsc` channel rather than an async one
/// because the waiting side is a plain thread that needs a *timeout*: a wedged
/// GPUI thread must cost the endpoint a bounded wait, not a thread that never
/// returns. It carries the request's claim, which the window must win before
/// it reloads anything.
pub(crate) struct ReloadRequest {
    pub(crate) what: ConfigVerb,
    pub(crate) reply: Relayed<String>,
    pub(crate) reply_connection: Option<crate::local_socket::ReplyConnection>,
}

/// How much longer an asker waits once the window has claimed its request.
///
/// Bounded, because a wedged GPUI thread must not pin an endpoint thread and
/// its connection slot forever. Generous, because by then the work is under
/// way and the only honest early answer is that it still is.
pub(crate) const AFTER_CLAIM: std::time::Duration = std::time::Duration::from_secs(10);

/// Where one relayed request stands, shared by the thread that asked and the
/// window that answers.
///
/// It starts waiting and leaves that state exactly once: the window claims it
/// in order to apply it, or the asker abandons it in order to report that
/// nothing changed. Both moves are one atomic step out of waiting, so
/// whichever comes second learns that it lost before it acts.
#[derive(Clone, Debug, Default)]
pub(crate) struct Claim(Arc<std::sync::atomic::AtomicU8>);

impl Claim {
    const WAITING: u8 = 0;
    const CLAIMED: u8 = 1;
    const ABANDONED: u8 = 2;

    fn leave_waiting(&self, to: u8) -> Result<u8, u8> {
        self.0.compare_exchange(
            Self::WAITING,
            to,
            std::sync::atomic::Ordering::AcqRel,
            std::sync::atomic::Ordering::Acquire,
        )
    }

    /// Moves a waiting request to claimed. True when the window may apply it.
    fn claim(&self) -> bool {
        matches!(
            self.leave_waiting(Self::CLAIMED),
            Ok(_) | Err(Self::CLAIMED)
        )
    }

    /// Moves a waiting request to abandoned. True when the window can no
    /// longer apply it, so the asker may say that nothing changed.
    pub(crate) fn abandon(&self) -> bool {
        matches!(
            self.leave_waiting(Self::ABANDONED),
            Ok(_) | Err(Self::ABANDONED)
        )
    }
}

/// The answering half of a relayed request: where the answer goes, and the
/// claim that decides whether there may be one.
///
/// Public because Surface requests carry it and are public; only `send` is
/// usable outside this crate.
pub struct Relayed<Answer> {
    claim: Claim,
    reply: std::sync::mpsc::SyncSender<Answer>,
}

impl<Answer> Relayed<Answer> {
    /// A waiting request's reply, and the asker's hold on its claim.
    pub(crate) fn waiting(reply: std::sync::mpsc::SyncSender<Answer>) -> (Self, Claim) {
        let claim = Claim::default();
        (
            Self {
                claim: claim.clone(),
                reply,
            },
            claim,
        )
    }

    /// Takes the request for applying.
    ///
    /// False when the asker has already given up and told its caller that
    /// nothing changed: the request must then be dropped without being
    /// applied.
    pub(crate) fn claim(&self) -> bool {
        self.claim.claim()
    }

    /// Sends the answer to an asker that may have stopped listening.
    pub fn send(&self, answer: Answer) -> Result<(), std::sync::mpsc::SendError<Answer>> {
        self.reply.send(answer)
    }
}

/// A reply nobody has claimed or abandoned yet, for a request built by hand.
impl<Answer> From<std::sync::mpsc::SyncSender<Answer>> for Relayed<Answer> {
    fn from(reply: std::sync::mpsc::SyncSender<Answer>) -> Self {
        Self::waiting(reply).0
    }
}

impl<Answer> std::fmt::Debug for Relayed<Answer> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Relayed")
            .field("claim", &self.claim)
            .finish_non_exhaustive()
    }
}

/// How long an asker waits on the window.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Patience {
    /// For the window to answer. A request it has not claimed by then is
    /// abandoned, and will never be applied.
    pub(crate) answer: std::time::Duration,
    /// Further, when the window claimed the request before the asker could
    /// abandon it.
    pub(crate) after_claim: std::time::Duration,
}

impl Patience {
    /// Waits `answer` for the window, and [`AFTER_CLAIM`] more once it has
    /// claimed the request.
    pub(crate) const fn new(answer: std::time::Duration) -> Self {
        Self {
            answer,
            after_claim: AFTER_CLAIM,
        }
    }
}

#[derive(Debug)]
pub(crate) enum RelayError {
    /// The window is gone; the request never reached it.
    Disconnected,
    /// The window did not claim the request in time and now never will, so
    /// nothing was applied.
    Timeout,
    /// The window claimed the request and has not answered within the extra
    /// wait. It may still be applying it, so nobody may say nothing changed.
    Applying,
}

/// Hands a request to the window and waits, within `patience`, for its answer.
///
/// The request carries a claim the window must win before acting on it. When
/// the first wait runs out, the asker tries to abandon the claim instead; only
/// if that succeeds is the request known never to be applied. If the window
/// won first, the asker keeps waiting for the real answer, up to
/// `patience.after_claim`.
pub(crate) fn relay<Request, Answer>(
    sender: &async_channel::Sender<Request>,
    patience: Patience,
    request: impl FnOnce(Relayed<Answer>) -> Request,
) -> Result<Answer, RelayError> {
    let (reply, answer) = std::sync::mpsc::sync_channel(1);
    let (reply, claim) = Relayed::waiting(reply);
    sender
        .send_blocking(request(reply))
        .map_err(|_| RelayError::Disconnected)?;
    // A reply dropped unanswered ends this wait early too; the claim, not the
    // channel, then decides what may be said.
    if let Ok(answer) = answer.recv_timeout(patience.answer) {
        return Ok(answer);
    }
    if claim.abandon() {
        return Err(RelayError::Timeout);
    }
    answer
        .recv_timeout(patience.after_claim)
        .map_err(|_| RelayError::Applying)
}
```

2. `crates/sprite-app/src/workspace/mod.rs` line 22 becomes:

```rust
pub(crate) use reload::{Patience, RelayError, Relayed, ReloadRequest, relay};
```

Replace the reload task at lines 239-255 with:

```rust
        let reload_task = cx.spawn(async move |workspace, cx| {
            while let Ok(request) = reload_rx.recv().await {
                // An endpoint that gave up waiting has already told its caller
                // that nothing was changed. Reloading now would make that
                // untrue, so a request it abandoned is dropped unapplied.
                if !request.reply.claim() {
                    continue;
                }
                let answer = workspace
                    .update(cx, |workspace, cx| match request.what {
                        ConfigVerb::Reload => {
                            workspace.reload(request.reply_connection.as_ref(), cx)
                        }
                        // Printed from what the window is *using*, which after
                        // a reload is not necessarily what the file says.
                        ConfigVerb::Print => workspace.settings.to_toml(),
                    })
                    .unwrap_or_else(|_| "this window is closing".to_owned());
                // The endpoint thread is waiting on this with a timeout of its
                // own, so a failure here costs it a wait rather than a thread.
                let _ = request.reply.send(answer);
            }
        });
```

3. `crates/sprite-app/src/observation/request.rs`. Line 23 becomes `use crate::workspace::{Patience, ReloadRequest};`. Replace lines 211-212 with:

```rust
/// How long an endpoint thread will wait for the window to answer a reload.
const RELOAD_PATIENCE: Patience = Patience::new(std::time::Duration::from_secs(2));
```

Line 243 becomes `return ask_window(reload, verb, reply_connection, RELOAD_PATIENCE);`. Replace lines 309-327 (`ask_window`) with:

```rust
/// Hands the question to the GPUI thread and waits, within `patience`, for
/// its answer.
fn ask_window(
    reload: &async_channel::Sender<ReloadRequest>,
    what: ConfigVerb,
    reply_connection: Option<crate::local_socket::ReplyConnection>,
    patience: Patience,
) -> String {
    use crate::workspace::{RelayError, relay};
    match relay(reload, patience, |reply| ReloadRequest {
        what,
        reply,
        reply_connection,
    }) {
        Ok(answer) => answer,
        Err(RelayError::Disconnected) => "this window is no longer answering".to_owned(),
        // Said only once the request is abandoned, which the window can then
        // no longer apply.
        Err(RelayError::Timeout) => {
            "this window did not answer in time; nothing was changed".to_owned()
        }
        // The window took the request, so the change may be happening now.
        Err(RelayError::Applying) => {
            "the window accepted this request and is still applying it".to_owned()
        }
    }
}
```

4. `crates/sprite-app/src/surface/channel.rs`:
- Line 19 `use std::sync::mpsc::SyncSender;` becomes `use crate::workspace::{Patience, Relayed};`.
- After line 54 (`const REPLY_TIMEOUT: ...`) add:

```rust
/// The same wait, plus the bounded extra wait for a request the window has
/// already claimed.
const REPLY_PATIENCE: Patience = Patience::new(REPLY_TIMEOUT);
```

- Lines 399-400 become:

```rust
pub type Reply = Relayed<Result<(), Refusal>>;
pub type JsonReply = Relayed<Result<Value, Refusal>>;
```

- After `refuse_with` (ends at line 497), inside `impl SurfaceRequest`, add:

```rust
    /// Takes a request that is waiting for an answer, so the window may act
    /// on it.
    ///
    /// False only when its connection has already given up and told its
    /// program so; such a request must be dropped unapplied. A request nobody
    /// waits on is always the window's.
    pub(crate) fn claim(&self) -> bool {
        match self {
            Self::Capabilities { reply, .. } => reply.claim(),
            Self::Open { reply, .. }
            | Self::FocusPane { reply, .. }
            | Self::RegisterToken { reply, .. } => reply.claim(),
            Self::Update { .. }
            | Self::Focus { .. }
            | Self::Close { .. }
            | Self::Closed { .. }
            | Self::Grid { .. }
            | Self::List { .. } => true,
        }
    }
```

- In `serve_surface`, replace lines 654-683 (from `use crate::workspace::{RelayError, relay};` through the `Err(RelayError::Timeout) => { ... }` arm and the closing `}` of the `match`) with:

```rust
    use crate::workspace::{RelayError, relay};
    match relay(requests, REPLY_PATIENCE, |reply| SurfaceRequest::Open {
        id,
        pane,
        open,
        connection,
        reply,
    }) {
        Ok(Ok(())) => {
            let _ = handle.establish(&event_opened(id));
        }
        Ok(Err(refusal)) => {
            handle.abandon();
            refuse(&mut stream, &refusal.reason());
            return;
        }
        Err(RelayError::Disconnected) => {
            handle.abandon();
            refuse(&mut stream, NOT_ANSWERING);
            return;
        }
        // The wire reason is the same either way. An abandoned `Open` is
        // dropped unapplied, but one the window claimed may still place a
        // Surface after this; telling the window this Surface is already gone
        // covers both, so it never keeps one with a dead connection.
        // `close_surface` ignores an unknown id.
        Err(RelayError::Timeout | RelayError::Applying) => {
            handle.abandon();
            refuse(&mut stream, NO_ANSWER);
            let _ = requests.send_blocking(SurfaceRequest::Closed { id, pane });
            return;
        }
    }
```

- Replace `one_shot` (lines 730-746) with:

```rust
/// Asks the window once and relays its answer, then ends the connection.
fn one_shot<T>(
    stream: &mut UnixStream,
    requests: &async_channel::Sender<SurfaceRequest>,
    request: impl FnOnce(Relayed<Result<T, Refusal>>) -> SurfaceRequest,
    success: impl FnOnce(T) -> String,
) {
    use crate::workspace::{RelayError, relay};
    match relay(requests, REPLY_PATIENCE, request) {
        Ok(Ok(value)) => {
            let _ = writeln!(stream, "{}", success(value));
            let _ = stream.shutdown(Shutdown::Write);
        }
        Ok(Err(refusal)) => refuse(stream, &refusal.reason()),
        Err(RelayError::Disconnected) => refuse(stream, NOT_ANSWERING),
        // The refusal on the wire is unchanged. It says the window did not
        // answer, which is true of both, and never that nothing changed.
        Err(RelayError::Timeout | RelayError::Applying) => refuse(stream, NO_ANSWER),
    }
}
```

5. `crates/sprite-app/src/workspace/surface_routing.rs`: at the top of `serve_surface_request` (before `if self.stopping {` at line 11) insert:

```rust
        // A connection that gave up waiting has already told its program the
        // request failed; carrying it out now would make that untrue.
        if !request.claim() {
            return;
        }
```

6. Requests built by hand in existing tests now need a `Relayed`. Change the struct-literal shorthand `reply,` to `reply: reply.into(),` at: `crates/sprite-app/src/workspace/close_gate.rs:578`; `crates/sprite-app/src/workspace/surface_routing.rs:120, 177, 199, 260, 273, 287` (lines 177 and 199 end `..., reply,` on one line); `crates/sprite-app/src/terminal_view/surfaces.rs:1752, 2187, 2268, 2424, 2483`. Do **not** change `surface_routing.rs:20` or `surfaces.rs:312, 322`, which are destructuring patterns. Stand-in windows that call `reply.send(...)` (in `surface/channel.rs` tests and `crates/sprite-app/tests/client.rs`) compile unchanged, because `Relayed::send` has `SyncSender::send`'s signature.

Then `cargo fmt --all`.

- [ ] **Step 4: Run tests, confirm pass**

```
TERM=dumb cargo test -p sprite-app --locked --offline --lib workspace::
TERM=dumb cargo test -p sprite-app --locked --offline --lib observation::request::tests
TERM=dumb cargo test -p sprite-app --locked --offline --lib surface::channel::tests
TERM=dumb cargo test -p sprite-app --locked --offline --lib terminal_view::surfaces
TERM=dumb cargo test -p sprite-app --locked --offline --test client
```

All pass. `surface::channel::tests::a_window_that_never_answers_still_hears_the_surface_closed` still waits out the real 5 s `REPLY_TIMEOUT` (its stand-in never claims, so the request is abandoned at 5 s, not 15 s).

- [ ] **Step 5: Commit**

```
git add crates/sprite-app/src/workspace/reload.rs crates/sprite-app/src/workspace/mod.rs crates/sprite-app/src/workspace/close_gate.rs crates/sprite-app/src/workspace/surface_routing.rs crates/sprite-app/src/observation/request.rs crates/sprite-app/src/surface/channel.rs crates/sprite-app/src/terminal_view/surfaces.rs
git commit -m "fix(workspace): claim a relayed request before applying it

Every relayed request (config reload/print, Surface open, capabilities,
focus, token registration) carries Waiting/Claimed/Abandoned state. The
window applies only after claiming; a timing-out endpoint abandons first
and only then says nothing changed. A request the window already claimed
gets up to 10 s more and otherwise reports that the window is still
applying it. (BCA-08, ADR 0028)

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 3: Every pane registers with the window's registry, endpoint or not (BCA-11, R-W1)

**Files:**
- Modify: `crates/sprite-app/src/workspace/pane_factory.rs:72-107` (`make_pane` link, `pane_link`)
- Modify: `crates/sprite-app/src/terminal_view.rs:77-78` (field comment only)
- No change needed: `crates/sprite-app/src/workspace/reload.rs:18-35` (`change_observation` opens a new endpoint over `self.panes`, which now already holds every pane) and `crates/sprite-app/src/terminal_view.rs:275-279` (registration already runs whenever a link is present).
- Test: `crates/sprite-app/src/workspace/reload.rs` (`mod tests`, GPUI, child-process pattern of `reload_reconciles_observation_endpoint_and_revokes_old_credentials`)

**Interfaces:**
- Consumes: nothing from Tasks 1-2 (the test speaks to the real endpoint over its socket, so it is unaffected by Task 1's ticket API).
- Produces: `pane_link(panes: &Arc<WindowPanes>, tab: TabId, pane: PaneId) -> PaneLink` (no `endpoint` parameter, never `None`).

- [ ] **Step 1: Write the failing test**

Append to `mod tests` in `crates/sprite-app/src/workspace/reload.rs`. Opening an endpoint reads `XDG_RUNTIME_DIR` (Linux) or `TMPDIR` (macOS), so, like the existing reload test, it re-runs itself in a child process that owns those variables. Helpers reused: `test_workspace` (`workspace/test_support.rs`), `Workspace::split` / `open_tab` (`pane_factory.rs`), `Workspace::reload`, `Workspace::begin_shutdown` (`close_gate.rs`).

```rust
    /// Observation turned on by a reload must find the panes that were opened
    /// while it was off, not only the ones opened afterwards.
    #[gpui::test]
    fn panes_opened_while_observation_was_off_are_observable_once_reload_turns_it_on(
        cx: &mut gpui::TestAppContext,
    ) {
        // A child owns runtime-directory variables without racing parallel
        // tests. Its private directory also works without a logged-in desktop.
        if std::env::var_os("SPRITE_OBSERVATION_EXISTING_PANES_TEST_CHILD").is_none() {
            let directory = std::env::temp_dir().join(format!("sp-e-{:x}", std::process::id()));
            std::fs::create_dir_all(&directory).unwrap();
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "workspace::reload::tests::panes_opened_while_observation_was_off_are_observable_once_reload_turns_it_on", "--nocapture"])
                .env("SPRITE_OBSERVATION_EXISTING_PANES_TEST_CHILD", "1")
                .env("XDG_RUNTIME_DIR", &directory)
                .env("TMPDIR", &directory)
                .output()
                .unwrap();
            std::fs::remove_dir_all(directory).unwrap();
            assert!(
                String::from_utf8_lossy(&output.stdout).contains("1 passed"),
                "the subprocess must run and pass its exact test\n{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(
                output.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        use std::io::{Read, Write};
        use std::os::unix::net::UnixStream;
        let (workspace, cx) = test_workspace(cx);
        let path = std::env::temp_dir().join(format!(
            "sprite-existing-panes-{}.toml",
            std::process::id()
        ));
        // Real sessions, so the panes can answer. The test workspace's own
        // first pane runs a program that does not exist and never has a
        // session to register, so it is left out of what is expected.
        let opened = workspace.update_in(cx, |workspace, window, cx| {
            workspace.config_path = Some(path.clone());
            workspace.command = Some(vec![
                "/bin/sh".into(),
                "-c".into(),
                "printf 'existing-pane\\n'; exec sleep 30".into(),
            ]);
            workspace.split(Orientation::Vertical, window, cx);
            workspace.open_tab(window, cx);
            let mut opened: Vec<u64> = workspace
                .tabs
                .all_panes()
                .into_iter()
                .map(|(_, pane, _)| pane.0)
                .filter(|pane| *pane != 0)
                .collect();
            opened.sort_unstable();
            opened
        });
        assert_eq!(opened.len(), 2, "a split and a new tab: {opened:?}");
        workspace.read_with(cx, |workspace, _| {
            assert!(workspace.endpoint.is_none(), "observation starts off");
        });

        std::fs::write(&path, "[pane_observation]\nenabled = true\n").unwrap();
        let report = workspace.update(cx, |workspace, cx| workspace.reload(None, cx));
        assert!(report.contains("applied now: pane_observation"), "{report}");
        let (socket, key) = workspace.read_with(cx, |workspace, _| {
            let endpoint = workspace
                .endpoint
                .as_ref()
                .expect("the reload turned observation on");
            (endpoint.socket_path().to_owned(), endpoint.key_hex())
        });

        let (answer, receive) = async_channel::bounded(1);
        let client = std::thread::spawn(move || {
            let result = (|| -> std::io::Result<String> {
                let mut stream = UnixStream::connect(socket)?;
                stream.set_read_timeout(Some(std::time::Duration::from_secs(5)))?;
                writeln!(stream, "{key} sprite-observation/1 panes snapshot --window")?;
                let mut answer = String::new();
                stream.read_to_string(&mut answer)?;
                Ok(answer)
            })();
            answer.send_blocking(result).unwrap();
        });
        let executor = cx.executor();
        executor.allow_parking();
        let response = executor
            .block_test(async { receive.recv().await.unwrap() })
            .unwrap();
        client.join().unwrap();

        let value: serde_json::Value = serde_json::from_str(&response)
            .unwrap_or_else(|error| panic!("{error}: {response}"));
        let mut answered: Vec<u64> = value["panes"]
            .as_array()
            .unwrap_or_else(|| panic!("a pane list: {response}"))
            .iter()
            .map(|pane| pane["pane"].as_u64().expect("a pane id"))
            .collect();
        answered.sort_unstable();
        assert_eq!(
            answered, opened,
            "every pane opened while observation was off is listed and answers: {response}"
        );
        assert_eq!(value["complete"], serde_json::json!(true), "{response}");

        let cleanups = workspace.update(cx, |workspace, cx| workspace.begin_shutdown(cx));
        executor.block_test(async move {
            for cleanup in cleanups {
                cleanup.await;
            }
        });
        std::fs::remove_file(path).unwrap();
    }
```

- [ ] **Step 2: Run it and confirm it fails**

```
TERM=dumb cargo test -p sprite-app --locked --offline --lib workspace::reload::tests::panes_opened_while_observation_was_off_are_observable_once_reload_turns_it_on -- --exact
```

Expected: the parent's first assertion fails, and the child output it prints contains `assertion left == right failed: every pane opened while observation was off is listed and answers ... left: [] right: [1, 2]` (the endpoint opens, but the registry is empty).

- [ ] **Step 3: Implement**

`crates/sprite-app/src/workspace/pane_factory.rs`: line 74 becomes

```rust
        let link = pane_link(services.panes, tab, pane);
```

and line 80 (`link,` in the `TerminalView::new` call) becomes `Some(link),`. Replace lines 92-107 (`pane_link` and its doc comment) with:

```rust
/// How a pane is reached by observation.
///
/// Every pane is linked, whether or not the window has an endpoint right now:
/// a reload can turn observation on while panes are running, and it then has
/// to find every one of them, not only those opened afterwards. The registry
/// is reachable only through an endpoint, so a window without one exposes
/// nothing by keeping it filled.
pub(super) fn pane_link(panes: &Arc<WindowPanes>, tab: TabId, pane: PaneId) -> PaneLink {
    PaneLink {
        pane,
        tab,
        panes: Arc::clone(panes),
    }
}
```

`crates/sprite-app/src/terminal_view.rs` line 77 becomes:

```rust
    /// How this pane is reached by observation. `None` for a pane whose
    /// session never started, for one built outside a window, and once the
    /// pane has begun shutting down.
```

- [ ] **Step 4: Run tests, confirm pass**

```
TERM=dumb cargo test -p sprite-app --locked --offline --lib workspace::
TERM=dumb cargo test -p sprite-app --locked --offline --lib terminal_view::tests
```

- [ ] **Step 5: Commit**

```
git add crates/sprite-app/src/workspace/pane_factory.rs crates/sprite-app/src/terminal_view.rs crates/sprite-app/src/workspace/reload.rs
git commit -m "fix(workspace): register every pane whether or not observation is on

Panes were linked to the window registry only when an endpoint existed, so
turning observation on by reload exposed no pane opened while it was off.
Every pane now registers at creation; the registry is reachable only
through an endpoint, so nothing is exposed while observation is off.
(BCA-11)

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 4: The local-socket accept loop never ends on an accept error (BCA-15, R-L1)

**Files:**
- Modify: `crates/sprite-app/src/local_socket.rs:18` (constants after `KEY_BYTES`), `:216-247` (listener loop), and add two private functions after `impl Drop for LocalSocket` (line 361)
- Test: `crates/sprite-app/src/local_socket.rs` (`mod tests`, plain `#[test]`; real `LocalSocket`, real `UnixStream`, real descriptor exhaustion in a child process)

**Interfaces:**
- Consumes: nothing from other tasks.
- Produces: no new public items. Private: `const ACCEPT_REST_MS: u16 = 100`, `enum AfterAcceptError { Poll, Rest }`, `fn after_accept_error(&io::Error) -> AfterAcceptError`, `fn rest(&UnixStream, &AtomicBool) -> bool`.

**Choice of seam.** The workspace's `nix` has features `fs, poll, process, signal`, not `resource`, so `nix::sys::resource::setrlimit` is unavailable. Adding the feature is not needed: `nix` always re-exports `libc` (`nix-0.28.0/src/lib.rs:97`, `pub use libc;`), so the test calls `nix::libc::setrlimit` / `getrlimit` / `getrusage` directly in test-only `unsafe` blocks. No crate, feature or `Cargo.toml` change, and the regression test provokes a genuine `EMFILE` from `accept` on the real listener rather than injecting one. Because the descriptor limit is process-wide, the test re-runs itself in a child process, as `workspace::reload::tests::reload_reconciles_observation_endpoint_and_revokes_old_credentials` does for environment variables. A pure classification test is added alongside it to pin that no `accept` error kind ends the loop.

- [ ] **Step 1: Write the failing test (compiles at a62247e)**

Append to `mod tests` in `crates/sprite-app/src/local_socket.rs`. Helpers reused from that module: `POLICY`, `Scratch`, `rejected`.

```rust
    /// Lowers this process's soft descriptor limit.
    fn lower_descriptor_limit(limit: nix::libc::rlim_t) {
        let mut current = nix::libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        // SAFETY: getrlimit and setrlimit only read and write the struct they
        // are given.
        unsafe {
            assert_eq!(
                nix::libc::getrlimit(nix::libc::RLIMIT_NOFILE, &mut current),
                0
            );
            let lowered = nix::libc::rlimit {
                rlim_cur: current.rlim_cur.min(limit),
                rlim_max: current.rlim_max,
            };
            assert_eq!(nix::libc::setrlimit(nix::libc::RLIMIT_NOFILE, &lowered), 0);
        }
    }

    /// Opens descriptors until the process has none left, then gives one back
    /// for the caller's own end of a connection.
    fn exhaust_descriptors() -> Vec<File> {
        let mut held = Vec::new();
        loop {
            match File::open("/dev/null") {
                Ok(file) => held.push(file),
                Err(error) if error.raw_os_error() == Some(Errno::EMFILE as i32) => break,
                Err(error) => panic!("exhausting descriptors failed another way: {error}"),
            }
        }
        held.pop()
            .expect("at least one descriptor opens under the lowered limit");
        held
    }

    /// CPU time this process has used, user and system together.
    fn cpu_time() -> Duration {
        let mut usage = std::mem::MaybeUninit::<nix::libc::rusage>::zeroed();
        // SAFETY: getrusage fills the struct it is given and reads nothing
        // else; it is fully written when the call returns 0.
        let usage = unsafe {
            assert_eq!(
                nix::libc::getrusage(nix::libc::RUSAGE_SELF, usage.as_mut_ptr()),
                0
            );
            usage.assume_init()
        };
        let time = |value: nix::libc::timeval| {
            Duration::from_secs(value.tv_sec as u64) + Duration::from_micros(value.tv_usec as u64)
        };
        time(usage.ru_utime) + time(usage.ru_stime)
    }

    /// Running out of descriptors is a pause, not the end of the listener.
    ///
    /// `accept` fails with `EMFILE` while a connection waits, and keeps failing
    /// until descriptors are freed. The listener used to stop at the first
    /// failure, leaving a socket nobody served; skipping the failure instead
    /// would spin, because `poll` keeps reporting the waiting connection.
    #[test]
    fn running_out_of_descriptors_rests_the_listener_instead_of_ending_it() {
        const CHILD: &str = "SPRITE_ACCEPT_EXHAUSTION_TEST_CHILD";
        // The descriptor limit is process-wide, so it is lowered in a child
        // running only this test rather than under parallel neighbours.
        if std::env::var_os(CHILD).is_none() {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "local_socket::tests::running_out_of_descriptors_rests_the_listener_instead_of_ending_it",
                    "--nocapture",
                    "--test-threads=1",
                ])
                .env(CHILD, "1")
                .output()
                .unwrap();
            assert!(
                String::from_utf8_lossy(&output.stdout).contains("1 passed"),
                "the subprocess must run and pass its exact test\n{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(
                output.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        use std::os::fd::AsRawFd;

        let scratch = Scratch::new();
        let (bodies, received) = mpsc::channel();
        let mut socket = LocalSocket::open_in(
            scratch.0.clone(),
            Arc::new(ObservationKey::generate().unwrap()),
            POLICY,
            rejected,
            move |connection| {
                let _ = bodies.send(connection.body);
            },
        )
        .unwrap();
        let key = socket.key_hex();
        let path = socket.socket_path().to_owned();
        // A limit just above what is open now, so exhausting it is quick and
        // freeing it leaves room for an accept and its clones.
        let lowest_free = File::open("/dev/null").unwrap().as_raw_fd();
        lower_descriptor_limit(lowest_free as nix::libc::rlim_t + 32);

        // A connection arrives while no descriptor is free to accept it with.
        let held = exhaust_descriptors();
        let mut first = UnixStream::connect(&path).unwrap();
        writeln!(first, "{key} first").unwrap();

        // The listener waits that out without spinning.
        let before = cpu_time();
        crate::test_blocking_wait::pause(Duration::from_millis(600));
        let spent = cpu_time().saturating_sub(before);
        assert!(
            spent < Duration::from_millis(200),
            "the listener spun for {spent:?} of CPU while descriptors were exhausted"
        );

        // Once descriptors return, the connection that waited is served.
        drop(held);
        assert_eq!(
            received
                .recv_timeout(Duration::from_secs(2))
                .expect("the listener must survive running out of descriptors"),
            "first"
        );

        // Closing still ends a listener that is resting.
        let held = exhaust_descriptors();
        let mut second = UnixStream::connect(&path).unwrap();
        writeln!(second, "{key} second").unwrap();
        crate::test_blocking_wait::pause(Duration::from_millis(250));
        let (done, completed) = mpsc::channel();
        std::thread::spawn(move || {
            socket.close();
            done.send(()).unwrap();
        });
        completed
            .recv_timeout(Duration::from_secs(2))
            .expect("closing must end a resting listener");
        drop(held);
        drop((first, second));
    }
```

- [ ] **Step 2: Run it and confirm it fails**

```
TERM=dumb cargo test -p sprite-app --locked --offline --lib local_socket::tests::running_out_of_descriptors_rests_the_listener_instead_of_ending_it -- --exact
```

Expected: the parent's first assertion fails; the child output it prints contains `the listener must survive running out of descriptors: Timeout` (at a62247e `Err(_) => break` ends the listener on the first `EMFILE`).

- [ ] **Step 1b: Write the classification test (fails to compile until Step 3)**

Append to `mod tests`:

```rust
    /// No failed `accept` ends the listener. Only "nothing was waiting" and an
    /// interrupted call go straight back to `poll`; everything else rests
    /// first, because `poll` would report the same waiting connection at once.
    #[test]
    fn no_accept_failure_ends_the_listener() {
        for errno in [
            Errno::EMFILE,
            Errno::ENFILE,
            Errno::ECONNABORTED,
            Errno::ENOBUFS,
            Errno::ENOMEM,
            Errno::EBADF,
            Errno::EINVAL,
        ] {
            assert_eq!(
                after_accept_error(&io::Error::from_raw_os_error(errno as i32)),
                AfterAcceptError::Rest,
                "{errno}"
            );
        }
        for errno in [Errno::EAGAIN, Errno::EINTR] {
            assert_eq!(
                after_accept_error(&io::Error::from_raw_os_error(errno as i32)),
                AfterAcceptError::Poll,
                "{errno}"
            );
        }
    }
```

Run `TERM=dumb cargo test -p sprite-app --locked --offline --lib local_socket::tests::no_accept_failure_ends_the_listener -- --exact`; expected `cannot find function after_accept_error` / `cannot find type AfterAcceptError`.

- [ ] **Step 3: Implement**

1. After line 18 (`const KEY_BYTES: usize = 32;`) add:

```rust

/// How long the listener rests after `accept` fails for any reason other than
/// "nothing is waiting".
///
/// Long enough that a listener the system has run out of descriptors for costs
/// nothing while it waits, short enough that service resumes almost as soon as
/// they are freed.
const ACCEPT_REST_MS: u16 = 100;
```

2. In the listener loop, replace lines 226-230 (the `match poll(&mut fds, PollTimeout::NONE) { ... }`) with:

```rust
                        match poll(&mut fds, PollTimeout::NONE) {
                            Err(Errno::EINTR) => continue,
                            // A failing `poll` is waited out like a failing
                            // `accept`: only closing the socket ends the loop.
                            Err(_) => {
                                if rest(&cancelled, &running) {
                                    break;
                                }
                                continue;
                            }
                            Ok(_) => {}
                        }
```

and replace lines 236-247 (the `let stream = match listener.accept() { ... };`) with:

```rust
                        let stream = match listener.accept() {
                            Ok((stream, _)) => stream,
                            Err(error) => match after_accept_error(&error) {
                                AfterAcceptError::Poll => continue,
                                AfterAcceptError::Rest => {
                                    if rest(&cancelled, &running) {
                                        break;
                                    }
                                    continue;
                                }
                            },
                        };
```

3. After `impl Drop for LocalSocket { ... }` (ends at line 361) add:

```rust
/// What the listener does after `accept` fails.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AfterAcceptError {
    /// Nothing was waiting after all, or a signal interrupted the call: wait
    /// in `poll` as usual.
    Poll,
    /// Anything else: rest, then try again. `poll` would report the same
    /// waiting connection at once, so going straight back would spin.
    Rest,
}

/// Classifies a failed `accept`. No failure ends the listener; only closing
/// the socket does.
///
/// Running out of descriptors (`EMFILE`, `ENFILE`), buffers (`ENOBUFS`) or
/// memory (`ENOMEM`), and a peer that hung up mid-accept (`ECONNABORTED`), all
/// pass, so the listener waits them out. An unexpected error is treated the
/// same way rather than ending the loop: a listener that stops silently leaves
/// a socket that accepts connections nobody will ever serve.
fn after_accept_error(error: &io::Error) -> AfterAcceptError {
    match error.kind() {
        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted => AfterAcceptError::Poll,
        _ => AfterAcceptError::Rest,
    }
}

/// Rests for [`ACCEPT_REST_MS`], waking early if the socket is closed.
///
/// Returns whether the socket is being closed.
fn rest(cancelled: &UnixStream, running: &AtomicBool) -> bool {
    let mut fds = [PollFd::new(cancelled.as_fd(), PollFlags::POLLIN)];
    if poll(&mut fds, PollTimeout::from(ACCEPT_REST_MS)).is_err() {
        // A failing `poll` must not turn the rest into a spin.
        std::thread::sleep(Duration::from_millis(u64::from(ACCEPT_REST_MS)));
    }
    !running.load(Ordering::SeqCst)
        || !fds[0]
            .revents()
            .unwrap_or_else(PollFlags::empty)
            .is_empty()
}
```

- [ ] **Step 4: Run tests, confirm pass**

```
TERM=dumb cargo test -p sprite-app --locked --offline --lib local_socket::
TERM=dumb cargo test -p sprite-app --locked --offline --lib observation::endpoint
TERM=dumb cargo test -p sprite-app --locked --offline --lib surface::channel::tests
```

- [ ] **Step 5: Commit**

```
git add crates/sprite-app/src/local_socket.rs
git commit -m "fix(local-socket): rest and retry on accept errors instead of stopping

A failed accept (EMFILE, ENFILE, ECONNABORTED, ENOBUFS, ENOMEM, or anything
unexpected) and a failed poll now rest about 100 ms in a poll that wakes on
cancellation, then retry. The loop no longer ends on an error and never
spins; only closing the socket ends it. (BCA-15)

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

---

### Task 5: `Confirmation<T>`, close/quit confirmation and non-repeating workspace actions (BCA-02, BCA-31; R-C3.1, R-C3.2, R-C3.3, R-C3.5)

**Files:**
- Create: `crates/sprite-app/src/confirmation.rs`
- Modify: `crates/sprite-app/src/lib.rs:10` (module list)
- Modify: `crates/sprite-app/src/workspace/keymap.rs:87-104` (`WorkspaceAction`), `:172-238` (`key_down`)
- Modify: `crates/sprite-app/src/workspace/close_gate.rs:1` (imports), `:148-176` (`may_close`), `:230-235` (`PendingClose`), `:287-308` (`CloseGate`), `:315-341` (unit test)
- Modify: `crates/sprite-app/src/workspace/mod.rs:136` (struct field), `:268-305` (`Workspace::new`), `:473` (render chain)
- Modify: `crates/sprite-app/src/workspace/layout_tests.rs:196-199`, `:290-293` (`PendingClose` literals)
- Test: `crates/sprite-app/src/confirmation.rs` (`#[cfg(test)] mod tests`), `crates/sprite-app/src/workspace/keymap.rs` (`mod tests`, line 240), `crates/sprite-app/src/workspace/close_gate.rs` (`mod tests`, line 310)

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces:
  - `crate::confirmation::Confirmation<T>` (`#[derive(Clone, Debug, Eq, PartialEq)]`, `Default` for every `T`) with `pub(crate) fn arm(&mut self, subject: T)`, `pub(crate) fn answer(&mut self, subject: &T, is_held: bool) -> bool`, `pub(crate) fn disarm(&mut self)`, `pub(crate) fn is_armed(&self) -> bool` (all on `impl<T: PartialEq>`).
  - `WorkspaceAction::repeats(self) -> bool` (`pub(super)`).
  - `PendingClose::new(scope: CloseScope, label: SharedString) -> PendingClose`; `PendingClose` now holds `confirmation: Confirmation<CloseScope>` instead of `scope`.

#### Cycle A: the `Confirmation<T>` type

- [ ] **Step 1: Write the failing test.** Create `crates/sprite-app/src/confirmation.rs` containing only this test module for now:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_fresh_press_on_the_same_subject_confirms() {
        let mut confirmation = Confirmation::default();
        assert!(!confirmation.is_armed());
        assert!(
            !confirmation.answer(&"close pane", false),
            "nothing was asked"
        );

        confirmation.arm("close pane");
        assert!(confirmation.is_armed());
        assert!(
            !confirmation.answer(&"close pane", true),
            "an auto-repeat is the first press still going"
        );
        assert!(
            !confirmation.answer(&"close tab", false),
            "a different subject is a different question"
        );
        assert!(confirmation.is_armed(), "neither withdrew the question");

        assert!(confirmation.answer(&"close pane", false));
        assert!(!confirmation.is_armed(), "an answer is used up");
        assert!(
            !confirmation.answer(&"close pane", false),
            "so a third press asks again rather than acting twice"
        );
    }

    #[test]
    fn a_withdrawn_question_cannot_be_answered() {
        let mut confirmation = Confirmation::default();
        confirmation.arm(String::from("one\ntwo"));
        confirmation.disarm();
        assert!(!confirmation.is_armed());
        assert!(!confirmation.answer(&String::from("one\ntwo"), false));
    }

    #[test]
    fn arming_again_replaces_the_subject() {
        let mut confirmation = Confirmation::default();
        confirmation.arm(1);
        confirmation.arm(2);
        assert!(!confirmation.answer(&1, false));
        assert!(confirmation.answer(&2, false));
    }
}
```

In `crates/sprite-app/src/lib.rs`, after line 10 (`mod config;`) add:

```rust
mod confirmation;
```

- [ ] **Step 2: Run it and confirm it fails.**
  `TERM=dumb cargo test -p sprite-app --locked --offline confirmation::tests`
  Expected: compile error `cannot find type `Confirmation` in this scope` (E0433/E0412) in `confirmation.rs`.

- [ ] **Step 3: Implement.** Put this above the test module in `crates/sprite-app/src/confirmation.rs`:

```rust
//! A question answered by doing the same thing again.
//!
//! Closing a busy pane and pasting text that would run as commands both ask
//! first, and both are answered by repeating the gesture. What counts as
//! "again" is the whole of the safety: the auto-repeat of a key that is still
//! held down is the first press continuing, not a second decision, and a
//! repeat aimed at something other than what was asked about answers nothing.
//! This type holds that rule so that each question does not re-derive it.

/// One pending question about a `T`, or none.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Confirmation<T> {
    armed: Option<T>,
}

// Written out rather than derived: a derived `Default` would demand
// `T: Default`, and an unarmed question needs no subject at all.
impl<T> Default for Confirmation<T> {
    fn default() -> Self {
        Self { armed: None }
    }
}

impl<T: PartialEq> Confirmation<T> {
    /// Asks about `subject`, replacing any earlier question.
    pub(crate) fn arm(&mut self, subject: T) {
        self.armed = Some(subject);
    }

    /// Whether this gesture answers the question.
    ///
    /// Only a fresh press aimed at the same subject does, and answering uses
    /// the question up, so a third press asks again instead of acting twice.
    /// Anything else leaves the question exactly as it was.
    pub(crate) fn answer(&mut self, subject: &T, is_held: bool) -> bool {
        if is_held || self.armed.as_ref() != Some(subject) {
            return false;
        }
        self.armed = None;
        true
    }

    /// Withdraws the question; nothing can answer it now.
    pub(crate) fn disarm(&mut self) {
        self.armed = None;
    }

    /// Whether a question is waiting for its answer.
    pub(crate) fn is_armed(&self) -> bool {
        self.armed.is_some()
    }
}
```

- [ ] **Step 4: Run tests, confirm pass.**
  `TERM=dumb cargo test -p sprite-app --locked --offline confirmation::tests`
  Expected: 3 passed. (A non-test build warns that `disarm` and `is_armed` are unused until Task 6 gives them production callers; do not add `allow` attributes.)

#### Cycle B: held keys, and what withdraws a close question

- [ ] **Step 1: Write the failing tests.** These compile at master; they use only existing items.

In `crates/sprite-app/src/workspace/keymap.rs`, inside `mod tests` (opens at line 240), after the `KeyboardPane` impls (after line 280) add the helpers and three tests:

```rust
    /// One auto-repeat of a key that is still held down.
    fn hold(cx: &mut gpui::VisualTestContext, keystroke: gpui::Keystroke) {
        cx.simulate_event(KeyDownEvent {
            keystroke,
            is_held: true,
        });
    }

    /// Two panes that both report a running program, so every close asks.
    fn busy_panes(workspace: &gpui::Entity<Workspace>, cx: &mut gpui::VisualTestContext) {
        workspace.update(cx, |workspace, cx| {
            let mut make = |_, _| {
                Rc::new(cx.new(|cx| BusyPane {
                    focus: cx.focus_handle(),
                })) as Rc<dyn PaneHandle<Request = SurfaceRequest>>
            };
            workspace.tabs = Tabs::new(&mut make);
            workspace.tabs.split(Orientation::Horizontal, &mut make);
            workspace.tabs.focus_pane(PaneId(0));
            workspace.refresh_layout(cx);
        });
        draw_workspace(cx);
    }

    fn asking(workspace: &gpui::Entity<Workspace>, cx: &mut gpui::VisualTestContext) -> bool {
        workspace.read_with(cx, |workspace, _| {
            matches!(workspace.mode, Mode::ConfirmingClose(_))
        })
    }

    /// Holding a close key auto-repeats it. The first press asks; the repeats
    /// that follow while the key is still down are that same press, so they
    /// neither answer the question nor dismiss it. Letting go and pressing
    /// again is what closes.
    #[gpui::test]
    fn a_held_close_key_cannot_answer_its_own_question(cx: &mut gpui::TestAppContext) {
        let (workspace, cx) = test_workspace(cx);
        busy_panes(&workspace, cx);

        cx.simulate_keystrokes("ctrl-shift-w");
        assert!(asking(&workspace, cx));
        for _ in 0..3 {
            hold(cx, press("w", ctrl_shift()));
        }
        workspace.read_with(cx, |workspace, _| {
            assert_eq!(
                workspace.tabs.active().unwrap().len(),
                2,
                "a held repeat closed the pane"
            );
        });
        assert!(asking(&workspace, cx), "a held repeat dismissed the question");
        cx.simulate_keystrokes("ctrl-shift-w");
        workspace.read_with(cx, |workspace, _| {
            assert_eq!(
                workspace.tabs.active().unwrap().len(),
                1,
                "a fresh second press still confirms"
            );
            assert!(matches!(workspace.mode, Mode::Idle));
        });

        // The same for a tab. A second tab keeps the window open when one closes.
        workspace.update(cx, |workspace, cx| {
            workspace.tabs.open(|_, _| {
                Rc::new(cx.new(|cx| BusyPane {
                    focus: cx.focus_handle(),
                })) as Rc<dyn PaneHandle<Request = SurfaceRequest>>
            });
            workspace.refresh_layout(cx);
        });
        draw_workspace(cx);
        cx.simulate_keystrokes("ctrl-shift-q");
        assert!(asking(&workspace, cx));
        for _ in 0..3 {
            hold(cx, press("q", ctrl_shift()));
        }
        workspace.read_with(cx, |workspace, _| {
            assert_eq!(workspace.tabs.len(), 2, "a held repeat closed the tab");
        });
        assert!(asking(&workspace, cx));
        cx.simulate_keystrokes("ctrl-shift-q");
        workspace.read_with(cx, |workspace, _| assert_eq!(workspace.tabs.len(), 1));
    }

    /// Holding the split key makes one split and holding the new-tab key one
    /// tab. Movement is what holding a key is for, so the font keeps growing.
    #[gpui::test]
    fn a_held_binding_acts_once_unless_it_is_movement(cx: &mut gpui::TestAppContext) {
        let (workspace, cx) = test_workspace(cx);
        draw_workspace(cx);
        cx.simulate_keystrokes("ctrl-shift-d");
        for _ in 0..3 {
            hold(cx, press("d", ctrl_shift()));
        }
        workspace.read_with(cx, |workspace, _| {
            assert_eq!(
                workspace.tabs.active().unwrap().len(),
                2,
                "holding the split key made more than one split"
            );
        });
        draw_workspace(cx);
        cx.simulate_keystrokes("ctrl-shift-t");
        for _ in 0..3 {
            hold(cx, press("t", ctrl_shift()));
        }
        workspace.read_with(cx, |workspace, _| {
            assert_eq!(
                workspace.tabs.len(),
                2,
                "holding the new-tab key opened more than one tab"
            );
        });
        draw_workspace(cx);
        let before = workspace.read_with(cx, |workspace, _| workspace.settings.font.size.get());
        cx.simulate_keystrokes("ctrl-shift-=");
        for _ in 0..2 {
            hold(cx, press("=", ctrl_shift()));
        }
        let after = workspace.read_with(cx, |workspace, _| workspace.settings.font.size.get());
        assert!(
            (after - before - 3.0).abs() < 1e-3,
            "a held zoom key keeps stepping: {before} -> {after}"
        );
    }

    /// A deliberate act withdraws a close question: a button press anywhere,
    /// or the window going to the background. Pointer motion, a wheel turn, a
    /// modifier on its own and a resize are not decisions and leave it standing.
    #[gpui::test]
    fn a_close_question_is_withdrawn_by_deliberate_input_only(cx: &mut gpui::TestAppContext) {
        use gpui::{MouseButton, ScrollDelta, ScrollWheelEvent};
        let (workspace, cx) = test_workspace(cx);
        busy_panes(&workspace, cx);
        cx.update(|window, _| window.activate_window());
        cx.run_until_parked();

        cx.simulate_keystrokes("ctrl-shift-w");
        assert!(asking(&workspace, cx));
        cx.simulate_resize(gpui::size(px(900.0), px(700.0)));
        draw_workspace(cx);
        let pane = cx.debug_bounds("workspace-pane-0").unwrap().center();
        cx.simulate_mouse_move(pane, None, Modifiers::default());
        cx.simulate_event(ScrollWheelEvent {
            position: pane,
            delta: ScrollDelta::Lines(gpui::point(0.0, 3.0)),
            ..Default::default()
        });
        cx.simulate_modifiers_change(ctrl());
        assert!(
            asking(&workspace, cx),
            "a resize, motion, a wheel turn or a modifier withdrew the question"
        );

        cx.simulate_mouse_down(pane, MouseButton::Right, Modifiers::default());
        cx.simulate_mouse_up(pane, MouseButton::Right, Modifiers::default());
        assert!(!asking(&workspace, cx), "a button press withdraws the question");
        cx.simulate_keystrokes("ctrl-shift-w");
        assert!(
            asking(&workspace, cx),
            "so the next close asks again rather than closing"
        );

        cx.deactivate_window();
        assert!(
            !asking(&workspace, cx),
            "switching away from the window withdraws the question"
        );
        workspace.read_with(cx, |workspace, _| {
            assert_eq!(workspace.tabs.active().unwrap().len(), 2)
        });
    }
```

In `crates/sprite-app/src/workspace/close_gate.rs`, inside `mod tests` (opens at line 310), after `a_close_question_names_the_gesture_that_answers_it` (ends line 430) add:

```rust
    /// The quit shortcut asks like any close, and holding it down cannot
    /// answer its own question.
    #[gpui::test]
    fn a_held_quit_shortcut_cannot_answer_its_own_question(cx: &mut gpui::TestAppContext) {
        use gpui::AppContext;
        QUIT_REQUESTS.with(|requests| requests.set(0));
        let (workspace, cx) = test_workspace(cx);
        workspace.update(cx, |workspace, cx| {
            workspace.tabs = Tabs::new(|_, _| {
                Rc::new(cx.new(|cx| BusyPane {
                    focus: cx.focus_handle(),
                })) as Rc<dyn PaneHandle<Request = SurfaceRequest>>
            });
            workspace.refresh_layout(cx);
        });
        draw_workspace(cx);
        let quit = if cfg!(target_os = "macos") {
            press("q", platform())
        } else {
            press("w", platform())
        };
        cx.simulate_event(gpui::KeyDownEvent {
            keystroke: quit.clone(),
            is_held: false,
        });
        workspace.read_with(cx, |workspace, _| {
            assert!(matches!(workspace.mode, Mode::ConfirmingClose(_)))
        });
        for _ in 0..3 {
            cx.simulate_event(gpui::KeyDownEvent {
                keystroke: quit.clone(),
                is_held: true,
            });
        }
        assert_eq!(
            QUIT_REQUESTS.with(|requests| requests.get()),
            0,
            "a held repeat answered the quit question"
        );
        workspace.read_with(cx, |workspace, _| {
            assert!(matches!(workspace.mode, Mode::ConfirmingClose(_)))
        });
        cx.simulate_event(gpui::KeyDownEvent {
            keystroke: quit,
            is_held: false,
        });
        assert_eq!(QUIT_REQUESTS.with(|requests| requests.get()), 1);
    }
```

- [ ] **Step 2: Run them and confirm they fail.** Three commands, one filter each:
  - `TERM=dumb cargo test -p sprite-app --locked --offline workspace::keymap::tests::a_held` → `a_held_close_key_cannot_answer_its_own_question` panics "a held repeat closed the pane"; `a_held_binding_acts_once_unless_it_is_movement` panics "holding the split key made more than one split".
  - `TERM=dumb cargo test -p sprite-app --locked --offline workspace::keymap::tests::a_close_question` → panics "a button press withdraws the question" (master dismisses only on a left click that refocuses a pane).
  - `TERM=dumb cargo test -p sprite-app --locked --offline workspace::close_gate::tests::a_held_quit` → panics "a held repeat answered the quit question".

- [ ] **Step 3: Implement.**

`crates/sprite-app/src/workspace/keymap.rs` — after the `WorkspaceAction` enum (after line 104), add:

```rust
impl WorkspaceAction {
    /// Whether holding the binding down acts again on every auto-repeat.
    ///
    /// Moving focus, nudging a boundary and stepping the font size are
    /// movements, and a person holds the key to keep them going. Everything
    /// else is one act per press: holding the split key makes one split, and
    /// holding a close key cannot answer the question its own first press
    /// raised. Resetting the size is one act too; repeating it would change
    /// nothing anyway.
    pub(super) fn repeats(self) -> bool {
        matches!(
            self,
            Self::Focus(_) | Self::Resize(_) | Self::FontLarger | Self::FontSmaller
        )
    }
}
```

In `key_down` (lines 209-212), before → after:

```rust
        let Some(action) = action else {
            return;
        };
        cx.stop_propagation();
        match action {
```
```rust
        let Some(action) = action else {
            return;
        };
        // Claimed even when ignored below, so a held binding is never typed
        // into the child either.
        cx.stop_propagation();
        if event.is_held && !action.repeats() {
            return;
        }
        match action {
```

`crates/sprite-app/src/workspace/close_gate.rs`:

Line 1, before → after:
```rust
use super::*;
```
```rust
use super::*;
use crate::confirmation::Confirmation;
```

Replace `may_close` (lines 148-176) with:

```rust
    /// Whether a close may go ahead now, or must be asked about first.
    ///
    /// PRD story 11, and the last thing between a mistyped binding and an hour
    /// of somebody's work. A pane sitting at a shell prompt closes without
    /// ceremony; one running a program asks, and the same gesture again
    /// answers. A pane whose state cannot be determined closes too — a
    /// question nobody can ever resolve is one people learn to dismiss unread.
    pub(super) fn may_close(&mut self, scope: CloseScope, cx: &mut Context<Self>) -> bool {
        if self.stopping {
            return false;
        }
        // Every caller is a fresh gesture: `key_down` drops a held key's
        // repeats for every action that does not repeat, and a click is never
        // held.
        let confirmed = match &mut self.mode {
            Mode::ConfirmingClose(pending) => pending.confirmation.answer(&scope, false),
            _ => false,
        };
        let running = if confirmed {
            Vec::new()
        } else {
            self.running_programs(scope, cx)
        };
        match CloseGate::decide(confirmed, scope, &running) {
            CloseGate::Allow => {
                self.mode = Mode::Idle;
                true
            }
            CloseGate::Ask(pending) => {
                self.mode = Mode::ConfirmingClose(pending);
                cx.notify();
                false
            }
        }
    }
```

Replace `PendingClose` (lines 230-235) with:

```rust
/// A close waiting on a second press.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct PendingClose {
    /// Armed with the scope that asked, so only the same close answers it.
    pub(super) confirmation: Confirmation<CloseScope>,
    pub(super) label: SharedString,
}

impl PendingClose {
    pub(super) fn new(scope: CloseScope, label: SharedString) -> Self {
        let mut confirmation = Confirmation::default();
        confirmation.arm(scope);
        Self {
            confirmation,
            label,
        }
    }
}
```

Replace `CloseGate::decide` (lines 293-308) with:

```rust
impl CloseGate {
    /// `confirmed` is whether this gesture answered a question already asked.
    fn decide(confirmed: bool, scope: CloseScope, running: &[Option<String>]) -> Self {
        if confirmed || running.is_empty() {
            return Self::Allow;
        }
        Self::Ask(PendingClose::new(
            scope,
            display_text(format!(
                "{} — {} to close this {}, Esc to keep it",
                describe_running(running),
                scope.again(),
                scope.noun()
            )),
        ))
    }
}
```

Replace the unit test `consent_is_only_for_the_repeated_scope_and_idle_panes_need_none` (lines 315-341) with:

```rust
    #[test]
    fn consent_is_only_for_the_repeated_scope_and_idle_panes_need_none() {
        let busy = [Some("editor".to_owned()), None];
        let scopes = [
            CloseScope::Pane,
            CloseScope::Tab,
            CloseScope::Window,
            CloseScope::Quit,
        ];
        for scope in scopes {
            assert_eq!(CloseGate::decide(false, scope, &[]), CloseGate::Allow);
            assert_eq!(CloseGate::decide(true, scope, &busy), CloseGate::Allow);
            let CloseGate::Ask(pending) = CloseGate::decide(false, scope, &busy) else {
                panic!("a busy {scope:?} close must ask");
            };
            for answer in scopes {
                let mut confirmation = pending.confirmation.clone();
                assert_eq!(
                    confirmation.answer(&answer, false),
                    answer == scope,
                    "{scope:?} answered by {answer:?}"
                );
            }
            let mut held = pending.confirmation.clone();
            assert!(!held.answer(&scope, true), "an auto-repeat is not an answer");
        }
    }
```

`crates/sprite-app/src/workspace/layout_tests.rs` — both literals (lines 196-199 and 290-293) become constructor calls:

```rust
                workspace.mode = Mode::ConfirmingClose(PendingClose::new(
                    CloseScope::Window,
                    "busy — click close again to close this window, Esc to keep it".into(),
                ));
```
```rust
                workspace.mode = Mode::ConfirmingClose(PendingClose::new(
                    CloseScope::Window,
                    "busy ".repeat(100).into(),
                ));
```

`crates/sprite-app/src/workspace/mod.rs`:

Struct, after line 136 (`_bounds: gpui::Subscription,`) add:
```rust
    /// Withdraws a close question when the window stops being the active one.
    _activation: gpui::Subscription,
```

In `Workspace::new`, after the `let bounds = cx.observe_window_bounds(...)` statement (ends line 275) add:
```rust
        // Switching to another window or application is leaving the pane, and
        // a close question asked before that is not answered by a press made
        // after coming back.
        let activation = cx.observe_window_activation(window, |workspace, window, cx| {
            if !window.is_window_active() {
                workspace.dismiss_pending_close(cx);
            }
        });
```
and in the `Self { ... }` literal, after `_bounds: bounds,` (line 297) add `_activation: activation,`.

In `render` (line 473), before → after:
```rust
            .capture_key_down(cx.listener(Self::key_down))
            .when(strip > 0.0, |element| {
```
```rust
            .capture_key_down(cx.listener(Self::key_down))
            // A button press is a deliberate act, like an unrelated key, so it
            // withdraws a close question wherever it lands. Capture phase, so a
            // pane or divider that handles the press itself cannot hide it.
            .capture_any_mouse_down(cx.listener(
                |workspace, _: &gpui::MouseDownEvent, _window, cx| {
                    workspace.dismiss_pending_close(cx);
                },
            ))
            .when(strip > 0.0, |element| {
```

- [ ] **Step 4: Run tests, confirm pass.**
  `TERM=dumb cargo test -p sprite-app --locked --offline workspace::`
  `TERM=dumb cargo test -p sprite-app --locked --offline confirmation::`
  Expected: all pass, including the pre-existing `modes_cancel_and_close_confirmation_remains_scope_specific`, `modal_keys_reach_only_the_intended_consumer` (other key dismisses: existing behaviour preserved), `window_focus_follows_split_tab_switch_and_close`, and the layout tests.

#### Cycle C: pin the repeat table

- [ ] **Step 1: Write the test.** In `keymap.rs` `mod tests`, after `rename_is_bound_to_ctrl_shift_r` (ends line 457):

```rust
    #[test]
    fn only_movement_and_zoom_repeat_while_held() {
        use WorkspaceAction::*;
        for action in [
            Focus(Direction::Left),
            Focus(Direction::Right),
            Focus(Direction::Up),
            Focus(Direction::Down),
            Resize(Direction::Left),
            Resize(Direction::Right),
            Resize(Direction::Up),
            Resize(Direction::Down),
            FontLarger,
            FontSmaller,
        ] {
            assert!(action.repeats(), "{action:?}");
        }
        for action in [
            SplitRight,
            SplitDown,
            ClosePane,
            FontReset,
            NewTab,
            CloseTab,
            Quit,
            RenameTab,
            NextTab,
            PreviousTab,
            CycleFocus,
        ] {
            assert!(!action.repeats(), "{action:?}");
        }
    }
```

- [ ] **Step 2: Run it.** `TERM=dumb cargo test -p sprite-app --locked --offline workspace::keymap::tests::only_movement_and_zoom_repeat_while_held -- --exact`. **Guard, not red/green:** it pins the table written in Cycle B, so it passes now (against master it only fails to compile, `no method named repeats`). This task's failing-at-master tests are the Cycle A and Cycle B tests.

- [ ] **Step 3: Implement.** Nothing further.

- [ ] **Step 4: Run tests, confirm pass.** `TERM=dumb cargo test -p sprite-app --locked --offline workspace::keymap::`

- [ ] **Step 5: Commit**

```bash
git add crates/sprite-app/src/confirmation.rs crates/sprite-app/src/lib.rs \
  crates/sprite-app/src/workspace/keymap.rs crates/sprite-app/src/workspace/close_gate.rs \
  crates/sprite-app/src/workspace/mod.rs crates/sprite-app/src/workspace/layout_tests.rs
git commit -m "fix(app): held keys no longer answer close confirmations or repeat one-shot bindings" \
  -m "A Confirmation<T> confirms only a fresh press on the same subject. Close, close-tab and quit use it; workspace actions other than focus movement, divider nudge and font zoom ignore key auto-repeat; any button press or window deactivation withdraws a pending close question." \
  -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 6: Unsafe paste through `Confirmation` (BCA-05; R-C3.2, R-C3.4)

**Files:**
- Modify: `crates/sprite-app/src/terminal_view.rs:120-121` (field), `:401` and `:495` (initialisers), `:517` (`Effect::HoldPaste`)
- Modify: `crates/sprite-app/src/terminal_view/input.rs:177-198` (`perform`), `:257-265` (`replace_text_in_range`)
- Modify: `crates/sprite-app/src/terminal_view/render.rs:6-8` (imports), `:430` (render chain), `:450-452` (`perform` call)
- Modify: `crates/sprite-app/src/terminal_events.rs:88-99` (notice constant)
- Test: `crates/sprite-app/src/terminal_view/tests.rs` (top-level `#[gpui::test]` functions; the file is the `terminal_view::tests` module)

**Interfaces:**
- Consumes: `crate::confirmation::Confirmation<T>` — `arm`, `answer`, `disarm`, `is_armed` (Task 5).
- Produces:
  - `TerminalView.unsafe_paste: Confirmation<String>` (replaces `pending_unsafe_paste`).
  - `TerminalView::drop_unsafe_paste(&mut self, cx: &mut Context<Self>)` (`pub(super)`, in `input.rs`) — Task 7 calls it on loss of Pane Focus.
  - `TerminalView::perform(&mut self, shortcut: Shortcut, is_held: bool, cx: &mut Context<Self>)`.
  - `crate::terminal_events::PASTE_HELD_NOTICE: &str`.
  - Test helpers in `terminal_view/tests.rs`: `wait_for_view`, `redraw`, `focus_and_draw`, `hold_unsafe_paste` (Tasks 7 and 8 reuse `redraw`).

Loss of Pane Focus also drops the hold (R-C3.2), but Pane Focus does not exist until Task 7; Task 7 wires that trigger and tests it.

- [ ] **Step 1: Write the failing tests.** In `crates/sprite-app/src/terminal_view/tests.rs`, after `wait_for_bundle` (ends line 16) add the helpers:

```rust
/// Waits briefly for the view itself to reach a state. The worker's answers
/// land through the event task, not as a snapshot, so `wait_for_bundle` cannot
/// see them.
fn wait_for_view(
    view: &gpui::Entity<TerminalView>,
    cx: &mut gpui::VisualTestContext,
    predicate: impl Fn(&TerminalView) -> bool,
) {
    let executor = cx.executor();
    executor.allow_parking();
    executor.block_test(view.condition::<()>(cx, |view, _| predicate(view)));
}

fn redraw(cx: &mut gpui::VisualTestContext) {
    cx.update(|window, cx| {
        window.refresh();
        window.draw(cx).clear();
    });
    cx.run_until_parked();
}

/// Gives the view the keyboard in an active window, and draws so that key and
/// mouse events find it.
fn focus_and_draw(view: &gpui::Entity<TerminalView>, cx: &mut gpui::VisualTestContext) {
    view.update_in(cx, |view, window, _| window.focus(&view.focus));
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    redraw(cx);
}

/// Delivers the worker's refusal of a multi-line paste exactly as the event
/// task delivers it.
fn hold_unsafe_paste(view: &gpui::Entity<TerminalView>, cx: &mut gpui::VisualTestContext, text: &str) {
    view.update(cx, |view, cx| {
        let decision = crate::terminal_events::decide(Ok(
            sprite_term::TerminalEvent::UnsafePaste(text.to_owned()),
        ));
        for effect in decision.effects {
            view.apply(effect, cx);
        }
        cx.notify();
    });
}
```

At the end of the file add:

```rust
/// A held paste is confirmed by asking for the same text again. The clipboard
/// is read on that second request: a clipboard that changed in between means
/// the person is pasting something else, which is checked on its own merits,
/// and an empty or unreadable one pastes nothing. Only the fresh text and the
/// text confirmed at the end may reach the child.
#[gpui::test]
fn a_confirming_paste_rereads_the_clipboard(cx: &mut gpui::TestAppContext) {
    use gpui::{KeyDownEvent, Keystroke};
    let expected = "freshthree\rfour";
    let script = format!(
        "stty raw -echo; printf 'READY\\r\\n'; dd bs=1 count={} status=none | od -An -tx1 -v | tr -d ' \\n'; printf '\\r\\nDONE\\r\\n'; sleep 30",
        expected.len()
    );
    let settings = crate::config::Settings::default();
    cx.set_global(crate::config::ActiveSettings(settings.clone()));
    cx.set_global(crate::tokens::TokenRegistry::new(&settings.colors));
    let (sender, _exits) = async_channel::unbounded();
    let (view, cx) = cx.add_window_view(|window, cx| {
        TerminalView::new(
            Some(vec!["/bin/sh".into(), "-c".into(), script.into()]),
            settings,
            Vec::new(),
            None,
            PaneExit {
                sender,
                identity: (crate::tabs::TabId(1), crate::pane_tree::PaneId(1)),
            },
            window,
            cx,
        )
    });
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        wait_for_bundle(&view, cx, |b| {
            b.pane.rows.iter().any(|r| r.text.contains("READY"))
        });
        focus_and_draw(&view, cx);
        let paste = |cx: &mut gpui::VisualTestContext, text: &str| {
            cx.write_to_clipboard(ClipboardItem::new_string(text.to_owned()));
            cx.simulate_keystrokes("ctrl-shift-v");
        };
        let held = |view: &gpui::Entity<TerminalView>, cx: &mut gpui::VisualTestContext| {
            view.read_with(cx, |view, _| view.unsafe_paste.is_armed())
        };

        // The child is not bracketing, so two lines would run as commands.
        paste(cx, "one\ntwo");
        wait_for_view(&view, cx, |view| view.unsafe_paste.is_armed());

        // The clipboard changed before the second request: the old text is
        // dropped, and the new one-line text is safe, so it is simply pasted.
        paste(cx, "fresh");
        assert!(!held(&view, cx), "a changed clipboard drops the held text");

        paste(cx, "three\nfour");
        wait_for_view(&view, cx, |view| view.unsafe_paste.is_armed());

        // An empty clipboard pastes nothing and drops the hold with its question.
        paste(cx, "");
        assert!(!held(&view, cx), "an empty clipboard drops the held text");
        view.read_with(cx, |view, _| {
            assert!(
                view.status
                    .as_ref()
                    .is_none_or(|status| !status.contains("paste held")),
                "the question went with it: {:?}",
                view.status
            )
        });

        // So the same text is asked about again, not sent.
        paste(cx, "three\nfour");
        wait_for_view(&view, cx, |view| view.unsafe_paste.is_armed());

        // The paste key auto-repeating is not the answer.
        cx.simulate_event(KeyDownEvent {
            keystroke: Keystroke::parse("ctrl-shift-v").unwrap(),
            is_held: true,
        });
        assert!(held(&view, cx), "an auto-repeat answered the question");

        // A fresh request with the same text on the clipboard is.
        cx.simulate_keystrokes("ctrl-shift-v");
        assert!(!held(&view, cx));

        let bundle = wait_for_bundle(&view, cx, |b| {
            b.pane.rows.iter().any(|r| r.text.contains("DONE"))
        });
        let text: String = bundle.pane.rows.iter().map(|r| r.text.trim()).collect();
        let actual = text
            .split("READY")
            .nth(1)
            .unwrap()
            .split("DONE")
            .next()
            .unwrap();
        let hex: String = expected.bytes().map(|byte| format!("{byte:02x}")).collect();
        assert_eq!(
            actual, hex,
            "only the fresh text and the confirmed text reach the child"
        );
    }));
    if let Some(cleanup) = view.update(cx, |view, _| view.begin_shutdown()) {
        cleanup.wait().unwrap();
    }
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}

/// Deliberate input withdraws a held paste: a key, a button press, an input
/// method commit. Pointer motion, a wheel turn, a modifier on its own, a
/// resize and the paste key auto-repeating are not decisions and leave it
/// standing.
#[gpui::test]
fn deliberate_input_drops_a_held_paste_and_passive_input_does_not(
    cx: &mut gpui::TestAppContext,
) {
    use gpui::{
        ElementInputHandler, InputHandler, KeyDownEvent, Keystroke, Modifiers, MouseButton,
        ScrollDelta, ScrollWheelEvent,
    };
    let settings = crate::config::Settings::default();
    cx.set_global(crate::config::ActiveSettings(settings.clone()));
    cx.set_global(crate::tokens::TokenRegistry::new(&settings.colors));
    let (view, cx) = cx.add_window_view(|window, cx| {
        TerminalView::failed("pane failed".into(), ".SystemUIFont".into(), window, cx)
    });
    focus_and_draw(&view, cx);
    let held = |view: &gpui::Entity<TerminalView>, cx: &mut gpui::VisualTestContext| {
        view.read_with(cx, |view, _| view.unsafe_paste.is_armed())
    };
    let inside = point(px(40.0), px(40.0));

    hold_unsafe_paste(&view, cx, "a\nb");
    cx.simulate_mouse_move(inside, None, Modifiers::default());
    cx.simulate_event(ScrollWheelEvent {
        position: inside,
        delta: ScrollDelta::Lines(point(0.0, -2.0)),
        ..Default::default()
    });
    cx.simulate_modifiers_change(Modifiers {
        shift: true,
        ..Default::default()
    });
    cx.simulate_resize(gpui::size(px(700.0), px(500.0)));
    redraw(cx);
    cx.simulate_event(KeyDownEvent {
        keystroke: Keystroke::parse("ctrl-shift-v").unwrap(),
        is_held: true,
    });
    assert!(
        held(&view, cx),
        "motion, a wheel turn, a modifier, a resize or an auto-repeat dropped the paste"
    );
    view.read_with(cx, |view, _| {
        assert!(view
            .status
            .as_ref()
            .is_some_and(|status| status.contains("paste held")))
    });

    cx.simulate_mouse_down(inside, MouseButton::Right, Modifiers::default());
    assert!(!held(&view, cx), "a button press drops it");
    view.read_with(cx, |view, _| {
        assert!(
            view.status
                .as_ref()
                .is_none_or(|status| !status.contains("paste held")),
            "and the question with it"
        )
    });

    hold_unsafe_paste(&view, cx, "a\nb");
    cx.simulate_keystrokes("x");
    assert!(!held(&view, cx), "a key drops it");

    hold_unsafe_paste(&view, cx, "a\nb");
    let mut handler = ElementInputHandler::new(gpui::Bounds::default(), view.clone());
    cx.update(|window, cx| handler.replace_text_in_range(None, "é", window, cx));
    assert!(!held(&view, cx), "an input method commit drops it");
}
```

- [ ] **Step 2: Run them and confirm they fail.**
  `TERM=dumb cargo test -p sprite-app --locked --offline terminal_view::tests::a_confirming_paste_rereads_the_clipboard -- --exact`
  Expected: compile error `no field `unsafe_paste` on type `TerminalView``. (Behaviourally, master sends `PasteConfirmed("one\ntwo")` at the second request without reading the clipboard, so the child would receive `one\rtwo…` instead of `fresh…`.)

- [ ] **Step 3: Implement.**

`crates/sprite-app/src/terminal_events.rs` — above `pub(crate) fn decide` (before line 50) add:

```rust
/// How the status line begins while a paste is held for confirmation, so the
/// view can clear that line, and only that line, when the hold is dropped.
pub(crate) const PASTE_HELD_NOTICE: &str = "[paste held:";
```

and in the `UnsafePaste` arm (lines 92-98) change the format string to use it (output unchanged):

```rust
            effects.push(Effect::Status(
                format!(
                    "{PASTE_HELD_NOTICE} {lines} lines would run as commands — \
                     repeat the paste shortcut to paste anyway]"
                )
                .into(),
            ));
```

`crates/sprite-app/src/terminal_view.rs`:
- Lines 120-121 become:
```rust
    /// A paste withheld as unsafe, awaiting a second explicit request for the
    /// same text.
    unsafe_paste: crate::confirmation::Confirmation<String>,
```
- Line 401 and line 495: `pending_unsafe_paste: None,` → `unsafe_paste: Default::default(),`
- Line 517: `Effect::HoldPaste(text) => self.pending_unsafe_paste = Some(text),` → `Effect::HoldPaste(text) => self.unsafe_paste.arm(text),`

`crates/sprite-app/src/terminal_view/input.rs` — replace `perform` (lines 177-198) with:

```rust
    pub(super) fn perform(&mut self, shortcut: Shortcut, is_held: bool, cx: &mut Context<Self>) {
        match shortcut {
            Shortcut::Copy => self.send(TerminalCommand::CopySelection),
            Shortcut::Paste => {
                // Read only on an explicit request, never speculatively. A
                // clipboard that cannot be read counts as empty.
                let text = cx
                    .read_from_clipboard()
                    .and_then(|item| item.text())
                    .unwrap_or_default();
                if self.unsafe_paste.is_armed() {
                    // A second request for the same text confirms the paste
                    // held back. The decision was made about that text, so it
                    // is compared with what the clipboard holds now rather
                    // than assumed.
                    if self.unsafe_paste.answer(&text, is_held) {
                        self.clear_paste_notice();
                        self.send(TerminalCommand::PasteConfirmed(text));
                        cx.notify();
                        return;
                    }
                    // The paste key auto-repeating from the request that was
                    // refused: neither an answer nor a new paste.
                    if is_held {
                        return;
                    }
                    // The clipboard changed or could not be read, so what is
                    // being pasted now is not what was asked about. The old
                    // text is dropped and the new text, if any, is checked
                    // from scratch.
                    self.drop_unsafe_paste(cx);
                }
                if !text.is_empty() {
                    self.send(TerminalCommand::Paste(text));
                }
            }
            Shortcut::DeleteLine => self.send(TerminalCommand::Input(vec![0x15])),
        }
    }

    /// Withdraws a paste held back as unsafe, with the line that asked about
    /// it. Called for deliberate input only — keys, button presses, input
    /// method commits, leaving the pane — never from `send`, which also
    /// carries hover lookups and resizes.
    pub(super) fn drop_unsafe_paste(&mut self, cx: &mut Context<Self>) {
        if !self.unsafe_paste.is_armed() {
            return;
        }
        self.unsafe_paste.disarm();
        self.clear_paste_notice();
        cx.notify();
    }

    /// Clears the status line only while it is still the held-paste question;
    /// a message that has replaced it since is left alone.
    fn clear_paste_notice(&mut self) {
        if self.status.as_ref().is_some_and(|status| {
            status.starts_with(crate::terminal_events::PASTE_HELD_NOTICE)
        }) {
            self.status = None;
        }
    }
```

In `replace_text_in_range` (lines 257-265), before → after:
```rust
        self.preedit = None;
        if !text.is_empty() {
            if let Some(id) = self.focused_surface(window).map(|surface| surface.id())
```
```rust
        self.preedit = None;
        if !text.is_empty() {
            // A committed composition is typing, and typing withdraws a held
            // paste just as a key press does.
            self.drop_unsafe_paste(cx);
            if let Some(id) = self.focused_surface(window).map(|surface| surface.id())
```

`crates/sprite-app/src/terminal_view/render.rs`:
- Imports (lines 6-8) become:
```rust
use super::input::{
    Drag, LinkClickBehavior, Shortcut, application_shortcut, dropped_paths_text,
    link_click_behavior,
};
```
- Line 451: `view.perform(shortcut, cx);` → `view.perform(shortcut, event.is_held, cx);`
- After `.track_focus(&self.focus)` (line 430) insert:
```rust
            // Deliberate input withdraws a held paste: any key but the paste
            // that would answer it, and any button press. Capture phase, so a
            // hosted Surface that handles the event itself cannot hide it from
            // the pane. Pointer motion, wheel turns and modifier changes are
            // not decisions and leave the question standing; GPUI reports a
            // modifier on its own as a modifiers change, never as a key down.
            .capture_key_down(cx.listener(|view, event: &KeyDownEvent, _window, cx| {
                if application_shortcut(&event.keystroke) != Some(Shortcut::Paste) {
                    view.drop_unsafe_paste(cx);
                }
            }))
            .capture_any_mouse_down(cx.listener(|view, _: &MouseDownEvent, _window, cx| {
                view.drop_unsafe_paste(cx);
            }))
```

- [ ] **Step 4: Run tests, confirm pass.**
  `TERM=dumb cargo test -p sprite-app --locked --offline terminal_view::tests::a_confirming_paste_rereads_the_clipboard -- --exact`
  `TERM=dumb cargo test -p sprite-app --locked --offline terminal_view::tests::deliberate_input_drops_a_held_paste_and_passive_input_does_not -- --exact`
  `TERM=dumb cargo test -p sprite-app --locked --offline terminal_view::`
  `TERM=dumb cargo test -p sprite-app --locked --offline terminal_events::`
  Expected: all pass (including `a_held_paste_explains_itself_and_is_kept`, whose text is unchanged).

- [ ] **Step 5: Commit**

```bash
git add crates/sprite-app/src/terminal_events.rs crates/sprite-app/src/terminal_view.rs \
  crates/sprite-app/src/terminal_view/input.rs crates/sprite-app/src/terminal_view/render.rs \
  crates/sprite-app/src/terminal_view/tests.rs
git commit -m "fix(app): a held unsafe paste is confirmed only by the same clipboard text" \
  -m "The confirming paste re-reads the clipboard: the same text is pasted, changed text goes through the normal safety check, an empty or unreadable clipboard pastes nothing. Keys, button presses and input method commits drop the hold; motion, wheel, modifiers, resizes and key auto-repeat do not." \
  -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 7: Pane Focus reaches the worker (BCA-01; R-C4.1, R-C4.2 for OSC 52 and focus reports; R-C3.2 focus-loss trigger)

**Files:**
- Modify: `crates/sprite-app/src/terminal_view.rs:50` and `:126` (fields), `:362-411` (`new`), `:436-505` (`failed`), `:567-584` (`begin_shutdown`), `:609-649` (`spawn_retry`), new methods after `apply` (after line 564)
- Modify: `crates/sprite-app/src/workspace/test_support.rs:1` (import), end of file (helper)
- Test: `crates/sprite-term/tests/clipboard.rs` (append before line 189), `crates/sprite-term/tests/paste.rs` (after `focus_is_reported_only_when_the_child_asks`, ends line 178), `crates/sprite-app/src/terminal_view/tests.rs` (top level), `crates/sprite-app/src/workspace/keymap.rs` (`mod tests`)

**Interfaces:**
- Consumes: `TerminalView::drop_unsafe_paste(&mut self, cx)` and `TerminalView.unsafe_paste` (Task 6); test helpers `redraw`, `wait_for_view`, `hold_unsafe_paste` (Task 6).
- Produces:
  - `TerminalView::pane_focused(&self) -> bool` (`pub(crate)`).
  - `TerminalView::refresh_pane_focus(&mut self, window: &mut Window, cx: &mut Context<Self>)` (private; Task 8 edits it).
  - `TerminalView::admit_focus(&mut self, focused: bool)` and fields `pending_focus: Option<bool>`, `told_focus: bool` (Command Admission, retried by `spawn_retry`).
  - `TerminalView::failed` now takes `window: &mut Window`.
  - Test helpers in `terminal_view/tests.rs`: `struct TwoPanes`, `fn two_panes(...)`; in `workspace/test_support.rs`: `fn terminal_view(workspace, pane, cx) -> Entity<TerminalView>`.

Pane Focus (glossary): the view's focus handle *contains* the window's focus and the window is active. GPUI's focus events (`vendor/gpui/src/window.rs:1941-1969`) already treat an inactive window as having an empty focus path, so `cx.on_focus_in` / `cx.on_focus_out` fire on window switches too; `cx.observe_window_activation` is added so that deactivation is reported immediately rather than at the next frame.

#### Cycle A: GUARD tests at the worker seam (pass at master by design)

**Guard, not red/green.** The worker already honours `Focus`; these pin the two transitions BCA-01 depends on and that no existing test covers (focused→unfocused clipboard denial; focus-out report). They pass at master. This task's failing-at-master tests are in Cycles B and C.

- [ ] **Step 1: Write the tests.** `crates/sprite-term/tests/clipboard.rs`, before line 189 (`// Keeps the unused-import warning honest…`):

```rust
/// Focus is a state the pane moves in and out of, not a grant made once: a
/// pane that had focus and lost it may not write the clipboard.
#[test]
fn losing_focus_withdraws_the_clipboard() {
    let sprite_term::Spawned {
        mut session,
        events,
        snapshots,
    } = session(
        "stty -echo; read _; printf '\\033]52;c;b25l\\007'; printf 'ONE\\n'; \
         read _; printf '\\033]52;c;dHdv\\007'; printf 'TWO\\n'; sleep 30",
    );
    let events = EventPump::new(events);
    let snapshots = SnapshotPump::new(snapshots);
    events.expect_ready();

    session.send(TerminalCommand::Focus(true)).expect("focus the pane");
    session
        .send(TerminalCommand::Input(b"\n".to_vec()))
        .expect("release the first write");
    snapshots.wait_for("the first marker", |bundle| pane_text(bundle).contains("ONE"));
    assert_eq!(events.try_next_clipboard().as_deref(), Some("one"));

    session.send(TerminalCommand::Focus(false)).expect("unfocus the pane");
    session
        .send(TerminalCommand::Input(b"\n".to_vec()))
        .expect("release the second write");
    snapshots.wait_for("the second marker", |bundle| pane_text(bundle).contains("TWO"));
    assert_eq!(
        events.try_next_clipboard(),
        None,
        "a pane that lost focus is denied"
    );
}
```

`crates/sprite-term/tests/paste.rs`, after `focus_is_reported_only_when_the_child_asks`:

```rust
/// Losing focus is reported as well, as CSI O, once the child has asked.
#[test]
fn focus_loss_is_reported_after_focus_gain() {
    let sprite_term::Spawned {
        mut session,
        events,
        snapshots,
    } = session(&hex_reader("printf '\\033[?1004h';", 6));
    let events = EventPump::new(events);
    let snapshots = SnapshotPump::new(snapshots);
    events.expect_ready();
    snapshots.wait_for("ready", |b| pane_text(b).contains("READY"));

    session.send(TerminalCommand::Focus(true)).expect("report focus in");
    session.send(TerminalCommand::Focus(false)).expect("report focus out");

    let bundle = snapshots.wait_for("both reports", |b| {
        pane_text(b).contains("1b 5b 49 1b 5b 4f")
    });
    assert!(
        pane_text(&bundle).contains("1b 5b 49 1b 5b 4f"),
        "CSI I then CSI O"
    );
}
```

- [ ] **Step 2: Run them.**
  `TERM=dumb cargo test -p sprite-term --test clipboard --locked --offline losing_focus_withdraws_the_clipboard -- --exact`
  `TERM=dumb cargo test -p sprite-term --test paste --locked --offline focus_loss_is_reported_after_focus_gain -- --exact`
  Expected: both pass at master (seam already correct). If either fails, stop: the worker, not the app, is broken, and that is outside this task.

- [ ] **Step 3: Implement.** Nothing.

#### Cycle B: the application tells the worker

- [ ] **Step 1: Write the failing tests.**

`crates/sprite-app/src/terminal_view/tests.rs` — after the Task 6 helpers add:

```rust
/// Two running panes side by side in one window, so the keyboard can move
/// between them as the workspace moves it.
struct TwoPanes {
    left: gpui::Entity<TerminalView>,
    right: gpui::Entity<TerminalView>,
}

impl gpui::Render for TwoPanes {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl gpui::IntoElement {
        use gpui::prelude::*;
        gpui::div()
            .flex()
            .flex_row()
            .size_full()
            .child(gpui::div().w(px(400.0)).h(px(300.0)).child(self.left.clone()))
            .child(gpui::div().w(px(400.0)).h(px(300.0)).child(self.right.clone()))
    }
}

fn two_panes<'a>(
    cx: &'a mut gpui::TestAppContext,
    settings: &crate::config::Settings,
    script: &str,
) -> (
    gpui::Entity<TerminalView>,
    gpui::Entity<TerminalView>,
    &'a mut gpui::VisualTestContext,
) {
    use gpui::AppContext as _;
    let (sender, _exits) = async_channel::unbounded();
    let (root, cx) = cx.add_window_view(|window, cx| {
        let mut pane = |id: usize| {
            cx.new(|cx| {
                TerminalView::new(
                    Some(vec!["/bin/sh".into(), "-c".into(), script.into()]),
                    settings.clone(),
                    Vec::new(),
                    None,
                    PaneExit {
                        sender: sender.clone(),
                        identity: (crate::tabs::TabId(1), crate::pane_tree::PaneId(id)),
                    },
                    window,
                    cx,
                )
            })
        };
        TwoPanes {
            left: pane(1),
            right: pane(2),
        }
    });
    let (left, right) = root.read_with(cx, |root, _| (root.left.clone(), root.right.clone()));
    (left, right, cx)
}
```

At the end of the file add:

```rust
/// Pane Focus is the keyboard *and* the active window. Each change reaches the
/// child once, as a focus report, and nothing is reported when nothing
/// changed. The child turns reporting on only after the pane's opening
/// `Focus(false)` has been handled, then prints each three-byte report on a
/// line of its own; a sentinel behind the reports proves nothing else came.
#[gpui::test]
fn pane_focus_follows_the_keyboard_and_the_window_and_reaches_the_child(
    cx: &mut gpui::TestAppContext,
) {
    let settings = crate::config::Settings::default();
    cx.set_global(crate::config::ActiveSettings(settings.clone()));
    cx.set_global(crate::tokens::TokenRegistry::new(&settings.colors));
    let script = "stty raw -echo; dd bs=1 count=1 status=none >/dev/null; \
                  printf '\\033[?1004hREADY\\r\\n'; \
                  while :; do dd bs=1 count=3 status=none | od -An -tx1 | tr -d ' \\n'; printf '\\r\\n'; done";
    let (left, right, cx) = two_panes(cx, &settings, script);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        for view in [&left, &right] {
            view.update(cx, |view, _| view.send(TerminalCommand::Input(b"g".to_vec())));
            wait_for_bundle(view, cx, |b| {
                b.pane.rows.iter().any(|r| r.text.contains("READY"))
            });
        }
        let focused = |cx: &mut gpui::VisualTestContext| {
            (
                left.read_with(cx, |view, _| view.pane_focused()),
                right.read_with(cx, |view, _| view.pane_focused()),
            )
        };

        // The keyboard alone is not Pane Focus while the window is behind others.
        left.update_in(cx, |view, window, _| window.focus(&view.focus));
        redraw(cx);
        assert_eq!(focused(cx), (false, false));
        cx.update(|window, _| window.activate_window());
        cx.run_until_parked();
        assert_eq!(focused(cx), (true, false));
        redraw(cx);

        // Moving the keyboard moves Pane Focus, and the pane left behind drops
        // the paste it was holding.
        hold_unsafe_paste(&left, cx, "a\nb");
        right.update_in(cx, |view, window, _| window.focus(&view.focus));
        redraw(cx);
        assert_eq!(focused(cx), (false, true));
        assert!(
            !left.read_with(cx, |view, _| view.unsafe_paste.is_armed()),
            "leaving the pane drops its held paste"
        );

        // Frames with nothing changed report nothing.
        redraw(cx);
        redraw(cx);

        cx.deactivate_window();
        assert_eq!(focused(cx), (false, false), "a background window has no Pane Focus");
        redraw(cx);
        left.update_in(cx, |view, window, _| window.focus(&view.focus));
        redraw(cx);
        assert_eq!(focused(cx), (false, false));

        let reports = |view: &gpui::Entity<TerminalView>, cx: &mut gpui::VisualTestContext| {
            view.update(cx, |view, _| view.send(TerminalCommand::Input(b"ZZZ".to_vec())));
            let bundle = wait_for_bundle(view, cx, |b| {
                b.pane.rows.iter().any(|r| r.text.contains("5a5a5a"))
            });
            bundle
                .pane
                .rows
                .iter()
                .map(|r| r.text.trim().to_owned())
                .skip_while(|line| !line.contains("READY"))
                .skip(1)
                .filter(|line| !line.is_empty())
                .collect::<Vec<_>>()
        };
        assert_eq!(reports(&left, cx), ["1b5b49", "1b5b4f", "5a5a5a"]);
        assert_eq!(reports(&right, cx), ["1b5b49", "1b5b4f", "5a5a5a"]);
    }));
    for view in [&left, &right] {
        if let Some(cleanup) = view.update(cx, |view, _| view.begin_shutdown()) {
            cleanup.wait().unwrap();
        }
    }
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}

/// OSC 52 is honoured only from the pane with Pane Focus: refused while the
/// pane is unfocused, accepted once it holds the keyboard in the active window.
#[gpui::test]
fn only_a_pane_with_pane_focus_may_write_the_clipboard(cx: &mut gpui::TestAppContext) {
    let settings = crate::config::Settings::default();
    cx.set_global(crate::config::ActiveSettings(settings.clone()));
    cx.set_global(crate::tokens::TokenRegistry::new(&settings.colors));
    let script = "stty -echo; read _; printf '\\033]52;c;b25l\\007'; printf 'ONE\\n'; \
                  read _; printf '\\033]52;c;dHdv\\007'; printf 'TWO\\n'; sleep 30";
    let (sender, _exits) = async_channel::unbounded();
    let (view, cx) = cx.add_window_view(|window, cx| {
        TerminalView::new(
            Some(vec!["/bin/sh".into(), "-c".into(), script.into()]),
            settings,
            Vec::new(),
            None,
            PaneExit {
                sender,
                identity: (crate::tabs::TabId(1), crate::pane_tree::PaneId(1)),
            },
            window,
            cx,
        )
    });
    cx.write_to_clipboard(ClipboardItem::new_string("before".to_owned()));
    let clipboard = |cx: &mut gpui::VisualTestContext| {
        cx.read_from_clipboard().and_then(|item| item.text())
    };
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        wait_for_bundle(&view, cx, |_| true);
        view.update(cx, |view, _| view.send(TerminalCommand::Input(b"\n".to_vec())));
        wait_for_bundle(&view, cx, |b| b.pane.rows.iter().any(|r| r.text.contains("ONE")));
        cx.run_until_parked();
        assert_eq!(clipboard(cx).as_deref(), Some("before"), "an unfocused pane is refused");

        view.update_in(cx, |view, window, _| window.focus(&view.focus));
        cx.update(|window, _| window.activate_window());
        cx.run_until_parked();
        redraw(cx);
        assert!(view.read_with(cx, |view, _| view.pane_focused()));
        view.update(cx, |view, _| view.send(TerminalCommand::Input(b"\n".to_vec())));
        wait_for_bundle(&view, cx, |b| b.pane.rows.iter().any(|r| r.text.contains("TWO")));
        cx.run_until_parked();
        assert_eq!(clipboard(cx).as_deref(), Some("two"), "the focused pane is honoured");
    }));
    if let Some(cleanup) = view.update(cx, |view, _| view.begin_shutdown()) {
        cleanup.wait().unwrap();
    }
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}
```

`crates/sprite-app/src/workspace/test_support.rs` — line 1 becomes:
```rust
use gpui::{Keystroke, Modifiers};
use sprite_pane::PaneHandle;
```
and append:
```rust
/// The terminal view occupying `pane`, for tests that look inside it.
pub(super) fn terminal_view(
    workspace: &gpui::Entity<super::Workspace>,
    pane: crate::pane_tree::PaneId,
    cx: &mut gpui::VisualTestContext,
) -> gpui::Entity<crate::terminal_view::TerminalView> {
    workspace.read_with(cx, |workspace, _| {
        workspace
            .tabs
            .all_panes()
            .into_iter()
            .find(|(_, id, _)| *id == pane)
            .map(|(_, _, handle)| handle.view())
            .expect("the pane is in the window")
            .downcast::<crate::terminal_view::TerminalView>()
            .ok()
            .expect("a terminal pane")
    })
}
```

`crates/sprite-app/src/workspace/keymap.rs` `mod tests` — after `window_focus_follows_split_tab_switch_and_close` (ends line 382):

```rust
    /// The workspace's focus routing and the window's activation together
    /// decide which terminal has Pane Focus.
    #[gpui::test]
    fn pane_focus_follows_the_focused_pane_and_the_active_window(
        cx: &mut gpui::TestAppContext,
    ) {
        let (workspace, cx) = test_workspace(cx);
        cx.update(|window, _| window.activate_window());
        cx.run_until_parked();
        draw_workspace(cx);
        cx.simulate_keystrokes("ctrl-shift-d");
        // The first frame moves the keyboard; the second reports the move.
        draw_workspace(cx);
        draw_workspace(cx);
        let first = terminal_view(&workspace, PaneId(0), cx);
        let second = terminal_view(&workspace, PaneId(1), cx);
        let focused = |cx: &mut gpui::VisualTestContext| {
            (
                first.read_with(cx, |view, _| view.pane_focused()),
                second.read_with(cx, |view, _| view.pane_focused()),
            )
        };
        assert_eq!(focused(cx), (false, true), "the split took the keyboard");
        workspace.update(cx, |workspace, cx| workspace.focus_pane(PaneId(0), cx));
        draw_workspace(cx);
        draw_workspace(cx);
        assert_eq!(focused(cx), (true, false));
        cx.deactivate_window();
        assert_eq!(focused(cx), (false, false), "a background window has no Pane Focus");
        cx.update(|window, _| window.activate_window());
        cx.run_until_parked();
        assert_eq!(focused(cx), (true, false), "and coming back restores it");
    }
```

- [ ] **Step 2: Run them and confirm they fail.**
  `TERM=dumb cargo test -p sprite-app --locked --offline terminal_view::tests::pane_focus_follows_the_keyboard_and_the_window_and_reaches_the_child -- --exact`
  Expected: compile error `no method named `pane_focused` found for struct `TerminalView``. (Behaviourally, master never sends `Focus`, so the child's report list would be `["5a5a5a"]` and the clipboard would stay `"before"`.)

- [ ] **Step 3: Implement.** In `crates/sprite-app/src/terminal_view.rs`:

Fields — after `blink_on: bool,` (line 126) add:
```rust
    /// Whether this pane has Pane Focus: it holds the keyboard in its window,
    /// and that window is the active one. The worker is told every change,
    /// because the same fact decides whether the child may write the clipboard
    /// and whether it hears focus reports.
    pane_focused: bool,
    /// Keeps the focus and window-activation observers alive.
    _pane_focus: [gpui::Subscription; 3],
```

`new` — before → after (lines 362-363):
```rust
        let (retry_task, retry_wake) = Self::spawn_retry(window, cx);
        Self {
```
```rust
        let (retry_task, retry_wake) = Self::spawn_retry(window, cx);
        let focus = cx.focus_handle();
        let pane_focus = Self::observe_pane_focus(&focus, window, cx);
        let mut view = Self {
```
In the same literal: `focus: cx.focus_handle(),` (line 383) → `focus,`; after `blink_on: true,` (line 403) add:
```rust
            pane_focused: false,
            _pane_focus: pane_focus,
```
and the literal's end (lines 409-411) before → after:
```rust
            _settings: settings_subscription,
        }
    }
```
```rust
            _settings: settings_subscription,
        };
        // The worker starts out denying focus. Saying so explicitly means the
        // two sides agree from the first byte, and every later message is a
        // change the worker hears exactly once.
        view.send(TerminalCommand::Focus(false));
        view
    }
```

`failed` — signature (lines 436-441) `window: &Window,` → `window: &mut Window,`. After the `let settings_subscription = …;` statement (ends line 452) add:
```rust
        let focus = cx.focus_handle();
        let pane_focus = Self::observe_pane_focus(&focus, window, cx);
```
In its literal, `focus: cx.focus_handle(),` (line 472) → `focus,`; after `blink_on: true,` (line 497) add:
```rust
            pane_focused: false,
            _pane_focus: pane_focus,
```
(A failed pane has no worker to tell, but it still tracks Pane Focus: a grid Surface hosted in it blinks by the same rule, Task 8.)

New methods — after `apply` (after line 564):
```rust
    /// Watches both halves of Pane Focus.
    ///
    /// GPUI's focus events already treat an inactive window as holding no
    /// focus, so focus-in and focus-out cover a switch between windows as well
    /// as between panes. Activation is watched too, because focus events wait
    /// for the next frame, and a pane whose window has gone to the background
    /// should stop taking the clipboard now rather than then.
    fn observe_pane_focus(
        focus: &FocusHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> [gpui::Subscription; 3] {
        [
            cx.on_focus_in(focus, window, |view, window, cx| {
                view.refresh_pane_focus(window, cx)
            }),
            cx.on_focus_out(focus, window, |view, _, window, cx| {
                view.refresh_pane_focus(window, cx)
            }),
            cx.observe_window_activation(window, |view, window, cx| {
                view.refresh_pane_focus(window, cx)
            }),
        ]
    }

    /// Recomputes Pane Focus from the window, and reports a change.
    ///
    /// The terminal's handle *containing* the focus is enough: a Surface the
    /// pane hosts is part of the pane, and a person typing into one is still
    /// working here.
    fn refresh_pane_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let focused = window.is_window_active() && self.focus.contains_focused(window, cx);
        if focused == self.pane_focused() {
            return;
        }
        self.pane_focused = focused;
        self.send(TerminalCommand::Focus(focused));
        if !focused {
            // Leaving the pane is a decision too: a paste held here is not
            // answered by a paste made after coming back.
            self.drop_unsafe_paste(cx);
        }
        cx.notify();
    }

    /// Whether this pane has Pane Focus.
    pub(crate) fn pane_focused(&self) -> bool {
        self.pane_focused
    }
```

- [ ] **Step 4: Run tests, confirm pass.**
  `TERM=dumb cargo test -p sprite-app --locked --offline terminal_view::tests::pane_focus_follows_the_keyboard_and_the_window_and_reaches_the_child -- --exact`
  `TERM=dumb cargo test -p sprite-app --locked --offline terminal_view::tests::only_a_pane_with_pane_focus_may_write_the_clipboard -- --exact`
  `TERM=dumb cargo test -p sprite-app --locked --offline workspace::keymap::tests::pane_focus_follows_the_focused_pane_and_the_active_window -- --exact`
  `TERM=dumb cargo test -p sprite-app --locked --offline terminal_view::`
  `TERM=dumb cargo test -p sprite-app --locked --offline workspace::`
  `TERM=dumb cargo test -p sprite-term --test clipboard --test paste --locked --offline`
  Expected: all pass, including the Task 6 tests (they focus and activate before holding, so no focus loss intervenes) and the Surface focus tests in `terminal_view::surfaces`.

#### Cycle C: a Focus refused under a full queue is delivered once admission recovers

`submit` refuses a command when the worker queue is full (`terminal_view.rs:590-607`) and wakes `spawn_retry` (`:609-649`), which retries `pending_settings` and `pending_resize`, the latest desired values, every 50 ms until they are admitted, or drops them once admission closes (`geometry.rs:140-147` `admit_resize` is the model). Focus joins that pattern as `pending_focus: Option<bool>`. The test reuses the real event-pressure harness of `settings_callback_recovers_latest_values_after_real_event_pressure` / `settings_callback_pressure_child` (`terminal_view/tests.rs:831-1010`): a parent test re-runs the test binary on one gated child test so that pausing the UI thread cannot affect other tests.

- [ ] **Step 1: Write the failing test.** In `crates/sprite-app/src/terminal_view/tests.rs`, after `settings_callback_pressure_child` (ends line 1010), add:

```rust
#[test]
fn focus_refused_under_a_full_queue_is_delivered_once_admission_recovers() {
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "terminal_view::tests::focus_admission_pressure_child",
            "--nocapture",
        ])
        .env("SPRITE_FOCUS_PRESSURE_CHILD", "1")
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(12);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            let output = child.wait_with_output().unwrap();
            assert!(
                status.success(),
                "focus admission regression failed: {}",
                String::from_utf8_lossy(&output.stdout)
            );
            assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
            break;
        }
        if std::time::Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("focus admission recovery stalled");
        }
        crate::test_blocking_wait::pause(std::time::Duration::from_millis(10));
    }
}

/// Runs only inside `focus_refused_under_a_full_queue_is_delivered_once_admission_recovers`.
///
/// The child turns focus reporting on, then waits at a gate. Once released it
/// floods the event mailbox with titles while the UI thread is paused, so the
/// worker stalls and the command queue fills. Pane Focus is gained while the
/// queue is full; the refused `Focus(true)` must still reach the child — as
/// CSI I — once the UI resumes and admission recovers.
#[gpui::test]
fn focus_admission_pressure_child(cx: &mut gpui::TestAppContext) {
    if std::env::var_os("SPRITE_FOCUS_PRESSURE_CHILD").is_none() {
        return;
    }
    let settings = crate::config::Settings::default();
    cx.set_global(crate::config::ActiveSettings(settings.clone()));
    cx.set_global(crate::tokens::TokenRegistry::new(&settings.colors));
    let gate = std::env::temp_dir().join(format!(
        "sprite-focus-pressure-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let titles: String = (0..70).map(|i| format!("\x1b]2;burst-{i}\x07")).collect();
    let program = format!(
        "stty raw -echo; printf '\\033[?1004hARMED'; while [ ! -e '{}' ]; do sleep 0.005; done; \
         printf '%s' '{titles}'; head -c 327680 /dev/zero; \
         dd bs=1 count=3 status=none | od -An -tx1 | tr -d ' \\n'; printf 'END'; sleep 30",
        gate.to_str().unwrap()
    );
    let (sender, _exits) = async_channel::unbounded();
    let (view, cx) = cx.add_window_view(|window, cx| {
        TerminalView::new(
            Some(vec!["/bin/sh".into(), "-c".into(), program.into()]),
            settings.clone(),
            Vec::new(),
            None,
            PaneExit {
                sender,
                identity: (crate::tabs::TabId(1), crate::pane_tree::PaneId(1)),
            },
            window,
            cx,
        )
    });
    wait_for_bundle(&view, cx, |bundle| {
        bundle.pane.rows.iter().any(|row| row.text.contains("ARMED"))
    });
    // The window is active and drawn, with nothing focused yet: no Focus is
    // sent, and the view's handle is in the rendered tree for later.
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    redraw(cx);
    assert!(!view.read_with(cx, |view, _| view.pane_focused()));

    std::fs::write(&gate, b"go").unwrap();
    crate::test_blocking_wait::pause(std::time::Duration::from_millis(750));
    view.update_in(cx, |view, window, cx| {
        // Whatever room the stalled worker left is taken, so the next command
        // is refused.
        let mut refused = false;
        for _ in 0..64 {
            if !view.submit(TerminalCommand::ClearSelection) {
                refused = true;
                break;
            }
        }
        assert!(refused, "the fixture never filled the command queue");
        window.focus(&view.focus);
        view.refresh_pane_focus(window, cx);
        assert!(view.pane_focused());
        assert_eq!(
            view.pending_focus,
            Some(true),
            "a refused Focus is kept for recovery"
        );
        assert!(view
            .status
            .as_ref()
            .is_some_and(|status| status.contains("queue is full")));
    });

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(6);
    loop {
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(5));
        cx.executor().tick();
        if view.read_with(cx, |view, _| {
            view.pending_focus.is_none()
                && view.bundle.as_ref().is_some_and(|bundle| {
                    bundle
                        .pane
                        .rows
                        .iter()
                        .any(|row| row.text.contains("1b5b49END"))
                })
        }) {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the refused Focus never reached the child: {:?}",
            view.read_with(cx, |view, _| (view.pending_focus, view.status.clone()))
        );
        crate::test_blocking_wait::pause(std::time::Duration::from_millis(1));
    }
    view.update(cx, |view, _| {
        view.begin_shutdown();
    });
    std::fs::remove_file(gate).unwrap();
}
```

- [ ] **Step 2: Run it and confirm it fails.**
  `TERM=dumb cargo test -p sprite-app --locked --offline terminal_view::tests::focus_refused_under_a_full_queue_is_delivered_once_admission_recovers -- --exact`
  Expected: compile error `no field `pending_focus` on type `TerminalView``. (With Cycle B's plain `send`, the refused `Focus(true)` is simply lost: `dd` never gets its three bytes and the child test panics "the refused Focus never reached the child".)

- [ ] **Step 3: Implement.** In `crates/sprite-app/src/terminal_view.rs`:

Fields — after `pending_resize: Option<sprite_term::ValidTerminalSize>,` (line 50) add:
```rust
    /// The Pane Focus the worker still has to be told, kept while the command
    /// queue refuses it so the latest value is delivered when room returns.
    pending_focus: Option<bool>,
    /// The Pane Focus the worker last accepted, so a change that is undone
    /// before it was delivered sends nothing at all.
    told_focus: bool,
```
In both constructors, after `pending_resize: None,` (lines 366 and 456) add:
```rust
            pending_focus: None,
            told_focus: false,
```

`begin_shutdown` — after `self.pending_resize = None;` (line 570) add `self.pending_focus = None;`.

`spawn_retry` — the closure body (lines 625-638), before → after:
```rust
                        if view.admission_closed
                            || !matches!(view.session, SessionState::Running(_))
                        {
                            view.pending_settings = None;
                            view.pending_resize = None;
                            return None;
                        }
                        if let Some(settings) = view.pending_settings.take() {
                            view.apply_settings(&settings, window, cx);
                        }
                        if let Some(size) = view.pending_resize {
                            view.admit_resize(size);
                        }
                        Some(view.pending_settings.is_some() || view.pending_resize.is_some())
```
```rust
                        if view.admission_closed
                            || !matches!(view.session, SessionState::Running(_))
                        {
                            view.pending_settings = None;
                            view.pending_resize = None;
                            view.pending_focus = None;
                            return None;
                        }
                        if let Some(settings) = view.pending_settings.take() {
                            view.apply_settings(&settings, window, cx);
                        }
                        if let Some(size) = view.pending_resize {
                            view.admit_resize(size);
                        }
                        if let Some(focused) = view.pending_focus {
                            view.admit_focus(focused);
                        }
                        Some(
                            view.pending_settings.is_some()
                                || view.pending_resize.is_some()
                                || view.pending_focus.is_some(),
                        )
```

In `refresh_pane_focus` (Cycle B), before → after:
```rust
        self.pane_focused = focused;
        self.send(TerminalCommand::Focus(focused));
```
```rust
        self.pane_focused = focused;
        self.admit_focus(focused);
```

After `refresh_pane_focus` add:
```rust
    /// Tells the worker the latest Pane Focus, or keeps it for the retry task
    /// when the command queue is full. Losing it there would leave the child
    /// allowed the clipboard, or deaf to focus reports, until the next change.
    fn admit_focus(&mut self, focused: bool) {
        self.pending_focus = None;
        if focused == self.told_focus {
            return;
        }
        if self.submit(TerminalCommand::Focus(focused)) {
            self.told_focus = focused;
        } else if !self.admission_closed {
            self.pending_focus = Some(focused);
        }
    }
```

(The explicit `view.send(TerminalCommand::Focus(false))` at session start stays: the queue is empty at that point, and `told_focus: false` matches what it says.)

- [ ] **Step 4: Run tests, confirm pass.**
  `TERM=dumb cargo test -p sprite-app --locked --offline terminal_view::tests::focus_refused_under_a_full_queue_is_delivered_once_admission_recovers -- --exact`
  `TERM=dumb cargo test -p sprite-app --locked --offline terminal_view::tests::settings_callback_recovers_latest_values_after_real_event_pressure -- --exact`
  `TERM=dumb cargo test -p sprite-app --locked --offline terminal_view::`
  Expected: all pass, including `natural_completion_retires_idle_admission_recovery` and `disconnected_worker_refuses_reload_and_retires_recovery` (closed admission clears `pending_focus` with the others).

- [ ] **Step 5: Commit**

```bash
git add crates/sprite-term/tests/clipboard.rs crates/sprite-term/tests/paste.rs \
  crates/sprite-app/src/terminal_view.rs crates/sprite-app/src/terminal_view/tests.rs \
  crates/sprite-app/src/workspace/test_support.rs crates/sprite-app/src/workspace/keymap.rs
git commit -m "fix(app): tell each terminal whether its pane has Pane Focus" \
  -m "A pane has Pane Focus when its focus handle contains the window's focus and the window is active. The view sends Focus(false) at session start and Focus(bool) on every change, retried like settings and resizes when the command queue is full, so OSC 52 writes are accepted from the focused pane and DECSET 1004 programs hear focus in and out; losing Pane Focus drops a held paste." \
  -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 8: One window clock; only the focused pane blinks; hollow, steady cursor elsewhere (R-R2, R-C4.2)

**Files:**
- Modify: `crates/sprite-pane/src/lib.rs:43-44` (`Pane::tick`), `:104-105` (`PaneHandle::tick`), `:131-133` (blanket impl)
- Create: `crates/sprite-app/src/workspace/clock.rs`
- Modify: `crates/sprite-app/src/workspace/mod.rs:19` (module list), struct (after the Task 5 `_activation` field), `Workspace::new` literal
- Modify: `crates/sprite-app/src/terminal_view.rs:35` (import), `:134` (`_blink` field), `:406` and `:500` (initialisers), `:413-428` (`spawn_blink`), `refresh_pane_focus` (Task 7), `:732-773` (`Pane` impl)
- Modify: `crates/sprite-app/src/terminal_view/render.rs:33-34` (constant), `:121-147` (`tick_blink`)
- Modify: `crates/sprite-app/src/terminal_view/theme.rs:278-284` (`grid_metrics`)
- Modify: `crates/sprite-app/src/surface/render.rs:135-140` (`GridMetrics`), `:224-236` (`render_grid`), tests `:611-618`, `:667-674`, `:681-692`
- Modify: `crates/sprite-app/src/grid_paint.rs:213-225` (`GridPaintSpec`), `:240-252` (`prepare`), `:299-313` (`new`), test `:1315-1327`
- Modify: `crates/sprite-app/src/paint_benchmark.rs:93-105`, `crates/sprite-app/src/surface_performance.rs:86-97`
- Test: `crates/sprite-app/src/grid_paint.rs` (`mod tests`, line 907), `crates/sprite-app/src/workspace/clock.rs` (`mod tests`), `crates/sprite-app/src/terminal_view/tests.rs` (top level)

**Interfaces:**
- Consumes: `TerminalView::pane_focused()`, `refresh_pane_focus` (Task 7); test helpers `two_panes`, `redraw` (Tasks 6-7).
- Produces:
  - `GridPaintSpec.focused: bool`; `GridMetrics.focused: bool`.
  - `sprite_pane::Pane::tick(&mut self, cx: &mut Context<Self>)` (default no-op); `sprite_pane::PaneHandle::tick(&self, cx: &mut App)`.
  - `TerminalView::clock_tick(&mut self, cx: &mut Context<Self>)` (`pub(super)`, replaces `tick_blink`).
  - `crate::workspace::clock::BLINK_INTERVAL` (`pub(super)`, moved from `terminal_view/render.rs`).

#### Cycle A: an unfocused pane draws a hollow block

- [ ] **Step 1: Write the failing test.** `crates/sprite-app/src/grid_paint.rs`, `mod tests`, after `drawing_prepares_decorations_for_whitespace_without_glyph_ink`:

```rust
    /// A pane without Pane Focus marks its cursor without competing with the
    /// one being typed into: a block becomes its outline and leaves the cell
    /// its own colours; a bar or an underline keeps its shape.
    #[test]
    fn an_unfocused_pane_outlines_a_block_cursor_and_keeps_other_shapes() {
        let spec = |cursor: CursorSnapshot, focused: bool| GridPaintSpec {
            rows: Arc::from([]),
            pass: RowPass::Whole,
            cursor: Some(cursor),
            cursor_color: None,
            default_fg: unpack(0xffffff),
            default_bg: unpack(0x112233),
            palette: None,
            cell_width: px(8.4),
            cell_height: px(16.8),
            font_family: ".SystemUIFont".into(),
            font_size: px(14.0),
            focused,
        };
        let block = CursorSnapshot {
            row: 0,
            column: 0,
            visible: true,
            blinking: true,
            style: CursorStyle::Block,
        };
        assert_eq!(
            GridPaint::new(spec(block, true)).cursor.unwrap().style,
            CursorStyle::Block
        );
        let unfocused = GridPaint::new(spec(block, false));
        assert_eq!(unfocused.cursor.unwrap().style, CursorStyle::BlockHollow);
        let cell = PositionedCell {
            column: 0,
            columns: 1,
            text: "x".into(),
            style: plain_style(SnapshotColor::Default, SnapshotColor::Default, false),
            selected: false,
            hovered_link: false,
        };
        let drawn = unfocused.draw(&cell, unfocused.cursor);
        assert_eq!(drawn.background, Some(rgb(0x112233)), "the cell is not inverted");
        assert_eq!(drawn.foreground, rgb(0xffffff));
        assert_eq!(
            drawn.cursor.map(|cursor| cursor.style),
            Some(CursorStyle::BlockHollow),
            "the outline is still drawn over the cell"
        );
        for style in [CursorStyle::Bar, CursorStyle::Underline] {
            let paint = GridPaint::new(spec(CursorSnapshot { style, ..block }, false));
            assert_eq!(paint.cursor.unwrap().style, style);
        }
    }
```

- [ ] **Step 2: Run it and confirm it fails.**
  `TERM=dumb cargo test -p sprite-app --locked --offline grid_paint::tests::an_unfocused_pane_outlines_a_block_cursor_and_keeps_other_shapes -- --exact`
  Expected: compile error `struct `GridPaintSpec` has no field named `focused``.

- [ ] **Step 3: Implement.**

`grid_paint.rs` — `GridPaintSpec` (after `pub font_size: Pixels,`, line 224) add:
```rust
    /// Whether the pane has Pane Focus. Without it a block cursor is drawn as
    /// its outline.
    pub focused: bool,
```
`GridPaint::prepare` literal (after `font_size: metrics.cells.font_size(),`, line 251) add `focused: metrics.focused,`.
`GridPaint::new` (lines 299-313) becomes:
```rust
    pub(crate) fn new(spec: GridPaintSpec) -> Self {
        // A pane without Pane Focus shows where its cursor is without
        // competing with the one being typed into: a block becomes its
        // outline, while a bar or an underline is already slight enough to
        // keep its shape. The pane holds such a cursor's blink phase visible.
        let cursor = spec.cursor.map(|cursor| match cursor.style {
            CursorStyle::Block if !spec.focused => CursorSnapshot {
                style: CursorStyle::BlockHollow,
                ..cursor
            },
            _ => cursor,
        });
        Self {
            rows: spec.rows,
            pass: spec.pass,
            cursor,
            cursor_color: spec.cursor_color,
            default_fg: spec.default_fg,
            default_bg: spec.default_bg,
            palette: spec.palette,
            cell_width: spec.cell_width,
            cell_height: spec.cell_height,
            font_family: spec.font_family,
            font_size: spec.font_size,
        }
    }
```
Existing test literal (line 1326, after `font_size: px(14.0),`) add `focused: true,`.

`surface/render.rs` — `GridMetrics` (after `pub blink_on: bool,`, line 139) add:
```rust
    /// Whether the pane has Pane Focus. A grid's cursor follows the terminal's
    /// rule: outlined and steady without it.
    pub focused: bool,
```
`render_grid` literal (after `font_size: metrics.cells.font_size(),`, line 235) add `focused: metrics.focused,`. Test literals at lines 617, 673, 691: after each `blink_on: true,` add `focused: true,`.

`terminal_view/theme.rs` `grid_metrics` (line 282) — after `blink_on: self.blink_on,` add `focused: self.pane_focused(),`.

`paint_benchmark.rs` literal (after `font_size: px(14.0),`, line 104) add `focused: true,`. `surface_performance.rs` (after `blink_on: true,`, line 96) add `focused: true,`.

- [ ] **Step 4: Run tests, confirm pass.**
  `TERM=dumb cargo test -p sprite-app --locked --offline grid_paint::`
  `TERM=dumb cargo test -p sprite-app --locked --offline surface::render::`

#### Cycle B: one clock per window

- [ ] **Step 1: Write the failing tests.**

Create `crates/sprite-app/src/workspace/clock.rs` with only the tests for now:

```rust
#[cfg(test)]
mod tests {
    use super::super::test_support::*;
    use super::*;

    struct TickingPane {
        focus: FocusHandle,
        ticks: Rc<std::cell::Cell<usize>>,
    }

    impl Focusable for TickingPane {
        fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
            self.focus.clone()
        }
    }

    impl Render for TickingPane {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().track_focus(&self.focus)
        }
    }

    impl gpui::EventEmitter<sprite_pane::TitleChanged> for TickingPane {}

    impl sprite_pane::Pane for TickingPane {
        type Request = SurfaceRequest;
        fn title(&self) -> Option<SharedString> {
            None
        }
        fn set_allocated(&mut self, _: Size<Pixels>) {}
        fn begin_shutdown(&mut self) -> Option<Box<dyn FnOnce() + Send>> {
            None
        }
        fn close_warning(&self) -> Option<sprite_pane::CloseWarning> {
            None
        }
        fn tick(&mut self, _: &mut Context<Self>) {
            self.ticks.set(self.ticks.get() + 1);
        }
    }

    /// One timer for the window, and every pane hears each beat once —
    /// including a pane in a background tab, whose tab label still needs its
    /// title.
    #[gpui::test]
    fn one_window_clock_beats_every_pane_once_per_interval(cx: &mut gpui::TestAppContext) {
        let (workspace, cx) = test_workspace(cx);
        let ticks: Vec<Rc<std::cell::Cell<usize>>> = (0..3).map(|_| Rc::default()).collect();
        workspace.update(cx, |workspace, cx| {
            let mut next = ticks.iter();
            let mut make = |_, _| {
                let ticks = Rc::clone(next.next().unwrap());
                Rc::new(cx.new(|cx| TickingPane {
                    focus: cx.focus_handle(),
                    ticks,
                })) as Rc<dyn PaneHandle<Request = SurfaceRequest>>
            };
            workspace.tabs = Tabs::new(&mut make);
            workspace.tabs.split(Orientation::Horizontal, &mut make);
            workspace.tabs.open(&mut make);
            workspace.refresh_layout(cx);
        });
        cx.run_until_parked();
        let counts = || ticks.iter().map(|count| count.get()).collect::<Vec<_>>();
        assert_eq!(counts(), [0, 0, 0]);
        cx.executor().advance_clock(BLINK_INTERVAL);
        cx.run_until_parked();
        assert_eq!(counts(), [1, 1, 1]);
        cx.executor().advance_clock(BLINK_INTERVAL);
        cx.run_until_parked();
        assert_eq!(counts(), [2, 2, 2]);
    }
}
```

In `crates/sprite-app/src/workspace/mod.rs` after line 19 (`mod surface_routing;`) add `mod clock;`.

`crates/sprite-app/src/terminal_view/tests.rs` — add a helper after `focus_and_draw`:

```rust
/// One beat of the window clock, delivered the way the workspace delivers it.
fn clock_beat(view: &gpui::Entity<TerminalView>, cx: &mut gpui::VisualTestContext) {
    view.update(cx, |view, cx| view.clock_tick(cx));
    cx.run_until_parked();
}
```

**Guard (passes at master by design):** the rewritten `fallback_titles_use_existing_blink_activity_and_close_checks_stay_live` keeps title discovery for a pane without Pane Focus, which the per-pane timers already did at master. This cycle's failing-at-master tests are `one_window_clock_beats_every_pane_once_per_interval` and `a_clock_beat_repaints_only_the_pane_with_pane_focus`; Cycle A's grid-paint test also fails at master.

In `fallback_titles_use_existing_blink_activity_and_close_checks_stay_live` (lines 221-368) replace every occurrence (four: lines 280-281, 309-310, 331-332, 354-355) of

```rust
    cx.executor().advance_clock(BLINK_INTERVAL);
    cx.run_until_parked();
```
with
```rust
    clock_beat(&view, cx);
```
change the message `"the existing blink timer discovers a silent job without a new snapshot"` to `"the window clock discovers a silent job without a new snapshot"`, and, as the test's last statement, add:
```rust
    assert!(
        !view.read_with(cx, |view, _| view.pane_focused()),
        "the clock found that title for a pane without Pane Focus"
    );
```

At the end of the file add:

```rust
/// A beat of the window clock repaints only the pane with Pane Focus. The
/// other pane keeps a steady, visible cursor and is not asked to repaint at
/// all; when focus moves, the roles swap and the pane left behind shows its
/// cursor again at once.
#[gpui::test]
fn a_clock_beat_repaints_only_the_pane_with_pane_focus(cx: &mut gpui::TestAppContext) {
    let mut settings = crate::config::Settings::default();
    settings.cursor.blink = Some(true);
    cx.set_global(crate::config::ActiveSettings(settings.clone()));
    cx.set_global(crate::tokens::TokenRegistry::new(&settings.colors));
    let (left, right, cx) = two_panes(cx, &settings, "printf READY; exec sleep 30");
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        for view in [&left, &right] {
            wait_for_bundle(view, cx, |b| {
                b.render.cursor.blinking
                    && b.render.cursor.visible
                    && b.pane.rows.iter().any(|r| r.text.contains("READY"))
            });
            // Only the clock may repaint from here on.
            view.update(cx, |view, _| view._snapshots = Task::ready(()));
        }
        left.update_in(cx, |view, window, _| window.focus(&view.focus));
        cx.update(|window, _| window.activate_window());
        cx.run_until_parked();
        redraw(cx);
        assert!(left.read_with(cx, |view, _| view.pane_focused()));
        assert!(!right.read_with(cx, |view, _| view.pane_focused()));

        let notifications = |view: &gpui::Entity<TerminalView>, cx: &mut gpui::VisualTestContext| {
            let count = std::rc::Rc::new(std::cell::Cell::new(0_usize));
            let seen = count.clone();
            let subscription = cx.update(|_, cx| {
                cx.observe(view, move |_, _| seen.set(seen.get() + 1))
            });
            (count, subscription)
        };
        let (left_count, _left) = notifications(&left, cx);
        let (right_count, _right) = notifications(&right, cx);

        for view in [&left, &right] {
            clock_beat(view, cx);
        }
        assert_eq!(left_count.get(), 1, "the pane with Pane Focus blinks");
        assert_eq!(right_count.get(), 0, "an unfocused pane is not repainted by the clock");
        assert!(!left.read_with(cx, |view, _| view.blink_on));
        assert!(right.read_with(cx, |view, _| view.blink_on), "and its cursor stays visible");

        right.update_in(cx, |view, window, _| window.focus(&view.focus));
        redraw(cx);
        assert!(
            left.read_with(cx, |view, _| view.blink_on),
            "losing Pane Focus shows the cursor at once"
        );
        left_count.set(0);
        right_count.set(0);
        for view in [&left, &right] {
            clock_beat(view, cx);
        }
        assert_eq!(left_count.get(), 0);
        assert_eq!(right_count.get(), 1);
    }));
    for view in [&left, &right] {
        if let Some(cleanup) = view.update(cx, |view, _| view.begin_shutdown()) {
            cleanup.wait().unwrap();
        }
    }
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}
```

- [ ] **Step 2: Run them and confirm they fail.**
  `TERM=dumb cargo test -p sprite-app --locked --offline workspace::clock::tests::one_window_clock_beats_every_pane_once_per_interval -- --exact`
  Expected: compile errors — `method `tick` is not a member of trait `sprite_pane::Pane``, unresolved names in `clock.rs` (`BLINK_INTERVAL`, and the workspace imports its tests expect from the not-yet-written `use super::*;`), and `no method named `clock_tick`` in `terminal_view/tests.rs`. (Behaviourally, at master every pane's own timer toggles its blink, so `right_count` would be 1: the unfocused pane repaints on every beat.)

- [ ] **Step 3: Implement.**

`crates/sprite-pane/src/lib.rs` — in `trait Pane`, after `cycle_surface_focus` (line 43) add:
```rust
    /// One beat of the window's clock, about twice a second, delivered to every
    /// pane in the window whether or not it is in front.
    ///
    /// For what has no event of its own: a terminal pane learns here that a
    /// silent program has taken the foreground, and the pane with Pane Focus
    /// blinks its cursor. A pane with nothing to poll or animate ignores it.
    fn tick(&mut self, _cx: &mut Context<Self>) {}
```
In `trait PaneHandle`, after `cycle_surface_focus` (line 105) add:
```rust
    /// See [`Pane::tick`].
    fn tick(&self, cx: &mut App);
```
In `impl<V: Pane> PaneHandle for gpui::Entity<V>`, after `cycle_surface_focus` (line 133) add:
```rust
    fn tick(&self, cx: &mut App) {
        self.update(cx, |pane, cx| pane.tick(cx));
    }
```

`crates/sprite-app/src/workspace/clock.rs` — above the test module add:
```rust
//! The window's one clock.
//!
//! A cursor blinks, and a silent program's name is discovered, on the same
//! beat for every pane in the window. One timer for the window rather than one
//! per pane: each pane decides what a beat means for it, and only the pane
//! with Pane Focus repaints.

use super::*;

/// Half a blink. The rate every terminal has used since the VT100.
pub(super) const BLINK_INTERVAL: std::time::Duration = std::time::Duration::from_millis(530);

impl Workspace {
    /// Starts the clock. It stops by itself once the window is gone.
    pub(super) fn spawn_clock(cx: &mut Context<Self>) -> gpui::Task<()> {
        cx.spawn(async move |workspace, cx| {
            loop {
                cx.background_executor().timer(BLINK_INTERVAL).await;
                if workspace
                    .update(cx, |workspace, cx| workspace.tick_panes(cx))
                    .is_err()
                {
                    return;
                }
            }
        })
    }

    /// Every pane in every tab, background tabs included: a tab shows its
    /// pane's title whether or not it is the tab in front.
    fn tick_panes(&mut self, cx: &mut Context<Self>) {
        let panes: Vec<_> = self
            .tabs
            .all_panes()
            .into_iter()
            .map(|(_, _, pane)| Rc::clone(pane))
            .collect();
        for pane in panes {
            pane.tick(cx);
        }
    }
}
```

`crates/sprite-app/src/workspace/mod.rs` — struct, after the `_activation` field added in Task 5:
```rust
    /// The window's one clock: blink phase and title discovery for every pane.
    _clock: gpui::Task<()>,
```
In `Workspace::new`, after the `let activation = …;` statement (Task 5) add `let clock = Self::spawn_clock(cx);`, and in the literal after `_activation: activation,` add `_clock: clock,`.

`crates/sprite-app/src/terminal_view/render.rs` — delete lines 33-34 (`/// Half a blink…` and `pub(super) const BLINK_INTERVAL…`). Replace `tick_blink` (lines 121-147) with:
```rust
    /// One beat of the window's clock.
    ///
    /// Every pane refreshes its title on every beat — a program that starts
    /// without output gives no other sign — but only the pane with Pane Focus
    /// blinks, so a window of many panes repaints one of them, not all. A pane
    /// without Pane Focus, or with nothing blinking, holds its cursor visible,
    /// so neither losing focus nor a program stopping the blink can leave the
    /// cursor hidden.
    pub(super) fn clock_tick(&mut self, cx: &mut Context<Self>) {
        self.refresh_display_title(cx);
        let terminal_blinks = self
            .bundle
            .as_ref()
            .is_some_and(|bundle| bundle.render.cursor.blinking && bundle.render.cursor.visible);
        // A grid Surface draws its cursor from this same phase, and a fill
        // grid hides the terminal behind it, so a grid asking for a blink is
        // reason enough for the pane to keep one.
        let grid_blinks = self.surfaces.iter().any(|surface| match &surface.body {
            Body::Grid { grid, .. } => grid.cursor_blinks(),
            Body::Elements { .. } | Body::List { .. } => false,
        });
        if !(self.pane_focused() && (terminal_blinks || grid_blinks)) {
            if !self.blink_on {
                self.blink_on = true;
                cx.notify();
            }
            return;
        }
        self.blink_on = !self.blink_on;
        cx.notify();
    }
```

`crates/sprite-app/src/terminal_view.rs`:
- Delete line 35 (`use render::BLINK_INTERVAL;`).
- Delete the field `_blink: Task<()>,` (line 134) and both initialisers `_blink: Self::spawn_blink(cx),` (lines 406 and 500).
- Delete `spawn_blink` with its doc comment (lines 413-428).
- In `refresh_pane_focus` (as left by Task 7 Cycle C), before → after:
```rust
        self.pane_focused = focused;
        self.admit_focus(focused);
```
```rust
        self.pane_focused = focused;
        self.admit_focus(focused);
        // A pane gaining focus starts its blink from visible, and one losing
        // it shows a steady cursor from the next frame on.
        self.blink_on = true;
```
- In `impl sprite_pane::Pane for TerminalView`, after `cycle_surface_focus` (ends line 746) add:
```rust
    fn tick(&mut self, cx: &mut Context<Self>) {
        self.clock_tick(cx);
    }
```

- [ ] **Step 4: Run tests, confirm pass.**
  `TERM=dumb cargo test -p sprite-app --locked --offline workspace::clock::tests::one_window_clock_beats_every_pane_once_per_interval -- --exact`
  `TERM=dumb cargo test -p sprite-app --locked --offline terminal_view::tests::a_clock_beat_repaints_only_the_pane_with_pane_focus -- --exact`
  `TERM=dumb cargo test -p sprite-app --locked --offline terminal_view::tests::fallback_titles_use_existing_blink_activity_and_close_checks_stay_live -- --exact`
  `TERM=dumb cargo test -p sprite-app --locked --offline`
  `TERM=dumb cargo test -p sprite-pane --locked --offline`
  Expected: all pass. `grep -rn "spawn_blink\|tick_blink\|_blink:" crates/sprite-app/src` prints nothing.

- [ ] **Step 5: Commit**

```bash
git add crates/sprite-pane/src/lib.rs crates/sprite-app/src/workspace/clock.rs \
  crates/sprite-app/src/workspace/mod.rs crates/sprite-app/src/terminal_view.rs \
  crates/sprite-app/src/terminal_view/render.rs crates/sprite-app/src/terminal_view/theme.rs \
  crates/sprite-app/src/terminal_view/tests.rs crates/sprite-app/src/surface/render.rs \
  crates/sprite-app/src/grid_paint.rs crates/sprite-app/src/paint_benchmark.rs \
  crates/sprite-app/src/surface_performance.rs
git commit -m "perf(app): one clock per window; only the focused pane blinks" \
  -m "The workspace ticks every pane on one 530 ms timer. Each pane refreshes its title on every beat, but only the pane with Pane Focus toggles its blink and repaints. Unfocused panes draw a steady cursor, with a block shown as its outline; a grid Surface's cursor follows the same rule." \
  -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

---

### Task 9: Coalesce worker work — one pass, one capture, latest title (BCA-16, BCA-23; PRD R-T1, R-T2)

Three cycles, each committed separately: **A** the pump fills a permit before handing it over; **B** title and working-directory notices keep only their latest value (and the event-pressure fixtures that relied on one event per title move to OSC 52 clipboard writes); **C** the worker drains already-queued messages into one bounded pass and captures once.

**Files:**
- Modify: `crates/sprite-term/src/pty_unix.rs:342-364` (read in `run`), `:499-521` (`read_once` → `read_available`), `:67-77` (test-only `OutputChunk::detached`), tests `:622-879`
- Modify: `crates/sprite-term/src/worker/mod.rs:96-121` (`Notices`, `register_bell` neighbours), `:256-313` + `:609-613` (`handle` → `handle`/`publish_notices`/`apply`), `:637-665` (`drain_accepted_output`), tests appended after `:818`
- Modify: `crates/sprite-term/src/worker/start.rs:90-124` (title/pwd registration)
- Modify: `crates/sprite-term/src/session.rs:56-59` (comment only)
- Modify (fixture migration, cycle B): `crates/sprite-term/tests/event_backpressure.rs:1-200`; `crates/sprite-app/src/lib.rs:24-25`; new `crates/sprite-app/src/test_event_pressure.rs`; `crates/sprite-app/src/terminal_view/submission_regressions.rs:21,67-71,135,214,364-368,489`; `crates/sprite-app/src/observation/panes.rs:293`; `crates/sprite-app/src/terminal_view/tests.rs:880-888,911,1064-1067,1082` (master lines; Tasks 6–8 insert tests above them, so locate by test name) and Task 7's `focus_admission_pressure_child` in the same file; `crates/sprite-app/src/terminal_view/theme.rs:442,455-465`
- Modify (docs, cycle C): `docs/adr/0021-bound-terminal-delivery-and-cancel-event-pressure.md`, `docs/adr/0026-bound-event-delivery-and-recover-ui-admission.md` (append one line each)
- Test: inline `#[cfg(test)] mod tests` in `pty_unix.rs`; new inline modules `notice_tests` and `coalescing_tests` in `worker/mod.rs` (the file already uses per-concern inline modules `bell_tests`, `closing_regressions`)

**Interfaces:**
- Consumes (Task 9 executes after Tasks 6 and 7):
  - Task 7: Pane Focus wiring in `TerminalView` — `pane_focused(&self) -> bool`, `refresh_pane_focus`, `admit_focus` with `pending_focus: Option<bool>` / `told_focus`, the `Focus(false)` sent at session start, and its test `terminal_view::tests::focus_admission_pressure_child` (whose 70-title burst this task migrates).
  - Task 6: test helpers `focus_and_draw(&Entity<TerminalView>, &mut VisualTestContext)` and `redraw(&mut VisualTestContext)` in `terminal_view/tests.rs`.
- Produces (Task 11 and Task 12 build on these):
  - `impl Session { fn handle(&mut self, first: Message) -> Flow; fn publish_notices(&mut self) -> Flow; fn apply(&mut self, message: Message) -> Flow }` in `worker/mod.rs`. Every per-message arm (`Select`, `ClearSelection`, `CaptureHistory`, `Focus`, …) now lives in **`apply`**, not `handle`.
  - `const BATCH_MESSAGES: usize = 16; const BATCH_OUTPUT_BYTES: usize = 16 * 1024;` (worker/mod.rs)
  - `fn register_title(&mut Terminal<'static,'static>, Rc<RefCell<Notices>>) -> Result<(), libghostty_vt::Error>` and `fn register_pwd(...)` (worker/mod.rs)
  - `#[cfg(test)] pub(crate) fn OutputChunk::detached(bytes: &[u8]) -> OutputChunk` (pty_unix.rs)
  - `#[cfg(test)] crate::test_event_pressure::{CLIPBOARD_WRITE, focus_and_release}` in sprite-app

#### Cycle A — the pump fills one permit per wake (R-T1, macOS half)

- [ ] **Step 1: Write the failing test** — append to `mod tests` in `crates/sprite-term/src/pty_unix.rs` (after `pump_keeps_its_endpoint_alive_after_the_callers_descriptor_closes`, before the module's closing `}`). It reuses the module's existing `Pump::start`/`sync_channel`/`Message` pattern from `chunks_can_outlive_the_stopped_pump`.

```rust
    /// One permit carries everything already waiting, not one read's worth.
    ///
    /// A datagram socket returns one datagram per read, which is how a macOS
    /// PTY behaves too: it hands over a small slice per read however much is
    /// queued. Three datagrams sent before the pump starts must therefore
    /// arrive as one chunk.
    #[test]
    fn one_permit_reads_everything_already_waiting() {
        let (master, peer) = std::os::unix::net::UnixDatagram::pair().expect("datagram pair");
        for part in [&b"first "[..], &b"second "[..], &b"third"[..]] {
            peer.send(part).expect("queue output");
        }
        let (commands, inbox) = sync_channel(crate::WORKER_QUEUE_CAPACITY);
        let mut pump = Pump::start(master.as_raw_fd(), commands).expect("pump");
        let message = inbox.recv_timeout(Duration::from_secs(2)).expect("chunk");
        // Disconnect before joining so a failing assertion cannot leave a full inbox.
        drop(inbox);
        pump.shutdown();
        let Message::PtyOutput(chunk) = message else {
            panic!("expected output");
        };
        assert_eq!(chunk.as_bytes(), b"first second third");
    }
```

- [ ] **Step 2: Run it and confirm it fails** — `TERM=dumb cargo test -p sprite-term --locked --offline pty_unix::tests::one_permit_reads_everything_already_waiting -- --exact`. Expected: assertion failure, `left: [102, 105, 114, 115, 116, 32]` (`"first "`) vs `right: "first second third"` bytes.

- [ ] **Step 3: Implement** — in `crates/sprite-term/src/pty_unix.rs`:

Replace the read call in `run` (line 349):
```rust
            match read_once(master.as_fd(), buffer) {
```
with
```rust
            match read_available(master.as_fd(), buffer) {
```

Replace the whole of `fn read_once` (lines 506-521) with:
```rust
/// Reads into one permit's buffer until the descriptor has nothing more to
/// give right now, or the buffer is full.
///
/// A macOS PTY hands over at most a small slice of what is waiting per read,
/// so one read per permit spent a permit — and a pass of the worker — on each
/// slice. Reading until `EAGAIN` makes a permit carry what one buffer can hold.
/// What has already been read is always delivered: if a later read in the same
/// fill fails or reports the end, the next poll reports that on its own.
fn read_available(master: BorrowedFd<'_>, buffer: &mut [u8]) -> ReadResult {
    let mut filled = 0;
    loop {
        match nix::unistd::read(master.as_raw_fd(), &mut buffer[filled..]) {
            Ok(0) if filled == 0 => return ReadResult::Eof,
            Ok(0) => return ReadResult::Chunk(filled),
            Ok(count) => {
                filled += count;
                if filled == buffer.len() {
                    return ReadResult::Chunk(filled);
                }
            }
            Err(Errno::EINTR) => continue,
            Err(_) if filled > 0 => return ReadResult::Chunk(filled),
            Err(Errno::EAGAIN) => return ReadResult::NotReady,
            // Linux reports the closed slave this way rather than with a
            // zero-length read; it is an ordinary end of session, not a fault.
            Err(Errno::EIO) => return ReadResult::Eof,
            Err(error) => return ReadResult::Failed(error.to_string()),
        }
    }
}
```

Update the module doc sentence at lines 7-12 so it stays true:
```rust
//! It carries both directions. Output is read under a permit scheme rather than
//! queue capacity: the pump must hold one of sixteen tokens before it waits for
//! readability, fills that token's buffer with whatever is already waiting, and
//! dropping the resulting chunk returns its buffer and token.
//! At most sixteen 16 KiB chunks can therefore
//! occupy the 17-slot worker queue, which structurally reserves the last slot
//! for input and lifecycle work.
```

- [ ] **Step 4: Run tests, confirm pass** — `TERM=dumb cargo test -p sprite-term --locked --offline pty_unix::` (the new test plus every existing pump test, including `steady_state_pump_delivers_sixty_four_chunks_without_allocating`).

- [ ] **Step 5: Commit**
```bash
git add crates/sprite-term/src/pty_unix.rs
git commit -m "fix(term): fill a pump permit with all waiting output

A macOS PTY returns a small slice per read, so each permit carried one
slice and the worker ran a pass per slice. Read until EAGAIN or a full
buffer before handing the chunk over.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

#### Cycle B — title and working directory keep their latest value (R-T2)

- [ ] **Step 1: Write the failing test** — add a new module at the end of `crates/sprite-term/src/worker/mod.rs`, after `mod closing_regressions`. It mirrors `bell_tests` (same `Terminal::new` + `Notices` + register pattern; `register_bell` exists at line 112).

```rust
#[cfg(test)]
mod notice_tests {
    use super::*;
    use libghostty_vt::terminal::Options as TerminalOptions;

    #[test]
    fn a_pass_of_title_changes_yields_one_event_with_the_latest_title() {
        let mut terminal = Terminal::new(TerminalOptions {
            cols: 80,
            rows: 24,
            max_scrollback: 0,
        })
        .expect("terminal");
        let notices = Rc::new(RefCell::new(Notices::default()));
        register_title(&mut terminal, Rc::clone(&notices)).expect("title callback");
        register_pwd(&mut terminal, Rc::clone(&notices)).expect("pwd callback");

        let retitles: String = (0..100)
            .map(|index| format!("\x1b]2;title-{index}\x07"))
            .collect();
        terminal.vt_write(retitles.as_bytes());
        // A second chunk in the same pass is still the same pass.
        terminal.vt_write(b"\x1b]7;file:///tmp/one\x07\x1b]7;file:///tmp/two\x07");

        let events = notices.borrow_mut().take();
        assert!(
            matches!(
                events.as_slice(),
                [
                    TerminalEvent::TitleChanged(Some(title)),
                    TerminalEvent::WorkingDirectoryChanged(Some(directory)),
                ] if title == "title-99" && directory.contains("/tmp/two")
            ),
            "{events:?}"
        );
        assert!(
            notices.borrow_mut().take().is_empty(),
            "a taken notice is not reported twice"
        );
    }
}
```

- [ ] **Step 2: Run it and confirm it fails** — `TERM=dumb cargo test -p sprite-term --locked --offline worker::notice_tests`. Expected: compile error `cannot find function `register_title` in this scope` (and `register_pwd`).

- [ ] **Step 3: Implement**

3a. In `crates/sprite-term/src/worker/mod.rs`, replace lines 96-110 (`#[derive(Default)] struct Notices { … }` and its `impl`) with:
```rust
/// What the parser raised and the worker has not yet published.
///
/// A title, a working directory and the bell each keep only their latest
/// state until publication: a program that retitles itself on every prompt or
/// progress tick would otherwise spend the event budget on names nobody sees.
#[derive(Default)]
struct Notices {
    bell_pending: bool,
    title: Option<Option<String>>,
    working_directory: Option<Option<String>>,
}

impl Notices {
    fn take(&mut self) -> Vec<TerminalEvent> {
        let mut events = Vec::new();
        if let Some(title) = self.title.take() {
            events.push(TerminalEvent::TitleChanged(title));
        }
        if let Some(directory) = self.working_directory.take() {
            events.push(TerminalEvent::WorkingDirectoryChanged(directory));
        }
        if std::mem::take(&mut self.bell_pending) {
            events.push(TerminalEvent::Bell);
        }
        events
    }
}
```

3b. Directly after `fn register_bell` (ends line 121) add:
```rust
fn register_title(
    terminal: &mut Terminal<'static, 'static>,
    notices: Rc<RefCell<Notices>>,
) -> Result<(), libghostty_vt::Error> {
    terminal
        .on_title_changed(move |terminal: &Terminal<'_, '_>| {
            let title = terminal
                .title()
                .ok()
                .filter(|value| !value.is_empty())
                .map(str::to_owned);
            notices.borrow_mut().title = Some(title);
        })
        .map(|_| ())
}

fn register_pwd(
    terminal: &mut Terminal<'static, 'static>,
    notices: Rc<RefCell<Notices>>,
) -> Result<(), libghostty_vt::Error> {
    terminal
        .on_pwd_changed(move |terminal: &Terminal<'_, '_>| {
            let directory = terminal
                .pwd()
                .ok()
                .filter(|value| !value.is_empty())
                .map(str::to_owned);
            notices.borrow_mut().working_directory = Some(directory);
        })
        .map(|_| ())
}
```

3c. In `crates/sprite-term/src/worker/start.rs`, replace lines 90-124 (both `let registered_title = …` and `let registered_pwd = …` blocks with their `if let Err` checks) with:
```rust
    if let Err(error) = register_title(&mut terminal, Rc::clone(&notices)) {
        return Err(SessionError::new("on_title_changed", error));
    }

    if let Err(error) = register_pwd(&mut terminal, Rc::clone(&notices)) {
        return Err(SessionError::new("on_pwd_changed", error));
    }
```

3d. **Migrate the event-pressure fixtures.** Thirteen existing tests flood titles to put many lossless events in front of a paused consumer. With titles coalesced, a flood is one event per pass and those tests lose their pressure (several assert exact title counts). OSC 52 clipboard writes are the one parser notice that is never coalesced; an unfocused pane is denied them, so each fixture focuses the pane before the child writes. Fixtures that own a bare `TerminalSession` (sprite-term tests, `submission_regressions.rs`, `panes.rs`) send `Focus(true)` directly. Fixtures whose session belongs to a `TerminalView` get real Pane Focus through the window instead, so Task 7's wiring sends `Focus(true)` itself and nothing later overrides it. `Q0xJUA==` is base64 for `CLIP`.

(i) New file `crates/sprite-app/src/test_event_pressure.rs`:
```rust
//! Event pressure for tests: lossless notices a child raises faster than a
//! paused consumer takes them.

#![cfg(test)]

use sprite_term::{CommandSender, TerminalCommand};

/// One OSC 52 clipboard write of `CLIP`, sixteen bytes.
///
/// Clipboard writes are the one parser notice the worker never coalesces — a
/// title keeps only its latest value per pass — so a run of them is what fills
/// the event mailbox.
pub(crate) const CLIPBOARD_WRITE: &str = "\x1b]52;c;Q0xJUA==\x07";

/// Focuses the pane, then releases a child that began with `read _`.
///
/// An unfocused pane is denied the clipboard and raises nothing, and the
/// child must not write before the focus is in place, so it waits for this
/// line.
pub(crate) fn focus_and_release(commands: &CommandSender) {
    commands
        .send(TerminalCommand::Focus(true))
        .expect("focus the pane");
    commands
        .send(TerminalCommand::Input(b"\n".to_vec()))
        .expect("release the child");
}
```
and in `crates/sprite-app/src/lib.rs` after lines 24-25 (`#[cfg(test)] mod test_blocking_wait;`) add:
```rust
#[cfg(test)]
mod test_event_pressure;
```

(ii) Shell-loop floods. In each of these, replace the loop body text `printf '\\033]2;TITLE%s\\007' $i` with `printf '\\033]52;c;Q0xJUA==\\007'`, prefix the script with `stty -echo; read _; ` (line 214 already starts with `stty -echo; ` — insert only `read _; ` after it), and add the focus line right after the `TerminalSession::spawn(…).unwrap();` statement:
- `crates/sprite-app/src/terminal_view/submission_regressions.rs:21`, `:135`, `:214` (spawn is at `:220`), `:489` — add `crate::test_event_pressure::focus_and_release(&session.commands());`
- `crates/sprite-app/src/observation/panes.rs:293` — add `crate::test_event_pressure::focus_and_release(&spawned.session.commands());`

For example line 135 becomes:
```rust
    let sprite_term::Spawned { session, events, mut snapshots } = TerminalSession::spawn(SessionConfig::command("/bin/sh", vec!["-c".into(), "stty -echo; read _; i=0; while [ $i -lt 100 ]; do printf '\\033]52;c;Q0xJUA==\\007'; i=$((i+1)); done; head -c 1048576 /dev/zero; sleep 30".into()])).unwrap();
    crate::test_event_pressure::focus_and_release(&session.commands());
```

(iii) `submission_regressions.rs:67-71` waits for `TITLE99`; replace that `loop { … }` with:
```rust
    // Every write of the burst has been delivered once a hundred have arrived.
    let mut writes = 0;
    while writes < 100 {
        if matches!(
            events_rx
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap(),
            sprite_term::TerminalEvent::ClipboardWrite(_)
        ) {
            writes += 1;
        }
    }
```
(The `RESTORED_READY` title wait at `:259` stays: that title is printed once per line read, so it is still published.)

(iv) `submission_regressions.rs:364-368`: replace
```rust
    let titles: String = (0..100)
        .map(|index| format!("\x1b]2;TITLE{index}\x07"))
        .collect();
    let script = format!(
        "stty -echo; printf '%s' '{titles}'; head -c 1048576 /dev/zero; printf 'INPUT_READY\\n'; while read line; do printf '\\nPTY:%s\\n' \"$(stty size)\"; done"
    );
```
with
```rust
    let burst = crate::test_event_pressure::CLIPBOARD_WRITE.repeat(100);
    let script = format!(
        "stty -echo; read _; printf '%s' '{burst}'; head -c 1048576 /dev/zero; printf 'INPUT_READY\\n'; while read line; do printf '\\nPTY:%s\\n' \"$(stty size)\"; done"
    );
```
and add `crate::test_event_pressure::focus_and_release(&session.commands());` after its `.unwrap();` (line 377).

(v) `crates/sprite-app/src/terminal_view/tests.rs:880-888` (`settings_callback_pressure_child`): replace
```rust
    // The title burst must fit one macOS PTY read (1 KiB) yet overflow the event mailbox;
    // a split burst leaves the worker holding a second chunk and the command queue short of full.
    let titles: String = (0..70).map(|i| format!("\x1b]2;burst-{i}\x07")).collect();
```
with
```rust
    // The burst must fit one macOS PTY read (1 KiB) yet overflow the event mailbox;
    // a split burst leaves the worker holding a second chunk and the command queue short of full.
    // Clipboard writes, because a title keeps only its latest value per pass.
    let titles = crate::test_event_pressure::CLIPBOARD_WRITE.repeat(60);
```
and immediately before line 911 `std::fs::write(&gate, b"go").unwrap();` add:
```rust
    // An unfocused pane is denied the clipboard, so the burst would raise
    // nothing. Pane Focus is real here: the view tells the worker itself,
    // while the queue still has room, before the gate releases the burst.
    focus_and_draw(&view, cx);
    assert!(view.read_with(cx, |view, _| view.pane_focused()));
```

(vi) `tests.rs:1064-1067` (`disconnected_worker_refuses_reload_and_retires_recovery`): replace
```rust
    let titles: String = (0..150).map(|i| format!("\x1b]2;title{i}\x07")).collect();
    let script = format!("sleep .3; printf '%s' '{titles}'; exit 7");
```
with
```rust
    let burst = crate::test_event_pressure::CLIPBOARD_WRITE.repeat(150);
    let script = format!("stty -echo; read _; printf '%s' '{burst}'; exit 7");
```
and replace line 1082 `wait_for_bundle(&view, cx, |_| true);` with:
```rust
    wait_for_bundle(&view, cx, |_| true);
    // Pane Focus first, through the window, so the view itself tells the
    // worker: an unfocused pane is denied the clipboard and the burst would
    // raise no events at all. Only then is the child released.
    focus_and_draw(&view, cx);
    assert!(view.read_with(cx, |view, _| view.pane_focused()));
    view.update(cx, |view, _| view.send(TerminalCommand::Input(b"\n".to_vec())));
```

(vii) `crates/sprite-app/src/terminal_view/theme.rs` (`refused_reload_then_revert_restores_defaults_before_first_snapshot`). This fixture cannot be given Pane Focus. Pane Focus needs an active, drawn window, and activating and drawing runs the executor, which delivers the first snapshot. The test exists to run before that (`assert!(view.bundle.is_none())`). So its pressure comes from commands, not from the child: each `CopySelection` makes the worker publish one `SelectionCopied`, which is lossless and needs no focus. With the view's event task held off inside `update_in`, the worker stalls on the thirty-third event and the queue stays full. Change the script at line 442 to `"sleep 30"`, and replace the loop at lines 455-465:
```rust
                loop {
                    if session
                        .try_send(TerminalCommand::Capture)
                        .is_err_and(|error| error.message.contains("queue is full"))
                    {
                        let since = full_since.get_or_insert_with(std::time::Instant::now);
                        if since.elapsed() >= std::time::Duration::from_millis(100) {
                            break;
                        }
                    } else {
                        full_since = None;
                    }
```
with
```rust
                // Every accepted copy is one lossless event the paused UI does
                // not take; past the mailbox's thirty-two the worker stalls and
                // the queue stays full. A full queue only counts once the worker
                // has certainly started taking from it.
                let mut accepted = 0;
                loop {
                    match session.try_send(TerminalCommand::CopySelection) {
                        Ok(()) => {
                            accepted += 1;
                            full_since = None;
                        }
                        Err(error) if error.message.contains("queue is full") => {
                            if accepted > 40 {
                                let since = full_since.get_or_insert_with(std::time::Instant::now);
                                if since.elapsed() >= std::time::Duration::from_millis(100) {
                                    break;
                                }
                            }
                        }
                        Err(error) => panic!("unexpected refusal: {error}"),
                    }
```
(the existing deadline `assert!` and `pause(1 ms)` that follow stay as they are).

(ix) Task 7's `focus_admission_pressure_child` (in `crates/sprite-app/src/terminal_view/tests.rs`) floods 70 titles while the pane is deliberately *unfocused*, and then gains Pane Focus under a full queue. After coalescing, that burst is one event and the queue never fills. OSC 52 cannot replace it, because an unfocused pane raises no clipboard events. Stall the worker with commands instead, before the gate releases the zero-byte flood. Delete the `let titles: String = …;` line and the `printf '%s' '{titles}'; ` fragment from `program`. Then replace
```rust
    std::fs::write(&gate, b"go").unwrap();
    crate::test_blocking_wait::pause(std::time::Duration::from_millis(750));
```
(the pair directly after `assert!(!view.read_with(cx, |view, _| view.pane_focused()));`) with
```rust
    // Lossless events the paused UI does not take: past the mailbox's
    // thirty-two the worker stalls. Titles no longer do this — a pass keeps
    // only the latest — and the clipboard is closed to an unfocused pane.
    view.update(cx, |view, _| {
        let mut accepted = 0;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while accepted < 48 {
            if view.submit(TerminalCommand::CopySelection) {
                accepted += 1;
            } else {
                assert!(
                    std::time::Instant::now() < deadline,
                    "the worker stalled before the copies were queued"
                );
                crate::test_blocking_wait::pause(std::time::Duration::from_millis(1));
            }
        }
    });
    std::fs::write(&gate, b"go").unwrap();
    crate::test_blocking_wait::pause(std::time::Duration::from_millis(750));
```
Forty-eight accepted copies exceed the 32-slot mailbox plus the worker's one retained event, so at least fifteen copies are left waiting in the queue. The zero-byte flood fills the output permits, and the existing `ClearSelection` loop takes whatever room remains. Run the parent test `focus_refused_under_a_full_queue_is_delivered_once_admission_recovers` to check it.

(viii) `crates/sprite-term/tests/event_backpressure.rs`: replace the four title-based tests (`shutdown_retains_accepted_titles_without_consumer_progress`, `resumed_consumer_receives_all_titles_in_order`, `dropping_the_consumer_releases_event_pressure`, `natural_exit_releases_event_pressure_before_shutdown_is_requested`) and add helpers. Keep `every_submission_method_bounds_variable_sized_input` unchanged. New top of file through the first test:
```rust
use sprite_term::{SessionConfig, TerminalCommand, TerminalEvent, TerminalSession};
use std::sync::mpsc;
use std::time::Duration;

/// Standard base64, enough to spell an OSC 52 payload.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for group in bytes.chunks(3) {
        let at = |index: usize| u32::from(group.get(index).copied().unwrap_or(0));
        let bits = (at(0) << 16) | (at(1) << 8) | at(2);
        for index in 0..4 {
            if index <= group.len() {
                out.push(char::from(ALPHABET[(bits >> (18 - 6 * index)) as usize & 63]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// `count` OSC 52 clipboard writes of `CLIP0`, `CLIP1`, …, as one string,
/// sixteen bytes each.
///
/// Clipboard writes are the one parser notice the worker never coalesces — a
/// title keeps only its latest value per pass — so they are how these tests
/// put many lossless events into one pass.
fn clipboard_writes(count: usize) -> String {
    (0..count)
        .map(|index| format!("\x1b]52;c;{}\x07", base64(format!("CLIP{index}").as_bytes())))
        .collect()
}

/// A child that waits for one line before writing `writes`, then runs `then`.
fn on_cue(writes: &str, then: &str) -> SessionConfig {
    SessionConfig::command(
        "/bin/sh",
        vec![
            "-c".into(),
            format!("stty -echo; read _; printf '%s' '{writes}'; {then}").into(),
        ],
    )
}

/// Focuses the pane, then releases the child: an unfocused pane is denied the
/// clipboard and the writes would raise nothing.
fn focus_and_release(session: &mut TerminalSession) {
    session.send(TerminalCommand::Focus(true)).unwrap();
    session
        .send(TerminalCommand::Input(b"\n".to_vec()))
        .unwrap();
}

#[test]
fn shutdown_retains_accepted_clipboard_writes_without_consumer_progress() {
    // One write, small enough for one PTY read even where a read returns at
    // most 1 KiB, and more events than the mailbox's thirty-two ordinary slots.
    let writes = clipboard_writes(60);
    assert!(writes.len() <= 1024);
    let sprite_term::Spawned {
        mut session,
        mut events,
        snapshots: _snapshots,
    } = TerminalSession::spawn(on_cue(&writes, "sleep 30")).unwrap();
    focus_and_release(&mut session);
    std::thread::sleep(Duration::from_millis(300));
    let handle = session.begin_shutdown().unwrap().unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(handle.wait());
    });
    let joined = rx.recv_timeout(Duration::from_secs(7));
    let mut retained = Vec::new();
    let mut exited = false;
    while let Ok(event) = events.next_blocking() {
        match event {
            TerminalEvent::ClipboardWrite(text) => {
                assert!(!exited);
                retained.push(text);
            }
            TerminalEvent::Exited(_) => exited = true,
            _ => {}
        }
    }
    assert!(
        joined.is_ok(),
        "shutdown depends on consumer progress: {joined:?}"
    );
    assert!(exited);
    assert_eq!(
        retained.len(),
        60,
        "one accepted parser pass survives cancellation"
    );
    for (i, text) in retained.iter().enumerate() {
        assert_eq!(text, &format!("CLIP{i}"));
    }
}
```
Replace `resumed_consumer_receives_all_titles_in_order` with:
```rust
#[test]
fn resumed_consumer_receives_all_clipboard_writes_in_order() {
    let writes = clipboard_writes(100);
    let sprite_term::Spawned {
        mut session,
        mut events,
        snapshots: _snapshots,
    } = TerminalSession::spawn(on_cue(&writes, "sleep 0.2; exit 7")).unwrap();
    focus_and_release(&mut session);
    std::thread::sleep(Duration::from_millis(100));
    let mut received = Vec::new();
    while let Ok(event) = events.next_blocking() {
        if let TerminalEvent::ClipboardWrite(text) = event {
            received.push(text);
        }
    }
    assert_eq!(received.len(), 100);
    for (index, text) in received.iter().enumerate() {
        assert_eq!(text, &format!("CLIP{index}"));
    }
    session.begin_shutdown().unwrap().unwrap().wait().unwrap();
}
```
Replace `dropping_the_consumer_releases_event_pressure` with:
```rust
#[test]
fn dropping_the_consumer_releases_event_pressure() {
    let writes = clipboard_writes(100);
    let sprite_term::Spawned {
        mut session,
        events,
        snapshots: _snapshots,
    } = TerminalSession::spawn(on_cue(&writes, "sleep 30")).unwrap();
    focus_and_release(&mut session);
    std::thread::sleep(Duration::from_millis(300));
    drop(events);
    let handle = session.begin_shutdown().unwrap().unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(handle.wait());
    });
    rx.recv_timeout(Duration::from_secs(7))
        .expect("drop releases producer")
        .unwrap();
}
```
Replace `natural_exit_releases_event_pressure_before_shutdown_is_requested` with:
```rust
#[test]
fn natural_exit_releases_event_pressure_before_shutdown_is_requested() {
    // One write: separate small writes exhaust the output permits while the consumer
    // stalls, and macOS PTYs then block the child before it can exit.
    let writes = clipboard_writes(100);
    let sprite_term::Spawned {
        mut session,
        mut events,
        snapshots: _snapshots,
    } = TerminalSession::spawn(on_cue(&writes, "exit 7")).unwrap();
    focus_and_release(&mut session);
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    loop {
        let ended = session
            .try_send(TerminalCommand::Capture)
            .is_err_and(|error| error.message == "the terminal worker ended");
        if ended {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "natural cleanup depended on consuming events"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let mut outcome = None;
    let mut received = Vec::new();
    while let Ok(event) = events.next_blocking() {
        match event {
            TerminalEvent::ClipboardWrite(text) => {
                assert!(
                    outcome.is_none(),
                    "retained writes precede the final outcome"
                );
                received.push(text);
            }
            TerminalEvent::Exited(exit) => outcome = Some(exit),
            _ => {}
        }
    }
    assert!(received.len() >= 32);
    for (index, text) in received.iter().enumerate() {
        assert_eq!(text, &format!("CLIP{index}"));
    }
    let outcome = outcome.expect("reserved exit outcome");
    assert_eq!(outcome.code, Some(7));
    assert!(!outcome.requested);
    session.begin_shutdown().unwrap().unwrap().wait().unwrap();
}
```

- [ ] **Step 4: Run tests, confirm pass** —
  - `TERM=dumb cargo test -p sprite-term --locked --offline worker::` (new `notice_tests`, existing `bell_tests`, `closing_regressions`)
  - `TERM=dumb cargo test -p sprite-term --locked --offline --test event_backpressure --test integration_metadata --test clipboard`
  - `TERM=dumb cargo test -p sprite-app --locked --offline --lib -- submission_regressions observation::panes terminal_view::tests::settings_callback terminal_view::tests::disconnected_worker terminal_view::tests::focus_refused_under_a_full_queue fallback_admission_tests`

- [ ] **Step 5: Commit**
```bash
git add crates/sprite-term/src/worker/mod.rs crates/sprite-term/src/worker/start.rs \
  crates/sprite-term/tests/event_backpressure.rs crates/sprite-app/src/lib.rs \
  crates/sprite-app/src/test_event_pressure.rs crates/sprite-app/src/terminal_view/submission_regressions.rs \
  crates/sprite-app/src/observation/panes.rs crates/sprite-app/src/terminal_view/tests.rs \
  crates/sprite-app/src/terminal_view/theme.rs
git commit -m "fix(term): publish only the latest title and working directory

Titles and working directories now keep their latest value until the
notices are published, as the bell already did. Event-pressure fixtures
that relied on one event per title use OSC 52 clipboard writes instead.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

#### Cycle C — one bounded pass per wake, one capture per pass (R-T1)

- [ ] **Step 1: Write the failing test** — first add the test-only constructor to `crates/sprite-term/src/pty_unix.rs`, directly after `impl OutputChunk { … }` (line 77):
```rust
#[cfg(test)]
impl OutputChunk {
    /// A chunk that belongs to no pump, so a test can queue output for a
    /// worker directly. Dropping it returns its buffer nowhere and wakes no one.
    pub(crate) fn detached(bytes: &[u8]) -> Self {
        let (returned, _closed) = sync_channel(1);
        let (wake, _peer) = UnixStream::pair().expect("wake pair");
        Self {
            permit: Permit {
                buffer: Some(bytes.to_vec()),
                pool: BufferPool {
                    returned,
                    wake: Arc::new(wake),
                },
            },
            len: bytes.len(),
        }
    }
}
```
Then append to `crates/sprite-term/src/worker/mod.rs` after `mod notice_tests`:
```rust
#[cfg(test)]
mod coalescing_tests {
    use super::*;
    use std::time::Duration;

    /// Runs `body` on its own thread, failing rather than hanging the suite if
    /// it has not finished in twenty seconds.
    fn within_watchdog(body: impl FnOnce() + Send + 'static) {
        let (done, finished) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            body();
            let _ = done.send(());
        });
        match finished.recv_timeout(Duration::from_secs(20)) {
            Ok(()) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                panic!("watchdog: the worker test made no progress for twenty seconds")
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                panic!("the worker test failed; its assertion is printed above")
            }
        }
    }

    /// A real worker over a silent child whose inbox already holds output when
    /// it starts.
    ///
    /// Events are published with no spare room, so the worker waits at each
    /// publication until the test takes it. The test therefore decides when the
    /// snapshot slot is empty, and sees every snapshot the worker publishes.
    struct Fixture {
        commands: SyncSender<Message>,
        events: crate::event_mailbox::Receiver,
        snapshots: async_channel::Receiver<Arc<SnapshotBundle>>,
        shutdown: Arc<AtomicBool>,
        worker: JoinHandle<Option<pty_unix::SessionProcesses>>,
    }

    impl Fixture {
        fn start(chunks: &[Vec<u8>]) -> Self {
            let (commands, inbox) = std::sync::mpsc::sync_channel(crate::WORKER_QUEUE_CAPACITY);
            for chunk in chunks {
                assert!(
                    commands
                        .try_send(Message::PtyOutput(pty_unix::OutputChunk::detached(chunk)))
                        .is_ok(),
                    "the inbox holds every queued chunk"
                );
            }
            let (events, receiver) = crate::event_mailbox::bounded(0);
            let (published, snapshots) = async_channel::bounded(1);
            let shutdown = Arc::new(AtomicBool::new(false));
            let worker = std::thread::spawn({
                let commands = commands.clone();
                let shutdown = Arc::clone(&shutdown);
                move || {
                    run(
                        SessionConfig::command(
                            "/bin/sh",
                            vec!["-c".into(), "exec sleep 30".into()],
                        ),
                        commands,
                        inbox,
                        events,
                        published,
                        shutdown,
                        Arc::new(crate::ForegroundWatch::default()),
                    )
                }
            });
            Self {
                commands,
                events: receiver,
                snapshots,
                shutdown,
                worker,
            }
        }

        fn next_event(&self) -> TerminalEvent {
            self.events
                .next_blocking()
                .expect("the worker is still publishing")
        }

        fn next_generation(&self) -> u64 {
            self.snapshots
                .recv_blocking()
                .expect("the worker is still capturing")
                .generation
        }

        /// Shuts the worker down; returns every event published after this
        /// point and the generation of any snapshot left unread.
        fn finish(self) -> (Vec<TerminalEvent>, Option<u64>) {
            self.shutdown.store(true, Ordering::SeqCst);
            let _ = self.commands.send(Message::Shutdown);
            if let Some(processes) = self.worker.join().expect("the worker did not panic") {
                finish_shutdown(processes, std::time::Instant::now());
            }
            let mut remaining = Vec::new();
            while let Ok(event) = self.events.next_blocking() {
                remaining.push(event);
            }
            let unread = self.snapshots.try_recv().ok().map(|bundle| bundle.generation);
            (remaining, unread)
        }
    }

    fn title_of(event: TerminalEvent) -> String {
        match event {
            TerminalEvent::TitleChanged(Some(title)) => title,
            other => panic!("expected a title, got {other:?}"),
        }
    }

    /// Sixteen chunks already waiting are one pass: one snapshot and one title.
    /// The seventeenth is the next pass.
    #[test]
    fn sixteen_queued_chunks_produce_one_snapshot() {
        within_watchdog(|| {
            let chunks: Vec<Vec<u8>> = (0..17)
                .map(|index| format!("\x1b]2;title-{index}\x07line {index}\r\n").into_bytes())
                .collect();
            let fixture = Fixture::start(&chunks);
            assert!(matches!(fixture.next_event(), TerminalEvent::Ready));
            assert_eq!(fixture.next_generation(), 0, "the frame before any output");
            assert_eq!(title_of(fixture.next_event()), "title-15");
            assert_eq!(
                fixture.next_generation(),
                16,
                "one snapshot for the first sixteen chunks"
            );
            assert_eq!(title_of(fixture.next_event()), "title-16");
            assert_eq!(fixture.next_generation(), 17);
            let (remaining, unread) = fixture.finish();
            assert!(
                !remaining
                    .iter()
                    .any(|event| matches!(event, TerminalEvent::TitleChanged(_))),
                "{remaining:?}"
            );
            assert_eq!(unread, None, "nothing was left to capture");
        });
    }

    /// A pass also ends once it has parsed 16 KiB of output.
    #[test]
    fn a_pass_stops_at_sixteen_kibibytes_of_output() {
        within_watchdog(|| {
            let chunks: Vec<Vec<u8>> = (0..3)
                .map(|index| {
                    let mut chunk = format!("\x1b]2;bytes-{index}\x07").into_bytes();
                    chunk.resize(8 * 1024, b'x');
                    chunk
                })
                .collect();
            let fixture = Fixture::start(&chunks);
            assert!(matches!(fixture.next_event(), TerminalEvent::Ready));
            assert_eq!(fixture.next_generation(), 0);
            assert_eq!(title_of(fixture.next_event()), "bytes-1");
            assert_eq!(fixture.next_generation(), 2, "two 8 KiB chunks fill a pass");
            assert_eq!(title_of(fixture.next_event()), "bytes-2");
            assert_eq!(fixture.next_generation(), 3);
            let (_, unread) = fixture.finish();
            assert_eq!(unread, None);
        });
    }
}
```

- [ ] **Step 2: Run it and confirm it fails** — `TERM=dumb cargo test -p sprite-term --locked --offline worker::coalescing_tests`. Expected: both tests panic with `assertion `left == right` failed` — `left: "title-0"`, `right: "title-15"` (and `"bytes-0"` vs `"bytes-1"`), reported through the watchdog's "the worker test failed" panic.

- [ ] **Step 3: Implement** — all in `crates/sprite-term/src/worker/mod.rs`.

3a. After `type PtyWriteError = …;` (line 44) add:
```rust

/// How much already-queued work one pass of the worker takes before it
/// captures: sixteen messages, or sixteen KiB of output, whichever comes first.
/// A burst then costs one snapshot rather than one per chunk, and a capture is
/// still never postponed behind an unbounded queue.
const BATCH_MESSAGES: usize = 16;
const BATCH_OUTPUT_BYTES: usize = 16 * 1024;

/// What one pass has taken so far, measured against the bounds above.
#[derive(Default)]
struct Batch {
    messages: usize,
    output_bytes: usize,
}

impl Batch {
    fn admit(&mut self, message: &Message) {
        self.messages += 1;
        if let Message::PtyOutput(chunk) = message {
            self.output_bytes += chunk.as_bytes().len();
        }
    }

    fn is_full(&self) -> bool {
        self.messages >= BATCH_MESSAGES || self.output_bytes >= BATCH_OUTPUT_BYTES
    }
}
```

3b. Replace lines 256-307 — from `impl Session {` through the end of the `Message::PtyOutput(chunk) => { … }` arm (the arm ending `if !events.publish(batch) { return Stop(()); } }`) — with:
```rust
impl Session {
    /// Handles one message, then whatever was already queued behind it, and
    /// captures once for the whole pass.
    ///
    /// A pass takes only what is already waiting — it never waits for more —
    /// and stops at `BATCH_MESSAGES` messages or `BATCH_OUTPUT_BYTES` of output.
    fn handle(&mut self, first: Message) -> Flow {
        let mut batch = Batch::default();
        let mut next = Some(first);
        while let Some(message) = next.take() {
            batch.admit(&message);
            // What earlier output in this pass raised is published before
            // anything else is handled, so a command's own event never
            // overtakes the notices that preceded it.
            if !matches!(message, Message::PtyOutput(_)) {
                self.publish_notices()?;
            }
            self.apply(message)?;
            if batch.is_full() || self.runtime.shutdown.load(Ordering::SeqCst) {
                break;
            }
            next = self.runtime.inbox.try_recv().ok();
        }
        self.publish_notices()?;
        self.capture()
    }

    /// Publishes, as one batch, everything parsing has raised since the last
    /// publication: the first refused reply, the latest title and working
    /// directory, at most one bell, and every accepted clipboard write.
    fn publish_notices(&mut self) -> Flow {
        let mut batch = Vec::new();
        if let Some(error) = self.write_error.borrow_mut().take() {
            batch.push(TerminalEvent::Error(error));
        }
        batch.extend(self.notices.borrow_mut().take());
        batch.extend(
            self.clipboard_pending
                .borrow_mut()
                .drain(..)
                .map(TerminalEvent::ClipboardWrite),
        );
        // Nothing to say is not a publication: the mailbox lock and the
        // receiver's wake are spent only on something to deliver.
        if batch.is_empty() {
            return Continue(());
        }
        if self.runtime.events.publish(batch) {
            Continue(())
        } else {
            Stop(())
        }
    }

    /// Applies one message to the terminal. Capturing is the pass's business.
    fn apply(&mut self, message: Message) -> Flow {
        let Self {
            owned:
                Owned {
                    projector,
                    encoder,
                    mouse_encoder,
                    terminal,
                },
            runtime:
                Runtime {
                    started: Started { master, .. },
                    events,
                    exit_status,
                    pump_stopped,
                    fatal,
                    ..
                },
            input,
            commands,
            focused,
            pending,
            size,
            has_selection,
            ..
        } = self;
        match message {
            Message::PtyOutput(chunk) => {
                // One chunk, one mutation, one generation. What the parser
                // raised on the way is published when the pass ends.
                terminal.vt_write(chunk.as_bytes());
                pending.mutated();
                drop(chunk);
            }
```
Every other arm (`CaptureRequested` through `Shutdown`) stays exactly as it is, now inside `apply`.

3c. Replace the tail of the old `handle` (lines 609-613):
```rust
            Message::Shutdown => return Stop(()),
        }

        self.capture()
    }
```
with
```rust
            Message::Shutdown => return Stop(()),
        }

        Continue(())
    }
```

3d. In `drain_accepted_output`, replace lines 657-663:
```rust
            if matches!(
                message,
                Message::PtyOutput(_) | Message::PumpStopped(_) | Message::ChildExited(_)
            ) && self.handle(message).is_break()
            {
                break;
            }
```
with
```rust
            // One message at a time and published as it goes: this drain must
            // not pull commands out of the queue the way a pass would.
            if matches!(
                message,
                Message::PtyOutput(_) | Message::PumpStopped(_) | Message::ChildExited(_)
            ) && (self.apply(message).is_break() || self.publish_notices().is_break())
            {
                break;
            }
```

3e. `crates/sprite-term/src/session.rs:56-59` — keep the comment true:
```rust
    /// Tells the worker the slot is free again, without ever blocking the
    /// consumer. A full queue already holds a mutation that will wake the
    /// worker, and the worker rechecks for pending work after every pass;
    /// an idle worker has room for this request.
```

3f. Docs — append one line, as its own final paragraph, to the end of each of `docs/adr/0021-bound-terminal-delivery-and-cancel-event-pressure.md` and `docs/adr/0026-bound-event-delivery-and-recover-ui-admission.md`:
```markdown

Amended (bug-class audit): notices are published per worker pass, not per parser chunk.
```

- [ ] **Step 4: Run tests, confirm pass** —
  - `TERM=dumb cargo test -p sprite-term --locked --offline worker::`
  - `TERM=dumb cargo test -p sprite-term --locked --offline` (whole crate: lifecycle natural-exit drain, `event_backpressure`, `session_output`, `snapshot_waiting`, `input_backpressure` exercise the pass)
  - `TERM=dumb cargo test -p sprite-app --locked --offline terminal_view::`

- [ ] **Step 5: Commit**
```bash
git add crates/sprite-term/src/pty_unix.rs crates/sprite-term/src/worker/mod.rs crates/sprite-term/src/session.rs \
  docs/adr/0021-bound-terminal-delivery-and-cancel-event-pressure.md \
  docs/adr/0026-bound-event-delivery-and-recover-ui-admission.md
git commit -m "perf(term): capture once per bounded pass of queued work

After handling a message the worker takes what is already queued, up to
16 messages or 16 KiB of output, publishes the pass's notices once and
captures once, so a burst of output produces one snapshot.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 10: The PNG decoder emits RGBA8 for every colour type and keeps no pixels between images (BCA-07, BCA-25)

**Files:**
- Modify: `crates/sprite-term/src/png_decoder.rs:43-110` (struct, `new`, `decode_png`; new `widen_to_rgba`), tests `:112-222`
- Modify: `crates/sprite-term/src/test_allocations.rs:6-51,68-77` (count freed bytes and the largest block)
- Test: inline `mod tests` in `png_decoder.rs` (existing)

**Interfaces:**
- Consumes: nothing.
- Produces: `PngDecoder` has no `buffer` field; `pub(crate) fn PngDecoder::new(limit: u64) -> Self` unchanged. `test_allocations::Sample` gains `pub freed: usize` and `pub largest: usize` (other users construct it only through `measure`, so they are unaffected).

- [ ] **Step 1: Write the failing test**

1a. Test scaffolding in `crates/sprite-term/src/test_allocations.rs` (counters the retention test needs). Replace lines 6-10 (`Sample`) with:
```rust
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Sample {
    pub allocations: usize,
    pub bytes: usize,
    /// Bytes released on the measured thread, a reallocation's old block
    /// included. With `bytes` this says what the work left allocated.
    pub freed: usize,
    /// The largest single block requested.
    pub largest: usize,
}
```
Replace `fn record` (lines 21-29) with:
```rust
fn record(bytes: usize) {
    let _ = SAMPLE.try_with(|sample| {
        if let Some(mut count) = sample.get() {
            count.allocations += 1;
            count.bytes += bytes;
            count.largest = count.largest.max(bytes);
            sample.set(Some(count));
        }
    });
}

fn record_free(bytes: usize) {
    let _ = SAMPLE.try_with(|sample| {
        if let Some(mut count) = sample.get() {
            count.freed += bytes;
            sample.set(Some(count));
        }
    });
}
```
Replace `realloc` and `dealloc` (lines 43-50) with:
```rust
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        record(new_size);
        record_free(layout.size());
        unsafe { System.realloc(ptr, layout, new_size) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        record_free(layout.size());
        unsafe { System.dealloc(ptr, layout) }
    }
```
In `counts_real_allocations_and_ignores_work_outside_measurement` (lines 68-77) add after `assert_eq!(sample.bytes, 29);`:
```rust
    assert_eq!(sample.freed, 29, "the vector was dropped inside the measurement");
    assert_eq!(sample.largest, 29);
```

1b. In `crates/sprite-term/src/png_decoder.rs` `mod tests`, replace `an_image_larger_than_the_limit_is_refused_before_it_is_decoded` (lines 166-179) — it reads the `buffer` field this task removes — with:
```rust
    #[test]
    fn an_image_larger_than_the_limit_is_refused_before_it_is_decoded() {
        // Room for far less than a 256x256 RGBA image.
        let mut decoder = PngDecoder::new(1024);
        let oversized = png_bytes(256, 256);
        let (refused, sample) =
            crate::test_allocations::measure(|| decode(&mut decoder, &oversized));
        assert!(refused.is_none());
        assert!(
            sample.largest < 256 * 256 * 4,
            "refused before a buffer was allocated for it: {sample:?}"
        );

        // The same decoder still accepts something that fits, so the limit
        // refuses an image rather than disabling the decoder.
        assert!(decode(&mut decoder, &png_bytes(4, 4)).is_some());
    }
```

1c. Add helpers and the two new tests to the same `mod tests` (after `png_bytes`):
```rust
    /// A PNG of one colour type and depth, every pixel `samples`, built by the
    /// same crate that reads it back. Indexed images get a two-entry palette
    /// whose entry 1 is `10 20 30`.
    fn encoded(
        color: png::ColorType,
        depth: png::BitDepth,
        width: u32,
        height: u32,
        samples: &[u16],
    ) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut out, width, height);
            encoder.set_color(color);
            encoder.set_depth(depth);
            if color == png::ColorType::Indexed {
                encoder.set_palette(vec![0, 0, 0, 0x10, 0x20, 0x30]);
            }
            let mut writer = encoder.write_header().expect("write the header");
            writer
                .write_image_data(&uniform_rows(width, height, depth, samples))
                .expect("write the pixels");
        }
        out
    }

    /// `width × height` copies of one pixel's samples at `depth` bits, each
    /// row starting on a byte boundary, as PNG lays them out.
    fn uniform_rows(width: u32, height: u32, depth: png::BitDepth, samples: &[u16]) -> Vec<u8> {
        let bits = depth as usize;
        let mut data = Vec::new();
        for _ in 0..height {
            let mut packed = 0_u16;
            let mut filled = 0;
            for _ in 0..width {
                for &sample in samples {
                    match bits {
                        16 => data.extend_from_slice(&sample.to_be_bytes()),
                        8 => data.push(sample as u8),
                        _ => {
                            packed = (packed << bits) | sample;
                            filled += bits;
                            if filled == 8 {
                                data.push(packed as u8);
                                packed = 0;
                                filled = 0;
                            }
                        }
                    }
                }
            }
            if filled > 0 {
                data.push((packed << (8 - filled)) as u8);
            }
        }
        data
    }

    /// libghostty accepts RGBA8 only, so every colour type at every depth PNG
    /// allows must arrive as four bytes a pixel with the right values.
    #[test]
    fn every_colour_type_and_bit_depth_decodes_to_rgba8() {
        use png::BitDepth::{Eight, Four, One, Sixteen, Two};
        use png::ColorType::{Grayscale, GrayscaleAlpha, Indexed, Rgb, Rgba};
        let (width, height) = (3_u32, 2_u32);
        let cases: &[(png::ColorType, png::BitDepth, &[u16], [u8; 4])] = &[
            (Grayscale, One, &[1], [0xFF, 0xFF, 0xFF, 0xFF]),
            (Grayscale, Two, &[2], [0xAA, 0xAA, 0xAA, 0xFF]),
            (Grayscale, Four, &[8], [0x88, 0x88, 0x88, 0xFF]),
            (Grayscale, Eight, &[0x80], [0x80, 0x80, 0x80, 0xFF]),
            (Grayscale, Sixteen, &[0x8000], [0x80, 0x80, 0x80, 0xFF]),
            (Rgb, Eight, &[0x10, 0x20, 0x30], [0x10, 0x20, 0x30, 0xFF]),
            (Rgb, Sixteen, &[0x1000, 0x2000, 0x3000], [0x10, 0x20, 0x30, 0xFF]),
            (Indexed, One, &[1], [0x10, 0x20, 0x30, 0xFF]),
            (Indexed, Two, &[1], [0x10, 0x20, 0x30, 0xFF]),
            (Indexed, Four, &[1], [0x10, 0x20, 0x30, 0xFF]),
            (Indexed, Eight, &[1], [0x10, 0x20, 0x30, 0xFF]),
            (GrayscaleAlpha, Eight, &[0x80, 0x40], [0x80, 0x80, 0x80, 0x40]),
            (GrayscaleAlpha, Sixteen, &[0x8000, 0x4000], [0x80, 0x80, 0x80, 0x40]),
            (Rgba, Eight, &[0x10, 0x20, 0x30, 0x40], [0x10, 0x20, 0x30, 0x40]),
            (Rgba, Sixteen, &[0x1000, 0x2000, 0x3000, 0x4000], [0x10, 0x20, 0x30, 0x40]),
        ];
        let mut decoder = PngDecoder::new(1024 * 1024);
        let allocator = Allocator::GLOBAL;
        for &(color, depth, samples, rgba) in cases {
            let png = encoded(color, depth, width, height, samples);
            let image = decoder
                .decode_png(&allocator, &png)
                .unwrap_or_else(|| panic!("{color:?} at {depth:?} must decode"));
            assert_eq!((image.width, image.height), (width, height), "{color:?} at {depth:?}");
            assert_eq!(
                image.data.len(),
                (width * height * 4) as usize,
                "{color:?} at {depth:?} must be four bytes a pixel"
            );
            for pixel in image.data.chunks_exact(4) {
                assert_eq!(pixel, &rgba[..], "{color:?} at {depth:?}");
            }
        }
    }

    /// Decoding a large image must not leave its size allocated afterwards.
    #[test]
    fn no_pixels_outlive_the_decode_that_produced_them() {
        let mut decoder = PngDecoder::new(64 * 1024 * 1024);
        let large = png_bytes(512, 512);
        let small = png_bytes(1, 1);
        let ((), sample) = crate::test_allocations::measure(|| {
            assert!(decode(&mut decoder, &large).is_some());
            assert!(decode(&mut decoder, &small).is_some());
        });
        let retained = sample.bytes.saturating_sub(sample.freed);
        assert!(
            retained < 64 * 1024,
            "{retained} bytes outlived the decodes: {sample:?}"
        );
    }
```

- [ ] **Step 2: Run it and confirm it fails** — `TERM=dumb cargo test -p sprite-term --locked --offline --lib -- png_decoder::tests test_allocations`. Expected: `every_colour_type_and_bit_depth_decodes_to_rgba8` fails `Grayscale at One must be four bytes a pixel` (`left: 12`, `right: 24`); `no_pixels_outlive_the_decode_that_produced_them` fails with about 1 MiB retained; the rewritten limit test and `counts_real_allocations…` pass.

- [ ] **Step 3: Implement** — replace lines 43-110 of `crates/sprite-term/src/png_decoder.rs` (from `/// Decodes PNG transmissions…` through the end of `impl DecodePng for PngDecoder`) with:
```rust
/// Decodes PNG transmissions into the RGBA pixels libghostty expects.
///
/// Holds no pixels between images: each decode allocates what that image
/// needs and releases it before returning, so one large image does not pin its
/// size for as long as the pane lives.
pub(crate) struct PngDecoder {
    /// The most decoded bytes this decoder will produce for one image.
    ///
    /// Matches the pane's storage limit: an image too large to be *kept* should
    /// never be decoded, because decoding is where the memory is actually spent.
    limit: usize,
}

impl PngDecoder {
    pub(crate) fn new(limit: u64) -> Self {
        Self {
            limit: usize::try_from(limit).unwrap_or(usize::MAX),
        }
    }
}

impl DecodePng for PngDecoder {
    fn decode_png<'alloc>(
        &mut self,
        alloc: &'alloc Allocator<'_>,
        data: &[u8],
    ) -> Option<DecodedImage<'alloc>> {
        let mut decoder = png::Decoder::new(std::io::Cursor::new(data));
        // Palettes gain an alpha channel, low-depth grayscale widens to eight
        // bits and sixteen-bit channels are reduced. Grayscale still arrives as
        // gray and alpha, two bytes a pixel, so it is widened to RGBA below.
        decoder.set_transformations(png::Transformations::ALPHA | png::Transformations::STRIP_16);

        let mut reader = decoder.read_info().ok()?;
        let (width, height) = reader.info().size();

        // libghostty stores four bytes a pixel whatever the PNG held, so that
        // is what the limit is checked against — and *before* allocating: the
        // declared size of a PNG is attacker-controlled, and a decoder that
        // allocates first and checks afterwards can be asked for a gigabyte.
        let stored = usize::try_from(width)
            .ok()?
            .checked_mul(usize::try_from(height).ok()?)?
            .checked_mul(4)?;
        let needed = reader.output_buffer_size()?;
        if stored == 0 || stored > self.limit || needed > stored {
            return None;
        }

        // Sized, not merely reserved: `next_frame` writes into the slice the
        // vector's *length* describes, and a reserved-but-empty vector is a
        // zero-length slice. This is the upstream bug this decoder exists to
        // avoid repeating.
        let mut decoded = vec![0_u8; needed];
        let info = reader.next_frame(&mut decoded).ok()?;
        reader.finish().ok()?;

        let produced = info.buffer_size();
        if info.bit_depth != png::BitDepth::Eight || produced == 0 || produced > decoded.len() {
            return None;
        }

        // The buffer must come from libghostty's allocator: it takes ownership
        // and frees it with the same allocator.
        let pixels = usize::try_from(info.width)
            .ok()?
            .checked_mul(usize::try_from(info.height).ok()?)?;
        let mut bytes = Bytes::new_with_alloc(alloc, pixels.checked_mul(4)?).ok()?;
        if !widen_to_rgba(info.color_type, &decoded[..produced], &mut bytes) {
            return None;
        }

        Some(DecodedImage {
            width: info.width,
            height: info.height,
            data: bytes,
        })
    }
}

/// Writes eight-bit `samples` of `color` into `rgba`, four bytes a pixel.
///
/// Returns `false`, writing nothing useful, when the two do not describe the
/// same number of pixels or the colour type is not one the transformations
/// above can produce.
fn widen_to_rgba(color: png::ColorType, samples: &[u8], rgba: &mut [u8]) -> bool {
    let channels = match color {
        png::ColorType::Rgba => 4,
        png::ColorType::Rgb => 3,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Grayscale => 1,
        // `ALPHA` expands every palette, so an index here is not a colour.
        png::ColorType::Indexed => return false,
    };
    if samples.len() % channels != 0 || samples.len() / channels != rgba.len() / 4 {
        return false;
    }
    for (pixel, out) in samples.chunks_exact(channels).zip(rgba.chunks_exact_mut(4)) {
        let (rgb, alpha) = match *pixel {
            [red, green, blue, alpha] => ([red, green, blue], alpha),
            [red, green, blue] => ([red, green, blue], u8::MAX),
            [gray, alpha] => ([gray; 3], alpha),
            [gray] => ([gray; 3], u8::MAX),
            _ => return false,
        };
        out[..3].copy_from_slice(&rgb);
        out[3] = alpha;
    }
    true
}
```
Also update the existing test `decoding_twice_works_as_well_as_once`'s second message (line 157) from `"a reused buffer grows for a larger image"` to `"a larger image after a smaller one decodes whole"` (no buffer is reused any more).

- [ ] **Step 4: Run tests, confirm pass** — `TERM=dumb cargo test -p sprite-term --locked --offline --lib -- png_decoder test_allocations capture_benchmark pty_unix::tests::steady_state` then `TERM=dumb cargo test -p sprite-term --locked --offline --test graphics_transfer --test graphics_fixtures --test png_decoder_leak --test graphics_policy`.

- [ ] **Step 5: Commit**
```bash
git add crates/sprite-term/src/png_decoder.rs crates/sprite-term/src/test_allocations.rs
git commit -m "fix(term): decode every PNG colour type to RGBA8 and keep no scratch

Grayscale PNGs decoded to two bytes a pixel and libghostty rejected them.
Widen every output to RGBA8, check the limit against the stored RGBA size,
and allocate the decode buffer per image instead of keeping the largest.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 11: A selection gesture's anchor follows its content (BCA-26)

The press does not select anything (see `Drag::moved` in `crates/sprite-app/src/terminal_view/input.rs:20-30`), so marking the gesture start cannot be a flag on `Select`: the press sends a new `TerminalCommand::BeginSelection { anchor }` in place of its `ClearSelection`. The worker pins that cell with `Terminal::track_grid_ref` and every character-mode `Select` that follows extends from the pinned content. A pin libghostty reports as lost (pruned page) clears the selection.

**Files:**
- Modify: `crates/sprite-term/src/command.rs:122-132` (`Select` doc, new `BeginSelection`)
- Modify: `crates/sprite-term/src/input/mouse.rs:1-6` (imports), `:227-271` (`apply_selection`), new `SelectionAnchor`, `track_selection_anchor`
- Modify: `crates/sprite-term/src/worker/mod.rs:34-37` (imports), `:87-94` (`Owned`), the `Select`/`ClearSelection` arms (`:405-433` at master; inside `Session::apply` after Task 9)
- Modify: `crates/sprite-term/src/worker/start.rs:182-187` (`Owned` construction)
- Modify: `crates/sprite-app/src/terminal_view/render.rs:491-502` (left press)
- Test: `crates/sprite-term/tests/selection.rs` (integration, existing helpers `session`, `at`, `EventPump`, `SnapshotPump`, `pane_text`); `crates/sprite-app/src/terminal_view/tests.rs` (GPUI, existing helper `wait_for_bundle`)

**Interfaces:**
- Consumes: Task 9's `Session::apply` (the arms below live there).
- Produces: `TerminalCommand::BeginSelection { anchor: CellPosition }` (public); `pub(crate) enum SelectionAnchor<'a> { Cell(CellPosition), Tracked(&'a TrackedGridRef) }`; `pub(crate) fn apply_selection(terminal: &Terminal<'_, '_>, anchor: SelectionAnchor<'_>, head: CellPosition, mode: SelectionMode, rectangle: bool) -> Result<bool, SessionError>` (`false` = anchor lost, selection cleared); `pub(crate) fn track_selection_anchor(&Terminal<'_, '_>, CellPosition) -> Result<TrackedGridRef, SessionError>`; `Owned.selection_anchor: Option<libghostty_vt::screen::TrackedGridRef>`.

#### Cycle A — Terminal Core tracks the anchor

- [ ] **Step 1: Write the failing test** — in `crates/sprite-term/tests/selection.rs`, change the import at lines 9-11 to:
```rust
use sprite_term::{
    CellPosition, SelectionMode, SessionConfig, SnapshotBundle, TerminalCommand, TerminalEvent,
    TerminalSession,
};
```
and append:
```rust
/// The viewport row whose text is exactly `text`, if it is on screen.
fn row_of(bundle: &SnapshotBundle, text: &str) -> Option<u16> {
    bundle
        .pane
        .rows
        .iter()
        .position(|row| row.text.trim_end() == text)
        .and_then(|row| u16::try_from(row).ok())
}

/// The text the next `SelectionCopied` carries.
fn copied(events: &EventPump) -> String {
    loop {
        match events.next() {
            TerminalEvent::SelectionCopied(text) => return text,
            TerminalEvent::Error(error) => panic!("selection failed: {error}"),
            _ => {}
        }
    }
}

/// The press that starts a gesture pins its anchor to the content under it,
/// so a drag that continues after output scrolled still extends from that
/// content, not from whatever now sits where it was.
#[test]
fn a_gesture_anchor_follows_its_content_while_output_scrolls() {
    let sprite_term::Spawned {
        mut session,
        events,
        snapshots,
    } = session(
        "stty -echo; seq 1 100; printf 'ANCHOR-TEXT\\nREADY'; IFS= read -r go; \
         printf '\\nafter-1\\nafter-2\\nafter-3'; sleep 30",
    );
    let events = EventPump::new(events);
    let snapshots = SnapshotPump::new(snapshots);
    events.expect_ready();

    // READY is the last thing printed before the child waits, so once it is
    // on screen nothing else will move.
    let before = snapshots.wait_for("the anchor line", |bundle| row_of(bundle, "READY").is_some());
    let pressed = at(row_of(&before, "ANCHOR-TEXT").expect("anchor row"), 0);
    session
        .send(TerminalCommand::BeginSelection { anchor: pressed })
        .expect("press");

    session
        .send(TerminalCommand::Input(b"go\n".to_vec()))
        .expect("release the child");
    let after = snapshots.wait_for("the scrolled output", |bundle| {
        row_of(bundle, "after-3").is_some()
    });
    let moved = row_of(&after, "ANCHOR-TEXT").expect("the anchor line is still on screen");
    assert!(moved < pressed.row, "output scrolled the anchor line up");

    session
        .send(TerminalCommand::Select {
            anchor: pressed,
            head: at(moved, 10),
            mode: SelectionMode::Character,
            rectangle: false,
        })
        .expect("extend the gesture");
    session
        .send(TerminalCommand::CopySelection)
        .expect("copy the selection");
    assert_eq!(copied(&events), "ANCHOR-TEXT");
}

/// When output evicts the anchored content from scrollback, the gesture
/// selects nothing rather than re-anchoring on whatever replaced it.
#[test]
fn a_gesture_whose_anchor_was_evicted_selects_nothing() {
    let mut config = SessionConfig::command(
        "/bin/sh",
        args(&[
            "-c",
            "stty -echo; seq 1 100; printf 'ANCHOR-TEXT\\nREADY'; IFS= read -r go; \
             seq 1 20000; printf 'STREAM-DONE'; sleep 30",
        ]),
    );
    // The smallest nonzero budget: libghostty keeps scrollback in whole pages
    // and prunes the oldest, which is what evicts the anchored line. (A zero
    // budget rotates rows in place instead; see the TSP's drafter notes.)
    config.scrollback_bytes = 4 * 1024;
    let sprite_term::Spawned {
        mut session,
        events,
        snapshots,
    } = TerminalSession::spawn(config).expect("spawn session");
    let events = EventPump::new(events);
    let snapshots = SnapshotPump::new(snapshots);
    events.expect_ready();

    let before = snapshots.wait_for("the anchor line", |bundle| row_of(bundle, "READY").is_some());
    let pressed = at(row_of(&before, "ANCHOR-TEXT").expect("anchor row"), 0);
    session
        .send(TerminalCommand::BeginSelection { anchor: pressed })
        .expect("press");
    session
        .send(TerminalCommand::Input(b"go\n".to_vec()))
        .expect("release the child");
    snapshots.wait_for("the end of the stream", |bundle| {
        row_of(bundle, "STREAM-DONE").is_some()
    });

    session
        .send(TerminalCommand::Select {
            anchor: pressed,
            head: at(0, 3),
            mode: SelectionMode::Character,
            rectangle: false,
        })
        .expect("extend the gesture");
    session
        .send(TerminalCommand::CopySelection)
        .expect("copy the selection");
    assert_eq!(copied(&events), "", "an evicted anchor selects nothing");
}
```

- [ ] **Step 2: Run it and confirm it fails** — `TERM=dumb cargo test -p sprite-term --locked --offline --test selection a_gesture`. Expected: compile error `no variant named `BeginSelection` found for enum `TerminalCommand``.

- [ ] **Step 3: Implement**

3a. `crates/sprite-term/src/command.rs`, replace lines 122-132:
```rust
    /// Replace the selection. Selection lives here rather than in the
    /// application because libghostty models it over the whole screen including
    /// scrollback, and because a cell is only reported as selected when the
    /// terminal itself holds the selection.
    ///
    /// In character mode, while a gesture begun with `BeginSelection` is under
    /// way, the selection extends from that gesture's pinned anchor and this
    /// `anchor` is not used; it describes a selection made without a press.
    Select {
        anchor: CellPosition,
        head: CellPosition,
        mode: SelectionMode,
        rectangle: bool,
    },
    /// The press that starts a selection gesture.
    ///
    /// Drops whatever was selected and pins `anchor` to the content under it,
    /// so every `Select` that follows extends from that content wherever output
    /// has since moved it. If that content is evicted from scrollback, the
    /// gesture selects nothing rather than re-anchoring on whatever replaced
    /// it. The gesture lasts until the next `BeginSelection` or
    /// `ClearSelection`.
    BeginSelection { anchor: CellPosition },
    ClearSelection,
```

3b. `crates/sprite-term/src/input/mouse.rs`: replace line 6 `use libghostty_vt::{Terminal, key};` with
```rust
use libghostty_vt::screen::TrackedGridRef;
use libghostty_vt::terminal::{Point, PointCoordinate};
use libghostty_vt::{Terminal, key};
```
and replace lines 227-271 (`apply_selection` and its doc) with:
```rust
/// Where a character-mode selection extends from.
pub(crate) enum SelectionAnchor<'a> {
    /// A viewport cell, resolved now.
    Cell(CellPosition),
    /// The content a gesture's press landed on, wherever output has moved it.
    Tracked(&'a TrackedGridRef),
}

fn viewport_point(position: CellPosition) -> Point {
    Point::Viewport(PointCoordinate {
        x: position.column,
        y: u32::from(position.row),
    })
}

/// Pins a gesture's anchor to the content under a viewport cell, so it follows
/// that content through scrolling, scrollback pruning and reflow.
pub(crate) fn track_selection_anchor(
    terminal: &Terminal<'_, '_>,
    anchor: CellPosition,
) -> Result<TrackedGridRef, SessionError> {
    terminal
        .track_grid_ref(viewport_point(anchor))
        .map_err(|error| SessionError::new("selection_anchor", error))
}

/// Installs a selection whose head is a viewport cell.
///
/// Word and line modes delegate to libghostty so Sprite agrees with Ghostty on
/// what a word or a wrapped line is, rather than inventing its own boundaries.
///
/// Returns `false` when a tracked anchor has lost its content; the selection
/// is then cleared rather than re-anchored on whatever took its place.
pub(crate) fn apply_selection(
    terminal: &Terminal<'_, '_>,
    anchor: SelectionAnchor<'_>,
    head: CellPosition,
    mode: SelectionMode,
    rectangle: bool,
) -> Result<bool, SessionError> {
    use libghostty_vt::selection::{SelectLineOptions, SelectWordOptions, Selection};

    let head_ref = terminal
        .grid_ref(viewport_point(head))
        .map_err(|error| SessionError::new("selection_grid_ref", error))?;

    let selection = match mode {
        SelectionMode::Character => {
            let anchor_ref = match anchor {
                SelectionAnchor::Cell(position) => terminal
                    .grid_ref(viewport_point(position))
                    .map_err(|error| SessionError::new("selection_grid_ref", error))?,
                SelectionAnchor::Tracked(tracked) => {
                    let pinned = tracked
                        .snapshot(terminal)
                        .map_err(|error| SessionError::new("selection_anchor", error))?;
                    let Some(anchor_ref) = pinned else {
                        terminal
                            .set_selection(None)
                            .map_err(|error| SessionError::new("clear_selection", error))?;
                        return Ok(false);
                    };
                    anchor_ref
                }
            };
            Some(Selection::new(anchor_ref, head_ref, rectangle))
        }
        SelectionMode::Word => terminal
            .select_word(SelectWordOptions::new(head_ref))
            .map_err(|error| SessionError::new("select_word", error))?,
        SelectionMode::Line => terminal
            .select_line(SelectLineOptions::new(head_ref))
            .map_err(|error| SessionError::new("select_line", error))?,
    };

    terminal
        .set_selection(selection.as_ref())
        .map_err(|error| SessionError::new("set_selection", error))?;
    Ok(true)
}
```

3c. `crates/sprite-term/src/worker/mod.rs`:

Imports (lines 34-37) become:
```rust
use crate::input::mouse::{
    SelectionAnchor, WheelDestination, apply_selection, encode_mouse, encode_wheel,
    selection_text, track_selection_anchor, wheel_destination,
};
```
`Owned` (lines 87-94) becomes:
```rust
// Fields drop in declaration order, including on early return or unwind.
// Projection scratch, both encoders and the selection anchor must be released
// before terminal state.
struct Owned {
    projector: Projector<'static>,
    encoder: key::Encoder<'static>,
    mouse_encoder: libghostty_vt::mouse::Encoder<'static>,
    /// The content the current selection gesture's press landed on, until the
    /// next gesture or `ClearSelection`.
    selection_anchor: Option<libghostty_vt::screen::TrackedGridRef>,
    terminal: Terminal<'static, 'static>,
}
```
In `Session::apply`'s destructuring, the `Owned { … }` pattern becomes:
```rust
                Owned {
                    projector,
                    encoder,
                    mouse_encoder,
                    selection_anchor,
                    terminal,
                },
```
Replace the `TerminalCommand::Select { … }` and `TerminalCommand::ClearSelection` arms (master lines 405-433) with:
```rust
                TerminalCommand::BeginSelection { anchor } => {
                    // A new gesture: what was selected goes, and the press is
                    // pinned to the content under it before later output can
                    // move that content out from under the pointer.
                    *selection_anchor = None;
                    *has_selection = false;
                    if let Err(error) = terminal
                        .set_selection(None)
                        .map_err(|error| SessionError::new("clear_selection", error))
                    {
                        emit(events, TerminalEvent::Error(error))?;
                    }
                    match track_selection_anchor(terminal, anchor) {
                        Ok(tracked) => *selection_anchor = Some(tracked),
                        // Reported; the gesture then extends from the cells its
                        // `Select`s name, as a selection without a press does.
                        Err(error) => emit(events, TerminalEvent::Error(error))?,
                    }
                    pending.mutated();
                }
                TerminalCommand::Select {
                    anchor,
                    head,
                    mode,
                    rectangle,
                } => {
                    let anchor = match selection_anchor.as_ref() {
                        Some(tracked) => SelectionAnchor::Tracked(tracked),
                        None => SelectionAnchor::Cell(anchor),
                    };
                    match apply_selection(terminal, anchor, head, mode, rectangle) {
                        // `false`: the anchored content was evicted, and the
                        // selection was cleared rather than moved.
                        Ok(installed) => {
                            *has_selection = installed;
                            pending.mutated();
                        }
                        // A selection that cannot be resolved is reported, but
                        // it does not end the session: the next gesture may
                        // well land somewhere valid.
                        Err(error) => {
                            emit(events, TerminalEvent::Error(error))?;
                        }
                    }
                }
                TerminalCommand::ClearSelection => {
                    *selection_anchor = None;
                    *has_selection = false;
                    if let Err(error) = terminal
                        .set_selection(None)
                        .map_err(|error| SessionError::new("clear_selection", error))
                    {
                        emit(events, TerminalEvent::Error(error))?;
                    }
                    pending.mutated();
                }
```

3d. `crates/sprite-term/src/worker/start.rs:182-187`:
```rust
    let owned = Owned {
        projector,
        encoder,
        mouse_encoder,
        selection_anchor: None,
        terminal,
    };
```

3e. **Not now — this is Cycle B's Step 3**, written here beside the command it uses. `crates/sprite-app/src/terminal_view/render.rs`, replace lines 491-502:
```rust
                        // The press drops whatever was selected and remembers
                        // where a drag would start from. It selects nothing
                        // itself — see `Drag::moved`.
                        view.drag = Some(Drag {
                            anchor: cell,
                            moved: false,
                        });
                        view.send(TerminalCommand::ClearSelection);
```
with
```rust
                        // The press drops whatever was selected and pins where
                        // a drag would start from to the content under it, so
                        // output that scrolls before the drag cannot move the
                        // start. It selects nothing itself — see `Drag::moved`.
                        view.drag = Some(Drag {
                            anchor: cell,
                            moved: false,
                        });
                        view.send(TerminalCommand::BeginSelection { anchor: cell });
```

- [ ] **Step 4: Run tests, confirm pass** — `TERM=dumb cargo test -p sprite-term --locked --offline --test selection` and `TERM=dumb cargo test -p sprite-term --locked --offline --lib -- input:: worker::`.

- [ ] **Step 5: Commit** (without render.rs — 3e belongs to Cycle B)
```bash
git add crates/sprite-term/src/command.rs crates/sprite-term/src/input/mouse.rs \
  crates/sprite-term/src/worker/mod.rs crates/sprite-term/src/worker/start.rs \
  crates/sprite-term/tests/selection.rs
git commit -m "fix(term): pin a selection gesture's anchor to its content

BeginSelection tracks the pressed cell with a libghostty TrackedGridRef,
so later Selects extend from that content as output scrolls; a pin lost
to scrollback pruning clears the selection instead of re-anchoring.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

#### Cycle B — the application marks the press

- [ ] **Step 1: Write the failing test** — append to `crates/sprite-app/src/terminal_view/tests.rs` (after `pointer_selection_keeps_shift_override_and_click_drag_semantics`, which this copies for setup and cleanup):
```rust
/// The centre of a viewport cell, in window coordinates.
fn cell_center(
    view: &gpui::Entity<TerminalView>,
    cx: &mut gpui::VisualTestContext,
    row: usize,
    column: usize,
) -> gpui::Point<gpui::Pixels> {
    view.read_with(cx, |view, _| {
        let origin = view.content_origin.unwrap_or(view.origin);
        gpui::point(
            origin.x + view.metrics.width() * (column as f32 + 0.5),
            origin.y + view.metrics.height() * (row as f32 + 0.5),
        )
    })
}

#[gpui::test]
fn a_drag_extends_from_the_pressed_content_after_output_scrolls(cx: &mut gpui::TestAppContext) {
    use gpui::{Modifiers, MouseButton};
    let settings = crate::config::Settings::default();
    cx.set_global(crate::config::ActiveSettings(settings.clone()));
    let (sender, _exits) = async_channel::unbounded();
    let (view, cx) = cx.add_window_view(|window, cx| TerminalView::new(
        Some(vec!["/bin/sh".into(), "-c".into(), "stty -echo; seq 1 300; printf 'ANCHOR-TEXT\\nREADY'; IFS= read -r go; printf '\\nafter-1\\nafter-2\\nafter-3'; sleep 30".into()]),
        settings, Vec::new(), None,
        PaneExit { sender, identity: (crate::tabs::TabId(1), crate::pane_tree::PaneId(1)) }, window, cx,
    ));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let row_of = |bundle: &SnapshotBundle, text: &str| {
            bundle.pane.rows.iter().position(|row| row.text.trim_end() == text)
        };
        let before = wait_for_bundle(&view, cx, |bundle| {
            bundle.pane.rows.iter().any(|row| row.text.trim_end() == "READY")
        });
        let pressed_row = row_of(&before, "ANCHOR-TEXT").expect("the anchor line is on screen");
        cx.update(|window, cx| {
            window.activate_window();
            window.refresh();
            window.draw(cx).clear();
        });
        let press = cell_center(&view, cx, pressed_row, 0);
        cx.simulate_mouse_down(press, MouseButton::Left, Modifiers::default());
        view.update(cx, |view, _| {
            view.send(TerminalCommand::Input(b"go\n".to_vec()))
        });
        let after = wait_for_bundle(&view, cx, |bundle| {
            bundle.pane.rows.iter().any(|row| row.text.trim_end() == "after-3")
        });
        let moved_row = row_of(&after, "ANCHOR-TEXT").expect("the anchor line is still on screen");
        assert!(moved_row < pressed_row, "output scrolled the anchor line up");
        let release = cell_center(&view, cx, moved_row, 10);
        cx.simulate_mouse_move(release, Some(MouseButton::Left), Modifiers::default());
        cx.simulate_mouse_up(release, MouseButton::Left, Modifiers::default());
        let bundle = wait_for_bundle(&view, cx, |bundle| {
            bundle
                .render
                .rows
                .iter()
                .any(|row| row.cells.iter().any(|cell| cell.selected))
        });
        let selected_rows: Vec<usize> = bundle
            .render
            .rows
            .iter()
            .enumerate()
            .filter(|(_, row)| row.cells.iter().any(|cell| cell.selected))
            .map(|(index, _)| index)
            .collect();
        assert_eq!(selected_rows, vec![moved_row], "only the pressed line is selected");
        let selected: String = bundle.render.rows[moved_row]
            .cells
            .iter()
            .filter(|cell| cell.selected)
            .map(|cell| cell.text.as_str())
            .collect();
        assert_eq!(selected, "ANCHOR-TEXT");
    }));
    if let Some(cleanup) = view.update(cx, |view, _| view.begin_shutdown()) {
        cleanup.wait().unwrap();
    }
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}
```

- [ ] **Step 2: Run it and confirm it fails** — with render.rs still sending `ClearSelection`: `TERM=dumb cargo test -p sprite-app --locked --offline terminal_view::tests::a_drag_extends_from_the_pressed_content_after_output_scrolls -- --exact`. Expected: `only the pressed line is selected` fails, `left: [moved_row, …, moved_row + 3]` vs `right: [moved_row]`.

- [ ] **Step 3: Implement** — apply 3e above to `crates/sprite-app/src/terminal_view/render.rs:491-502`.

- [ ] **Step 4: Run tests, confirm pass** — `TERM=dumb cargo test -p sprite-app --locked --offline --lib -- terminal_view::tests::a_drag_extends terminal_view::tests::pointer_selection terminal_view::tests::pointer_reports`.

- [ ] **Step 5: Commit**
```bash
git add crates/sprite-app/src/terminal_view/render.rs crates/sprite-app/src/terminal_view/tests.rs
git commit -m "fix(app): mark the press that starts a selection gesture

The left press now sends BeginSelection with the pressed cell, so the
worker pins the anchor before output can scroll it away.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 12: Session lifecycle fixes — paste backlog, released master, honest escalation (BCA-22, BCA-24, BCA-27)

Three cycles, committed separately.

**Files:**
- Modify: `crates/sprite-term/src/config.rs:16-19` (`max_clipboard_bytes` becomes `const fn`)
- Modify: `crates/sprite-term/src/pty_unix.rs:54-58` (backlog bound), `:116-121` (doc), `:523-528` (`GroupSignal` derives)
- Modify: `crates/sprite-term/src/foreground.rs:15-16,65-129` (`Mutex<Option<Attached>>`, `detach`), tests `:140-179`
- Modify: `crates/sprite-term/src/worker/mod.rs:135-144` (`Runtime.foreground`), `:186-196` (`run`)
- Modify: `crates/sprite-term/src/worker/closing.rs:20-192` (detach + close order; `Escalation`; `finish_shutdown`)
- Modify: `crates/sprite-term/src/pty_unix/processes.rs:77-95` (`signal` → `bool`, `signal_members`), `:165-216` (Darwin `read_process` via `sandwiched_record`)
- Test: `crates/sprite-term/tests/paste.rs`, `crates/sprite-term/tests/support/mod.rs` (new `try_next_error`), `crates/sprite-term/tests/lifecycle.rs`, inline `mod tests` in `foreground.rs`, `processes.rs`, new inline `mod tests` in `worker/closing.rs`

**Interfaces:**
- Consumes: Task 9's `worker::coalescing_tests::Fixture` passes a `ForegroundWatch` into `run` (unchanged signature).
- Produces: `pub(crate) const fn max_clipboard_bytes() -> usize`; `pub(crate) fn ForegroundWatch::detach(&self)`; `Runtime.foreground: Arc<ForegroundWatch>`; `pub(crate) fn SessionProcesses::signal(&mut self, &GroupSignal) -> bool`; `#[derive(Clone, Copy, Debug, PartialEq, Eq)] GroupSignal`; `EventPump::try_next_error(&self) -> Option<SessionError>` in tests/support.

#### Cycle A — the input backlog holds a whole bracketed paste (BCA-22)

- [ ] **Step 1: Write the failing test** — in `crates/sprite-term/tests/support/mod.rs`, inside the second `impl EventPump` block (after `try_next_clipboard`, line 180) add:
```rust
    /// Looks for an error event within a short window, ignoring everything
    /// else. A healthy session is silent, so this cannot block indefinitely;
    /// the window exists for the same reason as `try_next_clipboard`'s.
    pub fn try_next_error(&self) -> Option<SessionError> {
        let deadline = Instant::now() + Duration::from_millis(750);
        while Instant::now() < deadline {
            match self.receiver.recv_timeout(Duration::from_millis(50)) {
                Ok(Ok(TerminalEvent::Error(error))) => return Some(error),
                Ok(_) => {}
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return None,
            }
        }
        None
    }
```
Append to `crates/sprite-term/tests/paste.rs`:
```rust
/// A bracketed paste of the largest text a paste may carry is queued whole.
///
/// The input backlog must leave room for the brackets around the largest
/// admitted paste; otherwise that paste is refused with a claim that the
/// program stopped reading, when it never had the chance.
#[test]
fn a_bracketed_paste_of_the_largest_allowed_text_is_accepted() {
    let sprite_term::Spawned {
        mut session,
        events,
        snapshots,
    } = session("stty -echo; printf '\\033[?2004h'; printf 'READY\\n'; sleep 30");
    let events = EventPump::new(events);
    let snapshots = SnapshotPump::new(snapshots);
    events.expect_ready();
    snapshots.wait_for("ready", |b| pane_text(b).contains("READY"));

    // The clipboard bound, which `Paste` admission itself enforces.
    let largest = "x".repeat(1024 * 1024);
    session
        .send(TerminalCommand::Paste(largest))
        .expect("a paste at the clipboard bound is admitted");

    let refused = events.try_next_error();
    assert!(refused.is_none(), "the input backlog refused it: {refused:?}");
}
```

- [ ] **Step 2: Run it and confirm it fails** — `TERM=dumb cargo test -p sprite-term --locked --offline --test paste a_bracketed_paste_of_the_largest_allowed_text_is_accepted -- --exact`. Expected: `the input backlog refused it: Some(SessionError { … "the program is not reading its input: 0 bytes are already waiting for it, so 1048588 more were not delivered" })`.

- [ ] **Step 3: Implement**
  - `crates/sprite-term/src/config.rs:16-19`:
```rust
/// The OSC 52 size bound, in decoded bytes, and the most text one paste may carry.
pub(crate) const fn max_clipboard_bytes() -> usize {
    MAX_CLIPBOARD_BYTES
}
```
  - `crates/sprite-term/src/pty_unix.rs:54-58`, replace:
```rust
/// Input waiting for the PTY to have room, beyond which more is refused and
/// reported rather than held. Sixty-four maximal pastes: a program this far
/// behind has stopped reading, and hoarding more for it would only hide that
/// from the person typing.
const INPUT_BACKLOG_BYTES: usize = 1024 * 1024;
```
with
```rust
/// What bracketed paste adds around a paste: `ESC [ 200 ~` before it and
/// `ESC [ 201 ~` after it.
const BRACKETED_PASTE_OVERHEAD: usize = 12;

/// Input waiting for the PTY to have room, beyond which more is refused and
/// reported rather than held.
///
/// Exactly one largest admitted paste with its brackets: anything smaller and
/// a paste the session already accepted would be refused as if the program had
/// stopped reading. A program further behind than that has stopped reading,
/// and hoarding more for it would only hide that from the person typing.
const INPUT_BACKLOG_BYTES: usize = crate::max_clipboard_bytes() + BRACKETED_PASTE_OVERHEAD;
```
  - `pty_unix.rs:116-120` doc on `InputQueue::write`:
```rust
    /// Queues `bytes` for the PTY, after everything queued before them.
    ///
    /// Never blocks. Refused, with the reason, once the backlog would exceed
    /// `INPUT_BACKLOG_BYTES`, and once the pump has stopped, when there is
    /// nothing left to write to.
```

- [ ] **Step 4: Run tests, confirm pass** — `TERM=dumb cargo test -p sprite-term --locked --offline --test paste --test input_backpressure --test event_backpressure` and `TERM=dumb cargo test -p sprite-term --locked --offline --lib -- pty_unix:: config::`.

- [ ] **Step 5: Commit**
```bash
git add crates/sprite-term/src/config.rs crates/sprite-term/src/pty_unix.rs \
  crates/sprite-term/tests/paste.rs crates/sprite-term/tests/support/mod.rs
git commit -m "fix(term): size the input backlog for a bracketed maximal paste

The 1 MiB backlog equalled the 1 MiB paste limit, so a bracketed paste
near the limit was always refused. Derive the bound from the clipboard
limit plus the bracket overhead.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

#### Cycle B — an ended session releases its duplicate PTY master (BCA-24)

- [ ] **Step 1: Write the failing test**

Unit test, append to `mod tests` in `crates/sprite-term/src/foreground.rs`:
```rust
    #[test]
    fn detaching_closes_the_private_duplicate() {
        use std::io::Read;
        use std::os::fd::AsRawFd;
        use std::os::unix::net::UnixStream;

        let (master, mut peer) = UnixStream::pair().expect("socket pair");
        peer.set_read_timeout(Some(std::time::Duration::from_secs(2)))
            .expect("read deadline");
        let watch = ForegroundWatch::default();
        watch.attach(master.as_raw_fd(), Some(1));
        // The worker's own descriptor closes first, as it does at session end.
        drop(master);
        watch.detach();

        let mut byte = [0_u8; 1];
        let read = peer
            .read(&mut byte)
            .expect("the peer sees the close rather than waiting on an open duplicate");
        assert_eq!(read, 0);
        assert_eq!(watch.state(), ForegroundState::Unknown);
    }
```
Integration test, append to `crates/sprite-term/tests/lifecycle.rs`:
```rust
/// Once a session has ended on its own, nothing Sprite holds keeps the PTY
/// open: a descendant that ignored the hangup and still holds the terminal
/// sees it close, even while the application keeps the ended session.
#[test]
fn an_ended_session_releases_the_terminal_to_a_lingering_descendant() {
    let unique = format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
    );
    let pid_file = std::env::temp_dir().join(format!("sprite-hangup-{unique}.pid"));
    let hung_up = std::env::temp_dir().join(format!("sprite-hangup-{unique}.done"));
    struct Descendant(std::path::PathBuf, std::path::PathBuf);
    impl Drop for Descendant {
        fn drop(&mut self) {
            if let Ok(pid) = std::fs::read_to_string(&self.0) {
                let _ = std::process::Command::new("kill")
                    .args(["-KILL", pid.trim()])
                    .status();
            }
            let _ = std::fs::remove_file(&self.0);
            let _ = std::fs::remove_file(&self.1);
        }
    }
    let _descendant = Descendant(pid_file.clone(), hung_up.clone());
    // The descendant ignores SIGHUP and blocks reading the terminal; a read
    // only returns once no master is left open, and then it says so in a file.
    let script = r#"import os, signal, sys
r, w = os.pipe()
pid = os.fork()
if pid == 0:
    os.close(r)
    signal.signal(signal.SIGHUP, signal.SIG_IGN)
    os.write(w, b'ready')
    os.close(w)
    try:
        while os.read(0, 1):
            pass
    except OSError:
        pass
    with open(sys.argv[2], 'w') as f:
        f.write('hung up')
    os._exit(0)
os.close(w)
os.read(r, 5)
with open(sys.argv[1], 'w') as f:
    f.write(str(pid))
os._exit(0)
"#;
    let sprite_term::Spawned {
        session: _session,
        events,
        snapshots: _snapshots,
    } = TerminalSession::spawn(SessionConfig::command(
        "python3",
        vec![
            "-c".into(),
            script.into(),
            pid_file.into_os_string(),
            hung_up.clone().into_os_string(),
        ],
    ))
    .expect("spawn parent and lingering descendant");
    let events = EventPump::new(events);
    events.expect_ready();
    match events.next() {
        TerminalEvent::Exited(exit) => assert!(!exit.requested),
        other => panic!("expected natural Exited, got {other:?}"),
    }
    // `_session` is still alive here, as the application keeps an ended pane.
    let deadline = std::time::Instant::now() + support::WATCHDOG;
    while !hung_up.exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "the descendant still holds an open terminal after the session ended"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}
```

- [ ] **Step 2: Run it and confirm it fails** —
  - `TERM=dumb cargo test -p sprite-term --locked --offline foreground::tests` → compile error `no method named `detach` found for struct `ForegroundWatch``.
  - On Linux: `TERM=dumb cargo test -p sprite-term --locked --offline --test lifecycle an_ended_session_releases_the_terminal_to_a_lingering_descendant -- --exact` → panics `the descendant still holds an open terminal after the session ended` (about 2 s natural drain + 5 s). On macOS this test passes even before the fix (see Drafter notes); the unit test is the cross-platform witness.

- [ ] **Step 3: Implement**

3a. `crates/sprite-term/src/foreground.rs`: line 16 `use std::sync::OnceLock;` → `use std::sync::{Mutex, MutexGuard, PoisonError};`. Replace lines 65-129 (`ForegroundWatch`, its `impl`, and its `Debug` impl) with:
```rust
/// A handle on the "what is running" question for one pane.
///
/// Shared between the session and its worker: the worker attaches the PTY once
/// it exists, the application asks whenever it needs to know, and the worker
/// detaches it when the session ends.
#[derive(Default)]
pub struct ForegroundWatch {
    /// `None` until the worker attaches, and again once the session has ended.
    attached: Mutex<Option<Attached>>,
}

impl ForegroundWatch {
    /// Called once by the worker, as soon as the PTY and child exist.
    ///
    /// A session whose shell has no process group is left unattached, and
    /// answers `Unknown` forever — which is honest: without the group there is
    /// nothing to compare against.
    pub(crate) fn attach(&self, master_fd: RawFd, shell_group: Option<i32>) {
        let Some(shell_group) = shell_group else {
            return;
        };
        let Some(master) = pty_unix::duplicate(master_fd) else {
            return;
        };
        let mut attached = self.lock();
        if attached.is_none() {
            *attached = Some(Attached {
                master,
                shell_group,
            });
        }
    }

    /// Closes the private duplicate once the session has ended.
    ///
    /// Held any longer, it keeps the PTY master open after the worker has
    /// closed its own, and a descendant still holding the terminal is never
    /// told it hung up. Afterwards the pane answers `Unknown`, which is all an
    /// ended session can honestly say.
    pub(crate) fn detach(&self) {
        let detached = self.lock().take();
        drop(detached);
    }

    fn lock(&self) -> MutexGuard<'_, Option<Attached>> {
        self.attached.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Asks the kernel what is in the foreground of this pane, right now.
    pub fn state(&self) -> ForegroundState {
        let attached = self.lock();
        let Some(attached) = attached.as_ref() else {
            return ForegroundState::Unknown;
        };
        let Some(group) = pty_unix::foreground_group(&attached.master) else {
            return ForegroundState::Unknown;
        };
        if group == attached.shell_group {
            return ForegroundState::Idle;
        }
        // A group the terminal still names but nothing is left in belongs to a
        // program that has already finished; there is nothing to lose by
        // closing.
        if !pty_unix::group_is_alive(group) {
            return ForegroundState::Idle;
        }
        ForegroundState::Busy(executable_name(group))
    }

    /// Returns the process group when `pid` owns this pane's foreground.
    pub fn owner_group(&self, pid: u32) -> Option<i32> {
        let attached = self.lock();
        let attached = attached.as_ref()?;
        let foreground = pty_unix::foreground_group(&attached.master)?;
        let candidate = pty_unix::process_group_of(pid)?;
        (foreground == candidate).then_some(candidate)
    }
}

impl std::fmt::Debug for ForegroundWatch {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ForegroundWatch")
            .field("attached", &self.lock().is_some())
            .finish()
    }
}
```

3b. `crates/sprite-term/src/worker/mod.rs`: add the field to `Runtime` (lines 135-144) after `shutdown`:
```rust
    shutdown: Arc<AtomicBool>,
    /// Detached when the session closes, so its duplicate of the master
    /// closes with the worker's own.
    foreground: Arc<crate::ForegroundWatch>,
```
and in `run` (lines 186-196):
```rust
    foreground.attach(started.master_fd, started.process_group);
    let mut runtime = Runtime {
        started,
        pump: None,
        inbox,
        events,
        shutdown,
        foreground,
        exit_status: None,
        pump_stopped: true,
        fatal: None,
    };
```

3c. `crates/sprite-term/src/worker/closing.rs`: in `close`'s destructuring (lines 21-37) add `foreground,` after `shutdown,`. Replace lines 165-167:
```rust
    events.seal(outcomes);

    drop(master);
```
with
```rust
    // Every descriptor this session holds on the PTY master closes before the
    // outcome is published: the worker's own and the duplicate kept for
    // foreground questions. A descendant still holding the terminal then sees
    // it hang up, and whoever is told the session ended can rely on that.
    drop(master);
    foreground.detach();
    events.seal(outcomes);
```

- [ ] **Step 4: Run tests, confirm pass** — `TERM=dumb cargo test -p sprite-term --locked --offline --lib -- foreground worker::` and `TERM=dumb cargo test -p sprite-term --locked --offline --test lifecycle --test foreground --test session_jobs`; then `TERM=dumb cargo test -p sprite-app --locked --offline terminal_view::tests` (the app reads `foreground()` only while Running).

- [ ] **Step 5: Commit**
```bash
git add crates/sprite-term/src/foreground.rs crates/sprite-term/src/worker/mod.rs \
  crates/sprite-term/src/worker/closing.rs crates/sprite-term/tests/lifecycle.rs
git commit -m "fix(term): close the foreground watch's PTY duplicate at session end

ForegroundWatch kept a duplicate master open for as long as the ended
session was held, so a descendant ignoring SIGHUP never saw the hangup.
Detach it, with the worker's own master, before the outcome is sealed.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

#### Cycle C — a vanished member is skipped and only an attempted signal advances escalation (BCA-27)

The Darwin adapter cannot run on Linux CI, so the decisions move into platform-independent functions with unit tests, and the `cfg(target_os = "macos")` code only gathers readings.

- [ ] **Step 1: Write the failing test**

Append to `mod tests` in `crates/sprite-term/src/pty_unix/processes.rs`:
```rust
    #[test]
    fn a_process_that_vanishes_mid_read_is_gone_not_an_incomplete_scan() {
        let reading = Reading {
            pid: 43,
            group: 43,
            birth: 2,
            live: true,
        };
        let complete = Process {
            pid: 43,
            group: 43,
            session: 42,
            birth: 2,
            live: true,
        };
        assert_eq!(
            sandwiched_record(43, Ok(None), || panic!("not asked"), || panic!("not read")),
            Ok(None)
        );
        assert_eq!(
            sandwiched_record(43, Ok(Some(reading)), || Ok(None), || panic!("not read")),
            Ok(None),
            "gone before its session was read"
        );
        assert_eq!(
            sandwiched_record(43, Ok(Some(reading)), || Ok(Some(42)), || Ok(None)),
            Ok(None),
            "gone before the second reading"
        );
        assert_eq!(
            sandwiched_record(43, Ok(Some(reading)), || Ok(Some(42)), || Ok(Some(reading))),
            Ok(Some(complete))
        );
        // Only disappearance is proof; anything else leaves the scan incomplete.
        assert_eq!(
            sandwiched_record(43, Ok(Some(reading)), || Ok(Some(42)), || {
                Ok(Some(Reading { birth: 3, ..reading }))
            }),
            Err(())
        );
        assert_eq!(
            sandwiched_record(43, Ok(Some(reading)), || Err(()), || Ok(Some(reading))),
            Err(())
        );
        assert_eq!(
            sandwiched_record(43, Err(()), || Ok(Some(42)), || Ok(Some(reading))),
            Err(())
        );
    }

    #[test]
    fn a_signal_attempt_skips_vanished_members_and_reports_unreached_ones() {
        let scope = SessionProcesses {
            session: 42,
            leader_birth: Some(1),
            own_group: 7,
            retired: false,
        };
        let gone = Process {
            pid: 43,
            group: 43,
            session: 42,
            birth: 2,
            live: true,
        };
        let present = Process {
            pid: 44,
            group: 44,
            session: 42,
            birth: 3,
            live: true,
        };
        let sibling = Process {
            pid: 45,
            group: 44,
            session: 42,
            birth: 4,
            live: true,
        };

        let mut sent = Vec::new();
        assert!(
            scope.signal_members(
                Ok(vec![gone, present]),
                |pid| Ok((pid == present.pid).then_some(present)),
                |group| sent.push(group),
            ),
            "a member that vanished has nothing left to signal"
        );
        assert_eq!(sent, vec![44]);

        let mut sent = Vec::new();
        assert!(!scope.signal_members(Err(()), |_| unreachable!(), |group| sent.push(group)));
        assert!(sent.is_empty(), "an incomplete scan signals nobody");

        let mut sent = Vec::new();
        assert!(
            !scope.signal_members(
                Ok(vec![gone, present]),
                |pid| if pid == gone.pid { Err(()) } else { Ok(Some(present)) },
                |group| sent.push(group),
            ),
            "a member whose identity could not be read was not reached"
        );
        assert_eq!(sent, vec![44], "the members that could be reached still are");

        let mut sent = Vec::new();
        assert!(
            scope.signal_members(
                Ok(vec![sibling, present]),
                |pid| if pid == sibling.pid { Err(()) } else { Ok(Some(present)) },
                |group| sent.push(group),
            ),
            "an unreadable member whose group was signalled through another was reached"
        );
        assert_eq!(sent, vec![44]);
    }
```
Add a test module at the end of `crates/sprite-term/src/worker/closing.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escalation_advances_only_past_a_signal_that_was_attempted() {
        let mut escalation = Escalation::Hangup;
        assert_eq!(escalation.due(None), Some(GroupSignal::Hangup));
        escalation = escalation.after(GroupSignal::Hangup, false);
        assert_eq!(
            escalation.due(None),
            Some(GroupSignal::Hangup),
            "a hangup no scan delivered is still owed"
        );
        escalation = escalation.after(GroupSignal::Hangup, true);
        assert_eq!(escalation.due(None), None, "a natural close owes nothing more");
        assert_eq!(escalation.due(Some(TERM_AFTER - Duration::from_millis(1))), None);
        assert_eq!(escalation.due(Some(TERM_AFTER)), Some(GroupSignal::Terminate));
        escalation = escalation.after(GroupSignal::Terminate, false);
        assert_eq!(
            escalation.due(Some(TERM_AFTER + CLOSING_SLICE)),
            Some(GroupSignal::Terminate),
            "a TERM nobody received is tried again"
        );
        escalation = escalation.after(GroupSignal::Terminate, true);
        assert_eq!(escalation.due(Some(TERM_AFTER + CLOSING_SLICE)), None);
        assert_eq!(escalation.due(Some(KILL_AFTER)), Some(GroupSignal::Kill));
        assert_eq!(
            escalation.after(GroupSignal::Kill, true).due(Some(KILL_AFTER)),
            Some(GroupSignal::Kill),
            "KILL repeats until the scope is empty"
        );
    }

    #[test]
    fn kill_is_due_at_its_deadline_even_when_earlier_steps_never_landed() {
        assert_eq!(Escalation::Hangup.due(Some(KILL_AFTER)), Some(GroupSignal::Kill));
    }
}
```

- [ ] **Step 2: Run it and confirm it fails** — `TERM=dumb cargo test -p sprite-term --locked --offline --lib -- pty_unix::processes::tests worker::closing::tests`. Expected: compile errors `cannot find struct `Reading``, `cannot find function `sandwiched_record``, `no method named `signal_members``, `cannot find type `Escalation``.

- [ ] **Step 3: Implement**

3a. `crates/sprite-term/src/pty_unix.rs:523-528`: add derives to `GroupSignal`:
```rust
/// The bounded shutdown policy's escalation steps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GroupSignal {
```

3b. `crates/sprite-term/src/pty_unix/processes.rs`, replace `signal` (lines 77-95) with:
```rust
    /// Signals every owned group a fresh scan finds, after rereading each
    /// member's identity.
    ///
    /// Returns whether the attempt was complete. `false` means the scan, or a
    /// member's reread, could not finish, so someone who should have received
    /// this signal may not have; escalation must not count it as delivered.
    pub(crate) fn signal(&mut self, signal: &GroupSignal) -> bool {
        let members = self.members();
        self.signal_members(members, read_process, |group| signal_group(group, signal))
    }

    /// The decision half of `signal`, free of the platform's process table.
    fn signal_members(
        &self,
        members: Result<Vec<Process>, ()>,
        mut reread: impl FnMut(i32) -> Result<Option<Process>, ()>,
        mut send: impl FnMut(i32),
    ) -> bool {
        let Ok(members) = members else {
            return false;
        };
        let mut signalled = HashSet::new();
        let mut unreadable = HashSet::new();
        for member in members {
            if signalled.contains(&member.group) {
                continue;
            }
            match reread(member.pid) {
                Ok(Some(now)) if same_member(member, now) && self.owned(&now) => {
                    send(member.group);
                    signalled.insert(member.group);
                }
                // Gone, or a different process now: nothing of this session is
                // left there to signal.
                Ok(_) => {}
                Err(()) => {
                    unreadable.insert(member.group);
                }
            }
        }
        unreadable.iter().all(|group| signalled.contains(group))
    }
```
Replace the Darwin `read_process` (lines 165-216) with the platform-independent decision and a thin adapter:
```rust
/// The fields of one BSD-info reading that identify a process.
#[cfg(any(target_os = "macos", test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Reading {
    pid: i32,
    group: i32,
    birth: u64,
    live: bool,
}

/// One process's record from a reading, its session, and a second reading.
///
/// Each step answers `Ok(None)` once the kernel says the process no longer
/// exists, and that ends the record as "gone": a member that exits mid-scan
/// has nothing left to signal, and treating it as a failure would abort the
/// whole scan and signal nobody. Any other failure, or an identity that
/// changed between the readings, leaves the scan incomplete — only
/// disappearance is proof.
#[cfg(any(target_os = "macos", test))]
fn sandwiched_record(
    pid: i32,
    before: Result<Option<Reading>, ()>,
    session: impl FnOnce() -> Result<Option<i32>, ()>,
    after: impl FnOnce() -> Result<Option<Reading>, ()>,
) -> Result<Option<Process>, ()> {
    let Some(before) = before? else {
        return Ok(None);
    };
    let Some(session) = session()? else {
        return Ok(None);
    };
    let Some(after) = after()? else {
        return Ok(None);
    };
    if (before.pid, before.group, before.birth) != (after.pid, after.group, after.birth) {
        return Err(());
    }
    Ok(Some(Process {
        pid,
        group: after.group,
        session,
        birth: after.birth,
        live: after.live,
    }))
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
fn read_process(pid: i32) -> Result<Option<Process>, ()> {
    use nix::libc;
    fn bsd(pid: i32) -> Result<Option<Reading>, ()> {
        // SAFETY: BSD info is a plain C output record and its exact byte size is supplied.
        let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
        let size = std::mem::size_of_val(&info) as i32;
        // SAFETY: libproc writes only within this BSD info output buffer.
        let count = unsafe {
            libc::proc_pidinfo(
                pid,
                libc::PROC_PIDTBSDINFO,
                0,
                (&mut info as *mut libc::proc_bsdinfo).cast(),
                size,
            )
        };
        if count == size {
            return Ok(Some(Reading {
                pid: info.pbi_pid as i32,
                group: info.pbi_pgid as i32,
                birth: info.pbi_start_tvsec * 1_000_000 + info.pbi_start_tvusec,
                // Darwin sys/proc.h defines SZOMB as 5; libc exports no named binding.
                live: info.pbi_status != 5,
            }));
        }
        if nix::errno::Errno::last() == nix::errno::Errno::ESRCH {
            Ok(None)
        } else {
            Err(())
        }
    }
    sandwiched_record(pid, bsd(pid), || process_session(pid), || bsd(pid))
}
```
(`process_session` already maps `ESRCH` to `Ok(None)`; `scan` at line 251 then skips the vanished pid through `scoped_record` instead of aborting.)

3c. `crates/sprite-term/src/worker/closing.rs`. After `const CLOSING_SLICE …` (line 18) add:
```rust

/// The next signal escalation owes. It advances only once a signal was
/// actually attempted against every member a complete scan found: a scan that
/// could not finish signalled nobody, and counting it would spend the hangup
/// or TERM on no one and leave only KILL for a program that would have
/// honoured a politer request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Escalation {
    /// The single hangup has not reached the session yet.
    Hangup,
    /// The hangup was attempted; TERM is owed two seconds after a request.
    Terminate,
    /// TERM was attempted; only KILL remains, three seconds after a request.
    Kill,
}

impl Escalation {
    /// The signal due now, given how long ago shutdown was requested, if it was.
    ///
    /// KILL is due on every pass once its deadline passes, so a group created
    /// after the previous scan is still reached before the budget ends.
    fn due(self, requested: Option<Duration>) -> Option<GroupSignal> {
        match (self, requested) {
            (_, Some(waited)) if waited >= KILL_AFTER => Some(GroupSignal::Kill),
            (Self::Hangup, _) => Some(GroupSignal::Hangup),
            (Self::Terminate, Some(waited)) if waited >= TERM_AFTER => {
                Some(GroupSignal::Terminate)
            }
            _ => None,
        }
    }

    /// Where escalation stands after `signal`: past it only if it was attempted.
    fn after(self, signal: GroupSignal, attempted: bool) -> Self {
        match (attempted, signal) {
            (false, _) => self,
            (true, GroupSignal::Hangup) => Self::Terminate,
            (true, GroupSignal::Terminate | GroupSignal::Kill) => Self::Kill,
        }
    }
}

/// Sends whatever is due and reports where escalation then stands.
fn escalate(
    processes: &mut pty_unix::SessionProcesses,
    escalation: Escalation,
    requested: Option<Duration>,
) -> Escalation {
    let Some(signal) = escalation.due(requested) else {
        return escalation;
    };
    let attempted = processes.signal(&signal);
    escalation.after(signal, attempted)
}
```
In `close`, replace lines 46-50:
```rust
    // A hangup is the polite request every well-behaved program honours.
    if let Some(processes) = &mut processes {
        processes.signal(&GroupSignal::Hangup);
    }
    let mut escalation = 1_u8;
```
with
```rust
    // The first pass below sends the hangup every well-behaved program
    // honours; escalation moves past a step only once it reached the members.
    let mut escalation = Escalation::Hangup;
```
and replace lines 64-79:
```rust
        // Checked before every receive, so continuous output cannot postpone
        // escalation past its deadline.
        if let Some(since) = requested_at {
            let waited = since.elapsed();
            if waited >= KILL_AFTER {
                if let Some(processes) = &mut processes {
                    processes.signal(&GroupSignal::Kill);
                }
                escalation = 3;
            } else if waited >= TERM_AFTER && escalation < 2 {
                if let Some(processes) = &mut processes {
                    processes.signal(&GroupSignal::Terminate);
                }
                escalation = 2;
            }
        }
```
with
```rust
        // Checked before every receive, so continuous output cannot postpone
        // escalation past its deadline.
        if let Some(processes) = &mut processes {
            escalation = escalate(
                processes,
                escalation,
                requested_at.map(|since| since.elapsed()),
            );
        }
```
Replace `finish_shutdown` (lines 177-192) with:
```rust
pub(crate) fn finish_shutdown(mut processes: pty_unix::SessionProcesses, requested_at: Instant) {
    // The natural close already sent the session its hangup; what remains is
    // TERM and KILL, each counted only once a scan has reached the members.
    let mut escalation = Escalation::Terminate;
    while processes.is_alive() {
        let waited = requested_at.elapsed();
        escalation = escalate(&mut processes, escalation, Some(waited));
        if waited >= GIVE_UP_AFTER {
            break;
        }
        std::thread::sleep(CLOSING_SLICE.min(GIVE_UP_AFTER.saturating_sub(waited)));
    }
}
```
Line 2 `use crate::pty_unix::GroupSignal;` stays (used by `Escalation`).

- [ ] **Step 4: Run tests, confirm pass** — `TERM=dumb cargo test -p sprite-term --locked --offline --lib -- pty_unix:: worker::` then `TERM=dumb cargo test -p sprite-term --locked --offline --test lifecycle --test session_jobs`.

- [ ] **Step 5: Commit**
```bash
git add crates/sprite-term/src/pty_unix.rs crates/sprite-term/src/pty_unix/processes.rs \
  crates/sprite-term/src/worker/closing.rs
git commit -m "fix(term): skip vanished members and escalate only after an attempt

On Darwin a member exiting between its readings aborted the whole scan,
yet escalation still advanced and spent HUP and TERM on nobody. A vanished
member is now skipped, and escalation moves on only once a complete scan
signalled every member.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

---

### Task 13: Surface writer thread per connection (BCA-09)

PRD R-S1; ADR 0029 (amends ADR 0018's blocking write).

Today every `SurfaceConnection::send`/`send_batch`/`establish` does a blocking
`write_all` under a 2 s `SO_SNDTIMEO` on whichever thread calls it — for events,
the GPUI thread. A plugin that stops reading freezes rendering and input for the
whole Sprite Window for up to 2 s per send. After this task the connection owns
one writer thread and one byte-bounded queue; callers only enqueue.

**Files:**
- Modify: `crates/sprite-app/src/surface/channel.rs:20` (imports), `:205-390` (`EVENT_BUFFER_BYTES` through the end of `impl SurfaceConnection` — `Wire`, `SurfaceConnection`, `send`, `send_batch`, `establish`, `test_buffer_state`, `is_dead`, `abandon`), `:647-651` (comment in `serve_surface`)
- Modify: `crates/sprite-app/src/terminal_view/surfaces.rs:844-848` (`close_surface` doc comment)
- Test: `crates/sprite-app/src/surface/channel.rs` — existing `#[cfg(test)] mod tests` (line 754); changes to tests at `:842-1098`, `:1425-1456`, `:1472-1485`
- Test: `crates/sprite-app/src/terminal_view/surfaces.rs` — existing `#[cfg(test)] mod tests` (line 1483); helpers `open_request` `:1734-1767` and `events` `:1784-1793`, test `a_huge_finite_grid_wheel_stops_at_backpressure_with_bounded_storage` `:2002-2035` (assertions at `:2026-2031`)

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces (signatures unchanged, so no production caller changes):
  - `pub fn SurfaceConnection::send(&self, line: &str) -> bool` — now only queues; `false` = connection dead.
  - `pub fn SurfaceConnection::send_batch<'a>(&self, lines: impl IntoIterator<Item = &'a str>) -> bool`
  - `pub(crate) fn SurfaceConnection::establish(&self, line: &str) -> bool`
  - `pub(crate) fn SurfaceConnection::new(stream: &UnixStream) -> std::io::Result<Self>` — now also spawns the thread `sprite-surface-writer`.
  - `pub(crate) const MAX_PENDING_BYTES: usize = 4 * 1024 * 1024` (channel.rs). Admission rule: an event is queued whenever fewer than `MAX_PENDING_BYTES` are already pending (queued + being written); otherwise the connection dies. So the most ever pending is the bound plus one event, and a single event is never refused for its size alone (a large Surface paste behaves as today).
  - `pub(crate) const EVENT_BUFFER_BYTES: usize = 64 * 1024` (kept; now "largest buffer a writer keeps between writes")
  - Test-only (`#[cfg(test)]`): `pub(crate) fn is_dead(&self) -> bool`, `pub(crate) fn pending_bytes(&self) -> usize`, `pub(crate) fn settle(&self)`, `pub(crate) fn writer(&self) -> WriterProbe`, `pub(crate) struct WriterProbe` with `pub(crate) fn finished_within(&self, limit: Duration) -> bool`.
  - Test-only in `terminal_view/surfaces.rs` tests: `struct Peer { stream: UnixStream, connection: SurfaceConnection }`, `fn open_request(...) -> (Result<(), Refusal>, Peer)`, `fn events(peer: &mut Peer) -> Vec<serde_json::Value>` (Task 15 uses these).
  - Removed: `SurfaceConnection::test_buffer_state` (test-only; its two callers are rewritten below).

**Requirement → test map (R-S1):**

| R-S1 clause | Test |
|---|---|
| `send`/`send_batch`/`establish` only enqueue, never block | `sends_to_a_peer_that_never_reads_return_at_once_then_the_connection_closes` (new, fails at master) |
| 4 MiB pending bound (admit while < 4 MiB pending; one event over is fine) | `a_huge_batch_stops_consuming_when_the_peer_stops_reading`, `pre_open_overflow_discards_the_queue_and_cannot_establish_later`, `pre_open_gestures_stop_at_the_queue_limit_without_collecting_the_iterator` (adjusted), `a_single_event_larger_than_the_bound_is_delivered_whole` (new guard) |
| overflow → dead + socket shut down | new test above (peer reads to EOF), `pre_open_overflow_...` (peer reads to EOF, nothing escaped) |
| writer keeps 2 s `SO_SNDTIMEO`; timeout/failure → dead | `a_backpressured_batch_keeps_its_written_prefix_and_shuts_down_on_timeout`, `the_writer_stops_at_the_first_failed_write`, `a_closed_peer_stops_further_sends` (adjusted) |
| close/drop stops the thread (bounded by drain + `SO_SNDTIMEO`); never joined | `dropping_the_last_handle_never_waits_for_a_stalled_writer` (new guard) |
| graceful close sends what is queued, `closed` included | `a_graceful_close_sends_everything_queued_then_ends_the_stream` (new, fails at master) |
| ordering: `opened` first, contiguous batches/gestures | `batches_follow_opened_and_keep_concurrent_gestures_contiguous`, `chunked_batches_keep_the_gesture_lock_across_every_flush`, `an_event_sent_before_the_open_is_answered_follows_opened_on_the_wire` |

**Existing channel.rs tests — what happens to each:**

| Test | Verdict | Why |
|---|---|---|
| `a_huge_batch_stops_consuming_when_the_peer_stops_reading` | **change** | Bound moves from "≤ 4096 lines through a 64 KiB transport window" to "≤ `MAX_PENDING_BYTES/1024 + 1` lines" (lines are admitted while fewer than 4 MiB are pending, so the 4097th 1 KiB line finds the bound reached); `test_buffer_state` is gone → assert `pending_bytes() == 0`. |
| `a_failed_chunk_does_not_consume_the_remaining_iterator` | **replace** with `a_failed_write_leaves_later_batches_unconsumed` | A write failure is now discovered by the writer thread, after `send_batch` returned. The property kept: once the connection is dead, a batch consumes none of its iterator. |
| `pre_open_overflow_discards_the_queue_and_cannot_establish_later` | **change** (constants only) | Pre-open bound is now the one 4 MiB queue bound, not 64 KiB; `wire.queued` → `pending_bytes()`. Semantics (overflow kills, nothing escapes, `establish` fails) unchanged. |
| `pre_open_gestures_stop_at_the_queue_limit_without_collecting_the_iterator` | **change** | First half: constants only. Second half inverts: a single 4 MiB line on an empty queue is now admitted (an event is never refused for its own size); the *next* send finds the bound reached and kills the connection. |
| `chunked_batches_keep_the_gesture_lock_across_every_flush` | **change** (mechanics only) | Reached into `wire.stream` to half-close; now `drop(connection)` (graceful close) ends the peer's stream. Contiguity assertions unchanged. |
| `batches_follow_opened_and_keep_concurrent_gestures_contiguous` | **change** (mechanics only) | Same `wire.stream` reach-in → `drop(connection)`. Ordering assertions unchanged. |
| `batch_buffer_is_reused_and_closed_peers_stop_further_sends` | **replace** with `a_closed_peer_stops_further_sends` | Buffer-pointer reuse was an implementation detail of the synchronous writer. The closed-peer half is kept, waiting for the writer to discover the failure. |
| `a_large_batch_delivers_every_byte_while_the_peer_drains_in_small_chunks` | **change** (mechanics only) | The two 2 MiB lines still fit the admission rule (the second is admitted while about 2 MiB are pending). Only the `wire.stream` reach-in changes → `drop(connection)`. |
| `a_backpressured_batch_keeps_its_written_prefix_and_shuts_down_on_timeout` | **change** | `send_batch` now returns `true` (queued); the writer meets the stall, times out after `WRITE_TIMEOUT`, marks dead and shuts down. Prefix assertions unchanged. |
| `a_failed_write_marks_the_connection_dead_and_later_sends_return_at_once` | **keep body unchanged**; update its doc comment | Still passes: 64 KiB sends to a non-reading peer now overflow the 4 MiB queue (instead of timing out) at about the 64th send; later sends return at once. |
| `establish_stops_at_the_first_failed_write` | **replace** with `the_writer_stops_at_the_first_failed_write` | `establish` only queues now and returns `true`; the writer stops at the first failed write. |
| `an_event_sent_before_the_open_is_answered_follows_opened_on_the_wire`, `a_send_after_a_refused_open_reports_the_client_gone`, `an_open_reaches_the_window_and_its_answer_reaches_the_client`, `a_pipelined_open_update_and_close_survive_authentication_read_ahead`, `a_window_that_never_answers_still_hears_the_surface_closed`, `a_refused_open_and_an_unsupported_version_end_the_connection_with_their_reasons`, `a_grid_operation_reaches_the_window_as_one_request`, `a_focus_message_names_its_target`, all endpoint / handshake / socket-path tests | **keep unchanged** | They go through the real endpoint with blocking client reads, or depend only on `abandon` marking dead synchronously (still true). |

Outside channel.rs: `terminal_view/surfaces.rs` `a_huge_finite_grid_wheel_stops_at_backpressure_with_bounded_storage` **changes** (uses `test_buffer_state`), and its `events` helper **changes** (it read a non-blocking peer right after the GPUI side sent; with an async writer it must first wait for the writer — otherwise ~20 GPUI tests race). `surface/render.rs` tests, `workspace/surface_routing.rs` tests and `tests/client.rs` construct connections but never race a read against a send, so they stay unchanged.

- [ ] **Step 1: Write the failing tests**

Add both tests to `crates/sprite-app/src/surface/channel.rs`, `mod tests`, directly after `a_failed_write_marks_the_connection_dead_and_later_sends_return_at_once` (ends at line 1456). They use only items that exist at master (`SurfaceConnection::new`, `establish`, `send`, `send_batch`, the private test-only `is_dead`, `WRITE_TIMEOUT`, `event_opened`, `event_closed` via `pub use super::wire::*`), so they compile and fail on their assertions.

```rust
    /// The GPUI thread hands events to a connection and moves on: a program
    /// that has stopped reading costs it nothing, however many events pile
    /// up. Past the pending bound the connection dies and its socket is shut
    /// down, so the program sees the end of the stream.
    #[test]
    fn sends_to_a_peer_that_never_reads_return_at_once_then_the_connection_closes() {
        use std::io::Read;
        let (here, mut there) = UnixStream::pair().expect("pair");
        let connection = SurfaceConnection::new(&here).expect("connection");
        assert!(connection.establish(&event_opened(SurfaceId(1))));
        // 200 × 64 KiB is 12.5 MiB: several socket buffers, and three times
        // the pending bound.
        let line = "x".repeat(64 * 1024);
        let started = Instant::now();
        for _ in 0..200 {
            connection.send(&line);
        }
        let elapsed = started.elapsed();
        // Under one write timeout: a single blocking write that waits it out
        // fails this, however fast the rest are.
        assert!(
            elapsed < WRITE_TIMEOUT / 2,
            "200 sends to a peer that never reads took {elapsed:?} on the calling thread"
        );
        assert!(
            connection.is_dead(),
            "12.5 MiB of events cannot fit the pending bound"
        );
        there
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("timeout");
        let mut received = Vec::new();
        there
            .read_to_end(&mut received)
            .expect("the socket was shut down, so the peer reads to its end");
    }

    /// Dropping the last handle is a graceful close: everything already
    /// queued — the window's `closed` included — still reaches the program,
    /// and then its stream ends.
    #[test]
    fn a_graceful_close_sends_everything_queued_then_ends_the_stream() {
        use std::io::Read;
        let (here, mut there) = UnixStream::pair().expect("pair");
        let connection = SurfaceConnection::new(&here).expect("connection");
        let opened = event_opened(SurfaceId(1));
        assert!(connection.establish(&opened));
        let lines: Vec<String> = (0..100)
            .map(|i| json!({"type":"focus","i":i}).to_string())
            .collect();
        assert!(connection.send_batch(lines.iter().map(String::as_str)));
        assert!(connection.send(&event_closed()));
        drop(connection);
        there
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("timeout");
        let mut received = String::new();
        there
            .read_to_string(&mut received)
            .expect("the writer ends its side after the last line");
        let expected = std::iter::once(opened)
            .chain(lines)
            .chain([event_closed()])
            .map(|line| line + "\n")
            .collect::<String>();
        assert_eq!(received, expected);
        drop(here);
    }
```

- [ ] **Step 2: Run them and confirm they fail**

```
TERM=dumb cargo test -p sprite-app --locked --offline surface::channel::tests::sends_to_a_peer_that_never_reads_return_at_once_then_the_connection_closes -- --exact
TERM=dumb cargo test -p sprite-app --locked --offline surface::channel::tests::a_graceful_close_sends_everything_queued_then_ends_the_stream -- --exact
```

Expected at master:
- first: panics with `200 sends to a peer that never reads took 2xx.xxxms on the calling thread` (the first send that fills the socket buffer blocks for the whole test `WRITE_TIMEOUT` of 200 ms).
- second: panics with `the writer ends its side after the last line: Os { ..., kind: WouldBlock, ... }` after about 5 s — at master dropping the connection only closes its cloned descriptor; the test still holds `here`, so the peer never sees end-of-file.

- [ ] **Step 3: Implement**

**3a.** `crates/sprite-app/src/surface/channel.rs:20` — before → after:

```rust
use std::sync::{Arc, Mutex};
```
```rust
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
```

**3b.** Replace `crates/sprite-app/src/surface/channel.rs:205-390` — everything from `pub(crate) const EVENT_BUFFER_BYTES: usize = 64 * 1024;` through the closing `}` of `impl SurfaceConnection` (the line before `impl std::fmt::Debug for SurfaceConnection`) — with:

```rust
/// How far behind a program may fall. An event is queued whenever fewer
/// than this many bytes are pending — queued, or taken by the writer and not
/// yet written — so at most this plus one event waits, and no single event
/// is refused for its size alone. A program that lets this much pile up is
/// not reading; its connection is closed rather than allowed to grow.
pub(crate) const MAX_PENDING_BYTES: usize = 4 * 1024 * 1024;
/// The largest buffer a writer keeps between writes, so one burst does not
/// pin its memory for the rest of the connection's life.
pub(crate) const EVENT_BUFFER_BYTES: usize = 64 * 1024;

/// What one connection's handles and its writer thread share. The lock is
/// held to queue lines or to take them, never across a socket write.
#[derive(Default)]
struct Queue {
    /// `opened` is at the front of `queued`, so the writer may start. Lines
    /// queued before then wait behind it.
    ready: bool,
    /// The client is gone — refused, overflowed, timed out, or a write
    /// failed. Nothing more is queued or written.
    dead: bool,
    /// Every handle has been dropped: the writer sends what is queued and
    /// stops.
    closing: bool,
    /// Newline-terminated event lines, in the order they were sent.
    queued: Vec<u8>,
    /// Bytes the writer has taken from `queued` and is still writing. They
    /// count against the bound until the write returns.
    in_flight: usize,
    /// The writer thread has returned.
    #[cfg(test)]
    finished: bool,
}

impl Queue {
    /// Whether the writer has anything to do: lines it may send, or a
    /// reason to stop.
    fn has_work(&self) -> bool {
        self.dead || self.closing || (self.ready && !self.queued.is_empty())
    }

    /// Notes that the writer has returned, for the tests that wait on it.
    fn finish(&mut self) {
        #[cfg(test)]
        {
            self.finished = true;
        }
    }
}

struct Shared {
    queue: Mutex<Queue>,
    /// Wakes the writer when it has work, and anyone waiting on the writer
    /// when it has made progress.
    changed: Condvar,
    /// The socket. Only the writer thread writes to it; any other thread
    /// only ever shuts it down, which never blocks.
    stream: UnixStream,
}

impl Shared {
    /// Marks the connection dead and drops what was queued. With `shut_down`
    /// the socket is closed both ways too, so the connection thread's blocked
    /// read and the writer's blocked write both return at once. A refused
    /// open leaves it open: the connection thread writes the refusal next.
    fn kill(&self, mut queue: MutexGuard<'_, Queue>, shut_down: bool) {
        queue.dead = true;
        queue.queued = Vec::new();
        drop(queue);
        if shut_down {
            let _ = self.stream.shutdown(Shutdown::Both);
        }
        self.changed.notify_all();
    }
}

/// Owned by every clone of one [`SurfaceConnection`]. Dropping the last one
/// tells the writer to finish; it never waits for the writer to do so.
struct Handle {
    shared: Arc<Shared>,
}

impl Drop for Handle {
    fn drop(&mut self) {
        if let Ok(mut queue) = self.shared.queue.lock() {
            queue.closing = true;
        }
        self.shared.changed.notify_all();
    }
}

/// The window's end of one Surface's connection: the only way events reach
/// the program. Cloneable across threads because click handlers, focus
/// listeners, and the view all hold one — cloning shares the same queue and
/// the same socket, it does not open a second one.
///
/// Nothing here writes to the socket. Each connection owns one writer thread
/// that does, so `send`, `send_batch` and `establish` only queue and return:
/// a program that stops reading can never stall the GPUI thread. They all
/// share one queue, so lines reach the wire in the order they were queued and
/// a batch arrives in one piece.
///
/// A program may accept an `open` and send its first event in the same
/// breath, before the reply that accepted it has even reached the connection
/// thread. Lines queued before [`establish`](Self::establish) wait: it puts
/// `opened` ahead of them, so nothing a program was sent ever arrives ahead
/// of the confirmation that let it.
///
/// An event is queued only while fewer than [`MAX_PENDING_BYTES`] are
/// pending. Sending with the bound already reached, a write that times out,
/// or a failed write marks the connection dead and shuts the socket down; the connection thread's read then ends and the window hears
/// the Surface closed. Once the last clone is dropped the writer sends what
/// is still queued — `closed` included — ends its side, and returns. Nothing
/// joins the writer, so dropping a connection never waits on it.
#[derive(Clone)]
pub struct SurfaceConnection {
    handle: Arc<Handle>,
}

impl SurfaceConnection {
    pub(crate) fn new(stream: &UnixStream) -> std::io::Result<Self> {
        let stream = stream.try_clone()?;
        stream.set_write_timeout(Some(WRITE_TIMEOUT))?;
        let shared = Arc::new(Shared {
            queue: Mutex::new(Queue::default()),
            changed: Condvar::new(),
            stream,
        });
        let writer = Arc::clone(&shared);
        // Detached on purpose, and it still always ends. It returns as soon
        // as the connection is dead. Once the last handle is gone nothing more
        // can be queued, so what is left to drain is finite — at most the
        // pending bound plus one event — and every write that stops making
        // progress fails after `WRITE_TIMEOUT` and kills the connection. So
        // the thread outlives its last handle by at most that drain, and no
        // thread — the GPUI thread least of all — ever waits to join it.
        drop(
            std::thread::Builder::new()
                .name("sprite-surface-writer".to_owned())
                .spawn(move || write_events(&writer))?,
        );
        Ok(Self {
            handle: Arc::new(Handle { shared }),
        })
    }

    /// Queues one event line. `false` means the client is gone — refused,
    /// overflowed, timed out, or a write already failed — and nothing was
    /// queued. `true` means only that the line is queued, not that the
    /// program has read it.
    pub fn send(&self, line: &str) -> bool {
        self.send_batch([line])
    }

    /// Queues a complete gesture under one lock, so no other sender's lines
    /// land inside it. A line that finds the pending bound already reached
    /// kills the connection, and the iterator is not consumed past it.
    pub fn send_batch<'a>(&self, lines: impl IntoIterator<Item = &'a str>) -> bool {
        let shared = &self.handle.shared;
        let Ok(mut queue) = shared.queue.lock() else {
            return false;
        };
        if queue.dead {
            return false;
        }
        for line in lines {
            if queue.queued.len() + queue.in_flight >= MAX_PENDING_BYTES {
                shared.kill(queue, true);
                return false;
            }
            queue.queued.extend_from_slice(line.as_bytes());
            queue.queued.push(b'\n');
        }
        drop(queue);
        shared.changed.notify_all();
        true
    }

    /// Puts the connection's first line ahead of every line a program queued
    /// before it, and lets the writer start. Called once, by the connection
    /// thread that decided to accept the Surface — never by the program.
    /// `false` means the connection was already dead, or what a program
    /// queued before it had already reached the pending bound.
    pub(crate) fn establish(&self, line: &str) -> bool {
        let shared = &self.handle.shared;
        let Ok(mut queue) = shared.queue.lock() else {
            return false;
        };
        if queue.dead {
            return false;
        }
        if queue.queued.len() >= MAX_PENDING_BYTES {
            shared.kill(queue, true);
            return false;
        }
        let mut first = Vec::with_capacity(line.len() + 1 + queue.queued.len());
        first.extend_from_slice(line.as_bytes());
        first.push(b'\n');
        first.extend_from_slice(&queue.queued);
        queue.queued = first;
        queue.ready = true;
        drop(queue);
        shared.changed.notify_all();
        true
    }

    /// Marks the connection dead and drops anything a program queued: the
    /// open was refused or never answered, so nothing it sent was ever going
    /// to reach the wire, and a later `send` must say so rather than queue
    /// forever. The socket stays open for the refusal the connection thread
    /// writes next.
    fn abandon(&self) {
        let shared = &self.handle.shared;
        if let Ok(queue) = shared.queue.lock() {
            shared.kill(queue, false);
        }
    }

    #[cfg(test)]
    pub(crate) fn is_dead(&self) -> bool {
        self.handle
            .shared
            .queue
            .lock()
            .map(|queue| queue.dead)
            .unwrap_or(true)
    }

    /// Bytes queued or being written; zero once the connection is dead.
    #[cfg(test)]
    pub(crate) fn pending_bytes(&self) -> usize {
        let queue = self.handle.shared.queue.lock().unwrap();
        queue.queued.len() + queue.in_flight
    }

    /// Waits until the writer has put everything queued on the wire — or the
    /// connection died, or was never established — so a test can read the
    /// socket without racing the writer thread.
    #[cfg(test)]
    pub(crate) fn settle(&self) {
        let shared = &self.handle.shared;
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut queue = shared.queue.lock().unwrap();
        while !queue.dead && queue.ready && (!queue.queued.is_empty() || queue.in_flight > 0) {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            assert!(!left.is_zero(), "the writer did not drain its queue in 5 s");
            queue = shared.changed.wait_timeout(queue, left).unwrap().0;
        }
    }

    /// A view of this connection's writer that, unlike a clone, does not
    /// keep the writer running.
    #[cfg(test)]
    pub(crate) fn writer(&self) -> WriterProbe {
        WriterProbe(Arc::clone(&self.handle.shared))
    }
}

/// A connection's writer thread: waits for queued lines and writes them,
/// holding the lock only to take them. It returns when the connection dies,
/// or when every handle is gone and nothing that may be sent is left.
fn write_events(shared: &Shared) {
    let mut outgoing = Vec::new();
    loop {
        let Ok(mut queue) = shared.queue.lock() else {
            return;
        };
        while !queue.has_work() {
            queue = match shared.changed.wait(queue) {
                Ok(queue) => queue,
                Err(_) => return,
            };
        }
        if queue.dead || !queue.ready || queue.queued.is_empty() {
            // Dead, or closing with nothing left that may be sent. A
            // connection that was opened and is closing cleanly ends its
            // side, so the program reads every event and then the end of the
            // stream. A dead one was already shut down, or — refused — is
            // about to carry the connection thread's refusal.
            if !queue.dead && queue.ready {
                let _ = shared.stream.shutdown(Shutdown::Write);
            }
            queue.finish();
            drop(queue);
            shared.changed.notify_all();
            return;
        }
        std::mem::swap(&mut outgoing, &mut queue.queued);
        queue.in_flight = outgoing.len();
        drop(queue);
        let mut stream = &shared.stream;
        let written = stream.write_all(&outgoing).and_then(|()| stream.flush());
        outgoing.clear();
        if outgoing.capacity() > EVENT_BUFFER_BYTES {
            outgoing = Vec::new();
        }
        let Ok(mut queue) = shared.queue.lock() else {
            return;
        };
        queue.in_flight = 0;
        if written.is_err() {
            // The write timed out or failed: the program is not reading, or
            // is gone. Shutting the socket down ends the connection thread's
            // read, and the window hears the Surface closed.
            let _ = shared.stream.shutdown(Shutdown::Both);
            queue.finish();
            shared.kill(queue, false);
            return;
        }
        drop(queue);
        shared.changed.notify_all();
    }
}

/// A test's handle on one connection's writer thread.
#[cfg(test)]
pub(crate) struct WriterProbe(Arc<Shared>);

#[cfg(test)]
impl WriterProbe {
    /// Whether the writer thread returned within `limit`.
    pub(crate) fn finished_within(&self, limit: Duration) -> bool {
        let deadline = std::time::Instant::now() + limit;
        let mut queue = self.0.queue.lock().unwrap();
        while !queue.finished {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            if left.is_zero() {
                return false;
            }
            queue = self.0.changed.wait_timeout(queue, left).unwrap().0;
        }
        true
    }
}
```

`std::io::Write` is already imported (line 14) and covers `impl Write for &UnixStream`; `Shutdown` is imported (line 15). The `Debug` impl at old lines 392-396 stays as is.

**3c.** `crates/sprite-app/src/surface/channel.rs:647-651` (in `serve_surface`) — before:

```rust
    // Kept on this thread for as long as the connection lives: `connection`
    // itself moves into the request below, to the window, and every byte
    // this thread writes afterward — `opened`, and every refusal once the
    // Surface is open — has to go through the same lock the window's events
    // do, or the two race for the socket exactly as they used to.
```
after:
```rust
    // Kept on this thread for as long as the connection lives: `connection`
    // itself moves into the request below, to the window, and every line
    // this thread sends afterward — `opened`, and every refusal once the
    // Surface is open — goes into the same queue the window's events do, so
    // the connection's one writer puts them all on the wire in order.
```

**3d.** `crates/sprite-app/src/terminal_view/surfaces.rs:844-848` (`close_surface` doc) — before:

```rust
    /// Removes a Surface and returns its space to the grid. Always answers
    /// `closed`: a write to a connection that is already gone simply fails
    /// (and, per `SurfaceConnection::send`, marks it dead), and a client that
    /// only half-closed its write side — it is done sending, but is still
    /// reading — still hears `closed` the way its code expects.
```
after:
```rust
    /// Removes a Surface and returns its space to the grid. Always answers
    /// `closed`: on a connection that is already dead the line is simply
    /// dropped, and a client that only half-closed its write side — it is
    /// done sending, but is still reading — still hears `closed` the way its
    /// code expects, because the connection's writer sends everything queued
    /// before it stops.
```

**3e.** Update the existing tests in `crates/sprite-app/src/surface/channel.rs` `mod tests`.

Replace `a_huge_batch_stops_consuming_when_the_peer_stops_reading` (lines 842-864) with:

```rust
    #[test]
    fn a_huge_batch_stops_consuming_when_the_peer_stops_reading() {
        let (stream, _peer) = UnixStream::pair().unwrap();
        let connection = SurfaceConnection::new(&stream).unwrap();
        assert!(connection.establish(&event_opened(SurfaceId(1))));
        let line = "x".repeat(1023);
        let consumed = std::cell::Cell::new(0usize);
        let lines = std::iter::repeat_n(line.as_str(), 1_000_000_000).inspect(|_| {
            consumed.set(consumed.get() + 1);
            assert!(
                consumed.get() <= MAX_PENDING_BYTES / 1024 + 1,
                "iterator consumed past the pending bound"
            );
        });
        assert!(!connection.send_batch(lines));
        assert!(connection.is_dead());
        assert_eq!(connection.pending_bytes(), 0, "a dead connection holds nothing");
        println!(
            "backpressured batch consumed {} of 1000000000 lines",
            consumed.get()
        );
    }
```

Replace `a_failed_chunk_does_not_consume_the_remaining_iterator` (lines 866-882) with:

```rust
    #[test]
    fn a_failed_write_leaves_later_batches_unconsumed() {
        let (stream, peer) = UnixStream::pair().unwrap();
        let connection = SurfaceConnection::new(&stream).unwrap();
        let writer = connection.writer();
        assert!(connection.establish(&event_opened(SurfaceId(1))));
        drop(peer);
        // Whether the writer meets the closed peer on `opened` or on this
        // line, its write fails and the connection dies.
        let _ = connection.send("{}");
        assert!(writer.finished_within(WRITE_TIMEOUT * 10));
        assert!(connection.is_dead());
        let line = "x".repeat(1023);
        let consumed = std::cell::Cell::new(0);
        assert!(
            !connection.send_batch(
                std::iter::repeat_n(line.as_str(), 1_000_000_000)
                    .inspect(|_| consumed.set(consumed.get() + 1))
            )
        );
        assert_eq!(consumed.get(), 0);
    }
```

Replace `pre_open_overflow_discards_the_queue_and_cannot_establish_later` (lines 884-909) with:

```rust
    #[test]
    fn pre_open_overflow_discards_the_queue_and_cannot_establish_later() {
        use std::io::Read;
        let (stream, mut peer) = UnixStream::pair().unwrap();
        let connection = SurfaceConnection::new(&stream).unwrap();
        let line = "x".repeat(1023);
        assert!(connection.send_batch(std::iter::repeat_n(
            line.as_str(),
            MAX_PENDING_BYTES / 1024
        )));
        assert_eq!(connection.pending_bytes(), MAX_PENDING_BYTES);
        assert!(!connection.send("overflow"));
        assert_eq!(connection.pending_bytes(), 0);
        assert!(!connection.establish(&event_opened(SurfaceId(1))));
        assert!(connection.is_dead());
        let mut received = Vec::new();
        peer.read_to_end(&mut received).unwrap();
        assert!(
            received.is_empty(),
            "neither opened nor queued suffix may escape"
        );
    }
```

Replace `pre_open_gestures_stop_at_the_queue_limit_without_collecting_the_iterator` (lines 911-929) with:

```rust
    #[test]
    fn pre_open_gestures_stop_at_the_queue_limit_without_collecting_the_iterator() {
        let (stream, _peer) = UnixStream::pair().unwrap();
        let connection = SurfaceConnection::new(&stream).unwrap();
        let line = "x".repeat(1023);
        let consumed = std::cell::Cell::new(0);
        assert!(
            !connection.send_batch(
                std::iter::repeat_n(line.as_str(), 1_000_000_000)
                    .inspect(|_| consumed.set(consumed.get() + 1))
            )
        );
        assert_eq!(consumed.get(), MAX_PENDING_BYTES / 1024 + 1);
        assert!(connection.is_dead());
        // One event is never refused for its own size: it is admitted
        // because nothing was pending, and the next one finds the bound
        // reached.
        let (stream, _peer) = UnixStream::pair().unwrap();
        let connection = SurfaceConnection::new(&stream).unwrap();
        assert!(connection.send(&"x".repeat(MAX_PENDING_BYTES)));
        assert_eq!(connection.pending_bytes(), MAX_PENDING_BYTES + 1);
        assert!(!connection.send("x"));
        assert!(connection.is_dead());
        assert_eq!(connection.pending_bytes(), 0);
    }
```

In `chunked_batches_keep_the_gesture_lock_across_every_flush` (lines 931-977), replace the block from `connection` / `.wire` / `.lock()` / `.unwrap()` / `.stream` / `.shutdown(Shutdown::Write)` / `.unwrap();` (lines 956-962) with:

```rust
        assert!(!connection.is_dead());
        // A graceful close: the writer sends everything queued, then ends
        // its side, so the reader reaches the end of the stream.
        drop(connection);
```

and delete the last two lines of the test (lines 975-976):

```rust
        let (dead, buffer, queued) = connection.test_buffer_state();
        assert!(!dead && buffer <= EVENT_BUFFER_BYTES && queued <= EVENT_BUFFER_BYTES);
```

In `batches_follow_opened_and_keep_concurrent_gestures_contiguous` (lines 979-1023), replace lines 998-1004 (the same `connection.wire.lock().unwrap().stream.shutdown(Shutdown::Write).unwrap();` chain) with:

```rust
        drop(connection);
```

Replace `batch_buffer_is_reused_and_closed_peers_stop_further_sends` (lines 1025-1047) with:

```rust
    #[test]
    fn a_closed_peer_stops_further_sends() {
        let (stream, peer) = UnixStream::pair().unwrap();
        let connection = SurfaceConnection::new(&stream).unwrap();
        let writer = connection.writer();
        assert!(connection.establish(&event_opened(SurfaceId(1))));
        assert!(connection.send_batch(["one", "two"]));
        drop(peer);
        let _ = connection.send_batch(["gone"]);
        assert!(writer.finished_within(WRITE_TIMEOUT * 10));
        assert!(connection.is_dead());
        assert!(!connection.send_batch(["later"]));
    }
```

In `a_large_batch_delivers_every_byte_while_the_peer_drains_in_small_chunks` (lines 1049-1079), keep the 2 MiB lines (the second is admitted while about 2 MiB are pending) and replace lines 1071-1077 (the `connection.wire...shutdown(Shutdown::Write).unwrap();` chain) with:

```rust
        drop(connection);
```

Replace `a_backpressured_batch_keeps_its_written_prefix_and_shuts_down_on_timeout` (lines 1081-1098) with:

```rust
    #[test]
    fn a_backpressured_batch_keeps_its_written_prefix_and_shuts_down_on_timeout() {
        use std::io::Read;
        let (stream, mut peer) = UnixStream::pair().unwrap();
        let connection = SurfaceConnection::new(&stream).unwrap();
        let writer = connection.writer();
        let opened = event_opened(SurfaceId(1));
        assert!(connection.establish(&opened));
        let line = "x".repeat(2 * 1024 * 1024);
        let expected = format!("{opened}\n{line}\n");
        // The line fits the pending bound, so it is queued; the writer meets
        // the stall and waits out its write timeout.
        assert!(connection.send_batch([line.as_str()]));
        assert!(writer.finished_within(WRITE_TIMEOUT * 10));
        assert!(connection.is_dead());
        assert!(!connection.send("later"));
        let mut received = Vec::new();
        peer.read_to_end(&mut received).unwrap();
        assert!(received.len() > opened.len() + 1);
        assert!(received.len() < expected.len());
        assert!(expected.as_bytes().starts_with(&received));
    }
```

Change only the doc comment of `a_failed_write_marks_the_connection_dead_and_later_sends_return_at_once` (lines 1425-1427), body unchanged — before:

```rust
    /// A client that stops reading fills the socket; the write that hits the
    /// timeout marks the connection dead, and every send after it returns at
    /// once instead of waiting the timeout again.
```
after:
```rust
    /// A client that stops reading fills the socket and then the pending
    /// queue; passing the bound marks the connection dead, and every send
    /// after it returns at once instead of queuing more.
```

Replace `establish_stops_at_the_first_failed_write` (lines 1472-1485, doc comment included) with:

```rust
    /// The writer stops at the first failed write and marks the connection
    /// dead, so a queue of a thousand lines to a gone client costs one write.
    #[test]
    fn the_writer_stops_at_the_first_failed_write() {
        let (here, there) = UnixStream::pair().expect("pair");
        let connection = SurfaceConnection::new(&here).expect("connection");
        let writer = connection.writer();
        for _ in 0..4 {
            assert!(connection.send(&event_focus()));
        }
        drop(there);
        assert!(
            connection.establish(&event_opened(SurfaceId(1))),
            "establish only queues"
        );
        assert!(writer.finished_within(WRITE_TIMEOUT * 10));
        assert!(connection.is_dead());
        assert!(!connection.send(&event_focus()));
    }
```

Add this guard test after `a_graceful_close_sends_everything_queued_then_ends_the_stream` (it needs `writer()`, so it cannot be written before the implementation):

```rust
    /// The last handle is often dropped on the GPUI thread, so dropping it
    /// must never wait for the writer — even one stuck in a write — and the
    /// writer must still end on its own.
    #[test]
    fn dropping_the_last_handle_never_waits_for_a_stalled_writer() {
        let (here, _there) = UnixStream::pair().expect("pair");
        let connection = SurfaceConnection::new(&here).expect("connection");
        let writer = connection.writer();
        assert!(connection.establish(&event_opened(SurfaceId(1))));
        // Far more than a socket buffer holds, so the writer blocks in its
        // write until the timeout.
        assert!(connection.send(&"x".repeat(2 * 1024 * 1024)));
        let started = Instant::now();
        drop(connection);
        assert!(
            started.elapsed() < WRITE_TIMEOUT / 2,
            "dropping a connection waited on its writer"
        );
        assert!(
            writer.finished_within(WRITE_TIMEOUT * 10),
            "the writer outlived its connection and its write timeout"
        );
    }
```

And this guard after it — one event larger than the bound (a big Surface paste) is admitted and delivered, as it was with blocking writes:

```rust
    /// An event is never refused for its own size: one larger than the whole
    /// pending bound is admitted because nothing else was pending, and a
    /// program that reads gets every byte of it.
    #[test]
    fn a_single_event_larger_than_the_bound_is_delivered_whole() {
        use std::io::Read;
        let (here, mut there) = UnixStream::pair().expect("pair");
        let connection = SurfaceConnection::new(&here).expect("connection");
        let opened = event_opened(SurfaceId(1));
        assert!(connection.establish(&opened));
        let reader = std::thread::spawn(move || {
            let mut received = Vec::new();
            there.read_to_end(&mut received).unwrap();
            received
        });
        let paste = event_paste(&"x".repeat(MAX_PENDING_BYTES + 1024 * 1024));
        // Only `opened` can be pending, far under the bound, so the line is
        // admitted however large it is.
        assert!(connection.send(&paste));
        drop(connection);
        assert_eq!(
            reader.join().unwrap(),
            format!("{opened}\n{paste}\n").into_bytes()
        );
        drop(here);
    }
```

**3f.** `crates/sprite-app/src/terminal_view/surfaces.rs` tests: the writer thread now does the writing, so a test that reads a non-blocking peer right after the GPUI side sent would race it. Replace `open_request` and `events` (lines 1734-1767 and 1784-1793; leave `open_description` and `draw_test_window` between them as they are) with:

```rust
    /// The program's end of a test Surface's socket, with a handle on the
    /// window's end: events are written by the connection's writer thread,
    /// so a read first waits for it to put everything already sent on the
    /// wire.
    struct Peer {
        stream: std::os::unix::net::UnixStream,
        connection: SurfaceConnection,
    }

    fn open_request(
        host: &Entity<TerminalView>,
        cx: &mut gpui::VisualTestContext,
        id: SurfaceId,
        open: Open,
    ) -> (Result<(), Refusal>, Peer) {
        let (stream, peer) = std::os::unix::net::UnixStream::pair().unwrap();
        peer.set_nonblocking(true).unwrap();
        let connection = SurfaceConnection::new(&stream).unwrap();
        let mut peer = Peer {
            stream: peer,
            connection: connection.clone(),
        };
        let (reply, receiver) = std::sync::mpsc::sync_channel(1);
        dispatch(
            host,
            cx,
            SurfaceRequest::Open {
                pane: crate::pane_tree::PaneId(1),
                id,
                open,
                connection: connection.clone(),
                reply,
            },
        );
        let answer = receiver.try_recv().unwrap();
        if answer.is_ok() {
            let opened = crate::surface::channel::event_opened(id);
            assert!(connection.establish(&opened));
            draw_test_window(cx);
            let initial = events(&mut peer);
            assert_eq!(
                initial.first(),
                Some(&serde_json::from_str::<serde_json::Value>(&opened).unwrap())
            );
        }
        (answer, peer)
    }
```
```rust
    fn events(peer: &mut Peer) -> Vec<serde_json::Value> {
        use std::io::Read;
        peer.connection.settle();
        let mut wire = String::new();
        if let Err(error) = peer.stream.read_to_string(&mut wire) {
            assert_eq!(error.kind(), std::io::ErrorKind::WouldBlock);
        }
        wire.lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
```

Every existing `events(&mut peer)`, `open_request(...).0`, `let (answer, _peer) = open_request(...)` and `drop(peer)` call site compiles unchanged.

In `a_huge_finite_grid_wheel_stops_at_backpressure_with_bounded_storage`, replace lines 2026-2031:

```rust
            let (dead, buffer, queued) = host.surfaces.fill.as_ref().unwrap().connection.test_buffer_state();
            println!("huge wheel: elapsed={elapsed:?} buffer_capacity={buffer} queued_capacity={queued} dead={dead}");
            assert!(dead);
            assert!(buffer <= crate::surface::channel::EVENT_BUFFER_BYTES);
            assert!(queued <= crate::surface::channel::EVENT_BUFFER_BYTES);
            assert!(elapsed < std::time::Duration::from_secs(5));
```
with:
```rust
            let connection = &host.surfaces.fill.as_ref().unwrap().connection;
            let (dead, pending) = (connection.is_dead(), connection.pending_bytes());
            println!("huge wheel: elapsed={elapsed:?} pending={pending} dead={dead}");
            assert!(dead, "a billion wheel events cannot fit the pending bound");
            assert_eq!(pending, 0, "a dead connection holds nothing");
            assert!(elapsed < std::time::Duration::from_secs(5));
```

- [ ] **Step 4: Run tests, confirm pass**

```
TERM=dumb cargo test -p sprite-app --locked --offline surface::channel::tests
TERM=dumb cargo test -p sprite-app --locked --offline terminal_view::surfaces::tests
TERM=dumb cargo test -p sprite-app --locked --offline surface::render::tests
TERM=dumb cargo test -p sprite-app --locked --offline workspace::surface_routing
TERM=dumb cargo test -p sprite-app --locked --offline --test client
cargo clippy -p sprite-app --all-targets --locked --offline -- -D warnings
```

All pass, including the two Step 1 tests and the new guards `dropping_the_last_handle_never_waits_for_a_stalled_writer` and `a_single_event_larger_than_the_bound_is_delivered_whole`. Run `surface::channel::tests` three times in a row; the timing tests (`sends_to_a_peer_that_never_reads...`, `dropping_the_last_handle...`) must not flake.

- [ ] **Step 5: Commit**

```
git add crates/sprite-app/src/surface/channel.rs crates/sprite-app/src/terminal_view/surfaces.rs
git commit -m "fix(surface): write Surface events on a per-connection writer thread

send, send_batch and establish now only queue: each live connection owns
one writer thread and a 4 MiB pending queue, so a plugin that stops reading
can no longer freeze the GPUI thread for the write timeout. Overflow, a
write timeout or a write failure marks the connection dead and shuts the
socket down; dropping the last handle lets the writer send what is queued,
closed included, and is never joined.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 14: Linear-cost Surface ops (BCA-03, BCA-18)

PRD R-S2, R-S3. Two behaviours, two cycles: grid highlight relinks (14A), list guide validation (14B).

Today `Op::Highlights` (grid.rs:565-590) relinks each name by scanning every id's name list (`for names in self.groups.values_mut() { names.retain(..) }`) and then `retain`s the whole map — O(names × ids) per message, so 50k names cost billions of comparisons on the GPUI thread. And there is no cap on ids or names. `list.rs` `apply_state` (185-200) rebuilds the set of all guide ids by walking every row on each `active_guides` patch.

**Files:**
- Modify: `crates/sprite-app/src/surface/grid.rs:12` (imports), `:21-25` (new caps beside `MAX_COLS`/`MAX_ROWS`), `:445-486` (`GridSurface` fields and `new`), `:565-588` (`Op::Highlights` arm), `:752-763` (`style_for`), new `highlights_fit` after `apply`
- Modify: `crates/sprite-app/src/surface/list.rs:60-90` (`ListModel` + `Default`), `:149-178` (`apply_rows`), `:180-205` (`apply_state`), new `guide_ids` + test counter beside `id_index` (`:400-413`)
- Test: `crates/sprite-app/src/surface/grid.rs` existing `#[cfg(test)] mod tests` (line 838); `crates/sprite-app/src/surface/list.rs` existing `#[cfg(test)] mod tests` (line 536)

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces: `pub const MAX_HIGHLIGHT_IDS: usize = 262_144;` and `pub const MAX_GROUP_NAMES: usize = 262_144;` in `surface::grid`. `GridSurface::apply_all` / `apply` signatures unchanged; an over-cap highlights operation is refused as `Refusal::Malformed` like any other bad operation (earlier batch operations stand; `apply_all` prefixes `op N: `). `ListModel` public API unchanged (new private field `guide_ids`).

#### 14A — grid highlight reverse index and caps

- [ ] **Step 1: Write the failing tests**

Add to `crates/sprite-app/src/surface/grid.rs`, `mod tests`, right after `relinking_a_group_moves_its_name_to_the_new_id` (ends at line 1337). Reuses the module's `ops` helper (line 845). `check_groups` is a new helper in the style of the existing `check_pool`.

```rust
    /// The name index and the per-id lists describe the same thing: every
    /// name sits under exactly one id, at the stamp the index says, and no
    /// id is kept with no names.
    fn check_groups(grid: &GridSurface) {
        let mut seen = 0;
        for (id, names) in &grid.groups {
            assert!(!names.is_empty(), "id {id} is kept with no names");
            for (stamp, name) in names {
                assert_eq!(
                    grid.group_of.get(name),
                    Some(&(*id, *stamp)),
                    "{name} sits under id {id} but the index disagrees"
                );
                seen += 1;
            }
        }
        assert_eq!(seen, grid.group_of.len(), "the index names a group no id holds");
    }

    #[test]
    fn the_name_index_follows_every_relink_and_a_moved_name_leaves_its_old_id() {
        let mut grid = GridSurface::new(1, 1);
        let names = |grid: &GridSurface, id: u32| -> Vec<String> {
            grid.groups[&id].values().cloned().collect()
        };
        grid.apply_all(ops(json!({ "type": "highlights",
            "define": { "1": {}, "2": {} },
            "groups": { "Comment": 1, "@comment": 1, "String": 2 } })))
        .expect("applies");
        check_groups(&grid);
        assert_eq!(names(&grid, 1), ["Comment", "@comment"]);

        grid.apply_all(ops(json!({ "type": "highlights", "groups": { "Comment": 2 } })))
            .expect("applies");
        check_groups(&grid);
        assert_eq!(names(&grid, 1), ["@comment"], "Comment left id 1");
        assert_eq!(names(&grid, 2), ["String", "Comment"]);

        // Naming an id it already has moves the name last, so it layers over
        // the others exactly as a fresh name would.
        grid.apply_all(ops(json!({ "type": "highlights", "groups": { "String": 2 } })))
            .expect("applies");
        check_groups(&grid);
        assert_eq!(names(&grid, 2), ["Comment", "String"]);

        grid.apply_all(ops(json!({ "type": "highlights", "groups": { "@comment": 2 } })))
            .expect("applies");
        check_groups(&grid);
        assert_eq!(grid.group_ids(), vec![2], "id 1 kept no names, so it is not kept");
    }

    #[test]
    fn a_relink_touches_one_stored_entry_however_many_the_grid_holds() {
        let mut grid = GridSurface::new(1, 1);
        let names = |offset: u32| -> Vec<(String, u32)> {
            (0..50_000u32)
                .map(|n| (format!("group-{n}"), n + offset))
                .collect()
        };
        grid.apply(Op::Highlights {
            define: Vec::new(),
            groups: names(1),
        })
        .expect("applies");
        RELINKED_ENTRIES.with(|count| count.set(0));
        grid.apply(Op::Highlights {
            define: Vec::new(),
            groups: names(2),
        })
        .expect("applies");
        assert_eq!(
            RELINKED_ENTRIES.with(|count| count.get()),
            50_000,
            "each relink should touch only the entry it moves"
        );
        check_groups(&grid);
        assert_eq!(grid.group_ids().len(), 50_000);
    }

    /// A coarse guard against a quadratic relink, not a benchmark: the bound
    /// is generous enough for a debug build on a loaded machine, and the work
    /// runs on its own thread so a regression fails here instead of hanging.
    #[test]
    fn relinking_fifty_thousand_names_finishes_well_inside_a_generous_bound() {
        let (done, finished) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut grid = GridSurface::new(1, 1);
            for offset in [1, 2] {
                grid.apply(Op::Highlights {
                    define: Vec::new(),
                    groups: (0..50_000u32)
                        .map(|n| (format!("group-{n}"), n + offset))
                        .collect(),
                })
                .expect("applies");
            }
            let _ = done.send(grid.group_ids().len());
        });
        assert_eq!(
            finished.recv_timeout(std::time::Duration::from_secs(30)),
            Ok(50_000),
            "relinking 50k names took more than 30 s, or panicked"
        );
    }

    #[test]
    fn a_highlights_operation_past_either_cap_is_refused_before_it_changes_anything() {
        let mut grid = GridSurface::new(2, 1);
        grid.apply(Op::Highlights {
            define: (1..=MAX_HIGHLIGHT_IDS as u32)
                .map(|id| (id, Attrs::default()))
                .collect(),
            groups: Vec::new(),
        })
        .expect("exactly the id cap fits");
        grid.apply(Op::Highlights {
            define: Vec::new(),
            groups: (0..MAX_GROUP_NAMES).map(|n| (format!("g{n}"), 1)).collect(),
        })
        .expect("exactly the name cap fits");

        // On its own, an over-cap operation changes nothing, and a bare
        // operation's refusal carries no prefix.
        let before = grid.clone();
        for message in [
            json!({ "type": "highlights", "define": { "1": { "bold": true }, "262145": {} } }),
            json!({ "type": "highlights", "define": { "1": { "bold": true } },
                    "groups": { "g0": 2, "OneMore": 1 } }),
        ] {
            let refusal = grid
                .apply_all(ops(message.clone()))
                .expect_err("over a cap");
            assert!(
                matches!(&refusal, Refusal::Malformed(why) if !why.starts_with("op ")),
                "{message} was refused as {refusal:?}"
            );
            assert!(grid == before, "{message} changed the grid");
        }

        // Inside a batch it is refused like any other bad operation: the
        // operations before it stand, it changes nothing itself — not even
        // the id it would have redefined — and the reason names it.
        let refusal = grid
            .apply_all(ops(json!({ "type": "batch", "ops": [
                { "type": "rows", "rows": [{ "row": 0, "cells": [["x", 1]] }] },
                { "type": "highlights", "define": { "1": { "bold": true }, "262145": {} } }
            ] })))
            .expect_err("op 1 is over the id cap");
        assert!(
            refusal.reason().starts_with("malformed: op 1: "),
            "{}",
            refusal.reason()
        );
        assert_eq!(
            grid.texts.text(grid.cells[0][0].text).as_str(),
            "x",
            "op 0 stood"
        );
        assert!(grid.attrs == before.attrs, "the over-cap operation redefined an id");

        let refusal = grid
            .apply_all(ops(json!({ "type": "batch", "ops": [
                { "type": "cursor", "row": 0, "col": 1 },
                { "type": "highlights", "groups": { "g0": 2, "OneMore": 1 } }
            ] })))
            .expect_err("op 1 is over the name cap");
        assert!(
            refusal.reason().starts_with("malformed: op 1: "),
            "{}",
            refusal.reason()
        );
        assert_eq!(grid.cursor.col, 1, "op 0 stood");
        assert!(
            grid.groups == before.groups && grid.group_of == before.group_of,
            "the over-cap operation relinked a name"
        );
        check_groups(&grid);

        // At the caps, redefining an id and relinking a name grow nothing,
        // so they still apply.
        grid.apply_all(ops(json!({ "type": "highlights",
            "define": { "1": { "bold": true } }, "groups": { "g0": 2 } })))
        .expect("no growth");
        check_groups(&grid);
    }
```

- [ ] **Step 2: Run them and confirm they fail**

```
TERM=dumb cargo test -p sprite-app --locked --offline surface::grid::tests
```

Expected at master: compile errors — `no field 'group_of' on type '&GridSurface'`, `cannot find value 'RELINKED_ENTRIES' in this scope`, `cannot find value 'MAX_HIGHLIGHT_IDS' in this scope`, `MAX_GROUP_NAMES` likewise, and `no method named 'values' found for reference '&Vec<String>'` (the per-id lists are `Vec`s today). (If the executor wants to watch the quadratic cost itself, temporarily comment out the other three tests and `check_groups`: the 50k guard test then fails after 30 s.)

- [ ] **Step 3: Implement**

`grid.rs:12` — before → after:

```rust
use std::collections::HashMap;
```
```rust
use std::collections::{BTreeMap, HashMap, HashSet};
```

After `pub const MAX_ROWS: u16 = 1024;` (line 25), add:

```rust
/// How many highlight ids and group names one grid keeps. An editor's
/// adapter forwards ids as the editor allocates them and never retires one,
/// so these are generous: a refusal mid-session would leave that grid's
/// highlighting wrong for the rest of it.
pub const MAX_HIGHLIGHT_IDS: usize = 262_144;
pub const MAX_GROUP_NAMES: usize = 262_144;

#[cfg(test)]
thread_local! {
    /// Stored group entries a relink found and moved. Every stored name a
    /// relink reads must be counted here, so a test can tell work that
    /// grows with the message from work that grows with the grid.
    static RELINKED_ENTRIES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
```

In `struct GridSurface` (lines 452-458), before:

```rust
    attrs: HashMap<u32, Attrs>,
    /// Every group name an id has been given, in the order they arrived.
    ///
    /// One id commonly stands for several names — an editor maps `Comment`,
    /// `@comment`, and `@comment.lua` to the same attrs — so keeping only the
    /// last would let one name shadow a theme entry written for another.
    groups: HashMap<u32, Vec<String>>,
```
after:
```rust
    attrs: HashMap<u32, Attrs>,
    /// Every group name an id has been given, in the order they arrived,
    /// keyed by an arrival stamp so one name can leave its id without the
    /// others moving.
    ///
    /// One id commonly stands for several names — an editor maps `Comment`,
    /// `@comment`, and `@comment.lua` to the same attrs — so keeping only the
    /// last would let one name shadow a theme entry written for another.
    groups: HashMap<u32, BTreeMap<u64, String>>,
    /// Where each group name sits now: its id and its stamp under that id.
    /// A relink finds the one entry it moves here, instead of searching
    /// every id's names.
    group_of: HashMap<String, (u32, u64)>,
    /// The stamp the next group name to arrive is given.
    next_group_stamp: u64,
```

In `GridSurface::new` (line 482), before → after:

```rust
            groups: HashMap::new(),
```
```rust
            groups: HashMap::new(),
            group_of: HashMap::new(),
            next_group_stamp: 0,
```

`apply_all` and `apply` keep their bodies: an over-cap highlights operation is refused by `apply` like any other bad operation, so earlier operations in a batch stand and `apply_all` adds its usual `op N: ` prefix.

Replace the whole `Op::Highlights { define, groups } => { ... }` arm (lines 565-588) with:

```rust
            Op::Highlights { define, groups } => {
                // Checked before anything below runs, so an operation over a
                // cap changes nothing at all.
                self.highlights_fit(&define, &groups)?;
                let mut changed = false;
                for (id, attrs) in define {
                    changed |= self.attrs.get(&id) != Some(&attrs);
                    self.attrs.insert(id, attrs);
                }
                for (name, id) in groups {
                    changed |= self
                        .groups
                        .get(&id)
                        .and_then(|names| names.last_key_value())
                        .map(|(_, last)| last)
                        != Some(&name);
                    let stamp = self.next_group_stamp;
                    self.next_group_stamp += 1;
                    // A relink moves the name: an editor that now maps
                    // `Comment` to another attr id no longer means the old one
                    // by it. The index says where the name was, so a relink
                    // costs the same however many names the grid holds.
                    if let Some((old_id, old_stamp)) =
                        self.group_of.insert(name.clone(), (id, stamp))
                        && let Some(names) = self.groups.get_mut(&old_id)
                    {
                        #[cfg(test)]
                        RELINKED_ENTRIES.with(|count| count.set(count.get() + 1));
                        names.remove(&old_stamp);
                        // An id the relink emptied is dropped rather than kept
                        // with no names: `style_for` would look it up and find
                        // nothing to apply, and the entry would outlive the
                        // only reason it existed.
                        if names.is_empty() {
                            self.groups.remove(&old_id);
                        }
                    }
                    self.groups.entry(id).or_default().insert(stamp, name);
                }
                if changed {
                    self.invalidate();
                }
            }
```

Add this method directly after `apply` (i.e. before `pub fn invalidate`, line 708). It is called only from the `Op::Highlights` arm above:

```rust
    /// Refuses one highlights operation that would grow the grid past either
    /// cap. Ids and names are never forgotten, so a grid at its caps can still
    /// redefine and relink what it has, but not add to it.
    fn highlights_fit(
        &self,
        define: &[(u32, Attrs)],
        groups: &[(String, u32)],
    ) -> Result<(), Refusal> {
        let new_ids: HashSet<u32> = define
            .iter()
            .map(|(id, _)| *id)
            .filter(|id| !self.attrs.contains_key(id))
            .collect();
        let new_names: HashSet<&str> = groups
            .iter()
            .map(|(name, _)| name.as_str())
            .filter(|name| !self.group_of.contains_key(*name))
            .collect();
        if self.attrs.len() + new_ids.len() > MAX_HIGHLIGHT_IDS {
            return Err(malformed(format!(
                "a grid defines at most {MAX_HIGHLIGHT_IDS} highlight ids"
            )));
        }
        if self.group_of.len() + new_names.len() > MAX_GROUP_NAMES {
            return Err(malformed(format!(
                "a grid names at most {MAX_GROUP_NAMES} highlight groups"
            )));
        }
        Ok(())
    }
```

In `style_for` (line 760), before → after:

```rust
            for style in names.iter().filter_map(|name| theme.get(name)) {
```
```rust
            for style in names.values().filter_map(|name| theme.get(name)) {
```

`group_ids` (line 811) still reads `self.groups.keys()` and needs no change.

- [ ] **Step 4: Run tests, confirm pass**

```
TERM=dumb cargo test -p sprite-app --locked --offline surface::grid::tests
TERM=dumb cargo test -p sprite-app --locked --offline surface::render::tests::a_grid_becomes_an_element_without_a_window -- --exact
TERM=dumb cargo test -p sprite-app --locked --offline terminal_view::surfaces::tests
```

All existing grid tests (`relinking_a_group_moves_its_name_to_the_new_id`, `several_group_names_for_one_id_each_reach_the_theme_and_the_later_one_layers_over`, `cached_rows_follow_actual_cell_and_theme_changes`, `generated_grid_transitions_match_a_frozen_copy_oracle`, …) pass unchanged.

- [ ] **Step 5: Commit**

```
git add crates/sprite-app/src/surface/grid.rs
git commit -m "fix(surface): relink grid highlight groups through a name index

A highlights message relinked each group name by scanning every id's names,
so 50k names cost billions of comparisons on the GPUI thread. A name to id
index now moves each name directly. A grid keeps at most 262,144 highlight
ids and 262,144 group names; a highlights operation that would pass either
is refused before it changes anything, like any other bad operation in a
batch.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

#### 14B — list guide ids gathered once per `list_rows`

- [ ] **Step 6: Write the failing test**

Add to `crates/sprite-app/src/surface/list.rs`, `mod tests`, right after `identified_guides_follow_state_and_surviving_rows` (ends at line 657, before `malformed_state_keeps_the_last_complete_state`). It follows the `INDEX_BUILDS` counter pattern of `large_replacements_build_one_index_and_state_reuses_it`.

```rust
    #[test]
    fn active_guides_are_checked_against_ids_gathered_once_per_rows() {
        let rows = |revision: u64, guide: &str| {
            serde_json::json!({"type":"list_rows","revision":revision,"selected":null,
                "rows":(0..1_000).map(|i| serde_json::json!({"id":format!("r{i}"),"text":"row","indent":0,
                    "guides":[{"offset":8,"id":format!("{guide}-{}", i % 10)}]})).collect::<Vec<_>>()})
        };
        let state = |revision: u64, ids: Value| {
            parse_op(&serde_json::json!({"type":"list_state","revision":revision,"active_guides":ids}))
                .unwrap()
        };
        let mut model = ListModel::default();
        GUIDE_SCANS.with(|count| count.set(0));
        model.apply(parse_op(&rows(1, "old")).unwrap()).unwrap();
        for _ in 0..100 {
            model
                .apply(state(1, serde_json::json!(["old-3", "old-7"])))
                .unwrap();
        }
        assert_eq!(
            GUIDE_SCANS.with(|count| count.get()),
            1,
            "an active_guides patch walked the rows again"
        );

        model.apply(parse_op(&rows(2, "new")).unwrap()).unwrap();
        assert_eq!(GUIDE_SCANS.with(|count| count.get()), 2);
        assert!(model.active_guides.is_empty(), "no old guide survived the rows");
        assert!(
            model.apply(state(2, serde_json::json!(["old-3"]))).is_err(),
            "a guide of the replaced rows is no longer valid"
        );
        model.apply(state(2, serde_json::json!(["new-3"]))).unwrap();
        assert_eq!(model.active_guides, HashSet::from(["new-3".to_owned()]));
    }
```

- [ ] **Step 7: Run it and confirm it fails**

```
TERM=dumb cargo test -p sprite-app --locked --offline surface::list::tests::active_guides_are_checked_against_ids_gathered_once_per_rows -- --exact
```

Expected at master: compile error `cannot find value 'GUIDE_SCANS' in this scope`. (Behaviourally, master walks all 1,000 rows on each of the 100 patches.)

- [ ] **Step 8: Implement**

In `struct ListModel` (lines 69-70), before:

```rust
    pub active_guides: HashSet<String>,
    ids: HashMap<String, usize>,
```
after:
```rust
    pub active_guides: HashSet<String>,
    /// Every guide id the current rows carry, gathered once per `list_rows`
    /// so an `active_guides` patch checks its ids without walking the rows.
    guide_ids: HashSet<String>,
    ids: HashMap<String, usize>,
```

In `impl Default for ListModel` (lines 84-85), before → after:

```rust
            active_guides: HashSet::new(),
            ids: HashMap::new(),
```
```rust
            active_guides: HashSet::new(),
            guide_ids: HashSet::new(),
            ids: HashMap::new(),
```

In `apply_rows`, replace lines 170-177:

```rust
        let surviving: HashSet<&str> = self
            .rows
            .iter()
            .flat_map(|row| row.guides.iter().filter_map(|guide| guide.id.as_deref()))
            .collect();
        self.active_guides
            .retain(|id| surviving.contains(id.as_str()));
```
with:
```rust
        self.guide_ids = guide_ids(&self.rows);
        self.active_guides.retain(|id| self.guide_ids.contains(id));
```

In `apply_state`, replace lines 192-205:

```rust
        if let Some(value) = patch.get("active_guides") {
            active_guides = parse_active_guides(value)?;
            let available: HashSet<&str> = self
                .rows
                .iter()
                .flat_map(|row| row.guides.iter().filter_map(|guide| guide.id.as_deref()))
                .collect();
            if active_guides
                .iter()
                .any(|id| !available.contains(id.as_str()))
            {
                return Err(malformed("active guide does not exist"));
            }
        }
```
with:
```rust
        if let Some(value) = patch.get("active_guides") {
            active_guides = parse_active_guides(value)?;
            if active_guides.iter().any(|id| !self.guide_ids.contains(id)) {
                return Err(malformed("active guide does not exist"));
            }
        }
```

Directly after `id_index` (ends at line 413), add:

```rust
#[cfg(test)]
thread_local! { static GUIDE_SCANS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }

/// The distinct guide ids across `rows`: the one walk of every row's guides
/// that a `list_rows` pays, kept so a state patch never repeats it.
fn guide_ids(rows: &[ListRow]) -> HashSet<String> {
    #[cfg(test)]
    GUIDE_SCANS.with(|count| count.set(count.get() + 1));
    rows.iter()
        .flat_map(|row| row.guides.iter().filter_map(|guide| guide.id.clone()))
        .collect()
}
```

- [ ] **Step 9: Run tests, confirm pass**

```
TERM=dumb cargo test -p sprite-app --locked --offline surface::list::tests
TERM=dumb cargo test -p sprite-app --locked --offline terminal_view::list_view::tests
TERM=dumb cargo test -p sprite-app --locked --offline terminal_view::surfaces::tests
cargo clippy -p sprite-app --all-targets --locked --offline -- -D warnings
```

`identified_guides_follow_state_and_surviving_rows` and `shared_wire_fixture_applies_atomically_and_retains_last_good_state` pass unchanged.

- [ ] **Step 10: Commit**

```
git add crates/sprite-app/src/surface/list.rs
git commit -m "fix(surface): check active list guides against ids gathered per rows

Each active_guides patch walked every row to rebuild the set of guide ids.
The set is now built once per list_rows and kept, so a patch costs only
the ids it names.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 15: SVG cache by content survives update; list rows keyed by row key (BCA-10, BCA-17)

PRD R-S4 (BCA-10) and the BCA-17 local fix. Two behaviours, two cycles: element-image cache (15A), virtual-list row ids (15B).

Today an element Surface's decoded SVGs are cached by element index (`ElementImageCache.images: BTreeMap<u64, …>`, render.rs:24-35, filled at :299-311) and `Body::replace_description` throws the whole cache away on every `update` (terminal_view/surfaces.rs:112-117), so a plugin that re-sends an unchanged description re-rasterises every SVG. The budgets live at render.rs:57-59 (`MAX_SURFACE_IMAGE_BYTES` 64 MiB per Surface, `MAX_SVG_RASTER_BYTES` 16 MiB per bitmap, `MAX_SVG_RASTER_DIMENSION` 4096) and are enforced in `render_svg_with_budget` (render.rs:98-107) and by the `available` computation in `element()` (render.rs:301-306).

Virtual-list rows get `ElementId::NamedInteger("surface-<id>-row", index)` (terminal_view/list_view.rs:507-510). GPUI keeps a press's pending click under the element id (vendor/gpui/src/elements/div.rs:2124-2235), so if a row is inserted above between press and release, the release fires the click handler of whichever row now has that index, naming the wrong row.

**Files:**
- Modify: `crates/sprite-app/src/surface/render.rs:1-5` (module doc + imports), `:24-35` (`ElementImageCache`), `:299-317` (`Element::Image` arm of `element`)
- Modify: `crates/sprite-app/src/terminal_view/surfaces.rs:112-117` (`Body::replace_description`, `Elements` arm)
- Modify: `crates/sprite-app/src/terminal_view/list_view.rs:119-136` (add `row_element_id` after `revealed_pixel_offset`), `:158-169` (test accessor), `:506-510` (row id)
- Test: `crates/sprite-app/src/surface/render.rs` existing `#[cfg(test)] mod tests` (line 383; tests at :419-473 and :487-529 change); `crates/sprite-app/src/terminal_view/surfaces.rs` existing `#[cfg(test)] mod tests` (line 1483; `legacy_svg_cache_follows_description_update_and_surface_close` at :1626-1723 changes); `crates/sprite-app/src/terminal_view/list_view.rs` existing `#[cfg(test)] mod tests` (line 909)

**Interfaces:**
- Consumes (Task 13): in `terminal_view/surfaces.rs` tests, `struct Peer`, `fn open_request(host, cx, id, open) -> (Result<(), Refusal>, Peer)`, `fn events(peer: &mut Peer) -> Vec<serde_json::Value>` (waits for the writer via `SurfaceConnection::settle`).
- Produces: `pub(crate) fn ElementImageCache::retain_drawn_by(&mut self, description: &Description)`; the cache is `HashMap<Arc<str>, Option<Arc<RenderImage>>>` keyed by SVG text (hashed, then the stored text compared on a hit — no collisions possible); test-only `ElementImageCache::{image_for(&self, svg: &str) -> Option<Arc<RenderImage>>, cached_texts(&self) -> Vec<&str>, decodes(&self) -> usize, retained_bytes(&self) -> usize}`; `ElementImageCache::image_id` is removed. `fn row_element_id(surface: SurfaceId, key: &str) -> ElementId` (private to list_view.rs); test-only `VirtualListView::viewport_bounds(&self) -> Option<Bounds<Pixels>>`.

#### 15A — element images cached by content

- [ ] **Step 1: Write the failing tests**

Add to `crates/sprite-app/src/terminal_view/surfaces.rs`, `mod tests`, right after `legacy_svg_cache_follows_description_update_and_surface_close` (ends at line 1723, before `fn dispatch`). It follows that test's setup (`TerminalView::failed`, `open_surface` with a `Fill` placement, rendering via `crate::surface::render::render` exactly as `surface_element` does).

```rust
    #[gpui::test]
    fn an_update_keeps_the_decode_of_an_unchanged_svg_and_decodes_a_changed_one_once(
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
        let svg = |color: &str| {
            format!("<svg xmlns='http://www.w3.org/2000/svg' width='4' height='4'><rect width='4' height='4' fill='{color}'/></svg>")
        };
        let document = |first: &str, second: &str| {
            serde_json::json!({"version":1,"root":{"kind":"box","children":[
                {"kind":"image","style":"w_4 h_4","svg":svg(first)},
                {"kind":"image","style":"w_4 h_4","svg":svg(second)}
            ]}})
        };
        let id = SurfaceId(3);
        host.update_in(cx, |view, window, cx| {
            view.open_surface(
                id,
                Open {
                    placement: crate::surface::channel::Placement::Fill { owner_pid: None },
                    focus: false,
                    description: document("blue", "green"),
                },
                connection,
                window,
                cx,
            )
            .unwrap();
        });
        let registry = TokenRegistry::new(&settings.colors);
        // Builds the Surface's elements as a frame does, then reports how many
        // SVGs its cache has decoded in all and what it holds for each colour.
        let frame = |view: &mut TerminalView, colors: &[&str]| {
            let surface = view.surfaces.fill.as_mut().unwrap();
            let connection = surface.connection.clone();
            let Body::Elements {
                description,
                images,
            } = &mut surface.body
            else {
                panic!("expected elements")
            };
            let _ = crate::surface::render::render(
                description,
                id,
                &registry,
                &connection,
                None,
                images,
            );
            (
                images.decodes(),
                colors
                    .iter()
                    .map(|color| images.image_for(&svg(color)))
                    .collect::<Vec<_>>(),
            )
        };

        let (decodes, first) = host.update(cx, |view, _| frame(view, &["blue", "green"]));
        assert_eq!(decodes, 2);
        let blue = first[0].clone().expect("blue decoded");
        let green = first[1].clone().expect("green decoded");

        host.update(cx, |view, cx| {
            view.update_surface(id, document("blue", "green"), cx)
        });
        let (decodes, again) = host.update(cx, |view, _| frame(view, &["blue", "green"]));
        assert_eq!(decodes, 2, "an identical update decoded its SVGs again");
        assert!(std::sync::Arc::ptr_eq(&blue, again[0].as_ref().unwrap()));
        assert!(std::sync::Arc::ptr_eq(&green, again[1].as_ref().unwrap()));

        host.update(cx, |view, cx| {
            view.update_surface(id, document("blue", "red"), cx)
        });
        let (decodes, changed) =
            host.update(cx, |view, _| frame(view, &["blue", "green", "red"]));
        assert_eq!(decodes, 3, "only the changed SVG is decoded");
        assert!(std::sync::Arc::ptr_eq(&blue, changed[0].as_ref().unwrap()));
        assert!(
            changed[1].is_none(),
            "green is no longer drawn, so its pixels are released"
        );
        assert!(changed[2].is_some());
        host.update(cx, |view, _| {
            let Body::Elements { images, .. } = &view.surfaces.fill.as_ref().unwrap().body else {
                panic!("expected elements")
            };
            assert_eq!(
                images.retained_bytes(),
                2 * 4 * 4 * 4,
                "the budget counts only the images the description draws"
            );
        });
        cx.update(|window, _| window.remove_window());
        drop(host);
    }
```

Add to `crates/sprite-app/src/surface/render.rs`, `mod tests`, right after `element_image_cache_bounds_retained_pixels_and_resets` (ends at line 473). It reuses the module's imports (`UnixStream`, `json`, `Colors`, `description`) as the neighbouring tests do.

```rust
    #[test]
    fn retaining_a_new_description_releases_what_it_does_not_draw_and_retries_what_did_not_fit() {
        let (ours, _peer) = UnixStream::pair().unwrap();
        let connection = SurfaceConnection::new(&ours).unwrap();
        let registry = TokenRegistry::new(&Colors::default());
        // Each one is a distinct 16 MiB raster: four fill the 64 MiB budget.
        let svg = |n: usize| {
            format!("<svg xmlns='http://www.w3.org/2000/svg' width='2048' height='2048'><desc>{n}</desc></svg>")
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
        let mut cache = ElementImageCache::default();
        let _ = render(
            &document(&[0, 1, 2, 3, 4]),
            SurfaceId(1),
            &registry,
            &connection,
            None,
            &mut cache,
        );
        assert!(
            cache.image_for(&svg(4)).is_none(),
            "the fifth image is over the budget"
        );
        let kept = cache.image_for(&svg(1)).unwrap();
        let decodes = cache.decodes();

        let next = document(&[1, 4]);
        cache.retain_drawn_by(&next);
        assert_eq!(cache.retained_bytes(), 16 * 1024 * 1024);
        assert!(cache.image_for(&svg(0)).is_none());
        let _ = render(&next, SurfaceId(1), &registry, &connection, None, &mut cache);
        // Entries hold the full SVG text and a lookup compares it, so the
        // cache holds exactly the two texts drawn, each under its own text.
        assert_eq!(cache.cached_texts(), [svg(1).as_str(), svg(4).as_str()]);
        assert!(Arc::ptr_eq(&kept, &cache.image_for(&svg(1)).unwrap()));
        assert!(
            cache.image_for(&svg(4)).is_some(),
            "the update freed room for it"
        );
        assert_eq!(cache.decodes(), decodes + 1);
        assert_eq!(cache.retained_bytes(), 32 * 1024 * 1024);
    }
```

- [ ] **Step 2: Run them and confirm they fail**

```
TERM=dumb cargo test -p sprite-app --locked --offline an_update_keeps_the_decode_of_an_unchanged_svg_and_decodes_a_changed_one_once
TERM=dumb cargo test -p sprite-app --locked --offline retaining_a_new_description_releases_what_it_does_not_draw_and_retries_what_did_not_fit
```

Expected at master: compile errors — `no method named 'decodes' found for mutable reference '&mut ElementImageCache'`, `no method named 'image_for' found …`, `no method named 'retained_bytes' found …`, `no method named 'retain_drawn_by' found …`. (Behaviourally, master's `replace_description` resets the cache, so the identical update would decode twice more.)

- [ ] **Step 3: Implement**

`render.rs:1-5` — before:

```rust
//! Draws a Surface Description: one GPUI element per described element,
//! built fresh on every frame the way GPUI's own views are. Decoded SVGs stay
//! with the description so redraws reuse them and an update releases them.

use std::collections::BTreeMap;
```
after:
```rust
//! Draws a Surface Description: one GPUI element per described element,
//! built fresh on every frame the way GPUI's own views are. Decoded SVGs are
//! kept by their SVG text, so redraws — and updates that keep an SVG —
//! reuse them, and an update releases the ones it no longer draws.

use std::collections::{HashMap, HashSet};
```

Replace `render.rs:24-35` (`#[derive(Default)] pub(crate) struct ElementImageCache { … }` and its `#[cfg(test)] impl` with `image_id`) with:

```rust
/// The decoded images of one element Surface, kept across frames and across
/// updates for as long as its description draws them.
///
/// Keyed by the SVG text itself: the map hashes it to find an entry and then
/// compares the stored text, so two different SVGs can never share a
/// picture. Elements that draw the same SVG share one decode, and an update
/// that keeps an SVG keeps its decode wherever in the tree it moved.
#[derive(Default)]
pub(crate) struct ElementImageCache {
    images: HashMap<Arc<str>, Option<Arc<RenderImage>>>,
    retained_bytes: usize,
    #[cfg(test)]
    decodes: usize,
}

impl ElementImageCache {
    /// The picture for one SVG, decoded only if no earlier frame or
    /// description already did. A decode is bounded by what the Surface's
    /// budget has left, so the pictures it holds never pass 64 MiB together.
    fn picture(&mut self, svg: &str) -> Option<Arc<RenderImage>> {
        if let Some(picture) = self.images.get(svg) {
            return picture.clone();
        }
        #[cfg(test)]
        {
            self.decodes += 1;
        }
        let available = MAX_SURFACE_IMAGE_BYTES - self.retained_bytes;
        let picture = if available >= MAX_SVG_RASTER_BYTES {
            render_svg(svg, None)
        } else {
            render_svg_with_budget(svg, None, available)
        };
        if let Some(picture) = &picture {
            self.retained_bytes += picture.as_bytes(0).expect("raster frame").len();
        }
        self.images.insert(Arc::from(svg), picture.clone());
        picture
    }

    /// Keeps the decodes `description` still draws and releases the rest, so
    /// the budget counts only images on screen. A decode that failed or did
    /// not fit is forgotten too: the next frame tries it again against
    /// whatever the update freed.
    pub(crate) fn retain_drawn_by(&mut self, description: &Description) {
        let mut drawn = HashSet::new();
        drawn_images(&description.root, &mut drawn);
        self.images
            .retain(|svg, picture| picture.is_some() && drawn.contains(&**svg));
        self.retained_bytes = self
            .images
            .values()
            .flatten()
            .map(|picture| picture.as_bytes(0).expect("raster frame").len())
            .sum();
    }
}

/// The SVG text of every image a description's tree draws.
fn drawn_images<'a>(node: &'a Element, drawn: &mut HashSet<&'a str>) {
    match node {
        Element::Image { svg, .. } => {
            drawn.insert(svg.as_str());
        }
        Element::Box { children, .. } | Element::List { children, .. } => {
            for child in children {
                drawn_images(child, drawn);
            }
        }
        Element::Text { .. }
        | Element::Button { .. }
        | Element::Grid { .. }
        | Element::VirtualList { .. } => {}
    }
}

#[cfg(test)]
impl ElementImageCache {
    pub(crate) fn image_for(&self, svg: &str) -> Option<Arc<RenderImage>> {
        self.images.get(svg).cloned().flatten()
    }

    /// The SVG texts the cache holds entries for, sorted.
    pub(crate) fn cached_texts(&self) -> Vec<&str> {
        let mut texts: Vec<&str> = self.images.keys().map(|svg| &**svg).collect();
        texts.sort_unstable();
        texts
    }

    pub(crate) fn decodes(&self) -> usize {
        self.decodes
    }

    pub(crate) fn retained_bytes(&self) -> usize {
        self.retained_bytes
    }
}
```

In `element()`, replace the `Element::Image` arm (render.rs:299-317) — before:

```rust
        Element::Image { style, svg } => {
            let picture = images.images.entry(index).or_insert_with(|| {
                let available = MAX_SURFACE_IMAGE_BYTES - images.retained_bytes;
                let picture = if available >= MAX_SVG_RASTER_BYTES {
                    render_svg(svg, None)
                } else {
                    render_svg_with_budget(svg, None, available)
                };
                if let Some(picture) = &picture {
                    images.retained_bytes += picture.as_bytes(0).expect("raster frame").len();
                }
                picture
            });
            return match picture {
                Some(picture) => {
                    style::apply_all(img(picture.clone()), &style.utilities).into_any_element()
                }
                None => div().into_any_element(),
            };
        }
```
after:
```rust
        Element::Image { style, svg } => {
            return match images.picture(svg) {
                Some(picture) => {
                    style::apply_all(img(picture), &style.utilities).into_any_element()
                }
                None => div().into_any_element(),
            };
        }
```

`crates/sprite-app/src/terminal_view/surfaces.rs:112-117` (`Body::replace_description`) — before:

```rust
            (body @ Self::Elements { .. }, root) => {
                *body = Self::Elements {
                    description: Description { root },
                    images: Default::default(),
                };
            }
```
after:
```rust
            (Self::Elements { description, images }, root) => {
                *description = Description { root };
                // Images the new description still draws keep their decode;
                // the rest are released, so the budget counts only what is
                // on screen.
                images.retain_drawn_by(description);
            }
```

Update the existing tests that read the old index-keyed cache.

In `crates/sprite-app/src/surface/render.rs`, replace `element_image_cache_bounds_retained_pixels_and_resets` (lines 419-473, `#[test]` included) with — five *distinct* SVGs, because identical ones now share one decode:

```rust
    #[test]
    fn element_image_cache_bounds_retained_pixels_and_resets() {
        let (ours, _peer) = UnixStream::pair().unwrap();
        let connection = SurfaceConnection::new(&ours).unwrap();
        let registry = TokenRegistry::new(&Colors::default());
        // Distinct texts: identical SVGs would share a single decode.
        let svg = |n: usize| {
            format!("<svg xmlns='http://www.w3.org/2000/svg' width='2048' height='2048'><desc>{n}</desc></svg>")
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
        let mut cache = ElementImageCache::default();
        let _ = render(
            &document,
            SurfaceId(1),
            &registry,
            &connection,
            None,
            &mut cache,
        );
        let retained: usize = (0..5)
            .filter_map(|n| cache.image_for(&svg(n)))
            .map(|image| image.as_bytes(0).unwrap().len())
            .sum();
        assert_eq!(retained, 64 * 1024 * 1024);
        assert!(cache.image_for(&svg(4)).is_none());
        let first = cache.image_for(&svg(0)).unwrap().id;
        let _ = render(
            &document,
            SurfaceId(1),
            &registry,
            &connection,
            None,
            &mut cache,
        );
        assert_eq!(cache.image_for(&svg(0)).unwrap().id, first);
        assert!(cache.image_for(&svg(4)).is_none());
        cache = ElementImageCache::default();
        let fresh = description::parse(
            &json!({"version":1,"root":{"kind":"image","svg":svg(4)}}),
            &registry,
        )
        .unwrap()
        .description;
        let _ = render(
            &fresh,
            SurfaceId(1),
            &registry,
            &connection,
            None,
            &mut cache,
        );
        assert!(cache.image_for(&svg(4)).is_some());
    }
```

Replace `legacy_image_redraw_reuses_decode_and_new_cache_decodes_changed_svg` (lines 487-529, `#[test]` included) with:

```rust
    #[test]
    fn legacy_image_redraw_reuses_decode_and_new_cache_decodes_changed_svg() {
        let (ours, _theirs) = UnixStream::pair().unwrap();
        let connection = SurfaceConnection::new(&ours).unwrap();
        let registry = TokenRegistry::new(&Colors::default());
        let svg = |color: &str| {
            format!("<svg xmlns='http://www.w3.org/2000/svg' width='4' height='4'><rect width='4' height='4' fill='{color}'/></svg>")
        };
        let document = |color: &str| {
            description::parse(
                &json!({"version":1,"root":{"kind":"image","style":"w_4 h_4","svg":svg(color)}}),
                &registry,
            )
            .unwrap()
            .description
        };
        let blue = document("blue");
        let mut cache = ElementImageCache::default();
        let _ = render(
            &blue,
            SurfaceId(1),
            &registry,
            &connection,
            None,
            &mut cache,
        );
        let first = cache.image_for(&svg("blue")).unwrap();
        let _ = render(
            &blue,
            SurfaceId(1),
            &registry,
            &connection,
            None,
            &mut cache,
        );
        assert!(Arc::ptr_eq(&first, &cache.image_for(&svg("blue")).unwrap()));
        let first_id = first.id;
        drop(first);
        let red = document("red");
        cache = ElementImageCache::default();
        let _ = render(&red, SurfaceId(1), &registry, &connection, None, &mut cache);
        let second = cache.image_for(&svg("red")).unwrap();
        assert_eq!(&second.as_bytes(0).unwrap()[..4], &[0, 0, 255, 255]);
        assert_ne!(second.id, first_id);
    }
```

In `crates/sprite-app/src/terminal_view/surfaces.rs`, `legacy_svg_cache_follows_description_update_and_surface_close` (lines 1626-1723): replace the line

```rust
        let document = |color| serde_json::json!({"version":1,"root":{"kind":"image","style":"w_4 h_4","svg":format!("<svg xmlns='http://www.w3.org/2000/svg' width='4' height='4'><rect width='4' height='4' fill='{color}'/></svg>")}});
```
with
```rust
        let svg = |color: &str| {
            format!("<svg xmlns='http://www.w3.org/2000/svg' width='4' height='4'><rect width='4' height='4' fill='{color}'/></svg>")
        };
        let document = |color: &str| {
            serde_json::json!({"version":1,"root":{"kind":"image","style":"w_4 h_4","svg":svg(color)}})
        };
```
then replace each `images.image_id(0)` as follows:
- `let first = images.image_id(0).unwrap();` → `let first = images.image_for(&svg("blue")).unwrap().id;`
- `(first, images.image_id(0).unwrap())` → `(first, images.image_for(&svg("blue")).unwrap().id)`
- `assert!(images.image_id(0).is_none());` (after the update to red) → `assert!(images.image_for(&svg("blue")).is_none(), "blue is no longer drawn, so its decode was released");`
- `images.image_id(0).unwrap()` (in `replacement`) → `images.image_for(&svg("red")).unwrap().id`

The rest of that test is unchanged.

- [ ] **Step 4: Run tests, confirm pass**

```
TERM=dumb cargo test -p sprite-app --locked --offline surface::render::tests
TERM=dumb cargo test -p sprite-app --locked --offline terminal_view::surfaces::tests
TERM=dumb cargo test -p sprite-app --locked --offline terminal_view::list_view::tests
```

- [ ] **Step 5: Commit**

```
git add crates/sprite-app/src/surface/render.rs crates/sprite-app/src/terminal_view/surfaces.rs
git commit -m "fix(surface): keep element SVG decodes by content across updates

Every update of an element Surface discarded its whole image cache, so a
plugin re-sending an unchanged description re-rasterised every SVG. Decodes
are now keyed by their SVG text (compared, not just hashed), survive update,
and an update releases only the images it no longer draws, keeping the
16 MiB per-bitmap and 64 MiB per-Surface budgets.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

#### 15B — virtual-list row element ids from the row key

The row key is `ListRow::id: String` (surface/list.rs:20), unique within a `list_rows` message (`id_index`, list.rs:403-413).

- [ ] **Step 6: Write the failing test**

Test scaffolding first: add this accessor to `impl VirtualListView` in `crates/sprite-app/src/terminal_view/list_view.rs`, directly after `viewport_rows` (ends at line 169):

```rust
    #[cfg(test)]
    pub(super) fn viewport_bounds(&self) -> Option<Bounds<gpui::Pixels>> {
        self.viewport
    }
```

Add to `crates/sprite-app/src/terminal_view/surfaces.rs`, `mod tests`, right after `surface_list_100k_virtualization_probe` (ends at line 2107). It reuses `open_request`, `open_description`, `dispatch`, `draw_test_window` and `events`/`Peer` (Task 13), and the shared fixture `tests/fixtures/surface-list-v1.json` (row height 22) the neighbouring list tests use.

```rust
    #[gpui::test]
    fn a_row_inserted_above_between_press_and_release_does_not_take_the_click(
        cx: &mut gpui::TestAppContext,
    ) {
        fn list_clicks(peer: &mut Peer) -> Vec<serde_json::Value> {
            events(peer)
                .into_iter()
                .filter(|event| event["type"] == "list_click")
                .collect()
        }
        let settings = crate::config::Settings::default();
        cx.set_global(crate::config::ActiveSettings(settings.clone()));
        cx.set_global(TokenRegistry::new(&settings.colors));
        let (host, cx) = cx.add_window_view(|window, cx| {
            TerminalView::failed("test".into(), ".SystemUIFont".into(), window, cx)
        });
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/surface-list-v1.json"
        ))
        .unwrap();
        let id = SurfaceId(986);
        let pane = crate::pane_tree::PaneId(1);
        let (answer, mut peer) = open_request(
            &host,
            cx,
            id,
            open_description(fixture["description"].clone()),
        );
        assert_eq!(answer, Ok(()));
        let rows = |revision: u64, keys: &[&str]| {
            crate::surface::list::parse_op(&serde_json::json!({
                "type": "list_rows", "revision": revision, "selected": null,
                "rows": keys
                    .iter()
                    .map(|key| serde_json::json!({"id": key, "text": key, "indent": 0, "guides": []}))
                    .collect::<Vec<_>>(),
            }))
            .unwrap()
        };
        dispatch(
            &host,
            cx,
            SurfaceRequest::List {
                id,
                pane,
                op: rows(1, &["a", "b", "c"]),
            },
        );
        // The first frame measures the list's viewport; the second lays its
        // rows out inside it.
        draw_test_window(cx);
        draw_test_window(cx);
        let viewport = host.read_with(cx, |host, cx| {
            let Body::List { view, .. } = &host.surfaces.fill.as_ref().unwrap().body else {
                panic!("list")
            };
            view.read(cx)
                .viewport_bounds()
                .expect("the list was laid out")
        });
        // Rows start at the viewport's top while the list is unscrolled: this
        // is the middle of the second row, "b".
        let row_height = fixture["description"]["root"]["row_height"]
            .as_f64()
            .unwrap() as f32;
        let on_b = gpui::point(
            viewport.origin.x + px(40.0),
            viewport.origin.y + px(row_height * 1.5),
        );
        list_clicks(&mut peer);

        // With nothing in between, a press and release on b is a click on
        // b: the harness does deliver row clicks.
        cx.simulate_mouse_down(on_b, MouseButton::Left, gpui::Modifiers::default());
        cx.simulate_mouse_up(on_b, MouseButton::Left, gpui::Modifiers::default());
        let control = list_clicks(&mut peer);
        assert_eq!(control.len(), 1, "{control:?}");
        assert_eq!(control[0]["id"], "b");

        // A row arrives above b between press and release, so the pointer
        // now rests on the newcomer. The press belonged to b; the release
        // must not become a click on another row.
        cx.simulate_mouse_down(on_b, MouseButton::Left, gpui::Modifiers::default());
        dispatch(
            &host,
            cx,
            SurfaceRequest::List {
                id,
                pane,
                op: rows(2, &["a", "new", "b", "c"]),
            },
        );
        draw_test_window(cx);
        cx.simulate_mouse_up(on_b, MouseButton::Left, gpui::Modifiers::default());
        let moved = list_clicks(&mut peer);
        assert!(
            moved.iter().all(|click| click["id"] == "b"),
            "a press on b was released as {moved:?}"
        );
        cx.update(|window, _| window.remove_window());
        drop(host);
    }
```

- [ ] **Step 7: Run it and confirm it fails**

```
TERM=dumb cargo test -p sprite-app --locked --offline terminal_view::surfaces::tests::a_row_inserted_above_between_press_and_release_does_not_take_the_click -- --exact
```

Expected at master (plus Task 13): panics with `a press on b was released as [Object {"button": String("left"), "count": Number(1), "id": String("new"), …}]` — the press's pending click is stored under `surface-986-row-1`, which now belongs to `new`.

If the control assertion (`control.len() == 1`) fails instead, row clicks do not reach the list in this harness; stop and report rather than weakening the test (see Drafter notes).

- [ ] **Step 8: Implement**

In `crates/sprite-app/src/terminal_view/list_view.rs`, add after `revealed_pixel_offset` (ends at line 136, before the `TRUNCATE_CALLS` thread-local):

```rust
/// A row's element identity is its key, not its position. GPUI keeps a
/// press's pending click under the element's id, so a row inserted above
/// between press and release must not hand that press to whichever row now
/// sits where the pressed one was.
fn row_element_id(surface: SurfaceId, key: &str) -> ElementId {
    ElementId::NamedChild(
        Box::new(ElementId::NamedInteger(
            SharedString::from("surface-row"),
            surface.0,
        )),
        SharedString::from(key.to_owned()),
    )
}
```

In the `uniform_list` row builder (lines 506-510), before:

```rust
                        let mut line = div()
                            .id(ElementId::NamedInteger(
                                SharedString::from(format!("surface-{}-row", surface.0)),
                                index as u64,
                            ))
```
after:
```rust
                        let mut line = div()
                            .id(row_element_id(surface, &row.id))
```

(`index` is still used by `let row = &rows[index];`.)

Add a unit test to `list_view.rs` `mod tests` (line 909, after `scrollbar_handles_empty_and_fit_content`):

```rust
    #[test]
    fn a_rows_element_id_follows_its_key_not_its_position() {
        let surface = SurfaceId(7);
        assert_eq!(row_element_id(surface, "b"), row_element_id(surface, "b"));
        assert_ne!(row_element_id(surface, "b"), row_element_id(surface, "c"));
        assert_ne!(
            row_element_id(surface, "b"),
            row_element_id(SurfaceId(8), "b")
        );
    }
```

- [ ] **Step 9: Run tests, confirm pass**

```
TERM=dumb cargo test -p sprite-app --locked --offline terminal_view::surfaces::tests
TERM=dumb cargo test -p sprite-app --locked --offline terminal_view::list_view::tests
cargo clippy -p sprite-app --all-targets --locked --offline -- -D warnings
```

`a_row_inserted_above_…` now passes: after the insertion GPUI drops b's pending press, because b's hitbox is no longer under the pointer (div.rs:2224-2231), and `new` has no pending press, so no click is sent. `surface_list_100k_virtualization_probe` and the other list tests pass unchanged.

- [ ] **Step 10: Commit**

```
git add crates/sprite-app/src/terminal_view/list_view.rs crates/sprite-app/src/terminal_view/surfaces.rs
git commit -m "fix(surface): key virtual-list row elements by row id

Rows were identified by index, so a row inserted above between press and
release made the release click whichever row then sat under the pointer.
Each row's element id now comes from its key.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

---

### Task 16: Shaped-text cache, paint-ready cell text and allocation-free box drawing (BCA-04, BCA-29; PRD R-R1, R-R3)

Three test-then-implement cycles: (A) `PositionedCell.text` becomes a `SharedString` made at row layout; (B) a per-row shape cache beside `LayoutCache`, with count tests at Sprite's one grid `shape_line` call site; (C) box-drawing strokes and outlines in fixed arrays.

**Files:**
- Modify: `crates/sprite-app/src/grid.rs:12-17` (imports), `:54-65` (`PositionedCell`), `:82-109` (`lay_out_row`)
- Modify: `crates/sprite-app/src/grid_paint.rs:52-67` (imports), `:193-314` (`GridPaint`, `GridPaintSpec`, `prepare`, `prepare_spec`, `new`), `:546-611` (`paint` loop), `:615-703` (`blank_glyph`, `paint_glyph`)
- Modify: `crates/sprite-app/src/box_drawing.rs:65-70` (`Outline`), `:358-459` (`box_rects`), `:482-501` (`stroke_lines`), `:540-582` (`box_outlines`, `diagonal`)
- Modify: `crates/sprite-app/src/terminal_view.rs:103` (field), `:395` and `:486` (both constructors)
- Modify: `crates/sprite-app/src/terminal_view/render.rs:315` (`GridPaint::prepare` call)
- Modify: `crates/sprite-app/src/paint_benchmark.rs:40-63` (`PaintBenchmark`), `:92-106` (spec)
- Modify: `crates/sprite-app/src/surface/grid.rs:386-440` (`TextEntry`, `TextPool::intern`, `TextPool::text`)
- Modify: `crates/sprite-app/src/surface/render.rs:1-22` (imports), `:212-238` (`render_grid`), `:677` (test call)
- Modify: `crates/sprite-app/src/terminal_view/surfaces.rs:37-45` (`Body::Grid`), `:59-62` (construction), `:1130-1132` (`render_grid` call)
- Modify: `crates/sprite-app/src/surface_performance.rs:103-111` (`render_grid` calls)
- Test: `crates/sprite-app/src/grid.rs` (`mod tests`), `crates/sprite-app/src/grid_paint.rs` (`mod tests`), `crates/sprite-app/src/box_drawing.rs` (`mod tests`)

**Interfaces:**
- Consumes: Task 8's `pub focused: bool` field on `GridPaintSpec` (every `GridPaintSpec { .. }` literal below sets it; keep whatever value Task 8 already wrote at each existing literal).
- Produces:
  - `pub(crate) fn crate::grid::shared_text(text: &str) -> gpui::SharedString`
  - `PositionedCell.text: gpui::SharedString`
  - `pub(crate) struct crate::grid_paint::ShapeCache` (`Default`) with `pub(crate) fn begin_frame(&mut self, context: ShapeContext, rows: usize)` and `pub(crate) fn shaped(&mut self, row: usize, cells: &Arc<Vec<PositionedCell>>, column: usize, color: Rgba, shape: impl FnOnce() -> ShapedLine) -> Arc<ShapedLine>`
  - `pub(crate) struct crate::grid_paint::ShapeContext { family, font_size, scale, default_fg, default_bg, palette }`
  - `pub shapes: Rc<RefCell<ShapeCache>>` on `GridPaintSpec`; `shapes: Rc<RefCell<ShapeCache>>` on `GridPaint`
  - `GridPaint::prepare(snapshot, rows, metrics, split, shapes: &Rc<RefCell<ShapeCache>>)` (new last parameter)
  - `fn GridPaint::shape_context(&self, scale: f32) -> ShapeContext` (private, used by Task 21)
  - `pub(crate) fn crate::grid_paint::reaches_text_system(text: &str) -> bool`
  - `#[cfg(test)] pub(crate) static crate::grid_paint::SHAPED_CELLS: Cell<usize>` (thread-local)
  - `TerminalView.shape_cache: Rc<RefCell<ShapeCache>>`
  - `PaintBenchmark.shapes: Rc<RefCell<ShapeCache>>` (private field)
  - `render_grid(grid, highlights, metrics, shapes: &Rc<RefCell<ShapeCache>>)`

#### Cycle A — cell text is a `SharedString` made at layout

> **Superseded (2026-10-09, after the whole-branch review):** `PositionedCell.text` stays `sprite_term::CellText`, `ShapeKey` is keyed by `CellText`, and the `SharedString` is built only inside the shape-cache miss closure; `PRINTABLE_ASCII`, `shared_text` and this cycle's tests were removed (see PRD R-R1).

- [ ] **Step 1: Write the failing test** — append to `mod tests` in `crates/sprite-app/src/grid.rs` (after `an_empty_row_lays_out_to_nothing`, before the module's closing `}` at line 362). It reuses that module's existing `cell(text, width)` and `row(cells)` helpers (lines 249-263) and the test-only counting allocator `crate::surface_performance::measure` (`crates/sprite-app/src/surface_performance.rs:72`).

```rust
    /// The text the painter shapes is made once, when the row is laid out.
    /// A printable ASCII cell borrows a static string, so laying out a row of
    /// them costs the row's own vector and nothing per cell.
    #[test]
    fn layout_hands_the_painter_shared_text_without_copying_ascii() {
        let ascii = row(
            (0..200)
                .map(|column| cell(if column % 2 == 0 { "a" } else { " " }, CellWidth::Narrow))
                .collect(),
        );
        let (laid, allocations, _) = crate::surface_performance::measure(|| lay_out_row(&ascii));
        let text: &gpui::SharedString = &laid[0].text;
        assert_eq!(text, "a");
        assert_eq!(laid[1].text, " ");
        assert_eq!(allocations, 1, "one vector for the row, nothing per cell");

        let other = lay_out_row(&row(vec![
            cell("界", CellWidth::Wide),
            cell("", CellWidth::SpacerTail),
            cell("e\u{301}", CellWidth::Narrow),
        ]));
        assert_eq!(other[0].text, "界");
        assert_eq!(other[1].text, "e\u{301}");
    }

    #[test]
    fn the_printable_ascii_table_holds_each_character_at_its_offset() {
        assert_eq!(PRINTABLE_ASCII.len(), 95);
        for (index, byte) in PRINTABLE_ASCII.bytes().enumerate() {
            assert_eq!(usize::from(byte), 0x20 + index);
        }
        assert_eq!(shared_text("~"), "~");
        assert_eq!(shared_text(""), "");
    }
```

- [ ] **Step 2: Run it and confirm it fails**

```sh
TERM=dumb cargo test -p sprite-app --lib --locked --offline grid::tests::layout_hands_the_painter -- --exact
```

Expected: compile error — `mismatched types: expected &SharedString, found &CellText` at `let text: &gpui::SharedString = &laid[0].text;`, and `cannot find value PRINTABLE_ASCII` / `cannot find function shared_text`.

- [ ] **Step 3: Implement**

`crates/sprite-app/src/grid.rs` imports (line 14):

```rust
use gpui::{Pixels, Point, SharedString, Size, px, size};
```

Replace the `text` field of `PositionedCell` (line 61):

```rust
    /// The cell's text, already in the form the text system shapes, made
    /// once when the row is laid out rather than on every frame that paints it.
    pub text: SharedString,
```

Insert directly above `lay_out_row` (before line 78):

```rust
/// Every printable ASCII character, in order, so a one-byte cell can borrow
/// its text from here instead of allocating a copy.
const PRINTABLE_ASCII: &str = " !\"#$%&'()*+,-./0123456789:;<=>?@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_`abcdefghijklmnopqrstuvwxyz{|}~";

/// A cell's text in the form the text system takes.
///
/// Printable ASCII is by far the commonest cell and borrows a static string;
/// anything else is copied once, here, when its row is laid out.
pub(crate) fn shared_text(text: &str) -> SharedString {
    match text.as_bytes() {
        [] => SharedString::new_static(""),
        [byte @ b' '..=b'~'] => {
            let index = usize::from(byte - b' ');
            SharedString::new_static(&PRINTABLE_ASCII[index..=index])
        }
        _ => SharedString::from(text.to_owned()),
    }
}
```

In `lay_out_row` change line 96:

```rust
                    text: shared_text(&cell.text),
```

`crates/sprite-app/src/grid_paint.rs` `paint_glyph` line 651 (replaced again in Cycle B, but must compile now):

```rust
        let text = cell.text.clone();
```

`crates/sprite-app/src/surface/grid.rs` — the grid Surface keeps its text interned, so the pool holds the paint form directly and laying out a grid row copies no text. Replace lines 386-391:

```rust
#[derive(Clone, Debug, PartialEq)]
struct TextEntry {
    text: Arc<str>,
    paint: gpui::SharedString,
    references: usize,
}
```

Replace lines 410-414 in `TextPool::intern`:

```rust
        let entry = TextEntry {
            paint: gpui::SharedString::new(Arc::clone(&text)),
            text: text.clone(),
            references: 1,
        };
```

Replace lines 438-440:

```rust
    fn text(&self, id: u32) -> &gpui::SharedString {
        &self.entries[id as usize].as_ref().unwrap().paint
    }
```

(Callers at lines 634, 732, 737, 741, 1063, 1095 compile unchanged: `SharedString` has `as_str`, derefs to `str`, and `clone` is a reference-count increment.)

- [ ] **Step 4: Run tests, confirm pass**

```sh
TERM=dumb cargo test -p sprite-app --lib --locked --offline grid::
TERM=dumb cargo test -p sprite-app --lib --locked --offline surface::grid::
TERM=dumb cargo test -p sprite-app --lib --locked --offline terminal_view::placeholder::
TERM=dumb cargo test -p sprite-app --lib --locked --offline surface_allocation_probe
```

All pass; `surface_allocation_probe` keeps its existing bounds (`a <= 1` idle, `a <= 8` one-row) because the pool now hands out shared text.

- [ ] **Step 5: Commit**

```sh
git add crates/sprite-app/src/grid.rs crates/sprite-app/src/grid_paint.rs crates/sprite-app/src/surface/grid.rs
git commit -m "perf(grid): make cell text paint-ready once at row layout

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

#### Cycle B — shaped glyphs cached per row, counted at the call site

- [ ] **Step 1: Write the failing tests** — in `crates/sprite-app/src/grid_paint.rs` `mod tests` (line 906). Add the probe view and helper at the top of the module, directly after `use crate::tokens::unpack;` (line 909), and the two tests at the end of the module (before the closing `}` at line 1463). The probe paints `crate::paint_benchmark::fixture()` (`paint_benchmark.rs:110`, 60×200, cursor at row 21 column 14 on a selected wide cell) through the real `GridPaint` element in a GPUI test window, so every count is taken at the real `shape_line` call site. Reuses `plain_style` (line 1091) and `unpack`.

```rust
    /// A view that paints one fixture snapshot through the live painter, so a
    /// test can count what a real frame asks the text system for.
    struct ShapeProbe {
        snapshot: RenderSnapshot,
        layout: crate::grid::LayoutCache,
        shapes: Rc<RefCell<ShapeCache>>,
        blink_on: bool,
        font_size: Pixels,
    }

    impl gpui::Render for ShapeProbe {
        fn render(
            &mut self,
            _window: &mut Window,
            _cx: &mut gpui::Context<Self>,
        ) -> impl IntoElement {
            use gpui::{ParentElement, Styled};
            let rows = crate::grid::prepare_rows(&mut self.layout, Some(&self.snapshot), None);
            let cursor =
                Some(self.snapshot.cursor).filter(|cursor| self.blink_on || !cursor.blinking);
            let (paint, _) = GridPaint::prepare_spec(
                GridPaintSpec {
                    rows,
                    pass: RowPass::Whole,
                    cursor,
                    cursor_color: self.snapshot.cursor_color,
                    default_fg: self.snapshot.default_foreground,
                    default_bg: self.snapshot.default_background,
                    palette: Some(Arc::clone(&self.snapshot.palette)),
                    cell_width: px(8.4),
                    cell_height: px(18.0),
                    font_family: "monospace".into(),
                    font_size: self.font_size,
                    focused: true,
                    shapes: Rc::clone(&self.shapes),
                },
                false,
            );
            gpui::div().size_full().child(paint)
        }
    }

    /// Asks the cache for one cell and reports whether it had to shape.
    fn shapes_anew(cache: &mut ShapeCache, cells: &Arc<Vec<PositionedCell>>, color: Rgba) -> bool {
        let mut shaped = false;
        let _ = cache.shaped(0, cells, 0, color, || {
            shaped = true;
            ShapedLine::default()
        });
        shaped
    }
```

```rust
    /// The gate R-R1 sets, counted where Sprite calls `shape_line`: an
    /// unchanged frame shapes nothing, a blink at most the cursor's cell, a
    /// one-row change only that row, and a font or theme change everything a
    /// cold cache would.
    #[gpui::test]
    fn shaping_happens_only_for_cells_whose_drawn_text_changed(cx: &mut gpui::TestAppContext) {
        let (probe, cx) = cx.add_window_view(|_, _| ShapeProbe {
            snapshot: crate::paint_benchmark::fixture(),
            layout: Default::default(),
            shapes: Default::default(),
            blink_on: true,
            font_size: px(14.0),
        });
        let frame = |cx: &mut gpui::VisualTestContext| -> usize {
            SHAPED_CELLS.with(|count| count.set(0));
            cx.update(|window, cx| {
                window.refresh();
                window.draw(cx).clear();
            });
            SHAPED_CELLS.with(|count| count.get())
        };

        let first = frame(cx);
        assert!(first > 0, "the first frame shapes what it shows");
        assert_eq!(frame(cx), 0, "an unchanged frame shapes nothing");

        probe.update(cx, |probe, _| probe.blink_on = false);
        assert!(frame(cx) <= 1, "a blink reshapes at most the cursor's cell");
        probe.update(cx, |probe, _| probe.blink_on = true);
        assert!(frame(cx) <= 1, "a blink reshapes at most the cursor's cell");

        probe.update(cx, |probe, _| {
            probe.snapshot.generation += 1;
            Arc::make_mut(&mut probe.snapshot.rows[30]).cells[10].text = "Z".into();
        });
        let shapeable = probe.update(cx, |probe, _| {
            crate::grid::lay_out_row(&probe.snapshot.rows[30])
                .iter()
                .filter(|cell| reaches_text_system(&cell.text))
                .count()
        });
        let one_row = frame(cx);
        assert!(one_row >= 1, "the changed cell is shaped");
        assert!(
            one_row <= shapeable,
            "only the changed row may reshape: {one_row} shapes for {shapeable} cells"
        );

        probe.update(cx, |probe, _| probe.font_size = px(15.0));
        let refont = frame(cx);
        probe.update(cx, |probe, _| probe.shapes = Rc::default());
        let cold = frame(cx);
        assert!(cold > 0);
        assert_eq!(refont, cold, "a font change reshapes everything a cold cache would");

        probe.update(cx, |probe, _| probe.snapshot.default_foreground.r ^= 0xff);
        let rethemed = frame(cx);
        probe.update(cx, |probe, _| probe.shapes = Rc::default());
        assert_eq!(
            rethemed,
            frame(cx),
            "a theme change reshapes everything a cold cache would"
        );
    }

    /// Scale factor cannot be changed on a test window, so its invalidation is
    /// checked on the cache directly, alongside colour and row identity.
    #[test]
    fn a_scale_or_font_change_drops_every_cached_shape() {
        let context = |scale: f32, font_size: f32| ShapeContext {
            family: "monospace".into(),
            font_size: px(font_size),
            scale,
            default_fg: unpack(0xffffff),
            default_bg: unpack(0x000000),
            palette: None,
        };
        let row = || {
            Arc::new(vec![PositionedCell {
                column: 0,
                columns: 1,
                text: "a".into(),
                style: plain_style(SnapshotColor::Default, SnapshotColor::Default, false),
                selected: false,
                hovered_link: false,
            }])
        };
        let white = rgb(0xffffff);
        let mut cache = ShapeCache::default();
        let cells = row();
        cache.begin_frame(context(2.0, 14.0), 1);
        assert!(shapes_anew(&mut cache, &cells, white));
        cache.begin_frame(context(2.0, 14.0), 1);
        assert!(!shapes_anew(&mut cache, &cells, white), "an unchanged frame reuses the shape");
        assert!(shapes_anew(&mut cache, &cells, rgb(0xff0000)), "a new drawn colour reshapes");
        assert!(
            !shapes_anew(&mut cache, &cells, white),
            "the earlier colour is still pooled"
        );
        let rebuilt = row();
        assert!(
            !shapes_anew(&mut cache, &rebuilt, white),
            "a rebuilt row with the same text finds its shape in the pool"
        );
        cache.begin_frame(context(1.0, 14.0), 1);
        assert!(shapes_anew(&mut cache, &rebuilt, white), "a scale change reshapes");
        cache.begin_frame(context(1.0, 16.0), 1);
        assert!(shapes_anew(&mut cache, &rebuilt, white), "a font size change reshapes");
    }
```

- [ ] **Step 2: Run them and confirm they fail**

```sh
TERM=dumb cargo test -p sprite-app --lib --locked --offline grid_paint::tests::shaping_happens_only_for_cells_whose_drawn_text_changed -- --exact
```

Expected: compile errors — `cannot find type ShapeCache in this scope`, `cannot find type ShapeContext`, `cannot find value SHAPED_CELLS`, `cannot find function reaches_text_system`, `struct GridPaintSpec has no field named shapes`.

- [ ] **Step 3: Implement**

`crates/sprite-app/src/grid_paint.rs` imports — replace lines 52-59:

```rust
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    App, Bounds, ContentMask, Element, ElementId, Font, FontFeatures, FontStyle, FontWeight,
    GlobalElementId, InspectorElementId, IntoElement, LayoutId, Pixels, Position, Rgba,
    SharedString, ShapedLine, StrikethroughStyle, Style, TextRun, Window, fill, outline, point,
    px, relative, rgb,
};
```

Insert after `pub(crate) const CURSOR_STROKE: f32 = 0.12;` (line 73):

```rust
#[cfg(test)]
thread_local! {
    /// How many cells this thread has asked the text system to shape. Counted
    /// beside the one grid `shape_line` call, so a test can see what a frame
    /// actually cost.
    pub(crate) static SHAPED_CELLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The most distinct shapes a pane keeps for reuse.
///
/// A shaped line carries room for thirty-two decoration runs inline, a few
/// kilobytes each, so one shape is shared by every cell that would shape
/// identically and the pool of them is bounded. A screen of ordinary text
/// needs a few hundred; output that gives every cell its own truecolour simply
/// starts the pool again when it fills.
const MAX_DISTINCT_SHAPES: usize = 4096;

/// What every cached shape depends on besides the cell itself.
///
/// A change to any of it changes how every glyph is shaped or coloured, so the
/// whole cache goes rather than being checked cell by cell.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ShapeContext {
    pub family: SharedString,
    pub font_size: Pixels,
    pub scale: f32,
    pub default_fg: Rgb,
    pub default_bg: Rgb,
    pub palette: Option<Arc<[Rgb; 256]>>,
}

/// Everything that makes two cells shape to the same line.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct ShapeKey {
    text: SharedString,
    bold: bool,
    italic: bool,
    /// A hovered link is drawn a pixel larger.
    enlarged: bool,
    /// The drawn colour, bit for bit: the text system bakes it into the line.
    color: [u32; 4],
}

fn color_bits(color: Rgba) -> [u32; 4] {
    [
        color.r.to_bits(),
        color.g.to_bits(),
        color.b.to_bits(),
        color.a.to_bits(),
    ]
}

/// One cell's slot: the colour it was shaped in, and the shape.
type ShapedSlot = Option<(Rgba, Arc<ShapedLine>)>;

/// The shapes of one laid-out row.
#[derive(Default)]
struct ShapedRow {
    /// The row these shapes belong to. The layout cache hands back the same
    /// allocation while a row is unchanged, so a different one means the row
    /// was rebuilt and its shapes start again.
    source: Option<Arc<Vec<PositionedCell>>>,
    cells: Vec<ShapedSlot>,
}

/// Shaped glyphs kept between frames, filled lazily as cells are painted.
///
/// Sits beside the layout cache: rows the layout reuses keep their shapes, and
/// a cell is shaped again only when the colour it is drawn in differs from the
/// one it was shaped in. Shapes themselves are pooled, so a rebuilt row whose
/// cells look as they did before finds them without asking the text system.
#[derive(Default)]
pub(crate) struct ShapeCache {
    context: Option<ShapeContext>,
    rows: Vec<ShapedRow>,
    shapes: HashMap<ShapeKey, Arc<ShapedLine>>,
}

impl ShapeCache {
    /// Starts a frame of `rows` rows drawn under `context`, dropping every
    /// shape if the font, theme or scale changed since the last.
    pub(crate) fn begin_frame(&mut self, context: ShapeContext, rows: usize) {
        if self.context.as_ref() != Some(&context) {
            self.context = Some(context);
            self.rows.clear();
            self.shapes.clear();
        }
        self.rows.truncate(rows);
    }

    /// The shape for cell `column` of row `row`, drawn in `color`, calling
    /// `shape` only when neither the row nor the pool already holds it.
    pub(crate) fn shaped(
        &mut self,
        row: usize,
        cells: &Arc<Vec<PositionedCell>>,
        column: usize,
        color: Rgba,
        shape: impl FnOnce() -> ShapedLine,
    ) -> Arc<ShapedLine> {
        if self.rows.len() <= row {
            self.rows.resize_with(row + 1, ShapedRow::default);
        }
        let slots = &mut self.rows[row];
        if !slots
            .source
            .as_ref()
            .is_some_and(|source| Arc::ptr_eq(source, cells))
        {
            slots.source = Some(Arc::clone(cells));
            slots.cells.clear();
            slots.cells.resize(cells.len(), None);
        }
        if let Some((drawn, line)) = &slots.cells[column]
            && *drawn == color
        {
            return Arc::clone(line);
        }
        let cell = &cells[column];
        let key = ShapeKey {
            text: cell.text.clone(),
            bold: cell.style.bold,
            italic: cell.style.italic,
            enlarged: cell.hovered_link,
            color: color_bits(color),
        };
        let line = match self.shapes.get(&key) {
            Some(line) => Arc::clone(line),
            None => {
                if self.shapes.len() >= MAX_DISTINCT_SHAPES {
                    self.shapes.clear();
                }
                let line = Arc::new(shape());
                self.shapes.insert(key, Arc::clone(&line));
                line
            }
        };
        slots.cells[column] = Some((color, Arc::clone(&line)));
        line
    }
}
```

Add the field to `GridPaint` (after `font_size: Pixels,` line 205) and to `GridPaintSpec` (after `pub font_size: Pixels,` line 224). Task 8's `focused` field stays where Task 8 put it.

```rust
    shapes: Rc<RefCell<ShapeCache>>,
```

```rust
    pub shapes: Rc<RefCell<ShapeCache>>,
```

`GridPaint::prepare` — add the last parameter and the spec field (keep any argument or field Task 8 added):

```rust
    pub(crate) fn prepare(
        snapshot: Option<&RenderSnapshot>,
        rows: crate::grid::PositionedRows,
        metrics: &crate::surface::render::GridMetrics,
        split: bool,
        shapes: &Rc<RefCell<ShapeCache>>,
    ) -> (Self, Option<Self>) {
```

and in its `GridPaintSpec { .. }` literal, after `font_size: metrics.cells.font_size(),`:

```rust
                shapes: Rc::clone(shapes),
```

`prepare_spec` background literal (lines 259-265) becomes:

```rust
            let background = Self::new(GridPaintSpec {
                rows: spec.rows.clone(),
                pass: RowPass::Background,
                palette: spec.palette.clone(),
                font_family: spec.font_family.clone(),
                shapes: Rc::clone(&spec.shapes),
                ..spec
            });
```

`GridPaint::new` — after `font_size: spec.font_size,` (line 311) add `shapes: spec.shapes,`. Then add this method to the same `impl GridPaint` block, after `new`:

```rust
    /// What the shapes this element paints depend on, at `scale`.
    fn shape_context(&self, scale: f32) -> ShapeContext {
        ShapeContext {
            family: self.font_family.clone(),
            font_size: self.font_size,
            scale,
            default_fg: self.default_fg,
            default_bg: self.default_bg,
            palette: self.palette.clone(),
        }
    }
```

Insert the target struct after `struct CellBounds { .. }` (after line 474):

```rust
/// Which laid-out cell a glyph is, and the cache its shape is kept in.
///
/// One argument rather than four: the cache keys a shape by the row's
/// allocation and the cell's place in it, and those travel together.
struct GlyphTarget<'a> {
    row: usize,
    cells: &'a Arc<Vec<PositionedCell>>,
    column: usize,
    shapes: &'a mut ShapeCache,
}
```

`paint` — replace lines 546-548:

```rust
        let rows = Arc::clone(&self.rows);
        let shapes = Rc::clone(&self.shapes);
        let mut shapes = shapes.borrow_mut();
        if self.pass != RowPass::Background {
            shapes.begin_frame(self.shape_context(scale), rows.len());
        }
        // Resolving colors twice avoids allocating scratch storage for every ephemeral element.
        for (index, cells) in rows.iter().enumerate() {
```

and replace the glyph loop (lines 599-610; keep the `paint_cursor` call exactly as Task 8 left it):

```rust
            for (column, (cell, drawn)) in cells.iter().zip(resolved.clone()).enumerate() {
                let span = cell.span();
                let bounds = CellBounds {
                    left: edge(Col(span.start)),
                    right: edge(Col(span.end)),
                    top,
                    bottom,
                };
                let target = GlyphTarget {
                    row: index,
                    cells,
                    column,
                    shapes: &mut *shapes,
                };
                self.paint_glyph(target, &drawn, bounds, scale, window, cx);
                self.paint_decorations(cell, &drawn, bounds, window);
                self.paint_cursor(&drawn, bounds, scale, window);
            }
```

After `blank_glyph` (line 617) add:

```rust
/// Whether a cell's text goes to the text system, rather than being skipped
/// as blank or drawn as block or box geometry. `paint_glyph` makes the same
/// decision in the same order.
pub(crate) fn reaches_text_system(text: &str) -> bool {
    if blank_glyph(text) {
        return false;
    }
    let mut chars = text.chars();
    match (chars.next(), chars.next()) {
        (Some(ch), None) => block_fill(ch).is_none() && box_glyph(ch).is_none(),
        _ => true,
    }
}
```

Replace `paint_glyph` (lines 620-703) in full:

```rust
    /// Draws one cell's text on its own pixel, clipped to its own column.
    fn paint_glyph(
        &self,
        target: GlyphTarget<'_>,
        drawn: &Drawn,
        bounds: CellBounds,
        scale: f32,
        window: &mut Window,
        cx: &mut App,
    ) {
        let cell = &target.cells[target.column];
        // A cell holding nothing but blanks has no ink, and shaping one costs
        // the same as shaping a letter. Most of a terminal is blank.
        if blank_glyph(&cell.text) {
            return;
        }

        // A block element is drawn as geometry against the cell's own snapped
        // edges, never shaped: a glyph's ink is as wide as the font's advance,
        // which is not the snapped cell width, so a run of shaped blocks is
        // beaded with seams. See `block_elements`.
        if self.paint_block(cell, drawn, bounds, scale, window) {
            return;
        }

        // Box drawing is geometry for the same reason, and additionally has to
        // be drawn on whole device pixels to stay one pixel thick. See
        // `box_drawing`.
        if self.paint_box(cell, drawn, bounds, scale, window) {
            return;
        }
        debug_assert!(reaches_text_system(&cell.text));

        // Shaped once for the colour it is drawn in and kept with its row: a
        // frame that changes nothing about this cell reuses the shape, so an
        // idle or blinking pane asks the text system for nothing.
        let line = target.shapes.shaped(
            target.row,
            target.cells,
            target.column,
            drawn.foreground,
            || {
                #[cfg(test)]
                SHAPED_CELLS.with(|count| count.set(count.get() + 1));
                let run = TextRun {
                    len: cell.text.len(),
                    font: terminal_font(&self.font_family, cell.style.bold, cell.style.italic),
                    color: drawn.foreground.into(),
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                };
                let font_size = if cell.hovered_link {
                    self.font_size + px(1.0)
                } else {
                    self.font_size
                };
                window
                    .text_system()
                    .shape_line(cell.text.clone(), font_size, &[run], None)
            },
        );

        // The origin is the cell's snapped corner, the same one its background
        // and its neighbours use. The text system rasterises a glyph at one of
        // four subpixel offsets, so a column whose position lands on a
        // different fraction in each of five cells gets a differently
        // rasterised glyph in each — and a rule or a block, which is meant to
        // meet the one beside it, joins imperfectly wherever the two chosen
        // offsets disagree. Every cell starting on a whole device pixel gets
        // the same rasterisation of the same character, and a run of them is
        // continuous. The cell *width* is still the font's own 8.4, so the
        // columns do not drift: only where each one starts is rounded, by less
        // than half a device pixel.
        let origin = point(bounds.left.pixels(), bounds.top.pixels());

        // Every cell is clipped to its own column, not only the ones holding a
        // glyph too wide for it. A character that fills its cell — a rule, a
        // block — carries ink a little past its own advance so that a run of
        // them joins up; two neighbours both painting that overlap composite to
        // something brighter than either, and a bead appears at every join.
        //
        // The bounds are the snapped ones, so the mask follows the glyph rather
        // than cutting across it, and two neighbouring masks divide the pixels
        // between them exactly.
        let mask = ContentMask {
            bounds: Bounds::from_corners(
                point(bounds.left.pixels(), bounds.top.pixels()),
                point(
                    bounds.right.pixels(),
                    px(f32::from(bounds.top.pixels()) + f32::from(self.cell_height)),
                ),
            ),
        };
        window.with_content_mask(Some(mask), |window| {
            let _ = line.paint(origin, self.cell_height, window, cx);
        });
    }
```

Existing test `drawing_prepares_decorations_for_whitespace_without_glyph_ink` (line 1315): add `shapes: Default::default(),` after `font_size: px(14.0),` in its `GridPaintSpec` literal.

`crates/sprite-app/src/terminal_view.rs` — after `layout_cache: crate::grid::LayoutCache,` (line 103):

```rust
    /// Shaped glyphs kept between frames, beside the layout they belong to.
    shape_cache: std::rc::Rc<std::cell::RefCell<crate::grid_paint::ShapeCache>>,
```

In both `new` (line 395) and `failed` (line 486), after `layout_cache: Default::default(),` add:

```rust
            shape_cache: Default::default(),
```

`crates/sprite-app/src/terminal_view/render.rs:315`:

```rust
        let (background_grid, text_grid) =
            GridPaint::prepare(snapshot, rows, &metrics, split, &self.shape_cache);
```

(If Task 8 added an argument to `prepare`, keep it; `&self.shape_cache` is the last argument.)

`crates/sprite-app/src/paint_benchmark.rs` — struct and constructor (lines 40-63):

```rust
pub struct PaintBenchmark {
    snapshot: RenderSnapshot,
    changed: RenderSnapshot,
    cache: crate::grid::LayoutCache,
    shapes: std::rc::Rc<std::cell::RefCell<crate::grid_paint::ShapeCache>>,
}
```

```rust
        Self {
            snapshot,
            changed,
            cache: Default::default(),
            shapes: Default::default(),
        }
```

and in `prepare`'s `GridPaintSpec` literal after `font_size: px(14.0),`:

```rust
                shapes: std::rc::Rc::clone(&self.shapes),
```

`crates/sprite-app/src/surface/render.rs` — add to the imports after `use std::sync::{Arc, LazyLock};` (line 6):

```rust
use std::cell::RefCell;
use std::rc::Rc;
```

and change line 15 to `use crate::grid_paint::{GridPaint, GridPaintSpec, RowPass, ShapeCache, pack};`. `render_grid` signature (line 212):

```rust
pub(crate) fn render_grid(
    grid: &mut GridSurface,
    highlights: &Highlights,
    metrics: &GridMetrics,
    shapes: &Rc<RefCell<ShapeCache>>,
) -> AnyElement {
```

and in its `GridPaintSpec` literal after `font_size: metrics.cells.font_size(),`:

```rust
        shapes: Rc::clone(shapes),
```

Its test at line 677:

```rust
        let _element = render_grid(
            &mut grid,
            &crate::config::Highlights::default(),
            &metrics,
            &Rc::default(),
        );
```

`crates/sprite-app/src/terminal_view/surfaces.rs` — `Body::Grid` (lines 37-45):

```rust
    Grid {
        grid: Box<GridSurface>,
        /// The description's root element, kept for the `bg` and `color` it
        /// may carry: a grid's wrapper takes its colours from them, the way an
        /// element root's box does. A grid root refuses `style` and `border`,
        /// so those never arrive here.
        root: Element,
        /// The grid's shaped glyphs, kept between frames as the terminal's are.
        shapes: std::rc::Rc<std::cell::RefCell<crate::grid_paint::ShapeCache>>,
    },
```

construction (lines 59-62):

```rust
            Element::Grid { size, .. } => Body::Grid {
                grid: Box::new(GridSurface::new(size.cols, size.rows)),
                root,
                shapes: Default::default(),
            },
```

call (lines 1130-1132):

```rust
            Body::Grid { grid, shapes, .. } => {
                crate::surface::render::render_grid(grid, highlights, metrics, shapes)
            }
```

`crates/sprite-app/src/surface_performance.rs` — create the cache outside the measured closures. Before line 103 insert `let shapes = std::rc::Rc::default();` and change the three calls to pass `&shapes` as the fourth argument:

```rust
    let shapes = std::rc::Rc::default();
    let (_, a, b) = measure(|| render::render_grid(&mut grid, &Highlights::default(), &metrics, &shapes));
    println!("grid first render allocations={a} bytes={b}");
    let (_, a, b) = measure(|| render::render_grid(&mut grid, &Highlights::default(), &metrics, &shapes));
```

```rust
        render::render_grid(&mut grid, &Highlights::default(), &metrics, &shapes)
```

- [ ] **Step 4: Run tests, confirm pass**

```sh
TERM=dumb cargo test -p sprite-app --lib --locked --offline grid_paint::
TERM=dumb cargo test -p sprite-app --lib --locked --offline grid::
TERM=dumb cargo test -p sprite-app --lib --locked --offline paint_benchmark::
TERM=dumb cargo test -p sprite-app --lib --locked --offline surface::
TERM=dumb cargo test -p sprite-app --lib --locked --offline surface_allocation_probe
TERM=dumb cargo test -p sprite-app --locked --offline --test paint_benchmark_report
TERM=dumb cargo test -p sprite-app --lib --locked --offline terminal_view::
```

- [ ] **Step 5: Commit**

```sh
git add crates/sprite-app/src/grid_paint.rs crates/sprite-app/src/terminal_view.rs crates/sprite-app/src/terminal_view/render.rs crates/sprite-app/src/terminal_view/surfaces.rs crates/sprite-app/src/paint_benchmark.rs crates/sprite-app/src/surface/render.rs crates/sprite-app/src/surface_performance.rs
git commit -m "perf(paint): shape a cell only when its drawn text changes

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

#### Cycle C — box drawing without per-cell allocation

- [ ] **Step 1: Write the failing test** — append to `mod tests` in `crates/sprite-app/src/box_drawing.rs` (before its closing `}`), reusing that module's `cell()` (line 670) and `strokes()` (line 679) helpers and `crate::surface_performance::measure`.

```rust
    /// Drawing a box character asks the allocator for nothing: a character's
    /// strokes and outlines fit in fixed arrays, so a screen of box drawing is
    /// not thousands of small allocations a frame.
    #[test]
    fn drawing_box_characters_allocates_nothing() {
        let (c, s) = (cell(), strokes());
        let glyphs: Vec<BoxGlyph> = (0x2500u32..=0x257F)
            .filter_map(char::from_u32)
            .filter_map(box_glyph)
            .collect();
        assert!(glyphs.len() > 100, "the whole Box Drawing block is covered");
        let mut emitted = 0usize;
        let ((), allocations, bytes) = crate::surface_performance::measure(|| {
            for glyph in &glyphs {
                box_rects(glyph, c, s, |_| emitted += 1);
                box_outlines(glyph, c, s, |outline| emitted += outline.steps.len());
            }
        });
        assert!(emitted > glyphs.len(), "every character drew something");
        assert_eq!((allocations, bytes), (0, 0));
    }
```

- [ ] **Step 2: Run it and confirm it fails**

```sh
TERM=dumb cargo test -p sprite-app --lib --locked --offline box_drawing::tests::drawing_box_characters_allocates_nothing -- --exact
```

Expected: assertion failure `left: (N, M) right: (0, 0)` with N in the hundreds (one `Vec` per stroke axis per character and one per arc or diagonal outline).

- [ ] **Step 3: Implement** — in `crates/sprite-app/src/box_drawing.rs`:

`Outline` (lines 65-70):

```rust
/// A closed shape to fill, for the characters that are not rectangles.
///
/// Every such shape here is four steps — an arc's two curves and two joins, a
/// diagonal's four edges — so they are held inline rather than allocated.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Outline {
    pub start: Point,
    pub steps: [Step; 4],
}
```

In `box_outlines` change `steps: vec![` to `steps: [` (line 530) and in `diagonal` change `steps: vec![` to `steps: [` (line 572); the four elements and closing `]` stay as they are.

Replace `stroke_lines` (lines 478-501):

```rust
/// The strokes of one axis, held inline: none, one, or a double pair.
///
/// No axis of a box character has more than two strokes, so a fixed pair with
/// a length covers every case without an allocation per cell.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct StrokeLines {
    spans: [(f32, f32); 2],
    len: usize,
}

impl StrokeLines {
    fn as_slice(&self) -> &[(f32, f32)] {
        &self.spans[..self.len]
    }
}

/// The span each stroke of one axis occupies, centred on `centre`.
///
/// One stroke for a light or heavy arm, two for a double, and none where the
/// axis has no arm at all.
fn stroke_lines(centre: f32, weight: Option<Weight>, strokes: Strokes) -> StrokeLines {
    match weight {
        None => StrokeLines::default(),
        Some(Weight::Double) => {
            let half = strokes.light / 2.0;
            let offset = strokes.light;
            StrokeLines {
                spans: [
                    (centre - offset - half, centre - offset + half),
                    (centre + offset - half, centre + offset + half),
                ],
                len: 2,
            }
        }
        Some(other) => {
            let half = match other {
                Weight::Heavy => strokes.heavy,
                _ => strokes.light,
            } / 2.0;
            StrokeLines {
                spans: [(centre - half, centre + half), (0.0, 0.0)],
                len: 1,
            }
        }
    }
}
```

In `box_rects` replace lines 368-369:

```rust
    let h_strokes = stroke_lines(mid_y, horizontal_weight, strokes);
    let v_strokes = stroke_lines(mid_x, vertical_weight, strokes);
    let h_lines = h_strokes.as_slice();
    let v_lines = v_strokes.as_slice();
```

and, because `h_lines`/`v_lines` are now already slices (clippy's `needless_borrow` would reject `&v_lines`), change the four far-edge calls:

```rust
                emit((cell.left, top, v_lines_far(v_lines, true), bottom));
```
```rust
                emit((v_lines_far(v_lines, false), top, cell.right, bottom));
```
```rust
                emit((left, cell.top, right, h_lines_far(h_lines, true)));
```
```rust
                emit((left, h_lines_far(h_lines, false), right, cell.bottom));
```

- [ ] **Step 4: Run tests, confirm pass**

```sh
TERM=dumb cargo test -p sprite-app --lib --locked --offline box_drawing::
TERM=dumb cargo test -p sprite-app --lib --locked --offline grid_paint::
```

- [ ] **Step 5: Commit**

```sh
git add crates/sprite-app/src/box_drawing.rs
git commit -m "perf(box-drawing): hold strokes and outlines in fixed arrays

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 17: Hover link requested and repainted only on change (BCA-14)

**Files:**
- Modify: `crates/sprite-app/src/terminal_view.rs:104-105` (fields), `:147-152` (test counters), `:155-171` (`request_hover_link`), `:303-315` (event task), `:334-344` (snapshot task), `:396` and `:487` (constructors), `:509-564` (`apply`)
- Test: `crates/sprite-app/src/terminal_view/tests.rs` (module `terminal_view::tests`)

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces:
  - `fn TerminalView::apply(&mut self, effect: Effect, cx: &mut Context<Self>) -> bool` — returns whether the effect changed anything drawn; the event task notifies once per batch only if some effect returned `true`.
  - `TerminalView.hover_basis: Option<(CellPosition, Arc<RenderRow>)>`
  - `fn TerminalView::hover_basis_holds(&self, position: CellPosition) -> bool`, `fn TerminalView::follow_hover(&mut self)`
  - `#[cfg(test)] pub(crate) static HOVER_LINK_REQUESTS: Cell<usize>` (thread-local in `terminal_view.rs`)

- [ ] **Step 1: Write the failing test** — append to `crates/sprite-app/src/terminal_view/tests.rs` (after `buttonless_reporting_preserves_hyperlink_hover`, line 830). It copies that test's conventions: a real `/bin/sh` session in `add_window_view`, `wait_for_bundle` (line 5), `simulate_mouse_move`, `view.condition`, `catch_unwind` with `begin_shutdown` cleanup.

```rust
/// Waits for the outstanding hover answer by polling, not by condition: an
/// answer that changes nothing no longer notifies, and a notification-driven
/// condition would never wake for it.
fn settle_hover(view: &gpui::Entity<TerminalView>, cx: &mut gpui::VisualTestContext) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        cx.run_until_parked();
        if view.read_with(cx, |view, _| view.hover_request.is_none()) {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the hover answer did not arrive"
        );
        std::thread::yield_now();
    }
}

#[gpui::test]
fn hover_link_requests_and_repaints_follow_only_real_changes(cx: &mut gpui::TestAppContext) {
    let settings = crate::config::Settings::default();
    cx.set_global(crate::config::ActiveSettings(settings.clone()));
    let (sender, _exits) = async_channel::unbounded();
    let script = "stty -echo; printf '\\033]8;;https://example.com\\007LINK\\033]8;;\\007\\r\\nREADY\\r\\n'; IFS= read -r go; i=0; while [ $i -lt 5 ]; do printf '\\033[3;1Hrow%s' $i; sleep 0.05; i=$((i+1)); done; printf '\\033[4;1HQUIET'; IFS= read -r go; printf '\\033[1;10HCHANGED'; sleep 30";
    let (view, cx) = cx.add_window_view(|window, cx| {
        TerminalView::new(
            Some(vec!["/bin/sh".into(), "-c".into(), script.into()]),
            settings,
            Vec::new(),
            None,
            PaneExit {
                sender,
                identity: (crate::tabs::TabId(1), crate::pane_tree::PaneId(1)),
            },
            window,
            cx,
        )
    });
    let requests = || HOVER_LINK_REQUESTS.with(|count| count.get());
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        wait_for_bundle(&view, cx, |bundle| {
            bundle.pane.rows.iter().any(|row| row.text.contains("READY"))
        });
        cx.update(|window, cx| {
            window.activate_window();
            window.refresh();
            window.draw(cx).clear();
        });
        // The first frame fits the grid to the window, and that resize
        // redraws every row; it has to land before the hover is taken.
        let sized = view.read_with(cx, |view, _| view.size);
        wait_for_bundle(&view, cx, |bundle| {
            sized.is_none_or(|size| bundle.render.size == size)
        });
        let (on_link, plain, beside) = view.read_with(cx, |view, _| {
            let origin = view.content_origin.unwrap_or(view.origin);
            let width = view.metrics.width();
            let height = view.metrics.height();
            (
                gpui::point(origin.x + width * 0.5, origin.y + height * 0.5),
                gpui::point(origin.x + width * 30.5, origin.y + height * 10.5),
                gpui::point(origin.x + width * 31.5, origin.y + height * 10.5),
            )
        });
        let executor = cx.executor();
        executor.allow_parking();
        cx.simulate_mouse_move(on_link, None, gpui::Modifiers::default());
        executor.block_test(view.condition::<()>(cx, |view, _| {
            view.hovered_link.is_some() && view.hover_request.is_none()
        }));
        let before = requests();
        let generation = view.read_with(cx, |view, _| view.bundle.as_ref().unwrap().generation);

        // Output on other rows: the snapshots arrive, the link under the
        // pointer is not asked about again, and it stays drawn.
        view.update(cx, |view, _| {
            view.send(TerminalCommand::Input(b"GO\n".to_vec()))
        });
        let quiet = wait_for_bundle(&view, cx, |bundle| {
            bundle.pane.rows.iter().any(|row| row.text.contains("QUIET"))
        });
        assert!(quiet.generation > generation, "output elsewhere produced snapshots");
        assert_eq!(
            requests(),
            before,
            "snapshots that leave the hovered row alone must not re-request its link"
        );
        view.read_with(cx, |view, _| {
            let (stamped, _) = view.hovered_link.expect("the link stays hovered");
            assert_eq!(
                stamped,
                view.bundle.as_ref().unwrap().generation,
                "the kept answer is restamped so the painter still draws it"
            );
        });

        // Output on the hovered row: its link is asked about again.
        view.update(cx, |view, _| {
            view.send(TerminalCommand::Input(b"GO\n".to_vec()))
        });
        wait_for_bundle(&view, cx, |bundle| bundle.pane.rows[0].text.contains("CHANGED"));
        executor.block_test(view.condition::<()>(cx, |view, _| {
            view.hover_request.is_none() && view.hovered_link.is_some()
        }));
        assert!(requests() > before, "a change to the hovered row re-requests its link");

        // Two plain cells in turn: the second answer agrees with the first,
        // so it must not repaint.
        cx.simulate_mouse_move(plain, None, gpui::Modifiers::default());
        settle_hover(&view, cx);
        let notified = std::rc::Rc::new(std::cell::Cell::new(0usize));
        let counter = notified.clone();
        let _observer = cx.update(|_, cx| {
            cx.observe(&view, move |_, _| counter.set(counter.get() + 1))
        });
        let asked = requests();
        cx.simulate_mouse_move(beside, None, gpui::Modifiers::default());
        assert_eq!(requests(), asked + 1, "moving to another cell asks about it");
        settle_hover(&view, cx);
        assert_eq!(
            notified.get(),
            0,
            "an answer that changes nothing must not repaint"
        );
    }));
    if let Some(cleanup) = view.update(cx, |view, _| view.begin_shutdown()) {
        cleanup.wait().unwrap();
    }
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}
```

- [ ] **Step 2: Run it and confirm it fails**

```sh
TERM=dumb cargo test -p sprite-app --lib --locked --offline terminal_view::tests::hover_link_requests_and_repaints_follow_only_real_changes -- --exact
```

Expected: compile error `cannot find value HOVER_LINK_REQUESTS in this scope`. (With only the counter added, the run fails at `snapshots that leave the hovered row alone must not re-request its link`, because lines 338-342 re-request on every snapshot.)

- [ ] **Step 3: Implement** — `crates/sprite-app/src/terminal_view.rs`:

Test counter — add inside the `#[cfg(test)] thread_local!` block (after line 151):

```rust
    pub(crate) static HOVER_LINK_REQUESTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
```

Field — after `hover_request: Option<(u64, sprite_term::CellPosition)>,` (line 105):

```rust
    /// The cell the current hover answer was asked about, and that cell's row
    /// as it was then. Rows are shared between snapshots while their content
    /// is unchanged, so the same allocation means the answer still holds.
    hover_basis: Option<(sprite_term::CellPosition, Arc<sprite_term::RenderRow>)>,
```

In both constructors, after `hover_request: None,` (lines 396 and 487):

```rust
            hover_basis: None,
```

Replace `request_hover_link` (lines 155-171) and add the two helpers after it:

```rust
    fn request_hover_link(&mut self, position: sprite_term::CellPosition) {
        self.hovered_link = None;
        if !matches!(self.session, SessionState::Running(_)) {
            return;
        }
        if self.hover_request.is_none() {
            let request_id = self.next_link_request;
            self.next_link_request = self.next_link_request.wrapping_add(1);
            self.hover_request = Some((request_id, position));
            self.hover_basis = self
                .bundle
                .as_ref()
                .and_then(|bundle| bundle.render.rows.get(usize::from(position.row)))
                .map(|row| (position, Arc::clone(row)));
            #[cfg(test)]
            HOVER_LINK_REQUESTS.with(|count| count.set(count.get() + 1));
            if !self.submit(TerminalCommand::ResolveHyperlink {
                position,
                request_id,
            }) {
                self.hover_request = None;
                self.hover_basis = None;
            }
        }
    }

    /// Whether the newest snapshot still shows `position` exactly as it was
    /// when its link was last asked about.
    fn hover_basis_holds(&self, position: sprite_term::CellPosition) -> bool {
        let Some((asked, row)) = &self.hover_basis else {
            return false;
        };
        *asked == position
            && self
                .bundle
                .as_ref()
                .and_then(|bundle| bundle.render.rows.get(usize::from(position.row)))
                .is_some_and(|current| Arc::ptr_eq(row, current))
    }

    /// Carries the hover across a new snapshot.
    ///
    /// Output elsewhere on screen does not change what is under the pointer,
    /// so the answer already held is kept — restamped with the new generation,
    /// which is what the painter checks — rather than asked for again. Only a
    /// change to the hovered row itself asks again.
    fn follow_hover(&mut self) {
        let Some(cell) = self.hovered_cell else {
            return;
        };
        // An answer still in flight re-checks the row when it arrives.
        if self.hover_request.is_some() {
            return;
        }
        if !self.hover_basis_holds(cell) {
            self.request_hover_link(cell);
            return;
        }
        if let (Some(bundle), Some((_, span))) = (self.bundle.as_ref(), self.hovered_link) {
            self.hovered_link = Some((bundle.generation, span));
        }
    }
```

Event task (lines 303-315):

```rust
                if !decision.effects.is_empty() {
                    let applied = view.update(cx, |view, cx| {
                        let mut repaint = false;
                        for effect in decision.effects {
                            repaint |= view.apply(effect, cx);
                        }
                        // One notify for the batch, and none for a batch that
                        // changed nothing drawn: a hover answer agreeing with
                        // the last one repaints nothing.
                        if repaint {
                            cx.notify();
                        }
                    });
                    if applied.is_err() {
                        return;
                    }
                }
```

Snapshot task — replace lines 337-343:

```rust
                            view.refresh_display_title(cx);
                            view.follow_hover();
                            cx.notify();
```

`apply` — replace lines 507-564 in full (if Task 1 has already rewritten the `DeliverHistory` / `FailRequest` arms, keep Task 1's bodies and end each with `true`):

```rust
    /// Performs one decided effect, returning whether it changed anything the
    /// pane draws. Everything here needs `cx`; nothing here decides anything.
    fn apply(&mut self, effect: crate::terminal_events::Effect, cx: &mut Context<Self>) -> bool {
        use crate::terminal_events::Effect;
        match effect {
            Effect::Status(line) => {
                self.status = Some(line);
                true
            }
            Effect::Title(title) => {
                self.title = title.map(SharedString::from);
                self.refresh_display_title(cx);
                true
            }
            Effect::HoldPaste(text) => {
                self.pending_unsafe_paste = Some(text);
                true
            }
            Effect::HyperlinkResolved {
                position,
                request_id,
                generation,
                uri,
                span,
            } => {
                if self.pending_link_click == Some(request_id) {
                    self.pending_link_click = None;
                    if let Some(uri) = uri {
                        cx.open_url(&uri);
                    }
                }
                if self.hover_request != Some((request_id, position)) {
                    return false;
                }
                self.hover_request = None;
                let before = self.hovered_link.map(|(_, span)| span);
                if self.hovered_cell == Some(position) {
                    // The answer describes the row as it was asked about. While
                    // that row is unchanged it still holds for the newest
                    // snapshot, whatever generation the worker stamped it with.
                    match self.bundle.as_ref().map(|bundle| bundle.generation) {
                        Some(current)
                            if current == generation || self.hover_basis_holds(position) =>
                        {
                            self.hovered_link = span.map(|span| (current, span));
                        }
                        _ => self.request_hover_link(position),
                    }
                }
                if let Some(cell) = self.hovered_cell.filter(|cell| *cell != position) {
                    self.request_hover_link(cell);
                }
                self.hovered_link.map(|(_, span)| span) != before
            }
            Effect::Clipboard(text) => {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
                true
            }
            Effect::DeliverHistory(history) => {
                if let Some(link) = &self.observation {
                    link.panes.deliver(link.pane, history);
                }
                true
            }
            // A pane in a bad state must not leave an observation request
            // waiting out the deadline: the pane cannot answer, and this is why.
            Effect::FailRequest(reason) => {
                if let Some(link) = &self.observation {
                    link.panes.deliver_failure(link.pane, reason);
                }
                true
            }
        }
    }
```

(Every arm other than `HyperlinkResolved` returns `true`, which keeps today's one-notify-per-batch behaviour for them; only the hover answer gains the "unchanged → no repaint" rule. Existing callers that use `view.apply(..., cx);` as a statement are unaffected.)

- [ ] **Step 4: Run tests, confirm pass**

```sh
TERM=dumb cargo test -p sprite-app --lib --locked --offline terminal_view::tests::hover_link_requests_and_repaints_follow_only_real_changes -- --exact
TERM=dumb cargo test -p sprite-app --lib --locked --offline terminal_view::
TERM=dumb cargo test -p sprite-app --lib --locked --offline terminal_events::
```

`terminal_view::tests::buttonless_reporting_preserves_hyperlink_hover` and `terminal_view::submission_regressions::rejected_link_requests_recover_after_event_pressure` must still pass.

- [ ] **Step 5: Commit**

```sh
git add crates/sprite-app/src/terminal_view.rs crates/sprite-app/src/terminal_view/tests.rs
git commit -m "fix(hover): ask about a link only when the hovered row changes

A snapshot that leaves the hovered row's content alone keeps the
answer already held, and an answer that matches it no longer repaints.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 18: Render fidelity — faint, wide-tail cursor, hidden selected text, placeholder tiles (BCA-12, BCA-13, BCA-28, BCA-30)

Four test-then-implement cycles.

**Files:**
- Modify: `crates/sprite-app/src/grid_paint.rs:114-130` (`cell_colors`), `:336-399` (`GridPaint::draw`)
- Modify: `crates/sprite-term/src/snapshot.rs:578-584` (`cursor_snapshot`)
- Modify: `crates/sprite-app/src/terminal_view/placeholder.rs:1-6` (imports), after `:355` (new `tile_geometry`)
- Modify: `crates/sprite-app/src/terminal_view/render.rs:82-118` (`placeholder_element`), `:153-208` (`image_layers`), `:308-309` (call)
- Test: `crates/sprite-app/src/grid_paint.rs` (`mod tests`), `crates/sprite-term/src/snapshot.rs` (`mod sharing_tests`), `crates/sprite-app/src/terminal_view/placeholder.rs` (`mod tests`)

**Interfaces:**
- Consumes: Task 8's `focused: bool` and Task 16's `shapes: Rc<RefCell<ShapeCache>>` on `GridPaintSpec` (test helper sets both).
- Produces:
  - `cell_colors` no longer applies `invisible`; `GridPaint::draw` applies `invisible` and `faint` after selection/cursor inversion.
  - `const FAINT_OPACITY: f32 = 0.5` (grid_paint.rs, private)
  - `pub(super) struct placeholder::TileGeometry { left, top, width, height, image_left, image_top: Pixels }` and `pub(super) fn placeholder::tile_geometry(cell: &ImageCell<'_>, fit: &ImageFit, cell_width: Pixels, cell_height: Pixels, scale: f32) -> TileGeometry`
  - `TerminalView::image_layers(&self, rows, cell_width, cell_height, scale: f32)` (new last parameter)

#### Cycle A — faint text (BCA-12) and selected hidden text (BCA-28)

- [ ] **Step 1: Write the failing tests** — in `crates/sprite-app/src/grid_paint.rs` `mod tests`. Add these helpers after `plain_style` (line 1110):

```rust
    /// A painter over no rows, with the defaults the colour tests use.
    fn painter(pass: RowPass) -> GridPaint {
        GridPaint::new(GridPaintSpec {
            rows: Arc::from([]),
            pass,
            cursor: None,
            cursor_color: None,
            default_fg: unpack(0xaabbcc),
            default_bg: unpack(0x112233),
            palette: None,
            cell_width: px(8.4),
            cell_height: px(16.8),
            font_family: ".SystemUIFont".into(),
            font_size: px(14.0),
            focused: true,
            shapes: Default::default(),
        })
    }

    fn positioned(style: CellStyle) -> PositionedCell {
        PositionedCell {
            column: 0,
            columns: 1,
            text: "x".into(),
            style,
            selected: false,
            hovered_link: false,
        }
    }
```

Replace the two existing tests `invisible_collapses_the_foreground_onto_the_background` (lines 1150-1189) and `inverse_and_invisible_together_collapse_onto_the_original_foreground` (lines 1261-1303) — `cell_colors` no longer knows about hidden text, so they assert through `draw`, where that rule now lives:

```rust
    /// An invisible cell must vanish into its ground, not just match itself:
    /// the glyph has to take on the ground's colour, so a bug that collapsed
    /// the pair the other way round would still leave text visible.
    #[test]
    fn invisible_collapses_the_foreground_onto_the_background() {
        let fg_color = unpack(0x102030);
        let bg_color = unpack(0x405060);
        let mut style = plain_style(
            SnapshotColor::Rgb(fg_color),
            SnapshotColor::Rgb(bg_color),
            false,
        );
        style.invisible = true;
        let drawn = painter(RowPass::Whole).draw(&positioned(style), None);
        assert_eq!(Some(drawn.foreground), drawn.background);
        assert_eq!(
            drawn.foreground,
            rgb(pack(bg_color)),
            "invisible should collapse toward the background, not the foreground"
        );
    }

    /// Inverse swaps first and hiding acts on the result, so a reversed hidden
    /// cell settles on its original foreground, which is the ground it shows.
    #[test]
    fn inverse_and_invisible_together_collapse_onto_the_original_foreground() {
        let fg_color = unpack(0x102030);
        let bg_color = unpack(0x405060);
        let mut style = plain_style(
            SnapshotColor::Rgb(fg_color),
            SnapshotColor::Rgb(bg_color),
            true,
        );
        style.invisible = true;
        let drawn = painter(RowPass::Whole).draw(&positioned(style), None);
        assert_eq!(Some(drawn.foreground), drawn.background);
        assert_eq!(
            drawn.foreground,
            rgb(pack(fg_color)),
            "reversed and invisible together should settle on the pre-swap foreground"
        );
    }
```

Add at the end of the module:

```rust
    /// Faint (SGR 2) is ink at half strength — Ghostty's default
    /// `faint-opacity` — and never a translucent ground, even where the
    /// ground is the foreground colour, as under a selection.
    #[test]
    fn faint_text_draws_its_glyph_at_half_alpha_over_an_opaque_ground() {
        let mut style = plain_style(
            SnapshotColor::Rgb(unpack(0x102030)),
            SnapshotColor::Rgb(unpack(0x405060)),
            false,
        );
        style.faint = true;
        let mut cell = positioned(style);
        let drawn = painter(RowPass::Whole).draw(&cell, None);
        assert_eq!(drawn.foreground, Rgba { a: 0.5, ..rgb(0x102030) });
        assert_eq!(drawn.background, Some(rgb(0x405060)));

        cell.selected = true;
        let selected = painter(RowPass::Whole).draw(&cell, None);
        assert_eq!(selected.background, Some(rgb(0x102030)), "the selection ground stays opaque");
        assert_eq!(selected.foreground, Rgba { a: 0.5, ..rgb(0x405060) });
    }

    /// Selecting hidden text shows the selection over it while the glyphs
    /// stay hidden, in every pass that draws them.
    #[test]
    fn a_selected_hidden_cell_shows_the_selection_and_still_hides_its_glyph() {
        let mut style = plain_style(
            SnapshotColor::Rgb(unpack(0x102030)),
            SnapshotColor::Rgb(unpack(0x405060)),
            false,
        );
        style.invisible = true;
        let mut cell = positioned(style);
        let plain = painter(RowPass::Whole).draw(&cell, None);
        assert_eq!(plain.background, Some(rgb(0x405060)));
        assert_eq!(plain.foreground, rgb(0x405060), "hidden text is inked in its own ground");

        cell.selected = true;
        let selected = painter(RowPass::Whole).draw(&cell, None);
        assert_eq!(selected.background, Some(rgb(0x102030)), "the selection is shown");
        assert_eq!(
            selected.foreground,
            rgb(0x102030),
            "the glyph stays hidden in the selection's colour"
        );
        let text = painter(RowPass::Text).draw(&cell, None);
        assert_eq!(text.background, None);
        assert_eq!(text.foreground, rgb(0x102030), "the text pass hides it against the same ground");
    }
```

- [ ] **Step 2: Run them and confirm they fail**

```sh
TERM=dumb cargo test -p sprite-app --lib --locked --offline grid_paint::tests::faint_text_draws_its_glyph_at_half_alpha_over_an_opaque_ground -- --exact
TERM=dumb cargo test -p sprite-app --lib --locked --offline grid_paint::tests::a_selected_hidden_cell_shows_the_selection_and_still_hides_its_glyph -- --exact
```

Expected: the faint test fails `left: Rgba { r: .., a: 1.0 } right: Rgba { .., a: 0.5 }`; the hidden test fails at `the selection is shown` (`left: Some(<0x405060>) right: Some(<0x102030>)`). The two rewritten tests pass both before and after.

- [ ] **Step 3: Implement** — `crates/sprite-app/src/grid_paint.rs`:

Replace `cell_colors` (lines 114-130):

```rust
/// How strongly faint (SGR 2) text is inked: Ghostty's default
/// `faint-opacity`.
const FAINT_OPACITY: f32 = 0.5;

/// A cell's foreground and background, honouring reverse video.
///
/// Hidden and faint text are drawing decisions rather than colours: both
/// depend on whether the cell is selected or under the cursor, so `draw`
/// applies them once it knows.
pub(crate) fn cell_colors(
    style: &CellStyle,
    default_fg: Rgb,
    default_bg: Rgb,
    palette: Option<&[Rgb; 256]>,
) -> (Rgba, Rgba) {
    let mut foreground = resolve(style.foreground, default_fg, palette);
    let mut background = resolve(style.background, default_bg, palette);
    if style.inverse {
        std::mem::swap(&mut foreground, &mut background);
    }
    (foreground, background)
}
```

In `draw`, replace from `let fill = match self.pass {` through `let foreground = if inverted { background } else { foreground };` (lines 367-383) — the lines above it (including whatever Task 8 did to `here`, `is_block` and `cursor_paint`) stay:

```rust
        // The colour the cell's ground is, whether or not this pass paints it.
        let ground = match (is_block, inverted) {
            (true, _) => cursor_paint,
            (false, true) => foreground,
            (false, false) => background,
        };
        let fill = match self.pass {
            // The text half of a split draws no ground at all: the background
            // half already did, and an image may be sitting between them.
            RowPass::Text => None,
            // In a split pass a cell whose background is the terminal's default
            // is left unpainted, so an image behind it shows through. A cell
            // with a background of its own still covers the image, which is
            // what an explicit background means.
            RowPass::Background if !painted => None,
            _ => Some(ground),
        };

        let foreground = if cell.style.invisible {
            // Hidden text keeps the ground it is shown on — a selection
            // included, so selecting hidden text still shows the selection —
            // and inks its glyph in that same colour, so none of it shows.
            ground
        } else {
            let ink = if inverted { background } else { foreground };
            if cell.style.faint {
                // Faint dims the ink only. The ground stays opaque even where
                // it is the foreground colour, as it is under a selection.
                Rgba {
                    a: ink.a * FAINT_OPACITY,
                    ..ink
                }
            } else {
                ink
            }
        };
```

- [ ] **Step 4: Run tests, confirm pass**

```sh
TERM=dumb cargo test -p sprite-app --lib --locked --offline grid_paint::
TERM=dumb cargo test -p sprite-app --lib --locked --offline paint_benchmark::
```

- [ ] **Step 5: Commit**

```sh
git add crates/sprite-app/src/grid_paint.rs
git commit -m "fix(paint): draw faint text dimmed and show selection over hidden text

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

#### Cycle B — cursor on a wide character's tail (BCA-13)

- [ ] **Step 1: Write the failing test** — append to `mod sharing_tests` in `crates/sprite-term/src/snapshot.rs` (after `generated_cell_mutations_match_a_fresh_projector`, before the module's closing `}`), reusing its `fixture()` (4×12 terminal) and `capture` helpers.

```rust
    /// libghostty lets the cursor sit on the second column of a wide
    /// character. No drawable cell starts there, so reported as-is the cursor
    /// matched nothing and vanished; it belongs to the character itself, and
    /// the painter then draws it across both of that cell's columns.
    #[test]
    fn a_cursor_on_a_wide_tail_reports_the_lead_column() {
        let (mut projector, mut terminal, size) = fixture();
        terminal.vt_write("\x1b[3;1H界\x1b[3;2H".as_bytes());
        let bundle = capture(&mut projector, &terminal, size, false);
        assert_eq!(bundle.render.rows[2].cells[0].width, crate::CellWidth::Wide);
        assert_eq!(bundle.render.rows[2].cells[1].width, crate::CellWidth::SpacerTail);
        let cursor = bundle.render.cursor;
        assert!(cursor.visible);
        assert_eq!((cursor.row, cursor.column), (2, 0));
    }
```

- [ ] **Step 2: Run it and confirm it fails**

```sh
TERM=dumb cargo test -p sprite-term --lib --locked --offline snapshot::sharing_tests::a_cursor_on_a_wide_tail_reports_the_lead_column -- --exact
```

Expected: `assertion left == right failed: left: (2, 1) right: (2, 0)`.

- [ ] **Step 3: Implement** — `crates/sprite-term/src/snapshot.rs`, replace the `Some(position) => CursorSnapshot { .. }` arm (lines 579-585):

```rust
        Some(position) => CursorSnapshot {
            row: position.y,
            // On a wide character's second column the cursor belongs to the
            // character, as Ghostty's own renderer has it. The painter draws a
            // cursor over the cell whose column it names, and a wide cell
            // spans both of its columns, so naming the lead column draws the
            // cursor two cells wide.
            column: if position.at_wide_tail {
                position.x.saturating_sub(1)
            } else {
                position.x
            },
            visible,
            blinking,
            style,
        },
```

- [ ] **Step 4: Run tests, confirm pass**

```sh
TERM=dumb cargo test -p sprite-term --lib --locked --offline snapshot::
```

- [ ] **Step 5: Commit**

```sh
git add crates/sprite-term/src/snapshot.rs
git commit -m "fix(snapshot): put a cursor on a wide tail at its lead column

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

#### Cycle C — placeholder tiles on the cell grid (BCA-30)

- [ ] **Step 1: Write the failing test** — append to `mod tests` in `crates/sprite-app/src/terminal_view/placeholder.rs` (before its closing `}` at line 520), reusing `placement(image, placement)` (line 427) and `fit_image`.

```rust
    /// Placeholder tiles are laid out by taffy, which rounds in device pixels.
    /// A tile whose edges and size are already whole device pixels, on the
    /// same grid the painter snaps cells to, lands exactly on its cell and
    /// meets its neighbour; the image inside is placed from the corner every
    /// tile of the placement shares, so it runs on without a step.
    #[test]
    fn placeholder_tiles_snap_to_the_cell_grid_and_share_one_image_origin() {
        use crate::grid::{Col, Row, column_edge, row_edge};
        let placement = placement(1001, 3);
        let (width, height, scale) = (px(8.4), px(16.8), 2.0);
        let fit = fit_image(40, 40, 2.0 * 8.4, 16.8).unwrap();
        let tile = |column: u16, image_column: u32| {
            tile_geometry(
                &ImageCell {
                    placement: &placement,
                    column,
                    row: 3,
                    image_column,
                    image_row: 0,
                },
                &fit,
                width,
                height,
                scale,
            )
        };
        let (first, second) = (tile(5, 0), tile(6, 1));
        for value in [
            first.left,
            first.top,
            first.width,
            first.height,
            second.left,
            second.width,
        ] {
            let device = f32::from(value) * scale;
            assert!(
                (device - device.round()).abs() < 1e-3,
                "{value:?} is not on a device pixel"
            );
        }
        assert_eq!(first.left, column_edge(px(0.0), width, Col(5), scale).pixels());
        assert_eq!(first.top, row_edge(px(0.0), height, Row(3), scale).pixels());
        assert_eq!(first.left + first.width, second.left, "neighbouring tiles meet");
        let origin = |tile: TileGeometry| f32::from(tile.left + tile.image_left);
        assert!(
            (origin(first) - origin(second)).abs() < 1e-3,
            "both tiles place the image from the same corner"
        );
    }
```

- [ ] **Step 2: Run it and confirm it fails**

```sh
TERM=dumb cargo test -p sprite-app --lib --locked --offline terminal_view::placeholder::tests::placeholder_tiles_snap_to_the_cell_grid_and_share_one_image_origin -- --exact
```

Expected: compile errors `cannot find function tile_geometry`, `cannot find type TileGeometry`.

- [ ] **Step 3: Implement**

`crates/sprite-app/src/terminal_view/placeholder.rs` imports (after line 3):

```rust
use gpui::{Pixels, px};
```

and after `fit_image` (after line 355):

```rust
/// Where one placeholder tile and the image inside it sit, relative to the
/// grid's corner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct TileGeometry {
    pub left: Pixels,
    pub top: Pixels,
    pub width: Pixels,
    pub height: Pixels,
    /// The image's corner, relative to the tile's own.
    pub image_left: Pixels,
    pub image_top: Pixels,
}

/// Places one tile on the device-pixel grid the cells are painted on.
///
/// The tile's edges are the snapped column and row edges the painter uses, so
/// a tile meets its neighbours and the cells beside it edge to edge. The image
/// inside is positioned from the placement's own unsnapped corner, which every
/// tile of it shares, so the picture is one continuous image across tiles.
pub(super) fn tile_geometry(
    cell: &ImageCell<'_>,
    fit: &ImageFit,
    cell_width: Pixels,
    cell_height: Pixels,
    scale: f32,
) -> TileGeometry {
    use crate::grid::{Col, Row, column_edge, row_edge};
    let column = u32::from(cell.column);
    let left = column_edge(px(0.0), cell_width, Col(column), scale).pixels();
    let right = column_edge(px(0.0), cell_width, Col(column + 1), scale).pixels();
    let top = row_edge(px(0.0), cell_height, Row(cell.row), scale).pixels();
    let bottom = row_edge(px(0.0), cell_height, Row(cell.row + 1), scale).pixels();
    let image_left = px(
        (f32::from(cell.column) - cell.image_column as f32) * f32::from(cell_width) + fit.left,
    );
    let image_top =
        px((cell.row as f32 - cell.image_row as f32) * f32::from(cell_height) + fit.top);
    TileGeometry {
        left,
        top,
        width: right - left,
        height: bottom - top,
        image_left: image_left - left,
        image_top: image_top - top,
    }
}
```

`crates/sprite-app/src/terminal_view/render.rs` — replace `placeholder_element` (lines 82-118):

```rust
fn placeholder_element(
    cell: &super::placeholder::ImageCell<'_>,
    texture: Arc<gpui::RenderImage>,
    image: &sprite_term::ImagePixels,
    cell_width: Pixels,
    cell_height: Pixels,
    scale: f32,
) -> Option<gpui::Div> {
    let placement = cell.placement;
    if cell.image_column >= placement.columns || cell.image_row >= placement.rows {
        return None;
    }
    let fit = super::placeholder::fit_image(
        image.width,
        image.height,
        placement.columns as f32 * f32::from(cell_width),
        placement.rows as f32 * f32::from(cell_height),
    )?;
    // Whole device pixels throughout, on the grid the cells are painted on:
    // taffy then has nothing to round, and a tile can neither leave a seam
    // against its neighbour nor sit half a pixel off its cell.
    let tile =
        super::placeholder::tile_geometry(cell, &fit, cell_width, cell_height, scale);
    Some(
        div()
            .absolute()
            .left(tile.left)
            .top(tile.top)
            .w(tile.width)
            .h(tile.height)
            .overflow_hidden()
            .child(
                img(ImageSource::Render(texture))
                    .absolute()
                    .left(tile.image_left)
                    .top(tile.image_top)
                    .w(px(fit.width))
                    .h(px(fit.height)),
            ),
    )
}
```

`image_layers` (line 153) gains a `scale: f32` last parameter:

```rust
    pub(super) fn image_layers(
        &self,
        rows: &[std::sync::Arc<Vec<PositionedCell>>],
        cell_width: Pixels,
        cell_height: Pixels,
        scale: f32,
    ) -> [Vec<gpui::Div>; 3] {
```

and its placeholder call (lines 201-202):

```rust
            if let Some(element) = placeholder_element(
                &cell,
                texture,
                image.as_ref(),
                cell_width,
                cell_height,
                scale,
            ) {
```

`render` (lines 308-309):

```rust
        let [below_background, below_text, above_text] =
            self.image_layers(&rows, cell_width, cell_height, window.scale_factor());
```

- [ ] **Step 4: Run tests, confirm pass**

```sh
TERM=dumb cargo test -p sprite-app --lib --locked --offline terminal_view::placeholder::
TERM=dumb cargo test -p sprite-app --lib --locked --offline terminal_view::
```

- [ ] **Step 5: Commit**

```sh
git add crates/sprite-app/src/terminal_view/placeholder.rs crates/sprite-app/src/terminal_view/render.rs
git commit -m "fix(graphics): snap Kitty placeholder tiles to the cell pixel grid

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

---

### Task 19: Workspace small fixes (BCA-19, BCA-20, BCA-21)

Three independent defects, each its own test-first cycle and its own commit. Do them in order A, B, C; only cycle A and Task 20 share a file (`workspace/mod.rs`).

**Files:**
- Modify: `crates/sprite-app/src/workspace/mod.rs:62-67` (floor doc), `:95-98` (zoom field), `:248` (config print), `:281` (initializer)
- Modify: `crates/sprite-app/src/workspace/keymap.rs:26-58` (font zoom functions)
- Modify: `crates/sprite-app/src/workspace/reload.rs:65-84` (reload apply/publish)
- Modify: `crates/sprite-app/src/workspace/pane_factory.rs:18,38` (new panes get the active settings)
- Modify: `crates/sprite-app/src/config.rs:9` (module doc), `:506-526` (`load_from`, new `load_explicit`, `load_candidate`), `:552` (new `unreadable`)
- Modify: `crates/sprite-app/src/main.rs:78-82`
- Modify: `crates/sprite-app/src/observation/client.rs:166`
- Modify: `crates/sprite-app/src/workspace/divider.rs:131-146` (`divider_ratio`) and its test `:530-536`
- Test: `crates/sprite-app/src/workspace/reload.rs` (inline `mod tests`, line 136)
- Test: `crates/sprite-app/src/config.rs` (inline `mod tests`, line 570)
- Test: `crates/sprite-app/tests/client.rs` (binary-level integration tests, uses `run` helper at line 31 and `scratch` at line 269)
- Test: `crates/sprite-app/src/workspace/divider.rs` (inline `mod tests`, line 411)

**Interfaces:**
- Consumes: nothing from other tasks.
- Produces:
  - `Workspace.font_zoom: Option<crate::config::FontSize>` (private field; replaces `configured_font_size`)
  - `pub(super) fn Workspace::font_size(&self) -> crate::config::FontSize`
  - `pub(super) fn Workspace::active_settings(&self) -> crate::config::Settings` — what panes run with (file settings at the zoomed size). Any later task that publishes `ActiveSettings` or creates a pane from `self.settings` must use this.
  - `pub fn Settings::load_explicit(path: &Path) -> (Settings, Complaints)`
  - `fn unreadable(path: &Path, error: &std::io::Error) -> String` (private to `config.rs`; the one place the "could not be read" wording lives)
  - `divider_ratio` keeps its signature `(origin: f32, extent: f32, pointer: f32, floor: f32) -> f32`.

---

#### Cycle A — BCA-19: font zoom held apart from the file's settings

Today `apply_font_size` (keymap.rs:53) writes the zoomed size into `self.settings.font.size`, so `reload` (reload.rs:67) diffs the zoomed settings against the file, reports `font` as changed, and then replaces `self.settings` (reload.rs:79), throwing the zoom away. After the change `self.settings` holds only what the file says, the zoom lives in `font_zoom`, reload diffs file against file, and zoom survives. If the file changes `font.size` while zoomed, the zoom is kept (reset then goes to the new file size); an unzoomed window follows the file.

- [ ] **Step 1: Write the failing tests** — append to `mod tests` in `crates/sprite-app/src/workspace/reload.rs` (after `a_reload_report_says_what_happened_to_each_part`, before the module's closing `}` at line 368). Reuses `test_workspace` and `draw_workspace` from `workspace/test_support.rs` (already imported by `use super::super::test_support::*;`).

```rust
    /// Zoom is the person's, not the file's. A reload that only recolours the
    /// window must neither undo three steps of zoom nor claim the font changed,
    /// and reset still returns to the size the file asks for.
    #[gpui::test]
    fn a_colour_only_reload_keeps_the_zoom_and_reports_only_colours(
        cx: &mut gpui::TestAppContext,
    ) {
        let (workspace, cx) = test_workspace(cx);
        let path = std::env::temp_dir().join(format!(
            "sprite-zoom-reload-{}.toml",
            std::process::id()
        ));
        workspace.update(cx, |workspace, _| {
            workspace.config_path = Some(path.clone());
        });
        draw_workspace(cx);
        cx.simulate_keystrokes("ctrl-shift-= ctrl-shift-= ctrl-shift-=");
        let zoomed = crate::config::Font::DEFAULT_SIZE + 3.0;
        cx.update(|_, cx| {
            assert_eq!(
                cx.global::<crate::config::ActiveSettings>().0.font.size,
                zoomed,
                "three zoom steps reached the panes"
            );
        });

        // Observation stays off, as `test_workspace` set it, so the only
        // difference from the running file is the background colour.
        std::fs::write(
            &path,
            "[pane_observation]\nenabled = false\n\n[colors]\nbackground = \"#203040\"\n",
        )
        .unwrap();
        let report = workspace.update(cx, |workspace, cx| workspace.reload(None, cx));
        std::fs::remove_file(&path).unwrap();

        assert_eq!(
            report,
            format!("reloaded {}\napplied now: colors", path.display())
        );
        cx.update(|_, cx| {
            let active = &cx.global::<crate::config::ActiveSettings>().0;
            assert_eq!(active.font.size, zoomed, "the zoom survives the reload");
            assert_eq!(
                active.colors.background,
                crate::config::Colors::parse_hex("#203040")
            );
        });
        cx.simulate_keystrokes("ctrl-shift-0");
        cx.update(|_, cx| {
            assert_eq!(
                cx.global::<crate::config::ActiveSettings>().0.font.size,
                crate::config::Font::DEFAULT_SIZE,
                "reset returns to the file's size"
            );
        });
    }

    /// The file's size still matters while zoomed: it is where reset goes. An
    /// unzoomed window simply follows it.
    #[gpui::test]
    fn a_reloaded_font_size_is_where_reset_goes_and_zoom_stays_on_top(
        cx: &mut gpui::TestAppContext,
    ) {
        let (workspace, cx) = test_workspace(cx);
        let path = std::env::temp_dir().join(format!(
            "sprite-zoom-size-reload-{}.toml",
            std::process::id()
        ));
        workspace.update(cx, |workspace, _| {
            workspace.config_path = Some(path.clone());
        });
        draw_workspace(cx);
        let active_size = |cx: &mut gpui::VisualTestContext| {
            cx.update(|_, cx| {
                cx.global::<crate::config::ActiveSettings>()
                    .0
                    .font
                    .size
                    .get()
            })
        };
        cx.simulate_keystrokes("ctrl-shift-=");
        assert_eq!(active_size(cx), crate::config::Font::DEFAULT_SIZE + 1.0);

        std::fs::write(&path, "[pane_observation]\nenabled = false\n\n[font]\nsize = 20\n")
            .unwrap();
        let report = workspace.update(cx, |workspace, cx| workspace.reload(None, cx));
        assert_eq!(
            report,
            format!("reloaded {}\napplied now: font", path.display())
        );
        assert_eq!(
            active_size(cx),
            crate::config::Font::DEFAULT_SIZE + 1.0,
            "a zoomed window keeps its zoom"
        );
        cx.simulate_keystrokes("ctrl-shift-0");
        assert_eq!(active_size(cx), 20.0, "reset goes to the new file size");

        std::fs::write(&path, "[pane_observation]\nenabled = false\n\n[font]\nsize = 22\n")
            .unwrap();
        workspace.update(cx, |workspace, cx| workspace.reload(None, cx));
        std::fs::remove_file(&path).unwrap();
        assert_eq!(active_size(cx), 22.0, "an unzoomed window follows the file");
    }
```

- [ ] **Step 2: Run them and confirm they fail**

```
TERM=dumb cargo test -p sprite-app --locked --offline --lib workspace::reload::tests::a_colour_only_reload_keeps_the_zoom_and_reports_only_colours -- --exact
TERM=dumb cargo test -p sprite-app --locked --offline --lib workspace::reload::tests::a_reloaded_font_size_is_where_reset_goes_and_zoom_stays_on_top -- --exact
```

Expected: the first fails at the report assertion with `left: "reloaded /…/sprite-zoom-reload-<pid>.toml\napplied now: font, colors"` vs `right: "…\napplied now: colors"`. The second fails at `"a zoomed window keeps its zoom"` with `left: 20.0, right: 15.0`. If either fails earlier, at the first `"three zoom steps reached the panes"` / `DEFAULT_SIZE + 1.0` check, the keystroke did not reach the workspace: replace each `cx.simulate_keystrokes("ctrl-shift-=")` with `workspace.update(cx, |workspace, cx| workspace.adjust_font(1.0, cx));` and `"ctrl-shift-0"` with `workspace.update(cx, |workspace, cx| workspace.reset_font(cx));` and rerun (see Drafter notes b1).

- [ ] **Step 3: Implement**

3a. `crates/sprite-app/src/workspace/mod.rs:95-98` — replace the field:

```rust
    settings: crate::config::Settings,
    /// The size the configuration asked for, so "reset" returns to what a
    /// person set rather than to Sprite's own default.
    configured_font_size: crate::config::FontSize,
```

with:

```rust
    /// What the configuration file says, and nothing else.
    ///
    /// Font zoom is kept beside it in `font_zoom` rather than written into it,
    /// so a reload compares the file with the file: zooming is not a
    /// configuration change, and a reload that only recoloured the window
    /// neither undoes the zoom nor reports a font change. What panes run with
    /// is `active_settings`.
    settings: crate::config::Settings,
    /// The size font zoom chose, while it differs from the file's.
    ///
    /// `None` follows the file's size. Reset clears it, which returns to what
    /// a person configured rather than to Sprite's own default.
    font_zoom: Option<crate::config::FontSize>,
```

3b. `crates/sprite-app/src/workspace/mod.rs:281` — in `Workspace::new`'s `Self { … }` initializer replace

```rust
            configured_font_size: settings.font.size,
```

with

```rust
            font_zoom: None,
```

3c. `crates/sprite-app/src/workspace/mod.rs:246-248` — the config print verb; replace

```rust
                        // Printed from what the window is *using*, which after
                        // a reload is not necessarily what the file says.
                        ConfigVerb::Print => workspace.settings.to_toml(),
```

with

```rust
                        // Printed from what the window is *using*, which after
                        // a reload or a zoom is not necessarily what the file
                        // says.
                        ConfigVerb::Print => workspace.active_settings().to_toml(),
```

3d. `crates/sprite-app/src/workspace/keymap.rs:26-58` — replace `adjust_font`, `reset_font` and `apply_font_size` (everything from the `/// Changes the text size of every pane in this window.` doc comment through the closing `}` of `apply_font_size`) with:

```rust
    /// Changes the text size of every pane in this window.
    ///
    /// Every pane rather than the focused one: a window with one pane in a
    /// different size from its neighbours looks broken. Each pane re-measures
    /// its cell and tells its child the new grid, which is why this resizes
    /// rather than merely redraws.
    pub(super) fn adjust_font(&mut self, delta: f32, cx: &mut Context<Self>) {
        // A keystroke has no complaints channel, so the size is simply held
        // inside the readable range; a file setting goes through the same
        // rule and says so when it had to.
        let wanted = crate::config::FontSize::new(self.font_size().get() + delta);
        self.apply_font_size(wanted, cx);
    }
    /// Back to the configured size, which is what a person means by "reset" —
    /// not back to Sprite's built-in default.
    pub(super) fn reset_font(&mut self, cx: &mut Context<Self>) {
        let configured = self.settings.font.size;
        self.apply_font_size(configured, cx);
    }
    /// Zooms to `size`. A size equal to the file's is no zoom at all, so the
    /// window goes back to following the file.
    pub(super) fn apply_font_size(
        &mut self,
        size: crate::config::FontSize,
        cx: &mut Context<Self>,
    ) {
        let zoom = (size != self.settings.font.size).then_some(size);
        if zoom == self.font_zoom {
            return;
        }
        self.font_zoom = zoom;
        // The size travels the way a reload does: published once, applied by
        // every pane with its own window.
        cx.set_global(crate::config::ActiveSettings(self.active_settings()));
        cx.notify();
    }
    /// The size panes draw at: the zoom while there is one, else the file's.
    pub(super) fn font_size(&self) -> crate::config::FontSize {
        self.font_zoom.unwrap_or(self.settings.font.size)
    }
    /// What this window's panes run with: the file's settings, at the zoomed
    /// size when there is one.
    pub(super) fn active_settings(&self) -> crate::config::Settings {
        let mut settings = self.settings.clone();
        settings.font.size = self.font_size();
        settings
    }
```

3e. `crates/sprite-app/src/workspace/reload.rs:65-84` — replace from `let (settings, complaints) = candidate;` through `outcome.describe(&path, &complaints.0)` with:

```rust
        let (settings, complaints) = candidate;

        // File against file: the zoom lives beside `self.settings`, so a
        // zoomed window is not mistaken for a font change and the zoom
        // outlasts the reload.
        let outcome = self.settings.diff(&settings);
        if outcome.has(crate::config::LiveChange::Colors) {
            cx.global_mut::<crate::tokens::TokenRegistry>()
                .apply_theme(&settings.colors);
        }
        if outcome.has(crate::config::LiveChange::Observation) {
            self.change_observation(settings.pane_observation.enabled, reply, cx);
        }
        self.settings = settings;
        // A zoom that now matches the file's own size is no zoom at all, so
        // the window follows the file from here on.
        if self.font_zoom == Some(self.settings.font.size) {
            self.font_zoom = None;
        }
        // Published, not pushed: each pane observes the global with its own
        // window in hand, which is what a cell re-measure needs and what this
        // method, reached from an endpoint thread, does not have.
        cx.set_global(crate::config::ActiveSettings(self.active_settings()));
        cx.notify();

        outcome.describe(&path, &complaints.0)
```

3f. `crates/sprite-app/src/workspace/pane_factory.rs:18` and `:38` — in `split` and `open_tab`, replace each

```rust
                self.settings.clone(),
```

(line 18, inside `make_pane(` in `split`) and

```rust
            self.settings.clone(),
```

(line 38, inside `make_pane(` in `open_tab`) with `self.active_settings(),` at the same indentation, so a pane opened while zoomed starts at the zoomed size as it does today.

Leave `workspace.settings.clone()` in the test-only sites `surface_routing.rs:74` and `layout_tests.rs:145` unchanged.

- [ ] **Step 4: Run tests, confirm pass**

```
TERM=dumb cargo test -p sprite-app --locked --offline --lib workspace::reload::tests
TERM=dumb cargo test -p sprite-app --locked --offline --lib workspace::keymap::tests
TERM=dumb cargo test -p sprite-app --locked --offline --lib workspace::close_gate::tests::queued_reload_cannot_reenable_observation_during_shutdown -- --exact
TERM=dumb cargo test -p sprite-app --locked --offline --lib terminal_view
```

All pass, including the existing `reload_reconciles_observation_endpoint_and_revokes_old_credentials`, `changes_are_sorted_by_when_they_can_apply` and `a_reload_report_says_what_happened_to_each_part`.

- [ ] **Step 5: Commit**

```
git add crates/sprite-app/src/workspace/mod.rs crates/sprite-app/src/workspace/keymap.rs crates/sprite-app/src/workspace/reload.rs crates/sprite-app/src/workspace/pane_factory.rs
git commit -m "fix(workspace): keep font zoom apart from the file's settings

Reload now diffs the file against the file, so a colour-only reload keeps
the zoom and no longer reports a font change. Reset returns to the file's
size; an unzoomed window still follows the file.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

#### Cycle B — BCA-20: an explicit `--config` that cannot be read says so

`main.rs:81` calls `Settings::load_from`, which returns defaults with no complaint for *any* read failure (config.rs:508-510). Reload's wording is built inline in `load_candidate` (config.rs:524): `"{} could not be read: {error}"`. The fix moves that wording into one function, `unreadable`, uses it from `load_candidate`, from a new `load_explicit` (used by `main.rs` and by `config print --config`), and from discovery's `load_from` for any failure other than absence. An absent discovered file stays silent.

- [ ] **Step 1: Write the failing tests**

1a. Append to `mod tests` in `crates/sprite-app/src/config.rs`, after `a_missing_file_is_not_a_complaint` (which ends at line 1248) and before the module's closing `}` at line 1249:

```rust
    /// A file somebody named outright is one they expect to be used. Not being
    /// able to read it is said, in exactly the words a reload uses, whatever
    /// the reason; the window still opens with the defaults.
    #[test]
    fn an_explicit_file_that_cannot_be_read_is_said_in_the_words_reload_uses() {
        let directory = std::env::temp_dir().join(format!(
            "sprite-explicit-config-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let missing = directory.join("missing.toml");
        // A missing file, and a directory where a file was expected.
        for path in [missing.as_path(), directory.as_path()] {
            let (settings, complaints) = Settings::load_explicit(path);
            assert_eq!(settings, Settings::default());
            let reload = Settings::load_candidate(path).unwrap_err();
            assert!(
                reload.starts_with(&format!("{} could not be read: ", path.display())),
                "{reload}"
            );
            assert_eq!(complaints.0, vec![reload], "{}", path.display());
        }
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn an_explicit_file_without_read_permission_is_a_complaint() {
        use std::os::unix::fs::PermissionsExt;
        let path = std::env::temp_dir().join(format!(
            "sprite-forbidden-config-{}.toml",
            std::process::id()
        ));
        std::fs::write(&path, "[font]\nsize = 20\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
        // Root reads whatever it likes, so there is nothing to prove there.
        if std::fs::read_to_string(&path).is_ok() {
            std::fs::remove_file(&path).unwrap();
            return;
        }
        let (settings, complaints) = Settings::load_explicit(&path);
        std::fs::remove_file(&path).unwrap();
        assert_eq!(settings, Settings::default());
        assert_eq!(complaints.0.len(), 1, "{:?}", complaints.0);
        assert!(
            complaints.0[0].starts_with(&format!("{} could not be read: ", path.display())),
            "{:?}",
            complaints.0
        );
    }

    /// Only *absence* of the discovered file is silent. One that is there and
    /// cannot be read is somebody's settings going unused.
    #[test]
    fn a_discovered_file_that_is_there_but_cannot_be_read_is_a_complaint() {
        let directory = std::env::temp_dir().join(format!(
            "sprite-discovered-config-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let (settings, complaints) = Settings::load_from(&directory);
        std::fs::remove_dir_all(&directory).unwrap();
        assert_eq!(settings, Settings::default());
        assert_eq!(complaints.0.len(), 1, "{:?}", complaints.0);
        assert!(
            complaints.0[0].starts_with(&format!("{} could not be read: ", directory.display())),
            "{:?}",
            complaints.0
        );
    }
```

1b. Binary-level check. Insert into `crates/sprite-app/tests/client.rs` after line 517 (the closing `}` of `config_print_validates_the_window_answer_before_printing`). Reuses `run` (line 31) and `scratch` (line 269); `scratch()` names a directory that is never created, so the file is missing.

```rust
/// A file named outright that cannot be read is said to be unreadable, in the
/// words a reload uses, rather than quietly replaced by defaults.
#[test]
fn config_print_says_when_a_named_file_cannot_be_read() {
    let missing = scratch().join("missing.toml");
    let path = missing.to_string_lossy().into_owned();
    let outcome = run(&["config", "print", "--config", &path], &[]);
    assert_eq!(outcome.status, 0, "{}", outcome.errors);
    assert!(
        outcome
            .errors
            .contains(&format!("sprite: {path} could not be read: ")),
        "{}",
        outcome.errors
    );
    assert!(
        outcome.out.starts_with(&format!("# {path}\n")),
        "{}",
        outcome.out
    );
}
```

- [ ] **Step 2: Run them and confirm they fail**

```
TERM=dumb cargo test -p sprite-app --locked --offline --lib config::tests
TERM=dumb cargo test -p sprite-app --locked --offline --test client config_print_says_when_a_named_file_cannot_be_read -- --exact
```

Expected: the first does not compile — `error[E0599]: no function or associated item named 'load_explicit' found for struct 'Settings'`. The second compiles and fails its stderr assertion with an empty `outcome.errors` (today the missing file silently prints defaults).

- [ ] **Step 3: Implement**

3a. `crates/sprite-app/src/config.rs:9` — replace the module-doc line

```rust
//! Startup falls back to defaults if the file is unreadable or invalid TOML.
```

with

```rust
//! Startup falls back to defaults if the file is unreadable or invalid TOML,
//! and says so; only a discovered file that is absent is silent.
```

3b. `crates/sprite-app/src/config.rs:506-512` — replace `load_from`:

```rust
    /// Reads one file. A missing file is not a complaint: most people have none.
    pub fn load_from(path: &Path) -> (Self, Complaints) {
        let Ok(text) = std::fs::read_to_string(path) else {
            return (Self::default(), Complaints::default());
        };
        Self::parse(&text)
    }
```

with:

```rust
    /// Reads the file discovery found. Only its absence is not a complaint:
    /// most people have no configuration file, but one that is there and
    /// cannot be read is somebody's settings going unused.
    pub fn load_from(path: &Path) -> (Self, Complaints) {
        match std::fs::metadata(path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                (Self::default(), Complaints::default())
            }
            _ => Self::load_explicit(path),
        }
    }

    /// Reads a file somebody named outright, with `--config`.
    ///
    /// Naming a file says it should be used, so one that cannot be read —
    /// missing, forbidden, or not a file at all — is a complaint in the same
    /// words a reload uses. The defaults stand in for it, because a terminal
    /// must open.
    pub fn load_explicit(path: &Path) -> (Self, Complaints) {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text),
            Err(error) => (Self::default(), Complaints(vec![unreadable(path, &error)])),
        }
    }
```

3c. `crates/sprite-app/src/config.rs:522-526` — in `load_candidate`, replace

```rust
        let text = std::fs::read_to_string(path)
            .map_err(|error| format!("{} could not be read: {error}", path.display()))?;
```

with

```rust
        let text = std::fs::read_to_string(path).map_err(|error| unreadable(path, &error))?;
```

3d. `crates/sprite-app/src/config.rs` — insert directly above `/// Where this user's configuration lives, when it can be located at all.` (line 552):

```rust
/// Why a configuration file could not be read, worded once so that startup,
/// `config print` and reload all say it the same way.
fn unreadable(path: &Path, error: &std::io::Error) -> String {
    format!("{} could not be read: {error}", path.display())
}

```

3e. `crates/sprite-app/src/main.rs:78-82` — replace

```rust
    let (settings, complaints) = match &args.config {
        // Explicit, so it wins over discovery.
        Some(path) => Settings::load_from(path),
        None => Settings::load(),
    };
```

with

```rust
    let (settings, complaints) = match &args.config {
        // Explicit, so it wins over discovery — and must be there: a named
        // file that cannot be read is said rather than silently replaced.
        Some(path) => Settings::load_explicit(path),
        None => Settings::load(),
    };
```

(The existing loop at main.rs:83-85 already prints each complaint as `sprite: {complaint}` to stderr and the window then opens with the defaults.)

3f. `crates/sprite-app/src/observation/client.rs:166` — in `run_config_print`'s explicit-path branch replace

```rust
        let (settings, complaints) = crate::config::Settings::load_from(path);
```

with

```rust
        let (settings, complaints) = crate::config::Settings::load_explicit(path);
```

Leave `client.rs:216` (the discovery branch) on `load_from`.

- [ ] **Step 4: Run tests, confirm pass**

```
TERM=dumb cargo test -p sprite-app --locked --offline --lib config::
TERM=dumb cargo test -p sprite-app --locked --offline --lib workspace::reload::tests
TERM=dumb cargo test -p sprite-app --locked --offline --lib observation::client::tests
TERM=dumb cargo test -p sprite-app --locked --offline --test client config_print
```

All pass, including the existing `a_missing_file_is_not_a_complaint`.

- [ ] **Step 5: Commit**

```
git add crates/sprite-app/src/config.rs crates/sprite-app/src/main.rs crates/sprite-app/src/observation/client.rs crates/sprite-app/tests/client.rs
git commit -m "fix(config): say when an explicit --config file cannot be read

A named file that is missing, forbidden or a directory now prints the
same \"could not be read\" complaint reload uses, from one function, and
startup continues with defaults. Only an absent discovered file is silent.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

#### Cycle C — BCA-21: small splits can still move

> **Superseded (2026-10-09, after review):** the code, test names and test values in Steps 1–3 below are the first, piecewise draft (`extent < floor * 2.0`, with "splits of 240 px or more behave exactly as before"). That draft pinned 240–280 px splits. What landed is the smooth floor this paragraph describes, `floor.min(extent / 4.0)`: splits of 480 px and wider are unchanged, the 400 px divider tests moved to 800 px, and `a_split_smaller_than_two_floors_moves_within_its_middle_half` became `a_small_split_moves_within_its_middle_half`, joined by `travel_has_no_cliff_where_the_old_rule_changed` and `a_bigger_split_never_travels_less` (the drag and nudge tests kept their names). See drafter note a1 and the PRD's BCA-21 row.

`divider_ratio` (divider.rs:137-146) returns 0.5 for any split with `extent < floor * 2`, so a split under 240 px cannot move at all. The fix is a smooth floor: `min(120 px, extent / 4)`, so a split's travel grows with its size and no split is pinned; splits of 480 px and wider are unchanged. Both the drag (`DividerDrag::ratio_for`, line 313) and the nudge (`nudged_ratio`, line 155) go through `divider_ratio`, so this one change fixes both.

All existing divider tests stay unchanged except `a_split_too_small_for_two_floors_stays_even` (lines 530-536). That test asserts the defect, so the new tests replace it.

- [ ] **Step 1: Write the failing tests** — append to `mod tests` in `crates/sprite-app/src/workspace/divider.rs`, before the module's closing `}` at line 719:

```rust
    /// A split too small to give both sides the full floor still moves: each
    /// side keeps at least a quarter of it instead of the boundary being
    /// pinned to the middle.
    #[test]
    fn a_split_smaller_than_two_floors_moves_within_its_middle_half() {
        // 200 px: the floor relaxes to 50, a quarter of the split.
        assert!((divider_ratio(0.0, 200.0, 10.0, 120.0) - 0.25).abs() < 1e-6);
        assert!((divider_ratio(0.0, 200.0, 190.0, 120.0) - 0.75).abs() < 1e-6);
        // Inside that range the boundary is under the pointer.
        assert!((divider_ratio(0.0, 200.0, 80.0, 120.0) - 0.4).abs() < 1e-6);
        // Just under two floors is still a small split.
        assert!((divider_ratio(0.0, 239.0, 0.0, 120.0) - 0.25).abs() < 1e-6);
        // Two floors or more keep the full floor: 120 of 300 is 0.4.
        assert!((divider_ratio(0.0, 300.0, 0.0, 120.0) - 0.4).abs() < 1e-6);
        // No room at all still has nowhere to go but even.
        assert!((divider_ratio(0.0, 0.0, 10.0, 120.0) - 0.5).abs() < 1e-6);
    }
    #[test]
    fn a_drag_moves_a_split_smaller_than_two_floors() {
        // A 200 px split, grabbed on its line at 100.
        let placed = divider_placements(
            &[crate::pane_tree::Divider {
                pane: PaneId(0),
                direction: Direction::Right,
                orientation: Orientation::Horizontal,
                ratio: 0.5,
                area: crate::pane_tree::Rect::FULL,
            }],
            200.0,
            600.0,
            0.0,
        )[0];
        let drag = DividerDrag::begin(placed, 100.0);
        assert!(
            (drag.ratio_for(60.0) - 0.3).abs() < 1e-4,
            "the boundary follows the pointer"
        );
        assert!(
            (drag.ratio_for(-100.0) - 0.25).abs() < 1e-4,
            "and stops a quarter in"
        );
        assert!((drag.ratio_for(400.0) - 0.75).abs() < 1e-4);
    }
    #[test]
    fn a_nudge_moves_a_split_smaller_than_two_floors() {
        // A quarter of an 800 px container is a 200 px split, so one 20 px
        // step is 0.1 of it.
        let divider = |ratio| crate::pane_tree::Divider {
            pane: PaneId(0),
            direction: Direction::Left,
            orientation: Orientation::Horizontal,
            ratio,
            area: crate::pane_tree::Rect {
                x: 0.0,
                y: 0.0,
                width: 0.25,
                height: 1.0,
            },
        };
        let moved = nudged_ratio(&divider(0.5), 800.0, 400.0, Direction::Left);
        assert!((moved - 0.4).abs() < 1e-6, "a nudge moves it");
        let held = nudged_ratio(&divider(0.25), 800.0, 400.0, Direction::Left);
        assert!((held - 0.25).abs() < 1e-6, "and stops a quarter in");
    }
```

- [ ] **Step 2: Run them and confirm they fail**

```
TERM=dumb cargo test -p sprite-app --locked --offline --lib workspace::divider::tests::a_split_smaller_than_two_floors_moves_within_its_middle_half -- --exact
TERM=dumb cargo test -p sprite-app --locked --offline --lib workspace::divider::tests::a_drag_moves_a_split_smaller_than_two_floors -- --exact
TERM=dumb cargo test -p sprite-app --locked --offline --lib workspace::divider::tests::a_nudge_moves_a_split_smaller_than_two_floors -- --exact
```

Expected: each fails at its first assertion (`assertion failed: (divider_ratio(0.0, 200.0, 10.0, 120.0) - 0.25).abs() < 1e-6`, `the boundary follows the pointer`, `a nudge moves it`), because every call on a 200 px split returns 0.5.

- [ ] **Step 3: Implement**

3a. `crates/sprite-app/src/workspace/divider.rs:137-146` — replace `divider_ratio`:

```rust
pub(super) fn divider_ratio(origin: f32, extent: f32, pointer: f32, floor: f32) -> f32 {
    // A split with no room, or too little to honour the floor on both sides,
    // has no position that obeys the rule. Even is the least surprising of the
    // answers that break it.
    if extent <= 0.0 || extent < floor * 2.0 {
        return 0.5;
    }
    let low = floor / extent;
    ((pointer - origin) / extent).clamp(low, 1.0 - low)
}
```

with:

```rust
pub(super) fn divider_ratio(origin: f32, extent: f32, pointer: f32, floor: f32) -> f32 {
    // A split with no room has no position at all; even is the only answer.
    if extent <= 0.0 {
        return 0.5;
    }
    // A split too small to give both sides the full floor still moves: each
    // side keeps a quarter of it. Pinning such a split to the middle left a
    // boundary between two small panes that nothing could adjust.
    let floor = if extent < floor * 2.0 {
        extent / 4.0
    } else {
        floor
    };
    let low = floor / extent;
    ((pointer - origin) / extent).clamp(low, 1.0 - low)
}
```

3b. `crates/sprite-app/src/workspace/mod.rs:62-67` — extend the floor's doc comment. Replace

```rust
/// The narrowest either side of a dragged split may become.
///
/// Roughly fifteen columns or six rows at the default font size. It holds the
/// side, not the panes nested inside it: a side that is itself split shares
/// this width among its own panes.
const DIVIDER_FLOOR_PX: f32 = 120.0;
```

with

```rust
/// The narrowest either side of a dragged split may become.
///
/// Roughly fifteen columns or six rows at the default font size. It holds the
/// side, not the panes nested inside it: a side that is itself split shares
/// this width among its own panes. A split under four floors uses a quarter of
/// itself instead, so a small split can still be moved.
const DIVIDER_FLOOR_PX: f32 = 120.0;
```

3c. In `divider.rs`, delete `a_split_too_small_for_two_floors_stays_even` (lines 530-536, from its `#[test]` through its closing `}`). It asserts the defect. The new `a_split_smaller_than_two_floors_moves_within_its_middle_half` covers both of its inputs: 200 px now moves, and 0 px still gives 0.5.

Leave every other divider test as it is. Splits of 240 px or more behave exactly as before, so the tests on 400 px and 800 px splits still hold. That includes `a_nudge_still_stops_at_the_floor`, `neither_side_may_be_driven_below_the_floor`, `a_boundary_pushed_past_the_floor_comes_straight_back`, `a_nudge_steps_the_boundary_the_right_way_on_each_axis` and `a_drag_holds_the_floor_it_was_given`.

- [ ] **Step 4: Run tests, confirm pass**

```
TERM=dumb cargo test -p sprite-app --locked --offline --lib workspace::divider::tests
TERM=dumb cargo test -p sprite-app --locked --offline --lib workspace::layout_tests
TERM=dumb cargo test -p sprite-app --locked --offline --lib workspace::keymap::tests
```

- [ ] **Step 5: Commit**

```
git add crates/sprite-app/src/workspace/divider.rs crates/sprite-app/src/workspace/mod.rs
git commit -m "fix(workspace): let splits smaller than two floors move

A split narrower than twice the floor uses a quarter of itself as its
floor, so its boundary moves within [0.25, 0.75] by drag and nudge
instead of being pinned to 0.5. Larger splits keep the full floor.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 20: Pane cleanup on its own short-lived thread (BCA-32, PRD R-W6, ADR 0029)

`Workspace::shut_down` (close_gate.rs:41-64) runs each pane's blocking cleanup (`TerminalView`'s `ShutdownHandle::wait`, which joins the worker and runs HUP/TERM/KILL escalation in `worker::finish_shutdown`, session.rs:127-136) inside `cx.background_executor().spawn(…)`. On Linux that executor is a fixed pool of `available_parallelism()` threads. After this task each cleanup runs on its own thread named `sprite-pane-cleanup`. That thread reports through an `async_channel::bounded(1)`. The window's `PendingCleanup` keeps a GPUI task that only awaits that report, so it holds no executor thread while it waits. ADR 0024's lifetime rules stay as they are: the window owns pending cleanup, quit waits for removed and present panes, completed entries are pruned, and shutdown is idempotent.

**Files:**
- Modify: `crates/sprite-app/src/workspace/close_gate.rs:1-64` (constant, `shut_down`) and its tests `:431-682`
- Modify: `crates/sprite-app/src/workspace/mod.rs:107-108` (test-only field), `:287-288` (initializer). Line numbers are at a62247e. Task 19's cycles A and C add lines above these (+10 and +1), so the lines move down by about 11; apply by the context shown.
- Modify: `crates/sprite-app/src/workspace/surface_routing.rs:156-163` (test only: allow parking)
- Test: `crates/sprite-app/src/workspace/close_gate.rs` (inline `mod tests`, line 311)

**Interfaces:**
- Consumes: nothing from other tasks (Task 19's `mod.rs` edits touch neighbouring lines only).
- Produces:
  - `const CLEANUP_THREAD: &str = "sprite-pane-cleanup";` (private to `close_gate.rs`)
  - `#[cfg(test)] Workspace.cleanup_threads: Vec<Option<std::thread::JoinHandle<()>>>`: each cleanup thread's handle, in start order. Index `i` matches the `i`-th gate taken from `cleanup_gates`.
  - Test helper `close_gate::tests::release(workspace, cx, gates, index)`.
  - `Workspace::shut_down`, `begin_shutdown`, `shutdown_and_quit` and `PendingCleanup { task, completed }` keep their signatures.

- [ ] **Step 1: Write the failing test** — add to `mod tests` in `crates/sprite-app/src/workspace/close_gate.rs`, directly after `window_shutdown_includes_background_panes_in_identity_order_once` (before the module's closing `}` at line 683). It follows the same pattern as the neighbouring `ShutdownPane` (lines 342-386) and builds panes the way `gated_panes` (lines 431-462) does.

```rust
    /// Where each cleanup ran, and whether it ever saw every other cleanup
    /// running at the same moment as itself.
    struct Rendezvous {
        expected: usize,
        arrived: std::sync::Mutex<Vec<(Option<String>, std::thread::ThreadId)>>,
        everyone: std::sync::Condvar,
        met: std::sync::atomic::AtomicUsize,
    }

    impl Rendezvous {
        /// Blocks, as a real cleanup does, until every cleanup has arrived or
        /// five seconds pass. Cleanups that share one thread can never all
        /// arrive together, so they time out instead of hanging the test.
        fn arrive(&self) {
            let current = std::thread::current();
            let mut arrived = self.arrived.lock().unwrap();
            arrived.push((current.name().map(str::to_owned), current.id()));
            self.everyone.notify_all();
            let (_arrived, waited) = self
                .everyone
                .wait_timeout_while(arrived, std::time::Duration::from_secs(5), |arrived| {
                    arrived.len() < self.expected
                })
                .unwrap();
            if !waited.timed_out() {
                self.met.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
        }
    }

    struct RendezvousPane {
        focus: gpui::FocusHandle,
        rendezvous: std::sync::Arc<Rendezvous>,
        shutting_down: bool,
    }

    impl gpui::Focusable for RendezvousPane {
        fn focus_handle(&self, _: &gpui::App) -> gpui::FocusHandle {
            self.focus.clone()
        }
    }

    impl gpui::Render for RendezvousPane {
        fn render(
            &mut self,
            _: &mut gpui::Window,
            _: &mut gpui::Context<Self>,
        ) -> impl gpui::IntoElement {
            gpui::div()
        }
    }

    impl gpui::EventEmitter<sprite_pane::TitleChanged> for RendezvousPane {}

    impl sprite_pane::Pane for RendezvousPane {
        type Request = crate::surface::channel::SurfaceRequest;
        fn close_warning(&self) -> Option<sprite_pane::CloseWarning> {
            None
        }
        fn title(&self) -> Option<gpui::SharedString> {
            None
        }
        fn set_allocated(&mut self, _: gpui::Size<gpui::Pixels>) {}
        fn begin_shutdown(&mut self) -> Option<Box<dyn FnOnce() + Send>> {
            if std::mem::replace(&mut self.shutting_down, true) {
                return None;
            }
            let rendezvous = self.rendezvous.clone();
            Some(Box::new(move || rendezvous.arrive()))
        }
    }

    /// Blocking cleanup never runs on the GPUI thread or the shared background
    /// executor, whose fixed pool enough closing panes would otherwise fill.
    /// Each cleanup has a named thread of its own, and all of them block at
    /// once, so quitting takes as long as the slowest pane, not the sum.
    #[gpui::test]
    fn pane_cleanups_run_together_on_their_own_named_threads(cx: &mut gpui::TestAppContext) {
        use gpui::AppContext;
        const PANES: usize = 3;
        let (workspace, cx) = test_workspace(cx);
        cx.background_executor.allow_parking();
        let test_thread = std::thread::current().id();
        let rendezvous = std::sync::Arc::new(Rendezvous {
            expected: PANES,
            arrived: Default::default(),
            everyone: Default::default(),
            met: Default::default(),
        });
        workspace.update(cx, |workspace, cx| {
            let mut make = |_, _| {
                Rc::new(cx.new(|cx| RendezvousPane {
                    focus: cx.focus_handle(),
                    rendezvous: rendezvous.clone(),
                    shutting_down: false,
                })) as Rc<dyn PaneHandle<Request = SurfaceRequest>>
            };
            workspace.tabs = Tabs::new(&mut make);
            for _ in 1..PANES {
                workspace.tabs.split(Orientation::Vertical, &mut make);
            }
            workspace.refresh_layout(cx);
        });
        let cleanups = workspace.update(cx, |workspace, cx| workspace.begin_shutdown(cx));
        assert_eq!(cleanups.len(), PANES);
        cx.background_executor.block_test(async move {
            for cleanup in cleanups {
                cleanup.await;
            }
        });

        let arrived = rendezvous.arrived.lock().unwrap().clone();
        assert_eq!(arrived.len(), PANES);
        for (name, thread) in &arrived {
            assert_eq!(
                name.as_deref(),
                Some("sprite-pane-cleanup"),
                "a cleanup ran on the shared executor or the GPUI thread"
            );
            assert_ne!(*thread, test_thread);
        }
        let threads: std::collections::HashSet<_> =
            arrived.iter().map(|(_, thread)| *thread).collect();
        assert_eq!(threads.len(), PANES, "each cleanup has a thread of its own");
        assert_eq!(
            rendezvous.met.load(std::sync::atomic::Ordering::SeqCst),
            PANES,
            "every cleanup was running at the same time as all the others"
        );
    }
```

- [ ] **Step 2: Run it and confirm it fails**

```
TERM=dumb cargo test -p sprite-app --locked --offline --lib workspace::close_gate::tests::pane_cleanups_run_together_on_their_own_named_threads -- --exact
```

Expected: it fails after about ten seconds. At a62247e the three cleanups run one after another on the test thread: the first two each time out after 5 s, then the third. The first failing assertion is `a cleanup ran on the shared executor or the GPUI thread`, with `left: Some("workspace::close_gate::tests::pane_cleanups_run_together_on_their_own_named_threads")` (or `Some("main")` under `--test-threads=1`) and `right: Some("sprite-pane-cleanup")`.

- [ ] **Step 3: Implement**

3a. `crates/sprite-app/src/workspace/mod.rs:107-108` — after the existing test-only field

```rust
    #[cfg(test)]
    cleanup_gates: std::collections::VecDeque<async_channel::Receiver<()>>,
```

add

```rust
    /// Each pane cleanup thread, in the order it started, so a test can wait
    /// for one to finish before it runs the executor.
    #[cfg(test)]
    cleanup_threads: Vec<Option<std::thread::JoinHandle<()>>>,
```

and in `Workspace::new`'s initializer after (lines 287-288)

```rust
            #[cfg(test)]
            cleanup_gates: Default::default(),
```

add

```rust
            #[cfg(test)]
            cleanup_threads: Vec::new(),
```

3b. `crates/sprite-app/src/workspace/close_gate.rs` — after `fn quit_app` (line 12) and before `impl Workspace {` (line 14), insert:

```rust
/// What each pane cleanup thread is called, so a stuck one can be found by
/// name in a debugger or a process listing.
const CLEANUP_THREAD: &str = "sprite-pane-cleanup";

```

3c. `crates/sprite-app/src/workspace/close_gate.rs:40-64` — replace `shut_down` (from its `/// Blocking cleanup outlives removal…` doc line through its closing `}`) with:

```rust
    /// Starts a pane's blocking cleanup and keeps track of it until it
    /// finishes, even after the pane has left the layout.
    ///
    /// The cleanup — hangup, terminate and kill escalation, then joins — can
    /// take seconds, so it runs on a short-lived thread of its own rather than
    /// on GPUI's shared background executor. On Linux that executor is a fixed
    /// pool, and enough busy panes closing at once would occupy all of it and
    /// starve every other piece of background work. One thread per cleanup
    /// also keeps cleanups concurrent, so quitting takes as long as the slowest
    /// pane rather than the sum of them. What the window keeps is a task that
    /// only awaits the thread's report, which holds no executor thread while
    /// it waits.
    pub(super) fn shut_down(
        &mut self,
        pane: Rc<dyn PaneHandle<Request = SurfaceRequest>>,
        cx: &mut Context<Self>,
    ) {
        self.pending_cleanups
            .retain(|cleanup| !cleanup.completed.load(std::sync::atomic::Ordering::Acquire));
        let Some(cleanup) = pane.begin_shutdown(cx) else {
            return;
        };
        let completed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mark_completed = completed.clone();
        // The channel closing counts as the report too, so a cleanup that
        // panics still lets quit go ahead instead of holding it forever.
        let (report, reported) = async_channel::bounded::<()>(1);
        // Kept where a failed spawn can hand it back: a cleanup that never
        // runs leaves a busy child without its escalation.
        let slot = Arc::new(std::sync::Mutex::new(Some(cleanup)));
        #[cfg(test)]
        let gate = self.cleanup_gates.pop_front();
        let spawned = std::thread::Builder::new()
            .name(CLEANUP_THREAD.to_owned())
            .spawn({
                let slot = Arc::clone(&slot);
                move || {
                    #[cfg(test)]
                    if let Some(gate) = gate {
                        // A dropped gate releases the cleanup as surely as an
                        // opened one.
                        let _ = gate.recv_blocking();
                    }
                    let cleanup = slot.lock().ok().and_then(|mut slot| slot.take());
                    if let Some(cleanup) = cleanup {
                        cleanup();
                    }
                    let _ = report.send_blocking(());
                }
            });
        let task = match spawned {
            Ok(thread) => {
                // Detached: the report, not a join, is how the window learns
                // the cleanup finished. Tests keep the handle to wait on it.
                #[cfg(test)]
                self.cleanup_threads.push(Some(thread));
                #[cfg(not(test))]
                drop(thread);
                cx.background_executor().spawn(async move {
                    let _ = reported.recv().await;
                    mark_completed.store(true, std::sync::atomic::Ordering::Release);
                })
            }
            Err(_) => {
                // A machine that cannot start one more thread is better served
                // by a slow cleanup on the shared executor than by a skipped
                // one, which would leave the pane's children running.
                let cleanup = slot.lock().ok().and_then(|mut slot| slot.take());
                cx.background_executor().spawn(async move {
                    if let Some(cleanup) = cleanup {
                        cleanup();
                    }
                    mark_completed.store(true, std::sync::atomic::Ordering::Release);
                })
            }
        };
        self.pending_cleanups
            .push(PendingCleanup { task, completed });
    }
```

`shutdown_and_quit` (lines 66-85) stays unchanged. Its `finished` task only awaits the `PendingCleanup` tasks, which no longer block.

3d. Make the existing gated tests wait for the cleanup thread deterministically. GPUI's `run_until_parked` (vendor/gpui/src/platform/test/dispatcher.rs:191) only runs work already queued and does not wait for a foreign thread. So "open a gate, then `run_until_parked`" now races the cleanup thread. In `close_gate.rs`'s `mod tests`, add this helper directly after `gated_panes` (after line 462):

```rust
    /// Opens one cleanup's gate and waits until its thread has reported, so
    /// the executor then has the report to run rather than racing the thread.
    fn release(
        workspace: &gpui::Entity<Workspace>,
        cx: &mut gpui::VisualTestContext,
        gates: &[async_channel::Sender<()>],
        index: usize,
    ) {
        gates[index].try_send(()).unwrap();
        let thread = workspace
            .update(cx, |workspace, _| workspace.cleanup_threads[index].take())
            .expect("that cleanup's thread was started");
        thread.join().expect("the cleanup thread finished");
        cx.run_until_parked();
    }
```

Then make these replacements. Each `gates[i].try_send(()).unwrap();` + `cx.run_until_parked();` pair becomes one `release` call:

- `last_pane_waits_for_prior_and_final_cleanup_before_quit`: lines 476-477 → `release(&workspace, cx, &gates, 0);`; lines 484-485 → `release(&workspace, cx, &gates, 1);`
- `native_close_keeps_window_until_prior_and_current_cleanup_finish`: lines 507-508 → `release(&workspace, cx, &gates, 1);`; lines 511-512 → `release(&workspace, cx, &gates, 0);`
- `shortcut_quit_keeps_window_until_prior_and_current_cleanup_finish`: lines 533-534 → `release(&workspace, cx, &gates, 1);`; lines 537-538 → `release(&workspace, cx, &gates, 0);`
- `waiting_for_shutdown_refuses_new_tabs_and_splits`: lines 557-560

  ```rust
          for gate in gates {
              gate.try_send(()).unwrap();
          }
          cx.run_until_parked();
  ```
  →
  ```rust
          for index in 0..gates.len() {
              release(&workspace, cx, &gates, index);
          }
  ```
- `queued_reload_cannot_reenable_observation_during_shutdown`: lines 592-595 (same four lines) → the same `for index in 0..gates.len() { release(&workspace, cx, &gates, index); }` loop. The `QUIT_REQUESTS == 1` assertion after it stays.
- `completed_removed_cleanup_is_pruned_before_shutdown`: lines 604-605 → `release(&workspace, cx, &gates, 0);`. Leave lines 614-619 (`gates[1].try_send` then `block_test`) as they are: `gated_panes` already allowed parking, so `block_test` parks until the thread's report wakes it.

Thread order matches gate order in every test. `shut_down` takes gate `i` and pushes thread `i` in the same call: `close_focused_pane` on pane 0 starts thread 0, and the shutdown that follows starts thread 1 for pane 1.

3e. Two tests `block_test` on cleanup tasks without having allowed parking. With cleanup on a real thread, `block_test` must be allowed to park until the report wakes it, or it panics with "parked with nothing left to run":

- `close_gate.rs` `window_shutdown_includes_background_panes_in_identity_order_once`: after `let (workspace, cx) = test_workspace(cx);` (line 628) add `cx.background_executor.allow_parking();`
- `crates/sprite-app/src/workspace/surface_routing.rs` `shutdown_refuses_queued_surface_open`: after `let (workspace, cx) = test_workspace(cx);` (line 159) add `cx.background_executor.allow_parking();`

- [ ] **Step 4: Run tests, confirm pass**

```
TERM=dumb cargo test -p sprite-app --locked --offline --lib workspace::close_gate::tests
TERM=dumb cargo test -p sprite-app --locked --offline --lib workspace::surface_routing::tests
TERM=dumb cargo test -p sprite-app --locked --offline --lib workspace::
```

The new test passes in well under a second. Check the flake fix as well: run `TERM=dumb cargo test -p sprite-app --locked --offline --lib workspace::close_gate::tests` five times in a row and confirm all pass each time.

- [ ] **Step 5: Commit**

```
git add crates/sprite-app/src/workspace/close_gate.rs crates/sprite-app/src/workspace/mod.rs crates/sprite-app/src/workspace/surface_routing.rs
git commit -m "fix(workspace): run each pane cleanup on its own short-lived thread

Blocking HUP/TERM/KILL escalation and joins no longer occupy GPUI's
shared background executor. Each cleanup runs on a sprite-pane-cleanup
thread and reports over a channel the window's pending-cleanup task
awaits, so cleanups stay concurrent and quit still waits for removed and
present panes.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

---

### Task 21: Paint-bench shaping mode, performance record and final verification gates (PRD Evidence)

**Platform text system finding (decides this task's shape).** Shaping through the platform text system cannot be done headless in this GPUI, so the new mode records *counts* through the live shape cache and the record states that shaping timing was not measured. Evidence, all in the vendored crate:
- `vendor/gpui/src/text_system.rs:365` — `shape_line` exists only on `WindowTextSystem`, whose constructor is `pub(crate)` (`:340`) and which is reached only through a `Window`.
- `vendor/gpui/src/text_system.rs:65` — `TextSystem::new` is `pub(crate)`; the `TextSystem` an app exposes (`vendor/gpui/src/app.rs:222`, `Application::text_system`) has font metrics, `advance` and `line_wrapper`, and no line shaping.
- `vendor/gpui/src/platform/mac/text_system.rs:53,75` — `MacTextSystem` and its `new` are `pub(crate)`; `vendor/gpui/src/platform/linux/text_system.rs:22` — `CosmicTextSystem` is `pub(crate)`; `vendor/gpui/src/platform.rs:579` — `PlatformTextSystem` is a `pub(crate)` trait.
- `vendor/gpui/src/app.rs:143-151` — `Application::headless()` "prevents opening windows", so no `Window` (and no `WindowTextSystem`) exists headless; `vendor/gpui/src/platform/test/platform.rs:105` — the test platform's text system is `NoopTextSystem`.

**Files:**
- Modify: `crates/sprite-app/src/grid_paint.rs` (add `GridPaint::benchmark_shaping` after `benchmark_draw_decisions`, line 297)
- Modify: `crates/sprite-app/src/paint_benchmark.rs:1-2` (doc), `:52-74` (`PaintBenchmark` methods), new `ShapingSample`
- Modify: `crates/sprite-app/src/bin/sprite-paint-bench.rs:86-115` (`run`), `:263-299` (`Options`)
- Create: `docs/performance/bug-class-audit.md`, `docs/performance/bug-class-audit-paint-before.json`, `docs/performance/bug-class-audit-paint-after.json`
- Modify (re-freeze): `docs/performance/design-review-paint-shared.json` (the budget file `--check-budgets` is pointed at by `docs/performance/design-review.md:246` and `docs/performance/design-review-coverage.md:287`), and `docs/performance/design-review.md` (append a re-freeze section after the "Shared rows and paint preparation" section, which ends before `### Carried timing gates` at line ~253)
- Test: `crates/sprite-app/src/paint_benchmark.rs` (`mod tests`), `crates/sprite-app/tests/paint_benchmark_report.rs`

**Interfaces:**
- Consumes (Task 16): `ShapeCache::begin_frame`, `ShapeCache::shaped`, `GridPaint::shape_context`, `GridPaint.shapes`, `reaches_text_system`, `PaintBenchmark.shapes`.
- Produces: `pub struct paint_benchmark::ShapingSample { pub glyph_cells: usize, pub shape_calls: usize }`; `pub fn PaintBenchmark::shaping(&mut self, scenario: Scenario, split: bool) -> ShapingSample`; `pub(crate) fn GridPaint::benchmark_shaping(&self, scale: f32) -> (usize, usize)`; bench flag `--shaping` adding top-level `"shaping"` and `"shaping_timing"` report keys (the existing `schema`, `metrics` and budget checks are unchanged).

- [ ] **Step 1: Write the failing tests**

Append to `mod tests` in `crates/sprite-app/src/paint_benchmark.rs`:

```rust
    /// Each transition, counted through the live shape cache. The fixture has
    /// 110 cells per row that reach the text system (6,600 on screen); before
    /// the cache every one of them was shaped on every frame.
    #[test]
    fn shaping_counts_follow_the_shape_cache_through_each_transition() {
        for split in [false, true] {
            let mut benchmark = PaintBenchmark::new();
            let first = benchmark.shaping(Scenario::FirstFrame, split);
            assert_eq!(first.glyph_cells, 6_600);
            assert!(first.shape_calls > 0 && first.shape_calls <= first.glyph_cells);
            assert_eq!(
                benchmark.shaping(Scenario::FirstFrame, split),
                ShapingSample {
                    glyph_cells: 6_600,
                    shape_calls: 0
                },
                "an unchanged frame shapes nothing"
            );
            let blink = benchmark.shaping(Scenario::SameGenerationBlink, split);
            assert!(blink.shape_calls <= 1, "a blink shapes at most the cursor's cell");
            let hover = benchmark.shaping(Scenario::Hover, split);
            assert!(
                (1..=110).contains(&hover.shape_calls),
                "hover reshapes only within row 12: {}",
                hover.shape_calls
            );
            let changed = benchmark.shaping(Scenario::OneRowChange, split);
            assert_eq!(changed.glyph_cells, 6_601, "row 30 gained a glyph");
            assert!(
                (1..=111).contains(&changed.shape_calls),
                "a one-row change reshapes only row 30: {}",
                changed.shape_calls
            );
        }
    }
```

Append to `crates/sprite-app/tests/paint_benchmark_report.rs`:

```rust
#[test]
fn shaping_mode_records_shape_counts_and_says_timing_was_not_measured() {
    let directory =
        std::env::temp_dir().join(format!("sprite-paint-shaping-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let report_path = directory.join("report.json");
    let binary = env!("CARGO_BIN_EXE_sprite-paint-bench");
    let measured = Command::new(binary)
        .args(["--samples", "1", "--shaping", "--output"])
        .arg(&report_path)
        .output()
        .unwrap();
    assert!(measured.status.success(), "{:?}", measured);
    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&report_path).unwrap()).unwrap();
    assert_eq!(report["metrics"].as_object().unwrap().len(), 8);
    assert_eq!(report["shaping"].as_object().expect("a shaping section").len(), 8);
    for pass in ["whole", "split"] {
        let first = &report["shaping"][format!("{pass}_first_frame")];
        assert_eq!(first["glyph_cells"], 6_600);
        assert!(first["shape_calls"].as_u64().unwrap() > 0);
        let blink = &report["shaping"][format!("{pass}_same_generation_blink")];
        assert!(blink["shape_calls"].as_u64().unwrap() <= 1);
        let changed = &report["shaping"][format!("{pass}_one_row_change")];
        assert_eq!(changed["glyph_cells"], 6_601);
        assert!((1..=111).contains(&changed["shape_calls"].as_u64().unwrap()));
    }
    assert!(
        report["shaping_timing"]
            .as_str()
            .unwrap()
            .starts_with("not measured")
    );
    let plain = Command::new(binary).args(["--samples", "1"]).output().unwrap();
    assert!(plain.status.success(), "{:?}", plain);
    let plain: serde_json::Value = serde_json::from_slice(&plain.stdout).unwrap();
    assert!(plain.get("shaping").is_none(), "the default report is unchanged");
    std::fs::remove_dir_all(directory).unwrap();
}
```

- [ ] **Step 2: Run them and confirm they fail**

```sh
TERM=dumb cargo test -p sprite-app --lib --locked --offline paint_benchmark::tests::shaping_counts_follow_the_shape_cache_through_each_transition -- --exact
TERM=dumb cargo test -p sprite-app --locked --offline --test paint_benchmark_report shaping_mode_records_shape_counts_and_says_timing_was_not_measured
```

Expected: the lib test fails to compile — `no method named shaping found for struct PaintBenchmark`, `cannot find struct ShapingSample`; once the lib compiles but before the binary changes, the integration test fails at `assert!(measured.status.success())` with stderr `sprite-paint-bench: unknown argument: --shaping`.

- [ ] **Step 3: Implement**

`crates/sprite-app/src/grid_paint.rs` — after `benchmark_draw_decisions` (line 297):

```rust
    /// Walks every glyph live painting would hand to the text system, through
    /// the same shape cache, and returns how many there were and how many the
    /// cache had to shape. Nothing is actually shaped: GPUI shapes only
    /// through a window, so a default line stands in for each shape.
    pub(crate) fn benchmark_shaping(&self, scale: f32) -> (usize, usize) {
        if self.pass == RowPass::Background {
            return (0, 0);
        }
        let mut shapes = self.shapes.borrow_mut();
        shapes.begin_frame(self.shape_context(scale), self.rows.len());
        let (mut glyphs, mut shaped) = (0, 0);
        for (row, cells) in self.rows.iter().enumerate() {
            for (column, (cell, drawn)) in
                cells.iter().zip(self.resolve_row(row, cells)).enumerate()
            {
                if !reaches_text_system(&cell.text) {
                    continue;
                }
                glyphs += 1;
                let line = shapes.shaped(row, cells, column, drawn.foreground, || {
                    shaped += 1;
                    ShapedLine::default()
                });
                std::hint::black_box(line);
            }
        }
        (glyphs, shaped)
    }
```

`crates/sprite-app/src/paint_benchmark.rs` — module doc (lines 1-2):

```rust
//! Headless access to the terminal's live row preparation and cell decisions.
//! Window layout, image elements, glyph rasterisation and GPU submission are
//! excluded. Shaping is counted through the live shape cache but not timed:
//! GPUI shapes text only through a window, and none can be opened headless.
```

after the `Scenario` impl (line 37):

```rust
/// What one transition asks of the text system.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ShapingSample {
    /// Cells whose text reaches the text system: every one of them was shaped
    /// on every frame before the shape cache existed.
    pub glyph_cells: usize,
    /// Of those, how many the shape cache had to shape.
    pub shape_calls: usize,
}

/// The display scale shaping is counted at; any fixed value will do, since a
/// sample never changes it.
const BENCHMARK_SCALE: f32 = 2.0;
```

and in `impl PaintBenchmark`, after `run`:

```rust
    /// Counts, for one transition, the glyphs live painting hands to the text
    /// system and how many of them the shape cache has to shape.
    pub fn shaping(&mut self, scenario: Scenario, split: bool) -> ShapingSample {
        let (background, text) = self.prepare(scenario, split);
        let pass = text.as_ref().unwrap_or(&background);
        let (glyph_cells, shape_calls) = pass.benchmark_shaping(BENCHMARK_SCALE);
        ShapingSample {
            glyph_cells,
            shape_calls,
        }
    }
```

`crates/sprite-app/src/bin/sprite-paint-bench.rs` — add after `fn main` (line 84):

```rust
/// Why the shaping section has counts and no timings.
const SHAPING_TIMING: &str = "not measured: GPUI shapes text only through a window's text \
    system, and a headless process can open no window; counts come from the live shape \
    cache with a stand-in for each shape";

fn shaping_report() -> Value {
    let mut shaping = serde_json::Map::new();
    for split in [false, true] {
        for scenario in Scenario::ALL {
            // A fresh driver primed the way the allocation samples are, so each
            // count describes that one transition.
            let mut fixture = PaintBenchmark::new();
            if scenario != Scenario::FirstFrame {
                fixture.shaping(Scenario::FirstFrame, split);
            }
            let sample = fixture.shaping(scenario, split);
            shaping.insert(
                metric_name(scenario, split),
                json!({"glyph_cells": sample.glyph_cells, "shape_calls": sample.shape_calls}),
            );
        }
    }
    Value::Object(shaping)
}
```

in `run` change `let report = json!({` to `let mut report = json!({` and insert after the `json!` literal closes (after line 102):

```rust
    if options.shaping {
        report["shaping"] = shaping_report();
        report["shaping_timing"] = json!(SHAPING_TIMING);
    }
```

`Options` — add the field, its default and its flag:

```rust
struct Options {
    samples: usize,
    output: Option<PathBuf>,
    check_budgets: Option<PathBuf>,
    shaping: bool,
}
```

```rust
        let mut result = Self {
            samples: 30,
            output: None,
            check_budgets: None,
            shaping: false,
        };
```

and in the `match argument.to_str()` before the `_ =>` arm:

```rust
                Some("--shaping") => result.shaping = true,
```

- [ ] **Step 4: Run tests, confirm pass**

```sh
TERM=dumb cargo test -p sprite-app --lib --locked --offline paint_benchmark::
TERM=dumb cargo test -p sprite-app --locked --offline --test paint_benchmark_report
TERM=dumb cargo test -p sprite-app --locked --offline --bin sprite-paint-bench
```

- [ ] **Step 5: Commit**

```sh
git add crates/sprite-app/src/grid_paint.rs crates/sprite-app/src/paint_benchmark.rs crates/sprite-app/src/bin/sprite-paint-bench.rs crates/sprite-app/tests/paint_benchmark_report.rs
git commit -m "feat(paint-bench): count shaping through the live shape cache

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

- [ ] **Step 6: Record before and after** — no compilation, tests or other benchmark may run concurrently with the two measurements.

Before (master `a62247e`, in a throwaway worktree; no `git stash`). `vendor/ghostty` is a submodule pinned to the same commit at `a62247e` and on this branch (`ab0b9da9…`, check with the first command), so it is copied rather than fetched, which keeps the build offline:

```sh
cd /Users/davidlee/Projects/Sprite/.worktrees/bug-class-audit
git ls-tree a62247e vendor/ghostty; git ls-tree HEAD vendor/ghostty   # both must print ab0b9da9e88fcb4b0533a1854e84628f663930af
BEFORE="$(mktemp -d)/sprite-a62247e"
git worktree add --detach "$BEFORE" a62247e
rm -rf "$BEFORE/vendor/ghostty"
cp -R vendor/ghostty "$BEFORE/vendor/ghostty"
(cd "$BEFORE" && TERM=xterm-ghostty cargo build -p sprite-app --bin sprite-paint-bench --release --locked --offline)
TERM=xterm-ghostty "$BEFORE/target/release/sprite-paint-bench" --samples 30 \
  --output docs/performance/bug-class-audit-paint-before.json
git worktree remove --force "$BEFORE"
```

After (this branch, with every task applied):

```sh
cd /Users/davidlee/Projects/Sprite/.worktrees/bug-class-audit
TERM=xterm-ghostty cargo build -p sprite-app --bin sprite-paint-bench --release --locked --offline
TERM=xterm-ghostty target/release/sprite-paint-bench --samples 30 --shaping \
  --output docs/performance/bug-class-audit-paint-after.json
```

Extract the table rows for the record:

```sh
for f in before after; do
python3 - "docs/performance/bug-class-audit-paint-$f.json" <<'EOF'
import json, sys
report = json.load(open(sys.argv[1]))
for name, m in report["metrics"].items():
    print(f"| `{name}` | {m['allocations']['max']:,} | {m['bytes']['max']:,} | {m['timing']['median']:.6f} | {m['timing']['p95']:.6f} |")
for name, s in report.get("shaping", {}).items():
    print(f"| `{name}` | {s['glyph_cells']:,} | {s['shape_calls']:,} |")
EOF
done
```

- [ ] **Step 7: Re-freeze the paint budgets.** Cell text is now made at layout, so first frame, hover and one-row change allocate more inside the measured seam than the budgets frozen in `design-review-paint-shared.json` allow (122 / 3 / 3 allocations, budgets 135 / 4 / 4). That file must not be left in a state that fails. Follow the policy in `docs/performance/checkpoint-1.md:102-105`: "Budgets are re-frozen only by rerunning the full 30-sample release benchmark on a recorded machine and updating this file together with the JSON". Run nothing else concurrently.

Record the machine first. Every value goes into both `design-review.md` and `bug-class-audit.md`:

```sh
cd /Users/davidlee/Projects/Sprite/.worktrees/bug-class-audit
uname -sm
sysctl -n machdep.cpu.brand_string 2>/dev/null || lscpu | grep 'Model name'
sysctl -n hw.physicalcpu hw.logicalcpu 2>/dev/null || nproc
rustc --version; cargo --version
git rev-parse --short HEAD
```

Rerun the full 30-sample release benchmark, writing the new frozen report over the shared file. The default mode is used, without `--shaping`, so the file keeps exactly the schema it has today. The original pre-optimisation baseline must still pass, which guards the re-freeze against a real regression; the expected outcome is exit status 0:

```sh
TERM=xterm-ghostty cargo build -p sprite-app --bin sprite-paint-bench --release --locked --offline
TERM=xterm-ghostty target/release/sprite-paint-bench --samples 30 \
  --output docs/performance/design-review-paint-shared.json \
  --check-budgets docs/performance/design-review-paint-baseline.json
```

If that check fails, stop. Do not re-freeze; investigate the regression in the task that caused it.

Check the new frozen budgets against a fresh run. Expect exit status 0, and expect same-generation blink to stay at 0 allocations and 0 bytes. The benchmark asserts the blink figures itself, so a non-zero blink panics:

```sh
TERM=xterm-ghostty target/release/sprite-paint-bench --samples 30 \
  --check-budgets docs/performance/design-review-paint-shared.json
```

Print the new table rows:

```sh
python3 - docs/performance/design-review-paint-shared.json <<'EOF'
import json, sys
report = json.load(open(sys.argv[1]))
for name, m in report["metrics"].items():
    print(f"| `{name}` | {m['allocations']['max']:,} | {m['allocations']['budget']:,} | {m['bytes']['max']:,} | {m['bytes']['budget']:,} | {m['timing']['median']:.6f} | {m['timing']['p95']:.6f} |")
EOF
```

Append this section to `docs/performance/design-review.md`, directly before `### Carried timing gates`, filling every `<…>`:

````markdown
### Paint budgets re-frozen after the bug-class audit

`design-review-paint-shared.json` was re-frozen on <date> at commit <short sha>
by rerunning the full 30-sample release benchmark, per the regression policy in
`checkpoint-1.md`. Machine: <uname -sm>, <CPU model>, <physical>/<logical> cores;
<rustc --version>; Cargo's default release profile; nothing else running. The
run passed `--check-budgets design-review-paint-baseline.json` (the original
pre-optimisation budgets), and a second 30-sample run passed
`--check-budgets design-review-paint-shared.json`.

| Scenario/pass | Allocations max | Allocation budget | Requested bytes max | Byte budget | Median ms | p95 ms |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
<8 rows from the script above>

Why the budgets moved: cell text is now a `SharedString` made when a row is
laid out, not a `String` made each time a cell is painted. Laying out a row now
allocates once per non-ASCII cell (printable ASCII borrows a static string;
the fixture has 50 non-ASCII cells per row), so first frame, hover and one-row
change rose inside this seam. Per-frame paint, which is outside this seam,
fell: `a62247e` made a new string for every one of the 6,600 glyph cells on
every frame, idle frames included, and paint now makes none. Same-generation
blink still allocates nothing. See `bug-class-audit.md`.
````

- [ ] **Step 8: Write `docs/performance/bug-class-audit.md`** — fill every `<…>` from the Step 6 output and the machine you ran on (`uname -m`, `sysctl -n machdep.cpu.brand_string` on macOS or `lscpu` on Linux, `rustc --version`):

````markdown
# Bug-class audit paint record

Before/after record for the paint work in the bug-class audit (PRD
`docs/PRDs/10-09-2026-bug-class-audit.md`, R-R1 and R-R3). Recorded, not
enforced: no budget in this file is a gate.

## What changed in the measured seam

- Cell text is a `SharedString` made once when a row is laid out. Printable
  ASCII borrows a static string; other text is copied once per layout of its
  row. At `a62247e` paint copied every glyph cell's text into a new string on
  every frame, outside this benchmark's measured seam.
- Glyphs are shaped through a per-row shape cache beside the layout cache: a
  cell is shaped again only when its drawn colour differs from the one it was
  shaped in, and the cache is dropped on any font, theme or scale change.
  Shapes are pooled per pane (at most 4,096 distinct shapes).
- Box-drawing strokes and outlines are fixed arrays; drawing one allocates
  nothing.

## Platform text system: timing not measured

GPUI shapes text only through a window's text system
(`WindowTextSystem::shape_line`, `vendor/gpui/src/text_system.rs:365`). Its
constructor and the platform text systems (`MacTextSystem`, `CosmicTextSystem`)
are crate-private, and `Application::headless()` opens no window. Shaping cost
was therefore **not timed**, and no shaping speed-up is claimed. The
`--shaping` mode counts, through the live shape cache, how many cells reach the
text system and how many the cache had to shape.

## Commands

```sh
# before: master a62247e in a throwaway worktree (vendor/ghostty copied, same pinned commit)
TERM=xterm-ghostty <worktree>/target/release/sprite-paint-bench --samples 30 \
  --output docs/performance/bug-class-audit-paint-before.json
# after: this branch
TERM=xterm-ghostty target/release/sprite-paint-bench --samples 30 --shaping \
  --output docs/performance/bug-class-audit-paint-after.json
```

Machine: <arch>, <CPU>, <core count>; <rustc version>; release profile; 30
samples per metric; nothing else running.

## Preparation and decisions (existing seam)

| Scenario/pass | Allocations max (before) | Bytes max (before) | Median ms (before) | p95 ms (before) |
| --- | ---: | ---: | ---: | ---: |
<8 rows from bug-class-audit-paint-before.json>

| Scenario/pass | Allocations max (after) | Bytes max (after) | Median ms (after) | p95 ms (after) |
| --- | ---: | ---: | ---: | ---: |
<8 rows from bug-class-audit-paint-after.json>

Same-generation blink allocates nothing before and after. First frame, hover
and one-row change now allocate once per non-ASCII cell in each row laid out
(the fixture has 50 per row), because that text is made at layout instead of
at paint: layout allocations rose inside this seam. Per-frame paint
allocations, outside it, fell: the paint path this replaces made a new string
for every one of the 6,600 glyph cells on every frame, idle ones included, and
paint now makes none.

## Budgets re-frozen

The budgets in `design-review-paint-shared.json` were re-frozen with the full
30-sample release benchmark on the machine above, per `checkpoint-1.md`'s
regression policy. The new run also passed the original
`design-review-paint-baseline.json` budgets. A second run then passed
`--check-budgets design-review-paint-shared.json`. The new budgets and the
reason they moved are recorded in `design-review.md` under "Paint budgets
re-frozen after the bug-class audit".

| Scenario/pass | Allocations max | Allocation budget | Requested bytes max | Byte budget |
| --- | ---: | ---: | ---: | ---: |
<8 rows: name, allocations max/budget, bytes max/budget from design-review-paint-shared.json>

## Shaping (counts; not timed)

`glyph_cells` is what `a62247e` shaped on every frame of that transition (it
shaped every glyph cell every frame). `shape_calls` is what the shape cache
shapes now.

| Scenario/pass | Glyph cells (= shapes per frame at a62247e) | Shape calls now |
| --- | ---: | ---: |
<8 rows from the "shaping" section of bug-class-audit-paint-after.json>

The same gate is enforced at Sprite's real `shape_line` call site by
`grid_paint::tests::shaping_happens_only_for_cells_whose_drawn_text_changed`
(unchanged frame 0; blink ≤ 1; one-row change only that row; font or theme
change equal to a cold cache).

## Verification gates

| Gate | Result |
| --- | --- |
| `TERM=dumb cargo test --workspace --locked --offline` | <pass / failures> |
| `cargo clippy --workspace --all-targets --locked --offline -- -D warnings` | <pass> |
| `cargo fmt --all --check` | <pass> |
| `sprite-paint-bench --samples 30 --check-budgets docs/performance/design-review-paint-shared.json` (release) | <pass> |

## Native macOS behaviour not executed

These need a real desktop session (a key window, a display, an input method or
the system pasteboard) that the test platform and CI do not provide. They are
covered by tests against GPUI's test platform and the real terminal worker,
but were not exercised on a live macOS desktop:

- Pane Focus following real window activation: switching to another app sends
  focus-out to a program with DECSET 1004 and denies OSC 52 clipboard writes;
  returning sends focus-in.
- The unfocused pane's steady hollow block cursor, and only the focused pane
  blinking, as seen on screen.
- Holding Ctrl+Shift+W, close-tab, quit or split with real keyboard
  auto-repeat: the confirmation is not answered by the repeat, and one split
  is made.
- A real input method commit (for example Kotoeri) disarming a pending
  confirmation.
- Unsafe-paste confirmation against the real macOS pasteboard, including the
  clipboard changing between the two pastes.
- On a Retina (scale 2) display: faint text at half strength, selected hidden
  text showing the selection, a cursor on a wide character's second column
  drawn over both columns, and Kitty Unicode-placeholder images without seams
  between tiles.
- Shaping cost through CoreText (not timed; see above).
- Quitting with many busy panes: cleanup runs on per-pane threads and the
  window closes without stalling other background work.
````

- [ ] **Step 9: Final gates** — run each and paste the outcome into the record's table:

```sh
cd /Users/davidlee/Projects/Sprite/.worktrees/bug-class-audit
TERM=dumb cargo test --workspace --locked --offline
cargo clippy --workspace --all-targets --locked --offline -- -D warnings
cargo fmt --all --check
```

Then, after the last code change and with nothing else running, re-check the frozen paint budgets against the final tree. Expect exit status 0:

```sh
TERM=xterm-ghostty cargo build -p sprite-app --bin sprite-paint-bench --release --locked --offline
TERM=xterm-ghostty target/release/sprite-paint-bench --samples 30 \
  --check-budgets docs/performance/design-review-paint-shared.json
```

All four must succeed. A failure is fixed in the task that introduced it and re-run, not recorded as accepted. If a code change landed after Step 7, the budgets are not edited by hand: re-run Step 7 in full.

- [ ] **Step 10: Commit the record and the re-frozen budgets**

```sh
git add docs/performance/bug-class-audit.md docs/performance/bug-class-audit-paint-before.json docs/performance/bug-class-audit-paint-after.json docs/performance/design-review-paint-shared.json docs/performance/design-review.md
git commit -m "docs(performance): record the audit's paint work and re-freeze paint budgets

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

---

# Appendix: drafter notes

Findings recorded while drafting against the code. Decisions taken on them are already reflected in the tasks and the PRD; the executor should still check the listed assumptions as they come up.

## Notes for Tasks 1–4

**(a) PRD requirements that are impossible or wrong against the code**

- None outright impossible. Three clarifications:
  - R-C2.1 says "every relayed request", but ADR 0028 names only configuration reload and print. `relay` has four more callers, all in `surface/channel.rs` (Surface `Open`, and `one_shot` for `Capabilities`, `FocusPane`, `RegisterToken`). Task 2 covers them all. To keep "refusal semantics unchanged on the wire", Surface connections still answer `this window did not answer in time` both when the request was abandoned and when it was claimed but slow. That text never says "nothing was changed", so R-C2.2 holds. A claimed but slow Surface request now waits up to 5 s + 10 s instead of 5 s.
  - R-C1.3 drops the old status-line report of a failed history capture. `HistoryFailed` now only fails its own request; it does not also update the status line. If the status line should still show capture failures, add `effects.push(Effect::Status(...))` to the `HistoryFailed` arm and update `a_failed_capture_fails_only_its_own_request`.
  - Resolved by coordinator decision: when a session ends, its waiters are failed at once through `WindowPanes::fail_all`, called from the view's stop path (Task 1 second cycle). They no longer sit out the 500 ms deadline. Accepted: a failed capture no longer reaches the status line.

**(b) Assumptions the executor should double-check**

- Task 1 test: `TerminalCommand::Select` at row 500 on a 24-row screen makes `terminal.grid_ref` return `Err(InvalidValue)`, so the worker emits `Error(selection_grid_ref: ...)`. The libghostty-vt 0.2.2 docs for `track_grid_ref` state this for out-of-range points, and `grid_ref` presumably behaves the same. The test asserts the status line contains `selection_grid_ref` so that a silent no-op cannot pass. If it fails on that assertion, use another error the worker emits deterministically.
- Task 1 test relies on two things: the `view.update` closure runs without ticking the GPUI executor, and `VisualTestContext::run_until_parked` delivers cross-thread wakeups of the view's event task. The existing tests in `terminal_view/tests.rs` use the same pattern.
- Task 2: `pub struct Relayed` lives in the private module `workspace::reload` and is reached by the public `SurfaceRequest` through the `pub type Reply` alias. Because the type is nominally `pub`, there should be no `private_interfaces` warning. Check that `cargo clippy --all-targets -D warnings` agrees. If not, move `Relayed` into a `pub` module.
- Task 2: `Claim::claim` uses `Err(Self::CLAIMED)` as a pattern inside `matches!` (associated const in a pattern). This should compile on 1.97. If it does not, compare with `==` instead.
- Task 3: the child test needs both new panes to answer within the 500 ms `broker::DEADLINE` while `block_test` pumps the foreground. Newly spawned `/bin/sh` panes answer `CaptureHistory` straight from the worker, so this should be quick. If it is flaky, raise nothing in production: assert only on the listed pane ids from `value["panes"]` plus `value["failures"]`.
- Task 4: on macOS, as on Linux, `accept` must fail with `EMFILE` and leave the connection queued (POSIX). The CPU threshold (200 ms over a 600 ms window) assumes `RUSAGE_SELF` in a child running only this test. `timeval` field widths differ by platform, hence the `as u64` casts. `File::open("/dev/null").unwrap().as_raw_fd()` reads the fd of a temporary that is closed at the end of the statement. That is intended: it is the lowest free number.
- Task 4: the child process is started with `--test-threads=1` so that no other test thread allocates descriptors while they are exhausted.

**(c) Cross-task conflicts (same code edited by tasks outside 1-4)**

- `crates/sprite-term/src/worker/mod.rs`. Task 1 edits the `CaptureHistory` arm (564-582). R-T1 (drain queued messages before capture), BCA-26 (`Select` arm, tracked anchor) and R-T2 (title coalescing) edit the same `handle` match. Hunks are local, but merge order matters.
- `crates/sprite-app/src/terminal_events.rs` and `terminal_view.rs::apply`. Task 1 changes `Effect::DeliverHistory` / `FailRequest` and the `Error` arm. BCA-14 (hover link on change) touches `HyperlinkResolved` handling in `apply`. R-C3/R-C4 may add effects or touch `apply`.
- `crates/sprite-app/src/terminal_view.rs:77-78` (Task 3 comment) is near fields that R-R2 (window clock) and R-C4 (Pane Focus) will add or change.
- `crates/sprite-app/src/surface/channel.rs::serve_surface` / `one_shot`. Task 2 changes the relay arms. R-S1 (writer thread, `SurfaceConnection` no longer exposing the stream) rewrites `handle.establish` / `handle.abandon` / `refuse` in the same functions. The test-only `reply: reply.into()` edits in `terminal_view/surfaces.rs` and `workspace/surface_routing.rs` will touch the same test bodies R-S1 may edit.
- `crates/sprite-app/src/workspace/close_gate.rs`. Task 2 changes one test line (578). Task 3's test awaits `Workspace::begin_shutdown(cx) -> Vec<gpui::Task<()>>`. R-W6 (cleanup on dedicated threads) may change that return type; if so, adjust the last block of Task 3's test.
- `crates/sprite-app/src/workspace/reload.rs::reload` / `Settings.font.size`. BCA-19 (zoom held separately from `settings.font.size`) changes reload's font handling. Task 2's GPUI test reads `workspace.settings.font.size` as the "was the file applied" signal, which should still hold after BCA-19 (the file value lives in `settings.font.size`). Re-check if BCA-19 moves it.
- `crates/sprite-app/src/local_socket.rs`. Only Task 4 edits the accept loop. R-S1 does not touch the listener, but edits nearby connection handling in `surface/channel.rs`, not here.

## Notes for Tasks 5–8

**(a) PRD requirements found wrong or impossible against the code**

- R-R2 "Input resets the phase as today": no such reset exists at `a62247e`. Coordinator decision: drop it. No reset on input is added, and the PRD is being corrected. Task 8 resets the phase only on a change of Pane Focus.
- R-C3.5 "focus movement, divider nudge and font zoom": Task 5 reads these as `Focus(_)`, `Resize(_)`, `FontLarger` and `FontSmaller`. `FontReset`, `CycleFocus` (terminal ⇄ Surface), `NextTab` and `PreviousTab` do **not** repeat. The coordinator accepted this classification.
- Guard tests (accepted by the coordinator, and labelled GUARD in their steps): Task 5 Cycle C (repeat table), Task 7 Cycle A (two sprite-term seam tests) and Task 8's rewritten `fallback_titles_…`. Every task still has tests that fail at master. Task 5: Cycle A and the four Cycle B GPUI tests. Task 6: both tests. Task 7: the Cycle B app tests and the Cycle C admission test. Task 8: the grid-paint test, the clock test and the notification-count test.

**(b) Assumptions the executor should double-check**

- Pane Focus uses `focus.contains_focused(...)`, so a hosted Surface that has the keyboard still counts as Pane Focus for its pane: OSC 52 is allowed, the child is told focus-in, and the cursor blinks. The glossary says "holds keyboard focus in its Sprite Window", and `focus_active_pane` (`keymap.rs:59-68`) already treats Surfaces as part of the pane's focus subtree. If the terminal's child should be told focus-out while a Surface has the keyboard, change `contains_focused` to `is_focused` in `refresh_pane_focus`.
- GPUI focus listeners fire only at the end of a `draw` (`vendor/gpui/src/window.rs:1941-1969`). The workspace focuses the active pane in a `defer_in` from `render`, so the workspace tests draw twice. Activation observers fire immediately on `activate_window` / `deactivate_window` followed by `run_until_parked` (the test platform activates through a foreground task, `platform/test/platform.rs:197-217`). Test windows start inactive (`TestWindow::is_active` returns `false`).
- Focus admission (coordinator decision): `Focus` goes through `admit_focus`, using the same Command Admission pattern as `pending_settings` and `pending_resize`. A refusal keeps the latest value in `pending_focus` and `spawn_retry` resubmits it. `told_focus` stops a change that is undone before delivery from sending anything. The Cycle C test fills the queue with `ClearSelection` submissions (bounded at 64) after the title-burst stall, then gains Pane Focus by calling `refresh_pane_focus` inside the same synchronous update. Running the GPUI executor would drain the mailbox and free the queue. Check that `focus.contains_focused` is true there: it reads the rendered frame drawn before the gate, plus `window.focus`, which `window.focus(&view.focus)` sets synchronously.
- `may_close` passes `is_held = false` to `Confirmation::answer`, because `key_down` drops held repeats for every non-repeating action before dispatch, and a native title-bar close is never held. If a future caller can deliver a held event, thread `event.is_held` through `close_focused_pane` / `close_active_tab` / `quit`.
- The workspace's `capture_any_mouse_down` sits on the root `div`. A click on the confirmation banner itself may not reach it, because the banner `occlude()`s, so the root hitbox may not count as hovered there. The banner's own handlers stop propagation. No test depends on banner clicks; the test clicks a pane with the **right** button, because a left click on a pane already reset the mode through `focus_pane` at master (`Tabs::focus_pane` returns `true` for the already-focused pane).
- Documented limit (accepted by the coordinator): workspace bindings (`capture_key_down` on the workspace root with `stop_propagation`) never reach the pane, so pressing font zoom while a paste is held does not drop the hold. Split, new tab, tab switch and focus moves do drop it, because they take Pane Focus away (Task 7). IME commits drop a held close question only through the key-downs that precede them; the workspace has no IME hook.
- GPUI reports modifier-only presses as `ModifiersChanged`, never `KeyDown` (for example, X11 returns early for `keysym.is_modifier_key()`, `platform/linux/x11/client.rs:~1000`). The pane's and workspace's key-down disarm rules rely on this.
- `test_workspace` panes run `/sprite-test-command-does-not-exist`. Existing workspace tests rely on these panes persisting, and Task 7's workspace test does too. It only needs them to be `TerminalView`s, whether running or failed.
- The `unsafe_paste` / `pane_focused` / `_snapshots` / `blink_on` fields are read directly by tests in `terminal_view/tests.rs`. That is a child module, so private access is fine. `_snapshots = Task::ready(())` copies the isolation technique the existing `fallback_titles_…` test already uses.
- The `dd … status=none` and `od -An -tx1 -v` flags are already used by passing sprite-app PTY tests on both platforms. The Task 6 test uses `stty raw` (blocking `min 1`): if too few bytes arrive, `wait_for_bundle`'s 1 s condition timeout fails the test instead of hanging it, and `begin_shutdown` reaps the child.

**(c) Cross-task conflicts (same code edited by tasks outside 5–8)**

- `terminal_view.rs` `TerminalView` struct, `new` and `failed`: Tasks 6, 7 and 8 each edit fields and initialisers. BCA-11 (registration at creation) edits the `observation` registration in `new`. BCA-14 (hover request-on-change) edits the snapshot task and `request_hover_link`. BCA-06 (tickets) edits `apply` (`DeliverHistory` / `FailRequest`). Apply hunks by anchor, not by line number, after earlier tasks land. Task 7 Cycle C also edits `spawn_retry` and `begin_shutdown`; any task that adds another pending admission value, or BCA-22's backlog change, should merge into the same `Some(… || …)` expression.
- `terminal_view/render.rs` `Render::render`: Task 6 adds capture listeners next to `track_focus`. BCA-26 (selection anchor) and BCA-14 edit the mouse handlers in the same chain. R-R1 (shaped-text cache) edits `prepare_rows` / `GridPaint::prepare` call sites in the same function.
- `grid_paint.rs` `GridPaintSpec` / `GridPaint::new` / `draw`: Task 8 adds `focused`. R-R1 (per-row shaped cache, `PositionedCell.text: SharedString`), BCA-12 (faint), BCA-13 (wide-tail cursor drawn two cells wide) and BCA-28 (hidden + selected) touch the same struct, constructor and `draw`. Every `GridPaintSpec { … }` literal (prepare, benchmark, surface render, tests) must gain `focused`. BCA-13's two-cell cursor must also respect the `BlockHollow` mapping (an outline across both cells).
- `workspace/keymap.rs`: Task 5's held-zoom assertion reads `workspace.settings.font.size`. BCA-19 moves zoom out of `settings.font.size`, so whichever task lands second must point that assertion at the new zoom value. `adjust_font` / `apply_font_size` are edited by BCA-19 as well.
- `workspace/close_gate.rs`: R-W6 (cleanup threads) rewrites `shut_down`, `shutdown_and_quit` and `PendingCleanup` in the same file. Task 5 touches only `may_close`, `PendingClose` and `CloseGate`. The Task 5 quit test relies on `BusyPane::begin_shutdown` returning `None`, so it quits without any cleanup thread.
- `workspace/mod.rs` `Workspace` struct / `new`: Tasks 5 (`_activation`) and 8 (`_clock`). BCA-11 / R-W1 (pane registration) and BCA-19 (zoom field) also add fields here.
- `sprite-pane/src/lib.rs`: Task 8 adds `tick` to both traits. Any other task that adds a `Pane` member must keep the blanket `PaneHandle` impl in step.
- `terminal_events.rs`: Task 6 adds `PASTE_HELD_NOTICE` near the `UnsafePaste` arm. BCA-06 rewrites the `History` / `Error` arms in the same `decide`.

## Notes for Tasks 9–12

### (a) PRD requirements found impossible or wrong against the code

1. **R-T2 breaks thirteen existing event-pressure fixtures; the PRD does not mention it.** These tests flood titles to put many lossless events in front of a paused consumer, and several assert exact title counts. Once titles are coalesced, a flood is one event per pass. The tests are: `sprite-term/tests/event_backpressure.rs` (4 tests: 80/100 titles; `shutdown_retains_…` asserts `retained.len() == 80`); `sprite-app/src/terminal_view/submission_regressions.rs:21,135,214,368,489` (wait for `TITLE99` at :67); `sprite-app/src/observation/panes.rs:293`; `sprite-app/src/terminal_view/tests.rs:882,1066`; `sprite-app/src/terminal_view/theme.rs:442`. Task 9 Cycle B moves them to OSC 52 clipboard writes (coordinator: accepted). Those are the only parser notice that is never coalesced, and they need focus first, so each fixture now waits for a cue (`read _`) or a gate. Bare-session fixtures send `Focus(true)`; view-owned ones get real Pane Focus via Task 6's `focus_and_draw`. Two fixtures cannot be focused: `theme.rs` must run before the first snapshot, and Task 7's `focus_admission_pressure_child` must stay unfocused. They stall the worker with `CopySelection` commands instead, since each produces one lossless `SelectionCopied`. That makes 14 fixtures in total, counting Task 7's. This is the largest piece of Task 9. The executor must run every listed app test, not just sprite-term.
2. **BCA-26 with zero scrollback cannot "clear rather than re-anchor".** With `max_scrollback == 0`, Ghostty scrolls by rotating rows (`vendor/ghostty/src/terminal/Screen.zig:893-920` → `PageList.eraseRow`). `PageList.zig:4450-4456` only moves pins *below* the erased row, so a pin on row 0 stays put. It is never marked `garbage`, so `has_value()` stays true and the anchor silently lands on the next line. Page pruning (`PageList.zig:3586-3595`) marks pins garbage, so the PRD behaviour holds for any nonzero budget. The eviction test therefore uses `scrollback_bytes = 4 KiB`. A zero-scrollback pane still re-anchors, which is a residual gap. Fixing it would need either a libghostty change (out of scope per ADR 0003) or a worker heuristic.
3. **The brief's suggested `start: bool` on `Select` does not fit the app.** The press never sends a `Select`: `Drag::moved` was designed that way so a click is not a selection. The first `Select` comes on the first motion, possibly after output has already scrolled, so pinning at that point would still drift. Task 11 adds `TerminalCommand::BeginSelection { anchor }`, which replaces the press's `ClearSelection`. A character-mode `Select` uses the pinned anchor whenever one is held (from `BeginSelection` until `ClearSelection` or the next `BeginSelection`) and ignores its own `anchor` field. Word and line modes are unchanged; they use only `head`.
4. **R-T1 "on macOS the pump reads until EAGAIN"** is implemented on every platform. On Linux it costs one extra non-blocking `read` per wake, and in return CI tests it, using a datagram socket that returns one datagram per read the way a macOS PTY returns one slice.
5. **The BCA-24 integration test only discriminates on Linux.** When a macOS session leader exits, the kernel revokes the controlling tty (BSD `exit1` → `VOP_REVOKE`), so the lingering descendant sees EOF even before the fix. `foreground::tests::detaching_closes_the_private_duplicate` discriminates on both platforms. Linux sends only SIGHUP to the pty foreground group on leader exit (no vhangup for PTY slaves), so the test there depends on the duplicate being closed.
6. **ADR text drift (now Task 9 Cycle C step 3f).** ADR 0021 says "A parser chunk transfers its reply failure, ordered notices, and clipboard writes together", and ADR 0026 says something similar. After Task 9 the unit is a *pass* of at most 16 messages / 16 KiB. Notices are published before any non-output message, so ordering relative to commands is preserved. Task 9 appends a one-line amendment to each ADR.
7. **BCA-27 scope decision.** A changed identity between the two BSD readings (pid reuse, `setpgid`) still leaves the scan incomplete, as ADR 0025 requires ("only ESRCH proves disappearance"). Only `ESRCH` at any of the three probes becomes "gone". `finish_shutdown` starts at `Escalation::Terminate`: the retained scope arrives after a natural close that already owed its single HUP. If that close never managed a complete scan, the HUP is not retried later. This is a narrow residual gap.
8. **BCA-07 limit semantics tightened.** The storage limit is now checked against the RGBA size libghostty actually stores, not the PNG crate's output size, so a gray image is measured at 4 B/px.

### (b) Assumptions the executor should double-check

- `Terminal::on_pwd_changed` fires for `ESC ] 7 ; file:///tmp/two BEL` in libghostty-vt 0.2.2, and `pwd()` then contains `/tmp/two`. `tests/integration_metadata.rs` only proves the snapshot's working directory. If the callback reports a bare path, the `contains("/tmp/two")` assertion still holds.
- `crate::event_mailbox::bounded(0)` is a valid zero-room mailbox: every `publish` parks until the receiver pops. The coalescing fixture relies on this to decide when the snapshot slot is empty.
- `Allocator::GLOBAL`'s `Bytes` might not come from the Rust heap. The retention test tolerates either case. The refused-image test assumes png 0.18.1's `read_info` makes no single allocation of 256 KiB or more (its fdeflate state is tens of KiB).
- png 0.18.1 `Encoder` accepts 1/2/4-bit grayscale and indexed images with `set_palette(Vec<u8>)`, and expands low-depth gray by scaling (2-bit `2` → `0xAA`, 4-bit `8` → `0x88`). `ColorType` and `BitDepth` implement `PartialEq` and `Debug`.
- `Terminal::set_selection(None)` takes `&self` (the existing `apply_selection` calls `set_selection` through `&Terminal`). `TrackedGridRef` is `!Send`. That is fine because `Owned` is created and dropped on the worker thread.
- The GPUI test window shows fewer than about 300 rows, so `seq 1 300` scrolls. `Pixels * f32` is valid, as the existing pointer tests already use it.
- `UnixDatagram` sockets work under the pump unchanged: `fcntl` non-blocking, `poll` `POLLIN`.
- macOS `libc::proc_bsdinfo` field types: `pbi_pid`/`pbi_pgid`/`pbi_status` are `u32` and `pbi_start_tv*` are `u64`, as the existing code assumes. Not compiled on Linux.

### (c) Cross-task conflicts

- **Task 1 (history tickets)** edits the `CaptureHistory` arm. Task 9 Cycle C renames `Session::handle`'s body to `Session::apply` without touching that arm's text, so after Task 9 the arm lives in `apply`. If Task 1 lands first, Task 9's hunks still apply by context (they touch only the function header, the destructuring, the `PtyOutput` arm and the tail). If Task 1 adds fields to `Session` or changes `emit`, re-check the `apply` destructuring in Task 9 3b and Task 11 3c.
- **Task 7 (Pane Focus / `Focus` command)** may edit the worker's `Focus` arm (inside `apply` after Task 9) and `start.rs:18-22`. Task 9 edits `start.rs:90-124` and Task 11 edits `start.rs:182-187`, so there is no textual overlap. Task 9 runs after Task 7 and depends on its wiring (see Task 9 Consumes): the view-spawned fixtures rely on Task 7 delivering `Focus(true)` once real Pane Focus is gained, and Task 9 migrates Task 7's `focus_admission_pressure_child` burst. Task 7's own drafter note ("fills the queue … after the title-burst stall") is superseded by Task 9 (ix).
- **The confirmation/disarm task (R-C3.2: a mouse button press disarms)** will likely edit the same left-button `on_mouse_down` handler in `crates/sprite-app/src/terminal_view/render.rs:473-504` that Task 11 changes (`ClearSelection` → `BeginSelection`).
- **The R-W6 pane-cleanup-thread task** calls `ShutdownHandle::wait` → `worker::finish_shutdown`. Task 12 changes that function's body but not its signature.
- Within this set, Tasks 9, 11 and 12 all edit `worker/mod.rs`, and are written to apply in order 9 → 11 → 12. Task 12 adds `Runtime.foreground`, and Task 9's `coalescing_tests::Fixture` already passes a `ForegroundWatch` to `run`, so they are compatible.

## Notes for Tasks 13–15

### (a) PRD requirements that are impossible, wrong, or need a decision

Coordinator decisions (2026-10-09) are folded in; items marked *decided* need no further action.

1. *Decided — never joined.* The writer is detached and provably ends: at once when the connection dies; otherwise, once the last handle is dropped, after draining a finite queue (at most the bound plus one event), with every write that stops progressing failing after `WRITE_TIMEOUT`. The code comment in `SurfaceConnection::new` says this; the guard `dropping_the_last_handle_never_waits_for_a_stalled_writer` checks it. The connection thread cannot join instead, because it usually returns before the window drops its clone (the window still has to send `closed`).
2. *Decided — ADR 0029 is being corrected to 64* (`surface/channel.rs:40`); nothing in Tasks 13-15.
3. *Decided — admission rule.* An event is admitted whenever fewer than `MAX_PENDING_BYTES` are already pending (queued plus being written), so the most ever pending is 4 MiB plus one event and a single event — such as a large Surface paste (`terminal_view/surfaces.rs:1228-1235`) — is never refused for its own size. Tests: `pre_open_gestures_…` (second half: a 4 MiB line on an empty queue is admitted, the next send dies), `a_single_event_larger_than_the_bound_is_delivered_whole` (5 MiB paste delivered), and `a_large_batch_…` keeps its 2 MiB lines.
4. *Decided — O(log k) relink.* A `BTreeMap<stamp, name>` per id keeps the arrival-order layering `style_for` depends on; no relink touches another id or scans the grid.
5. *Decided — batch semantics for the caps.* An over-cap highlights operation is validated inside `GridSurface::apply` (the `Op::Highlights` arm calls `highlights_fit` before any mutation), so the operation itself changes nothing. Earlier operations in the batch stand, and `apply_all` prefixes the refusal with `op N: `, exactly as in `a_refusal_inside_a_batch_names_the_operation_that_failed`. The test `a_highlights_operation_past_either_cap_is_refused_before_it_changes_anything` asserts all three. R-S2's "a message … is refused whole" is read as "the operation is refused whole".
6. *Decided — key on SVG text only.* Element images are decoded at intrinsic size (`render_svg(svg, None)`), so there is no scaled-size component. This departs from R-S4's wording ("hash of SVG text plus scaled pixel size"). The virtual list's own icon cache (`list_view.rs` `sync_image_cache`, keyed by asset id, reset on scale change) is separate and untouched.
7. *Decided — no collisions.* The cache is `HashMap<Arc<str>, Option<Arc<RenderImage>>>`: the map hashes the text and compares the stored text on every hash hit, so two SVGs can never share a picture. Cost: one copy of each distinct SVG text per Surface, bounded by the description size. The unit test asserts the cache holds exactly the drawn texts (`cached_texts()`). A forced hash collision is not tested; std `HashMap`'s key comparison is relied on.
8. **BCA-17 test shape.** The brief suggests "mouse up → click names b". GPUI cannot do that. It drops a pending press whose element is no longer under the pointer (div.rs:2224-2231), so with key ids the correct outcome is *no* click. The test asserts that no click names another row, with a positive control first. If the GPUI harness turns out not to deliver row clicks (the control fails), fall back to the `a_rows_element_id_follows_its_key_not_its_position` unit test alone and say so in the PR.

### (b) Assumptions the executor should double-check

- `impl Write for &UnixStream` (std) is used for the writer's writes; `SO_SNDTIMEO` set on the writer's cloned descriptor applies to the socket, as it did before.
- SIGPIPE is ignored in Rust test binaries (the runtime sets `SIG_IGN`); existing channel tests already write to closed peers.
- Writing to a Unix stream socket whose peer has closed fails at once with `EPIPE` on both Linux and macOS (`a_failed_write_leaves_later_batches_unconsumed`, `a_closed_peer_stops_further_sends`, `the_writer_stops_at_the_first_failed_write` depend on it).
- `a_backpressured_batch_keeps_its_written_prefix…` and `dropping_the_last_handle…` assume a 2 MiB line overflows the socket buffer on both platforms (true at master too).
- The 100-line graceful-close test assumes its ~3 KB fits a socket buffer or is read promptly. It is read right after the drop, so either way works.
- `window.draw(cx)` in `draw_test_window` swaps in the drawn frame, so `simulate_mouse_*` hit-tests against it (existing click tests in `terminal_view/tests.rs` rely on this). The test window is tall enough that rows 0-3 under the fixture's 35 px heading and 22 px section are inside it.
- In `TerminalView::failed`, a fill list Surface is drawn and its rows accept clicks (the 100k probe test proves it draws; the control assertion proves clicks).
- `serde_json` has `preserve_order` (workspace Cargo.toml:28), which the name-order assertions in Task 14 rely on.
- The grid cap test builds 262,144 attrs and 262,144 names directly (no JSON) and clones the grid once. It should take well under a few seconds in a debug build. Check that the time is acceptable on CI.
- `#[cfg(test)] fn finish(&mut self)` on `Queue` has an empty body in non-test builds. Clippy's default set does not flag it; confirm with the all-target clippy run.

### (c) Cross-task conflicts

- **Task 15 depends on Task 13**'s test helpers (`Peer`, `events(&mut Peer)`, `open_request` returning `Peer`) in `terminal_view/surfaces.rs` tests. Run 13 before 15. If 15 is ever run first, change `list_clicks`'s parameter to `&mut std::os::unix::net::UnixStream` (master's `events` signature); nothing else in Task 15 depends on Task 13.
- `terminal_view/surfaces.rs` is edited by Task 13 (`close_surface` doc, test helpers, wheel test) and Task 15 (`replace_description`, legacy SVG test, two new tests). The regions don't overlap. Other tasks likely to touch this file: the Pane Focus / Confirmation work (R-C4.1, R-C3.2 — key, mouse and IME handlers including the Surface key handler around lines 1205-1260), and R-R2 (a grid Surface's cursor follows the window clock — `GridMetrics.blink_on` and `render_grid` in `surface/render.rs`, beside Task 15's cache edits).
- `surface/grid.rs` is edited by Task 14 (struct fields, `apply_all`, `apply`, `Op::Highlights`, `style_for`). R-R1 (`PositionedCell.text` becomes a `SharedString` created at row layout) is expected to edit `GridSurface::lay_out` (grid.rs:727-748) in the same file; the regions don't overlap, but `style_for` is called from `lay_out`.
- `surface/channel.rs` `serve_surface` may also be touched by C2 / R-C2 (relay `Waiting | Claimed | Abandoned`) work: `relay(...)` and its `RelayError::Timeout` arm are at channel.rs:654-685, next to Task 13's comment edit at :647-651. The PRD's R-C2 applies to "every relayed request", which includes the Surface `Open` relay here.

## Notes for Tasks 16–18, 21

### (a) PRD requirements that are impossible or wrong as written, with evidence

1. **R-R1 "per-cell `(drawn foreground, ShapedLine)`" taken literally would cost tens of MB per pane.** `ShapedLine` holds `SmallVec<[DecorationRun; 32]>` inline (`vendor/gpui/src/text_system/line.rs:30-38`); at roughly 90 bytes per `DecorationRun` that is about 3 KB per line. One per glyph cell on a fully inked 200×60 screen is about 36 MB. Task 16 keeps the per-row, per-cell slot `(Rgba, Arc<ShapedLine>)` the PRD describes, but the `Arc` points into a per-pane pool keyed by `(text, bold, italic, enlarged, colour bits)` and capped at 4,096 entries. Every count gate still holds (pool hits only lower the counts). Worst case remains truecolour output where every cell is unique: the row slots then hold up to one shape per visible glyph cell until those rows change. Reviewers should accept or tighten that.
2. **"A mode that includes shaping through the platform text system" is impossible headless without patching vendored GPUI.** Evidence is in Task 21's preamble. The PRD's fallback ("the record says timing was not measured") is what Task 21 does. It also records shape counts, so the before/after still says something.
3. **BCA-13 "mark the cursor as wide so paint draws it two cells wide" needs no new flag.** Paint attaches the cursor to the cell whose `column` equals `cursor.column` (`grid_paint.rs:343`) and draws it within that cell's span bounds (`:599-609`). A wide cell's span is already two columns (`grid.rs:72-75`). Mapping `at_wide_tail` to the lead column is therefore enough. Adding a `wide` field to `sprite_term::CursorSnapshot` would touch 9 literal sites across both crates for no behaviour.
4. **Faint is applied in `draw`, not `cell_colors`.** `cell_colors`' foreground is also the selection, inverse and block-cursor *fill* (`grid_paint.rs:376-380`), so halving it there would make those grounds translucent. Hidden text (BCA-28) moved to `draw` for the same reason: only `draw` knows about selection. The two existing `cell_colors` invisible tests are rewritten to assert through `draw`, with the same expectations.
5. **Making text paint-ready at layout raises allocation counts inside the existing bench seam.** First frame, hover and one-row change go up by about 50 per laid-out fixture row, because the fixture has 50 non-ASCII cells per row. Meanwhile paint stops making 6,600 strings per frame, which happens outside the seam. Without a change, the committed `design-review-paint-shared.json` budgets for those three metrics would fail `--check-budgets`. **Coordinator decision:** Task 21 Step 7 re-freezes that file per `checkpoint-1.md`'s policy: a full 30-sample release rerun on a recorded machine, which must also still pass the original `design-review-paint-baseline.json`. It records the machine and the reason in `design-review.md`, and then runs `--check-budgets` against the new file, expecting success. Step 9 repeats that check on the final tree.

### (b) Assumptions the executor should double-check

- `CUP` onto the second column of a wide character (`\x1b[3;2H` after `界` at column 0) leaves libghostty's cursor on the spacer tail with `CursorViewport.at_wide_tail == true` (`libghostty-vt-0.2.2/src/render.rs:495-505`, `vendor/ghostty/src/renderer/generic.zig:3219-3230`). If libghostty clamps the cursor to the lead instead, Task 18 Cycle B's test passes at master; then find another way to put the cursor on a tail (for example print a wide character at the last column and check `SpacerHead`), and keep the mapping.
- GPUI test windows paint `GridPaint` through `NoopTextSystem` and still call `shape_line` (existing TerminalView paint tests rely on this). `window.draw(cx)` re-renders the root view after `window.refresh()` without a notify.
- `Entity::condition` is notification-driven with a 1 s timeout (`vendor/gpui/src/app/test_context.rs:600-633`). That is why Task 17's "no repaint" phase polls with `settle_hover` instead. The blink clock advances only via `advance_clock`, so no blink notify can land inside the observed window.
- In Task 17's test, the first `window.draw` may resize the grid (full redraw, so every row gets a new `Arc`). The test waits for a bundle at `view.size` before it hovers. If the test platform's window gives a grid whose `size` never matches, check `synchronise_size`/`admit_resize` in `terminal_view/geometry.rs`.
- Task 18 Cycle C assumes GPUI's taffy lays out in device pixels with rounding (`vendor/gpui/src/taffy.rs:41`, `to_taffy` scales by `scale_factor`). Under that assumption, a tile whose relative offset and size are whole device pixels lands exactly at `round(grid_box) + snapped offset`. That is where `GridPaint` snaps the same cell, because its bounds origin is the rounded grid box. The test checks only the geometry; seam-freedom on screen is in the unexecuted-on-macOS list.
- `GlyphTarget` passes `&mut ShapeCache` inside a by-value struct, and `target.shapes.shaped(..)` is called through a non-`mut` binding. That compiles (it is a reborrow through a `&mut` field). Do not add `mut`, because `unused_mut` fails clippy.
- After Task 16 Cycle A, `surface_allocation_probe` (`surface_performance.rs:107,115`) should keep its `a <= 1` and `a <= 8` bounds, because the grid Surface's `TextPool` now hands out shared text. Confirm this; if it fails, do not raise the bound. Find the allocation instead.
- The interner cap (4,096) and `FAINT_OPACITY = 0.5` are constants chosen here. 0.5 is the PRD's value; the cap is mine.
- The fixture-derived numbers in Task 21's tests come from `paint_benchmark::fixture()`: 6,600 glyph cells, 6,601 after the row-30 change, and 110 or 111 per row. If Task 18's faint change or any other task alters the fixture, recompute them with `reaches_text_system`.

### (c) Cross-task conflicts (same code edited by tasks outside 16/17/18/21)

- **`GridPaintSpec` / `GridPaint` struct literals:** Task 8 adds `focused`, Task 16 adds `shapes`, at the same sites: `grid_paint.rs` (`prepare`, `prepare_spec`, `new`, test literal at :1315), `paint_benchmark.rs:93`, `surface/render.rs:224`. Task 18's test helper and Task 16's probe set both fields.
- **`GridPaint::prepare` signature:** Task 8 may add a focus argument and Task 16 appends `shapes`. The call in `terminal_view/render.rs:315` must carry both.
- **`GridPaint::draw`:** Task 8 (unfocused cursor becomes hollow/steady, likely touching `here`/`is_block`) and Task 18 (fill, ground and ink block). Task 18's hunk starts at `let fill = match self.pass {` and leaves the lines above it to Task 8.
- **`GridPaint::paint` glyph loop:** Task 16 rewrites the `paint_glyph` call next to `paint_cursor`, which Task 8 may change. Keep Task 8's `paint_cursor` line.
- **`TerminalView::apply`:** Task 1 rewrites the history arms (`DeliverHistory`/`FailRequest`, and possibly new ticketed variants). Task 17 makes `apply` return `bool`. Whichever lands second must end each of Task 1's arms with `true`.
- **TerminalView constructors (`new` at :363-410, `failed` at :453-504):** Task 7 and Task 8 (focus state, window clock), Task 16 (`shape_cache`) and Task 17 (`hover_basis`) all add field initialisers.
- **`terminal_view/render.rs::render`:** Task 8 (cursor filter / blink, R-R2 window clock may also remove `tick_blink`'s timer path), Task 16 (`prepare` argument) and Task 18 (`image_layers` scale argument).
- **Snapshot task in `TerminalView::new` (:322-351):** Task 17 replaces the hover re-request. Any task that changes snapshot delivery (R-T1 is in `sprite-term`, so probably none) would collide here.
- **`crates/sprite-term/src/snapshot.rs`:** Task 18 edits `cursor_snapshot` only. Any task touching capture/coalescing (R-T1/R-T2) edits other functions in the same file.
- **`docs/performance/`:** Task 21 adds new files. It also rewrites `design-review-paint-shared.json` (the re-freeze) and appends one section to `design-review.md`. Any other task that re-runs the paint benchmark or edits those two files must come before Task 21 Step 7, or Step 7 must be repeated.

## Notes for Tasks 19–20

### (a) PRD requirements found impossible or wrong against the code

- **a1. BCA-21 was drafted as a piecewise rule; review found it pinned 240–280 px splits and the user replaced it with the PRD's smooth `min(floor, extent / 4)` (2026-10-09). The text below describes the superseded piecewise draft.** The PRD formula would also lower the floor for splits from 240 to 480 px and break the existing 400 px divider tests. Under the chosen rule, a split of `2 * floor` (240 px) or more keeps the full floor as today. Only a smaller split uses `extent / 4`. The one cost is a jump at 240 px. A split of 239 px moves within [0.25, 0.75], but at exactly 240 px it is pinned at 0.5 as before, and just above 240 px it moves only within a narrow middle band. This is accepted as decided. The PRD's BCA-21 row should be updated to match.
- **a2. R-W6's "a separate background_executor task still runs" test cannot discriminate under GPUI's test dispatcher.** `TestDispatcher` runs every background runnable on the test thread and has no fixed pool. The gated cleanups at a62247e also `.await` their gate rather than block, so such a test passes before and after the fix. The plan instead asserts the thread name (`sprite-pane-cleanup`), not the test thread, one distinct thread per cleanup, and that all N cleanups block at the same moment (a 5 s rendezvous). The last point proves they are concurrent and never serialised on one executor thread. It fails at a62247e in about 10 s without hanging.
- **a3. The existing quit-waits-for-cleanup tests cannot "keep passing" unchanged.** Their "open gate → `run_until_parked` → assert" sequence assumes cleanup runs on the test dispatcher. `run_until_parked` (`vendor/gpui/src/platform/test/dispatcher.rs:191`) never waits for a foreign thread, so with real threads those assertions would race. Their assertions are kept word for word. Only the wait changes: the `release` helper joins the cleanup thread before `run_until_parked`. Two tests that `block_test` on cleanup tasks also need `allow_parking()`.

### (b) Assumptions the executor should double-check

- **b1.** `cx.simulate_keystrokes("ctrl-shift-=")` / `"ctrl-shift-0"` reach `Workspace::key_down` as `FontLarger` / `FontReset`. The `"="`/`"+"` and `"0"`/`")"` spellings both map, per keymap.rs:154-156. Neighbouring tests drive `ctrl-shift-d`/`w`/`t` the same way after `draw_workspace`. If the keystroke does not land, Step 2 of cycle A says to fall back to calling `adjust_font` / `reset_font` directly.
- **b2.** Zoom semantics when the *file's* `font.size` changes while zoomed: the plan keeps the zoom, so reset goes to the new file size, and an unzoomed window follows the file. This is the literal PRD wording ("reload … keeps zoom") and Ghostty's behaviour as I recall it, not verified. The alternative is to drop the zoom when the file's size changes. The second test in cycle A pins the chosen behaviour; change it there if the owner prefers the alternative.
- **b3.** Cycle B goes slightly beyond the brief: a *discovered* default file that exists but cannot be read (for example a directory, or no permission) now complains too. Only absence stays silent, which is how I read the PRD's "Discovery of an absent default file stays silent". `config print --config <path>` (client.rs:166) is treated as explicit as well. Both are separable: drop `a_discovered_file_that_is_there_but_cannot_be_read_is_a_complaint` and keep the old `load_from` body to revert the first. The binary-level test uses `config print --config`, because `sprite --config` opens a real window, which CI cannot do. The window path (`main.rs:81`) shares `load_explicit` and is covered by the config-level tests.
- **b4.** `std::fs::read_to_string` on a directory returns an error (EISDIR) on both Linux and macOS. The permission test returns early when running as root (CI containers), because root can read a mode-000 file.
- **b5.** `async-channel` is pinned at `=2.5.0` (Cargo.toml:14), which has `Receiver::recv_blocking` and `Sender::send_blocking`; the latter is already used in the repo. `std::thread::current().name()` returns the full Rust-side name even though Linux truncates the OS-level name to 15 bytes.
- **b6.** `shut_down`'s spawn-failure fallback runs the cleanup on the shared executor, which is the pre-change behaviour. This is a deliberate exception to R-W6's "never on the shared executor", taken only when the OS refuses a thread. The alternative is dropping the cleanup, which skips `worker::finish_shutdown`'s TERM/KILL escalation and leaves children running. It cannot be tested, because a thread spawn cannot be made to fail. If reviewers want R-W6 literally, replace the `Err(_)` arm with dropping the cleanup and marking it complete.
- **b7.** `#[cfg(not(test))] drop(thread);` assumes clippy's `drop_non_drop` does not fire on `JoinHandle<()>`. It has drop glue, so it should not. If it does, use `let _detached = thread;` instead.
- **b8.** Line numbers are at a62247e. HEAD `5e2ed14` differs only in docs and `crates/CONTEXT.md`, so they match the worktree today.

### (c) Cross-task conflicts (same function or file edited outside Tasks 19-20)

- `workspace/mod.rs` `struct Workspace` and the `Self { … }` initializer in `Workspace::new`: Task 19 (field `font_zoom`) and Task 20 (field `cleanup_threads`) both edit them, as will any task adding window state, such as R-R2's per-window 530 ms clock or C3's `Confirmation`. Apply by context, not line number.
- `workspace/keymap.rs`: Task 19 edits only the font functions (26-58). The C3/BCA-31 task (`WorkspaceAction::repeats()`, disarming confirmations) will edit `key_down` (172-238) and the `WorkspaceAction` enum in the same file. They are different functions, but merge carefully.
- `workspace/reload.rs`: Task 19 edits `reload()` (65-84) and appends tests. The BCA-08 task (relay `Waiting | Claimed | Abandoned`) edits `relay`/`ReloadRequest` (104-133), and possibly `reload()`'s caller in mod.rs:239-255.
- `workspace/close_gate.rs`: Task 20 edits `shut_down` and the tests module (`gated_panes` neighbours and six existing tests). R-C3.3 (close/quit confirmation through `Confirmation<T>`) edits `may_close`/`CloseGate`/`PendingClose` and may edit tests in the same module.
- `workspace/surface_routing.rs` test `shutdown_refuses_queued_surface_open`: Task 20 adds one `allow_parking()` line. The R-S1 tasks (Surface writer thread) may touch this test file.
- `crates/sprite-app/tests/client.rs`: Task 19 inserts one test after line 517. Other CLI or Surface tasks may also add tests there.
- `ActiveSettings` publication: R-R1 (shape cache invalidated on font or theme change) and R-C4 consume `ActiveSettings`. After Task 19, any code that publishes settings or builds a pane from the workspace must use `Workspace::active_settings()`, not `self.settings`, or zoom will be lost.

