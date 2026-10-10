# Sprite Terminal Core (Phase 1)

The terminal foundation that powers the standalone Sprite Terminal while
remaining independent of Croft and future IDE products.

## Language

**Terminal Core**:
The Sprite component that owns terminal sessions and exposes their behavior to
the Sprite application.
_Avoid_: backend, terminal SDK, libghostty wrapper

**Checkpoint**:
A testable stage that extends the same Phase 1 architecture and leaves the
product runnable for inspection. It is neither a separate release nor a
disposable prototype.
_Avoid_: phase, prototype, temporary implementation

**Terminal Session**:
One independent running terminal and its child process. A Terminal Session
belongs to exactly one Pane and is never shared between Panes.
After its direct child exits, the owner allows a fixed two seconds output drain
before cancelling a PTY retained by descendants, then parses already accepted
output under a bounded final-drain budget before the final snapshot. Explicit
shutdown keeps the
existing bounded HUP/TERM/KILL cleanup policy.
_Avoid_: shell (only one possible child), terminal instance

**Command Admission**:
The point at which a Terminal Session accepts an application request. A refused
request has not changed terminal state; a Pane can retain the latest desired
settings or size until it can admit them.
_Avoid_: applied setting (admission precedes the terminal's change)

**Accepted Event Batch**:
The ordered notices produced by one completed terminal change. Ending a session
preserves this batch before its final outcome, even when its reader is paused.
_Avoid_: pending output (unparsed terminal output is a different obligation)

**Ordinary Session Job**:
A running program that remains in its Terminal Session's process session,
including background and foreground jobs. A program that deliberately starts a
new process session is detached and outside that Terminal Session's cleanup
ownership. Natural child exit does not release ownership of retained Ordinary
Session Jobs; explicit Terminal Session cleanup still owes those jobs its
bounded shutdown policy.
Unreadable metadata of a proven foreign session does not block this cleanup;
unknown membership keeps cleanup pending within its existing deadline.
_Avoid_: descendant (ancestry alone does not establish current ownership)

**Terminal Generation**:
The identity shared by coherent views of one completed terminal-state change.
_Avoid_: frame number, render version

**Pane**:
One visible leaf in a tab's split layout that owns exactly one Terminal Session.
_Avoid_: split (the action or layout relationship), terminal

**Pane Focus**:
Whether a Pane is where the person is typing: it holds keyboard focus in its
Sprite Window and that window is the active one. Only a Pane with Pane Focus
may write the clipboard on its child's request, has its child told focus
arrived, and blinks its cursor; any other Pane's child is told focus left.
_Avoid_: active pane (ambiguous with the active tab), selected pane

**Pane Title**:
What a Pane reports it is called: for a terminal Pane, the title its child set
through the terminal, or else the name of the program in the foreground. Absent
rather than guessed when the Pane has been told nothing. Every Pane type reports
one through the pane interface.
_Avoid_: window title, tab title, label

**Tab Name**:
A name a person gave a tab, kept by the tab itself and shown in place of any
Pane Title until removed. Survives whatever the tab's Panes go on to run; is not
kept across a restart.
_Avoid_: tab title, label, override

**Divider**:
The boundary between the two sides of one split, addressed as the boundary on a
given side of a Pane rather than by an identity of its own. Moving a Divider
changes only how space is shared; it never creates, ends, reorders, or refocuses
a Pane, and never disturbs a Terminal Session.
_Avoid_: splitter, gutter, sash, handle, border

**Sprite Window**:
The top-level desktop window that owns tabs and Panes. It also owns pending
cleanup after a Pane leaves the layout; its last window stays alive until
removed and present Pane cleanup completes.
_Avoid_: workspace, session, terminal window

**Pane Observation**:
Protected, local, read-only access by a process launched inside Sprite to Pane
content in the same Sprite Window. It never grants control of a Pane or its
child.
_Avoid_: pane sharing, screen scraping, AI integration

**Pane Snapshot**:
An owned, text-focused view of a Pane's identity and active terminal content for
authorized observation and accessibility consumers. Its terminal text is
always untrusted data.
_Avoid_: transcript, terminal dump, screen capture

**Render Snapshot**:
An owned, immutable view of one Terminal Generation containing the rich visual
state Sprite needs to draw a Pane. It is internal to Sprite and is not the Pane
Observation format.
_Avoid_: Pane Snapshot, screen buffer, frame

**Observation Client**:
A local shell tool, initially `sprite panes snapshot`, that requests Pane
Snapshots from its Sprite Window.
_Avoid_: LLM client, agent, remote client

**Surface**:
Native UI that a program running in a Pane describes and Sprite draws inside
that Pane — at one position: fill, dock, or overlay — for as long as the
program keeps its Surface Channel connection open. Drawn by GPUI as elements,
never as terminal cells; takes the keyboard when it opens unless it declines,
and may hand it back.
_Avoid_: panel, widget, popup, view (unqualified), native pane

**Surface Channel**:
Protected, local access by a process launched inside Sprite to open and
update Surfaces in its own Pane and to receive their input and events. It is
the one line that grants control of what a Pane shows; Pane Observation grants
none, and the two never share a grammar.
_Avoid_: the observation socket, the socket (unqualified), IPC, the API

**Surface Description**:
The versioned document a program sends over the Surface Channel saying what a
Surface contains: element kinds, utility tokens for style, and token names
for colour. It is what Sprite draws; it is never code.
An update preserves the existing element, grid or virtual-list body kind.
Surface SVG decoding bounds final raster dimensions to 4096, each bitmap to
16MiB and retained decoded pixels to 64MiB per cache. These bounds do not
cover SVG parsing/filter intermediates or process-wide memory.
_Avoid_: markup, HTML, template, DSL, layout code

**Semantic Token**:
A named colour role — `terminal.background`, `ansi.4`, `scm.addedForeground`
— that a Surface Description or the terminal grid refers to instead of a
literal colour. Built into Sprite or registered by a program; each carries
one default and a description.
_Avoid_: colour variable, CSS variable, theme key, palette entry (for a
registered token)

**Token Registry**:
The single table of Semantic Tokens Sprite resolves at draw time: built-in
defaults, then program-registered defaults, then the active theme's overrides
by name. Session-scoped; the first registration of a name stands. One active
theme; no theme kinds.
_Avoid_: theme struct, colour map, stylesheet

**Surface Client**:
A local shell tool — `sprite surface` and `sprite token` — that opens and
drives Surfaces in its own Pane from the command line: events out as JSON
lines, updates in from standard input. The reference implementation of the
Surface Channel; other clients may speak the socket directly.
_Avoid_: the CLI (unqualified), sprite.nvim, the adapter

**Croft Compatibility Smoke Test**:
Unmodified upstream Croft used as an optional, manually invoked check against
its moving `main`; it is not part of Sprite Terminal or a required CI gate.
_Avoid_: Croft dependency, Croft fork, required Croft gate
