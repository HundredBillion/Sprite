# Sprite Project

Project-wide vocabulary for the Sprite effort and its products. The terminal
core's own vocabulary lives in `phase_1/CONTEXT.md`; see `CONTEXT-MAP.md`.

## Language

**Sprite**:
The project as a whole — the repository, the effort, and the family of
products below. Unqualified "Sprite" never names a single product in
documents written after 2026-09-05; earlier phase_1 documents use "Sprite"
to mean Sprite Terminal and are grandfathered.
_Avoid_: Sprite (as a product name), the app

**Sprite Terminal**:
The standalone terminal product: the `sprite-app` crate compiled into
`/usr/bin/sprite`. Independently useful, installable, and versioned; no IDE
or editor is ever a dependency of it.
_Avoid_: sprite-term, the terminal library

**Sprite Engine**:
The terminal engine library (crate `sprite-engine`, named `sprite-term`
until the rename lands): PTY, child lifecycle, libghostty state, and owned
snapshots. A library only — it has no window and is never standalone.
_Avoid_: sprite-term (post-rename), terminal backend, the terminal

**Studio**:
The pane-first workspace product (crate working name `sprite-studio`). The
pane is the primitive; terminal panes are the default and majority case; the
editor is one optional pane type. Designated home of future workspace
features. Ships separately from Sprite Terminal.
_Avoid_: the IDE, Sprite (unqualified), third crate

**The Fork**:
The GPUI-native editor built from Croft's model with its ratatui view layer
replaced by GPUI. GPUI-only — it maintains no TUI mode. Hosted as an editor
pane in Studio; never linked into Sprite Terminal. Product name chosen at
Phase 2.6 branding.
_Avoid_: Croft (unqualified), Croft fork TUI, headless Croft

**Croft (upstream)**:
The unmodified `vitali87/croft` TUI workspace. Two roles: acceptance
application for Sprite Terminal, and the source of the Fork's model.
_Avoid_: our Croft, the editor

**Pane-first**:
The identity that distinguishes Studio from editor-first products (VS Code,
Zed): the workspace is a collection of panes, terminals are the default pane
type, and an editor earns no special architectural status by being present.
_Avoid_: terminal-first (when describing Studio's UI), editor with a terminal
