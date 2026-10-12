# Admit commands through a control lane the worker drains first

A Terminal Session's worker read one bounded queue (17 slots: 16 output
permits and one spare) for PTY output, commands, capture requests and helper
reports. Under an output flood the spare slot was shared between the
application's commands and the snapshot consumer's capture requests, so a
keystroke could be refused as "queue is full" (GAP-C11). Commands and history
requests now wait on a second channel, the control lane (64 deep), which the
worker checks at the top of every pass and after every message it takes from
the inbox. A control send is followed by a nonblocking `Wake` on the inbox:
it lands when the worker is idle and wakes it; it fails when the worker is
busy, which is exactly when the worker will reach the control lane on its
own. Shutdown keeps its flag and its inbox knock; the closing loops read only
the inbox, so closing applies no commands.

Control-lane messages (commands, history requests) are therefore served before
PTY output that was already queued in the inbox. This replaces the earlier
single-queue FIFO, and the test
`a_queued_command_is_applied_before_output_that_arrived_first` pins it at both
drain sites. The reason is that a keystroke must never wait behind a flood.
History requests and the Pane Snapshots taken for Pane Observation overtake
queued output the same way: a capture requested after output was received but
before it was parsed reflects the screen as shown, not the bytes in flight, so
a snapshot can miss up to 16 chunks already read but not yet parsed.
Control-first is strict priority with no fairness bound: the only producers
that could sustain it are observation connections issuing history requests,
and a bound is a follow-up, not a change made here.

Every nonblocking offer answers with one type, `Admitted<T>`: `Accepted`,
`Saturated(T)` or `Closed(T)`, the latter two handing the command back.
`Err` from `try_admit` is the command itself being refused (too large, or the
session shutting down), carried by `SessionError.kind`. No caller reads error
text to decide anything.

The same shape splits the Sprite Window's Surface requests into a stream
lane (64; a connection blocks on it) and a one-shot lane (16; handshakes are
refused as busy rather than queued behind another program's stream,
GAP-C12). The lanes live in `surface/lanes.rs` and the window drains them in
`workspace/lane_drain.rs`. The drain follows a yield-once rule: the window's
drain task yields to the executor after every served request, so a lane that
never runs dry cannot starve the other lane or the frame. Without the yield
the second lane never ran; the test
`a_lane_that_never_runs_dry_does_not_starve_the_other_lane` pins it.

Rejected: one larger queue (a flood is unbounded; any shared bound is
reachable); priority inside one queue (a reorderable queue needs a lock on
the hot path the pump writes to); a third thread to referee (ADR 0008 keeps
one owner per session).
