# Bound terminal delivery and cancel event pressure

A Terminal Session exposes blocking `send` for off-thread consumers and
nonblocking `try_send` for the GPUI thread. Both validate owned input before
queueing. Raw input, committed text, and combined logical-key/text data are
limited to 16 KiB; paste and confirmed paste retain the existing 1 MiB clipboard
limit. The seventeen-slot worker inbox and sixteen 16 KiB output permits stay
unchanged. Accepted commands keep their existing inbox order. Saturated UI
submissions set the pane status; observation requests refuse and remove their
waiter. Hyperlink hover and click requests acquire outstanding response state
only after a running session accepts their submission; refusal leaves them
retryable.
The observation registry uses nonblocking submission even off-thread because
its mutex is also needed by the GPUI event consumer.

Live configuration acknowledgement follows each owner independently. Renderer
font metrics, padding, fallback colors, and texture budget record their actual
local application even when a terminal command refuses. The accepted terminal
color defaults and cursor settings advance only on successful submission; the
geometry cache likewise records only accepted Resize commands. Reconciliation
compares these owner states with the requested settings, so returning from a
partially applied B to A restores local state and retries accepted terminal
groups that still need reverting. An unchanged reload does not reset terminal
defaults. Submission acknowledges queue acceptance, not worker completion.

Lifecycle delivery uses one mutex-owned mailbox: thirty-two normal events,
at most one whole produced mutation batch waiting for space, and up to two
reserved final outcomes. A parser chunk transfers its reply failure, ordered
notices, and clipboard writes together. The worker cannot perform another
mutation while that retained batch waits. A condition variable applies producer
pressure; a single bounded async-channel wake token supports either public event
receiver method. Empty batches do not wake the receiver. No forwarding task,
extra runtime, unbounded queue, or larger output buffer is introduced.

Requested shutdown wakes event pressure independently of the inbox. Receiver
drop also wakes pressure and releases retained payloads. Cleanup seals its error
and child outcome without waiting for consumption. The receiver drains normal
events, the retained batch, and final outcomes in that order, including after
the worker joins. A worker-owned completion guard seals an error if Rust unwinding
bypasses cleanup; retaining the session's cancellation handle cannot keep the
stream open after its producer terminates.

The child waiter starts a two-second natural drain deadline before reporting
`ChildExited` into the shared inbox. This keeps child exit observable even when
the worker is parked on event pressure or its exit report is behind queued PTY
chunks. During natural drain, the pump remains live and output continues through
the terminal owner; intermediate snapshot projections are skipped, then the
finite read deadline cancels the pump. Already accepted PTY chunks, including a
send waiting on the inbox, are then parsed before the final projection replaces
the latest-only slot. This queue drain ends at the existing six-second budget
measured from direct-child exit. It skips further mutations if a produced event
batch is still retained under pressure. EOF can complete the drain sooner.
A descendant retaining the PTY cannot extend the fixed deadline. Requested shutdown still uses the established HUP/TERM/KILL
cleanup policy; natural completion owes the established single HUP.

Two seconds leaves room for a bounded backlog in debug builds: an initial
500 ms deadline failed the existing flood/input benchmark before its final
marker reached the terminal. A two-second cutoff also intermittently failed to
show the marker; a successful instrumented run parsed the marker with only
179 ms remaining. Cancellation therefore stops new reads while accepted chunks
retain their bounded final drain. Omitting obsolete intermediate
projections avoids spending the drain budget on rendering instead of parsing.
The finite drain still deliberately stops unread output at its deadline, including a descendant
that keeps producing output or an event receiver that never resumes. Already
produced event batches and the final outcome survive this stop. Unbounded output
retention and unconditional lossless delivery of future, unparsed PTY output
would contradict bounded memory and finite cleanup.

This refines ADR 0010's lifecycle ownership and ADR 0011's cancellation contract.
Real PTY regressions cover saturated UI submission, held events through shutdown,
whole-batch ordering, resumed consumption, receiver drop, natural exit under
event pressure, and final title/text survival with a stubborn descendant. GPUI
and observation regressions cover visible refusal, hyperlink recovery, retryable
geometry, and restoration after partial local or terminal settings acceptance.

## Reconciliation with merged audits (2026-10-06)

PR52 has been reconciled with master `a303969`, including merged PR53 and PR54. ADR0024–0027 describe the current drain, ordinary-job ownership, UI retry and native-text contracts; the original ADR0021 and task evidence are historical where those decisions differ. Local package versions now derive from the workspace manifest; release updates retain staging/rollback and update only the fixed-version distribution recipe. The original fixed local-recipe validation assertions have been superseded. Current integrated tests and master regressions cover these retained decisions.

The remaining PR52 changes retain Surface CLI cancellation, configuration response validation, Surface description replacement checks, drawing decorations and link readiness. Its submission regression tests run against master’s admission/retry implementation. Pane cleanup ownership and the native-text provenance patch are retained.

Merge verification: `TERM=dumb cargo test --workspace --locked --offline` passed **847 tests, 0 failed, 2 optional ignored**. Independent full resulting-PR review against master found no actionable runtime regression. Native desktop and Cocoa scheduling remain unexecuted.

Amended (bug-class audit): notices are published per worker pass, not per parser chunk.
