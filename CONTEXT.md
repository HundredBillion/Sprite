# Sprite Project

Project-wide vocabulary for the Sprite effort and its products. The terminal
core's own vocabulary lives in `crates/CONTEXT.md`; see `CONTEXT-MAP.md`.

## Language

**Sprite**:
The project as a whole — the repository, the effort, and the product and
editor panes below. Unqualified "Sprite" names the project, not a binary;
say "Sprite Terminal" when the product is meant.
_Avoid_: Sprite (as a product name), the app

**Sprite Terminal**:
The standalone terminal product: the `sprite-app` crate compiled into
`/usr/bin/sprite`. The only product. Independently useful, installable, and
versioned; no editor is ever a dependency of it. It defines the pane
interface, hosts every pane type, and is the designated home of future
workspace features.
_Avoid_: sprite-term, the terminal library, Studio

**`sprite-term`**:
The terminal engine library: PTY, child lifecycle, libghostty state, and
owned snapshots. A library only — it has no window and is never standalone.
The 2026-09-05 plan to rename it `sprite-engine` was reverted on 2026-09-07
(Addendum A.16); a crate name names its domain, not its rank in the
dependency graph.
_Avoid_: sprite-engine, terminal backend, the terminal

**Pane**:
The primitive. A rectangle in the tree that owns its content, focus, title,
input, and session state. Terminal panes are the default and majority case;
an editor pane is one optional kind among them and earns no special
architectural status by being present.
_Avoid_: window, tab, panel, view (unqualified)

**`sprite-pane`**:
The crate holding the pane interface — the trait every pane implements,
including Sprite Terminal's own `TerminalView`. Depends on `gpui` and
`sprite-term` types and nothing else. The socket that editor panes plug into.
_Avoid_: the editor trait, the plugin API, the extension interface

**Editor Pane**:
Any implementation of `sprite-pane`'s trait that provides an editor, living
in its own repository and never a dependency of Sprite Terminal. Three are
scheduled: the Neovim pane, the Helix fork, and the Croft fork. Each is
GPUI-native, maintains no TUI mode, and chooses its own product name at its
Phase 2.6 branding step.
_Avoid_: the fork (unqualified), the IDE, the plugin

**Dependency invariant**:
The rule that replaced the old product boundary: Sprite's `Cargo.toml` never
names a concrete editor. Editor repositories depend on Sprite; Sprite depends
on no editor. Mechanically checkable, unlike the prose rule it replaces.
_Avoid_: the product boundary (post-2026-09-07), the firewall

**Composed build**:
A Sprite Terminal binary produced by the distribution crate with one or more
editor panes linked in. The default `sprite` build carries terminal panes
only. Composition is at build time because GPUI rendering requires the same
process, and Rust has no stable ABI for runtime plugins (Addendum A.16).
_Avoid_: plugin, extension, add-on

**Pane-first**:
The identity that distinguishes Sprite Terminal from editor-first products
(VS Code, Zed): the workspace is a collection of panes, terminals are the
default pane type, and an editor earns no special architectural status by
being present.
_Avoid_: terminal-first (when describing the UI), editor with a terminal

**Croft (upstream)**:
The unmodified `vitali87/croft` TUI workspace, under MIT. Two roles:
acceptance application for Sprite Terminal, and the model source for the
Croft fork pane. Forked last of the three, being the hardest surgery.
_Avoid_: our Croft, the editor, the fork

**Helix (upstream)**:
The unmodified `helix-editor/helix` editor, under MPL-2.0. Model source for
the Helix fork pane; `helix-core` and `helix-view` are already separated from
the TUI in `helix-term`, so the GPUI frontend replaces a component against an
existing seam. License obligations differ from Croft's and are reviewed
before the fork begins.
_Avoid_: our Helix, the Helix panel

**Neovim (upstream)**:
The unmodified `neovim/neovim` editor, under Apache-2.0. Two roles: a
first-class terminal program, and the model behind the Neovim pane. Never
forked or linked — it runs as a child process and the pane speaks msgpack-RPC
to it through `nvim_ui_attach`. The first external implementation of the pane
interface.
_Avoid_: the Neovim fork, NeovimPanel, embedded Neovim
