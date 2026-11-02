# Open pane control to every process of the owner

Approved by the owner on 2026-11-01. Supersedes ADR 0034 and ADR 0001; amends
ADR 0013.

Every Sprite Window serves Pane Control on `window-<id>.sock` in the per-user
runtime directory. The directory is 0700 and the socket 0600, and that is the
only check: any process running as the same user can list, read, create,
label, move, focus, resize, close and type into every Pane, and reload or print
the window's configuration. There are no grants, keys, access files or prompts.

The owner's agents run under a long-lived orchestrator that no Sprite window
started, so the old model made every agent wait for a person to click Allow.
The owner wants that to never happen, and wants no Sprite code that asks a
person to approve an action. A process running as the owner can already type
into the owner's shells by other means, so a prompt protected little and cost
every orchestration a click.

Removed with it: the observation socket and its key, `sprite panes snapshot`,
the `[pane_observation]` setting (a leftover section is ignored with one log
warning), the unsafe-paste hold, and the confirmation before closing a busy
Pane, tab or window. The quit toast stays. The Surface Channel keeps its own
key, given only to a window's own sessions, so drawing native UI in a Pane still
needs that Pane's environment.

A Pane Address is `<window>:<pane>`, and every session gets its own in
`SPRITE_PANE`. Window ids are 24 hex characters, which keeps the socket path
within macOS's 103-byte limit inside `$TMPDIR`.

Considered and rejected: a key file readable by the owner (the same boundary
with more moving parts) and one socket for all windows (a single owner process
Sprite doesn't have; each window already owns its own lifetime).
