# Keep the Surface Channel separate from Pane Observation

Surfaces travel over a **second endpoint** — its own socket file, exported to
children as `SPRITE_SURFACE_SOCKET`, speaking newline-delimited JSON — that
shares the observation key, runtime directory, and authentication code but
**never its grammar**. Pane Observation remains read-only by construction.

Taken while grilling the native-surfaces PRD on 2026-09-07, after the PRD's
first draft had put surface verbs on the observation socket.

## Why

Every child Sprite spawns already holds `SPRITE_PANE`,
`SPRITE_OBSERVATION_SOCKET`, and `SPRITE_OBSERVATION_KEY`, so reusing that
socket looked free. It was not, for two reasons.

**The observation grammar's defining promise is that nothing on it can
mutate.** `observation/request.rs` states it in code: "`broker` promises that
a request which could mutate cannot be constructed," and "there is
deliberately no variant that writes, sends input, subscribes, or opens a
stream." The glossary defines Pane Observation as access that "never grants
control of a Pane or its child." That promise is what makes it safe for any
program — and for an LLM reading a pane through it — to hold the socket. A
surface is nothing but control: open, update, close, register, and a stream
carrying input back. Adding those verbs would not extend the promise; it would
repeal it.

**The framing is wrong for the payload.** An observation request "crosses two
processes as a line of text" of space-separated words; a surface description
is a structured document flowing in both directions. A second grammar was
needed regardless.

**Distinguishing verbs by protocol token on one socket** was considered and
rejected: it makes the read-only promise true of *some lines on a socket*
rather than of the socket, which is exactly the blur the comment in
`request.rs` was written to prevent. Zed and VS Code's Agent Host keep the
same separation — Zed labels every agent tool `ToolKind::Read` or
`ToolKind::Edit` and gates only edits; VS Code routes every tool call through
a `CanUseTool` gate — and a second endpoint is the structural form of that
rule: control is *absent* from the read line, not merely denied.

## What follows from it

**Cost is one descriptor and one parked thread.** The endpoint runs one thread
asleep in `accept()` per listener; per message the two designs share the same
kernel path. The chatty grid stream — an update per keystroke — stays off the
listener an LLM reads through.

**A future permission model has a home.** Any gating of who may draw into
which pane belongs on the Surface Channel, where acting happens; the
observation line never needs it.

**Two glossary terms defined against each other.** Surface Channel is "the
one line that grants control of what a Pane shows; Pane Observation grants
none, and the two never share a grammar."

**Descriptions are not capped small.** Zed's own agent transport records that
"512 KiB is not enough" for its stream buffer
(`crates/agent_servers/src/acp.rs:674`); an SVG icon set in a description can
be large.
