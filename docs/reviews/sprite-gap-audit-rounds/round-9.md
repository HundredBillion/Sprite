# Sprite gap audit round 9 — production event backpressure cycle

Focused read-only review at `091efef3f56aa3b9d0ce90d121e05b1ec93cba4d`. No checkout/index/HEAD changes. Existing audit C-002 is promoted to **SPR-023 (P1)** with production callback evidence; no production fixes were made.

## Strengths

PTY output owns sixteen permits and returns buffers on drop, snapshots are latest-only, snapshot-consumer wake requests use try_send, and session Drop avoids joining on the UI thread. These bounds limit retention. They do not remove the cycle between synchronous UI command submission and synchronous event publication.

## Over-engineering / simplification

Lean already. Ship.

## Important finding — SPR-023 (P1): a settings reload can deadlock the GPUI thread against terminal event delivery

Anchor: `crates/sprite-term/src/worker/mod.rs:161` (`events.send_blocking`). Related boundaries: `session.rs:233` synchronous command enqueue; `sprite-app/src/terminal_view/theme.rs:239` and `:245` Colors/Cursor sends in one `apply_settings` call; production event receiver `terminal_view.rs:264–296`; global settings observation `terminal_view.rs:337–342`.

Scenario: child emits enough title/cwd/other lossless notices to fill the32-event queue while UI consumption is delayed. Worker stops in event publication, so it cannot consume its17-entry command/output inbox. PTY pump can occupy16 slots with its permits; a settings reload changing colors and cursor submits multiple commands on the UI thread. The reserved seventeenth slot only protects one enqueue, so another command waits for inbox space. UI cannot finish callback or poll its event task; worker cannot receive commands until UI drains events. The cycle persists even though each queue is bounded and the output pump has a reserved application slot. All pane/window UI sharing that GPUI thread stops responding.

Executed proof exercises the actual `TerminalView::new` event/snapshot tasks and unchanged `apply_settings`; it does not substitute an intentionally unused library EventStream:

1. Compile app crate copy plus one appended GPUI test under `/tmp/sprite-round9-probe`, using exact dependency fingerprint from baseline `test-lib-sprite_app-2af947b4bd494136`. Builder `/tmp/sprite-round9-build.py`; appended test `/tmp/sprite-round9-test.rs`; full compile arguments `/tmp/sprite-round9-probe/rustc-command.json`. Production files are unchanged copies of HEAD. Command `python /tmp/sprite-round9-build.py` exited0.
2. Real child `/usr/bin/python /tmp/sprite-round9-probe/child.py` records its PID, waits for a /tmp gate, then writes150 distinct OSC2 titles (`burst-0` through `burst-149`) followed by2MiB text. Child has10s alarm; no unrelated host processes/resources are consumed or exhausted.
3. GPUI test starts a real session and consumes its initial snapshot through the normal task. It opens child gate, then allows worker/pump750ms of real time before invoking the next UI callback. This explicitly models delayed UI consumption/scheduler ordering or preemption. It does **not** establish that750ms delays normally occur or that this deadlock is frequent.
4. Blocked case invokes `view.update_in(... view.apply_settings(...))` with foreground color and cursor blink changed. Real event/snapshot tasks remain installed. The external runner prints `round9: before actual apply_settings`, then times out6s without `after actual apply_settings` or test completion. It kills only the test process and its tracked unique child.
5. Control uses the identical producer and callback, but ticks the real GPUI dispatcher until the actual view title reaches `burst-149`. It then prints before **and after** `apply_settings`; test exits0 with1pass (2.82s in original uncontended run). This distinguishes the cycle from generic settings/application-start failure.

Original uncontended outputs are preserved in `/tmp/sprite-round9-probe/blocked-first.out` and `drained-first.out`. Exact runner: `python /tmp/sprite-round9-run.py`. The runner creates gate/PID files shared by both cases and must be run **serially**. A subsequent reviewer/parent rerun overlapped briefly; its outputs are excluded from independent-repetition evidence. After both ended, no matching unique child script process remained (`ps -eo pid,args | rg '[p]ython /tmp/sprite-round9-probe/child.py'` returned no matches). Current runner checks `/proc/PID/cmdline` contains the unique `/tmp/sprite-round9-probe/child.py` before killing it and polls `/proc` disappearance. Parent independently reran a fresh serialized pair after ownership handoff, saved `/tmp/sprite-gap-round9-parent.log`: blocked6s timeout, drainedexit0 (1pass,2.69s), tracked child PIDs1433357/1433612 both remainingFalse. This second pair had no overlap and confirms the original result.

Source trace gives the specific mechanism: `worker/mod.rs:291–309` parses a chunk, returns its permit, takes all queued notices then emits each losslessly. Output queue at `session.rs:2` has17 slots; pump permits16 (`pty_unix.rs:49`) and each chunk enqueue is synchronous (`pty_unix.rs:355`). Event queue depth32 (`session.rs:17`). `apply_settings` synchronously enqueues Colors then Cursor while running on GPUI; event task awaits next event and applies it using the same view/UI context. The exact blocked internal enqueue was not instrumented; the callback-level hang plus control is executed evidence, while which command occupies the final slot follows this inspected schedule. Font/grid reload adds another synchronous resize send and is an additional inspected caller, not independently exercised. Other input paths also use synchronous `TerminalView::send` (`terminal_view.rs:551–558`).

History: `git blame` points `emit`/Session::send to refactor `b3ab5a4` and Colors/Cursor reload calls to `291d237` / `e81ac72` (earlier code predates file split). SPR-001 audit fix8010540 adjusts natural-exit output draining but leaves blocking event publication and UI submission in place. Earlier audit C-002 remained unconfirmed specifically because an intentionally undrained library stream did not prove an actual GUI callback cycle; this round closes that evidence gap.

Separate all-refs/origin branch commit `ad119fd` (`fix(term): decouple cleanup from bounded event delivery`) was inspected without checkout/cherry-pick: it replaces the bounded event channel with a bounded-batch cancellation-aware mailbox, adds nonblocking command submission, updates UI reload/refusal handling, and introduces ADR0021 plus PTY/GPUI regression tests. Its design directly addresses these boundaries; it is **not** in reviewed HEAD and was **not** executed here. No assumption that its entire implementation is correct or should be merged wholesale.

Remedy: make UI command submission nonblocking with visible/recoverable refusal/coalescing and ensure event publication can be cancelled independently of consuming the worker inbox. Increasing queue capacities or reserving one additional slot leaves the structural cycle possible under multi-command callbacks; do not weaken bounded retention or silently drop ordered protocol replies/events.

## Additional inspected candidate and limits

Child waiter thread-spawn failure: `worker/start.rs:423` drops parent slave then attempts `spawn_child_waiter`; helper takes ownership of already launched child into a closure and maps thread creation failure to SessionError (`:438–449`). Exceptional failure could need explicit child termination/reaping; no failure injection was executed and no arbitrary host resource exhaustion attempted. Master-PTY closure may provide HUP, so abandonment/reaping consequences require a dedicated safe fake-thread seam or process-scoped injection before treating this as confirmed.

No native desktop interaction or entire compositor freeze was executed; this is a GPUI test callback on a real terminal session, with its ordinary receiver tasks. The artificial750ms scheduler hold creates a feasible overlap rather than measuring production frequency. No high-volume benchmark or platform/macOS reproduction attempted. Baseline parent reports798tests passed; that suite is independent existing evidence, not a new execution by this reviewer. No full unmerged-branch assessment. Callback code copied unchanged; only the extra test/producer/runner lives under /tmp. Initial drained-control attempts used run_until_parked / condition timeout and did not establish a passing control; the final tick-until-title fixture replaces them and its original uncontended control passes.

## Assessment

Within terminal delivery/production UI submission scope: with fixes. Confirmed production callback deadlock justifies promoting C-002 to SPR-023; the bounded queue invariant alone does not protect the UI when it can submit more than one command before its event receiver resumes.

Canonical finding IDs and final parent verification are recorded in `../sprite-gap-audit.md`. Temporary probe paths identify this review session; they are not repository scripts.
