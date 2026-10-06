# Sprite remaining-gap review — 2026-10-06

## Scope and baseline

Five additional read-only review-agent rounds target the coverage gaps from
`sprite-bug-audit.md`. Existing defects are in scope. This pass records evidence
and recommendations; it does not implement fixes, commit, push, or publish.
Production code, index, branch, and HEAD stay unchanged during review. Temporary
probes and reviewer reports live outside the checkout.

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

| ID | Severity | Discovery round | Scenario and location | Evidence | Root cause | Status |
| --- | --- | --- | --- | --- | --- | --- |
| SPR-014 | P2 | 7 | macOS update copy fails after a working installation exists; `packaging/macos/update.sh:39–40` | Parent reran `python3 /tmp/sprite-round7-repro.py`: actual byte-for-byte updater in a safe temporary fixture exits 73, original app absent, existing CLI link dangling. Native copy failure is injected; no real installation occurs. | Installed app is deleted before replacement copy succeeds; no recovery. | Confirmed; review-only, unfixed |
| SPR-015 | P2 | 8 | Nested Fish sources installed integration after parent did; `crates/sprite-term/shell-integration/sprite.fish:6–9` | Parent reran `python3 /tmp/sprite-gap8-shells/probe.py` with temporary Fish 4.9.3: parent hooks exist and emit OSC, child sources same actual file but has no hooks and emits no prompt sequences. | Exported initialization marker is inherited; shell-local functions are not. Guard mistakes parent initialization for child initialization. | Confirmed; review-only, unfixed |
| SPR-016 | P2 | 8 | Zsh prompt hook runs after sourcing integration; `crates/sprite-term/shell-integration/sprite.zsh:15–19` | Same parent-rerun probe with temporary Zsh 5.9.2: `__sprite_precmd:1: read-only variable: status`, no OSC 133 D/A or OSC 7. Probe's overall shell exit is 0; the error and missing hook output prove the failure. | `status` is a readonly Zsh special parameter; local assignment aborts hook. | Confirmed; review-only, unfixed |
| SPR-017 | P2 | 6 | Non-ASCII IME marked/selected range; `crates/sprite-app/src/terminal_view/input.rs:222,234` | Parent reran isolated actual-handler GPUI probe: `é` returns caret 2..2 and marked 0..2, expected UTF-16 1..1 and 0..1. Pinned Cocoa consumers forward these ranges into NSRange. | String byte length substitutes for UTF-16 code units. | Confirmed; review-only, unfixed; overlaps unmerged 090389e |
| SPR-018 | P2 | 6 | Close confirmation appears above vertically split panes; `workspace/mod.rs:364–374,472–493`, `workspace/divider.rs:250–279` | Parent reran actual GPUI render probe: pane origin changes 0→34px and height 1080→1046, divider cache still origin 0/boundary 540. Existing allocations remain based on pre-banner room. | Cached pane geometry accounts only for tab chrome; dynamic banner changes actual container room and pointer coordinates. | Confirmed; review-only, unfixed; promotes prior banner candidate |
| SPR-019 | P2 | 6 | Wayland multibyte commit without preedit; `terminal_view/input.rs:259–260` | Pinned CommitString("日本")→InsertText→replace_text_in_range path has no KeyDown fallback. Actual handler GPUI command recorder emits zero commands; marked-text control emits CommitText("日本"). Parent reran. | Preedit presence incorrectly stands in for whether text was already delivered by keydown. | Confirmed by handler probe and demonstrated native caller path; review-only, unfixed |
| SPR-020 | P2 | 6 | Terminal mouse reporting for middle/right buttons or Alt/Control; `terminal_view/render.rs:473–475,555–557`, `terminal_view/input.rs:158–164` | Parent inspected entire terminal mouse listener/route census: only left down/up listeners; route always sends Left and hard-codes Alt/Control false. Motion similarly discards supplied pressed button. | Native mouse payload omitted at application-to-core boundary despite supported core fields. | Confirmed by demonstrated caller path; native event reproduction unexecuted; unfixed; overlaps unmerged 090389e |
| SPR-021 | P1 | 10 | Explicit shutdown with ordinary Bash job-control background group; `sprite-term/src/worker/closing.rs:79,188–200` | Parent reran actual public-session `/tmp/sprite-round10-process.py`: shell disappears, wait completes in 50ms, background sleep remains live/reparented in a separate group within the same terminal session. Probe kills only its tracked children afterward. | Cleanup enumerates recorded shell and foreground groups, excluding other ordinary job-control groups; completion predicate checks only enumerated groups. | Confirmed; review-only, unfixed; promotes prior process-group candidate |
| SPR-022 | P3 | 10 | Rust tmux graphics fixture with TMPDIR containing spaces; `sprite-term/tests/graphics_tmux.rs:78–83` | Parent reran `/tmp/sprite-round10-tmux.py`: exact baseline test binary passes with ordinary TMPDIR, fails with spaced TMPDIR (exit 101, snapshot stream ended). | Generated shell command interpolates fixture config/image paths without shell quoting. | Confirmed test-fixture portability defect; review-only, unfixed; no engine graphics regression claimed |
| SPR-023 | P1 | 9 | Child event burst and multi-command settings reload while UI consumption is delayed; `sprite-term/src/worker/mod.rs:161`, `session.rs:233`, `terminal_view/theme.rs:239,245` | Parent independently reran serialized actual-session GPUI probe: callback prints before, stalls past 6s; identical case with dispatcher draining notices prints after and passes in 2.69s. Actual ordinary event/snapshot tasks are installed. | Worker blocks publishing to 32-event queue; pump fills 16 of 17 inbox slots; multiple blocking UI command sends exhaust the reserved slot and prevent the same UI thread draining events. | Confirmed; review-only, unfixed; promotes C-002; overlaps unmerged ad119fd |

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

These are review recommendations, not approved implementation choices. No
production change or new architecture is introduced by this pass.

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

## Final review result and remaining limits

All five additional read-only review rounds are complete. **Ten confirmed
findings**: two P1, seven P2, one P3 (SPR-014–SPR-023). They remain unfixed because
the current request is for reviews. No prior resolved SPR-001–SPR-013 repair was
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

Production source and HEAD remain at the baseline. Only this report, the five
retained reports, and a follow-up link in the prior audit are edited. No commit,
push, merge, publishing, system installation, user-dotfile edit or desktop
interaction was performed.
