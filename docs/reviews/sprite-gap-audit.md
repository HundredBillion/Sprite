# Sprite remaining-gap review — 2026-10-06

## Scope and baseline

Five additional read-only review-agent rounds target the coverage gaps from
`sprite-bug-audit.md`. Existing defects are in scope. The five review rounds were read-only. The subsequent request to fix all
confirmed defects and open a PR authorizes implementation, commits and publication
on `fix/sprite-gap-audit-2026-10-06`. Temporary review probes remain outside the
checkout; implementation adds durable regression tests.

Baseline: `091efef3f56aa3b9d0ce90d121e05b1ec93cba4d`, merged PR #53,
with an initially clean `master` matching `origin/master`. The previous audit
resolved SPR-001–SPR-013; those findings must not be reopened without evidence of
a remaining failure. New confirmed findings continue at SPR-014. Earlier
unconfirmed candidates retain their identity and provenance.

Repository instructions: user-supplied AGENTS.md workflow preference, root and
crates CONTEXT.md, applicable ADRs and prior audit/PRD/TSP. Relevant commit history
must be checked before recommending a change, especially Pane cleanup ordering.

The host is Linux. Display environment variables are present, but this review
does not interact with the user's desktop. Native macOS execution is unavailable.
Fish/Zsh were absent at initial environment inspection; isolated temporary shell
binaries may be used for safe probes. Native and headless evidence are distinct.

## Round plan

| Round | Focus and failure scenarios | Status | Evidence and remaining gaps |
| --- | --- | --- | --- |
| 6 | Native GUI lifecycle: window close/quit, pending cleanup, multiple windows, focus/input/scale/banner interactions | Complete | SPR-017–020; three isolated GPUI failures, mouse caller trace, 12 existing cleanup tests passed; native desktop untouched |
| 7 | macOS: PTY/platform assumptions, bundle install/replacement failures, identity and hosted CI | Complete | SPR-014; Darwin and Dock quit source-inspected, native runtime unavailable |
| 8 | Fish/Zsh: nested shells, repeated hooks, startup flags, cwd/title and identity propagation | Complete | SPR-015/016; actual interactive PTY probes, no user dotfile changes |
| 9 | Event backpressure: worker/event/UI command cycle, close under pressure, bounded natural exit | Complete | SPR-023 promotes C-002; actual settings callback stalls, drained control passes; scheduling frequency unmeasured |
| 10 | Resource and operational stress: SVG intermediates/cache multiplicity, process groups, framing and path edge cases | Complete | SPR-021/022; safely capped decoder cost measured; distribution consequence and native frame effects remain unverified |

## Baseline verification

Fresh full workspace tests completed with inherited `TERM=dumb`:
`TERM=dumb cargo test --workspace --locked --offline --no-fail-fast`.
Exit 0: **798 passed, 0 failed, 2 ignored, 42 test binaries**, including
doctests. Log: `/tmp/sprite-gap-baseline-tests.log`. Green baseline tests do
not cover the newly exercised installation and shell failure scenarios.
Python release/packaging baseline: **14 passed** with
`PYTHONDONTWRITEBYTECODE=1 python3 -m unittest scripts.test_release_automation scripts.test_packaging_automation`;
log `/tmp/sprite-gap-baseline-python.log`.

## Confirmed findings

Locations below identify the reviewed baseline. Implementation sections record
fix commits and durable regression paths against the resulting branch.

| ID | Severity | Discovery round | Scenario and location | Evidence | Root cause | Status |
| --- | --- | --- | --- | --- | --- | --- |
| SPR-014 | P2 | 7 | macOS update copy fails after a working installation exists; `packaging/macos/update.sh:39–40` | Parent reran `python3 /tmp/sprite-round7-repro.py`: actual byte-for-byte updater in a safe temporary fixture exits 73, original app absent, existing CLI link dangling. Native copy failure is injected; no real installation occurs. | Installed app is deleted before replacement copy succeeds; no recovery. | Resolved in 81e6672; regression verified; independent spec/quality review passed |
| SPR-015 | P2 | 8 | Nested Fish sources installed integration after parent did; `crates/sprite-term/shell-integration/sprite.fish:6–9` | Parent reran `python3 /tmp/sprite-gap8-shells/probe.py` with temporary Fish 4.9.3: parent hooks exist and emit OSC, child sources same actual file but has no hooks and emits no prompt sequences. | Exported initialization marker is inherited; shell-local functions are not. Guard mistakes parent initialization for child initialization. | Resolved in 81e6672/bb64ff1; inherited/nested/reload sibling regressions verified; independent reviews passed |
| SPR-016 | P2 | 8 | Zsh prompt hook runs after sourcing integration; `crates/sprite-term/shell-integration/sprite.zsh:15–19` | Same parent-rerun probe with temporary Zsh 5.9.2: `__sprite_precmd:1: read-only variable: status`, no OSC 133 D/A or OSC 7. Probe's overall shell exit is 0; the error and missing hook output prove the failure. | `status` is a readonly Zsh special parameter; local assignment aborts hook. | Resolved in 81e6672; regression verified; independent spec/quality review passed |
| SPR-017 | P2 | 6 | Non-ASCII IME marked/selected range; `crates/sprite-app/src/terminal_view/input.rs:222,234` | Parent reran isolated actual-handler GPUI probe: `é` returns caret 2..2 and marked 0..2, expected UTF-16 1..1 and 0..1. Pinned Cocoa consumers forward these ranges into NSRange. | String byte length substitutes for UTF-16 code units. | Resolved in 074cc1a; actual-handler/PTY/layout red-green verified; independent spec/quality review passed |
| SPR-018 | P2 | 6 | Close confirmation appears above vertically split panes; `workspace/mod.rs:364–374,472–493`, `workspace/divider.rs:250–279` | Parent reran actual GPUI render probe: pane origin changes 0→34px and height 1080→1046, divider cache still origin 0/boundary 540. Existing allocations remain based on pre-banner room. | Cached pane geometry accounts only for tab chrome; dynamic banner changes actual container room and pointer coordinates. | Resolved in 074cc1a; actual-handler/PTY/layout red-green verified; independent spec/quality review passed |
| SPR-019 | P2 | 6 | Wayland multibyte commit without preedit; `terminal_view/input.rs:259–260` | Pinned CommitString("日本")→InsertText→replace_text_in_range path has no KeyDown fallback. Actual handler GPUI command recorder emits zero commands; marked-text control emits CommitText("日本"). Parent reran. | Preedit presence incorrectly stands in for whether text was already delivered by keydown. | Resolved in d271b54/6f529e5; PTY/socket and portable source-policy regressions verified; independent initial/focused reviews passed |
| SPR-020 | P2 | 6 | Terminal mouse reporting for middle/right buttons or Alt/Control; `terminal_view/render.rs:473–475,555–557`, `terminal_view/input.rs:158–164` | Parent inspected entire terminal mouse listener/route census: only left down/up listeners; route always sends Left and hard-codes Alt/Control false. Motion similarly discards supplied pressed button. | Native mouse payload omitted at application-to-core boundary despite supported core fields. | Resolved in 074cc1a; actual-handler/PTY/layout red-green verified; independent spec/quality review passed |
| SPR-021 | P1 | 10 | Explicit shutdown with ordinary Bash job-control background group; `sprite-term/src/worker/closing.rs:79,188–200` | Parent reran actual public-session `/tmp/sprite-round10-process.py`: shell disappears, wait completes in 50ms, background sleep remains live/reparented in a separate group within the same terminal session. Probe kills only its tracked children afterward. | Cleanup enumerates recorded shell and foreground groups, excluding other ordinary job-control groups; completion predicate checks only enumerated groups. | Resolved in 5e50493/69cc473; immediate/late cleanup and permission regressions verified; independent review passed |
| SPR-022 | P3 | 10 | Rust tmux graphics fixture with TMPDIR containing spaces; `sprite-term/tests/graphics_tmux.rs:78–83` | Parent reran `/tmp/sprite-round10-tmux.py`: exact baseline test binary passes with ordinary TMPDIR, fails with spaced TMPDIR (exit 101, snapshot stream ended). | Generated shell command interpolates fixture config/image paths without shell quoting. | Resolved in 81e6672; regression verified; independent spec/quality review passed |
| SPR-023 | P1 | 9 | Child event burst and multi-command settings reload while UI consumption is delayed; `sprite-term/src/worker/mod.rs:161`, `session.rs:233`, `terminal_view/theme.rs:239,245` | Parent independently reran serialized actual-session GPUI probe: callback prints before, stalls past 6s; identical case with dispatcher draining notices prints after and passes in 2.69s. Actual ordinary event/snapshot tasks are installed. | Worker blocks publishing to 32-event queue; pump fills 16 of 17 inbox slots; multiple blocking UI command sends exhaust the reserved slot and prevent the same UI thread draining events. | Resolved in a2d8fab/605463d; main pressure/recovery and pre-snapshot fallback regressions verified; independent reviews passed |

Shell scripts are optional and must be sourced by shell configuration; Sprite
does not automatically inject them. Shell findings concern that supported
opt-in use, not a promise to automatically instrument every nested shell.

## Unconfirmed candidates and rejected hypotheses

| Candidate | Discovery | Current evidence and status |
| --- | --- | --- |
| GAP-C01 | Round 7 | Dock/system termination may bypass window cleanup gate. Pinned GPUI termination invokes App shutdown; Cocoa event schedule not exercised. Unconfirmed. |
| GAP-C02 | Round 8 | Nested Sprite with endpoints disabled may inherit the parent's endpoint variables. Enabled panes overwrite them; disabled case requires an actual nested-window/settings probe. Unconfirmed. |
| GAP-C03 | Round 10 | Small SVG filter documents perform seconds of synchronous decoder work. Actual decoder bodies with a stub final image wrapper: 512×512/1MiB raster, 32 filter results in 2237 encoded bytes took 3.378s; 128 results in 8437 bytes hit a 5s CPU cap. Native frame/input stall not exercised. Quantified responsiveness risk, not a violation of the documented final-raster/cache budgets. |
| GAP-C04 | Prior round 5; now round 10 | Distribution PKGBUILD labels a fixed version but fetches unpinned moving Git HEAD. Current manifest still matches version; no stale-recipe/future-release build consequence exercised. Unconfirmed consequence. |
| GAP-C05 | Round 7/9 | Child-waiter thread creation fails after launch. No safe fault injection executed; PTY closure may already provide HUP, so abandonment/reaping consequences are unconfirmed. |
| C-002 | Prior round 1; now round 9 | Confirmed and deduplicated into SPR-023 after actual GPUI callback reproduction and passing drained control. |

Prior candidates and limits are listed in `sprite-bug-audit.md`; they are not
automatically confirmed in this pass. Repeat-source Fish does not exit the shell:
actual repeat-source probe survives, retaining the earlier rejection. Multiple
Sprite windows in one process are not a supported current entry point; a
hypothetical multiwindow quit mismatch is not a new bug.

## History and recommendation checks

SPR-014's delete-before-copy was introduced in `cd1e975`; the one-bundle/CLI-link
decision does not require that ordering. A staged replacement with ordinary
failure recovery can preserve it. SPR-015/016 date to `e69d28b`; Fish marker
repair already exists in unmerged `b33e594`, absent HEAD ancestry. Coordinate that
work rather than produce an independent competing implementation. Zsh's readonly
name requires a targeted variable correction.

Recommendations must identify the actual failure and preserve established
contracts unless the evidence demonstrates that contract is defective.

SPR-017/020 already have a related pending repair in unmerged `090389e`.
SPR-019's preedit gate was introduced in `f2e66a0` to prevent plain keystrokes
being sent twice and preserved in `35b0f7a` for Surface routing: simply removing
the gate would reverse that fix. Any repair must preserve plain-key encoding and
single delivery while accepting native direct commits. SPR-018 predates the
merged cleanup change; it requires geometry correction, not Pane ownership churn.
SPR-021 likewise concerns which process groups are reached, not whether Workspace
retains and awaits cleanup tasks.
SPR-023's blocking boundaries precede the merged audit. Separate `ad119fd`
already changes event delivery, UI command admission and refusal handling; it is
not in HEAD and its whole implementation was not validated by this review.
Compare/reconcile that existing work before proposing another delivery model.

## Round evidence

Round 6 inspected workspace/main/close gate/stopping admission, TerminalView
input/render/Surfaces, divider layout, pinned GPUI native consumers, and prior
Pane/IME repairs. Isolated current-source GPUI tests ran via direct rustc with
fingerprint-matched existing dependencies; no checkout test edits. Parent reran
`/tmp/sprite-round6-probe/sprite_app round6_ --nocapture`: three regression
assertions failed in 0.01s (UTF-16, banner, direct commit). Probe instrumentation
adds a pane-container debug selector and a command recorder at send entry only.
The direct-commit test uses a failed view and proves handler command generation,
not actual PTY delivery. Native dispatch is established separately by the pinned
Wayland caller path. UTF-16 runtime probe stops at `é`; Japanese/emoji variants
are inspected calculations, not additional executed assertions. Reviewer also
ran 12 existing actual cleanup tests successfully. Report `/tmp/sprite-gap-round6.md`;
parent output `/tmp/sprite-gap-round6-parent.log`.

Round 7 inspected macOS update/bundle scripts, minimum OS claim, terminfo lookup,
Darwin PTY pump/waiter/closing, native quit consumers, CI and introducing history.
Executed actual updater in a sandboxed temporary failure fixture. Parent reran
the exact reproducer. Detailed report: `/tmp/sprite-gap-round7.md`.

Round 8 inspected actual Fish/Zsh/Bash scripts, shell preference and identity
builders, endpoint environment propagation, OSC consumers and history. Executed
temporary Fish 4.9.3 and Zsh 5.9.2, repeat-source and nested-source probes, and
interactive controlling-PTY sessions with temporary HOME. Parent independently
reran both `/tmp/sprite-gap8-shells/probe.py` and `interactive.py`. Interactive
Zsh produced three readonly errors and no prompt-start/command-done/cwd markers;
nested interactive Fish reported `nested_hook_status:1` while its parent retained
normal markers. Detailed report: `/tmp/sprite-gap-round8.md`.

Round 9 inspected worker notice publication, command/event/output capacities,
real GPUI receiver and settings/input/resize callers, child-waiter fault path and
unmerged delivery history. Added one GPUI test only to an isolated source copy;
actual production callback and terminal session were unchanged. Child emitted
150 title notices then 2MiB text. A 750ms hold models delayed UI consumption; it
does not establish frequency under ordinary use. Original uncontended callback
stalls while drained control passes. A briefly overlapping repeated fixture run
was discarded; parent then reran a fresh serial pair in
`/tmp/sprite-gap-round9-parent.log`: callback stalls past 6s, drained control
passes (1 test, 2.69s), both tracked children gone. Exact rerun:
`python3 /tmp/sprite-round9-run.py`, serial execution required. Report:
`/tmp/sprite-gap-round9.md`.

Round 10 inspected ordinary shell job-control groups, signaling/completion,
decoder admission/filter intermediates and synchronous GUI callers, tmux's two
shell layers, distribution source/version history and cache multiplicity.
Executed actual public TerminalSession shutdown probes, actual existing tmux test
under normal/spaced TMPDIR, and decoder subprocesses capped at 256MiB address
space, 5s CPU, 10s wall time, with core dumps disabled. Parent independently reran
the process and tmux probes. Decoder probe transplants the unchanged decoder
body; font paths are relocated and only final RenderImage wrapping is stubbed.
This proves decoder cost, not an executed native UI freeze. Independent probes
kill only their recorded temporary child processes. Detailed report:
`/tmp/sprite-gap-round10.md`.

## Recommendation boundaries

These were the read-only review recommendations. The approved PRD/TSP selects
the simplest effective repairs and records architectural decisions during fixes.

| Root-cause class / IDs | Simplest candidate repair and invariant | Compatibility and risks to preserve |
| --- | --- | --- |
| Installation lifetime / SPR-014 | Stage replacement before removing the working app; retain/restore old bundle on ordinary failure. Shared installation boundary. | Preserve single bundle plus CLI symlink; distinguish ordinary recovery from crash atomicity. |
| Shell-local initialization / SPR-015 | Track hook installation locally; inherited marker cannot suppress child-local functions. Structural state-scope correction. | Preserve repeat-source behavior; coordinate unmerged b33e594 instead of parallel repairs. |
| Shell reserved name / SPR-016 | Use an ordinary variable for previous exit code. Targeted fix. | Preserve command exit status and existing hook sequencing; no shell startup redesign. |
| Native input contract / SPR-017,020 | Use UTF-16 ranges and forward actual mouse buttons/modifiers at the shared application boundary. | Coordinate unmerged 090389e; retain Shift selection and terminal protocol behavior. |
| Geometry source / SPR-018 | Include dynamic chrome in actual pane bounds, or use non-space-consuming confirmation chrome. Shared geometry invariant. | Validate banner show/dismiss/drag and tab-strip combinations; preserve cleanup ownership. |
| Text-delivery provenance / SPR-019 | Distinguish direct native commits from already-delivered ordinary key fallback. Shared input boundary. | A blanket preedit-guard removal reintroduces f2e66a0's doubled keystrokes; test terminal and Surface destinations. |
| Shutdown target enumeration / SPR-021 | Reach ordinary job groups of the terminal session and verify their completion within existing deadlines. Shared lifecycle boundary. | Preserve off-UI, bounded cleanup; handle PID/group reuse and platform differences, avoid unrelated/detached sessions. Merely waiting longer cannot find omitted groups. |
| Fixture argument boundaries / SPR-022 | Quote/pass configuration and image paths as arguments through both shell layers. Targeted test-fixture fix. | Preserve tmux passthrough policy; do not classify this as an engine graphics failure. |
| Delivery dependency cycle / SPR-023 | Nonblocking UI command admission with recoverable refusal/retry, plus cancellation independent of event consumption. Shared runtime boundary. | Compare existing ad119fd; preserve ordered required events/replies and bounded retention. Capacity increases do not prevent the cycle. Reload failures must not silently claim settings were applied. |

Detailed round reports are retained in `sprite-gap-audit-rounds/`; reviewer-local
labels map to the canonical stable IDs above. Temporary probes remain outside
the checkout and are not installed tools or committed regression tests.

## Completed review result and remaining limits

All five additional read-only review rounds are complete. **Ten confirmed
findings**: two P1, seven P2, one P3 (SPR-014–SPR-023). Their implementation and verification are tracked below under the subsequent
fix-and-PR request. No prior resolved SPR-001–SPR-013 repair was
reversed. Source/history and actual-path evidence distinguish this pass from a
second cleanup redesign. Findings were deduplicated by root cause and caller
boundary; the earlier banner, Fish, process-group, tmux-path and backpressure
observations gained stronger evidence rather than being invented as new quotas.

Retained round reports:
[6](sprite-gap-audit-rounds/round-6.md),
[7](sprite-gap-audit-rounds/round-7.md),
[8](sprite-gap-audit-rounds/round-8.md),
[9](sprite-gap-audit-rounds/round-9.md),
[10](sprite-gap-audit-rounds/round-10.md).

Remaining limits: native Cocoa/Dock termination, live compositor/IME/mouse input,
fractional monitor-scale transitions, native macOS updater behavior, older shell
versions/prompt plugins, real Arch installation/moving-source version mismatch,
nested disabled endpoint routing and injected waiter-thread failure remain
unexecuted. Headless GPUI and native consumer traces prove only their stated
contracts. SVG intermediate/process-wide memory and CPU remain outside ADR0022's
pixel budgets; capped measurements show a material synchronous decoding risk,
without a measured native frame stall. Delayed-consumption backpressure was
reproduced, but its production frequency was not measured.

Read-only hosted CI observation for master run 37532424660: release version
automation and Arch Linux passed; macOS remained in progress at final inspection.
No remote workflow was launched and no CI completion was inferred from elapsed
time. No macOS access is essential to finish this review-only pass; it is needed
to close the explicit native validation gaps.

## Implementation and verification

The user subsequently requested all confirmed fixes and a PR. Requirements:
`docs/PRDs/10-06-2026-sprite-gap-fixes.md`; sequential implementation and independent
reviews: `docs/TSPs/10-06-2026-sprite-gap-fixes.md`. No merge is authorized.

| Task | Finding IDs | Current status | Verification |
| --- | --- | --- | --- |
| 1 | SPR-014/015/016/022 | Resolved; independent spec/quality review passed | 25 Python and 3 real tmux tests pass; actual Fish/Zsh PTY controls, syntax/fmt and targeted clippy pass; installer red/green includes failed copy/replacement/rollback |
| 2 | SPR-017/018/020 | Resolved; independent review passed | 562 app tests, 11 terminal and 4 layout tests pass; fmt/clippy pass; three actual-path baseline failures verified |
| 3 | SPR-021 | Resolved; independent fix review passed | 66 library, 21 integration tests pass; ordinary job red-green, detached/independent negatives and fast-leader capture pass |
| 4 | SPR-023 | Resolved; independent initial/fix reviews passed | 226 term and 596 app tests, benchmark, fmt/clippy pass; actual pressure/recovery and paused-consumer regressions verified |
| 5 | SPR-019 | Pending | Original direct-commit failure and native call path retained |

Unconfirmed GAP-C01–05 remain separate. Final verification and PR publication are
pending; baseline success alone is not evidence for the fixes.

### Task 1 verification: SPR-014/015/016/022

Commit `81e6672` stages the real install transaction before replacement and retains
an old bundle if rollback fails. Actual temporary-bundle/link fixtures were red
for copy/replacement/recovery failures and pass after the change. Fish guards
shell-local functions and unexports the inherited marker; Zsh uses an ordinary
exit-code variable. Actual Fish 4.9.3/Zsh 5.9.2 nested/repeated/interactive controls
pass. tmux receives arguments through both shell layers; its actual new fixture
failed before quoting and passes with spaces and shell metacharacters.

Verification: `python -m unittest scripts.test_release_automation
scripts.test_packaging_automation scripts.test_shell_integration -v` with explicit
temporary shell paths: 25 passed, no skips. `cargo test -p sprite-term --test
graphics_tmux --locked --offline -- --nocapture`: 3 passed. Shell syntax,
`cargo fmt --all -- --check`, and targeted tmux clippy with `-D warnings` pass.
Independent review inspected installer cleanup, shell hooks, tmux boundaries,
updater and CI callers; spec and quality passed. Native macOS installation,
power-loss and concurrent-install atomicity remain outside exercised evidence.

### Selected invariants and architecture comparison

| IDs | Classification | Selected repair and invariant | Architectural alternative, limits and cost |
| --- | --- | --- | --- |
| SPR-014 | Guarded at shared runtime boundary | One installation transaction stages before replacement and retains recoverable old bytes on rollback failure. | A versioned bundle manager would add link switching/state; ordinary-error staging is sufficient. No crash/concurrent-install atomicity claim. |
| SPR-015 | Structurally prevented | Hook initialization belongs to each shell's local functions; exported parent state cannot stand in for child initialization. | A shared integration generator adds cross-shell complexity; local guards and marker unexport cover inherited and upgrade variants without changing optional sourcing. |
| SPR-016 | Targeted code fix | An ordinary variable stores Zsh's exit status. | A generic shell-hook framework cannot make reserved parameter assignment valid and would add unnecessary abstraction. |
| SPR-017/020 | Guarded at shared runtime boundary | Native ranges use UTF-16; mouse conversion preserves supported buttons and modifiers. | Replacing input protocols adds compatibility cost without fixing the omitted payload. Existing core types suffice; native runtime qualification remains distinct. |
| SPR-018 | Structurally prevented | Confirmation is an overlay, so pane layout and divider coordinates use unchanged space. | Measuring banner height and recomputing every pane/divider would add coupled geometry state. The overlay needs occlusion and pointer interception; wrapping remains bounded by available window space. |
| SPR-019 | Structurally prevented | Explicit fallback callback separates already-delivered keys from native commits; composition and independent insertions remain native. | App-only timers/text receipts cannot distinguish identical later commits. Pinned local GPUI adds maintenance/source size; default forwarding preserves other clients. Cocoa provenance requires native qualification. |
| SPR-021 | Guarded at shared runtime boundary | Captured session ownership, fresh group discovery and identity checks reach ordinary jobs while excluding detached/foreign sessions. | A process supervisor or per-pane registry adds lifecycle ownership without reaching unregistered shell jobs. Session-scoped discovery costs platform adapters and bounded scans; it excludes detached sessions and retains the portable check-to-signal race. |
| SPR-022 | Targeted code fix | Fixture arguments retain their boundaries through both shells. | A new tmux harness would add code without improving argument correctness; passthrough behavior remains unchanged. |
| SPR-023 | Structurally prevented | UI admission does not wait for queue room; cancellation/final outcomes do not depend on event consumption; latest settings/resize retry stays bounded. | Increasing capacities only postpones the cycle. A global asynchronous dispatcher adds ordering/lifetime complexity. Private mailbox cancellation and one pending-value recovery task enforce the invariant locally; ephemeral refused input still reports refusal rather than replaying it. |

Sibling search follows the affected boundary: all install entry points, optional
Bash/Fish/Zsh guards, both marked/selected ranges, terminal mouse listeners,
terminal/Surface text consumers and every pinned native fallback caller, natural
and explicit close, UI settings/resize/observation admission, and both tmux shell
layers. Implementation reviews record actual exercised callers and residual gaps;
this table states the chosen design rather than additional confirmed findings.

### Task 2 verification: SPR-017/018/020

Commit `074cc1a` reconciles UTF-16 and native mouse payload repairs from `090389e`
and renders confirmation as an absolute overlay. Actual EntityInputHandler
regression failed for `é` (caret 2 rather than 1) before the fix, then passed for
ASCII/BMP/supplementary/combining/empty/unmark cases. Real GPUI mouse listeners to
a raw no-echo PTY produced five incorrect SGR packets before the fix and ten
correct button/modifier/motion/release packets afterward. Shift selection emits
zero mouse bytes; link hover and release-only opening pass.

The actual rendered geometry test failed at y=34/height=666 versus y=0/height=700
before overlay, then passed for one/two tabs, both orientations, two sizes,
show/dismiss/resize and pointer drag without initial jump. Label clicks on all
three buttons are intercepted above covered pane/tab content. Verification:
`cargo test -p sprite-app --locked --offline --lib`: 562 passed; terminal tests:
11 passed; layout tests: 4 passed. App all-targets clippy with `-D warnings`,
format and diff checks passed. Native compositor acquisition and Cocoa remain
unexecuted; SPR-019 text provenance is unchanged until its separate task.

### Task 3 verification: SPR-021

Commit `5e50493` introduces private SessionProcesses ownership captured before
the waiter, fresh live session-group discovery, member birth/SID/group validation
and permanent retirement after a complete empty scan. Explicit cleanup repeats
KILL discovery within the existing budget; natural cleanup still owes one HUP
and retains ADR0024's output-drain behavior. ForegroundWatch group APIs remain.
ADR0025 records the invariant, alternatives and portable identity race.

Actual public Bash job-control regression failed before the fix: shutdown returned
in 0.03s with an ordinary job alive. It passes afterward in about 3.1s, with two
ordinary ignoring groups gone and detached/independent sessions still alive.
Public fast-leader and deterministic capture-after-zombie-exit tests pass, as do
selection, identity-change, incomplete/empty scan and permanent retirement tests.
Verification: term library 66 passed; lifecycle 10, session_jobs 2, session_output
8 and input_backpressure 1 passed. Formatting, diff check and strict all-targets
term clippy pass. Native macOS runtime/compilation and restricted libproc/procfs
behavior remain unexecuted; inaccessible scans conservatively wait to the bounded
deadline rather than infer emptiness. Portable killpg retains a check-to-signal
race and is not adversarial containment.

Task 3 independent review established two important refinements, both within
SPR-021: Darwin full BSD-info lookup for foreign users is denied by XNU and
aborts the scan before any signal; ownership was also discarded after natural
worker completion. Actual public-session late request reproduced natural Exited
at 2028ms, then explicit wait returning in 0ms with an ordinary job alive. The
real auto-close path accepts an Ended session and awaits the same handle, so this
is operational. Requirements now retain private cleanup ownership after natural
completion for the existing off-UI blocking wait, while preserving natural
single HUP and the request-relative explicit budget. Proven foreign SIDs are
excluded before protected metadata; unknown possibly owned records remain
incomplete. These variants required repair and re-review; the original immediate-job green
test alone did not resolve them. Subsequent results follow below.

Task 3 review repair `69cc473` filters proven foreign SIDs before Darwin BSD-info
reads and returns remaining SessionProcesses through the private worker join
result after natural completion. Existing off-UI ShutdownHandle::wait escalates
that scope; a worker that already attempted explicit cleanup returns no scope,
avoiding a second budget. The decision uses observed request timing rather than
a late flag during exit publication. New foreign-permission adapter regression
and post-natural public-session regression were red before the repair and pass.
The external late probe now reports natural Exited at 2026ms, explicit wait
3101ms, and no live ordinary survivor. Fresh verification: 67 library tests;
3 session_jobs, 10 lifecycle, 8 session_output and 1 input_backpressure; strict
clippy, fmt and diff check pass. Native macOS remains unexecuted. Focused review of both important findings and affected handoff interleavings
is recorded below.

Task 3 focused re-review passed spec and quality after both repairs, including
exit-publication timing, exactly-once handoff, foreign SID exclusion and no
second explicit budget. Native macOS qualification remains an explicit limit.

### SPR-015 sibling verification during implementation

Root ran actual optional scripts in isolated temporary HOME with Fish 4.9.3,
Zsh 5.9.2 and Bash: source the baseline exported-marker Fish script, then start
new Bash/Zsh and source current integration. Both report `child_hook_status:1`
with empty stderr. Sourcing the updated Fish script in the already initialized
parent first also leaves child Zsh uninitialized: its early local-function
return runs before unexporting the inherited old marker. These are verified
upgrade/inheritance variants of the same root cause, deduplicated into SPR-015.
Evidence: `/tmp/sprite-gap-inherited-marker-variant.txt` and
`/tmp/sprite-gap-inherited-marker-siblings.txt`. Correction must unexport Fish's
marker even on repeat loading and make Bash/Zsh guards depend on local installed
hooks rather than inherited marker alone. No user configurations were changed.
The original ordinary nested-Fish regression remains valid; sibling correction
and review results are recorded below.

### Task 4 verification: SPR-023

Commit `a2d8fab` reconciles the bounded cancelable mailbox and UI admission from
`ad119fd` while retaining Task3 cleanup ownership and ADR0024 output budgets.
Mailbox retention is 32 normal events, one accepted parser/command batch overflow
and two final outcomes. Waiter-driven natural deadline does not depend on inbox
room. UI try_send exposes refusal; a single capacity-one wake task retries only
pending latest settings/Resize, sleeps idle and retires on end/disconnect.
Fieldwise accepted settings permit a partial reload followed by a full revert.

Actual public paused-consumer shutdown failed at 7.31s before the fix and passes
in 0.32s. Actual GPUI callback/installed receivers test asserts colors accepted,
cursor refused, revert/resize refused, then restored worker foreground/cursor and
latest size after pressure clears. Status renders before another snapshot.
Restoring blocking submission stalls that exact callback beyond six seconds.
Disabling the independent natural deadline fails the paused natural-exit test;
restoring it passes with contiguous accepted titles before final outcome.
Five public backpressure tests pass; task retirement/disconnect tests pass.

Verification: 226 term tests (two existing ignored), 596 app tests, benchmark
44.61s, strict clippy, formatting and diff check pass. New pointer fixture now
awaits actual CAPTURING acknowledgement after GO; ten exact SGR packets remain
asserted and five serial focused repeats pass. Hover fixture checks unchanged
span against the current generation after a legitimate resize, rather than
assuming a generation never changes. No production mouse semantics changed.
Independent spec/concurrency review passed with one minor source-established
case: before the first snapshot, refused colors B then revert A leaves UI fallback
B despite admitted A. The actual regression and targeted repair are recorded below.

SPR-015 follow-up commit `bb64ff1` moves Fish unexport before repeat-return and
makes Bash/Zsh guards shell-local. Five inherited/nested/reload subcases were red
before correction; nine shell and 29 full Python tests, syntax and interactive
Bash/Fish/Zsh controls pass. Focused review results follow below.

SPR-015 focused re-review passed spec and quality, independently exercising
partial/complete function guards and export attributes in all three shells.
No open or new actionable initialization finding remains in that scope.

Task 4 review repair `605463d` makes pre-snapshot fallback follow every desired
reload, independently of admitted worker settings. Actual GPUI/real installed
receiver regression reproduced refused B then revert A leaving B (0.16s), then
passed with A (0.23s). All 66 covering terminal-view tests and app clippy/fmt/diff
checks pass. Focused independent review resolves the minor and reports no open
Task4 finding. At this checkpoint nine confirmed findings were resolved; Task5 subsequently
implements SPR-019 below.

### Integrated CI check during final preparation

The exact hosted prohibition command for `thread::sleep|Timer::after|request_animation_frame`
currently exits 1 on bounded, `cfg(test)`-only waits in the new pressure and
pointer regressions. Production scheduling remains event driven. This is an
introduced integration failure, not a new audit finding. Before publication,
keep production sources covered by the prohibition and isolate intentional
blocking test coordination through a narrowly scoped test-only helper; verify a
production-match negative control. Repair and independent review remain pending.

### Task 5 verification: SPR-019

Commits `d271b54` and `6f529e5` add explicit GPUI fallback provenance while
accepting independent native commits regardless of preedit. Default forwarding
preserves other GPUI clients; Sprite suppresses already delivered or refused key
fallback even after focus changes. Cocoa uses synchronous nested dispatch scopes
and invalidates every enclosing scope on marking/transformed/ranged insertion.
Wayland preserves composing one-byte commits as InsertText rather than a
suppressed synthetic key. ADR0027 records the alternatives, compatibility and
maintenance cost; `vendor/gpui.provenance.md` inventories every caller.

Actual raw PTY native-commit test failed with missing Japanese/emoji/ASCII bytes
before the repair, then passed including repeated identical native ASCII commits.
Actual Surface socket old-gate negative control produced two events instead of
four; the repaired handler preserves key, independent ASCII/Japanese and marked
emoji delivery. Removing fallback suppression produces an extra ASCII byte after
the enhanced-protocol sequence; the repaired implementation produces one encoded
key and one independent native commit. Ordinary repeats, Shift, Unicode, delayed
identical input, default Entity/Element forwarding, grid/element Surface focus
transfer and ownership refusal all pass.

Portable tests compile the production Cocoa scope helper (five policies) and the
exact Wayland CommitString/PreeditString/Done arms with protocol/window adapters.
Restoring the old one-byte condition fails the composing-ASCII trace. These are
source-policy tests, not execution of Cocoa or a live compositor/IME.
The checksum-verified published GPUI0.2.2 archive plus the recorded seven-file
patch matches every local source byte; a copied-tree license tamper is rejected.
No dependency versions or features change. Published source/licenses remain
intact, with narrow whitespace exceptions only for preserved Metal indentation
and unified-patch context; first-party checks remain enforced.

Task verification: 573 application library tests, deferred Surface traces,
workspace strict all-target clippy, locked/offline build, first-party/helper
formatting, exact integrity/three portable Python tests and diff check pass.
Independent Task5 review and final integrated gates follow below. Live Cocoa,
X11/Wayland IME and macOS compilation remain unexecuted locally.

### Integrated CI repair

Commit `33ce995` isolates all six deliberate blocking test waits in a dedicated
module excluded from normal compilation with cfg(test). The hosted scheduling
guard excludes only that helper filename and continues inspecting production
sources and all other forbidden-state boundaries. Original guard failed with
six matches; repaired guard passes, and adding a temporary production blocking
call makes that exact guard fail. All original wait durations are preserved.
The 72 covering terminal-view tests, normal application build, strict app clippy,
formatting and diff checks pass. Root independently ran the entire hosted
Forbidden states script extracted from CI: exit0. The stale Surface comment
identified by Task5 review now explains explicit fallback suppression and
independent native commit acceptance. Internal execution reports remain local;
durable outcomes are recorded in this audit and the TSP.

Fresh packaging/release/shell/patch Python verification: 32 tests pass, zero
skips, including temporary Fish4.9.3 and Zsh5.9.2. Bash/Fish/Zsh and macOS
installation/update/bundle shell syntax checks pass. Final full workspace
verification and independent branch review remain pending.

Task5 independent review passed the specification and execution checks, with
one minor stale-comment finding repaired in33ce995. Focused spec/quality
re-review passed and found no remaining Task5 issue. All ten confirmed audit
findings now have reviewed fixes; final branch review and fresh integrated
verification are the remaining publication gates.
