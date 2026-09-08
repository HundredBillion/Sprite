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
_Avoid_: shell (only one possible child), terminal instance

**Terminal Generation**:
The identity shared by coherent views of one completed terminal-state change.
_Avoid_: frame number, render version

**Pane**:
One visible leaf in a tab's split layout that owns exactly one Terminal Session.
_Avoid_: split (the action or layout relationship), terminal

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
The top-level desktop window that owns tabs and Panes.
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
by name. One active theme; no theme kinds.
_Avoid_: theme struct, colour map, stylesheet

**Croft Compatibility Gate**:
Unmodified upstream Croft used as an external acceptance application against
its moving `main`; it is not part of Sprite Terminal.
_Avoid_: Croft dependency, Croft fork
