# Eliminate reviewed terminal failure classes

## Outcome

Repair the eleven findings from the repository review at `eaa553e`, using structural changes where ordering, cancellation, or duplicated state caused the defect. Preserve normal terminal behavior and bound memory under stalled consumers.

## Constraints

- Rust 1.97.1, pinned existing dependencies, one libghostty-owning worker per Terminal Session.
- No new runtime or dependency unless an existing dependency cannot implement a necessary OS operation.
- Keep Surface Channel authentication, observation scope, and protocol version 1.
- No publication, installation, or messages to third parties.
- Implement on `fix/review-bug-classes` in the isolated worktree; leave the original checkout unchanged.
- Use real process, stream, and GPUI regression tests; tests must fail before the corresponding repair.

## Terminal lifecycle and pressure

The application must never synchronously wait for a worker queue slot on the GPUI thread. Add a nonblocking command submission method with explicit saturation errors; retain blocking submission for existing off-thread consumers. Validate variable-sized input at both submission methods.

Replace the worker's blocking event-channel dependency with a bounded shared event mailbox. It owns normal events, at most one produced batch waiting for space, and reserved terminal outcome events. Ordinary publication may apply backpressure to the worker, but cancellation interrupts that wait. Queued events and the terminal outcome remain readable after worker termination. Dropping the event consumer wakes the producer. The mailbox must not become an unbounded forwarding queue or spawn tasks for each command.

Direct-child exit starts a finite natural-exit drain phase. Drain the child's final output, including queued chunks and readable PTY output, before cancellation and the final snapshot. A descendant holding the slave open must not prevent completion. Requested shutdown retains the existing HUP/TERM/KILL policy.

## Surface client lifecycle

The CLI must finish when stdin closes, the socket reaches EOF, or output delivery fails. Use one cancellation owner and OS readiness for the actual input descriptor so no blocked reader thread remains behind. Reading pretty-printed and consecutive JSON documents must keep working; a partial later document must be interruptible. Normal stdin EOF still half-closes the socket and drains the server's final events.

## UI state

Texture budget changes reconcile the cache against the retained snapshot immediately. No new terminal generation is required to restore a visible image. Every legal image id, including `u32::MAX`, is evictable.

Surface body kind is stable after opening. An incompatible replacement is refused atomically and preserves existing content. Element descriptions must not admit grid/list roots or nested specialized roots that render as empty elements.

IME positions use UTF-16 units. Terminal mouse reporting preserves actual button and modifiers, including buttonless motion; selection override and hyperlink behavior remain intact. Decorations paint independently of glyph ink so whitespace does not break an underline.

## Local correctness

Configuration printing distinguishes denied/malformed replies from valid settings and returns a failure with empty stdout. Release preparation keeps both Arch recipes and the README synchronized with the workspace version. Fish's duplicate-source guard returns from the sourced script rather than terminating the shell.

## Evidence and scope

The earlier whole-workspace suite and nine release-script tests passed. Separate probes reproduced terminal queue deadlock, natural exit blocked by descendants, Surface CLI EOF hang, denied configuration success, incorrect UTF-16 ranges, and failed image recovery. Mouse motion, Surface kind transitions, decoration omission, stale packaging version, and Fish duplicate sourcing were established by inspection and now require covering checks where the runtime is available.

## Decisions

Choose focused changes to the owning modules over a full runtime rewrite or unbounded queues. Keep off-thread command compatibility and make the GPUI submission contract explicitly nonblocking. Preserve pending event data while allowing cleanup to complete independently of consumption. Reject Surface kind changes rather than implementing a new undocumented transition.

The user's workflow preference preapproves routine design, PRD, planning, implementation, and verification gates. Requirements were hardened against retained event consumers, output tails, repeated cache changes, partial JSON input, native UTF-16 semantics, and incompatible Surface updates.

## Reconciliation with merged audits (2026-10-06)

PR52 has been reconciled with master `a303969`, including merged PR53 and PR54. ADR0024–0027 describe the current drain, ordinary-job ownership, UI retry and native-text contracts; the original ADR0021 and task evidence are historical where those decisions differ. Local package versions now derive from the workspace manifest; release updates retain staging/rollback and update only the fixed-version distribution recipe. The original fixed local-recipe validation assertions have been superseded. Current integrated tests and master regressions cover these retained decisions.

The remaining PR52 changes retain Surface CLI cancellation, configuration response validation, Surface description replacement checks, drawing decorations and link readiness. Its submission regression tests run against master’s admission/retry implementation. Pane cleanup ownership and the native-text provenance patch are retained.

Merge verification: `TERM=dumb cargo test --workspace --locked --offline` passed **847 tests, 0 failed, 2 optional ignored**. Independent full resulting-PR review against master found no actionable runtime regression. Native desktop and Cocoa scheduling remain unexecuted.
