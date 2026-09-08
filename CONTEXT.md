# Sprite Project

Project-wide vocabulary for the Sprite effort and its products. The terminal
core's own vocabulary lives in `crates/CONTEXT.md`; see `CONTEXT-MAP.md`.

## Language

**Sprite**:
The project as a whole — the repository, the effort, the product, and the
editor-side repositories such as `sprite.nvim`. Unqualified "Sprite" names the
project, not a binary;
say "Sprite Terminal" when the product is meant.
_Avoid_: Sprite (as a product name), the app

**Sprite Terminal**:
The standalone terminal product: the `sprite-app` crate compiled into
`/usr/bin/sprite`. The only product. Independently useful, installable, and
versioned; no editor is ever a dependency of it. It defines the pane
interface, hosts every pane and every Surface, and is the designated home of future
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
input, and session state. Every pane is a terminal pane; a program running in
it may describe a Surface that Sprite draws inside it, and an editor earns no
special architectural status by being one such program.
_Avoid_: window, tab, panel, view (unqualified), editor pane

**`sprite-pane`**:
The crate holding the pane interface — the trait Sprite Terminal's own
`TerminalView` implements, and the contract a hosted Surface presents to the
pane that draws it. Depends on `gpui` and `sprite-term` types and nothing
else.
_Avoid_: the editor trait, the plugin API, the extension interface, the
socket editor panes plug into

**Dependency invariant**:
The rule that replaced the old product boundary: Sprite's `Cargo.toml` never
names a concrete editor. Editor-side repositories speak Sprite's Surface
Channel and never link into it; Sprite depends on no editor. Mechanically
checkable, unlike the prose rule it replaces.
_Avoid_: the product boundary (post-2026-09-07), the firewall

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
The unmodified `neovim/neovim` editor, under Apache-2.0. A first-class
terminal program in Sprite Terminal, and the first program to drive a
Surface: `sprite.nvim`'s adapter attaches to it through `nvim_ui_attach` and
forwards its screen as a grid Surface. Never forked, never linked; Sprite
Terminal never speaks its protocol.
_Avoid_: the Neovim fork, the Neovim pane, NeovimPanel, embedded Neovim

**`sprite.nvim`**:
The Neovim-side repository: the adapter that attaches to Neovim's UI protocol
and drives a grid Surface, the Lua API plugins call to describe Surfaces and
register Semantic Tokens, and the `nvim` launcher that fails open to text
mode. Runs beside Neovim, outside Sprite Terminal's process, speaking only
the Surface Channel; Sprite Terminal never names it.
_Avoid_: the Neovim pane, the Neovim plugin (unqualified), embedded Neovim,
the distro (for the repository)

## Superseded

Terms retired on 2026-09-07 by `docs/PRDs/09-07-2026-native-surfaces.md`,
kept so older documents still read.

**Editor Pane** *(retired 2026-09-07)*:
Formerly: an implementation of `sprite-pane`'s trait providing an editor, in
its own repository, GPUI-native with no TUI mode. Dissolved — an editor is a
program that runs in a terminal pane and may drive a Surface; see Surface in
`crates/CONTEXT.md`.

**Composed build** *(retired 2026-09-07)*:
Formerly: a Sprite Terminal binary produced by a distribution crate with
editor panes linked in. Dissolved — nothing links into Sprite Terminal and
there is no distribution crate; editors are clients of the Surface Channel.
