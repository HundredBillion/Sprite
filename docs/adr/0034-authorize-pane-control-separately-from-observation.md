# Authorize pane control separately from observation

Approved for issue #106 on 2026-10-26.

Pane Control uses separate credentials and a versioned local protocol, over the
existing protected local transport. Human approval grants read, manage and input
rights for all panes in one window or an explicit set. Observation and Surface
credentials cannot approve grants. An observation caller can only request one.
The requester pane identity is a claim, not authentication of a program.

Extending observation would weaken its deliberately narrow read-only interface
and require multi-key authentication. Automating keyboard actions would target
focus instead of identity. Separate grant endpoints preserve both contracts.

A public address includes the window incarnation and pane identity. Same-window
moves transfer the existing handle, not its Terminal Session, and resolve live
tab membership on observation and exit. Cross-window operations use an explicit
grant for each window; cross-window moves are refused without side effects.

Authorization is checked again at execution and human confirmation. A pending
close cannot survive revocation as permission to act. Repeated protocol requests
never confirm a close. Existing shutdown ownership and process cleanup apply;
no arbitrary host-process signal operation is exposed.

Grilling scenarios: observation credentials requesting their own elevation;
forged requester identity; revocation after admission; moved pane exiting with
its original tab ID; resize followed by redraw; final-pane response racing
endpoint shutdown. The TSP must exercise the public operation path for these
scenarios. None requires the canceled command/event roadmap.

Amended 2026-10-27: a selected-pane grant's scope holds only open panes. The
approved design bounded agent-created resources at 128 live panes, but the
first implementation kept every pane a grant had ever created in its scope, so
a long-lived grant was refused as `busy` after 128 creations even with none
open. The workspace's layout refresh, which every way of closing a pane runs,
now removes closed panes from every grant's scope. This can't widen a grant:
pane identities are never reused, so a removed identity can never name a pane
again. The 128 limit still bounds a scope's size and what one grant can hold
open.
