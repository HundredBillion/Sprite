# Task 3 — typed defaults and session ownership

Implemented from reviewed base `1be3df0` on `fix/issue-48-design`.

- `ColorDefaults.base: Option<BaseColors>` replaces independent optional foreground/background values. Cursor and sparse palette remain independent. Complete and compile-fail doctests use identical imports; incomplete construction fails because the background field is missing.
- `TerminalSession::spawn` returns `Spawned { session, events, snapshots }`. Stream Options and both take methods are removed. All production, test-helper, integration, and benchmark consumers are migrated. The obsolete take-once test is removed.
- `SessionState` represents NeverStarted, Running, and Ended. Ended retains the handle for shutdown, ignores sends, and reports idle/no foreground owner. Startup and reload share `theme::session_defaults`, including renderer fallback colours.
- Existing direct idle colour and cursor reload regressions remain; added fallback mapping and real GPUI startup/reload comparison tests. The latter also checks ended-handle shutdown ownership. No app `thread::sleep` calls were introduced.

## Source findings and decisions

Inspected existing worker `apply_color_defaults`, snapshot colour projection, stream ownership/drop and shutdown implementations, application startup/reload paths, helper functions returning sessions, and benchmark `await_ready`. The dependency source at `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/libghostty-vt-0.2.1/src/terminal.rs:155` documents separate default and OSC override layers; lines 657 and 673 implement the default setters. Both paired values still go through those setters, preserving program precedence/reset behavior rather than changing effective colours directly.

Internal workspace API migration is atomic: no compatibility wrapper, persistence, or external protocol transition is needed. Test bindings named `_session`, `_events`, and `_snapshots` retain ownership until scope exit; dropping the unused session immediately would shut down the worker before assertions. Shell helpers now return Spawned where their consumers need streams; observation registry tests intentionally select only the session. The adapter returns named colours, cursor, and fallback fields to keep startup/reload use straightforward. Later size and lib.rs module changes remain out of this task.

## Verification evidence

- `cargo check --workspace --all-targets --locked --offline`: initial migration failed on one remaining observation helper mismatch, then corrected. Compiler warnings identified all unused bindings; their lifetimes were preserved using underscore-prefixed names.
- `TERM=xterm-ghostty cargo test -p sprite-app terminal_view::tests --locked --offline`: 3 passed, 0 failed. Log `/tmp/sprite-task3-app-test.log`.
- Mutation check: temporarily changed the adapter's background fallback to the foreground token. `TERM=xterm-ghostty cargo test -p sprite-app session_defaults_pair --locked --offline` exited 101 with the expected background mismatch (216/216/224 versus 16/16/20). Restored original source. Log `/tmp/sprite-task3-red.log`.
- `TERM=xterm-ghostty cargo test --workspace --locked --offline --no-fail-fast`: exit 0, 658 passed, 0 failed, 2 ignored, including 459 app unit tests and both positive/compile-fail BaseColors doctests. Log `/tmp/sprite-task3-final-test.log`.
- Baseline tmux issue is separate: the prior baseline inherited TERM=dumb; the parent established TERM=xterm-ghostty as the appropriate test identity without changing production code. This task's full suite ran `graphics_tmux` with 2 passed, 0 failed.
- `TERM=xterm-ghostty cargo clippy --workspace --all-targets --locked --offline -- -D warnings`: exit 0. Log `/tmp/sprite-task3-clippy-final.log`. Cargo retains its pre-existing future incompatibility advisory for proc-macro-error2 2.0.1.
- Unbounded `rg -n 'TerminalSession::spawn' crates --glob '*.rs'`: 52 sites in 27 files (previous 53 minus the removed take-once test). Census `/tmp/sprite-task3-census.txt`. Unbounded searches for either take method in crates and `thread::sleep` in app Rust files return no matches.
- After redundant binding cleanup, `TERM=xterm-ghostty cargo test -p sprite-term --test session_output --test benchmark --locked --offline`: exit 0, 9 passed, 0 failed; `/tmp/sprite-task3-cleanup-test.log`. All-target Clippy was also repeated successfully after cleanup.
- `cargo fmt --all` and `git diff --check`: clean.

## Self-review

Reviewed public interface, worker application, app state transitions, startup/reload adapter, all helper families and benchmark lifetime management. Removed unnecessary stream rebinding statements during review. Colour precedence/reset tests retain their assertions and paired defaults; the idle reload test supplies the existing background with its new foreground. Ended handles remain owned until cleanup; repeat shutdown yields no second join handle. No task-2 ownership changes, later-wave work, or parent plan edits are included. Independent task review is handled by the parent workflow.
