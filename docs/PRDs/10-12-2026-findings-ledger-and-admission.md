# Make "closed" checkable, and close the admission bug class

## Outcome

After four whole-repository audits in ten days (PRs #52, #53, #54, #58), ten
findings are still open and only one of them is a GitHub issue; zero tests
cite the finding they guard. The review of `v0.2.2..v0.2.4` (2026-10-12) also
found three new defects in the same class as the last two audits: a bounded
queue refusing input it should never compete with.

This PR makes "all bugs closed" a statement the repository can check, and
removes the structural reasons the admission, error-classification, timeout
and timing-test classes keep recurring. Every change below is test-first;
every finding it closes is cited by its regression test.

New finding IDs introduced by this work continue the `GAP-C` series:

| ID | Defect | Disposition here |
|---|---|---|
| GAP-C06 | BCA-26's "zero scrollback" selection-anchor drift is reachable: `ScrollbackBytes` accepts 0. | Open, #66 |
| GAP-C07 | A Surface writer that drains slowly keeps one of the 64 connection slots. | Deferred by design, #67 |
| GAP-C08 | Placeholder tiles snap against the previous frame's origin; a stale seam persists in an unfocused pane. | Open, #68 |
| GAP-C09 | A relayed request refused at admission is reported as "did not answer in time"; ADR 0018 still describes a blocking enqueue. | Fixed here, closes #69 |
| GAP-C10 | Arch CI is flaky on timing-sensitive tests (GitHub #57). | Partly fixed here; #57 stays open until hosted evidence |
| GAP-C11 | A keystroke can be refused while a program floods output: the worker's one spare inbox slot is shared with capture requests. | Fixed here, closes #70 |
| GAP-C12 | One Surface streaming updates keeps the shared 64-slot request queue full, so every other program's open/focus/token request is refused at once. | Fixed here, closes #71 |
| GAP-C13 | The Surface and observation clients time out at 15 s; the window's "still applying" answer arrives after 5 s + 10 s measured from later, so the client never sees it. | Fixed here, closes #72 |
| GAP-C14 | Cached GPUI views per pane (PRD 10-09 "out of scope"). | Deferred, #73 |
| GAP-C15 | Divider-drag per-move layout work (PRD 10-09 "out of scope"). | Deferred, #74 |
| GAP-C16 | Kitty Unicode-placeholder row/column inheritance (PRD 10-09 "out of scope"). | Deferred, #75 |

GAP-C02 (inherited endpoint variables) and GAP-C05 (child leaked when a later
start-up step fails) are also fixed here because each is a small, contained
change. GAP-C01, GAP-C03 and GAP-C04 stay open as issues.

## Constraints

- Rust 1.97.1, pinned dependencies, no new crates, no async runtime.
- One libghostty-owning worker thread per Terminal Session (ADR 0008). **No new
  threads.** The only threads remain the Surface writers and pane cleanups
  (ADR 0029).
- Surface Channel wire protocol version 1 and the observation grammar are
  unchanged on the wire. The only user-visible text change is the refusal
  wording in R-A5. The pane's status line also gains one text for a saturated
  lane: "the terminal is busy; the last command was not delivered" (R-A3),
  replacing the error's Display "send: the terminal command queue is full".
- Linux and macOS source compatibility.
- Work happens on `fix/findings-ledger-and-admission` in
  `.worktrees/findings-ledger-and-admission`; the original checkout stays
  unchanged. No release or version bump.
- Every fix is test-first: each regression is written, observed failing against
  `d699029`, then made to pass. Tests cross the real seam (worker, socket,
  GPUI view), as earlier audits required.
- GitHub issues are created only with the user's explicit go; nothing else is
  published.

## R-L. The findings ledger is the definition of "closed"

- R-L1 `docs/reviews/findings.toml` holds one `[[finding]]` per ID: `id`,
  `title`, `status` (`open | resolved | deferred | wontfix`), `source` (the
  document that reported it); `test` and `fixed_by` when resolved; `issue`
  when open or deferred; `reason` when wontfix. It registers every existing
  ID (SPR-001..023, BCA-01..32, GAP-C01..05) and the new GAP-C06..16.
- R-L2 `scripts/check_findings.py` fails when: a resolved finding names no
  test, or names one that is neither a Rust `fn` under `crates/` nor a Python
  `test_*` under `scripts/`; an open or deferred finding has no issue number;
  a wontfix has no reason; an ID is listed twice or malformed; or any SPR, BCA
  or GAP-C ID appears under `docs/` without a ledger entry. It runs in CI's
  `release-automation` job through `scripts/test_findings.py`, which also
  asserts the real ledger passes.
- R-L3 The 55 resolved findings cite the test that guards them (49 Rust, 6
  Python), from the mapping verified on 2026-10-12.
- R-L4 `.github/pull_request_template.md` carries a Findings table: ID,
  status after this PR, guarding test or issue. The narrative audit documents
  are unchanged; status lives only in the ledger.
- R-L5 Issues #61..#75 were filed on 2026-10-12 for GAP-C01..09 and
  GAP-C11..16 (GAP-C10 is #57); the ledger carries each number. The 55
  resolved findings are not filed as issues; the ledger records the PR that
  fixed each.

## R-K. Lints replace comments as the carrier of rules

- R-K1 Root `Cargo.toml` gains `[workspace.lints.clippy]` with `unwrap_used`,
  `expect_used`, `wildcard_enum_match_arm`, `let_underscore_must_use` and
  `significant_drop_tightening` at `deny`; each crate opts in with
  `[lints] workspace = true`. A root `clippy.toml` sets
  `allow-unwrap-in-tests` and `allow-expect-in-tests`.
- R-K2 Every production site is fixed, not allowed: a deliberate ignored
  result becomes `.ok()` or a named helper; a poisoned lock is recovered
  through one `locked(&mutex)` helper per crate (R-S1); wildcard arms name
  their variants. A per-site `#[allow]` is permitted only with a reason and
  only where the lint is wrong, never for convenience.
- R-K3 `crates/sprite-app/clippy.toml` lists `disallowed-methods` for
  `std::thread::sleep`, `gpui::Timer::after` and `request_animation_frame`, each
  with the reason the CI guard gave. `JoinHandle::join` is not listed: the
  GPUI-thread rule it stood for is the one `local_socket` close path, which is
  already a named site, and the test suites join their helper threads
  twenty-five times. The
  two sanctioned sites (`local_socket.rs` failed-poll rest, the
  `cfg(test)` blocking helper) carry `#[allow(clippy::disallowed_methods)]`
  with that reason. The matching grep lines leave `ci.yml`; the dependency
  greps (`tokio` and friends, libghostty in sprite-app, `unsafe impl Send`)
  stay. `scripts/test_ci_policy.py` follows.

## R-A. One admission type and two lanes

- R-A1 sprite-term exports
  `#[must_use] enum Admitted<T> { Accepted, Saturated(T), Closed(T) }`.
  `TerminalSession::try_admit(cmd) -> Result<Admitted<TerminalCommand>, SessionError>`
  replaces `try_send`; the `Err` is only a validation refusal. A saturated or
  closed admission hands the command back.
- R-A2 The worker reads two channels: `inbox` (output chunks, capture
  requests, pump and child messages; capacity unchanged) and `control`
  (`Command`, `HistoryRequest`; bounded 64). At the top of every pass and
  after every inbox message the worker drains `control` first, so input never
  queues behind output. Control messages count against the pass's
  `BATCH_MESSAGES` budget like any other; a pass that ends on the budget
  captures and the next pass starts on `control` again. A control send is followed by `inbox.try_send(Wake)`:
  success wakes an idle worker; a full inbox means the worker is busy and
  reaches `control` after its current message. `Shutdown` keeps its flag and
  knock; the closing loops are unchanged.
- R-A3 `TerminalView` keeps one `Pending { settings, resize }`, replacing
  the separate slots: a resize is kept from `Saturated(_)`, and settings are
  kept whole while any live change was not admitted. Focus is not in
  `Pending`: `PaneFocus` (`terminal_view/pane_focus.rs`) keeps a refused
  Pane Focus value from `Saturated(_)` and delivers the latest one when room
  returns. `admission_closed` is set from `Closed(_)`, never from error
  text. Keys, paste and text commits report a refusal, as today; with a
  64-deep control lane that now means a wedged worker, not a flood.
- R-A4 The workspace's `SurfaceRequest` channel becomes two: a stream lane
  (Update, Grid, List, Focus, Close, Closed; `send_blocking` from the
  connection thread; capacity 64) and a one-shot lane (the four variants that
  carry a `reply`: Open, Capabilities, FocusPane, RegisterToken; capacity 16;
  admitted through `Admitted`). Each lane has its own drain task of the
  existing per-channel shape. Order within a connection holds because a
  connection sends Open, waits for the answer, then streams. A one-shot
  request that finds its lane full is refused and the connection closed,
  exactly as a refused Open is today; nothing waits.
- R-A5 `relay` returns `RelayError::Refused` for a saturated admission; the
  observation endpoint and the Surface refusal both say "this window is busy;
  nothing was changed" (Surface reasons are free text today, so this is not a
  wire change). ADR 0018 is amended to describe nonblocking admission
  (closing GAP-C09).
- R-A6 Evidence: with 16 outstanding output chunks and a pending capture
  request, `try_admit(Key)` is `Accepted`; across a sustained `yes` flood no
  keystroke is refused at the view; with one connection holding the stream
  lane full, another connection's Open is answered inside `REPLY_TIMEOUT`.

## R-E. Errors carry a kind

- R-E1 `SessionError` gains `kind: ErrorKind`, a `#[non_exhaustive]` enum
  `{ Invalid, Ended, Transient, Other }`. `new()` defaults to `Other`;
  `invalid`, `ended` and `transient` constructors set the others at the sites
  that matter (validation refusals, "worker ended" and "shutting down", the
  openpty retry).
- R-E2 No code outside a constructor inspects `message`. The openpty errno
  text match stays at its construction site only, because portable-pty 0.9
  flattens the errno; its pinned unit test stays.

## R-T. Timeouts are related at compile time

- R-T1 `crates/sprite-app/src/timeouts.rs` holds the durations that stand in
  a relation to each other: `REPLY_TIMEOUT`, `AFTER_CLAIM`, `HANDSHAKE_TIMEOUT`,
  `WRITE_TIMEOUT`, `BROKER_DEADLINE`, `SILENT_CLIENT_TIMEOUT` (today's
  `observation/endpoint.rs` `CLIENT_TIMEOUT`, renamed because it is the
  server's patience with a client that sends nothing) and
  `CLIENT_TIMEOUT = REPLY_TIMEOUT + AFTER_CLAIM + 1 s`, with their `cfg(test)`
  variants, and `const _: () = assert!(..)` for: client > reply + after-claim;
  handshake < client; write < client; broker < reply; silent < reply. The
  test-child and bench deadlines stay where they are. The Surface and
  observation clients import `CLIENT_TIMEOUT` (closing GAP-C13).
- R-T2 In sprite-term, `closing.rs` asserts
  `TERM_AFTER < KILL_AFTER < GIVE_UP_AFTER`, and `event_mailbox.rs`
  references `GIVE_UP_AFTER` instead of a 6 s literal.

## R-F. One owner for Pane Focus

- R-F1 A `PaneFocus` type on `TerminalView` owns what is now `pane_focused`,
  `told_focus` and `pending_focus`, computes the value from keyboard focus and
  window activation in one place, and delivers through `try_admit`.
  `pane_focused()` remains the single read.
- R-F2 `crates/CONTEXT.md` distinguishes Pane Focus from a tab's current pane
  (what observation reports as `focused`), so the two stop sharing a word in
  comments.

## R-W. Waits are conditions, not pauses

- R-W1 sprite-term `tests/support` gains `wait_until(what, poll) -> T`;
  `WATCHDOG` reads `SPRITE_TEST_WATCHDOG_SECS` (default 5). The nine ad hoc
  wait helpers call it. sprite-app's test deadlines read the same variable; GPUI's own `condition`
  timer is the vendored crate's and stays.
- R-W2 The six tests in #57 wait on their condition: in particular
  `a_running_program_is_reported_by_name` waits for `program() == Some("sleep")`.
- R-W3 Evidence: each of the six passes 20 consecutive runs under full CPU
  load on this macOS host. #57 stays open; its "done when" needs hosted Arch
  runs.

## R-S. The review's standards fixes

- R-S1 One `locked(&Mutex<T>) -> MutexGuard<T>` helper per crate recovers a
  poisoned lock; every `lock().unwrap()`, `unwrap_or_else(into_inner)` and the
  silent `if let Ok` in `Handle::drop` use it.
- R-S2 `grid_paint::paint_decorations` resolves font id and metrics once per
  paint pass per (bold, italic, enlarged), not per decorated cell; the shape
  pool evicts entries no visible slot references (`Arc::strong_count == 1`)
  instead of clearing all 4,096.
- R-S3 `#![deny(unsafe_code)]` moves to the sprite-term crate root with the one
  libproc FFI module allowed; the per-module deny goes.
- R-S4 `pty_unix/processes.rs` unit errors become
  `enum ScanError { Denied, Unparsable, Changed }` (disappearance is already
  `Ok(None)`).
- R-S5 GAP-C05: when a start-up step after spawn fails, the child is hung up,
  killed and reaped through the existing escalation before the error is
  sealed. GAP-C02: when endpoints are disabled, the child environment removes
  inherited `SPRITE_` endpoint variables.
- R-S6 Housekeeping: `nix` declared once in sprite-app's `Cargo.toml`; the
  unreachable refusal arm in `surfaces.rs` removed; `sweep_dead_sockets`
  under `cfg(test)`; `descendant` comments reworded to the glossary's terms;
  `#[must_use]` on `Relayed::claim`, `SurfaceRequest::claim` and
  `Confirmation::answer`, but not on the Surface handle's `send`/`send_batch`:
  nineteen call sites (`surfaces.rs` x16, `input.rs`, `render.rs`, one
  `channel.rs` test) send fire-and-forget events to a client that may already
  be gone and legitimately discard the bool, and the `let _ =` idiom is denied
  by R-K.

## Explicitly out of scope

- GAP-C01 (Dock termination), GAP-C03 (SVG filter CPU and memory outside ADR
  0022), GAP-C04 (unpinned PKGBUILD source), GAP-C06, GAP-C07, GAP-C08,
  GAP-C14..16: tracked as issues, not changed here.
- Replacing portable-pty to get a typed openpty errno.
- Any change to the Surface Channel or observation wire grammar.

## Evidence

- Gate before the PR: `cargo test --workspace --locked --offline --no-fail-fast`,
  `cargo clippy --workspace --all-targets --locked --offline -- -D warnings`
  with the new lint table, `cargo fmt --all -- --check`, the Python suites
  including `scripts.test_findings` and `scripts.test_ci_policy`, and the
  looped #57 runs of R-W3.
- Linux through the emulated Ubuntu route PR #58 used if it is available on
  this host; otherwise hosted Arch on the PR. Native desktop behaviour CI
  cannot run is listed as unexecuted, not claimed.
- ADR 0030 records the control lane and `Admitted`; ADR 0031 records the
  ledger as the definition of closed; ADR 0018 is amended.
- The PR's Findings table lists every ID this PR resolves with its test, and
  every ID it leaves open with its issue.
