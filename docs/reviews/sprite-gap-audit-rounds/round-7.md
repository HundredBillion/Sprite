# Sprite gap audit round 7: macOS/package lifecycle/CI

Revision inspected: 091efef3f56aa3b9d0ce90d121e05b1ec93cba4d. Read-only review; no checkout, fixes, commits, pushes, workflow dispatches, real installs or native macOS execution. Linux host. Reviewed the requesting-code-review template, CONTEXT.md, crates/CONTEXT.md, prior sprite-bug-audit.md, macOS PRD/TSP and packaging history, ADR 0011/0015/0024, shell identity, Unix pump/start/closing, app window close hooks, pinned GPUI termination paths and CI.

## Strengths

The installed CLI is a link into one bundle, so replacement need not synchronize two binaries. The bundle seals its terminfo and version and verifies codesigning. The PTY pump owns its duplicate, uses nonblocking input and private cancellation; this preserves the deliberately selected ADR 0015 solution to Darwin's small PTY input queue. CI explicitly distinguishes compile/headless coverage from interactive macOS and compositor gates.

## Confirmed new issue

### R7-001 — P2 / Important: failed app copy removes the working installation

- Anchors: packaging/macos/update.sh:38–40; installed CLI link :46. The script runs `rm -rf "$app"` before `ditto target/Sprite.app "$app"` under `set -eu`.
- Scenario: re-run the documented macOS updater with a working /Applications/Sprite.app and existing /usr/local/bin/sprite link. Build and bundle succeed, but the installation copy fails (e.g. space/I/O failure). The previous app has already been deleted. The updater exits immediately without restoration; the existing CLI link is now dangling or points into a partial replacement.
- Executed proof: byte-for-byte copy of the actual update.sh into /tmp/sprite-round7-install-9m87p7zi/packaging/macos/update.sh. PATH fixtures stub successful build/terminfo/bundle; the rm wrapper accepts only the exact `-rf /Applications/Sprite.app` call and remaps it to the temporary installed app. The ditto wrapper injects exit 73 before copying. No command touches /Applications or invokes privileged installation. `/bin/sh <copied-update.sh>` returned 73, with `Original app remains: False`, `Existing CLI symlink resolves: False`, `Replacement bundle built: True`. Full output: /tmp/sprite-round7-install-9m87p7zi/result.txt.
- Exact rerun: `python3 /tmp/sprite-round7-repro.py` (creates only a fresh temporary fixture and asserts the observed failure). Reproducer saved outside checkout.
- Scope of proof: actual script control flow, ordinary copy failure and resulting filesystem loss exercised on Linux; native ditto, codesign, Finder and real package install not executed. The reproducer deliberately injects a command failure; it does not claim to reproduce the cause of a native disk failure.
- Root cause: delete-before-copy violates retaining a working installation until replacement is ready. This resolves the prior audit's explicit unconfirmed “macOS replacement recovery” gap. It is not SPR-011 local terminfo preparation, SPR-012 version drift or SPR-013 Python release transaction recovery.
- History: introduced by cd1e975a5ce10b5cd943b4f5155304837f02c675 (2026-09-07). Neither that commit nor the PRD requires delete-before-copy; they require one binary reached via bundle and CLI link. Preserve that decision while staging the replacement alongside the destination and keeping/restoring the previous bundle when installation fails. Account for staged-copy failure and replacement-rename failure; do not overclaim crash atomicity.

## Unconfirmed platform gap

### R7-C1 — native Dock/system quit may bypass close confirmation and owned cleanup

- Anchors: sprite-app/src/main.rs:117–124 installs only `on_window_should_close`; workspace/close_gate.rs:66–86/135–147 implements the custom close/keyboard cleanup gate. No Sprite `on_app_quit` or native application-termination handler was found. Pinned gpui 0.2.2 platform/mac/platform.rs:1421 `will_terminate` calls the platform quit callback; app.rs:686 installs callback to App::shutdown; app.rs:697 clears windows and only awaits registered app-quit observers (100ms). TerminalSession Drop at session.rs:255–260 requests shutdown without joining; worker/closing.rs needs two/three seconds for TERM/KILL.
- Concrete schedule to validate on macOS: a pane child ignores HUP/TERM; remove another pane to start pending cleanup; issue Quit from the Dock/system application mechanism instead of Sprite's custom Cmd+Q handler. Determine whether Cocoa consults the Sprite window close callback or reaches `will_terminate` directly, and whether child cleanup finishes before process termination.
- Source concern: the observed App shutdown route does not invoke Workspace::shutdown_and_quit, so it has no owned wait for current/pending session cleanup. Possible program interruption without confirmation and child leakage. Native termination event schedule is unexecuted on this Linux host; do not promote it to confirmed solely from source.
- History: ADR 0024 and SPR-007 fix converge Sprite-owned cx.quit/native-window-close/custom-keyboard routes and retain pending cleanup. This is a different external entry point. Any remedy must preserve the bounded owner/cleanup architecture and not revert to detached cleanup or blocking UI joins. Report as targeted native validation gap pending evidence.

## Other inspected limits

- Darwin PTY EOF/poll readiness, F_DUPFD_CLOEXEC, nonblocking wake/cancel and process-name libproc paths were source-inspected only; no confirmed Darwin-specific defect. Do not reverse ADR 0015 based on Linux observations.
- Shell fallback selects /bin/zsh on Darwin; packaged terminfo canonicalizes executable path before Contents/Resources lookup; PATH deliberately uses executable directory. Existing explicit-command identity fix remains present. No native Finder/Zsh/Fish runtime executed.
- Info.plist advertises 10.15.7 per PRD/GPUI compilation floor; macos-latest cannot establish runtime compatibility at that minimum. No independently demonstrated minimum-version defect; retain as coverage limitation.
- Bundle output is recreated before icon/terminfo/signing succeeds, but it is a build artifact, not the installed app; no separate installation-loss issue reported for this.
- Child-waiter thread spawn failure after child creation (worker/start.rs:425) was noticed and communicated to the terminal-focused reviewer; not probed or classified here.

## Executed versus inspected coverage

Executed: two read-only gh run view calls for run37532424660; actual updater failure fixture; git log/show/status/rev-parse and source searches. No Cargo tests were run for this review. Current master run HEAD matches the revision: release automation and Arch jobs passed; macOS job remained in_progress in Offline locked gate, bundle verification pending at the last query. That does not prove macOS gates pass or fail. No indefinite wait or remote workflows were launched. CI evidence: https://github.com/HundredBillion/Sprite/actions/runs/37532424660/job/112504870269.

Inspected: bundle/install copy/link failure ordering; PRD constraints and introducing commits; terminfo resolution through symlink; Darwin pump cancellation/EOF/readiness; window-close/custom-quit and GPUI native termination; CI bundle proof and claimed minimum OS support. No native macOS/Finder/Dock interaction, real install, full native shell runtime or old-macOS execution.

## Over-engineering / simplification

Lean already for this focused scope. No speculative refactors.

## Assessment

With fixes in the focused macOS updater scope: the ordinary copy failure is proven to remove a working installation. Native Dock termination remains a separate unconfirmed coverage gap. This verdict does not certify the whole repository or native macOS runtime.

Canonical finding IDs and final parent verification are recorded in `../sprite-gap-audit.md`. Temporary probe paths identify this review session; they are not repository scripts.
