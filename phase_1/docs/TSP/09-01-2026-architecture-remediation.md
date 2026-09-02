# Sprite Architecture Remediation Technical Spec

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> dmi-superpowers:subagent-driven-development (recommended) or
> dmi-superpowers:executing-plans to implement this TSP task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the modules that hold Sprite's untested behaviour reachable by
tests, and remove the duplication and dead surface that hid two defects.

**Architecture:** Eight structural changes, no behaviour changes. Each task
either narrows an interface, extracts a pure decision from a GPUI-bound one, or
folds duplicated code together. Four of the eight are justified entirely by a
testability claim and must land the test they promise — that is their acceptance
criterion, not a nicety.

**Tech Stack:** Unchanged. Rust 2024, `libghostty-vt 0.2.1`, `gpui =0.2.2`.
No dependency is added, removed, or upgraded by this TSP.

## Global Constraints

- **Offline and locked.** Every cargo invocation uses `--locked --offline`.
- **Clippy is a gate.** `cargo clippy --workspace --all-targets --locked
  --offline -- -D warnings` must pass **before every commit**. This matters more
  than usual here: Task 1 deliberately turns on warnings that were previously
  suppressed by a public re-export, and a task that leaves one unresolved cannot
  commit.
- **Formatting.** `cargo fmt --all -- --check` before every commit.
- **Never add `#[allow(dead_code)]`.** If the compiler says something is unused,
  either delete it, gate it with `#[cfg(test)]`, or give it the caller it was
  always meant to have. Silencing it defeats the entire point of Task 1.
- **No behaviour changes.** Every task here must leave the existing suite green
  without modifying an existing assertion. If a test needs changing to pass, stop
  — that is a behaviour change and it belongs in a different branch.
  **One sanctioned exception:** Task 9 Step 3 moves `placements` and `pixels`
  from dropping at end of scope to dropping with the `Projector`, ahead of
  `terminal`. That is a deliberate, reviewed change of observable drop order and
  is not a breach of this constraint. It is the *only* one; if any other task
  finds itself changing behaviour, the constraint applies and the task stops.
  Task 9's own "if a test fails, record it and stop" instruction remains the
  safety net for that exception.
- **Linux first.**

## Prerequisite

`09-01-2026-defect-fixes.md` must be merged first. Task 6 below builds directly
on the `respond(&dyn PaneSource, …)` signature and the `protocol_check` function
that TSP introduces.

## Source of truth

The PRD is `phase_1/docs/PRD/08-27-2026-architecture-remediation.html`. Read its
"Implementation plan" section first. Where the PRD's older card text disagrees
with that section, the plan wins. In particular: **C2 is dropped** and **C10 is
dissolved** into Tasks 2 and 6 — do not implement them as cards.

---

### Task 1: Narrow the public surface, and let the compiler find the rest

Every method on `PaneRegistry`, `PaneTree` and `Tabs` is `pub`, and each type is
re-exported from `lib.rs`. That public path is why `dead_code` never fired: an
item reachable from any `pub` export is "used" as far as the compiler is
concerned. Of 46 exported names, 24 have an external user; 22 do not.

**This task is instrumentation, not tidying.** Narrowing the exports made the
compiler report three items that two careful manual read-throughs of this
codebase missed, one of which is a real defect. Run it first so that every task
after it is designed against the true inventory.

**Files:**
- Modify: `crates/sprite-app/src/lib.rs` (the `pub use` block)
- Modify: `crates/sprite-app/src/workspace.rs` (delete `observation_enabled`)
- Modify: `crates/sprite-app/src/tabs.rs` (delete `active_mut`, `get`)
- Modify: `crates/sprite-app/src/pane_registry.rs` (delete `get_mut`, `focused_mut`, `len`)
- Modify: `crates/sprite-app/src/pane_tree.rs` (document `is_empty`)
- Modify: `crates/sprite-app/src/graphics_cache.rs` (gate `len`, `is_empty`, `used_bytes`)
- Modify: `crates/sprite-app/src/observation/panes.rs` (give `deliver_failure` its caller)
- Modify: `crates/sprite-app/src/terminal_view.rs` (call it)
- Modify: `phase_1/docs/adr/0012-use-gpui-for-the-application-shell.md`

**Interfaces:**
- Consumes: nothing.
- Produces: a `lib.rs` export list of exactly 24 names. Task 6 relies on
  `PaneSource`, `PaneAddress`, `Pending`, `parse as parse_request` and
  `collect as collect_panes` remaining exported.

- [ ] **Step 1: Narrow the exports**

Replace the entire `pub use` block in `crates/sprite-app/src/lib.rs` with:

```rust
pub use cli::{Invocation, USAGE, WindowArgs, parse_arguments};
pub use config::Settings;
pub use observation::broker::{
    Failure, FailureKind, PaneAddress, PaneReport, PaneSource, Pending, Report,
    collect as collect_panes, parse as parse_request,
};
pub use observation::client::{run_config_print, run_config_reload, run_snapshot};
pub use observation::endpoint::Endpoint;
pub use observation::schema::render as render_schema;
pub use pane_tree::{PaneId, Rect};
pub use sprite_term::HistoryLines;
pub use tabs::TabId;
pub use workspace::Workspace;
```

Leave `pub mod config;` and the `mod` declarations above it untouched.

- [ ] **Step 2: Run check and read the inventory**

Run:
```bash
cargo check -p sprite-app --all-targets --locked --offline
```
Expected: **compiles**, with exactly these warnings:

```
warning: methods `len`, `is_empty`, and `used_bytes` are never used
   --> crates/sprite-app/src/graphics_cache.rs:140:12
warning: method `deliver_failure` is never used
   --> crates/sprite-app/src/observation/panes.rs:150:12
warning: methods `len`, `get_mut`, and `focused_mut` are never used
  --> crates/sprite-app/src/pane_registry.rs:36:12
warning: method `is_empty` is never used
   --> crates/sprite-app/src/pane_tree.rs:249:12
warning: methods `active_mut` and `get` are never used
  --> crates/sprite-app/src/tabs.rs:69:12
```

If the crate does **not** compile, an export was removed that something still
needs — add just that name back and note it in the commit message. Do not add
back the whole block.

Note what is *not* in this list: `Workspace::observation_enabled`. `Workspace`
stays public, so its methods remain formally reachable and the compiler cannot
see that one. Steps 3–7 handle each warning, and Step 4 handles the invisible one.

- [ ] **Step 3: Delete the genuinely dead methods**

In `crates/sprite-app/src/tabs.rs`, delete `active_mut` (:69) and `get` (:77) in
full, including their doc comments.

In `crates/sprite-app/src/pane_registry.rs`, delete `get_mut` (:48) and
`focused_mut` (:56) in full. `get_mut`'s only caller was `focused_mut`, which is
itself dead.

`len` (:36) is **gated, not deleted** — the same treatment Step 6 gives
`GraphicsCache`. Eight existing assertions call it (six here, two in `tabs.rs`
via `tabs.active().len()`), so deleting it would force editing test assertions,
which the global constraints forbid. `#[cfg(test)]` removes it from the
production surface just as effectively.

- [ ] **Step 4: Delete the method the compiler cannot see**

In `crates/sprite-app/src/workspace.rs`, delete `observation_enabled` (:185).
Verify it has no callers first:

```bash
grep -rn "observation_enabled" crates/
```
Expected: only the definition. Then delete it.

- [ ] **Step 5: Keep `PaneTree::is_empty` — it is correct, not aspirational**

**Do not delete this one.** Returning the literal `false` is right: `PaneTree::new`
seeds one leaf, and `close` refuses to remove the last (`pane_tree.rs:270`,
`if self.len() == 1 { return None; }`). A `PaneTree` is never empty by invariant.
`len` is live at that same line, so deleting `is_empty` would trip clippy's
`len_without_is_empty`.

**Compute it, and give it one real call site.** A literal `false` would be
*correct* — the tree is never empty — but it cannot be checked, and an assertion
built on it asserts nothing. Since `PaneTree` loses its `lib.rs` export, `dead_code`
also needs a non-test caller, and `len` has a live production caller so `is_empty`
cannot be `#[cfg(test)]`-gated without waking `clippy::len_without_is_empty`.

```rust
    /// Whether the tree holds no panes.
    ///
    /// Always false in practice: `new` seeds one leaf and `close` refuses to
    /// remove the last, so no sequence of operations empties a tree. Computed
    /// rather than returned as a constant so that the assertion in `close` is a
    /// real check — a constant would make it assert nothing.
    pub fn is_empty(&self) -> bool {
        self.panes().is_empty()
    }
```

and in `close`, immediately before returning:

```rust
        // The tree keeps its final leaf, so a close can never empty it. Checked
        // here rather than trusted: this is the function that maintains the
        // invariant, so it is the function that can break it. Debug-only, so
        // the traversal costs release builds nothing.
        debug_assert!(!self.is_empty(), "close emptied the tree");
```

- [ ] **Step 6: Gate the cache's test observers**

`GraphicsCache::len`, `is_empty` and `used_bytes` are used only by that module's
own tests (`graphics_cache.rs:334–420`). **Deleting them breaks the build.**
Gate them instead:

```rust
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    #[cfg(test)]
    pub fn used_bytes(&self) -> usize {
        self.used
    }
```

Keep any doc comments already attached to them.

- [ ] **Step 7: Give `deliver_failure` the caller it was written for**

This is **defect 5** from the PRD, and it must be fixed here rather than in
Task 2, because `-D warnings` will not let this task commit with an unused
method. Task 2 then carries the behaviour into `decide` unchanged.

`deliver_failure` is the error counterpart to `deliver`. Nothing calls it, so a
pane that genuinely fails never tells the observation waiter; the requester
waits out the full 500 ms `DEADLINE` and is reported as `FailureKind::Timeout`,
which is the wrong reason. `FailureKind::Errored` is unreachable in production
as a result.

In `crates/sprite-app/src/terminal_view.rs`, in the `TerminalEvent::Error` arm
(around :340), add the failure delivery before the status update:

```rust
                    Ok(TerminalEvent::Error(error)) => {
                        // A pane in a bad state must not leave an observation
                        // request waiting out the deadline. Any session error
                        // fails an in-flight request: the pane cannot answer,
                        // and the reason it cannot is this one.
                        if let Some(link) = &event_link {
                            link.panes.deliver_failure(link.pane, error.to_string());
                        }
                        if view
                            .update(cx, |view, cx| {
                                view.status = Some(error.to_string().into());
                                cx.notify();
                            })
                            .is_err()
                        {
                            return;
                        }
                    }
```

- [ ] **Step 8: Verify the inventory is clean**

Run:
```bash
cargo check -p sprite-app --all-targets --locked --offline
cargo test --workspace --locked --offline
cargo clippy --workspace --all-targets --locked --offline -- -D warnings
cargo fmt --all -- --check
```
Expected: **no `never used` warnings remain**, all tests pass, clippy clean.

- [ ] **Step 9: Amend ADR-0012 with the real figure**

`phase_1/docs/adr/0012-use-gpui-for-the-application-shell.md:78` bounds the risk
at *"about 600 lines across three files"*. Measure the truth:

```bash
cd phase_1/crates/sprite-app/src
for f in $(find . -name '*.rs'); do grep -q "gpui" "$f" && echo "$(wc -l < $f) $f"; done | sort -n
```
Expected at time of writing: seven files, 4,615 lines. Re-measure rather than
copying that number — this task has just deleted code.

Append to the ADR, below the existing "Why the risk is bounded" section:

```markdown
## Amendment, September 2026

The bound above was wrong, and recording it is the point of an ADR.

`sprite-app` names `gpui` in **seven files totalling 4,615 lines**, against the
"about 600 lines across three files" this decision was accepted on — roughly
7.7 times the budgeted figure. No `observation/` module touches GPUI, so the
`sprite-term` seam that made the risk acceptable has held exactly as described;
what grew is the shell itself.

The decision is not reversed. GPUI remains the right choice for the reasons
given above, and the restructure analysis is unaffected. But a future reader
should not take the 600-line figure as a live constraint that Sprite is
meeting, and anyone re-running that restructure cost estimate should scale it
accordingly.
```

- [ ] **Step 10: Commit**

```bash
git add crates/sprite-app/src/ phase_1/docs/adr/0012-use-gpui-for-the-application-shell.md
git commit -m "Let the compiler police the application's public surface"
```

---

### Task 2: Cut the event pump out of the Pane view

`terminal_view.rs` is 1,581 lines and 23 fields. Its event pump — thirteen
`match` arms — sits inside `cx.spawn(async move |view, cx| …)`, so nothing in it
can run without a live GPUI `Window`. All seven of the file's inline tests
exercise `grid_size`, a free function. **Zero** touch the pump.

Reading all thirteen arms shows the pump needs no state at all: every arm only
*writes* view state, none reads it. So the decision half is a pure function and
the fields stay exactly where they are. This is a better outcome than the PRD's
original "pump absorbs 3 of 23 fields" — the view's field count does not change.

This task also carries **defect 3**: `Self::failed` spawns a real `/bin/sh` on a
real PTY purely to fill the `session` field, then kills it, and `.expect()`
panics the whole window if that fork fails — on the error path, where the user's
configured shell has *already* failed.

Note: defect 3 is **not** caused by the extraction. The pump never touches
`session`. They are batched because they are the same file and the same review.

**Files:**
- Create: `crates/sprite-app/src/terminal_events.rs`
- Modify: `crates/sprite-app/src/lib.rs` (add `mod terminal_events;`)
- Modify: `crates/sprite-app/src/terminal_view.rs` (loop, `session` field, `Self::failed`)

**Interfaces:**
- Consumes: `sprite_term::{TerminalEvent, SessionError, HistorySnapshot}`,
  `gpui::SharedString`.
- Produces:
  - `pub(crate) enum Effect { Status(SharedString), HoldPaste(String), OpenUrl(String), Clipboard(String), DeliverHistory(HistorySnapshot), FailRequest(String) }`
  - `pub(crate) struct Decision { pub effects: Vec<Effect>, pub stop: bool }`
  - `pub(crate) fn decide(event: Result<TerminalEvent, SessionError>) -> Decision`

- [ ] **Step 1: Write the failing tests**

Create `crates/sprite-app/src/terminal_events.rs` with only the test module and
the type declarations it needs, so the tests fail on behaviour rather than on
missing names. Write the full file in Step 3; for now write this:

```rust
//! What a terminal event asks the view to do.

use gpui::SharedString;
use sprite_term::{HistorySnapshot, SessionError, TerminalEvent};

/// One thing an event asks the view to do.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Effect {
    Status(SharedString),
    HoldPaste(String),
    OpenUrl(String),
    Clipboard(String),
    DeliverHistory(HistorySnapshot),
    FailRequest(String),
}

/// What one event implies, and whether the stream is finished.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Decision {
    pub effects: Vec<Effect>,
    pub stop: bool,
}

pub(crate) fn decide(_event: Result<TerminalEvent, SessionError>) -> Decision {
    todo!("Step 3")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn effects(event: TerminalEvent) -> Vec<Effect> {
        decide(Ok(event)).effects
    }

    #[test]
    fn events_with_no_presentation_ask_for_nothing() {
        for event in [
            TerminalEvent::Ready,
            TerminalEvent::Bell,
            TerminalEvent::WorkingDirectoryChanged(None),
            TerminalEvent::Hyperlink {
                position: Default::default(),
                uri: None,
            },
        ] {
            assert!(effects(event).is_empty());
        }
    }

    #[test]
    fn a_held_paste_explains_itself_and_is_kept() {
        let held = effects(TerminalEvent::UnsafePaste("one\ntwo\n".to_owned()));
        assert_eq!(held.len(), 2, "a held paste both holds and explains");
        assert!(matches!(held[0], Effect::HoldPaste(ref text) if text == "one\ntwo\n"));
        assert!(matches!(held[1], Effect::Status(ref line) if line.contains("2 lines")));
    }

    #[test]
    fn an_allowed_link_is_opened() {
        let opened = effects(TerminalEvent::Hyperlink {
            position: Default::default(),
            uri: Some("https://example.invalid/".to_owned()),
        });
        assert!(matches!(opened.as_slice(), [Effect::OpenUrl(uri)] if uri == "https://example.invalid/"));
    }

    #[test]
    fn empty_clipboard_writes_are_not_performed() {
        assert!(effects(TerminalEvent::ClipboardWrite(String::new())).is_empty());
        assert!(effects(TerminalEvent::SelectionCopied(String::new())).is_empty());
    }

    #[test]
    fn clipboard_writes_carry_their_text() {
        assert!(matches!(
            effects(TerminalEvent::ClipboardWrite("copied".to_owned())).as_slice(),
            [Effect::Clipboard(text)] if text == "copied"
        ));
        assert!(matches!(
            effects(TerminalEvent::SelectionCopied("selected".to_owned())).as_slice(),
            [Effect::Clipboard(text)] if text == "selected"
        ));
    }

    /// Defect 5: a pane that fails must say so, not leave a requester waiting
    /// out the observation deadline.
    #[test]
    fn an_error_both_reports_and_fails_the_waiter() {
        let raised = effects(TerminalEvent::Error(SessionError::new("read", "broke")));
        assert_eq!(raised.len(), 2);
        assert!(matches!(raised[0], Effect::FailRequest(_)));
        assert!(matches!(raised[1], Effect::Status(_)));
    }

    #[test]
    fn a_finished_stream_stops_the_loop() {
        assert!(decide(Err(SessionError::new("ended", "closed"))).stop);
        assert!(decide(Ok(TerminalEvent::Ready)).effects.is_empty());
        assert!(!decide(Ok(TerminalEvent::Ready)).stop);
    }

    /// Nothing sets a window title, so the event has no presentation. The old
    /// arm called `cx.notify()` for it, repainting once per shell prompt.
    #[test]
    fn a_title_change_asks_for_nothing() {
        assert!(effects(TerminalEvent::TitleChanged(Some("x".to_owned()))).is_empty());
    }
}
```

Add `mod terminal_events;` to `crates/sprite-app/src/lib.rs` beside the other
`mod` declarations.

Check the real variant shapes before running — `TerminalEvent::Hyperlink`'s
`position` field type and `WorkingDirectoryChanged`'s payload come from
`crates/sprite-term/src/lib.rs`. Adjust the constructors above to match; do not
change what the tests assert.

- [ ] **Step 2: Run the tests to verify they fail**

Run:
```bash
cargo test -p sprite-app --locked --offline --lib terminal_events
```
Expected: **FAIL**, every test panicking at `not yet implemented: Step 3`.

- [ ] **Step 3: Write `decide`**

Replace the `todo!` with the full mapping. Each arm is the body of the
corresponding arm in `terminal_view.rs:249–361`, with the `view.update` and
`cx` calls replaced by the effect they performed:

```rust
/// One event in, the effects it implies out.
///
/// Pure on purpose. Every arm of the old pump only *wrote* view state, never
/// read it, so there is nothing to own here — which is what lets all thirteen
/// arms be tested without a GPUI Window.
pub(crate) fn decide(event: Result<TerminalEvent, SessionError>) -> Decision {
    let mut effects = Vec::new();
    let mut stop = false;

    match event {
        // Nothing to present. Working directory and bell are carried for
        // observation and for a future bell policy; a title change has no
        // presentation because nothing sets a window title.
        Ok(TerminalEvent::Ready)
        | Ok(TerminalEvent::Bell)
        | Ok(TerminalEvent::WorkingDirectoryChanged(_))
        | Ok(TerminalEvent::TitleChanged(_))
        | Ok(TerminalEvent::Graphics(_))
        // No link, or a refused scheme. Indistinguishable on purpose.
        | Ok(TerminalEvent::Hyperlink { uri: None, .. }) => {}

        Ok(TerminalEvent::UnsafePaste(text)) => {
            // Held, not performed. The person sees why and repeats the paste.
            let lines = text.lines().count();
            effects.push(Effect::HoldPaste(text));
            effects.push(Effect::Status(
                format!(
                    "[paste held: {lines} lines would run as commands — \
                     press Ctrl+Shift+V again to paste anyway]"
                )
                .into(),
            ));
        }

        // Terminal Core already applied the scheme policy, so reaching here
        // means the target is allowed. Sprite never builds a command line from
        // terminal-provided text.
        Ok(TerminalEvent::Hyperlink { uri: Some(uri), .. }) => {
            effects.push(Effect::OpenUrl(uri));
        }

        // Belongs to whoever asked for it. The view forwards because it is the
        // single consumer of this session's events, and arrival order is what
        // lets the registry pair answers with waiters.
        Ok(TerminalEvent::History(history)) => {
            effects.push(Effect::DeliverHistory(history));
        }

        // Terminal Core already applied the OSC 52 policy for one and the
        // person asked for the other; neither needs a policy here.
        Ok(TerminalEvent::ClipboardWrite(text)) | Ok(TerminalEvent::SelectionCopied(text)) => {
            if !text.is_empty() {
                effects.push(Effect::Clipboard(text));
            }
        }

        Ok(TerminalEvent::Error(error)) => {
            // The waiter first: a pane in a bad state must not leave an
            // observation request waiting out the deadline.
            effects.push(Effect::FailRequest(error.to_string()));
            effects.push(Effect::Status(error.to_string().into()));
        }

        Ok(TerminalEvent::Exited(exit)) => {
            effects.push(Effect::Status(crate::terminal_view::describe_exit(&exit).into()));
            stop = true;
        }

        // After the session ends the stream simply closes. That is completion,
        // not a new failure to report.
        Err(_) => stop = true,
    }

    Decision { effects, stop }
}
```

**Move `describe_exit` into this module**, above `decide`, keeping it private.
It is pure presentation text with no view state, and the arm in `decide` is its
only caller. Leaving it behind would force it to widen to `pub(crate)` and leave
this Window-free module depending on the 1,500-line view it was extracted from —
unreadable and unmovable without it, which defeats the point of the extraction.

- [ ] **Step 4: Run the tests to verify they pass**

Run:
```bash
cargo test -p sprite-app --locked --offline --lib terminal_events
```
Expected: **PASS**, `8 passed`.

- [ ] **Step 5: Commit the pure half *together with* Step 6's rewiring**

Do Step 6 before committing. The two cannot be separated: until the view calls
`decide`, it is `pub(crate)` code with no non-test caller, so `dead_code` fires
and `-D warnings` refuses the commit — and `#[allow(dead_code)]` is forbidden.
Commit once the pure function and its caller are both in place.

```bash
git add crates/sprite-app/src/terminal_events.rs crates/sprite-app/src/lib.rs \
        crates/sprite-app/src/terminal_view.rs
git commit -m "Decide what a terminal event means without a Window"
```

- [ ] **Step 6: Replace the pump with `decide` plus `apply`**

In `crates/sprite-app/src/terminal_view.rs`, replace the whole
`match events.next().await { … }` block (`:248–362`) with:

```rust
            loop {
                let decision = crate::terminal_events::decide(events.next().await);
                if !decision.effects.is_empty() {
                    let applied = view.update(cx, |view, cx| {
                        for effect in decision.effects {
                            view.apply(effect, cx);
                        }
                        // One notify for the batch: an event that asked for
                        // nothing does not repaint.
                        cx.notify();
                    });
                    if applied.is_err() {
                        return;
                    }
                }
                if decision.stop {
                    return;
                }
            }
```

Then add `apply` to `impl TerminalView`:

```rust
    /// Performs one decided effect. Everything here needs `cx`; nothing here
    /// decides anything.
    fn apply(&mut self, effect: crate::terminal_events::Effect, cx: &mut Context<Self>) {
        use crate::terminal_events::Effect;
        match effect {
            Effect::Status(line) => self.status = Some(line),
            Effect::HoldPaste(text) => self.pending_unsafe_paste = Some(text),
            Effect::OpenUrl(uri) => cx.open_url(&uri),
            Effect::Clipboard(text) => cx.write_to_clipboard(ClipboardItem::new_string(text)),
            Effect::DeliverHistory(history) => {
                if let Some(link) = &self.observation {
                    link.panes.deliver(link.pane, history);
                }
            }
            Effect::FailRequest(reason) => {
                if let Some(link) = &self.observation {
                    link.panes.deliver_failure(link.pane, reason);
                }
            }
        }
    }
```

`apply` reads `self.observation` rather than the captured `event_link`, so the
`let event_link = observation.clone();` line above the spawn becomes unused —
delete it.

This supersedes Task 1 Step 7: the `deliver_failure` call now lives in
`Effect::FailRequest`. Delete the version Task 1 added to the old `Error` arm —
it goes away with the arm.

- [ ] **Step 7: Fix defect 3 — a failed view owns no session**

Change the field in `struct TerminalView`:

```rust
    session: Option<TerminalSession>,
```

In `Self::failed`, delete the shell spawn entirely:

```rust
    /// A view that shows why it could not start. It owns no session, so its
    /// streams are already-closed no-ops.
    fn failed(message: String, font_family: SharedString, cx: &mut Context<Self>) -> Self {
        Self {
            session: None,
            // A view that never started a session has nothing to observe.
            observation: None,
```

Leave the rest of that constructor's fields as they are. In `Self::new`, wrap
the session: `session: Some(session),`.

Then fix every remaining `self.session` use. Find them:

```bash
grep -n "self\.session" crates/sprite-app/src/terminal_view.rs
```

For each, the failed view should do nothing rather than panic. The two shapes
you need:

```rust
        // Sending to a view with no session is a no-op, not an error: a failed
        // pane has nothing to send to.
        let Some(session) = self.session.as_mut() else {
            return;
        };
```

and, where a value is required:

```rust
        let Some(session) = self.session.as_ref() else {
            return None;
        };
```

`begin_shutdown` must return `None` for a failed view rather than panicking.

- [ ] **Step 8: Verify nothing regressed**

Run:
```bash
cargo test --workspace --locked --offline
cargo clippy --workspace --all-targets --locked --offline -- -D warnings
cargo fmt --all -- --check
```
Expected: all green, with no existing assertion modified.

- [ ] **Step 9: Manual check — a failed pane no longer forks a shell**

```bash
cargo run -p sprite-app --locked --offline -- --shell /nonexistent/shell
```
Expected: Sprite opens and the pane shows the failure message. It must **not**
panic, and `ps` must show no stray `/bin/sh` from the attempt. Before this task
that path spawned a real PTY and killed it immediately.

If `--shell` is not the right flag, set `shell` in the config file instead —
check `cargo run -p sprite-app --locked --offline -- --help`.

- [ ] **Step 10: Check the inherited doc-comment claim**

The PRD carries one claim that was **never verified against source**: that
`terminal_view.rs` has "five doc comments attached to the function *below* the
one they describe, and two comments that the code five lines away contradicts".
It came from the original exploration pass and no one re-read it.

This task has just rewritten a third of the file, so check it now:

```bash
grep -n "^\s*///" crates/sprite-app/src/terminal_view.rs
```

Read each doc comment against the item directly beneath it. If the claim holds,
re-anchor the comments to the functions they describe — a doc comment on the
wrong function actively misdirects the next reader, human or AI, through a
1,500-line file. If it does not hold, say so in the commit message so the claim
stops being repeated.

Do **not** rewrite comments that are merely terse. The claim is about comments
attached to the wrong thing, not about comment quality.

- [ ] **Step 11: Commit**

```bash
git add crates/sprite-app/src/terminal_view.rs crates/sprite-app/src/terminal_events.rs
git commit -m "Let a failed Pane own no session at all"
```

---

### Task 3: Build a Pane in one place

`Workspace` constructs a `TerminalView` in three places — `new` (:108),
`split` (:211) and `open_tab` (:234). The closure bodies at `:211` and `:234` are
character-identical; the two sites differ only in which `Tabs` method receives
the closure and in the trailing focus call. Adding anything to a Pane's
construction — one environment variable, a per-Pane title — means finding and
editing three sites, and no test constructs a `Workspace`, so nothing catches a
missed one.

**Files:**
- Modify: `crates/sprite-app/src/workspace.rs`

**Interfaces:**
- Consumes: `session_environment` (:621), `pane_link` (:603), both already free
  functions in this file. `TerminalView::new(command, settings, environment,
  observation, window, cx)`.
- Produces: `fn make_pane<'a>(…) -> impl FnOnce(TabId, PaneId) -> gpui::Entity<TerminalView> + 'a`.

- [ ] **Step 1: Add the one constructor**

A **free** function, not a method. Every call site holds `&mut self.tabs` while
the closure runs, so a `&self` method could not be called there — which is why
the PRD's "third copy folds once `self` allows" is unnecessary pessimism. A free
function folds all three now.

Add beside `pane_link` in `crates/sprite-app/src/workspace.rs`:

```rust
/// Builds one Pane, wherever a Pane is built.
///
/// Free rather than a method: each caller holds `&mut self.tabs` while this
/// closure runs, so a `&self` method could not be called there. Everything a
/// Pane needs is passed in, which is also what lets `Workspace::new` use it
/// before `self` exists.
fn make_pane<'a>(
    command: Option<Vec<std::ffi::OsString>>,
    settings: crate::config::Settings,
    panes: &'a Arc<WindowPanes>,
    endpoint: Option<&'a Endpoint>,
    window: &'a mut Window,
    cx: &'a mut Context<Workspace>,
) -> impl FnOnce(TabId, PaneId) -> gpui::Entity<TerminalView> + 'a {
    move |tab, pane| {
        let environment = session_environment(endpoint, tab, pane);
        let link = pane_link(panes, endpoint, tab, pane);
        cx.new(|cx| TerminalView::new(command, settings, environment, link, window, cx))
    }
}
```

- [ ] **Step 2: Use it in `split`**

```rust
    fn split(&mut self, orientation: Orientation, window: &mut Window, cx: &mut Context<Self>) {
        // A split starts a fresh session; panes never share one.
        let pane = self.tabs.split(
            orientation,
            make_pane(
                self.command.clone(),
                self.settings.clone(),
                &self.panes,
                self.endpoint.as_ref(),
                window,
                cx,
            ),
        );
        self.request_focus(pane);
        cx.notify();
    }
```

- [ ] **Step 3: Use it in `open_tab`**

```rust
    fn open_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.tabs.open(make_pane(
            self.command.clone(),
            self.settings.clone(),
            &self.panes,
            self.endpoint.as_ref(),
            window,
            cx,
        ));
        self.request_focus(self.tabs.active().focus());
        cx.notify();
    }
```

If the borrow checker objects to `&self.panes` alongside `&mut self.tabs` in one
expression, bind the shared pieces to locals first, exactly as the current code
does — the fields are disjoint, so this always works:

```rust
        let panes = &self.panes;
        let endpoint = self.endpoint.as_ref();
        let program = self.command.clone();
        let pane_settings = self.settings.clone();
        let pane = self.tabs.split(
            orientation,
            make_pane(program, pane_settings, panes, endpoint, window, cx),
        );
```

- [ ] **Step 4: Use it in `Workspace::new`**

`new` builds its Pane before `self` exists, which is exactly why `make_pane` is
free. Replace the `Tabs::new(|tab, pane| { … })` closure with:

```rust
        let tabs = Tabs::new(make_pane(
            command.clone(),
            settings.clone(),
            &panes,
            endpoint.as_ref(),
            window,
            cx,
        ));
```

The `let program = command.clone();` and `let pane_settings = settings.clone();`
lines above it become unused — delete them.

- [ ] **Step 5: Verify and commit**

Run:
```bash
cargo test --workspace --locked --offline
cargo clippy --workspace --all-targets --locked --offline -- -D warnings
cargo fmt --all -- --check
```
Expected: all green. The net line change is small — around four lines — because
the new function costs about twenty lines and each of the three sites sheds
about eight. **Line count is not the point of this task and is not a measure of
whether it worked.** The win is locality: Pane construction moves from three
sites to one, so adding an environment variable or a per-Pane title becomes a
single edit rather than three, with nothing to forget.

```bash
git add crates/sprite-app/src/workspace.rs
git commit -m "Build a Pane in one place"
```

---

### Task 4: Fold the graphics traversal written twice

`capture_frame` (:291) and `capture_placements` (:443) exist for a sound reason —
**ADR-0006 forbids the observation path from touching pixels** — but that
guarantee lives in one line (the presence or absence of `image.data()`), not in
the 48 duplicated lines around it. The 14-line layer array and the 7-line
iterator preamble are character-identical; the `ImageFormat` match appears twice,
differing only in rustfmt wrapping.

**The guarantee stays exactly where it is.** Two entry points remain; only the
traversal preamble and the format mapping are shared.

`crates/sprite-app/tests/graphics_observation.rs:134`
(`the_image_itself_never_reaches_the_response`) plants a marker byte and asserts
the decoded pixels never appear in a response, in several plausible encodings,
then asserts the response is metadata-sized. That is the end-to-end guard on this
task. It must pass unchanged.

**Files:**
- Modify: `crates/sprite-term/src/graphics.rs`

**Interfaces:**
- Consumes: nothing from other tasks.
- Produces: `const LAYERS` and `fn transmitted_format` — both private to
  `graphics.rs`. No signature outside this file changes.

- [ ] **Step 1: Extract the layer bands**

Add near the top of `crates/sprite-term/src/graphics.rs`:

```rust
/// The three layer bands, in paint order.
///
/// Three passes rather than one, because the binding classifies placements by
/// filtering rather than by reporting a z-index. Each pass is proportional to
/// the number of placements, never to cells on screen.
const LAYERS: [(libghostty_vt::kitty::graphics::Layer, Layer); 3] = [
    (
        libghostty_vt::kitty::graphics::Layer::BelowBg,
        Layer::BelowBackground,
    ),
    (
        libghostty_vt::kitty::graphics::Layer::BelowText,
        Layer::BelowText,
    ),
    (
        libghostty_vt::kitty::graphics::Layer::AboveText,
        Layer::AboveText,
    ),
];
```

- [ ] **Step 2: Extract the format mapping**

```rust
/// The wire vocabulary for a transmitted image format.
///
/// Deliberately total: an unknown format is reported as unknown rather than
/// guessed at, so a future libghostty format cannot silently become the wrong
/// one on the wire.
fn transmitted_format(format: libghostty_vt::kitty::graphics::ImageFormat) -> TransmittedFormat {
    use libghostty_vt::kitty::graphics::ImageFormat;
    match format {
        ImageFormat::Rgb => TransmittedFormat::Rgb,
        ImageFormat::Rgba => TransmittedFormat::Rgba,
        ImageFormat::Png => TransmittedFormat::Png,
        ImageFormat::Gray => TransmittedFormat::Gray,
        ImageFormat::GrayAlpha => TransmittedFormat::GrayAlpha,
        _ => TransmittedFormat::Unknown,
    }
}
```

- [ ] **Step 3: Use both in `capture_frame`**

Replace the inline `for layer in [ … ]` array with `for layer in LAYERS {`.

Replace the inline `transmitted:` match (:400–413) with:

```rust
                    transmitted: transmitted_format(image.format().map_err(vt("image_format"))?),
```

Leave `pixels: image.data().map_err(vt("image_data"))?.to_vec(),` exactly as it
is. **That line is the ADR-0006 boundary.**

- [ ] **Step 4: Use both in `capture_placements`**

Replace its `for layer in [ … ]` with `for layer in LAYERS {`.

Replace its `format:` match (:487–496) with:

```rust
                format: transmitted_format(image.format().map_err(vt("image_format"))?),
```

Add **no** `pixels` field here, and do not call `image.data()`. The absence is
the guarantee.

- [ ] **Step 5: Verify the guarantee still holds**

Run:
```bash
cargo test -p sprite-term --locked --offline
cargo test -p sprite-app --test graphics_observation --locked --offline
cargo clippy --workspace --all-targets --locked --offline -- -D warnings
cargo fmt --all -- --check
```
Expected: all green. `the_image_itself_never_reaches_the_response` passing is the
acceptance criterion for this task — if it fails, pixels reached the observation
path and the change must be reverted, not patched.

- [ ] **Step 6: Commit**

```bash
git add crates/sprite-term/src/graphics.rs
git commit -m "Traverse graphics layers once, in one place"
```

---

### Task 5: Extend the complaint helper the file already has

`Settings::parse_candidate` is 257 lines (`config.rs:396–652`) for 15 settings.
Ten sites repeat the same shape — `Some(other) => complaints.0.push(format!(…
other.type_str() …))` / `None => {}` — totalling 61 lines measured span by span.

**Correction, measured after implementation:** that 61-line figure counts each
whole arm, but the message text inside it is *per-case content*, not repetition.
The genuinely duplicated scaffold is only `push(format!(… other.type_str() …))`,
roughly two lines per site. This task therefore **adds** about 28 lines rather
than removing 41. It is still worth doing, but for locality rather than size:
the message template `"{key} must be {wanted}, not {type}; {consequence}"` ends
up in exactly one place, so the wording cannot drift between the ten sites and
changing it — adding the file path, say — is one edit instead of ten. Folding
away ten `None => {}` arms is a real control-flow simplification. Do not judge
this task by its line count.

The author already factored this once: the named closure at `:476` handles three
colour settings in 19 lines where open-coding would take about 42. This task
extends that treatment to the remaining sites.

**Do not reach for serde.** The PRD records why: this file exists so that
"absent or invalid configuration produces defaults rather than an error", and
serde aborts the document on the first type error, so one mistyped
`graphics.enabled` would lose the other fourteen settings. Per-field fallback
via `deserialize_with` costs more code than the arm it replaces.

**Files:**
- Modify: `crates/sprite-app/src/config.rs`

**Interfaces:**
- Consumes: nothing.
- Produces: nothing outside this file. `Settings::parse_candidate`'s signature
  and every complaint string are unchanged.

- [ ] **Step 1: Confirm every complaint string is asserted somewhere**

Run:
```bash
cargo test -p sprite-app --locked --offline --lib config
```
Expected: **PASS**. Note the count — these tests assert fourteen distinct
complaint strings by name, and they are your safety net. **No complaint text may
change in this task.** If a test fails later, you altered wording.

- [ ] **Step 2: Add the helper beside the existing closure**

```rust
        /// Records "this key is the wrong type" in the one shape the file uses.
        ///
        /// Returns nothing and takes no decision: a key that is absent is not a
        /// complaint, and a key that is present but wrong keeps its default.
        /// This is the same treatment the `named` closure below already gives
        /// the three colour settings.
        let mut wrong_type = |key: &str, value: Option<&toml::Value>, wanted: &str| {
            if let Some(other) = value {
                complaints.0.push(format!(
                    "{key} must be {wanted}, not {}; keeping the default",
                    other.type_str()
                ));
            }
        };
```

Place it immediately after `let mut complaints = Complaints::default();` so every
section can reach it.

- [ ] **Step 3: Convert the sites one at a time**

There are ten. Convert **one**, run the config tests, and only then convert the
next. Converting several at once makes a wording regression hard to locate.

The sites are at `config.rs:410, 444, 485, 518, 536, 544, 560, 586, 602, 637`.
The last (`:637`) is eleven lines rather than five or six — leave it until last,
and if its complaint text cannot be produced by `wrong_type` without changing the
wording, **leave it open-coded**. Preserving the message matters more than
folding the tenth site.

Each conversion replaces this shape:

```rust
                Some(other) => complaints.0.push(format!(
                    "font.size must be a number, not {}; keeping the default",
                    other.type_str()
                )),
                None => {}
```

with a call, moved out of the match:

```rust
                wrong_type("font.size", section.get("size"), "a number");
```

- [ ] **Step 4: Run the config tests after every single conversion**

Run:
```bash
cargo test -p sprite-app --locked --offline --lib config
```
Expected: **PASS** every time, with no test edited.

- [ ] **Step 5: Verify and commit**

Run:
```bash
cargo test --workspace --locked --offline
cargo clippy --workspace --all-targets --locked --offline -- -D warnings
cargo fmt --all -- --check
```
Expected: all green. The net line change is **positive** — around +28 — for the
reasons given above; that is the expected outcome, not a failure. The 165-plus lines of
genuinely per-case logic — clamp-and-report for `font.size` and
`scrollback.bytes`, per-entry survival in `colors.palette`, whole-list rejection
for `shell.args`, empty-string handling — are untouched.

```bash
git add crates/sprite-app/src/config.rs
git commit -m "Complain about a mistyped setting in one place"
```

---

### Task 6: Move `respond` out of the window view

`broker.rs:79` promises that "a request that could mutate cannot be
constructed". That is true of `broker::Scope` and false of the endpoint: config
reload is checked first and never reaches the type. An auditor reads `broker.rs`,
finds the guarantee, and stops.

The subsystem's real entry point should not be a private `fn` inside a
1,248-line GPUI view.

This task also absorbs the second half of the dissolved C10: the `Malformed` and
`UnsupportedProtocol` arms at `workspace.rs:556–564` are unreachable, because
`collect`'s only `?` is `resolve(…)?` (`broker.rs:311`) and `resolve` returns
only `Refusal::Denied`. They cannot simply be deleted — the match must stay
exhaustive — so the fix is to narrow what `collect` can return.

**Files:**
- Create: `crates/sprite-app/src/observation/request.rs`
- Modify: `crates/sprite-app/src/observation.rs` (add the module)
- Modify: `crates/sprite-app/src/observation/broker.rs` (`collect`'s error type)
- Modify: `crates/sprite-app/src/workspace.rs` (move `respond`, `config_request`,
  `protocol_check`, `ask_window`, `ConfigVerb` out)

**Interfaces:**
- Consumes: `respond(panes: &dyn PaneSource, reload: &async_channel::Sender<ReloadRequest>, body: &str) -> String`
  and `protocol_check(body: &str) -> Result<&str, Refusal>`, both introduced by
  Task 2 of `09-01-2026-defect-fixes.md`.
- Produces: `crate::observation::request::respond` with the same signature, and
  `broker::collect(…) -> Result<Report, Denied>`.

- [ ] **Step 1: Narrow what `collect` can return**

In `crates/sprite-app/src/observation/broker.rs`, add:

```rust
/// The only refusal a carried-out request can produce.
///
/// A request that reached `collect` has already parsed, so it cannot be
/// malformed and cannot name an unknown protocol. Saying so in the type is what
/// removes the two arms every caller had to write and no caller could reach.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Denied;
```

Change `collect`'s signature to `-> Result<Report, Denied>` and change `resolve`
to `-> Result<Vec<PaneAddress>, Denied>`, replacing each `Refusal::Denied` in
`resolve` with `Denied`.

- [ ] **Step 2: Run the broker's own tests**

Run:
```bash
cargo test -p sprite-app --locked --offline --lib broker
```
Expected: **FAIL to compile** in the broker's test module, where assertions read
`assert_eq!(report.unwrap_err(), Refusal::Denied)` (`broker.rs:632, 635, 648,
655`). Change those four to `Denied`. Do not change what they assert.

Re-run. Expected: **PASS**.

- [ ] **Step 3: Create the request module**

Create `crates/sprite-app/src/observation/request.rs` and move these five items
out of `workspace.rs` **unchanged apart from visibility**: `respond`,
`protocol_check`, `config_request`, `ask_window`, and the `ConfigVerb` enum.
Move the tests Task 2 of the defects TSP added along with them, plus
`a_configuration_request_is_told_from_a_pane_query`.

Head the file:

```rust
//! The observation endpoint's request grammar, in one place.
//!
//! `broker` promises that a request which could mutate cannot be constructed,
//! and that is true of everything it defines. The endpoint is where the whole
//! grammar meets — every read, and the one write — so it belongs beside that
//! promise rather than inside the window view, where an auditor reading
//! `broker.rs` would never find it.

use crate::observation::broker::{self, Denied, PaneSource, Refusal};
use crate::observation::endpoint::DENIED;
use crate::observation::schema;
use crate::workspace::ReloadRequest;
```

Add `pub(crate) mod request;` to `crates/sprite-app/src/observation.rs`.

`ReloadRequest` and `ConfigVerb` must become `pub(crate)` in `workspace.rs` for
this to compile. `ConfigVerb` moves; `ReloadRequest` stays in `workspace.rs`
because the reload task there owns it.

- [ ] **Step 4: Delete the arms that can no longer be written**

In the moved `respond`, the second match collapses, because `collect` can now
only fail one way:

```rust
    match broker::collect(&query, panes, broker::DEADLINE) {
        Ok(report) => schema::render(&report, query.pretty),
        Err(Denied) => DENIED.to_owned(),
    }
```

Seven lines go, and the compiler now guarantees they were unreachable rather than
a reviewer having to trace it.

- [ ] **Step 5: Point the endpoint at the new home**

In `crates/sprite-app/src/workspace.rs`, `open_endpoint` becomes:

```rust
    Endpoint::open(move |request| {
        crate::observation::request::respond(panes.as_ref(), &reload, &request.body)
    })
    .ok()
```

- [ ] **Step 6: Verify and commit**

Run:
```bash
cargo test --workspace --locked --offline
cargo clippy --workspace --all-targets --locked --offline -- -D warnings
cargo fmt --all -- --check
```
Expected: all green. The protocol tests written in the defects TSP must pass
**unchanged** in their new location — that is the check that this move changed no
behaviour.

```bash
git add crates/sprite-app/src/observation/ crates/sprite-app/src/workspace.rs
git commit -m "Give the observation grammar a home of its own"
```

---

### Task 7: Define the snapshot request grammar once

The five-flag grammar has no single definition. `cli::Scope` is
`Tab | TabWithSelf | Pane | Window`; `broker::Scope` is
`Tab { include_self } | Pane | Window`. Two enums of different shape, one
validation rule written twice, and adding `--since` means six sites in three
modules.

The seam stays exactly where ADR-0001 put it: **the wire text between two
processes.** What changes is that one module owns both directions across it.

The client-side duplication at `cli.rs:438` is deliberate and documented —
refusing a bad flag combination locally gives a better message than relaying the
window's refusal. **Keep that behaviour.** It does not require a second
definition of the grammar.

**Files:**
- Modify: `crates/sprite-app/src/observation/request.rs` (add `Scope`, `Query`,
  `render`, `parse`)
- Modify: `crates/sprite-app/src/observation/broker.rs` (re-export, delete its `Scope`)
- Modify: `crates/sprite-app/src/cli.rs` (use the shared `Scope`)
- Modify: `crates/sprite-app/src/observation/client.rs` (use `render`)

**Interfaces:**
- Consumes: `crate::observation::request` from Task 6.
- Produces: `request::Scope`, `request::Query`, `request::render(&Query) -> String`,
  `request::parse(&str) -> Result<Query, Refusal>`.

- [ ] **Step 1: Write the failing round-trip test**

This test is the acceptance criterion for the task. It is what actually retires
the two-enums problem: it fails if the two directions ever disagree.

In `crates/sprite-app/src/observation/request.rs`:

```rust
    /// The grammar has two directions and they must be the same grammar.
    ///
    /// This is the test that could not be written while `cli` and `broker` each
    /// defined their own `Scope`: there was no single value to round-trip.
    #[test]
    fn every_request_survives_the_wire_and_back() {
        let cases = [
            Query { scope: Scope::Window, from: None, pretty: false, lines: HistoryLines::default() },
            Query { scope: Scope::Tab { include_self: false }, from: Some(PaneId(4)), pretty: false, lines: HistoryLines::default() },
            Query { scope: Scope::Tab { include_self: true }, from: Some(PaneId(4)), pretty: true, lines: HistoryLines::default() },
            Query { scope: Scope::Pane(PaneId(7)), from: None, pretty: true, lines: HistoryLines::default() },
        ];

        for original in cases {
            let text = render(&original);
            let parsed = parse(&text).unwrap_or_else(|error| {
                panic!("{text:?} did not parse back: {error:?}")
            });
            assert_eq!(parsed, original, "round trip changed {text:?}");
        }
    }
```

`Query` needs `#[derive(Clone, Copy, Debug, Eq, PartialEq)]` for this. Add it.

- [ ] **Step 2: Run to verify it fails**

Run:
```bash
cargo test -p sprite-app --locked --offline --lib every_request_survives
```
Expected: **FAIL to compile** — `render` does not exist, and `Scope` is not in
this module.

- [ ] **Step 3: Move `Scope` and `Query` into `request`**

Move `broker::Scope` and `broker::Query` into `request.rs` verbatim, keeping
their doc comments — especially `Scope`'s, which records that every variant
reads and that there is deliberately no variant that writes.

In `broker.rs`, replace them with a re-export so its own signatures still read
naturally:

```rust
pub use crate::observation::request::{Query, Scope};
```

Move `broker::parse` into `request.rs` as `parse`. In `broker.rs`, re-export it:
`pub use crate::observation::request::parse;` — `lib.rs` exports it as
`parse_request` and `sprite-observation-bench` calls it, so the name must survive.

- [ ] **Step 4: Add the other direction**

```rust
/// Renders a query as the wire text a client sends.
///
/// The inverse of `parse`, and kept beside it so the two cannot drift. ADR-0001
/// puts the seam at this text; this function is one side of it.
pub fn render(query: &Query) -> String {
    let mut text = format!("{} panes snapshot", broker::PROTOCOL);
    match query.scope {
        Scope::Window => text.push_str(" --window"),
        Scope::Pane(pane) => text.push_str(&format!(" --pane {}", pane.0)),
        Scope::Tab { include_self } => {
            if include_self {
                text.push_str(" --include-self");
            }
        }
    }
    if let Some(from) = query.from {
        text.push_str(&format!(" --from {}", from.0));
    }
    if query.pretty {
        text.push_str(" --pretty");
    }
    text
}
```

Check `HistoryLines`' flag spelling against `client.rs:229–260` and add its arm
to match — the round-trip test will catch it if you miss it.

- [ ] **Step 5: Run the round-trip test**

Run:
```bash
cargo test -p sprite-app --locked --offline --lib every_request_survives
```
Expected: **PASS**. If a case fails, `render` and `parse` genuinely disagree —
fix `render`, not the test.

- [ ] **Step 6: Point `cli` and `client` at the shared grammar**

Delete `cli::Scope` and use `request::Scope` in `SnapshotArgs`. `TabWithSelf`
becomes `Tab { include_self: true }`.

In `client.rs`, replace the hand-built request line at `:229–260` with
`request::render(&query)`.

**Keep** `cli.rs:438`'s local validation and its message. It refuses a bad flag
combination before a socket is opened, which is a better experience than relaying
the window's refusal.

`lib.rs` exports `SnapshotArgs`? It does not after Task 1 — check before
assuming, and if `cli::Scope` was exported, remove it from the export list too.

- [ ] **Step 7: Verify and commit**

Run:
```bash
cargo test --workspace --locked --offline
cargo clippy --workspace --all-targets --locked --offline -- -D warnings
cargo fmt --all -- --check
```
Expected: all green.

```bash
git add crates/sprite-app/src/
git commit -m "Define the snapshot request grammar once, in both directions"
```

---

### Task 8: Break the view to painter cycle

`grid_paint.rs:62` reads `use crate::terminal_view::{CURSOR_STROKE, RowPass,
cell_colors, pack, terminal_font};` while `terminal_view.rs:1151` constructs
`crate::grid_paint::GridPaint`. A mutual `use` is a seam that is not one.

`cell_colors` and `CURSOR_STROKE` have exactly one caller each — both in
`grid_paint`, zero in the module that declares them. Commit `ad3e06e` promoted
four symbols to `pub(crate)` and moved nothing.

Because `terminal_view` already depends on `grid_paint`, moving the painting
vocabulary **down** into the painter makes the dependency one-directional.

**Promise restated.** The PRD's original claim — "`GridPaint::draw` becomes
testable" — overpromises: `draw` paints into a GPUI `Window`, and moving colour
vocabulary does not change that. What this task genuinely earns is `cell_colors`
testable as a pure function. **That is the acceptance criterion.**

**Files:**
- Modify: `crates/sprite-app/src/grid_paint.rs`
- Modify: `crates/sprite-app/src/terminal_view.rs`

**Interfaces:**
- Consumes: nothing from other tasks. Do this after Task 2 — both edit
  `terminal_view.rs`, and Task 2 is the larger diff.
- Produces: `grid_paint::{CURSOR_STROKE, RowPass, cell_colors, pack, terminal_font}`,
  and `GridPaint::new(GridPaintSpec)`.

- [ ] **Step 1: Write the failing test**

In `crates/sprite-app/src/grid_paint.rs`, inside its existing `mod tests`:

```rust
    /// Colour resolution is arithmetic, not painting: it needs no Window.
    #[test]
    fn a_cell_with_no_opinion_takes_the_defaults() {
        let default_fg = Rgb { r: 0xaa, g: 0xbb, b: 0xcc };
        let default_bg = Rgb { r: 0x11, g: 0x22, b: 0x33 };
        let (foreground, background) = cell_colors(
            SnapshotColor::Default,
            SnapshotColor::Default,
            false,
            default_fg,
            default_bg,
            None,
        );
        assert_eq!(foreground, rgb(pack(default_fg)));
        assert_eq!(background, rgb(pack(default_bg)));
    }

    /// Reverse video swaps them, which is the one rule worth pinning.
    #[test]
    fn reverse_video_swaps_foreground_and_background() {
        let default_fg = Rgb { r: 0xaa, g: 0xbb, b: 0xcc };
        let default_bg = Rgb { r: 0x11, g: 0x22, b: 0x33 };
        let (foreground, background) = cell_colors(
            SnapshotColor::Default,
            SnapshotColor::Default,
            true,
            default_fg,
            default_bg,
            None,
        );
        assert_eq!(foreground, rgb(pack(default_bg)));
        assert_eq!(background, rgb(pack(default_fg)));
    }
```

Read `cell_colors`' real signature at `terminal_view.rs:692` first and match the
parameter order and the reverse-video flag's actual name. Do not change the
function to fit the test.

- [ ] **Step 2: Run to verify it fails**

Run:
```bash
cargo test -p sprite-app --locked --offline --lib grid_paint
```
Expected: **FAIL to compile** — `cell_colors` is not in this module.

- [ ] **Step 3: Move the painting vocabulary down**

Move from `terminal_view.rs` into `grid_paint.rs`, unchanged: `CURSOR_STROKE`
(:66), the colour block `pack` and `cell_colors` (:687–707), `terminal_font`
(:761), and `enum RowPass` (:883).

Delete `grid_paint.rs:62`'s `use crate::terminal_view::{…}` line entirely.

In `terminal_view.rs`, add `use crate::grid_paint::{RowPass, pack, terminal_font};`
— it still uses those at `:785`, `:1150–1183` and `:1339`. It does **not** use
`cell_colors` or `CURSOR_STROKE`; if the compiler says otherwise, one was missed.

- [ ] **Step 4: Run the test to verify it passes**

Run:
```bash
cargo test -p sprite-app --locked --offline --lib grid_paint
```
Expected: **PASS**, six tests (four pre-existing plus two new).

- [ ] **Step 5: Replace eleven positional arguments with one value**

`GridPaint::new` takes eleven positional arguments under
`#[allow(clippy::too_many_arguments)]`. Introduce:

```rust
/// Everything one row pass needs to paint itself.
///
/// A struct rather than eleven positional arguments: five of them are colours
/// and three are lengths, so at a call site the positional form is unreadable
/// and a transposition is invisible.
pub(crate) struct GridPaintSpec {
    pub rows: Vec<Vec<PositionedCell>>,
    pub pass: RowPass,
    pub cursor: Option<CursorSnapshot>,
    pub cursor_color: Option<Rgb>,
    pub default_fg: Rgb,
    pub default_bg: Rgb,
    pub palette: Option<Arc<[Rgb; 256]>>,
    pub cell_width: Pixels,
    pub cell_height: Pixels,
    pub font_family: SharedString,
    pub font_size: Pixels,
}
```

Change `new` to `pub(crate) fn new(spec: GridPaintSpec) -> Self` and delete all
three `#[allow(clippy::too_many_arguments)]` attributes in the file (`:79`,
`:348`, `:416`) — apply the same treatment to the other two if clippy still
objects.

Update the `build` closure at `terminal_view.rs:1150` to construct a
`GridPaintSpec`.

- [ ] **Step 6: Verify and commit**

Run:
```bash
cargo test --workspace --locked --offline
cargo clippy --workspace --all-targets --locked --offline -- -D warnings
cargo fmt --all -- --check
```
Expected: all green, and `grep -n "use crate::terminal_view" crates/sprite-app/src/grid_paint.rs`
returns nothing. The cycle is gone.

```bash
git add crates/sprite-app/src/grid_paint.rs crates/sprite-app/src/terminal_view.rs
git commit -m "Let the painter own the painting vocabulary"
```

---

### Task 9: Bundle the projector's scratch state

`snapshot::capture` takes nine parameters and `publish` takes eleven, both under
`#[allow(clippy::too_many_arguments)]`. Five of the nine are mutable scratch
buffers that only `snapshot.rs` and `graphics.rs` know how to use, but that
`worker::run` must construct, thread through four sites, and destroy.

**This is C3a only. Do not write a `Drop` impl.**

The PRD's C3b — encoding the drop order into a type — is **gated, not
scheduled**. `worker.rs:792` says "Every libghostty value goes first", but the
list at `:795–799` omits `placements`, one of the four objects `render_objects()`
returns. All four are declared `'static`, so these are not Rust borrows: the
ordering concerns libghostty's internal state, which the compiler will never
check. Writing a `Drop` impl now would freeze a guess, and because `Drop` fires
implicitly a future reader would have *less* signal than the explicit list.

**Keep the explicit `drop()` list exactly as it is**, now against the struct's
fields.

**Open question for whoever picks up C3b:** must `placements` drop before
`terminal`? If yes, the current code has a latent bug and that is a defect, not a
refactor. If no, the comment at `:792` is wrong and should name the three values
that do matter. This is a libghostty question, and it blocks nothing here.

**Files:**
- Modify: `crates/sprite-term/src/snapshot.rs` (`capture`'s signature)
- Modify: `crates/sprite-term/src/worker.rs` (`Projector`, `run`, delete `publish`)

**Interfaces:**
- Consumes: `RenderObjects` and `render_objects()` (`worker.rs:1160`), which
  already exist and are half of this task.
- Produces: `pub(crate) struct Projector<'vt>` with
  `fn capture(&mut self, generation: u64, size: TerminalSize, has_selection: bool, terminal: &Terminal<'vt, '_>) -> Result<SnapshotBundle, SessionError>`.

- [ ] **Step 1: Introduce the value**

In `crates/sprite-term/src/worker.rs`, replacing `RenderObjects`:

```rust
/// The scratch state a projection needs, owned in one place.
///
/// The four libghostty objects share one allocator lifetime, and the pixel
/// cache is reused across captures so an unchanged image is copied once rather
/// than once per frame. They were nine parameters threaded through four call
/// sites; nothing outside a projection ever needs them individually.
///
/// Drop order is still the explicit list in `run`, deliberately. See the TSP.
pub(crate) struct Projector<'vt> {
    render_state: RenderState<'vt>,
    rows: RowIterator<'vt>,
    cells: CellIterator<'vt>,
    placements: PlacementIterator<'vt>,
    pixels: crate::graphics::PixelCache,
}

impl Projector<'static> {
    fn new() -> Result<Self, SessionError> {
        let render_state =
            RenderState::new().map_err(|error| SessionError::new("create_render_state", error))?;
        let rows =
            RowIterator::new().map_err(|error| SessionError::new("create_row_iterator", error))?;
        let cells =
            CellIterator::new().map_err(|error| SessionError::new("create_cell_iterator", error))?;
        let placements = PlacementIterator::new()
            .map_err(|error| SessionError::new("create_placement_iterator", error))?;
        Ok(Self {
            render_state,
            rows,
            cells,
            placements,
            pixels: crate::graphics::PixelCache::default(),
        })
    }
}
```

Copy the exact error strings from the existing `render_objects()` — tests may
assert them.

- [ ] **Step 2: Give it the capture method**

```rust
impl<'vt> Projector<'vt> {
    /// One projection, against one generation of one terminal.
    fn capture(
        &mut self,
        generation: u64,
        size: TerminalSize,
        has_selection: bool,
        terminal: &Terminal<'vt, '_>,
    ) -> Result<SnapshotBundle, SessionError> {
        snapshot::capture(
            generation,
            size,
            has_selection,
            terminal,
            &mut self.render_state,
            &mut self.rows,
            &mut self.cells,
            &mut self.placements,
            &mut self.pixels,
        )
    }
}
```

Leave `snapshot::capture`'s signature alone for now — this step alone removes the
threading. Removing its `#[allow]` is Step 4.

- [ ] **Step 3: Delete `publish`, and call the projector**

`publish` (31 lines, `worker.rs:1132`) becomes a three-line closure over the
projector. Replace all three of its call sites — the initial one at `:378`, the
gate at `:756`, and the final capture at `:775` — with:

```rust
        if dirty && snapshots.is_empty() {
            dirty = match projector.capture(generation, size, has_selection, &terminal) {
                Ok(bundle) => !snapshots.try_send(Arc::new(bundle)).is_ok(),
                Err(error) => {
                    let _ = events.send_blocking(TerminalEvent::Error(error));
                    true
                }
            };
        }
```

Keep the closing `drop()` list, rewritten against the struct's fields:

```rust
    // ---- Closing ----
    //
    // Every libghostty value goes first, which also removes the PTY-write
    // callback: from here the terminal can neither be mutated nor generate a
    // reply, and application commands are read only to be discarded.
    //
    // Still explicit rather than a Drop impl: see the open question in
    // docs/TSP/09-01-2026-architecture-remediation.md Task 9.
    drop(projector);
    drop(encoder);
    drop(terminal);
```

**Note this changes the observable order**: `placements` and `pixels` now drop
with the projector, before `terminal`, where previously they dropped at end of
scope. If any test fails or the app misbehaves at shutdown, that is the answer to
the open question above — record it and stop.

- [ ] **Step 4: Shrink `snapshot::capture`**

Move `Projector` to `snapshot.rs` if `capture` is the only caller of its fields,
and reduce `capture` to taking `&mut Projector` plus the four scalars. Remove
`#[allow(clippy::too_many_arguments)]` from both `snapshot.rs:169` and the now
deleted `publish`.

If moving `Projector` across modules fights the `'vt` lifetime, leave it in
`worker.rs` and keep `capture`'s existing signature — Steps 1–3 already deliver
the threading win. Do not spend more than one attempt on this step.

- [ ] **Step 5: Verify and commit**

Run:
```bash
cargo test -p sprite-term --locked --offline
cargo test --workspace --locked --offline
cargo clippy --workspace --all-targets --locked --offline -- -D warnings
cargo fmt --all -- --check
```
Expected: all green. `graphics_projection.rs` and `graphics_policy.rs` exercise
the capture path end-to-end and are the real check here.

```bash
git add crates/sprite-term/src/worker.rs crates/sprite-term/src/snapshot.rs
git commit -m "Let a projection own the state it projects with"
```

---

## Open follow-ups

Tracked here rather than in a task report, because nothing in the build will
raise them on its own.

### Narrow `broker::parse`'s error type

After Task 6, `Refusal::Denied` has **no constructor anywhere**. Its one
remaining mention is the arm at `observation/request.rs:65`, handling
`broker::parse`'s error — and `parse` constructs only `Malformed` and
`UnsupportedProtocol`. So an unreachable arm survives, the same defect class
Task 6 removed one function over.

It was deliberately left: narrowing `parse` is a second type change, and keeping
it out is what made Task 6 reviewable as a pure move. But a `pub` enum variant
with no constructor never draws a `dead_code` warning, so unlike every other dead
item on this branch the compiler will not prompt this one.

The fix is to give `parse` an error type that cannot express `Denied`. Note
`parse` is public API — re-exported as `parse_request` and called by
`sprite-observation-bench` — so its error type is part of that surface; the bench
only calls `.expect(…)`, so any `Debug` error satisfies it.

**When this is done, move the security rationale with it.** `Refusal::Denied`'s
doc comment carries the reason that matters — one answer for "no such pane" and
"not your pane", so a caller cannot map the window by watching the refusal change
— and the `Denied` struct that now carries that meaning documents only the
exhaustiveness argument. That sentence must land on `Denied`, not be deleted with
the variant.

### Assert `respond`'s refusal path end to end

`NoPanes` now lives in `request.rs`'s test module, so the path returning `DENIED`
— the security-relevant answer — could be asserted in one line:
`respond(&NoPanes, &reload, "panes snapshot --from 42")`. Today it is covered only
at the `collect` level in `broker.rs`.

## Definition of done

Per-task acceptance criteria, which a reviewer checks rather than judges:

- [ ] **Task 1** — `cargo check -p sprite-app --all-targets` reports **no**
  `never used` warnings, and **no `#[allow(dead_code)]` was added**. ADR-0012
  carries the amendment with a freshly measured figure.
- [ ] **Task 2** — `decide` is unit-tested across all thirteen arms with no GPUI
  in the test binary, including the two that emit two effects, and an `Error`
  event produces `Effect::FailRequest` carrying the reason. Tests must assert
  **both** halves of `Decision` — a helper that returns only `effects` lets a
  wrong `stop` pass unnoticed, and a stray `stop = true` on the `Error` arm would
  silently end the pump on the first recoverable error.
- [ ] **Task 3** — existing suite green.
- [ ] **Task 4** — `the_image_itself_never_reaches_the_response` passes unchanged.
- [ ] **Task 5** — every complaint string is byte-identical; no config test edited.
- [ ] **Task 6** — `respond` is tested through `&dyn PaneSource` for the read
  path, the config path, and the protocol refusal on both, and those tests moved
  from the defects TSP **unchanged**.
- [ ] **Task 7** — the round-trip test covers every scope and flag combination.
- [ ] **Task 8** — `cell_colors` is tested as a pure function, and
  `grid_paint.rs` no longer imports from `terminal_view`.
- [ ] **Task 9** — existing suite green; no `Drop` impl written; the `placements`
  open question recorded in the PR if Step 3 surfaced an answer.

Whole-branch:

- [ ] `cargo test --workspace --locked --offline` green.
- [ ] `cargo clippy --workspace --all-targets --locked --offline -- -D warnings` clean.
- [ ] `cargo fmt --all -- --check` clean.
- [ ] No existing test assertion was modified by any task. If one was, that task
      changed behaviour and belongs in a different branch.
