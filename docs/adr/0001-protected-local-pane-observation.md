# Provide protected local Pane Observation without an LLM dependency

Phase 1 gives local tools launched inside a Sprite Window automatic, read-only
access to Pane Snapshots in that window without bundling an LLM or granting pane
control. The supported interface is the bundled `sprite panes snapshot` command;
its versioned JSON is stable while the private per-window Unix-socket protocol
remains internal.

This capability deliberately trades prompts for convenience. A temporary
window key excludes clients that were never given access, but an authorized
child can copy that key, and printed terminal content may contain secrets or
prompt injection. The contract therefore labels all terminal content untrusted,
provides no remote or mutation operations, and includes a live kill switch that
destroys the endpoint and key. The PRD owns the exact schema, scope, limits,
timeouts, and re-enable behavior.
