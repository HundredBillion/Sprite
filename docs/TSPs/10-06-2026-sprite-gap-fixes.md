# Sprite Gap Fixes Technical Spec

> **For agentic workers:** REQUIRED SUB-SKILL: Use dmi-superpowers:subagent-driven-development or dmi-superpowers:executing-plans task-by-task. Record actual-path red/green evidence and obtain independent reviews.

**Goal:** Resolve SPR-014–SPR-023 and open a verified PR.

**Architecture:** Target existing installation, input, geometry and lifecycle
seams. Reconcile narrow existing repairs; preserve Pane ownership and natural
tail drain. Add native fallback provenance only where pinned GPUI cannot express
the distinction, and cancelable bounded event delivery where queues otherwise
form a dependency cycle.

**Tech stack:** Rust 1.97.1, pinned GPUI 0.2.2, nix 0.28.0, portable-pty 0.9.0,
Python stdlib and POSIX shell; existing dependencies only.

## Global Constraints

- Baseline `091efef3f56aa3b9d0ce90d121e05b1ec93cba4d`; dedicated branch
  `fix/sprite-gap-audit-2026-10-06` in the existing checkout, preserving its
  available build artifacts and exact reproduction paths. Other worktrees stay
  untouched. The user preapproves routine gates and authorizes PR publication.
- Preserve terminal independence of GPUI, editor-free dependency invariant,
  bounded retained state, existing wire grammars, Pane ownership and ADR0024.
- No automatic shell injection, user-dotfile changes, real system install,
  desktop interaction, unrelated branch merge or speculative GAP-C01–05 fixes.
- Native macOS behavior is source/compile-inspected unless actually executed;
  do not report Linux fixtures as Cocoa runtime tests.
- Implementation tasks run sequentially; every task gets a fresh implementer
  and independent spec/quality review before the next task.
- Regression tests must exercise actual callers. External deadlines bound
  deadlock tests and every probe cleans only its own recorded children.

## Task 1: Safe installation and portable shell/tool fixtures (SPR-014/015/016/022)

**Blocked by:** None

**Files:** packaging/macos/update.sh; new packaging/macos/install.sh;
crates/sprite-term/shell-integration/sprite.fish and sprite.zsh;
scripts/test_packaging_automation.py; new scripts/test_shell_integration.py;
crates/sprite-term/tests/graphics_tmux.rs; packaging documentation/CI as needed.

**Interfaces:** install.sh accepts source bundle and destination bundle as two
quoted arguments; updater supplies existing default paths. Shell integration
remains optional. No Rust public interface change.

- [x] Extract the actual install transaction and reproduce failed copy with
  a real temporary old bundle/link. Add failed replacement and rollback cases
  using only fixture command failures, plus successful replacement.
- [x] Stage sibling destination via mktemp, copy with ditto first, rename old
  into retained backup, then rename staged app into place. Restore backup on
  ordinary replacement failure; preserve/report backup if recovery fails.
  Trap cleanup only for owned staging artifacts; never delete a working old
  bundle on a failed staged copy.
- [x] Exercise actual optional Fish/Zsh scripts using installed or explicitly
  supplied temporary binaries. Record red for nested/inherited Fish and Zsh
  prompt callback. Guard Fish by local installed functions and unexport marker;
  use an ordinary Zsh exit-code variable. Repeat-source and interactive control
  must pass without touching user configs.
- [x] Add actual tmux regression with spaced/metacharacter fixture directory.
  Pass script arguments rather than interpolate unquoted filenames through its
  two shell layers. Normal and spaced controls must both pass.
- [x] Run Python automation/shell tests, shell syntax and actual tmux tests;
  save exact commands/results and red/green logs in task report; commit task.

## Task 2: Native ranges, mouse payloads and confirmation geometry (SPR-017/018/020)

**Blocked by:** Task 1

**Files:** crates/sprite-app/src/terminal_view/input.rs, render.rs, tests.rs;
workspace/mod.rs, layout_tests.rs; relevant documentation.

**Interfaces:** route_mouse takes `Option<gpui::MouseButton>` and full
`gpui::Modifiers`; no wire protocol or Pane interface change.

```rust
let end = text.encode_utf16().count();
let button = match button {
    Some(gpui::MouseButton::Left) => Some(sprite_term::MouseButton::Left),
    Some(gpui::MouseButton::Middle) => Some(sprite_term::MouseButton::Middle),
    Some(gpui::MouseButton::Right) => Some(sprite_term::MouseButton::Right),
    _ => None,
};
```

- [ ] Add actual EntityInputHandler range tests for ASCII, BMP non-ASCII and
  supplementary characters; watch baseline fail, then reconcile UTF-16 repair
  from 090389e without unrelated changes.
- [ ] Add actual native listener/PTY regressions for middle/right press, drag,
  release, buttonless motion and Alt/Control. Preserve Shift selection and
  hyperlink hover/click behavior. Reconcile corresponding 090389e repair.
- [ ] Reproduce banner coordinate mismatch with actual GPUI layout. Render
  confirmation as an absolute overlay painted above panes so it consumes no
  flex space. Verify show/dismiss, single/multiple tabs, both split orientations,
  resize, bottom fit and pointer drag coordinates. Overlay must intercept label
  clicks instead of passing them to covered content.
- [ ] Run covering GPUI/input/layout/cleanup tests and clippy; update exact
  red/green report and commit. Leave SPR-019 preedit/delivery behavior to Task 5.

## Task 3: Own all ordinary terminal-session job groups (SPR-021)

**Blocked by:** Task 2

**Files:** crates/sprite-term/src/worker/start.rs, mod.rs, closing.rs;
pty_unix.rs and new private process-ownership module; tests/lifecycle.rs or
new tests/session_jobs.rs; ADR0025 and core glossary.

**Interfaces:** private `SessionProcesses` records SID/leader identity at spawn;
fresh group discovery, signal and live-completion methods hide platform details.
Keep existing process_group needed by ForegroundWatch; public session API unchanged.

- [ ] Read `/tmp/sprite-gap-cleanup-design.md` and pinned portable-pty setsid
  contract. Add public-session red test with two HUP/TERM-ignoring ordinary Bash
  job groups plus negative independent/detached sessions, bounded waits and
  identity-aware fixture cleanup.
- [ ] Capture session ownership before child waiter handoff. Linux adapter
  parses numeric /proc stat after final ')'; Darwin uses existing nix::libc
  proc_listallpids/proc_pidinfo plus getsid. Distinguish failed/incomplete scans
  from empty scopes. Reject own/nonpositive/unexpected session IDs.
- [ ] Revalidate live PID birth/SID/group immediately before signaling. Rescan
  at escalation/completion, including newly created groups after KILL stage;
  do not resurrect an empty retired scope or trust only killpg(0) zombies.
  Preserve bounded natural single-HUP policy and explicit HUP/TERM/KILL.
- [ ] Add safe pure selection/identity-change tests alongside actual sessions;
  confirm unrelated/detached jobs survive while ordinary jobs are gone when
  shutdown reports completion. Run lifecycle/output/backpressure suites.
- [ ] Record ADR0025 invariant, tradeoffs and residual portable signaling race;
  report red/green/platform evidence, commit and get task review.

## Task 4: Cancelable delivery and nonblocking UI admission (SPR-023)

**Blocked by:** Task 3

**Files:** crates/sprite-term/src/event_mailbox.rs, lib.rs, session.rs,
worker/mod.rs, start.rs, closing.rs; sprite-app terminal_view.rs/theme.rs and
observation/panes.rs; event_backpressure/lifecycle and actual GPUI tests;
ADR0026 and core glossary.

**Interfaces:** additive `CommandSender::try_send` and
`TerminalSession::try_send` return explicit saturation errors; blocking send
remains available to non-UI consumers. EventStream next/next_blocking retain
their public result types. Mailbox owns cancellation/final-outcome ordering.

- [ ] Promote the external round9 producer/control into bounded actual GPUI
  regression for multi-command settings reload and real installed receivers;
  record baseline callback stall. Add public-session paused-event-consumer
  shutdown and natural-exit outcome tests with retained event ordering.
- [ ] Read exact ad119fd delivery changes/ADR0021 and reconcile its mailbox,
  cancellation and UI admission design rather than importing its whole branch.
  Retain merged two-second read/six-second accepted-tail drain from ADR0024.
  Publication must retain the rest of an accepted bounded parser batch when
  cancelling; final error/exit outcomes must not await consumer room.
- [ ] Route actual UI input/resize/settings and observation enqueue through
  nonblocking admission. Expose refusal and retain pending/unapplied live
  settings and Resize for recoverable latest-value retry; do not mark failed
  updates applied. geometry.rs currently advances self.size before send: only
  advance that admission cache when accepted, and retry unchanged desired size
  after pressure clears. Retry is bounded/coalesced per view, never unbounded
  threads/tasks. Record successful settings admission per worker-facing field,
  so partial color success followed by cursor refusal and a revert to original
  settings still restores actual worker defaults.
- [ ] Prove settings eventually match latest requested values after pressure
  clears, shutdown cancels blocked publication, final outcomes remain visible,
  natural output tail and existing throughput/input benchmark still pass.
- [ ] Record ADR0026 alternatives/bounds/compatibility and exact source chain;
  run term/app covering suites and clippy, report red/green and commit/review.

## Task 5: Distinguish native text commits from key fallback (SPR-019)

**Blocked by:** Task 4

**Files:** pinned local GPUI 0.2.2 copy/patch/provenance and Cargo patch config;
platform/input bridges and native fallback callers; actual Sprite input and
Surface handlers/tests; ADR0027 and dependency documentation/CI checks.

**Interfaces:** additive default-compatible `replace_text_in_range_from_key`
callback at GPUI's InputHandler/EntityInputHandler seam. Default forwards to
existing replacement behavior; Sprite ignores already-delivered key fallback
and accepts ordinary native replace calls regardless of preedit.

- [ ] Read `/tmp/sprite-gap-ui-design.md`, pinned platform consumers and
  f2e66a0/35b0f7a. Reject timer/deferred receipts and blanket stop-propagation;
  these respectively lose identical commits and block native composition onset.
- [ ] Add direct-commit actual PTY/Surface red tests for multibyte and identical
  ASCII text, marked composition and ordinary-key no-double delivery, including
  enhanced key protocol. Verify explicit origin under deferred/focus changes.
- [ ] Preserve pinned GPUI source/licenses. Add only the fallback callback and
  bridge. Label Linux explicit key/replay fallback and mac held-key fallback;
  Cocoa ordinary insertText uses active native key scope and exact text match,
  invalidated by marking/transformed composition. Independent insertions remain
  native commits. Scope save/restore handles nested dispatch; no frame timers.
- [ ] Preserve Wayland one-byte composition commits: capture composing state
  before CommitString resets it. Only uncomposed one-byte text synthesizes the
  existing ordinary KeyDown; active-composition commits route directly to
  InsertText. Terminal and Surface preedit listeners otherwise suppress that
  synthetic key. Include default-client compatibility and CommitString→Done
  regression traces rather than only direct application replacement tests.
- [ ] Audit every fallback/native caller, add bridge/native-classification tests
  and verification against pinned upstream/recorded patch. Preserve other GPUI
  clients through default forwarding. Source-inspect Darwin and run available
  compiler/native CI rather than pretending Cocoa ran on Linux.
- [ ] Remove Sprite's preedit-only commit gate; retain key encoding and current
  Surface target/refusal rules. Test native commits, key fallback, active marks,
  focus changes and protocols. Record ADR0027 limits/cost and commit/review.

## Plan review and final verification

- [x] Self-review PRD coverage: Task1 014/015/016/022; Task2 017/018/020;
  Task3 021; Task4 023; Task5 019. No confirmed ID omitted.
- [x] Independent plan review including GUI compatibility seam, session scope,
  rollback recovery, settings retries and original bug invariants.
- [ ] Complete independent task spec/quality reviews and resolve substantive
  findings; maintain plan-local progress ledger and audit verification statuses.
- [ ] Final whole-branch review from baseline including new files, affected
  unchanged callers, native input source and all earlier audit regressions.
- [ ] Run `cargo fmt --all -- --check`, locked/offline full workspace build,
  clippy all-targets with -D warnings, TERM=dumb tests including doctests,
  Python automation/shell tests, shell syntax, vendored patch integrity and
  safe staged installation as appropriate. Inspect actual command outputs.
- [ ] Update findings, ADRs/context and PR evidence; commit remaining docs,
  push branch and open PR against current origin/master without merging it.

Independent plan review: `/tmp/sprite-gap-plan-review.md`; three corrections adopted before implementation: retain latest refused Resize, route one-byte Wayland composition commits natively, and track partial settings admission across a latest-value revert. All ten IDs covered. Task1 has no remaining plan blocker.
