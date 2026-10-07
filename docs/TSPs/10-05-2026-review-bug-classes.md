# Review Bug Classes Technical Spec

> **For agentic workers:** REQUIRED SUB-SKILL: Use dmi-superpowers:subagent-driven-development (recommended) or dmi-superpowers:executing-plans to implement this TSP task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Repair the reviewed bugs and remove their shared deadlock, cancellation, and state-coordination causes.

**Architecture:** Keep Terminal Core independent of GPUI. A bounded event mailbox separates event consumption from worker cleanup; explicit nonblocking submission protects the application thread. Owning modules reconcile caches and enforce Surface body kinds; the CLI waits on cancellable descriptor input.

**Tech Stack:** Rust 1.97.1, async-channel 2.5.0, nix 0.28.0, GPUI 0.2.2, Python standard library, POSIX shell.

## Global Constraints

- Use existing pinned dependencies and one terminal-owner worker per session.
- Preserve protocol version 1, authentication, observation scope, and normal process/output behavior.
- Bound memory; no unbounded forwarding queues or detached blocked input threads.
- Worktree: `/home/hundredbillion/Projects/Sprite/.worktrees/review-bug-classes`.
- Branch starts at `eaa553eb73154e387fc8b12d16542995ab4962dd`.
- Existing user approval covers routine design and implementation decisions; record any refined design in the report and ADR.
- Each repaired behavior gets a red/green check at its real caller seam; do not test only a duplicate of the implementation.

### Task 1: Terminal event ownership, submission, and natural exit

**Blocked by:** None

**Files:**
- Create: `crates/sprite-term/src/event_mailbox.rs` and `crates/sprite-term/tests/event_backpressure.rs`.
- Modify: `crates/sprite-term/src/lib.rs`, `session.rs`, `worker/mod.rs`, `worker/closing.rs`, `worker/start.rs` as needed, `tests/lifecycle.rs`.
- Modify affected GPUI submission callers in `crates/sprite-app/src/terminal_view.rs`, `terminal_view/theme.rs`, `terminal_view/surfaces.rs`; preserve other tasks' ownership by only changing submission calls.
- Create: `docs/adr/0021-bound-terminal-delivery-and-cancel-event-pressure.md`.

**Interfaces:**
- Consumes: existing `TerminalCommand`, `TerminalEvent`, PTY pump, `SessionConfig`, `SnapshotStream`.
- Produces: `TerminalSession::try_send(&mut self, TerminalCommand) -> Result<(), SessionError>` and `CommandSender::try_send(&self, TerminalCommand) -> Result<(), SessionError>`.
- Retains: `send`, `EventStream::next`, `EventStream::next_blocking`, `begin_shutdown`, `ShutdownHandle::wait` public behavior.
- Internal mailbox publication accepts a whole produced event batch; seal/finalize publishes terminal errors and exit without waiting for consumption.

- [x] Write and run a regression based on `/tmp/sprite-term-review-ui-cycle.rs`: distinct OSC2 titles plus sustained PTY output, event receiver held, capture request occupying the reserved inbox slot. One UI submission must return promptly as accepted or an explicit saturation error, never block.
- [x] Implement the nonblocking method using the existing validated `try_send` primitive:

```rust
match self.commands.try_send(worker::Message::Command(command)) {
    Ok(()) => Ok(()),
    Err(TrySendError::Full(_)) => Err(SessionError::new("send", "the terminal command queue is full")),
    Err(TrySendError::Disconnected(_)) => Err(SessionError::new("send", "the terminal worker ended")),
}
```

Share validation between submission methods. Cover Input, Paste, PasteConfirmed, CommitText, and key text where variable-sized. Convert GPUI-thread callers to nonblocking submission; surface errors visibly or refuse requests.

- [x] Write and run an undrained-events shutdown regression. It must join within the existing cleanup bound, then drain the retained events and final outcome in order. Add coverage for a batch larger than normal event capacity, receiver drop, and resumed consumption.
- [x] Implement a mutex-owned bounded mailbox using the existing async-channel wake primitive and a condition variable for producer pressure. Retain one event batch when normal capacity is exhausted. Suspend further mutations until that batch drains; cancellation wakes publication without discarding the retained batch. Reserve final outcome storage and seal it after cleanup. Receiver drop wakes publication. Replace direct blocking channel emission in live and final paths.
- [x] Write and run a direct-child-exit test with a descendant ignoring HUP/TERM while retaining the PTY. Verify the original exit code, finite completion, and final output survival. Implement a finite natural-output drain state before cleanup, independent of EOF. Keep requested shutdown escalation behavior.
- [x] Run `cargo test -p sprite-term --locked --offline` and affected app submission tests. Inspect queue bounds and accepted-order behavior. Record exact commands and red/green outcomes.
- [x] Update ADR 0010/0011 or add ADR 0021 to explain the changed event retention/cancellation contract; commit only Task 1 files.

### Task 2: UI state invariants and input/rendering correctness

**Blocked by:** Task 1

**Files:**
- Modify: `crates/sprite-app/src/graphics_cache.rs`, `terminal_view/theme.rs`, `terminal_view/input.rs`, `terminal_view/render.rs`, `terminal_view/surfaces.rs`, `terminal_view/tests.rs`, `surface/description.rs`, `surface/host.rs` if needed, `grid_paint.rs`.
- Test: current GPUI tests and a real terminal mouse test where appropriate.

**Interfaces:**
- Consumes: nonblocking session submission from Task 1; current `SnapshotBundle`, `Body`, description parser, GPUI `InputHandler`.
- Produces: existing methods with repaired invariants; no external protocol changes.

- [x] Port the failing GPUI regressions from `/tmp/sprite-ui-review-n6i6uf40` one at a time: UTF-16 caret/marked ranges and visible image recovery after budget zero/restoration. Run each red before its repair.
- [x] Change ranges to `text.encode_utf16().count()`. Add supplementary-plane and combining text checks through InputHandler.
- [x] Make a texture budget update rebuild from the retained bundle, retaining bounded cache behavior. Replace the eviction sentinel with `Option<u32>` so `u32::MAX` can be evicted, and cover that id through actual cache use.
- [x] Exercise valid element Surface followed by grid/list replacement; incompatible updates must refuse and preserve content. Make body-kind validation one owning operation, sharing specialized-root rules between opening and updating. Reject nested grid/list roots rather than allowing an element renderer to silently erase them.
- [x] Exercise pointer hover, drag, and press/release reporting at the UI routing seam. Preserve optional buttons and actual modifiers; route buttonless movement without interfering with hyperlink hover. Keep Shift selection override and ordinary selection semantics. Support Left/Middle/Right consistently.
- [x] Exercise underlined whitespace in drawing preparation. Move decoration drawing outside glyph-only fast paths and preserve underline color, selection/cursor colors, strikethrough, and clipping. Keep normal blank cells cheap; no redundant glyph shaping for ordinary blanks.
- [x] Run `cargo test -p sprite-app --lib --locked --offline` plus affected terminal mouse tests; record results and commit Task 2.

### Task 3: Cancellable Surface CLI and local correctness

**Blocked by:** Task 2

**Files:**
- Modify: `crates/sprite-app/src/surface/client.rs`, `main.rs`, `observation/client.rs`, `crates/sprite-app/Cargo.toml`, `tests/client.rs`.
- Modify: `scripts/prepare_release.py`, `scripts/test_release_automation.py`, `packaging/PKGBUILD.local`, `crates/sprite-term/shell-integration/sprite.fish`.

**Interfaces:**
- Consumes: Unix descriptors and readiness from existing nix 0.28.0, Surface protocol v1.
- Produces: descriptor-backed `run_surface_open` input contract used by `main.rs`; no blocked helper thread survives return.
- Retains: JSON streaming, CLI exit codes, authentication, normal stdin EOF half-close/drain behavior.

- [x] Add a real-binary regression using the existing SurfaceEndpoint fixture: send `opened` then close while child stdin remains open. Repeat with a partial subsequent JSON document. Bound tests with cleanup so failures cannot hang the suite.
- [x] Implement cancellation-aware descriptor reading with OS `poll` on input plus a cancellation socket. Socket event EOF and output errors cancel input. Read descriptor bytes directly rather than mixing raw readiness with buffered `Stdin::read`. Join the events thread and distinguish canceled input from malformed JSON. Add nix as a direct target dependency from the existing workspace entry only if required.
- [x] Add a real endpoint wrong-key `sprite config print` regression, then validate configuration responses before printing; denied or malformed replies go to stderr and return refusal with empty stdout.
- [x] Extend existing release-preparation tests to include both recipes. Synchronize `PKGBUILD.local` to workspace version and include it in atomic preparation validation before any files are written.
- [x] Change Fish's sourced duplicate guard to `return 0` and keep its marker global but unexported (`set -g -u`) so child shells can install their own hooks. Exercise repeat/nested sourcing if Fish is available; otherwise record the missing runtime, do not invent an execution result.
- [x] Run `cargo test -p sprite-app --test client --locked --offline`, `python3 -m unittest discover -s scripts -p 'test_*.py'`, and shell syntax checks appropriate to installed runtimes; commit Task 3.

### Task 4: Whole-change verification and independent review

**Blocked by:** Tasks 1, 2, 3

**Files:**
- Update this plan's isolated progress ledger and requirements evidence; repair reviewed task files if necessary.

**Interfaces:**
- Consumes: all repaired workflows and task reports.
- Produces: verified branch, independent review verdict, final user handoff.

- [x] Run `cargo fmt --all -- --check`, `cargo test --workspace --locked --offline`, Python release tests, and focused original repro checks. Capture complete long output in evidence files and inspect failure counts and exit status.
- [x] Generate whole-branch review package from recorded starting commit to HEAD. Independent reviewer traces actual callers, cancellation interleavings, bounded memory, and unchanged consumers; review both spec compliance and code quality.
- [x] Fix substantive review findings, repeat covering checks, and request focused re-review. Do not claim the whole class is eliminated beyond the explicitly enforced invariants.
- [x] Record completed tasks, commits, validation, runtime limitations, and any deviations. Keep the branch local and provide the worktree path; publishing and installation remain out of scope.


## Completion evidence — 2026-10-05

All four tasks are complete. The independent whole-branch review traced the changed callers and lifecycle contracts from `eaa553e` through `8d1f76c`. It found two additional caller-state defects, repaired at `38bdf89`; the focused re-review cleared both with no open Critical, Important, or Minor findings. Fresh workspace verification passed at final runtime head `38bdf89918f02470c343f110b1c27681a354bef3`. The branch remains local at `fix/review-bug-classes` in the worktree above.

The final submission contract records acceptance by each owner. Local renderer state follows actual font, padding, fallback-color, and texture-budget changes; terminal cursor settings and ColorDefaults retain their independently accepted values. A partially refused reload can revert local state and retry terminal groups, and unchanged reloads preserve terminal defaults. Hyperlink hover/click owns a pending response only after a running worker accepts the request. Acceptance means queued, not completed. ADR 0021 records these refinements. Real GPUI/PTY red/green regressions cover rejected hyperlink recovery, partial reload reversion, and both cursor/color refusal orderings.

Implementation and repair commits:

- Task 1: `ad119fd` (bounded event delivery and cleanup), `353ff74` (refused resize/font retry).
- Task 2: `090389e` (UI invariants/input/decorations), `b9dbd61` (owned graphics refusal status).
- Task 3: `b33e594` (Surface cancellation/config/release/Fish), `ce79fbd` (confirmed blocked socket-write regression), `8d1f76c` (explicit package-list guidance). `6fd7875` records the Fish shell-local guard refinement.
- Final review repair: `38bdf89` (per-owner terminal submission acceptance and accepted-only hyperlink response ownership).

Verification evidence:

| Command | Result | Complete log |
| --- | --- | --- |
| `TERM=xterm-256color cargo test --workspace --locked --offline` at `38bdf89` | exit 0; 43 groups, 802 passed, 0 failed, 2 existing ignored | `/tmp/sprite-task4-workspace-round1.log` |
| `cargo fmt --all -- --check` at `38bdf89` | exit 0 | `/tmp/sprite-task4-fmt-round1.log` |
| `git diff --check` at `38bdf89` | exit 0 | `/tmp/sprite-task4-diff-round1.log` |
| Thirteen focused original-workflow Rust runs at `8d1f76c` | all exit 0; 33 passed, no zero-test filters | `/tmp/sprite-task4-focused-summary.log` and `/tmp/sprite-task4-focused-statuses.json` |
| `PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s scripts -p 'test_*.py'` | exit 0; 10 passed | `/tmp/sprite-task4-python.log` |
| `bash -n packaging/PKGBUILD`; `bash -n packaging/PKGBUILD.local`; `bash -n crates/sprite-term/shell-integration/sprite.bash` (issued individually) | each exit 0 | `/tmp/sprite-task4-bash-pkg.log`, `/tmp/sprite-task4-bash-local.log`, `/tmp/sprite-task4-bash-integration.log` |
| `makepkg -p PKGBUILD.local --packagelist` from `packaging` | exit 0; Sprite and debug package paths both contain `0.2.2-1` | `/tmp/sprite-task4-packagelist.log` |

The fresh final workspace includes and passes all three new submission regressions, the original process/stream/GPUI regressions, and the benchmark and tmux fixtures. Python, shell, and packaging files did not change in the final repair, so their successful evidence was retained. Workspace and both recipes are version 0.2.2; tools are Rust/Cargo 1.97.1, Python 3.14.7, Bash 5.3.15, and makepkg 7.1.0. Detailed eleven-finding/variant evidence and prior defect-specific red/green results are recorded in the ignored local task reports under `.superpowers/sdd/10-05-2026-review-bug-classes-a61253095fd69afa21b58c1ac7698c978ceda4ef/`.

Verification limits remain explicit. Fish and Zsh are unavailable; Fish duplicate/nested sourcing and Fish/Zsh syntax execution are not claimed. macOS execution and live desktop pixels were unavailable; GPUI tests verify production decoration preparation and existing paint paths. The Linux blocked-write regression requires readable child procfs syscall/wchan and socket wait symbols. The pre-existing ignored tests are `croft_checkpoint_one_capabilities` (external Croft) and `measure_maximum_request` (largest-request measurement). The pinned `proc-macro-error2 v2.0.1` future-incompatibility warning remains. Produced event batches are retained within the bounded mailbox contract; finite natural draining does not promise retention of unlimited future unparsed output for a permanently stalled consumer. Release validation occurs before writes but does not provide transaction durability against disk failures. No publication, installation, merge, dependency upgrade, or original-checkout mutation was performed.

## Reconciliation with merged audits (2026-10-06)

PR52 has been reconciled with master `a303969`, including merged PR53 and PR54. ADR0024–0027 describe the current drain, ordinary-job ownership, UI retry and native-text contracts; the original ADR0021 and task evidence are historical where those decisions differ. Local package versions now derive from the workspace manifest; release updates retain staging/rollback and update only the fixed-version distribution recipe. The original fixed local-recipe validation assertions have been superseded. Current integrated tests and master regressions cover these retained decisions.

The remaining PR52 changes retain Surface CLI cancellation, configuration response validation, Surface description replacement checks, drawing decorations and link readiness. Its submission regression tests run against master’s admission/retry implementation. Pane cleanup ownership and the native-text provenance patch are retained.

Merge verification: `TERM=dumb cargo test --workspace --locked --offline` passed **847 tests, 0 failed, 2 optional ignored**. Independent full resulting-PR review against master found no actionable runtime regression. Native desktop and Cocoa scheduling remain unexecuted.
