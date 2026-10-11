# Follow-up review of PR 58

The review at `061a925` found conditional failures that ordinary typing does not
exercise. The follow-up uses tests written and observed failing before changing
behavior. Existing wire protocol v1, pinned dependencies and cleanup deadlines
are preserved.

| Finding | Correction | Regression seam |
| --- | --- | --- |
| A refused selection press reused the previous gesture's anchor. | Arm a drag only when BeginSelection is admitted. | Actual GPUI mouse handlers and a session with a full command queue; ordered snapshot checks the old selection and successful next gesture. |
| Abandoned reload/print requests could block the next wave before its timeout began. | Refuse full queue admission immediately; keep claim/abandon arbitration for admitted requests. | Production-capacity relay queue filled with timed-out requests, then reload and print. |
| An unclaimed Surface Open tried to send unnecessary cleanup into the same full queue. | Abandoned Open returns without sending Closed; it cannot create a Surface. | Real Unix socket and serving thread with a full queue. |
| A claimed Surface Open could block while sending cleanup. | Cleanup admission is nonblocking; the existing window clock removes dead Surfaces even in panes that are not rendering, restoring focus and layout. | Full queue after Open is claimed, plus a GPUI clock-driven cleanup test. |
| Native text without a key event left close confirmation armed. | Native commits notify the owning workspace. | GPUI text-input bridge, busy sibling pane, and tab/window/quit confirmation. |
| Workspace shortcuts left unsafe paste armed. | Consumed workspace actions cancel focused-pane pending input. | Real workspace shortcuts and a held paste. |
| Natural exit forgot incomplete hangup work. | Retry transient HUP failures before publishing the outcome; retain retry responsibility for later cleanup if the deadline expires. | Real cleanup state handoff with injected scan failure and existing deadline. |
| Socket cancellation test measured thread scheduling delay. | Assert cancellation readiness with the peer closed, retaining the ordinary wait's lower bound. | Delayed-closer reproduction and cancellation-disabled mutation. |
| Linux tests timed out during allowed final output draining. | Exit test permits the existing six-second drain; input test and benchmark keep their child alive until observing the marker. | Reproduced failures on Linux, parser timing probes, and deadline-renewal mutation that still fails. |
| Login identity test depended on personal startup files. | Use a clean, explicitly configured login shell and wait for a readiness reply before probing identity. | Controlled slow-startup fixture fails before the change; login, identity, terminfo and PATH assertions remain. |

The CI source guards now explicitly fail on each forbidden match. Bash does not
apply `set -e` to a negated command, so earlier matches could be hidden by a
later clean check. Tests run the actual workflow guard against allowed and
forbidden fixtures. Socket-thread fallback waits remain allowed; UI waits and
animation timers remain forbidden.

The original clipboard claim was inaccurate: the base worker considered every
pane unfocused, so child-requested OSC 52 clipboard writes were denied even in
the focused pane. The user's ordinary Copy uses a different path. The PR wires
focus and adds a second clipboard check for writes arriving after focus loss.

Highlight lookup avoids scanning all names; ordered per-id maps still take
logarithmic work. Paint results count shaping calls, not a platform text-system
speedup. The 4,096-entry shared shape pool is not a global memory cap: visible
rows retain their own references. Unique visible glyph/colour combinations can
retain one shape per cell; this is bounded by the visible grid.

Scope limits remain explicit: zero-scrollback row rotation can move a tracked
selection anchor; a slowly progressing Surface writer can retain one of the
bounded connection slots; placeholder alignment uses the prior frame's origin.


## Validation

The local workspace run on macOS with Rust **and** Cargo 1.97.1 completed with
**962 passed, 0 failed, 3 ignored**:

```sh
PATH="$HOME/.cargo/bin:$PATH" cargo +1.97.1 test --workspace --locked --offline --no-fail-fast
PATH="$HOME/.cargo/bin:$PATH" cargo +1.97.1 fmt --all -- --check
PATH="$HOME/.cargo/bin:$PATH" cargo +1.97.1 clippy --workspace --all-targets --locked --offline -- -D warnings
python3 -m unittest scripts.test_ci_policy scripts.test_gpui_patch -v
```

The six Python tests pass. Clippy reports no workspace errors; existing vendored
GPUI/future-compatibility warnings remain. A later test-helper-only change was
checked with its focused Surface writer test. Source guard tests reject every
prohibited fixture and accept the repository. Removing the explicit guard exits
reproduces seven failures, proving earlier matches cannot be silently ignored.

The natural-exit and sustained-output failures were reproduced and corrected
in Ubuntu x86_64 under emulation, using Rust 1.97.1. This is targeted Linux
evidence, not a native Arch/full-workspace result. A deliberately renewed drain
deadline still fails at ten seconds; restoring the production deadline passes.
The clean login fixture also passes on Linux, and removing login mode makes
its readiness assertion fail.

Each new product regression was observed failing before its fix: refused
selection, second-wave relay admission, unclaimed and claimed Surface cleanup,
dead-Surface reconciliation, native text, workspace shortcut cancellation, and
both natural/retained hangup obligations. Cleanup tests inject incomplete scans
at the real state handoff rather than forcing kernel process-table failure.
GPUI tests exercise the real input bridge; no manual OS IME session is claimed.

The socket test proves cancellation readiness and the ordinary rest duration;
it no longer asserts sub-period wake latency under thread scheduling. Existing
listener shutdown coverage remains. Independent review found no blocking
correctness issues in the fixes.


## Native CI benchmark follow-up

The first follow-up run on native Arch passed the two original Linux regressions
but exposed the same early-exit fixture in `sprite-term-bench`. The unchanged
benchmark report test failed with `snapshot stream ended waiting for MARK:Z`;
this was reproduced locally on Linux before editing the harness.

The input-under-load benchmark now waits for its flood producer to stop and
keeps the shell alive until its marker is observed. `finish(session)` then
performs the existing explicit cleanup. Terminal drain deadlines and report
assertions are unchanged. Searches for `read line; kill $producer` and for
`yes ` in terminal Rust sources found two sites: the integration fixture already
fixed above and this benchmark. Both are covered by their existing failing
integration tests. This metric now excludes natural-exit draining; compare
future numbers with a new baseline, not the old mixed-lifetime measurement.

The unchanged benchmark report test passes after the repair on both macOS and
Linux under emulation. All-target `sprite-term` Clippy and workspace formatting
also pass. These scoped checks follow the full 962-test workspace run above.

Independent review identified a missing-marker hang: a blocking snapshot receive
could never reach the timeout check once output became silent. The under-load
measurement now observes the async stream with an absolute deadline and returns
through session cleanup on either success or failure. A new real-PTY test keeps
a shell alive after printing the wrong marker, checks the timeout error, and
asserts that the shell has been reaped before the measurement returns. It failed
at its ten-second watchdog before the timed receive and passed after the fix.
This adds one test. The terminal engine and its public API are unchanged.
