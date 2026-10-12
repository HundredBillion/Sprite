# Findings Ledger and Admission Technical Spec

> **For agentic workers:** REQUIRED SUB-SKILL: Use dmi-superpowers:subagent-driven-development (recommended) or dmi-superpowers:executing-plans to implement this TSP task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make "all bugs closed" a statement the repository checks, and remove the structural causes of the admission, error-classification, timeout and timing-test bug classes, as the PRD `docs/PRDs/10-12-2026-findings-ledger-and-admission.md` requires.

**Architecture:** A TOML ledger plus a Python checker becomes the definition of "closed" and runs in CI. In `sprite-term`, commands leave the worker's output queue for a 64-deep control lane the worker drains first, and every nonblocking offer returns one `Admitted` type. In `sprite-app`, Surface requests split into a stream lane and a one-shot lane, the view keeps one `Pending` and one `PaneFocus`, related timeouts live in one module with compile-time asserts, and five clippy lints replace comment-carried rules.

**Tech Stack:** Rust 1.97.1 (edition 2024), `std::sync::mpsc`, `async_channel` 2.5.0, GPUI 0.2.2 (vendored), Python 3 `unittest`, GitHub Actions.

## Global Constraints

- Toolchain 1.97.1; every cargo command is `PATH="$HOME/.cargo/bin:$PATH" cargo +1.97.1 … --locked --offline`.
- No new crates. No async runtime. No new threads beyond ADR 0029's Surface writers and pane cleanups.
- One libghostty-owning worker thread per Terminal Session (ADR 0008).
- Surface Channel wire protocol version 1 and the observation grammar are unchanged on the wire; refusal reasons are free text today, and the only new wire text is "this window is busy; nothing was changed". The pane's status line gains one new text, "the terminal is busy; the last command was not delivered" (decided 2026-10-12 while grilling the TSP; Task 14 records it in the PRD).
- Linux and macOS source compatibility.
- Work happens on branch `fix/findings-ledger-and-admission` in `/Users/davidlee/Projects/Sprite/.worktrees/findings-ledger-and-admission`. Never `cd` out of it; use absolute paths. The main checkout stays untouched. No release, no version bump.
- Every fix is test-first: write the regression, see it fail, make it pass. Tests cross the real seam (worker, socket, GPUI view).
- Issues #61–#75 are already filed; nothing else is published without the user's explicit go (the PR itself is created in Task 14 as the PRD's deliverable).
- The shell is zsh: quote variables, and a failing glob aborts the whole command line.
- Commit messages end with `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>`.
- The PRD's two amendments settled while writing this TSP, both recorded in Task 14: `ScanError` is `{ Denied, Unparsable, Changed }` (disappearance is already `Ok(None)`), and the closing invariant is `TERM_AFTER < KILL_AFTER < GIVE_UP_AFTER` (both measured from the request). `JoinHandle::join` is not a disallowed method: it has twenty-five legitimate test uses and the client binary joins its own reader; the GPUI-thread rule it stood for is the `local_socket.rs` close path, which Task 13 names explicitly.

---

## File Structure

New files, each with one responsibility:

| Path | Responsibility |
|---|---|
| `.github/pull_request_template.md` | The Findings table every PR fills in. |
| `clippy.toml` | Test exemptions for `unwrap`/`expect`. |
| `crates/sprite-app/clippy.toml` | The same exemptions plus `disallowed-methods` for the application crate. |
| `crates/sprite-term/src/locked.rs`, `crates/sprite-app/src/locked.rs` | One poison-recovering lock helper per crate. |
| `crates/sprite-app/src/surface/lanes.rs` | The two Surface request lanes and their capacities. |
| `crates/sprite-app/src/terminal_view/pane_focus.rs` | The `PaneFocus` owner. |
| `crates/sprite-app/src/timeouts.rs` | Related durations and their compile-time relations. |
| `docs/adr/0030-admit-commands-through-a-control-lane.md`, `docs/adr/0031-the-findings-ledger-defines-closed.md` | The two decisions. |

Modified files are named per task. The three large files this work touches (`worker/mod.rs`, `surface/channel.rs`, `terminal_view.rs`) are not restructured; the new state types are split out of `terminal_view.rs` into small modules beside it, which follows the existing `terminal_view/` layout.

---

### Task 1: The ledger runs in CI and the PR template carries a Findings table

**Files:**
- Modify: `.github/workflows/ci.yml:33`
- Create: `.github/pull_request_template.md`
- Test: `scripts/test_findings.py`

**Interfaces:**
- Consumes: `scripts/check_findings.py` and `docs/reviews/findings.toml` (already committed at `f159963`).
- Produces: nothing code-facing.

- [ ] **Step 1: Write the failing tests**

Append these two methods to the `FindingsLedgerTests` class in `scripts/test_findings.py` (`ROOT` is already defined at line 11):

```python
    def test_ci_runs_the_findings_suite(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        line = next(l for l in workflow.splitlines() if "python3 -m unittest scripts.test_release_automation" in l)
        self.assertIn("scripts.test_findings", line)

    def test_the_pull_request_template_carries_a_findings_table(self):
        template = (ROOT / ".github/pull_request_template.md").read_text()
        self.assertIn("| ID | Status after this PR | Guarding test or issue |", template)
        self.assertIn("docs/reviews/findings.toml", template)
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `python3 -m unittest scripts.test_findings -v`
Expected: the two new tests FAIL (`AssertionError` and `FileNotFoundError`); the ten existing ones pass.

- [ ] **Step 3: Wire CI and write the template**

In `.github/workflows/ci.yml` line 33 change the `run:` to:

```yaml
        run: python3 -m unittest scripts.test_release_automation scripts.test_packaging_automation scripts.test_shell_integration scripts.test_ci_policy scripts.test_findings -v
```

Create `.github/pull_request_template.md`:

```markdown
## What changed

<!-- One paragraph. What a reader of the diff could not infer. -->

## Findings

Status lives only in `docs/reviews/findings.toml`; this table is the PR's
summary of what it changes there. `scripts/check_findings.py` fails CI when a
resolved finding names no existing test or an open one has no issue.

| ID | Status after this PR | Guarding test or issue |
|---|---|---|
| | | |

## Evidence

<!-- The commands run and their results. List anything CI cannot run as
unexecuted rather than claimed. -->
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `python3 -m unittest scripts.test_findings scripts.test_ci_policy -v`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git -C /Users/davidlee/Projects/Sprite/.worktrees/findings-ledger-and-admission add .github/workflows/ci.yml .github/pull_request_template.md scripts/test_findings.py
git -C /Users/davidlee/Projects/Sprite/.worktrees/findings-ledger-and-admission commit -m "$(cat <<'EOF'
Run the findings checker in CI and add the PR Findings table

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
EOF
)"
```

---

### Task 2: One `locked` helper per crate (R-S1)

**Files:**
- Create: `crates/sprite-term/src/locked.rs`, `crates/sprite-app/src/locked.rs`
- Modify: `crates/sprite-term/src/lib.rs` (add `mod locked;`), `crates/sprite-app/src/lib.rs` (add `mod locked;`)
- Modify: `crates/sprite-term/src/event_mailbox.rs:60,87,110,112,121,126,134,139,146,154,211`, `crates/sprite-term/src/foreground.rs:110`, `crates/sprite-app/src/local_socket.rs:185,292,364,383`, `crates/sprite-app/src/surface/channel.rs:295,386,442,448,459,470,551`, `crates/sprite-app/src/observation/panes.rs:205,233`
- Test: unit tests inside each `locked.rs`

**Interfaces:**
- Produces: `pub(crate) fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T>` in both crates.

- [ ] **Step 1: Write the failing test**

Create `crates/sprite-term/src/locked.rs`:

```rust
//! One rule for a poisoned lock: take the guard anyway.
//!
//! Every mutex in this crate guards state that a panicking holder cannot
//! leave half-written in a way the next holder cannot survive, so recovery is
//! always the right answer and no site should decide it alone.

use std::sync::{Mutex, MutexGuard, PoisonError};

/// Locks `mutex`, recovering the guard if a previous holder panicked.
pub(crate) fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::locked;
    use std::sync::{Arc, Mutex};

    #[test]
    fn a_poisoned_lock_still_hands_out_its_guard() {
        let mutex = Arc::new(Mutex::new(1));
        let poisoner = Arc::clone(&mutex);
        let panicked = std::thread::spawn(move || {
            let _guard = poisoner.lock().unwrap();
            panic!("poison the lock");
        })
        .join();
        assert!(panicked.is_err());
        assert!(mutex.is_poisoned());
        assert_eq!(*locked(&mutex), 1);
    }
}
```

Create `crates/sprite-app/src/locked.rs` with the identical content (the two crates cannot share test-free code without a third crate, and six lines are cheaper than one).

Add `mod locked;` to `crates/sprite-term/src/lib.rs` after `mod foreground;` and to `crates/sprite-app/src/lib.rs` after `mod local_socket;`.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `PATH="$HOME/.cargo/bin:$PATH" cargo +1.97.1 test --workspace --locked --offline locked::tests`
Expected: both tests FAIL with `not yet implemented`.

- [ ] **Step 3: Implement**

Replace `todo!()` in both files with:

```rust
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
```

Then route every production lock through it:

- `event_mailbox.rs`: every `self.state.lock().unwrap()` and `.lock().unwrap_or_else(|error| error.into_inner())` becomes `crate::locked::locked(&self.state)` (or `&self.0.state`, `&self.mailbox.state` as the receiver is). The `Condvar::wait` and `wait_timeout` results on lines 110 and 112 are not mutex locks; leave them for Task 13.
- `foreground.rs:110`: `fn lock(&self)` body becomes `crate::locked::locked(&self.attached)`.
- `local_socket.rs:185,292,364,383`: `self.connections.lock().unwrap()` becomes `crate::locked::locked(&self.connections)`.
- `surface/channel.rs:295` (`Handle::drop`): replace `if let Ok(mut queue) = self.shared.queue.lock() { queue.closing = true; }` with `crate::locked::locked(&self.shared.queue).closing = true;`.
- `surface/channel.rs:386` (`send_batch`): replace `let Ok(mut queue) = shared.queue.lock() else { return false; };` with `let mut queue = crate::locked::locked(&shared.queue);`.
- `surface/channel.rs:442` (`abandon`): `if let Ok(queue) = shared.queue.lock() { shared.kill(queue, false); }` becomes `shared.kill(crate::locked::locked(&shared.queue), false);`.
- `surface/channel.rs:448` (`is_dead`): the `.lock().map(|queue| queue.dead).unwrap_or(true)` chain becomes `crate::locked::locked(&self.handle.shared.queue).dead`.
- `surface/channel.rs:459,470,551` and `observation/panes.rs:205,233`: same substitution.

- [ ] **Step 4: Run the tests and the suite**

Run: `PATH="$HOME/.cargo/bin:$PATH" cargo +1.97.1 test --workspace --locked --offline --no-fail-fast`
Expected: all pass (baseline 992 + 2).

- [ ] **Step 5: Commit**

```bash
git -C /Users/davidlee/Projects/Sprite/.worktrees/findings-ledger-and-admission add -A crates/sprite-term/src crates/sprite-app/src
git -C /Users/davidlee/Projects/Sprite/.worktrees/findings-ledger-and-admission commit -m "$(cat <<'EOF'
Recover poisoned locks through one helper per crate

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
EOF
)"
```

---

### Task 3: `SessionError` carries a kind (R-E)

**Files:**
- Modify: `crates/sprite-term/src/event.rs:74-88`, `crates/sprite-term/src/session.rs:85-142,207`
- Test: unit tests in `event.rs`; `crates/sprite-term/tests/event_backpressure.rs:214`

**Interfaces:**
- Produces:
  ```rust
  #[non_exhaustive] #[derive(Clone, Copy, Debug, Eq, PartialEq)]
  pub enum ErrorKind { Invalid, Ended, Transient, Other }
  pub struct SessionError { pub operation: &'static str, pub message: String, pub kind: ErrorKind }
  impl SessionError {
      pub(crate) fn new(operation, message) -> Self        // kind Other
      pub(crate) fn invalid(operation, message) -> Self    // kind Invalid
      pub(crate) fn ended(operation, message) -> Self      // kind Ended
      pub(crate) fn transient(operation, message) -> Self  // kind Transient
  }
  ```
- Later tasks compare with `error.kind == ErrorKind::Ended`; never `match` on it (non_exhaustive would force a wildcard arm).

- [ ] **Step 1: Write the failing tests**

Append to `crates/sprite-term/src/event.rs`:

```rust
#[cfg(test)]
mod error_kind_tests {
    use super::{ErrorKind, SessionError};

    #[test]
    fn constructors_set_the_kind_and_new_defaults_to_other() {
        assert_eq!(SessionError::new("send", "x").kind, ErrorKind::Other);
        assert_eq!(SessionError::invalid("send", "x").kind, ErrorKind::Invalid);
        assert_eq!(SessionError::ended("send", "x").kind, ErrorKind::Ended);
        assert_eq!(SessionError::transient("open_pty", "x").kind, ErrorKind::Transient);
    }

    #[test]
    fn display_is_unchanged_by_the_kind() {
        assert_eq!(SessionError::ended("send", "gone").to_string(), "send: gone");
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `PATH="$HOME/.cargo/bin:$PATH" cargo +1.97.1 test -p sprite-term --locked --offline error_kind_tests`
Expected: compile error, `no variant or associated item named ...`.

- [ ] **Step 3: Implement**

Replace the `SessionError` definition and `impl` in `event.rs` with:

```rust
/// What class of failure a [`SessionError`] is, so a caller can act on it
/// without reading the message.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ErrorKind {
    /// The request itself was refused: too large, or not allowed.
    Invalid,
    /// The session is over or ending; nothing will be accepted again.
    Ended,
    /// A retry may succeed.
    Transient,
    /// Anything else.
    Other,
}

/// A failure attributed to the operation that produced it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionError {
    pub operation: &'static str,
    pub message: String,
    pub kind: ErrorKind,
}

impl SessionError {
    pub(crate) fn new(operation: &'static str, message: impl fmt::Display) -> Self {
        Self::of(ErrorKind::Other, operation, message)
    }

    pub(crate) fn invalid(operation: &'static str, message: impl fmt::Display) -> Self {
        Self::of(ErrorKind::Invalid, operation, message)
    }

    pub(crate) fn ended(operation: &'static str, message: impl fmt::Display) -> Self {
        Self::of(ErrorKind::Ended, operation, message)
    }

    pub(crate) fn transient(operation: &'static str, message: impl fmt::Display) -> Self {
        Self::of(ErrorKind::Transient, operation, message)
    }

    fn of(kind: ErrorKind, operation: &'static str, message: impl fmt::Display) -> Self {
        Self {
            operation,
            message: message.to_string(),
            kind,
        }
    }
}
```

In `session.rs`:
- `validate()` (line ~207): `SessionError::new("send", format!("input of {bytes} ..."))` becomes `SessionError::invalid(...)`.
- `CommandSender::validate` (line ~137): `"the terminal session is shutting down"` becomes `SessionError::ended(...)`.
- Every `"the terminal worker ended"` construction in `send`, `try_send`, `request_history` becomes `SessionError::ended(...)`. (`try_send` is removed in Task 4; change it here anyway so this task compiles on its own.)

In `crates/sprite-term/tests/event_backpressure.rs:214`, replace `.is_err_and(|error| error.message == "the terminal worker ended")` with `.is_err_and(|error| error.kind == sprite_term::ErrorKind::Ended)`. Note: after a natural exit the sender is disconnected, which `try_send` reports as `ended`; Task 4 revisits this line.

The openpty retry (`start.rs:372-402`) keeps its text match at the one place it is constructed, and the error `open_pty` finally returns stays `Other`: a failure that survived eight retries is not transient.

- [ ] **Step 4: Run tests**

Run: `PATH="$HOME/.cargo/bin:$PATH" cargo +1.97.1 test --workspace --locked --offline --no-fail-fast`
Expected: all pass. `transient` has no production caller yet (the openpty retry already gives up for good after eight attempts, which is `Other`); it is `pub(crate)` and covered by the unit test, so `dead_code` does not fire. If it does, the constructor is used in Task 9's `discard` path for the "no thread" failure — wire it there and not with an `allow`.

- [ ] **Step 5: Commit**

```bash
git -C /Users/davidlee/Projects/Sprite/.worktrees/findings-ledger-and-admission add -A crates/sprite-term
git -C /Users/davidlee/Projects/Sprite/.worktrees/findings-ledger-and-admission commit -m "$(cat <<'EOF'
Give SessionError a kind so callers stop reading its text

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
EOF
)"
```

---

### Task 4: `Admitted` and the worker's control lane (R-A1, R-A2; GAP-C11)

**Files:**
- Modify: `crates/sprite-term/src/session.rs` (whole `CommandSender`/`TerminalSession` surface), `crates/sprite-term/src/worker/mod.rs:95-114,222-236,250-262,336-346,352-380,476,996-1010`
- Test: `crates/sprite-term/tests/event_backpressure.rs` (new regression + line 214), `session.rs` unit tests, `worker/mod.rs:996` test

**Interfaces:**
- Produces (all `pub`, exported through `pub use session::*`):
  ```rust
  pub const CONTROL_CAPACITY: usize = 64;
  #[must_use = "a saturated or closed admission hands the command back"]
  #[derive(Debug)]
  pub enum Admitted<T> { Accepted, Saturated(T), Closed(T) }
  impl<T> Admitted<T> { pub fn is_accepted(&self) -> bool }
  impl CommandSender {
      pub fn try_admit(&self, command: TerminalCommand) -> Result<Admitted<TerminalCommand>, SessionError>;
      pub fn send(&self, command: TerminalCommand) -> Result<(), SessionError>;        // unchanged
      pub fn request_history(&self, ticket, lines) -> Result<HistoryRequest, SessionError>; // unchanged
  }
  impl TerminalSession { pub fn try_admit(&mut self, command) -> Result<Admitted<TerminalCommand>, SessionError>; }
  ```
  `try_send` is removed from both. `Err` is only a validation refusal (`ErrorKind::Invalid`) or "shutting down" (`ErrorKind::Ended`).
- Worker: `Message::Wake`; `Runtime.control: Receiver<Message>`; `run(config, commands, inbox, control, events, snapshots, shutdown, foreground)`.

- [ ] **Step 1: Write the failing regression (GAP-C11) against the old API**

Append to `crates/sprite-term/tests/event_backpressure.rs`:

```rust
/// GAP-C11: while every output permit is outstanding and the worker is
/// stalled, a keystroke is still admitted. Before the control lane, the one
/// spare inbox slot was shared with capture requests, so under a flood the
/// second command was refused as "queue is full".
#[test]
fn input_is_admitted_while_every_output_permit_is_outstanding() {
    let writes = clipboard_writes(100);
    let sprite_term::Spawned {
        mut session,
        events: _events,
        snapshots: _snapshots,
    } = TerminalSession::spawn(on_cue(&writes, "yes")).unwrap();
    focus_and_release(&mut session);
    // Pressure: the events are never taken, so the worker stalls in publish
    // and the pump fills every permit behind it. More time only means more
    // pressure, never less, so this pause cannot make the assertion flaky.
    std::thread::sleep(Duration::from_millis(300));
    for n in 0..64 {
        assert!(
            session.try_send(TerminalCommand::Input(b"x".to_vec())).is_ok(),
            "keystroke {n} refused under an output flood"
        );
    }
    session.begin_shutdown().unwrap().unwrap().wait().unwrap();
}
```

- [ ] **Step 2: Run it to verify it fails on the current code**

Run: `PATH="$HOME/.cargo/bin:$PATH" cargo +1.97.1 test -p sprite-term --locked --offline --test event_backpressure input_is_admitted_while_every_output_permit_is_outstanding`
Expected: FAIL with `keystroke 1 refused under an output flood` (or a small n). Record the n in the commit body.

- [ ] **Step 3: Implement the lane in `session.rs`**

Add after `WORKER_QUEUE_CAPACITY`:

```rust
/// Commands and history requests wait here, apart from output, so a flood
/// of output never refuses a keystroke (GAP-C11, ADR 0030). Sixty-four is a
/// wedged worker's worth of typing, not a flood's.
pub const CONTROL_CAPACITY: usize = 64;

/// What became of a command offered without waiting.
#[must_use = "a saturated or closed admission hands the command back"]
#[derive(Debug)]
pub enum Admitted<T> {
    /// Queued for the worker.
    Accepted,
    /// The control lane is full: the worker is wedged or far behind. The
    /// command is handed back for the caller to keep or drop.
    Saturated(T),
    /// The worker is gone; nothing will ever take the command. Handed back.
    Closed(T),
}

impl<T> Admitted<T> {
    pub fn is_accepted(&self) -> bool {
        matches!(self, Self::Accepted)
    }
}

/// Offers one command to the control lane, then knocks on the inbox so an
/// idle worker wakes. A full inbox means the worker is busy and reaches the
/// control lane after its current message, so a failed knock is not an error.
fn admit(
    control: &SyncSender<worker::Message>,
    inbox: &SyncSender<worker::Message>,
    command: TerminalCommand,
) -> Admitted<TerminalCommand> {
    match control.try_send(worker::Message::Command(command)) {
        Ok(()) => {
            knock(inbox);
            Admitted::Accepted
        }
        Err(TrySendError::Full(message)) => Admitted::Saturated(message.into_command()),
        Err(TrySendError::Disconnected(message)) => Admitted::Closed(message.into_command()),
    }
}

/// Wakes a worker that may be blocked on an empty inbox.
fn knock(inbox: &SyncSender<worker::Message>) {
    match inbox.try_send(worker::Message::Wake) {
        Ok(()) | Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => {}
    }
}
```

`CommandSender` becomes:

```rust
#[derive(Clone)]
pub struct CommandSender {
    control: SyncSender<worker::Message>,
    inbox: SyncSender<worker::Message>,
    shutdown: Arc<AtomicBool>,
}

impl CommandSender {
    /// Submits a command, waiting for lane space on the calling thread.
    pub fn send(&self, command: TerminalCommand) -> Result<(), SessionError> {
        self.validate(&command)?;
        self.control
            .send(worker::Message::Command(command))
            .map_err(|_| SessionError::ended("send", "the terminal worker ended"))?;
        knock(&self.inbox);
        Ok(())
    }

    /// Offers a command without waiting. `Err` is a refusal of the command
    /// itself; a full or closed lane hands the command back instead.
    pub fn try_admit(
        &self,
        command: TerminalCommand,
    ) -> Result<Admitted<TerminalCommand>, SessionError> {
        self.validate(&command)?;
        Ok(admit(&self.control, &self.inbox, command))
    }

    /// Requests history without waiting for lane space; answers retain `ticket`.
    pub fn request_history(
        &self,
        ticket: crate::Ticket,
        lines: crate::HistoryLines,
    ) -> Result<HistoryRequest, SessionError> {
        self.validate(&TerminalCommand::CaptureHistory { ticket, lines })?;
        let cancelled = Arc::new(AtomicBool::new(false));
        let request = HistoryRequest {
            cancelled: Arc::clone(&cancelled),
        };
        match self.control.try_send(worker::Message::HistoryRequest {
            ticket,
            lines,
            cancelled,
        }) {
            Ok(()) => {
                knock(&self.inbox);
                Ok(request)
            }
            Err(TrySendError::Full(_)) => Err(SessionError::new(
                "send",
                "the terminal command queue is full",
            )),
            Err(TrySendError::Disconnected(_)) => {
                Err(SessionError::ended("send", "the terminal worker ended"))
            }
        }
    }
    // validate() unchanged
}
```

`TerminalSession` fields: replace `commands: SyncSender<worker::Message>` with `control` and `inbox`. In `spawn`:

```rust
        let (inbox, inbox_rx) = mpsc::sync_channel(WORKER_QUEUE_CAPACITY);
        let (control, control_rx) = mpsc::sync_channel(CONTROL_CAPACITY);
        …
                let commands = inbox.clone();
                move || {
                    worker::run(config, commands, inbox_rx, control_rx, event_tx, snapshot_tx, shutdown, foreground)
                }
        …
        let requests = inbox.clone();
        Ok(Spawned {
            session: Self { control, inbox, shutdown, foreground, worker: Some(worker), events: event_cancel },
            …
```

`commands()` builds `CommandSender { control: self.control.clone(), inbox: self.inbox.clone(), shutdown: … }`. `send` delegates as before. Replace `try_send` with:

```rust
    /// Offers a command without waiting, suitable for the UI thread.
    pub fn try_admit(
        &mut self,
        command: TerminalCommand,
    ) -> Result<Admitted<TerminalCommand>, SessionError> {
        self.commands().try_admit(command)
    }
```

`request_shutdown` uses `self.inbox.try_send(worker::Message::Shutdown)` (unchanged semantics). The history-cancellation test's `gate()` sends `HistoryGate` through `sender.control`, and its `PtyOutput` through `commands.inbox`.

- [ ] **Step 4: Implement the worker side in `worker/mod.rs`**

Add to `Message`:

```rust
    /// A knock: something is waiting on the control lane. Carries nothing.
    Wake,
```

Add `fn into_command(self) -> TerminalCommand` on `Message` (an `impl Message` block after the enum):

```rust
impl Message {
    /// The command a refused control message carried.
    pub(crate) fn into_command(self) -> TerminalCommand {
        match self {
            Self::Command(command) => command,
            Self::HistoryRequest { .. }
            | Self::PtyOutput(_)
            | Self::CaptureRequested
            | Self::Wake
            | Self::PumpStopped(_)
            | Self::ChildExited(_)
            | Self::Shutdown => unreachable!("only a command is offered through admit"),
            #[cfg(test)]
            Self::HistoryGate { .. } => unreachable!("only a command is offered through admit"),
        }
    }
}
```

`Runtime` gains `control: Receiver<Message>` after `inbox`. `run` takes `control: Receiver<Message>` after `inbox` and stores it. The main loop becomes:

```rust
        while !session.runtime.shutdown.load(Ordering::SeqCst) {
            // Input first: a command never waits behind output (ADR 0030).
            let message = match session.runtime.control.try_recv() {
                Ok(message) => Some(message),
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => {
                    match session.runtime.events.drain_remaining() {
                        Some(remaining) if remaining.is_zero() => break,
                        Some(remaining) => session.runtime.inbox.recv_timeout(remaining).ok(),
                        None => session.runtime.inbox.recv().ok(),
                    }
                }
            };
            let Some(message) = message else { break };
            if session.handle(message).is_break() {
                break;
            }
        }
```

(`use std::sync::mpsc::{Receiver, SyncSender, TryRecvError};`.) In `handle`, the line `next = self.runtime.inbox.try_recv().ok();` becomes:

```rust
            // Control first, then whatever output is already waiting. A
            // control message counts against the batch like any other.
            next = self
                .runtime
                .control
                .try_recv()
                .ok()
                .or_else(|| self.runtime.inbox.try_recv().ok());
```

In `apply`: `Message::CaptureRequested => {}` becomes `Message::CaptureRequested | Message::Wake => {}` (keep the comment). The `drain_accepted_output` loop reads only `inbox` and stays as it is: closing applies no commands.

The test at `worker/mod.rs:996` builds its own channels; add `let (_control, control_rx) = std::sync::mpsc::sync_channel(1);` and pass `control_rx` to `run`.

- [ ] **Step 5: Rewrite the regression against the new API and add the unit test**

In the test from Step 1 replace the loop body with:

```rust
        assert!(
            session
                .try_admit(TerminalCommand::Input(b"x".to_vec()))
                .unwrap()
                .is_accepted(),
            "keystroke {n} refused under an output flood"
        );
```

and `0..64` with `0..sprite_term::CONTROL_CAPACITY`. Change line 214's natural-exit probe to:

```rust
        let ended = matches!(
            session.try_admit(TerminalCommand::Capture),
            Ok(sprite_term::Admitted::Closed(_))
        );
```

Add to `session.rs` a unit test module:

```rust
#[cfg(test)]
mod admission_tests {
    use super::*;

    #[test]
    fn a_full_control_lane_hands_the_command_back_and_does_not_knock() {
        let (control, _control_rx) = mpsc::sync_channel(1);
        let (inbox, inbox_rx) = mpsc::sync_channel(1);
        assert!(admit(&control, &inbox, TerminalCommand::Capture).is_accepted());
        assert!(matches!(inbox_rx.try_recv(), Ok(worker::Message::Wake)));
        let second = admit(&control, &inbox, TerminalCommand::ClearSelection);
        assert!(matches!(second, Admitted::Saturated(TerminalCommand::ClearSelection)));
        assert!(inbox_rx.try_recv().is_err(), "a refused command must not knock");
    }

    #[test]
    fn a_closed_control_lane_reports_closed() {
        let (control, control_rx) = mpsc::sync_channel::<worker::Message>(1);
        let (inbox, _inbox_rx) = mpsc::sync_channel(1);
        drop(control_rx);
        assert!(matches!(
            admit(&control, &inbox, TerminalCommand::Capture),
            Admitted::Closed(TerminalCommand::Capture)
        ));
    }
}
```

- [ ] **Step 6: Fix the sprite-app callers so the workspace compiles**

`sprite-app` has two callers of `try_send`: `crates/sprite-app/src/terminal_view.rs:807` and the test at `terminal_view/theme.rs:534`. Task 6 redesigns both; for this task make the smallest change that compiles and keeps today's behaviour:

```rust
        match session.try_admit(command) {
            Ok(sprite_term::Admitted::Accepted) => true,
            Ok(sprite_term::Admitted::Saturated(_)) => {
                self.status = Some("send: the terminal command queue is full".into());
                self.admission_notice = true;
                let _ = self.retry_wake.force_send(());
                false
            }
            Ok(sprite_term::Admitted::Closed(_)) => {
                self.admission_closed = true;
                self.status = Some("send: the terminal worker ended".into());
                self.admission_notice = true;
                let _ = self.retry_wake.force_send(());
                false
            }
            Err(error) => {
                self.admission_closed = error.kind == sprite_term::ErrorKind::Ended;
                self.status = Some(error.to_string().into());
                self.admission_notice = true;
                let _ = self.retry_wake.force_send(());
                false
            }
        }
```

and in `theme.rs:534` the loop becomes:

```rust
                    match session.try_admit(TerminalCommand::CopySelection) {
                        Ok(sprite_term::Admitted::Accepted) => { accepted += 1; full_since = None; }
                        Ok(sprite_term::Admitted::Saturated(_)) => { /* body unchanged */ }
                        Ok(sprite_term::Admitted::Closed(_)) | Err(_) => panic!("unexpected refusal"),
                    }
```

- [ ] **Step 7: Run the suites and note what the lane changes**

Run: `PATH="$HOME/.cargo/bin:$PATH" cargo +1.97.1 test --workspace --locked --offline --no-fail-fast 2>&1 | tee /private/tmp/claude-502/-Users-davidlee/24ab0588-26c4-4436-8560-541b86578a1f/scratchpad/task4-tests.log; grep -E "^test .* FAILED|^failures:" /private/tmp/claude-502/-Users-davidlee/24ab0588-26c4-4436-8560-541b86578a1f/scratchpad/task4-tests.log`
Expected: sprite-term green including the new regression. In sprite-app, the tests that produced refusals by filling the output queue now fail, because that is the behaviour this task removes. They are, by name: `submission_regressions::{rejected_link_requests_recover_after_event_pressure, partially_refused_reload_reverts_actual_local_state, accepted_colors_revert_after_the_other_reload_groups_refuse, refused_resize_and_font_reload_retry_the_identical_layout, saturated_ui_submission_and_reload_are_visible_refusals}`, `tests::pressure::*` (two tests that fill the queue), `theme::tests` (the copy-selection and refused-reload cases), and `observation::panes::tests::saturated_observation_refuses_without_holding_the_ui_registry`. Confirm the failing set is a subset of these; any other failure is a bug in this task. Task 6 migrates them to a deterministic fixture.

- [ ] **Step 8: Commit**

```bash
git -C /Users/davidlee/Projects/Sprite/.worktrees/findings-ledger-and-admission add -A crates
git -C /Users/davidlee/Projects/Sprite/.worktrees/findings-ledger-and-admission commit -m "$(cat <<'EOF'
Admit commands through a control lane the worker drains first

GAP-C11: a keystroke was refused while a program flooded output because
the worker's one spare inbox slot was shared with capture requests.
Commands and history requests now wait on a 64-deep control lane that
the worker reads before output; a nonblocking offer answers with
Admitted and hands a refused command back. The sprite-app tests that
manufactured refusals by filling the output queue fail until the next
task gives them a deterministic fixture.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
EOF
)"
```

---

### Task 5: Two Surface lanes and a refused relay (R-A4, R-A5; GAP-C12, GAP-C09)

**Files:**
- Create: `crates/sprite-app/src/surface/lanes.rs`
- Modify: `crates/sprite-app/src/surface.rs` (add `pub(crate) mod lanes;` beside the other `mod` lines), `crates/sprite-app/src/surface/channel.rs:54-60,707-735,774-785,831-935,935-955`, `crates/sprite-app/src/workspace/mod.rs:137,218-224,288-297,346`, `crates/sprite-app/src/workspace/reload.rs:265-300`, `crates/sprite-app/src/observation/request.rs:318-333`, `docs/adr/0018-*.md`
- Test: `channel.rs` tests (harness at 996-1012, direct `serve_surface` calls at 1178, 1229, 1283, the reason assertion at ~1222 and 2076), `request.rs` tests, new GAP-C12 and GAP-C09 tests

**Interfaces:**
- Produces:
  ```rust
  // surface/lanes.rs
  pub(crate) const STREAM_CAPACITY: usize = 64;
  pub(crate) const ONE_SHOT_CAPACITY: usize = 16;
  #[derive(Clone)] pub struct SurfaceLanes { pub(crate) stream: Sender<SurfaceRequest>, pub(crate) one_shot: Sender<SurfaceRequest> }
  pub(crate) struct SurfaceReceivers { pub(crate) stream: Receiver<SurfaceRequest>, pub(crate) one_shot: Receiver<SurfaceRequest> }
  pub(crate) fn surface_lanes() -> (SurfaceLanes, SurfaceReceivers)
  pub(crate) fn surface_lanes_with(stream: usize, one_shot: usize) -> (SurfaceLanes, SurfaceReceivers)
  // channel.rs
  impl SurfaceEndpoint { pub fn open(key, lanes: SurfaceLanes) -> io::Result<Self>; pub fn open_in(directory, key, lanes: SurfaceLanes) -> io::Result<Self> }
  // workspace/reload.rs
  pub(crate) enum RelayError { Disconnected, Refused, Timeout, Applying }
  ```

- [ ] **Step 1: Write the failing GAP-C12 test**

In `channel.rs`'s `mod tests`, add (it uses the existing `Scratch`, `connect`, `line`, `open_message` helpers; the update line is the one the test at line ~1713 already writes):

```rust
    /// GAP-C12: one program streaming faster than the window drains must not
    /// refuse another program's Open.
    #[test]
    fn a_full_stream_lane_does_not_refuse_another_connections_open() {
        let scratch = Scratch::new();
        let (endpoint, receivers) = endpoint(&scratch);
        // Only the one-shot lane is served; the stream lane is left to fill.
        let one_shot = receivers.one_shot;
        let answering = std::thread::spawn(move || {
            while let Ok(request) = one_shot.recv_blocking() {
                if let SurfaceRequest::Open { reply, .. } = request {
                    assert!(reply.claim());
                    let _ = reply.send(Ok(()));
                } else {
                    panic!("only Opens reach the one-shot lane here");
                }
            }
        });
        let (mut first, mut first_reader) = connect(&endpoint);
        writeln!(first, "{} {}", endpoint.key_hex(), open_message(1)).expect("write");
        assert_eq!(line(&mut first_reader)["type"], "opened");
        for _ in 0..(crate::surface::lanes::STREAM_CAPACITY + 8) {
            writeln!(first, r#"{{"type":"update","description":{{"version":1,"root":{{"kind":"text","text":"hi"}}}}}}"#).expect("write");
        }
        let (mut second, mut second_reader) = connect(&endpoint);
        writeln!(second, "{} {}", endpoint.key_hex(), open_message(1)).expect("write");
        assert_eq!(
            line(&mut second_reader)["type"],
            "opened",
            "the second program's Open waited behind the first program's stream"
        );
        drop(second);
        drop(first);
        drop(endpoint);
        drop(receivers.stream);
        answering.join().expect("the answering thread finished");
    }
```

Before the lanes exist this test cannot compile (`receivers.one_shot`). To see the defect itself fail first, run the equivalent on the single channel: temporarily write the test against `endpoint()`'s current `(SurfaceEndpoint, Receiver)` return, with `let answering` serving that single receiver only for the first 1 request then sleeping 10 s; the second Open is refused `"this window did not answer in time"`. Run it once, record the refusal in the commit body, then replace it with the version above.

- [ ] **Step 2: Write the failing GAP-C09 tests**

In `observation/request.rs` tests, next to the existing `ask_window` timeout test (around line 625), add:

```rust
    #[test]
    fn a_refused_admission_says_the_window_is_busy_not_that_it_did_not_answer() {
        let (requests, _window) = async_channel::bounded(1);
        requests
            .try_send(ReloadRequest {
                what: ConfigVerb::Print,
                reply: Relayed::waiting(std::sync::mpsc::sync_channel(1).0).0,
                reply_connection: None,
            })
            .expect("fill the lane");
        let answer = ask_window(
            &requests,
            ConfigVerb::Reload,
            None,
            Patience { answer: Duration::from_millis(30), after_claim: Duration::from_secs(5) },
        );
        assert_eq!(answer, "this window is busy; nothing was changed");
    }
```

(`Relayed::waiting` is `pub(crate)` in `workspace::reload`; import it as the file's other tests import `Patience`.) In `channel.rs` tests, change the assertion in `a_surface_open_refused_admission_finishes_with_the_queue_still_full` from `NO_ANSWER` to `BUSY`.

- [ ] **Step 3: Run both to verify they fail**

Run: `PATH="$HOME/.cargo/bin:$PATH" cargo +1.97.1 test -p sprite-app --locked --offline a_refused_admission_says_the_window_is_busy a_surface_open_refused_admission`
Expected: the first FAILS (`did not answer in time; nothing was changed`), the second fails to compile (`BUSY` undefined) — define `const BUSY: &str = "this window is busy; nothing was changed";` under `NO_ANSWER` in `channel.rs` and re-run: it FAILS on the reason text.

- [ ] **Step 4: Implement `lanes.rs`**

```rust
//! Two lanes into the window for Surface requests (ADR 0030).
//!
//! A connection streams Updates, Grid and List operations for as long as it
//! lives and blocks on the stream lane when the window is behind. The
//! one-shot requests — Open, Capabilities, FocusPane, RegisterToken — are
//! answered once and must not queue behind another program's stream
//! (GAP-C12), so they have a lane of their own. Order within a connection
//! holds because a connection sends Open, waits for the answer, then streams.

use crate::surface::channel::SurfaceRequest;

/// One wave of streaming programs; a connection past it blocks, which is
/// its own thread's problem and nobody else's.
pub(crate) const STREAM_CAPACITY: usize = 64;

/// Handshakes in flight at once. Past this, a program is refused as busy
/// and reconnects; it never waits.
pub(crate) const ONE_SHOT_CAPACITY: usize = 16;

/// The connection threads' ends.
#[derive(Clone)]
pub struct SurfaceLanes {
    pub(crate) stream: async_channel::Sender<SurfaceRequest>,
    pub(crate) one_shot: async_channel::Sender<SurfaceRequest>,
}

/// The window's ends, each drained by its own task.
pub(crate) struct SurfaceReceivers {
    pub(crate) stream: async_channel::Receiver<SurfaceRequest>,
    pub(crate) one_shot: async_channel::Receiver<SurfaceRequest>,
}

pub(crate) fn surface_lanes() -> (SurfaceLanes, SurfaceReceivers) {
    surface_lanes_with(STREAM_CAPACITY, ONE_SHOT_CAPACITY)
}

pub(crate) fn surface_lanes_with(stream: usize, one_shot: usize) -> (SurfaceLanes, SurfaceReceivers) {
    let (stream_tx, stream_rx) = async_channel::bounded(stream);
    let (one_shot_tx, one_shot_rx) = async_channel::bounded(one_shot);
    (
        SurfaceLanes { stream: stream_tx, one_shot: one_shot_tx },
        SurfaceReceivers { stream: stream_rx, one_shot: one_shot_rx },
    )
}
```

- [ ] **Step 5: Route the endpoint through the lanes**

In `channel.rs`:
- `SurfaceEndpoint::open` and `open_in` take `lanes: SurfaceLanes` instead of `requests`; the closure becomes `move |connection| converse(connection, &lanes)`.
- `converse(connection: Authenticated, lanes: &SurfaceLanes)`: the three `one_shot(&mut stream, requests, …)` calls pass `&lanes.one_shot`; the Open arm calls `serve_surface(stream, reader, connection, pane, open, lanes)`.
- `serve_surface(…, lanes: &SurfaceLanes)`: `relay(&lanes.one_shot, REPLY_PATIENCE, |reply| SurfaceRequest::Open { … })`; add the arm
  ```rust
        // The handshake lane is full of other programs' handshakes. Refuse
        // and close, as any refused Open is; the program reconnects.
        Err(RelayError::Refused) => {
            handle.abandon();
            refuse(&mut stream, BUSY);
            return;
        }
  ```
  and every `requests.send_blocking(...)` / `requests.try_send(SurfaceRequest::Closed …)` in the function uses `lanes.stream`.
- `one_shot(stream, requests: &async_channel::Sender<SurfaceRequest>, …)` keeps its signature (callers pass the one-shot sender) and gains `Err(RelayError::Refused) => refuse(stream, BUSY),` before the `Timeout | Applying` arm.

In `workspace/reload.rs`, `RelayError` gains:

```rust
    /// The lane was full, so the request never reached the window. Nothing
    /// was applied, and the asker may say so.
    Refused,
```

and `relay`'s `Full` arm returns `Err(RelayError::Refused)` (still calling `claim.abandon()` first). Update `Timeout`'s doc to "The window did not claim the request in time and now never will, so nothing was applied."

In `observation/request.rs::ask_window` add `Err(RelayError::Refused) => "this window is busy; nothing was changed".to_owned(),`.

In `workspace/mod.rs`:
- line 218: `let (surface_lanes, surface_receivers) = crate::surface::lanes::surface_lanes();` and `SurfaceEndpoint::open(key, surface_lanes)`.
- lines 288-297 become two tasks through one helper:
  ```rust
        let crate::surface::lanes::SurfaceReceivers { stream, one_shot } = surface_receivers;
        let surface_tasks = [
            Self::drain_surface_lane(stream, window, cx),
            Self::drain_surface_lane(one_shot, window, cx),
        ];
  ```
  with, in `impl Workspace`:
  ```rust
    /// Serves one lane of Surface requests until the window or the lane ends.
    fn drain_surface_lane(
        lane: async_channel::Receiver<SurfaceRequest>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::Task<()> {
        cx.spawn_in(window, async move |workspace, cx| {
            while let Ok(request) = lane.recv().await {
                let served = workspace.update_in(cx, |workspace, window, cx| {
                    workspace.serve_surface_request(request, window, cx);
                });
                if served.is_err() {
                    break;
                }
            }
        })
    }
  ```
- field `_surface_requests: gpui::Task<()>` becomes `_surface_requests: [gpui::Task<()>; 2]`, initialised from `surface_tasks`.

Test harness in `channel.rs`:
- `fn endpoint(scratch) -> (SurfaceEndpoint, SurfaceReceivers)` builds `surface_lanes_with(8, 8)`.
- `fn window(receivers: SurfaceReceivers, script)` forwards both lanes into one `std::sync::mpsc` channel from two forwarding threads, then runs the script on that channel, so every existing scripted test keeps its order-insensitive behaviour:
  ```rust
    fn window(
        receivers: SurfaceReceivers,
        mut script: impl FnMut(SurfaceRequest) -> bool + Send + 'static,
    ) -> std::thread::JoinHandle<()> {
        let (merged, requests) = mpsc::channel();
        for lane in [receivers.stream, receivers.one_shot] {
            let merged = merged.clone();
            std::thread::spawn(move || {
                while let Ok(request) = lane.recv_blocking() {
                    if merged.send(request).is_err() {
                        break;
                    }
                }
            });
        }
        drop(merged);
        std::thread::spawn(move || {
            while let Ok(request) = requests.recv() {
                if !script(request) {
                    break;
                }
            }
        })
    }
  ```
- The three direct `serve_surface(...)` tests (1178, 1229, 1283) build `let (lanes, receivers) = surface_lanes_with(1, 1);`, prefill `lanes.one_shot` where they prefilled `requests` (the "queue still full" tests), read answers from `receivers.one_shot` / `receivers.stream` where they read `window`, and pass `&lanes`.
- `open_in` call at 1445 passes lanes.

- [ ] **Step 6: Amend ADR 0018**

In `docs/adr/0018-keep-the-surface-channel-separate-from-pane-observation.md` (the file's actual name under `docs/adr/0018-*`), replace the paragraph beginning "The generic workspace reply relay retains the existing queue submission and reply-wait semantics: blocking enqueue, …" with:

```markdown
The generic workspace reply relay admits without blocking: the connection
thread offers its request to a bounded lane and, if the lane is full, reports
"this window is busy; nothing was changed" at once (amended 2026-10-12,
GAP-C09; the earlier text described a blocking enqueue that the code never
had). Surface requests use two lanes (ADR 0030): a stream lane that a
connection blocks on, and a one-shot lane for handshakes that must not queue
behind another program's stream. Its reply deadline (5 seconds for Surface,
the existing configuration timeout for config) does not cover arbitrary
adapter work. Closing interrupts socket I/O; it does not join workers awaiting
the GPUI queue/reply. Accepted workers remain bounded by each listener's
connection cap. No new timer thread, polling animation, or dependency is
introduced.
```

- [ ] **Step 7: Run tests**

Run: `PATH="$HOME/.cargo/bin:$PATH" cargo +1.97.1 test -p sprite-app --locked --offline --no-fail-fast surface:: workspace:: observation::`
Expected: all pass including the two new tests. (Task 4's known failures in `terminal_view` remain until Task 6.)

- [ ] **Step 8: Commit**

```bash
git -C /Users/davidlee/Projects/Sprite/.worktrees/findings-ledger-and-admission add -A crates/sprite-app docs/adr
git -C /Users/davidlee/Projects/Sprite/.worktrees/findings-ledger-and-admission commit -m "$(cat <<'EOF'
Give Surface handshakes their own lane and name a refused relay

GAP-C12: one program streaming updates kept the shared 64-slot request
queue full, so every other program's Open was refused. GAP-C09: a relay
refused at admission said the window "did not answer in time"; ADR 0018
described a blocking enqueue the code never had.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
EOF
)"
```

---

### Task 6: One `Pending`, one `PaneFocus`, and deterministic saturation in tests (R-A3, R-F1)

**Files:**
- Create: `crates/sprite-app/src/terminal_view/pane_focus.rs`
- Modify: `crates/sprite-app/src/terminal_view.rs:47-58,146-152,470-530,560-625,686,717-870`, `crates/sprite-app/src/terminal_view/theme.rs:268-330,525-570`, `crates/sprite-app/src/terminal_view/geometry.rs:140-149`, `crates/sprite-app/src/test_event_pressure.rs`, `crates/sprite-app/src/test_blocking_wait.rs`
- Test: `crates/sprite-app/src/terminal_view/submission_regressions.rs`, `crates/sprite-app/src/terminal_view/tests/pressure.rs`, `crates/sprite-app/src/terminal_view/tests.rs:708`, `crates/sprite-app/src/observation/panes.rs:499-540`, new `pane_focus.rs` unit tests

**Interfaces:**
- Produces, in `terminal_view.rs`:
  ```rust
  pub(crate) const BUSY_STATUS: &str = "the terminal is busy; the last command was not delivered";
  pub(crate) const ENDED_STATUS: &str = "the terminal has ended";
  #[derive(Default)] struct Pending { settings: Option<Settings>, resize: Option<ValidTerminalSize> }
  impl Pending { fn clear(&mut self); fn is_empty(&self) -> bool }
  impl TerminalView {
      fn admit(&mut self, command: TerminalCommand) -> Result<Admitted<TerminalCommand>, SessionError>;
      fn submit(&mut self, command: TerminalCommand) -> bool;   // = admit(..) is Ok(Accepted)
      fn admit_focus(&mut self);                                 // no argument now
      pub(crate) fn pane_focused(&self) -> bool;                  // unchanged read
  }
  ```
- Produces, in `pane_focus.rs`:
  ```rust
  #[derive(Default)] pub(super) struct PaneFocus { held: bool, told: bool, pending: Option<bool> }
  impl PaneFocus {
      pub(super) fn compute(window: &Window, focus: &FocusHandle, cx: &App) -> bool;
      pub(super) fn held(&self) -> bool;
      pub(super) fn update(&mut self, held: bool) -> Option<bool>;   // Some(new) when it changed
      pub(super) fn owed(&mut self) -> Option<bool>;                  // what the worker still has to hear; clears pending
      pub(super) fn delivered(&mut self, focused: bool);
      pub(super) fn refused(&mut self, focused: bool);
      pub(super) fn pending(&self) -> Option<bool>;
      pub(super) fn forget_pending(&mut self);
  }
  ```
- Produces, in `test_event_pressure.rs`: `pub(crate) fn saturate_control(commands: &CommandSender)`; in `test_blocking_wait.rs`: `pub(crate) fn watchdog() -> Duration`.

- [ ] **Step 1: Write the failing `PaneFocus` unit tests**

Create `crates/sprite-app/src/terminal_view/pane_focus.rs`:

```rust
//! Pane Focus has one owner: what the window computed, what the worker was
//! told, and what it is still owed.

use gpui::{App, FocusHandle, Window};

#[derive(Default)]
pub(super) struct PaneFocus {
    /// Whether this Pane has Pane Focus (see `crates/CONTEXT.md`).
    held: bool,
    /// The value the worker last accepted.
    told: bool,
    /// A value the worker refused, kept so the latest one is delivered when
    /// room returns.
    pending: Option<bool>,
}

impl PaneFocus {
    /// Pane Focus as the window sees it: the handle *containing* keyboard
    /// focus is enough, because a Surface the pane hosts is part of the pane.
    pub(super) fn compute(window: &Window, focus: &FocusHandle, cx: &App) -> bool {
        window.is_window_active() && focus.contains_focused(window, cx)
    }

    pub(super) fn held(&self) -> bool {
        self.held
    }

    /// Records the computed value; `Some(new)` when it changed.
    pub(super) fn update(&mut self, held: bool) -> Option<bool> {
        todo!()
    }

    /// What the worker still has to hear, or `None` when it already has.
    /// Clears any kept refusal: the caller is about to deliver or re-keep.
    pub(super) fn owed(&mut self) -> Option<bool> {
        todo!()
    }

    pub(super) fn delivered(&mut self, focused: bool) {
        todo!()
    }

    pub(super) fn refused(&mut self, focused: bool) {
        todo!()
    }

    pub(super) fn pending(&self) -> Option<bool> {
        self.pending
    }

    pub(super) fn forget_pending(&mut self) {
        self.pending = None;
    }
}

#[cfg(test)]
mod tests {
    use super::PaneFocus;

    #[test]
    fn a_change_undone_before_delivery_owes_the_worker_nothing() {
        let mut focus = PaneFocus::default();
        assert_eq!(focus.update(true), Some(true));
        assert_eq!(focus.update(false), Some(false));
        assert_eq!(focus.owed(), None, "the worker was told false at the start");
    }

    #[test]
    fn a_refused_value_is_kept_until_delivered() {
        let mut focus = PaneFocus::default();
        assert_eq!(focus.update(true), Some(true));
        assert_eq!(focus.owed(), Some(true));
        focus.refused(true);
        assert_eq!(focus.pending(), Some(true));
        assert_eq!(focus.owed(), Some(true), "still owed");
        focus.delivered(true);
        assert_eq!(focus.pending(), None);
        assert_eq!(focus.owed(), None);
    }

    #[test]
    fn an_unchanged_value_is_not_a_change() {
        let mut focus = PaneFocus::default();
        assert_eq!(focus.update(false), None);
    }
}
```

Add `mod pane_focus;` to `terminal_view.rs` beside the other `mod` lines (`mod theme;`, `mod geometry;` …).

- [ ] **Step 2: Run to verify failure**

Run: `PATH="$HOME/.cargo/bin:$PATH" cargo +1.97.1 test -p sprite-app --locked --offline pane_focus::tests`
Expected: FAIL with `not yet implemented`.

- [ ] **Step 3: Implement `PaneFocus`**

```rust
    pub(super) fn update(&mut self, held: bool) -> Option<bool> {
        if held == self.held {
            return None;
        }
        self.held = held;
        Some(held)
    }

    pub(super) fn owed(&mut self) -> Option<bool> {
        self.pending = None;
        (self.held != self.told).then_some(self.held)
    }

    pub(super) fn delivered(&mut self, focused: bool) {
        self.told = focused;
        self.pending = None;
    }

    pub(super) fn refused(&mut self, focused: bool) {
        self.pending = Some(focused);
    }
```

- [ ] **Step 4: Rewrite the view's admission state**

In `terminal_view.rs`:

Replace the fields `pending_settings`, `pending_resize`, `pending_focus`, `told_focus` (lines 49-56) with:

```rust
    /// What the worker could not take yet, kept so the latest value is
    /// delivered when room returns.
    pending: Pending,
```

Replace the field `pane_focused: bool` (line 150) with `pane_focus: pane_focus::PaneFocus,` and rename `_pane_focus: [gpui::Subscription; 3]` to `_focus_observers`. Add near the top of the file:

```rust
pub(crate) const BUSY_STATUS: &str = "the terminal is busy; the last command was not delivered";
pub(crate) const ENDED_STATUS: &str = "the terminal has ended";

/// What the worker could not take yet. Settings keep the whole reload,
/// because its groups are admitted separately and a partial reload is
/// reverted as one.
#[derive(Default)]
struct Pending {
    settings: Option<crate::config::Settings>,
    resize: Option<sprite_term::ValidTerminalSize>,
}

impl Pending {
    fn clear(&mut self) {
        *self = Self::default();
    }

    fn is_empty(&self) -> bool {
        self.settings.is_none() && self.resize.is_none()
    }
}
```

Both constructors (`new` around line 480 and `failed` around line 569) initialise `pending: Pending::default()`, `pane_focus: pane_focus::PaneFocus::default()`, `_focus_observers: pane_focus` (rename the local too).

`refresh_pane_focus` becomes:

```rust
    fn refresh_pane_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let held = pane_focus::PaneFocus::compute(window, &self.focus, cx);
        let Some(focused) = self.pane_focus.update(held) else {
            return;
        };
        self.admit_focus();
        self.blink_on = true;
        if !focused {
            self.drop_unsafe_paste(cx);
        }
        cx.notify();
    }

    /// Tells the worker the latest Pane Focus, or keeps it for the retry task
    /// when the lane is full.
    fn admit_focus(&mut self) {
        let Some(focused) = self.pane_focus.owed() else {
            return;
        };
        match self.admit(TerminalCommand::Focus(focused)) {
            Ok(Admitted::Accepted) => self.pane_focus.delivered(focused),
            Ok(Admitted::Saturated(_)) => self.pane_focus.refused(focused),
            Ok(Admitted::Closed(_)) | Err(_) => self.pane_focus.forget_pending(),
        }
    }

    pub(crate) fn pane_focused(&self) -> bool {
        self.pane_focus.held()
    }
```

(Import `sprite_term::{Admitted, ErrorKind}`.) Line 686 becomes `Effect::ChildClipboard(_) if !self.pane_focus.held() => false,`. `begin_shutdown` replaces the three `None` assignments with `self.pending.clear(); self.pane_focus.forget_pending();`.

`submit`/`admit`:

```rust
    fn send(&mut self, command: TerminalCommand) {
        let _accepted = self.submit(command);
    }

    fn submit(&mut self, command: TerminalCommand) -> bool {
        matches!(self.admit(command), Ok(Admitted::Accepted))
    }

    /// Offers a command to the worker. A saturated or closed lane hands the
    /// command back so the caller can keep what is worth keeping; an `Err`
    /// is the command itself being refused.
    fn admit(
        &mut self,
        command: TerminalCommand,
    ) -> Result<Admitted<TerminalCommand>, SessionError> {
        let SessionState::Running(session) = &mut self.session else {
            return Ok(Admitted::Accepted);
        };
        let outcome = session.try_admit(command);
        match &outcome {
            Ok(Admitted::Accepted) => {}
            Ok(Admitted::Saturated(_)) => self.refuse(BUSY_STATUS),
            Ok(Admitted::Closed(_)) => {
                self.admission_closed = true;
                self.refuse(ENDED_STATUS);
            }
            Err(error) => {
                self.admission_closed = error.kind == ErrorKind::Ended;
                self.refuse(&error.to_string());
            }
        }
        outcome
    }

    fn refuse(&mut self, status: &str) {
        self.status = Some(status.to_owned().into());
        self.admission_notice = true;
        self.retry_wake.force_send(()).ok();
    }
```

(`SessionError` is imported from `sprite_term`; `let _accepted` names the ignored bool, which Task 13's lint accepts.)

`spawn_retry`'s inner closure becomes:

```rust
                        if view.admission_closed
                            || !matches!(view.session, SessionState::Running(_))
                        {
                            view.pending.clear();
                            view.pane_focus.forget_pending();
                            return None;
                        }
                        if let Some(settings) = view.pending.settings.take() {
                            view.apply_settings(&settings, window, cx);
                        }
                        if let Some(size) = view.pending.resize {
                            view.admit_resize(size);
                        }
                        if view.pane_focus.pending().is_some() {
                            view.admit_focus();
                        }
                        Some(!view.pending.is_empty() || view.pane_focus.pending().is_some())
```

`geometry.rs::admit_resize`:

```rust
    pub(super) fn admit_resize(&mut self, size: ValidTerminalSize) {
        match self.admit(TerminalCommand::Resize(size)) {
            Ok(sprite_term::Admitted::Accepted) => {
                self.size = Some(size);
                self.pending.resize = None;
            }
            Ok(sprite_term::Admitted::Saturated(_)) => self.pending.resize = Some(size),
            Ok(sprite_term::Admitted::Closed(_)) | Err(_) => self.pending.resize = None,
        }
    }
```

`theme.rs::apply_settings`: the two `self.submit(...)` calls stay; the final assignment becomes `self.pending.settings = (!self.admission_closed && !admitted.diff(settings).live.is_empty()).then(|| settings.clone());`.

Every other `pending_settings` / `pending_resize` / `pending_focus` / `told_focus` / `pane_focused` *field* reference in the crate (grep `pending_settings\|pending_resize\|pending_focus\|told_focus\|\.pane_focused\b`) moves to `pending.settings`, `pending.resize`, `pane_focus.pending()`, and `pane_focus.held()`; the tests are rewritten in Step 5.

- [ ] **Step 5: The deterministic saturation fixture, and the test migration**

`test_blocking_wait.rs`:

```rust
//! Blocking coordination keeps real worker timing independent of the paused GPUI test scheduler.

#![cfg(test)]

use std::time::Duration;

pub(crate) fn pause(duration: Duration) {
    std::thread::sleep(duration);
}

/// How long a test waits for a condition before failing: five seconds, or
/// `SPRITE_TEST_WATCHDOG_SECS` where a loaded runner needs longer.
pub(crate) fn watchdog() -> Duration {
    std::env::var("SPRITE_TEST_WATCHDOG_SECS")
        .ok()
        .and_then(|seconds| seconds.parse().ok())
        .map_or(Duration::from_secs(5), Duration::from_secs)
}
```

`test_event_pressure.rs` gains:

```rust
/// Fills the worker's control lane so the next command is refused, and
/// returns once it is. The lane only stays full while the worker is stalled
/// by event pressure; until it is, the worker drains what this sends, so
/// this keeps trying within the watchdog instead of pausing and hoping.
pub(crate) fn saturate_control(commands: &CommandSender) {
    let deadline = std::time::Instant::now() + crate::test_blocking_wait::watchdog();
    loop {
        for _ in 0..=sprite_term::CONTROL_CAPACITY {
            if let sprite_term::Admitted::Saturated(_) = commands
                .try_admit(TerminalCommand::Capture)
                .expect("a capture is always valid")
            {
                return;
            }
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the control lane never saturated: the worker is not stalled"
        );
        crate::test_blocking_wait::pause(std::time::Duration::from_millis(10));
    }
}
```

Migration, test by test. The rule: wherever a test *filled the output queue by pausing* to produce a refusal, it now calls `saturate_control(&session.commands())` (or the view-level loop below) after `focus_and_release`, and the `pause(300 ms)` that only existed to let output build up is deleted; wherever it asserted `status.contains("queue is full")`, it asserts `status.contains(BUSY_STATUS)`.

`submission_regressions.rs`:
- `rejected_link_requests_recover_after_event_pressure` (15): after `focus_and_release`, replace `pause(300)` + `snapshots.next_blocking()` with `saturate_control(&session.commands());`. Keep the rest.
- `partially_refused_reload_reverts_actual_local_state` (151): same replacement; `"queue is full"` → `BUSY_STATUS`. Its intent (renderer state applied locally while the worker refused) holds under full saturation, so nothing else changes.
- `accepted_colors_revert_after_the_other_reload_groups_refuse` (235): the "reserved slot accepts one group" mechanics no longer exist, and the one-group case (exactly one free lane slot when the reload runs) cannot be produced on purpose: a free slot is only observable by taking it. It remains reachable in production under a race, which is why `apply_settings` keeps its per-group `admitted` bookkeeping. Rewrite the test's first block so that, with the lane saturated, a reload leaves `applied_settings` as before and keeps the reload whole and pending, then once pressure clears the retry delivers it:
  ```rust
        crate::test_event_pressure::saturate_control(&session.commands());
        let mut reloaded = settings.clone();
        reloaded.colors.foreground = Some(changed_color);
        reloaded.cursor.blink = Some(false);
        view.update_in(cx, |view, window, cx| {
            view.session = SessionState::Running(session);
            view.applied_settings = settings.clone();
            view.apply_settings(&reloaded, window, cx);
            assert!(view.status.as_ref().is_some_and(|status| status.contains(BUSY_STATUS)));
            assert_eq!(view.applied_settings.colors, settings.colors, "nothing was admitted");
            assert_eq!(view.applied_settings.cursor.blink, settings.cursor.blink);
            assert_eq!(view.pending.settings.as_ref(), Some(&reloaded), "the reload is kept whole");
            view.apply_settings(&settings, window, cx);
            assert!(view.pending.settings.is_none(), "reverting to what is applied owes nothing");
        });
  ```
  Delete the `for accept_colors in [true, false]` loop (one pass), keep the event/snapshot drains and the history probes that follow, and keep the final `unchanged.generation == settled.generation` assertion. Rename the test `a_refused_reload_is_kept_whole_and_reverted_whole`.
- `refused_resize_and_font_reload_retry_the_identical_layout` (396): replace `pause(300)` with `saturate_control(&session.commands());` and keep `let initial = snapshots.next_blocking().unwrap();` after it (the test needs `initial.pane.size`). `"queue is full"` → `BUSY_STATUS`. The `wait_for_text` closures in this file use `Duration::from_secs(5)`: replace with `crate::test_blocking_wait::watchdog()`.
- `saturated_ui_submission_and_reload_are_visible_refusals` (513): same replacement; `"queue is full"` → `BUSY_STATUS`.

`tests/pressure.rs`:
- Add at the top of the file:
  ```rust
  /// Takes every control-lane slot a stalled worker left, so the next command
  /// is refused. True when it was.
  fn saturate(view: &mut TerminalView) -> bool {
      (0..=sprite_term::CONTROL_CAPACITY).any(|_| !view.submit(TerminalCommand::Capture))
  }
  ```
- The test around lines 60-100: after `std::fs::write(&gate, b"go")` and the 750 ms pause (keep that pause: it lets the burst raise the events that stall the worker, and more time only means more pressure), add `view.update(cx, |view, _| assert!(saturate(view), "the fixture never filled the lane"));`. Then the reload block asserts: status contains `BUSY_STATUS`; `applied_settings.colors == settings.colors` and `applied_settings.cursor == settings.cursor` (neither group admitted); `view.pending.settings.is_some()`; after `apply_settings(&settings)`, `view.pending.settings.is_none()`. Drop the "reserved command slot accepted colors" assertions.
- Lines 258-270: replace the `for _ in 0..64 { submit(ClearSelection) }` loop with `let refused = saturate(view);`. Line 274 `view.pending_focus` → `view.pane_focus.pending()`; `"queue is full"` → `BUSY_STATUS`.
- Lines 495-515: same loop replacement; `"queue is full"` → `BUSY_STATUS`.

`tests.rs:708`: `"queue is full"` → `BUSY_STATUS`.

Add to `tests/pressure.rs` the PRD's second R-A6 evidence, a view-level flood with the view draining snapshots and events as in production:

```rust
/// R-A6: across a sustained output flood no keystroke is refused at the
/// view. Before the control lane this depended on whether the worker kept
/// up with the pump; now a keystroke never competes with output.
#[gpui::test]
fn no_keystroke_is_refused_under_a_sustained_output_flood(cx: &mut gpui::TestAppContext) {
    let settings = crate::config::Settings::default();
    cx.set_global(crate::config::ActiveSettings(settings.clone()));
    cx.set_global(crate::tokens::TokenRegistry::new(&settings.colors));
    let (sender, _exits) = async_channel::unbounded();
    let (view, cx) = cx.add_window_view(|window, cx| {
        TerminalView::new(
            Some(vec!["/bin/sh".into(), "-c".into(), "yes".into()]),
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
        bundle.pane.rows.iter().any(|row| row.text.starts_with('y'))
    });
    // `yes` never reads, so stay under the kernel's 1 KiB input queue: the
    // pump would otherwise report a write refusal, which is a different fact.
    for n in 0..300 {
        view.update(cx, |view, _| {
            assert!(
                view.submit(TerminalCommand::Input(b"x".to_vec())),
                "keystroke {n} refused under a flood: {:?}",
                view.status
            );
        });
        cx.run_until_parked();
        crate::test_blocking_wait::pause(std::time::Duration::from_millis(5));
    }
}
```

Run it once against `d699029` by checking that commit out in a second worktree (`git worktree add /tmp/sprite-d699029 d699029`, same zig-cache seeding as this worktree) with `submit` reading `try_send`: it fails only if the worker fell behind the pump during the run. Whatever it does there, record it; the GAP-C11 regression is the sprite-term test in Task 4, and this one guards the view.

`theme.rs` tests (525-570): the copy-selection loop was rewritten in Task 4; now delete its `full_since` timing and keep: loop `try_admit(CopySelection)` until `Saturated`, with `accepted > 40` as the floor. `"queue is full"` → `BUSY_STATUS`; `view.pending_settings` → `view.pending.settings`.

`observation/panes.rs::saturated_observation_refuses_without_holding_the_ui_registry` (499): replace `pause(300)` + `snapshots.next_blocking()` with `crate::test_event_pressure::saturate_control(&spawned.session.commands());`. The refusal text `"queue is full"` is `request_history`'s own and stays.

- [ ] **Step 6: Run the application suite**

Run: `PATH="$HOME/.cargo/bin:$PATH" cargo +1.97.1 test -p sprite-app --locked --offline --no-fail-fast`
Expected: all pass, including every test Task 4 left red. Then run the two formerly flaky tests twenty times under load:

```bash
for core in $(seq "$(sysctl -n hw.ncpu)"); do yes > /dev/null & done
for i in $(seq 20); do
  PATH="$HOME/.cargo/bin:$PATH" cargo +1.97.1 test -p sprite-app --locked --offline a_refused_reload_is_kept_whole_and_reverted_whole refused_resize_and_font_reload_retry_the_identical_layout -- --exact --nocapture > /dev/null 2>&1 || echo "run $i FAILED"
done
kill %1 %2 %3 %4 %5 %6 %7 %8 2>/dev/null; jobs -p | xargs kill 2>/dev/null
```

Expected: no `FAILED` line. Record the result for Task 14.

- [ ] **Step 7: Commit**

```bash
git -C /Users/davidlee/Projects/Sprite/.worktrees/findings-ledger-and-admission add -A crates/sprite-app
git -C /Users/davidlee/Projects/Sprite/.worktrees/findings-ledger-and-admission commit -m "$(cat <<'EOF'
Keep one Pending and one PaneFocus on the view, and saturate tests on purpose

The view decides what to keep from what Admitted hands back, never from
error text. Pane Focus has one owner. Tests that manufactured refusals by
pausing until output filled the queue now fill the control lane and
return when it is full, which is a condition, not a schedule.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
EOF
)"
```

---

### Task 7: Related timeouts in one module (R-T; GAP-C13)

**Files:**
- Create: `crates/sprite-app/src/timeouts.rs`
- Modify: `crates/sprite-app/src/lib.rs` (add `mod timeouts;`), `crates/sprite-app/src/surface/channel.rs:43-57`, `crates/sprite-app/src/workspace/reload.rs:141`, `crates/sprite-app/src/observation/broker.rs:36`, `crates/sprite-app/src/observation/endpoint.rs:47`, `crates/sprite-app/src/surface/client.rs:32`, `crates/sprite-app/src/observation/client.rs:30`, `crates/sprite-term/src/worker/closing.rs:9-14`, `crates/sprite-term/src/event_mailbox.rs:140`
- Test: unit tests in `timeouts.rs` and `closing.rs`

**Interfaces:**
- Produces: `crate::timeouts::{REPLY_TIMEOUT, AFTER_CLAIM, HANDSHAKE_TIMEOUT, WRITE_TIMEOUT, BROKER_DEADLINE, SILENT_CLIENT_TIMEOUT, CLIENT_TIMEOUT}` all `pub(crate) const Duration`; `crate::worker::closing::GIVE_UP_AFTER` becomes `pub(crate)`.

- [ ] **Step 1: Write the failing tests**

Create `crates/sprite-app/src/timeouts.rs`:

```rust
//! Every duration that stands in a relation to another, with the relation
//! checked when the crate compiles.
//!
//! GAP-C13: the clients gave up at 15 s while the window's "still applying"
//! answer could arrive at 5 s + 10 s measured from a later moment, so the
//! client never saw it. A client now always outlives the server's two waits.

use std::time::Duration;

/// How long a connection waits for the window to answer a request.
pub(crate) const REPLY_TIMEOUT: Duration = Duration::from_secs(5);

/// How much longer an asker waits once the window has claimed its request.
pub(crate) const AFTER_CLAIM: Duration = Duration::from_secs(10);

/// A client that connects and sends no first line for this long has its
/// thread taken back. Short under test so handshake tests finish quickly.
#[cfg(not(test))]
pub(crate) const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
#[cfg(test)]
pub(crate) const HANDSHAKE_TIMEOUT: Duration = Duration::from_millis(200);

/// A client that will not accept an event for this long is treated as gone.
#[cfg(not(test))]
pub(crate) const WRITE_TIMEOUT: Duration = Duration::from_secs(2);
#[cfg(test)]
pub(crate) const WRITE_TIMEOUT: Duration = Duration::from_millis(200);

/// How long the observation broker waits for every pane to answer.
pub(crate) const BROKER_DEADLINE: Duration = Duration::from_millis(500);

/// How long the observation endpoint tolerates a connected client that
/// sends nothing. Not a client's patience: the server's.
pub(crate) const SILENT_CLIENT_TIMEOUT: Duration = Duration::from_secs(2);

/// A client's patience for one exchange: the window's reply wait, its
/// after-claim wait, and a second for the socket.
pub(crate) const CLIENT_TIMEOUT: Duration = REPLY_TIMEOUT
    .saturating_add(AFTER_CLAIM)
    .saturating_add(Duration::from_secs(1));

const fn longer(a: Duration, b: Duration) -> bool {
    a.as_millis() > b.as_millis()
}

const _: () = assert!(
    longer(CLIENT_TIMEOUT, REPLY_TIMEOUT.saturating_add(AFTER_CLAIM)),
    "a client must outlive the window's reply and after-claim waits"
);
const _: () = assert!(longer(CLIENT_TIMEOUT, HANDSHAKE_TIMEOUT));
const _: () = assert!(longer(CLIENT_TIMEOUT, WRITE_TIMEOUT));
const _: () = assert!(longer(REPLY_TIMEOUT, BROKER_DEADLINE));
const _: () = assert!(longer(REPLY_TIMEOUT, SILENT_CLIENT_TIMEOUT));

#[cfg(test)]
mod tests {
    use super::*;

    /// GAP-C13. The compile-time asserts above are the guard; this names it.
    #[test]
    fn a_client_outlives_the_windows_reply_and_after_claim_waits() {
        assert!(CLIENT_TIMEOUT > REPLY_TIMEOUT + AFTER_CLAIM);
    }
}
```

In `crates/sprite-term/src/worker/closing.rs` after the three constants add:

```rust
// Both measured from the moment shutdown was requested: TERM is owed first,
// KILL after it, and cleanup gives up only after KILL had its chance.
const _: () = assert!(TERM_AFTER.as_millis() < KILL_AFTER.as_millis());
const _: () = assert!(KILL_AFTER.as_millis() < GIVE_UP_AFTER.as_millis());
```

and make `GIVE_UP_AFTER` `pub(crate)`. In the existing `closing.rs` tests module add:

```rust
    #[test]
    fn escalation_fits_inside_the_give_up_budget() {
        assert!(TERM_AFTER < KILL_AFTER && KILL_AFTER < GIVE_UP_AFTER);
    }
```

- [ ] **Step 2: Run to verify the GAP-C13 test is meaningful**

Temporarily set `CLIENT_TIMEOUT` to `Duration::from_secs(15)` (today's value) and run `PATH="$HOME/.cargo/bin:$PATH" cargo +1.97.1 build -p sprite-app --locked --offline`. Expected: compile error from the first `const _` assert. Restore the formula.

- [ ] **Step 3: Point every site at the module**

- `channel.rs:43-57`: delete the local `WRITE_TIMEOUT`, `HANDSHAKE_TIMEOUT`, `REPLY_TIMEOUT` definitions; `use crate::timeouts::{HANDSHAKE_TIMEOUT, REPLY_TIMEOUT, WRITE_TIMEOUT};`. `REPLY_PATIENCE` stays.
- `reload.rs:141`: delete `AFTER_CLAIM`; `use crate::timeouts::AFTER_CLAIM;` (it is `pub(crate)` there already; nothing else imports it from `reload`).
- `broker.rs:36`: `pub const DEADLINE: Duration = crate::timeouts::BROKER_DEADLINE;` (the bench binary keeps its own name).
- `endpoint.rs:47`: delete `CLIENT_TIMEOUT`; `use crate::timeouts::SILENT_CLIENT_TIMEOUT;` and rename its uses.
- `surface/client.rs:32` and `observation/client.rs:30`: delete `TIMEOUT`; `use crate::timeouts::CLIENT_TIMEOUT;` and rename uses (keep the doc comment about megabyte responses on the observation one as a comment above the `use`).
- `event_mailbox.rs:140`: `Duration::from_secs(6)` becomes `crate::worker::closing::GIVE_UP_AFTER` (make `mod closing` reachable: in `worker/mod.rs` change `mod closing;` to `pub(crate) mod closing;`).

- [ ] **Step 4: Run tests**

Run: `PATH="$HOME/.cargo/bin:$PATH" cargo +1.97.1 test --workspace --locked --offline --no-fail-fast`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git -C /Users/davidlee/Projects/Sprite/.worktrees/findings-ledger-and-admission add -A crates
git -C /Users/davidlee/Projects/Sprite/.worktrees/findings-ledger-and-admission commit -m "$(cat <<'EOF'
Relate the timeouts at compile time

GAP-C13: the Surface and observation clients timed out at 15 s while the
window's "still applying" answer needed 5 s + 10 s measured from later.
CLIENT_TIMEOUT is now derived from those two waits, and the closing
escalation asserts its own order.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
EOF
)"
```

---

### Task 8: Waits are conditions (R-W; GAP-C10 partial)

**Files:**
- Modify: `crates/sprite-term/tests/support/mod.rs:25,61,113,126-155`, `crates/sprite-term/tests/foreground.rs:22-54,101-120`, `crates/sprite-term/tests/graphics_transfer.rs:25-35`, `crates/sprite-term/tests/graphics_policy.rs:25-35`, `crates/sprite-term/tests/history.rs:40-55`, `crates/sprite-term/tests/session_jobs.rs:106-116,166-170`, `crates/sprite-term/tests/prompt_latency.rs:23-35,152`, `crates/sprite-term/tests/lifecycle.rs:299-302,473,553-561`, `crates/sprite-term/tests/snapshot_waiting.rs:17,29,55`, `.github/workflows/ci.yml` (arch job `env`)
- Test: the six #57 tests, looped

**Interfaces:**
- Produces, in `tests/support/mod.rs`:
  ```rust
  pub fn watchdog() -> Duration;                                   // SPRITE_TEST_WATCHDOG_SECS, default 5
  pub fn wait_within<T>(what: &str, limit: Duration, poll: impl FnMut() -> Option<T>) -> T;
  pub fn wait_until<T>(what: &str, poll: impl FnMut() -> Option<T>) -> T;   // = wait_within(what, watchdog(), poll)
  ```
  `WATCHDOG` the constant is removed.

- [ ] **Step 1: Write the failing foreground test first**

In `tests/foreground.rs::a_running_program_is_reported_by_name` replace the wait with:

```rust
    let state = wait_for(&session, "sleep to be in the foreground", |state| {
        state.program() == Some("sleep")
    });
    assert_eq!(state.program(), Some("sleep"));
```

Run: `PATH="$HOME/.cargo/bin:$PATH" cargo +1.97.1 test -p sprite-term --locked --offline --test foreground a_running_program_is_reported_by_name`
Expected: PASS (the old form sampled `sh` before it exec'd `sleep`; this form cannot). The failing-first evidence for this one is #57's own run 37265312104; note that in the commit.

- [ ] **Step 2: Add the helpers to `support/mod.rs`**

Replace `pub const WATCHDOG` with:

```rust
/// Every blocking wait in the suite fails after this long without progress.
///
/// A hang detector: it measures *silence*, not elapsed time. Five seconds by
/// default; `SPRITE_TEST_WATCHDOG_SECS` raises it on a runner that starves
/// a worker past that (GitHub #57).
pub fn watchdog() -> Duration {
    std::env::var("SPRITE_TEST_WATCHDOG_SECS")
        .ok()
        .and_then(|seconds| seconds.parse().ok())
        .map_or(Duration::from_secs(5), Duration::from_secs)
}

/// Polls until `poll` returns a value, failing after `limit`.
///
/// Polling rather than sleeping, because the assertion that follows is about
/// the value this returns: a test that slept and then looked would be
/// asserting on a different moment than the one it waited for.
pub fn wait_within<T>(what: &str, limit: Duration, mut poll: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + limit;
    loop {
        if let Some(value) = poll() {
            return value;
        }
        assert!(
            Instant::now() < deadline,
            "watchdog: {what} did not happen within {limit:?}"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

/// `wait_within` with the watchdog as the limit.
pub fn wait_until<T>(what: &str, poll: impl FnMut() -> Option<T>) -> T {
    wait_within(what, watchdog(), poll)
}
```

Replace every `WATCHDOG` in `support/mod.rs` (lines 61, 113, 117, 126) with `watchdog()`, and in the other test files: `snapshot_waiting.rs` (`use support::{SnapshotPump, pane_text, watchdog}`, `watchdog().as_secs()`, `started.elapsed() > watchdog()`), `prompt_latency.rs:152`, `lifecycle.rs:299,302,473,553`.

- [ ] **Step 3: Convert the nine helpers**

1. `foreground.rs::wait_for`:
   ```rust
   fn wait_for(session: &TerminalSession, what: &str, wanted: impl Fn(&ForegroundState) -> bool) -> ForegroundState {
       support::wait_until(what, || {
           let state = session.foreground();
           wanted(&state).then_some(state)
       })
   }
   ```
2. `foreground.rs::wait_for_owner`:
   ```rust
   fn wait_for_owner(session: &TerminalSession, pid: u32, present: bool) -> Option<i32> {
       support::wait_until(&format!("owner {pid} to be {}foreground", if present { "" } else { "not " }), || {
           let owner = session.foreground_owner_group(pid);
           (owner.is_some() == present).then_some(owner)
       })
   }
   ```
3. and 4. `graphics_transfer.rs` / `graphics_policy.rs::wait_for_graphics`: keep the loop over `events.next()` (each `next` has the watchdog) but bound the whole wait with `support::PROGRESS_CEILING` instead of a local 20 s, and panic with `what`-style text. These two bodies are identical; they stay in their files.
5. `history.rs::wait_for_history`: same treatment as 3.
6. `session_jobs.rs:106-116`: 
   ```rust
   let status = support::wait_within("the fault subprocess to exit", Duration::from_secs(12), || child.try_wait().unwrap());
   ```
   and on timeout the `assert!` in `wait_within` fires; move the `child.kill()` into a `Drop` guard struct around `child` so a timed-out subprocess is still killed:
   ```rust
   struct KillOnDrop(std::process::Child);
   impl Drop for KillOnDrop { fn drop(&mut self) { let _killed = self.0.kill(); let _reaped = self.0.wait(); } }
   ```
   The `timed_out` flag goes; the assertion message keeps the stdout/stderr dump.
7. `session_jobs.rs:166-170`: `support::wait_within(&format!("{name} to write its pid"), Duration::from_secs(3), || fs::read_to_string(&file).ok().filter(|s| !s.is_empty()))`.
8. `prompt_latency.rs:23-35`: `support::wait_within(name, timeout, || child.try_wait().expect("poll prompt test"))` with the same `KillOnDrop` guard as 6.
9. `lifecycle.rs:553-561`: `support::wait_until("the descendant to release the terminal", || hung_up.exists().then_some(()));`.

- [ ] **Step 4: Give the Arch job a longer watchdog**

In `.github/workflows/ci.yml` the `arch` job (line 35) gains, under its `container:` block, an `env:` with `SPRITE_TEST_WATCHDOG_SECS: "20"` (if the job already has an `env:` map, add the key to it). The macOS job keeps the default.

- [ ] **Step 5: Run and loop**

Run: `PATH="$HOME/.cargo/bin:$PATH" cargo +1.97.1 test -p sprite-term --locked --offline --no-fail-fast`
Expected: all pass. Then loop the four sprite-term tests from #57 twenty times under CPU load:

```bash
for core in $(seq "$(sysctl -n hw.ncpu)"); do yes > /dev/null & done
for i in $(seq 20); do
  PATH="$HOME/.cargo/bin:$PATH" cargo +1.97.1 test -p sprite-term --locked --offline --test foreground --test lifecycle --test graphics_identity --test graphics_fixtures -- --exact a_running_program_is_reported_by_name natural_exit_deadline_is_not_renewed_by_continuous_output storage_eviction_removes_an_image_from_the_projection a_crop_selects_a_region_of_the_image > /dev/null 2>&1 || echo "run $i FAILED"
done
jobs -p | xargs kill 2>/dev/null
```

Expected: no `FAILED` line. Record the result for Task 14.

- [ ] **Step 6: Commit**

```bash
git -C /Users/davidlee/Projects/Sprite/.worktrees/findings-ledger-and-admission add -A crates/sprite-term/tests .github/workflows/ci.yml
git -C /Users/davidlee/Projects/Sprite/.worktrees/findings-ledger-and-admission commit -m "$(cat <<'EOF'
Wait on conditions, with one watchdog the runner can lengthen

GAP-C10 (#57, partial): the foreground test waits for the program it
expects instead of sampling once; nine ad hoc wait loops share one
helper; the watchdog reads SPRITE_TEST_WATCHDOG_SECS and Arch CI sets
it to 20 s. #57 stays open until hosted Arch runs show it.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
EOF
)"
```

---

### Task 9: A failed start-up discards its child, and endpoint variables are not inherited (R-S5; GAP-C05, GAP-C02)

**Files:**
- Modify: `crates/sprite-term/src/worker/start.rs:337-344,405-486`
- Test: unit tests in `start.rs`

**Interfaces:**
- Produces (all private to `start.rs`):
  ```rust
  type SpawnedChild = Box<dyn portable_pty::Child + Send + Sync>;
  fn adopt_child(pair: PtyPair, child: SpawnedChild, commands: &SyncSender<Message>, events: Arc<Mailbox>,
                 spawn_waiter: impl FnOnce(SpawnedChild, SyncSender<Message>, Arc<Mailbox>) -> Result<JoinHandle<()>, (SpawnedChild, SessionError)>)
      -> Result<Started, SessionError>;
  fn spawn_child_waiter(child, commands, events) -> Result<JoinHandle<()>, (SpawnedChild, SessionError)>;
  fn discard(child: SpawnedChild, master: Box<dyn MasterPty + Send>);
  ```

- [ ] **Step 1: Write the failing tests**

In `start.rs`'s `mod tests` add:

```rust
    /// GAP-C02: a window exports SPRITE_* endpoint variables to its children.
    /// A child that is itself a Sprite with endpoints disabled must not pass
    /// the parent's on to its own children.
    #[test]
    fn inherited_sprite_endpoint_variables_are_removed() {
        let mut command = CommandBuilder::new("/bin/sh");
        command.env("SPRITE_SURFACE_SOCKET", "/parent/surface.sock");
        command.env("SPRITE_OBSERVATION_KEY", "deadbeef");
        command.env("SPRITE_PANE", "3");
        super::configure_child_environment(&mut command, &[]);
        assert_eq!(command.get_env("SPRITE_SURFACE_SOCKET"), None);
        assert_eq!(command.get_env("SPRITE_OBSERVATION_KEY"), None);
        assert_eq!(command.get_env("SPRITE_PANE"), None);
    }

    #[test]
    fn this_windows_endpoint_variables_still_reach_the_child() {
        let mut command = CommandBuilder::new("/bin/sh");
        command.env("SPRITE_PANE", "3");
        let own = [(
            std::ffi::OsString::from("SPRITE_PANE"),
            std::ffi::OsString::from("7"),
        )];
        super::configure_child_environment(&mut command, &own);
        assert_eq!(command.get_env("SPRITE_PANE").map(|v| v.to_str()), Some(Some("7")));
    }

    /// GAP-C05: when a start-up step after spawn fails, the child was left
    /// running and unreaped. It must be hung up, killed and reaped before
    /// the error is returned.
    #[test]
    fn a_failed_start_up_step_discards_the_child() {
        let (commands, _inbox) = std::sync::mpsc::sync_channel(1);
        let (events, _receiver) = crate::event_mailbox::bounded(1);
        let config = SessionConfig::command("/bin/sh", vec!["-c".into(), "sleep 30".into()]);
        let pid = std::cell::Cell::new(None);
        let started = super::start_with(&config, &commands, events, |child, _, _| {
            pid.set(child.process_id());
            Err((child, SessionError::new("spawn_child_waiter", "no thread for the test")))
        });
        assert_eq!(started.err().map(|error| error.operation), Some("spawn_child_waiter"));
        let pid = nix::unistd::Pid::from_raw(pid.get().expect("the child had a pid") as i32);
        let gone = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while nix::sys::signal::kill(pid, None).is_ok() {
            assert!(std::time::Instant::now() < gone, "the child outlived its failed start-up");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `PATH="$HOME/.cargo/bin:$PATH" cargo +1.97.1 test -p sprite-term --locked --offline start::tests`
Expected: the first two FAIL (`SPRITE_SURFACE_SOCKET` still set) or the third fails to compile (`start_with` missing). Implement the seam (`start_with` with `spawn_waiter` injected, but no `discard` yet) and run again: the third test FAILS with `the child outlived its failed start-up`. That is the red for GAP-C05.

- [ ] **Step 3: Implement**

`configure_child_environment`:

```rust
/// A child inherits this process's environment except what another Sprite
/// put there: a window's own endpoint variables are added back by
/// `environment` when its endpoints are on, and must not leak from a parent
/// window when they are off (GAP-C02).
fn configure_child_environment(
    command: &mut CommandBuilder,
    environment: &[(std::ffi::OsString, std::ffi::OsString)],
) {
    command.env_remove("NO_COLOR");
    let inherited: Vec<String> = command
        .iter_full_env_as_str()
        .filter(|(key, _)| key.starts_with("SPRITE_"))
        .map(|(key, _)| key.to_owned())
        .collect();
    for key in inherited {
        command.env_remove(key);
    }
    for (key, value) in environment {
        command.env(key, value);
    }
}
```

`start` becomes a thin wrapper:

```rust
type SpawnedChild = Box<dyn portable_pty::Child + Send + Sync>;

pub(super) fn start(
    config: &SessionConfig,
    commands: &SyncSender<Message>,
    events: Arc<crate::event_mailbox::Mailbox>,
) -> Result<Started, SessionError> {
    start_with(config, commands, events, spawn_child_waiter)
}

/// `start`, with the waiter's spawn injectable so a test can make a step
/// after the child exists fail.
fn start_with(
    config: &SessionConfig,
    commands: &SyncSender<Message>,
    events: Arc<crate::event_mailbox::Mailbox>,
    spawn_waiter: impl FnOnce(
        SpawnedChild,
        SyncSender<Message>,
        Arc<crate::event_mailbox::Mailbox>,
    ) -> Result<JoinHandle<()>, (SpawnedChild, SessionError)>,
) -> Result<Started, SessionError> {
    let size = config.size;
    let pair = open_pty(PtySize {
        rows: size.rows(),
        cols: size.cols(),
        pixel_width: size.pixel_width(),
        pixel_height: size.pixel_height(),
    })?;
    let mut command = CommandBuilder::new(&config.program);
    for argument in &config.args {
        command.arg(argument);
    }
    if let Some(directory) = &config.working_directory {
        command.cwd(directory);
    }
    configure_child_environment(&mut command, &config.environment);
    let child = pair.slave.spawn_command(command).map_err(|error| SessionError::new("spawn_child", error))?;
    // The parent's slave handle would otherwise hold the PTY open and hide
    // the child's exit.
    drop(pair.slave);
    adopt_child(pair.master, child, commands, events, spawn_waiter)
}

/// Everything after the child exists. A failure here discards the child:
/// hung up by closing the master, killed, and reaped (GAP-C05).
fn adopt_child(
    master: Box<dyn MasterPty + Send>,
    child: SpawnedChild,
    commands: &SyncSender<Message>,
    events: Arc<crate::event_mailbox::Mailbox>,
    spawn_waiter: impl FnOnce(
        SpawnedChild,
        SyncSender<Message>,
        Arc<crate::event_mailbox::Mailbox>,
    ) -> Result<JoinHandle<()>, (SpawnedChild, SessionError)>,
) -> Result<Started, SessionError> {
    let Some(child_pid) = child.process_id() else {
        discard(child, master);
        return Err(SessionError::new("spawn_child", "the child reported no process id"));
    };
    let process_group = pty_unix::process_group_of(child_pid);
    let processes = pty_unix::SessionProcesses::capture(child_pid);
    let Some(master_fd) = master.as_raw_fd() else {
        discard(child, master);
        return Err(SessionError::new("open_pty", "the PTY master exposed no file descriptor"));
    };
    let waiter = match spawn_waiter(child, commands.clone(), events) {
        Ok(waiter) => waiter,
        Err((child, error)) => {
            discard(child, master);
            return Err(error);
        }
    };
    Ok(Started { master, master_fd, process_group, processes, waiter })
}

/// A child that has no session to belong to. It is at most the shell and
/// has had no time to start anything, so a hangup, KILL and reap are the
/// whole escalation.
fn discard(mut child: SpawnedChild, master: Box<dyn MasterPty + Send>) {
    drop(master);
    let _killed = child.kill();
    let _reaped = child.wait();
}
```

`spawn_child_waiter` returns `Err((child, SessionError::new("spawn_child_waiter", error)))` when `thread::Builder::spawn` fails; because `spawn` consumes the closure that owns `child`, build the closure's captures so the child is moved in only on success: spawn a thread that receives the child over a `std::sync::mpsc::sync_channel(1)` created before spawning, and send the child after the thread exists; if spawning fails, the child is still in this function to hand back.

(`portable_pty::Child` has `kill`, `wait`, `process_id`; `MasterPty::as_raw_fd` is what `start` used already.)

- [ ] **Step 4: Run tests**

Run: `PATH="$HOME/.cargo/bin:$PATH" cargo +1.97.1 test --workspace --locked --offline --no-fail-fast`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git -C /Users/davidlee/Projects/Sprite/.worktrees/findings-ledger-and-admission add -A crates/sprite-term
git -C /Users/davidlee/Projects/Sprite/.worktrees/findings-ledger-and-admission commit -m "$(cat <<'EOF'
Discard a child whose start-up failed, and drop inherited SPRITE_ variables

GAP-C05: a start-up step failing after spawn left the child running and
unreaped. GAP-C02: a child inherited a parent window's endpoint
variables when this window's endpoints were off.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
EOF
)"
```

---

### Task 10: `unsafe` policy at the crate root, and `ScanError` (R-S3, R-S4)

**Files:**
- Modify: `crates/sprite-term/src/lib.rs` (crate attribute), `crates/sprite-term/src/pty_unix.rs:23`, `crates/sprite-term/src/pty_unix/processes.rs` (every `Result<_, ()>`), `crates/sprite-term/src/test_allocations.rs:1`, `crates/sprite-term/src/worker/cadence_tests.rs` (the `unsafe` block's fn)
- Test: existing `processes.rs` tests, updated

**Interfaces:**
- Produces, in `processes.rs`:
  ```rust
  /// Why a scan or a reading could not finish. Disappearance is not an error: it is `Ok(None)`.
  #[derive(Clone, Copy, Debug, PartialEq, Eq)]
  pub(crate) enum ScanError { /// The kernel refused the question. Denied, /// The answer could not be read. Unparsable, /// The process changed identity between two readings. Changed }
  ```

- [ ] **Step 1: Change the tests first**

In `processes.rs` tests, every `Err(())` becomes a `ScanError` value by what the test models: injected failures from `sid`/`metadata`/`reread` closures become `Err(ScanError::Denied)`; the identity-mismatch expectation in the `sandwiched_record` tests (the case where `before` and `after` differ) expects `Err(ScanError::Changed)`; `scope.select(Err(ScanError::Denied)).is_err()` stays an `is_err` check. Run `PATH="$HOME/.cargo/bin:$PATH" cargo +1.97.1 test -p sprite-term --locked --offline processes` — expected: compile error, `ScanError` undefined.

- [ ] **Step 2: Implement `ScanError`**

Add the enum at the top of `processes.rs`. Replace every `Result<…, ()>` with `Result<…, ScanError>` in signatures (lines 53, 60, 152, 166, 175, 193-205, 224-250, 254-256, 292, 302) and map each `Err(())`:

| Line | Was | Becomes |
|---|---|---|
| 67 | leader found but no recorded birth | `Err(ScanError::Unparsable)` |
| 128 | `Err(()) =>` in `signal_members` | `Err(ScanError::Denied \| ScanError::Unparsable \| ScanError::Changed) =>` |
| 170 | `parse_stat` failure | `Err(ScanError::Unparsable)` |
| 241 | identity differs between readings | `Err(ScanError::Changed)` |
| 285 | `proc_pidinfo` errno other than ESRCH | `Err(ScanError::Denied)` |
| 296 | `getsid` errno other than ESRCH | `Err(ScanError::Denied)` |
| 307, 316, 330 | `proc_listallpids` gave nothing / kept growing | `Err(ScanError::Denied)` |

`?` propagation sites keep working because the error type is now uniform.

- [ ] **Step 3: Move the `unsafe` policy**

- `crates/sprite-term/src/lib.rs`: add `#![deny(unsafe_code)]` as the first attribute (above the `//!` docs' `mod` lines; keep any existing crate attributes).
- `pty_unix.rs:23`: delete `#![deny(unsafe_code)]`. The two `#[allow(unsafe_code)]` fns there (597, 632) and the two in `processes.rs` (253, 301) keep their attribute and SAFETY comments; add a `reason`: `#[allow(unsafe_code, reason = "libproc FFI; see the SAFETY comments")]` and `#[allow(unsafe_code, reason = "adopts a descriptor dup(2) just returned")]` for `pty_unix.rs:632`.
- `test_allocations.rs`: add `#![allow(unsafe_code, reason = "a counting GlobalAlloc for tests and benches forwards to System")]` as its first line (it is included under `#[path]` by a bench and as `mod test_allocations` under `cfg(test)`; a crate-root deny reaches both).
- `cadence_tests.rs`: the fn containing the `clock_gettime` block gets `#[allow(unsafe_code, reason = "reads the thread CPU clock; the out-param is a plain C struct")]`.

- [ ] **Step 4: Run tests and clippy**

Run: `PATH="$HOME/.cargo/bin:$PATH" cargo +1.97.1 test -p sprite-term --locked --offline --no-fail-fast && PATH="$HOME/.cargo/bin:$PATH" cargo +1.97.1 clippy -p sprite-term --all-targets --locked --offline -- -D warnings`
Expected: green. (`cargo check --target x86_64-unknown-linux-gnu` is not available offline; the Linux `parse_stat`/`scan` paths are compiled in Task 14's Linux run.)

- [ ] **Step 5: Commit**

```bash
git -C /Users/davidlee/Projects/Sprite/.worktrees/findings-ledger-and-admission add -A crates/sprite-term
git -C /Users/davidlee/Projects/Sprite/.worktrees/findings-ledger-and-admission commit -m "$(cat <<'EOF'
Deny unsafe at the crate root and name why a process scan failed

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
EOF
)"
```

---

### Task 11: Decoration metrics once per pass; the shape pool evicts, not clears (R-S2)

**Files:**
- Modify: `crates/sprite-app/src/grid_paint.rs:155-220,860-880,1020-1058`
- Test: `crates/sprite-app/src/grid_paint/tests.rs`

**Interfaces:**
- Produces (private): `struct DecorationMetrics { baseline: Pixels, descent: Pixels, ascent: Pixels }`; `GridPaint::paint_decorations(&self, cell, drawn, bounds, metrics: &mut DecorationMetricsCache, window)` where `type DecorationMetricsCache = [Option<DecorationMetrics>; 8]` indexed by `(bold, italic, enlarged)`.

- [ ] **Step 1: Write the failing shape-pool test**

In `grid_paint/tests.rs` (it already has `positioned(style)`, `plain_style(...)`, and `shapes_anew(cache, cells, color)`):

```rust
#[test]
fn a_full_pool_keeps_the_shapes_rows_still_hold() {
    let mut cache = ShapeCache::default();
    let style = plain_style(SnapshotColor::Default, SnapshotColor::Default, false);
    let color = Rgba { r: 1.0, g: 1.0, b: 1.0, a: 1.0 };
    // A visible row whose shapes the pool must keep.
    let kept: Arc<Vec<PositionedCell>> = Arc::new(
        (0..4)
            .map(|n| {
                let mut cell = positioned(style.clone());
                cell.text = sprite_term::CellText::from(format!("k{n}").as_str());
                cell
            })
            .collect(),
    );
    for column in 0..kept.len() {
        let _shaped = cache.shaped(0, &kept, column, color, ShapedLine::default);
    }
    // Enough distinct shapes on a second row to fill the pool.
    let filler: Arc<Vec<PositionedCell>> = Arc::new(
        (0..MAX_DISTINCT_SHAPES)
            .map(|n| {
                let mut cell = positioned(style.clone());
                cell.text = sprite_term::CellText::from(format!("f{n}").as_str());
                cell
            })
            .collect(),
    );
    for column in 0..filler.len() {
        let _shaped = cache.shaped(1, &filler, column, color, ShapedLine::default);
    }
    // The next new shape has to evict. Row 0's four are still held by their
    // slots and must survive; row 1's slots were overwritten as the row
    // filled, so its shapes are the ones to go.
    let one_more: Arc<Vec<PositionedCell>> = Arc::new(vec![{
        let mut cell = positioned(style.clone());
        cell.text = sprite_term::CellText::from("new");
        cell
    }]);
    let _shaped = cache.shaped(2, &one_more, 0, color, ShapedLine::default);
    assert!(
        !shapes_anew(&mut cache, &kept, color),
        "a shape a visible slot still holds was evicted"
    );
}
```

(`shapes_anew` returns whether re-shaping the row calls the shaper; read its definition at `tests.rs:42` and adapt the call if its signature differs.)

- [ ] **Step 2: Run to verify failure**

Run: `PATH="$HOME/.cargo/bin:$PATH" cargo +1.97.1 test -p sprite-app --locked --offline a_full_pool_keeps_the_shapes_rows_still_hold`
Expected: FAIL — today the pool is cleared whole.

- [ ] **Step 3: Implement eviction**

In `ShapeCache::shaped` replace `if self.shapes.len() >= MAX_DISTINCT_SHAPES { self.shapes.clear(); }` with:

```rust
                if self.shapes.len() >= MAX_DISTINCT_SHAPES {
                    // Only a shape no visible slot holds can go: one still
                    // drawn would be shaped again on the next frame. Each
                    // slot holds a clone, so the pool's own reference is the
                    // only one left on an unused shape.
                    self.shapes.retain(|_, line| Arc::strong_count(line) > 1);
                    if self.shapes.len() >= MAX_DISTINCT_SHAPES {
                        self.shapes.clear();
                    }
                }
```

- [ ] **Step 4: Resolve decoration metrics once per pass**

Add beside `Drawn`:

```rust
/// Where a decoration sits in a cell, which depends only on the font variant.
#[derive(Clone, Copy)]
struct DecorationMetrics {
    baseline: Pixels,
    descent: Pixels,
    ascent: Pixels,
}

/// One slot per (bold, italic, enlarged), filled on first use in a pass.
type DecorationMetricsCache = [Option<DecorationMetrics>; 8];

fn decoration_slot(bold: bool, italic: bool, enlarged: bool) -> usize {
    usize::from(bold) | usize::from(italic) << 1 | usize::from(enlarged) << 2
}
```

In `Element::paint` (around line 860) declare `let mut decoration_metrics: DecorationMetricsCache = [None; 8];` before the row loop and pass `&mut decoration_metrics` to `self.paint_decorations(cell, &drawn, bounds, &mut decoration_metrics, window)`. `paint_decorations` becomes:

```rust
    fn paint_decorations(
        &self,
        cell: &PositionedCell,
        drawn: &Drawn,
        bounds: CellBounds,
        metrics: &mut DecorationMetricsCache,
        window: &mut Window,
    ) {
        if drawn.underline.is_none() && drawn.strikethrough.is_none() {
            return;
        }
        let slot = decoration_slot(cell.style.bold, cell.style.italic, cell.hovered_link);
        let DecorationMetrics { baseline, descent, ascent } = *metrics[slot].get_or_insert_with(|| {
            // Decorations span the cell even when its glyph has no ink or uses geometry.
            let font = terminal_font(&self.font_family, cell.style.bold, cell.style.italic);
            let font_size = self.font_size + if cell.hovered_link { px(1.0) } else { px(0.0) };
            let text = window.text_system();
            let font_id = text.resolve_font(&font);
            DecorationMetrics {
                baseline: text.baseline_offset(font_id, font_size, self.cell_height),
                descent: text.descent(font_id, font_size),
                ascent: text.ascent(font_id, font_size),
            }
        });
        let left = bounds.left.pixels();
        let top = bounds.top.pixels();
        let width = bounds.right.pixels() - left;
        let mask = ContentMask {
            bounds: Bounds::from_corners(
                point(left, top),
                point(bounds.right.pixels(), top + self.cell_height),
            ),
        };
        window.with_content_mask(Some(mask), |window| {
            if let Some(style) = drawn.underline.as_ref() {
                window.paint_underline(point(left, top + baseline + descent * 0.618), width, style);
            }
            if let Some(style) = drawn.strikethrough.as_ref() {
                window.paint_strikethrough(
                    point(left, top + (ascent * 0.5 + baseline) * 0.5),
                    width,
                    style,
                );
            }
        });
    }
```

- [ ] **Step 5: Run tests**

Run: `PATH="$HOME/.cargo/bin:$PATH" cargo +1.97.1 test -p sprite-app --locked --offline --no-fail-fast grid_paint`
Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git -C /Users/davidlee/Projects/Sprite/.worktrees/findings-ledger-and-admission add -A crates/sprite-app
git -C /Users/davidlee/Projects/Sprite/.worktrees/findings-ledger-and-admission commit -m "$(cat <<'EOF'
Resolve decoration metrics once per pass and evict only unheld shapes

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
EOF
)"
```

---

### Task 12: Housekeeping (R-S6)

**Files:**
- Modify: `crates/sprite-app/Cargo.toml:24,50-51`, `crates/sprite-app/src/terminal_view/surfaces.rs:112-116`, `crates/sprite-app/src/local_socket.rs:499-501`, `crates/sprite-term/src/worker/closing.rs:108,145,214,235`, `crates/sprite-app/src/workspace/reload.rs:213`, `crates/sprite-app/src/surface/channel.rs:380-386,678`, `crates/sprite-app/src/confirmation.rs:35`

- [ ] **Step 1: Apply each change**

1. `Cargo.toml`: delete the `[target.'cfg(unix)'.dependencies]` block with its `nix` line (line 50-51); `nix` stays once under `[dependencies]`. (`nix` has no Windows build anyway and the crate has no Windows target.)
2. `surfaces.rs:112-116`: the guard at line 98 already refuses a body-kind mismatch, so the `_ =>` arm is unreachable. Replace the match with the two reachable arms and let the compiler prove it: 
   ```rust
        match (self, parsed.description.root) {
            (Self::List { root, view }, replacement @ Element::VirtualList { .. }) => { … }
            (Self::Elements(elements), root) => elements.replace(Description { root }),
            (Self::List { .. }, root) => unreachable!(
                "the body-kind guard above refused {:?} for a list Surface",
                root.body_kind()
            ),
        }
   ```
3. `local_socket.rs:499`: replace `#[allow(dead_code)] // Retained for endpoint test fixtures; …` with `#[cfg(test)]`. If a non-test caller appears in the build error, that caller is production use and the attribute stays off; report it instead.
4. `closing.rs`: the glossary (`crates/CONTEXT.md`) avoids "descendant" because ancestry alone does not establish ownership; the term is **Ordinary Session Job**. Reword:
   - 108: "The pump may be parked on a PTY that an Ordinary Session Job keeps open forever, so …"
   - 145: rename `descendants_gone` to `jobs_gone`.
   - 214: "… with every helper thread finished and the Ordinary Session Job policy …"
   - 235: "An Ordinary Session Job still holding the terminal then sees …"
   Also the comment at `worker/mod.rs:120-121` ("so descendants can still be reached") → "so the session's Ordinary Session Jobs can still be reached", and `start.rs` `spawn_child_waiter`'s doc ("descendants holding the PTY open") → "Ordinary Session Jobs holding the PTY open".
5. `#[must_use]`:
   - `reload.rs:213` `Relayed::claim`: `#[must_use = "a false claim means the request must be dropped unapplied"]`.
   - `channel.rs:678` `SurfaceRequest::claim`: same text.
   - `confirmation.rs:35` `Confirmation::answer`: `#[must_use = "the answer says whether the subject was confirmed"]`.
   - `channel.rs:380` `send` and `:386` `send_batch`: `#[must_use = "false means the client is gone and nothing was queued"]`.
   Every caller already uses the bool; the build proves it.

- [ ] **Step 2: Run build, clippy and tests**

Run: `PATH="$HOME/.cargo/bin:$PATH" cargo +1.97.1 clippy --workspace --all-targets --locked --offline -- -D warnings && PATH="$HOME/.cargo/bin:$PATH" cargo +1.97.1 test --workspace --locked --offline --no-fail-fast && PATH="$HOME/.cargo/bin:$PATH" cargo +1.97.1 tree --locked --offline --duplicates`
Expected: green; `cargo tree --duplicates` output unchanged from master.

- [ ] **Step 3: Commit**

```bash
git -C /Users/davidlee/Projects/Sprite/.worktrees/findings-ledger-and-admission add -A crates
git -C /Users/davidlee/Projects/Sprite/.worktrees/findings-ledger-and-admission commit -m "$(cat <<'EOF'
Housekeeping from the 2026-10-12 review

nix declared once; the unreachable update arm says why it is; the socket
sweep is test-only; closing speaks of Ordinary Session Jobs; claims,
answers and sends are must_use.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
EOF
)"
```

---

### Task 13: Lints carry the rules (R-K)

This task is five commits, one per lint group, each ending with `cargo clippy --workspace --all-targets --locked --offline -- -D warnings` green. Fix every production site; use `#[expect(clippy::…, reason = "…")]` (never `allow`: `expect` fails when the lint stops firing) only where the lint is wrong about an invariant that cannot be restructured in a handful of lines. Tests are exempt from `unwrap`/`expect` through `clippy.toml`; they are **not** exempt from the other three lints and are fixed too.

Inventory before starting (counts are from 2026-10-12, `--all-targets`, unique sites):

| Lint | Sites | Where |
|---|---|---|
| `let_underscore_must_use` | 193 | 24 observation/client.rs, 23 surface/client.rs, 20 surface/channel.rs, 20 local_socket.rs, 9 observation/endpoint.rs, 7 pty_unix.rs, 6 each terminal_view.rs / observation/panes.rs / worker/mod.rs / tests/lifecycle.rs, the rest scattered |
| `unwrap_used` + `expect_used` (non-test) | ~64 | bench binaries (`src/bin/*-bench.rs`, `test_allocations.rs`), event_mailbox.rs ×9, surface/grid.rs ×4, surface/description.rs ×5, surface/render.rs ×3, local_socket.rs ×3, pty_unix.rs ×3, config.rs ×2, main.rs ×2, list_view.rs ×2, and single sites in paint_benchmark.rs, wire.rs, tokens.rs, surface_routing.rs, terminal_view.rs, terminal_view/surfaces.rs, theme.rs, sprite-term render.rs |
| `wildcard_enum_match_arm` | 69 | 25 in channel.rs tests, 17 in production (box_drawing ×2, grid_paint ×4, description ×2, list ×3, close_gate, surface_routing, paint_benchmark, foreground, session, worker/mod), the rest in test files |
| `significant_drop_tightening` | 15 | event_mailbox ×4, foreground ×2, panes ×4, channel ×2, close_gate, cadence_tests, tests/client.rs |

Regenerate it at any point with:

```bash
PATH="$HOME/.cargo/bin:$PATH" cargo +1.97.1 clippy --workspace --all-targets --locked --offline --message-format=short -- -W clippy::unwrap_used -W clippy::expect_used -W clippy::wildcard_enum_match_arm -W clippy::let_underscore_must_use -W clippy::significant_drop_tightening 2>&1 | grep -E "^crates/.*warning:" | sort -u
```

**Files:**
- Create: `clippy.toml`, `crates/sprite-app/clippy.toml`
- Modify: `Cargo.toml` (root), `crates/*/Cargo.toml`, every site above, `.github/workflows/ci.yml:156-162`, `scripts/test_ci_policy.py`, `crates/sprite-app/src/local_socket.rs:430,657`, `crates/sprite-app/src/test_blocking_wait.rs`

- [ ] **Step 13a-1: Deny `let_underscore_must_use` and fix every site**

Root `Cargo.toml` gains:

```toml
[workspace.lints.clippy]
let_underscore_must_use = "deny"
```

Each of the three `crates/*/Cargo.toml` gains, after `[package]`'s last key:

```toml
[lints]
workspace = true
```

Create root `clippy.toml` (also used by 13b):

```toml
allow-unwrap-in-tests = true
allow-expect-in-tests = true
```

Fix rule, applied site by site:
- A `Result` whose failure is deliberately ignored: `expr.ok();` — e.g. `let _ = self.stream.shutdown(Shutdown::Both);` → `self.stream.shutdown(Shutdown::Both).ok();`. Add a comment only where the reason is not obvious from the surrounding one.
- A `Result` the code should have handled: handle it (`?`, `if let Err`, or a named `let _reason = …` *only* with a name that says why it is ignored, such as `let _already_closed = …`).
- Non-`Result` must_use values (`ControlFlow`, `Admitted`, `bool` from a `#[must_use]` fn): an irrefutable `match` listing every outcome, as `session.rs` already does for `TrySendError`, or a named binding.
- In tests the same rules apply; `let _ = tx.send(())` in a helper thread becomes `tx.send(()).ok();`.

Run: `PATH="$HOME/.cargo/bin:$PATH" cargo +1.97.1 clippy --workspace --all-targets --locked --offline -- -D warnings && PATH="$HOME/.cargo/bin:$PATH" cargo +1.97.1 test --workspace --locked --offline --no-fail-fast`
Expected: green. Commit: `Deny let_underscore_must_use and name every ignored result`.

- [ ] **Step 13b-1: Deny `unwrap_used` and `expect_used`**

Add `unwrap_used = "deny"` and `expect_used = "deny"` to the workspace table. Fix rule:
- Bench and measurement binaries (`crates/sprite-term/src/bin/sprite-graphics-bench.rs`, `sprite-term-bench.rs`, `crates/sprite-app/src/bin/sprite-observation-bench.rs`, `sprite-paint-bench.rs`, `sprite-startup-bench.rs` if it has any) and `test_allocations.rs`: one crate-level `#![expect(clippy::unwrap_used, clippy::expect_used, reason = "a benchmark is a developer tool; a failed setup stops it with the cause")]`. If a file has only one of the two, `expect` only that one.
- `event_mailbox.rs:110,112` (`Condvar::wait*`): `self.space.wait(state).unwrap_or_else(PoisonError::into_inner)` — a poisoned condvar wait returns the guard the same way.
- `surface/grid.rs:439-453,493` (`self.entries[id as usize].as_mut().unwrap()`): add `fn entry_mut(&mut self, id: u32) -> &mut TextEntry` that does the `get_mut` with `unwrap_or_else(|| unreachable!("id {id} was interned and never released"))`; `unreachable!` is the honest statement and is not an `unwrap`. Same for `entry(&self, id)`.
- `surface/description.rs:446-464` (`text.expect("validated text")` etc.): restructure so each `Kind` arm takes its field from the `Option` with `let Some(text) = text else { return Err(Refusal::Malformed(…)) }` — the validation already ran, so write it as the same refusal the validation would have raised; no panic path.
- `surface/render.rs:94,114` and `list_view.rs:66,79` (`picture.as_bytes(0).expect("raster frame")`): one helper `fn raster_bytes(picture: &Picture) -> usize { picture.as_bytes(0).map_or(0, <[u8]>::len) }` in `surface/render.rs`, used by both files.
- `surface/render.rs:588` (`id.expect("every clickable element has an id")`): `let Some(id) = walk.clicks.next() else { return boxed; }` — a missing id draws the element unclickable rather than panicking, and the traversal-order test from the review's note is added: `fn click_ids_and_element_walk_the_tree_in_the_same_order()` asserting `click_ids(description).len() == number of on_click elements rendered` for a nested fixture (reuse the fixture the file's tests already build for clicks).
- `wire.rs:193` (`number.as_u64().expect("checked")`): `Some(Value::Number(number)) => match number.as_u64() { Some(id) => Ok(FocusTarget::Surface(SurfaceId(id))), None => Err(Refusal::Malformed(…)) }` and delete the guard.
- `tokens.rs:191` (`lookup(role.fallback()).expect(...)`): the fallback names are inserted by `TokenRegistry::new` and never removed, and the table is built at run time, so `#[expect(clippy::expect_used, reason = "the fallback tokens are inserted by TokenRegistry::new and never removed")]` on `resolve`.
- `config.rs:383,411` (`toml::to_string(&tokens).expect(...)`): `#[expect(clippy::expect_used, reason = "a table of strings always serialises")]` on `to_toml`.
- `main.rs:95,118`: both sit inside the closure `Application::run` takes, which returns `()`, so `?` cannot propagate; `#[expect(clippy::expect_used, reason = "before the window exists there is nowhere else to report")]` on `main`.
- `local_socket.rs:185,292,383` were replaced in Task 2; if any remain they are `locked()` sites.
- `pty_unix.rs:83,371,384` (`permit.buffer.as_ref().expect("live permit")`): the buffer is taken only by the permit's `Drop`, which returns it to the pool; `#[expect(clippy::expect_used, reason = "a permit's buffer is taken only by its Drop")]` on `OutputChunk::as_bytes` and on the pump loop fn that holds lines 371-384.
- `terminal_view.rs:324` (`command.split_first().expect("a program to run")`): an empty command list is no command, so it takes the shell branch:
  ```rust
        let mut config = match command.as_deref().and_then(<[std::ffi::OsString]>::split_first) {
            Some((program, arguments)) => SessionConfig::terminal_command(program, arguments.to_vec()),
            None => match SessionConfig::shell(&shell.session_preference()) {
                // (the existing shell arm, unchanged)
  ```
- `terminal_view/surfaces.rs:106` (`replacement.list().expect("virtual_list")`): the `replacement @ Element::VirtualList { config }` pattern already binds the config; use `config.clone()` from the pattern (`replacement @ Element::VirtualList { ref config }`) and drop the `list()` call.
- `theme.rs:153` (`FontCatalog` global `.unwrap()`): it was filled two lines above when empty, so:
  ```rust
    let Some(available) = cx.global::<FontCatalog>().0.as_ref() else {
        unreachable!("the font catalog was filled just above");
    };
  ```
- `surface_routing.rs:39` (`request.pane().expect("pane request")`): `let Some(pane) = request.pane() else { request.refuse_with(Refusal::Ineligible); return; };`.
- `sprite-term/src/render.rs:112` (`from_utf8(...).expect("encoded scalar")`): `#[expect(clippy::expect_used, reason = "the inline bytes were written by char::encode_utf8")]` on `as_str`.
- `paint_benchmark.rs:196` (`ValidTerminalSize::new(TerminalSize { rows: 60, cols: 200, … }).expect(...)`): a fixed size in the library half of the bench; `#[expect(clippy::expect_used, reason = "a fixed 60×200 benchmark size is always valid")]` on the fn that builds the snapshot.

Run clippy + tests as above. Expected: green. Commit: `Deny unwrap and expect outside tests`.

- [ ] **Step 13c-1: Deny `wildcard_enum_match_arm`**

Add `wildcard_enum_match_arm = "deny"`. Fix rule: name the variants. Where a match on a large foreign enum (e.g. `serde_json::Value`, GPUI's `Kind`) has many irrelevant variants, list them with `|`. For `#[non_exhaustive]` enums from other crates a wildcard is required by the compiler and the lint does not fire. Sites: the 17 production ones in the inventory, `worker/mod.rs:424` (`other => other` → list the variants that pass through), `session.rs:204,448`, the test files. In `channel.rs` tests, `other => panic!("unexpected {other:?}")` arms become `if let … else { panic!() }` or list the variants.

Run clippy + tests. Commit: `Deny wildcard enum arms`.

- [ ] **Step 13d-1: Deny `significant_drop_tightening`**

Add `significant_drop_tightening = "deny"`. Fix rule: take what you need from the guard into a local, then let the guard drop before the rest of the function — the lint's suggestion is usually right. Sites: `event_mailbox.rs:60,154,167,211`, `foreground.rs:115,136` (`state`/`owner_group` hold the lock across kernel calls on the master fd; copy the `BorrowedFd`/`shell_group` out first — read the `Attached` struct to see what can be copied), `panes.rs:205,233,344,534`, `channel.rs:470,551`, `close_gate.rs:890`, `cadence_tests.rs:33`, `tests/client.rs:409`.

Run clippy + tests. Commit: `Drop lock guards as early as their last use`.

- [ ] **Step 13e-1: Disallowed methods replace the CI greps**

Create `crates/sprite-app/clippy.toml`:

```toml
# Clippy reads the nearest clippy.toml to the crate being checked and does
# not merge it with the workspace root's, so the test exemptions are
# repeated here.
allow-unwrap-in-tests = true
allow-expect-in-tests = true

# UI code stays event driven (ADR 0026, CI "Forbidden states" until
# 2026-10-12). The two sanctioned sites carry an expect attribute.
disallowed-methods = [
  { path = "std::thread::sleep", reason = "the GPUI thread never sleeps; the socket listener's failed-poll rest and the test pause helper are the sanctioned sites" },
  { path = "gpui::Timer::after", reason = "no animation timers; the view repaints on worker events" },
  { path = "gpui::Window::request_animation_frame", reason = "no animation frames; the view repaints on worker events" },
]
```

Then:
- `local_socket.rs:430` (`rest`): `#[expect(clippy::disallowed_methods, reason = "a failed poll must not turn this rest into a spin; this is the listener thread")]` on `fn rest`. Line 657 is in a test fn; same attribute with reason "a test's own pacing".
- `test_blocking_wait.rs::pause`: `#[expect(clippy::disallowed_methods, reason = "the one blocking pause tests may use")]`.
- Verify the config is picked up: run `PATH="$HOME/.cargo/bin:$PATH" cargo +1.97.1 clippy -p sprite-app --all-targets --locked --offline -- -D warnings` once *without* the two `allow-*-in-tests` lines in the new file. Expected: hundreds of `unwrap_used` errors in tests, proving the root file is not merged. Restore the lines; expected green.
- Check the `gpui::Window::request_animation_frame` path exists (`grep -rn "fn request_animation_frame" vendor/gpui/src`); if the method lives on another type, use that path.
- `.github/workflows/ci.yml`: delete lines 156-162 (the `thread::sleep` and `Timer::after|request_animation_frame` greps with their comment). Keep the dependency greps (147-155).
- `scripts/test_ci_policy.py`: remove the three `src/ui.rs` sleep/Timer/raf cases from `test_every_forbidden_pattern_fails_even_before_a_later_clean_guard`; delete `test_background_and_test_waits_are_allowed_but_no_animation_timers`; add:
  ```python
    def test_event_driven_rule_lives_in_clippy_config(self):
        config = (ROOT / "crates/sprite-app/clippy.toml").read_text()
        for path in ["std::thread::sleep", "gpui::Timer::after", "request_animation_frame"]:
            self.assertIn(path, config)
        self.assertIn("allow-unwrap-in-tests = true", config)
  ```

Run: `python3 -m unittest scripts.test_ci_policy -v && PATH="$HOME/.cargo/bin:$PATH" cargo +1.97.1 clippy --workspace --all-targets --locked --offline -- -D warnings && PATH="$HOME/.cargo/bin:$PATH" cargo +1.97.1 test --workspace --locked --offline --no-fail-fast`
Expected: green. Commit: `Carry the event-driven rule as disallowed methods, not greps`.

---

### Task 14: ADRs, the ledger's new entries, evidence, and the PR

**Files:**
- Create: `docs/adr/0030-admit-commands-through-a-control-lane.md`, `docs/adr/0031-the-findings-ledger-defines-closed.md`
- Modify: `docs/reviews/findings.toml` (GAP-C02, C05, C09, C11, C12, C13 → resolved), `docs/PRDs/10-12-2026-findings-ledger-and-admission.md` (the two amendments), `scripts/check_findings.py` only if the format needs it

- [ ] **Step 1: Write the ADRs**

`docs/adr/0030-admit-commands-through-a-control-lane.md`:

```markdown
# Admit commands through a control lane the worker drains first

A Terminal Session's worker read one bounded queue (17 slots: 16 output
permits and one spare) for PTY output, commands, capture requests and helper
reports. Under an output flood the spare slot was shared between the
application's commands and the snapshot consumer's capture requests, so a
keystroke could be refused as "queue is full" (GAP-C11). Commands and history
requests now wait on a second channel, the control lane (64 deep), which the
worker checks at the top of every pass and after every message it takes from
the inbox. A control send is followed by a nonblocking `Wake` on the inbox:
it lands when the worker is idle and wakes it; it fails when the worker is
busy, which is exactly when the worker will reach the control lane on its
own. Shutdown keeps its flag and its inbox knock; the closing loops read only
the inbox, so closing applies no commands.

Every nonblocking offer answers with one type, `Admitted<T>`: `Accepted`,
`Saturated(T)` or `Closed(T)`, the latter two handing the command back.
`Err` from `try_admit` is the command itself being refused (too large, or the
session shutting down), carried by `SessionError.kind`. No caller reads error
text to decide anything.

The same shape splits the Sprite Window's Surface requests into a stream
lane (64; a connection blocks on it) and a one-shot lane (16; handshakes are
refused as busy rather than queued behind another program's stream,
GAP-C12).

Rejected: one larger queue (a flood is unbounded; any shared bound is
reachable); priority inside one queue (a reorderable queue needs a lock on
the hot path the pump writes to); a third thread to referee (ADR 0008 keeps
one owner per session).
```

`docs/adr/0031-the-findings-ledger-defines-closed.md`:

```markdown
# The findings ledger defines "closed"

Four audits in ten days (PRs #52, #53, #54, #58) resolved 55 findings and
left ten open, and nobody could say so from the repository: the findings
lived in narrative review documents, one of the ten had a GitHub issue, and
no test cited the finding it guarded. `docs/reviews/findings.toml` now holds
one entry per finding ID with its status, the test that guards a resolved
one, or the issue that tracks an open one. `scripts/check_findings.py` fails
CI when a resolved finding names no test that exists, an open or deferred one
has no issue, a wontfix has no reason, or any finding ID appears under
`docs/` without an entry. The PR template carries a Findings table so a PR
says which rows it changes. "All bugs closed" is therefore a statement the
checker can refute.

Rejected: tracking in issues alone (a closed issue cites no test, and the
audit documents keep their IDs); a comment convention in tests (nothing
reads comments); a database or external tracker (nothing to install, nothing
to drift).
```

- [ ] **Step 2: Update the ledger and the PRD**

In `docs/reviews/findings.toml`, change the six entries this PR fixes to `status = "resolved"` with their tests and `fixed_by = "fix/findings-ledger-and-admission"` (the PR number replaces it in Step 5), removing their `issue` line:

| ID | `test` |
|---|---|
| GAP-C02 | `inherited_sprite_endpoint_variables_are_removed` |
| GAP-C05 | `a_failed_start_up_step_discards_the_child` |
| GAP-C09 | `a_refused_admission_says_the_window_is_busy_not_that_it_did_not_answer` |
| GAP-C11 | `input_is_admitted_while_every_output_permit_is_outstanding` |
| GAP-C12 | `a_full_stream_lane_does_not_refuse_another_connections_open` |
| GAP-C13 | `a_client_outlives_the_windows_reply_and_after_claim_waits` |

If the ledger's schema requires `fixed_by` to be an integer, make it accept a string here and change it in Step 5. Run `python3 scripts/check_findings.py && python3 -m unittest scripts.test_findings -v`; expected: green.

In the PRD, under R-S4 change `{ Denied, Unparsable, Gone }` to `{ Denied, Unparsable, Changed }` with "(disappearance is already `Ok(None)`)"; under R-T2 change the invariant to `TERM_AFTER < KILL_AFTER < GIVE_UP_AFTER`; under R-K3 remove `std::thread::JoinHandle::join` and add the sentence "`JoinHandle::join` is not listed: the GPUI-thread rule it stood for is the one `local_socket` close path, which is already a named site, and the test suites join their helper threads twenty-five times." Under R-W1 change "sprite-app's GPUI waits use the same value" to "sprite-app's test deadlines read the same variable; GPUI's own `condition` timer is the vendored crate's and stays." Under Constraints, after the relay-wording sentence, add: "The pane's status line also gains one text for a saturated lane: 'the terminal is busy; the last command was not delivered' (R-A3), replacing the error's Display 'send: the terminal command queue is full'."

Commit: `docs: ADR 0030, ADR 0031, and the ledger's six new resolutions`.

- [ ] **Step 3: The gate**

Run, in order, and keep the output:

```bash
W=/Users/davidlee/Projects/Sprite/.worktrees/findings-ledger-and-admission
S=/private/tmp/claude-502/-Users-davidlee/24ab0588-26c4-4436-8560-541b86578a1f/scratchpad
cd "$W" || exit 1
PATH="$HOME/.cargo/bin:$PATH" cargo +1.97.1 fmt --all -- --check
PATH="$HOME/.cargo/bin:$PATH" cargo +1.97.1 clippy --workspace --all-targets --locked --offline -- -D warnings
PATH="$HOME/.cargo/bin:$PATH" cargo +1.97.1 test --workspace --locked --offline --no-fail-fast 2>&1 | tee "$S/gate-tests.log"
PATH="$HOME/.cargo/bin:$PATH" cargo +1.97.1 build --workspace --locked --offline
PATH="$HOME/.cargo/bin:$PATH" cargo +1.97.1 tree --locked --offline --duplicates
python3 -m unittest scripts.test_release_automation scripts.test_packaging_automation scripts.test_shell_integration scripts.test_ci_policy scripts.test_findings scripts.test_gpui_patch -v
python3 scripts/check_findings.py
grep -E "^test result" "$S/gate-tests.log" | awk '{p+=$4; f+=$6} END {print "passed="p" failed="f}'
```

(The `cd` here is the worktree itself, so the harness cwd does not leave it.) Expected: every command exits 0; `failed=0`.

- [ ] **Step 4: Linux**

Docker (OrbStack) is on this host. Run the four #57 sprite-term tests and the three Linux-only compile paths under emulation:

```bash
W=/Users/davidlee/Projects/Sprite/.worktrees/findings-ledger-and-admission
docker run --rm --platform linux/amd64 \
  -v "$W":/work -v "$HOME/.cargo/registry":/usr/local/cargo/registry:ro -w /work \
  -e SPRITE_TEST_WATCHDOG_SECS=20 \
  rust:1.97.1-bookworm bash -c '
    set -euo pipefail
    apt-get update -qq && apt-get install -y -qq xz-utils python3 ncurses-bin > /dev/null
    ZIG=0.16.0
    curl -sSL "https://ziglang.org/download/${ZIG}/zig-x86_64-linux-${ZIG}.tar.xz" | tar -xJ -C /opt
    export PATH="/opt/zig-x86_64-linux-${ZIG}:$PATH"
    zig version
    cargo test -p sprite-term --locked --offline --test foreground --test lifecycle --test graphics_identity --test graphics_fixtures
    cargo clippy -p sprite-term --all-targets --locked --offline -- -D warnings
  '
```

The zig download is the only network use; the `ZIG` value is `ZIG_VERSION` from `ci.yml:21`. Use a separate `CARGO_TARGET_DIR` inside the container (`-e CARGO_TARGET_DIR=/work/target/linux-amd64`) so macOS artefacts are not overwritten. If the route fails for an environment reason (zig cache, emulation speed), stop, record exactly what failed, and rely on the hosted Arch job on the PR; say so in the PR body as unexecuted locally, not as passed.

- [ ] **Step 5: The PR**

```bash
W=/Users/davidlee/Projects/Sprite/.worktrees/findings-ledger-and-admission
git -C "$W" push -u origin fix/findings-ledger-and-admission
gh pr create --repo HundredBillion/Sprite --base master --head fix/findings-ledger-and-admission \
  --title "Make closed checkable, and close the admission bug class" \
  --body-file "$W/docs/PRDs/.pr-body.md"
```

Write `docs/PRDs/.pr-body.md` first (do not commit it; delete it after) from the template: one paragraph on what changed; the Findings table with every ID this PR resolves (the six above, each with its test) and every ID it leaves open with its issue (GAP-C01 #61, C03 #63, C04 #64, C06 #66, C07 #67, C08 #68, C10 #57, C14 #73, C15 #74, C16 #75); `Closes #62, #65, #69, #70, #71, #72`; the Evidence section with the gate output summary, the two looped runs from Tasks 6 and 8 (counts), the Linux result or its exact failure, and the list of native desktop behaviours CI cannot run (window activation, real keyboard focus) as unexecuted. End with `🤖 Generated with [Claude Code](https://claude.com/claude-code)`.

Then, with the PR number `N` from `gh pr create`'s output:
- replace `fixed_by = "fix/findings-ledger-and-admission"` with `fixed_by = N` (matching the type the other `fixed_by` fields use) in `docs/reviews/findings.toml`,
- run `python3 scripts/check_findings.py`,
- commit `Record PR #N in the ledger` and push.

- [ ] **Step 6: Report**

Tell the user: the PR URL, the gate numbers, the two loop results, the Linux result, and anything listed as unexecuted. Issues #62, #65, #69–#72 close on merge; #57 and the rest stay open.
