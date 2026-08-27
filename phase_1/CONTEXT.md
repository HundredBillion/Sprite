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

**Croft Compatibility Gate**:
Unmodified upstream Croft used as an external acceptance application against
its moving `main`; it is not part of Sprite Terminal.
_Avoid_: Croft dependency, Croft fork
