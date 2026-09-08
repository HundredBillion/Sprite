# Native Surfaces

**Date:** 2026-09-07
**Type:** Architecture (a generic capability of Sprite Terminal)
**Target:** `crates/sprite-app` (a new `surface` module beside `observation/`,
`terminal_view.rs`, `grid_paint.rs`, `config.rs`), `crates/CONTEXT.md`,
`terminal-project-brief.md`,
`docs/PRDs/09-07-2026-pane-trait-and-editor-plurality.md` (status note)
**Status:** Designed 2026-09-07 (brainstorming session). Supersedes
`09-07-2026-program-takeover-of-terminal-panes.md`, which is removed in the
same commit, and supersedes the "Terminal pane / Editor pane" framing of the
plurality PRD for editors. That PRD's dependency invariant — *Sprite's
manifest names no editor* — is preserved unchanged and is made true by
construction here.

## Summary

Sprite gains one capability: **a program running in a pane can describe user
interface, and Sprite draws it natively.** A docked list. A tree with
full-colour SVG icons. A whole text grid. The description travels over a
second authenticated socket beside the one every child process already knows —
the Surface Channel — and Sprite renders it with GPUI, styled by name against a
registry of semantic tokens that programs can extend and themes can override.

The capability knows nothing about editors. Neovim is simply its first user,
and it uses it **entirely from its own side of the wire**: Neovim's UI protocol
already emits a complete, structured description of the editing screen on every
keystroke — every line, every highlight group, every window's position, the
cursor — so a small adapter beside Neovim forwards that stream as a grid
surface, and plugins such as `svgtree.nvim` and `scm.nvim` send Surfaces of their own. All of that code lives in a Neovim repository. Sprite never learns the
name `nvim`. Nothing links into Sprite. Nothing needs bundling. There are two
repositories — Sprite, and `sprite.nvim` — plus the plugins.

This document exists because two earlier designs for the same goal were wrong
in instructive ways. The roadmap planned editors as a second *kind of pane*,
chosen when a pane opens. The takeover PRD this one replaces kept one pane
type but still assumed a native renderer had to be **Rust linked into Sprite**,
which forced a separate renderer repository, a bundling step, a PATH-shadowing
helper, and a live edge case around shell aliases. The insight that dissolves
all of that: once the boundary is *a description over a socket*, the code that
understands an editor never touches GPUI, so it does not need to be in
Sprite's process at all. The renderer becomes generic; the editor-specific
part becomes a client.

Two mature codebases were read for this design and are cited throughout for
context: **Zed** (`~/Projects/zed`), whose GPUI is the framework Sprite draws
with, and **VS Code** (`~/Projects/vscode`), the reference for how a large
editor themes an extensible UI. Neither is copied wholesale. Zed supplies the
rendering substrate and the shape of highlight styling; VS Code supplies the
one idea Zed lacks that Sprite needs — an extensible token registry — and two
ideas Sprite deliberately declines.

## User outcome

- A person opens a Sprite pane and types `nvim .`, exactly as today. The
  editing area is drawn natively — real font rendering, stylesheet-controlled
  line height, gutter, padding, colours by highlight group. Quitting returns
  the shell, same directory, `$?` intact.
- `svgtree.nvim` shows a real file tree beside the editor: proportional text,
  full-colour vector icons, hover, smooth scrolling. `scm.nvim` shows a real source-control Surface docked beside the editor. Clicking a file in either opens it in Neovim. Neither
  plugin contains a line of Rust.
- Any program — a shell script, `htop`, an agent's CLI — can put up a Surface in its pane the same way, because nothing in the mechanism is
  Neovim's. A terminal built for agents gains a way for agents to show
  structured, styled output.
- Every terminal program, unmodified, gets **Level 0** styling for free: the
  existing text grid draws with the theme's font, spacing, and token-remapped
  colours. Helix and Croft need no fork to look right.
- Plain `sprite` is unchanged until a program creates a surface. If a program
  cannot reach the socket, is refused, or speaks a protocol version Sprite
  does not know, it runs in text mode as it does today. The fallback is what
  happens when nothing native answers, not a mode to configure.

## Decisions, and why

**One generic capability, not a renderer per editor.** The takeover PRD needed
`sprite-nvim-pane` (a Rust renderer), a distribution crate to bundle it, an
`nvim` helper on PATH, and a fix for shell aliases that bypass PATH — all
because it assumed the renderer had to link into Sprite. A description
boundary removes the assumption: the program *sends* what to draw, Sprite
draws it. The editor-specific code (speaking Neovim's protocol) sends
descriptions and never calls GPUI, so it runs as its own process beside
Neovim. Sprite's manifest names no editor; there is nothing to bundle;
plugins stay in Lua. This is the same conclusion Zed reached for its own
extensions — WASM extensions get no rendering access, because in-process
rendering across an ABI is unsafe — arrived at from the other direction:
instead of denying extensions rendering, Sprite gives them a *language* for
it.

**The description is CSS-shaped, not CSS-syntax.** Two reference points fix
this. **Zed** draws everything through GPUI's `Styled` trait — 116 utility
methods (`crates/gpui/src/styled.rs`) that mirror Tailwind's classes one for
one: `.flex()`, `.flex_col()`, `.gap_3()`, `.p_2()`, `.rounded_md()`,
`.border_1()`, `.bg(..)`, `.text_xl()` — laid out by **Taffy 0.13**
(`crates/gpui/src/taffy.rs`), a Rust implementation of the CSS flexbox and
grid algorithms. Zero files in Zed parse a stylesheet. **VS Code** is the
opposite: 458 real `.css` files under `src/vs`, a DOM, and the browser's
cascade. Sprite's substrate is GPUI, so an element in a description carries
**utility tokens** — `"style": "flex flex-col gap-2 p-3 rounded-md"` — that
map onto `Styled` methods that already exist. Sprite invents no layout or
style engine; it translates tokens into calls. Real CSS text with selectors
and a cascade was rejected: GPUI has none of it, VS Code gets it from the
browser for free, and Sprite would be building a CSS engine to obtain a
*syntax* whose real value (below) is available without one.

**An extensible semantic token registry, referenced by name.** This is the
idea taken from VS Code, because Zed cannot express it. In VS Code a colour
is *registered*: `registerColor('editor.background', { light: '#ffffff',
dark: '#1E1E1E', hcDark: Color.black, hcLight: Color.white }, "Editor
background color.")` (`src/vs/platform/theme/common/colors/editorColors.ts:19`;
the function at `colorUtils.ts:252`). Themes override registered tokens, and
— the property Sprite needs — **extensions register their own** through a
`colors` extension point (`src/vs/workbench/services/themes/common/colorExtensionPoint.ts:28`).
Registered tokens reach stylesheets as CSS variables named
`--vscode-editor-background` (`colorUtils.ts:35`), so a stylesheet references
a *name* and a theme swaps the *value*. Zed's theme is a JSON file filling a
fixed `ThemeColors` struct; a plugin cannot add a field. Sprite's plugins
must: `scm.nvim` needs `scm.addedForeground`, `svgtree.nvim` needs icon
tokens. So Sprite keeps a registry: a program registers a token over the
Surface Channel with **one default** and a description; the theme in
`config.rs` overrides by name; descriptions reference tokens by name and
Sprite resolves them at draw time, so changing the theme restyles every
surface at once. VS Code's per-kind defaults (`light`, `dark`, `hcDark`,
`hcLight`) are deliberately not copied: Sprite has one active theme and no
theme kinds (`config.rs`'s `Colors` is a single `background`, `foreground`,
`cursor`, and `palette`), and inventing kinds is a feature no plugin asked
for. The light/dark toggle the project owner wants later is a feature that
selects *which theme is active*; it sits above the registry and changes
nothing about how a token is registered, since a per-kind default is a
superset of a single one. Resolution precedence, in one place: for the
terminal grid, a program's own OSC colours → the theme's override → the
program-registered default → the built-in default → libghostty's colour
(the first step is the rule `config.rs:100–105` already keeps); for a
surface, the same without the OSC step. A literal colour is accepted as an
escape hatch and is discouraged in the documentation.

**A first-class grid widget, styled by a flat highlight map.** The editing
area is not a thousand tiny elements; it is one **grid** widget, updated
incrementally, drawn by a generalisation of the painter Sprite already has for
its terminal (`grid_paint.rs`, which paints cells without a layout pass for
exactly this reason). Its cells carry a highlight id, and the theme maps
highlight groups to style with a **flat** map in Zed's shape —
`"Comment": { "color": .., "font_style": "italic", "font_weight": .. }` —
which is how Zed's own themes style syntax (`assets/themes/one/one.json`,
`"syntax"` block). VS Code instead uses TextMate **scope selectors**
(`"tokenColors"` in `extensions/theme-defaults/themes/dark_vs.json`) with a
second semantic-token registry layered on top
(`tokenClassificationRegistry.ts`). Sprite does not need selectors: Neovim's
UI protocol delivers *already-resolved* attributes per highlight id
(`hl_attr_define`) together with group names (`hl_group_set`), having applied
`hi link` hierarchy itself. A selector language would solve a problem the
protocol has already solved.

**Layout: the pane tree stays geometry; flex lives inside a surface.** VS Code
lays out its workbench with a programmatic, pixel-computed `Grid` and
`SplitView` (`src/vs/base/browser/ui/grid/grid.ts:222`,
`splitview/splitview.ts:427`) and uses CSS only *inside* those regions.
Sprite already has that shape: `pane_tree.rs` is pure geometry (`PaneId`,
`Orientation`, `Divider`, `Rect`). A surface therefore takes one of three
positions *within a pane*: **fill** (it replaces the grid view — the editing
area), **dock** (it takes a strip beside the terminal, which is resized and
told so through the PTY, as on any window resize), or **overlay** (it floats
above). Inside the surface, Taffy flex does the work through GPUI, as in
Zed. Nothing about tabs, splits, or dividers changes. How many, and where:
at most **one fill**; at most **one dock per side**, a dock's `open` naming
its side — **left or right** in this project, top and bottom deferred until a
plugin wants one; **overlays stack**, last opened on top. A second `open` for
an occupied fill or dock side is refused with its own reason, `position
occupied`, so the program can take the other side or fall back. A dock is an
internal split of the pane's own rectangle, not a tree Divider, so it never
creates, ends, or reorders a pane. Unlimited docks sharing a side were
rejected as a layout engine inside the strip; one Surface per pane was
rejected because `svgtree.nvim` and `scm.nvim` are both docks and must
coexist.

**Transport: a second endpoint beside observation — the Surface Channel.**
Every child already receives `SPRITE_PANE` (`observation/endpoint.rs:308`),
`SPRITE_OBSERVATION_SOCKET`, and `SPRITE_OBSERVATION_KEY`, so the
authentication and the pane identity a surface needs already exist. What must
not be reused is the observation *grammar*. `observation/request.rs` is
explicit that it is read-only by construction — "`broker` promises that a
request which could mutate cannot be constructed" (lines 12–15), and "there is
deliberately no variant that writes, sends input, subscribes, or opens a
stream" (lines 25–29) — and the Terminal Core glossary defines Pane
Observation as access that "never grants control of a Pane or its child."
That promise is what makes it safe for any program, and for any LLM reading a
pane through it, to hold that socket; a surface is nothing but control. The
grammar is also the wrong shape: a request "crosses two processes as a line
of text" of space-separated words, while a description is a structured
document flowing both ways. So surfaces get a **second `Endpoint`** with its
own socket file, named to children as `SPRITE_SURFACE_SOCKET`, sharing the
key, the runtime directory, and the authentication code, and speaking
newline-delimited JSON. One connection per surface stays open for the
surface's life: the program streams incremental updates down it and receives
input and events (a click, a key while the surface has focus, a resize) up
it; closing the connection — or the program dying — removes the surface. The
observation endpoint's short request timeout does not apply, and the line
buffer is not capped small, a lesson Zed records in its own agent transport
(`crates/agent_servers/src/acp.rs:674`: "512 KiB is not enough").

Two mature LLM integrations keep exactly this separation. Zed labels every
agent tool `ToolKind::Read` or `ToolKind::Edit` and gates only edits
(`crates/agent/src/tools/read_file_tool.rs:215`, `edit_file_tool.rs:235`;
`acp_thread/src/connection.rs:538`). VS Code's Agent Host routes every tool
call through a `CanUseTool` gate and a resource-scoped
`AgentHostPermissionMode` (`src/vs/platform/agentHost/common/agentHostResourceService.ts:47`).
Reading is ungated; acting is where gating lives. A second endpoint puts
Sprite's one read line beyond gating altogether — control is *absent* from
it, not merely denied — and puts any future permission model where acting
is. The cost is one file descriptor and one thread asleep in `accept()`
(`endpoint.rs:267`); per message the two options share the same kernel path,
and the second line keeps a keystroke-rate grid stream off the listener an
LLM reads through. Two alternatives were rejected: a private OSC sequence
through the PTY, as before (`sprite-term` surfaces only OSCs 7, 10, 11, 12,
4, 52, 133 and has no passthrough); and surface verbs on the observation
socket distinguished by protocol token, because that makes the read-only
promise true of *some lines on a socket* rather than of the socket.

**Focus: a Surface takes the keyboard when it opens.** A person opens a dock
to use it — `neo-tree` and `nvim-tree` already work this way: toggle the tree,
focus lands in it, `j`/`k`/Enter — so a Surface receives focus on `open` by
default at every position. Fill always holds it, there being nothing else to
focus; an overlay takes it on open and **returns it to the previous holder on
close**. Two rules keep the default from misfiring. First, focus changes only
on `open`, never on `update`: a dock refreshing itself steals nothing.
Second, `open` carries `focus: bool`, default `true`, so a program pushing a
Surface the person did not ask for — an agent's status view arriving
mid-word — passes `focus: false`. The channel also carries a **`focus`
request** in the other direction, by which a program hands the keyboard back
to the terminal or to another of its Surfaces; that is how a tree returns
you to the buffer after a pick, and it is the plugin's decision, as in
`neo-tree`. Sprite sends `focus` and `blur` events so a Surface can draw its
focused state. Clicking focuses what was clicked, as GPUI already does
(`TerminalView` owns one `FocusHandle`, `terminal_view.rs:74`, and a hosted
Surface brings its own through `PaneHandle::focus_handle()`). One workspace
keybinding cycles focus terminal → Surfaces → terminal, a safety net for a
program that forgets to hand it back. Two alternatives were rejected: every
Surface declaring intent with no default makes the common case verbose, and
Surfaces never taking the keyboard kills keyboard navigation of a file tree —
the very thing a Neovim user does.

**A small, versioned schema, grown only under demand.** The single largest
risk in this design is scope: a description language for UI is a browser
engine if nobody says no. VS Code's answer to "extensions want arbitrary UI"
is the **webview** (`src/vs/workbench/contrib/webview/browser/`, a sandboxed
iframe running `pre/index.html`) — an actual embedded browser, which VS Code
can afford because it *is* one. Sprite declines that route. The schema is
enumerated: a handful of element kinds (box, text, list, tree, image/SVG,
button, grid), the utility tokens the interpreter maps, the token registry
verbs, and the events. It carries a protocol version of its
own, and an unknown version is refused the way the observation grammar refuses
one. A kind or token is added when a real plugin
needs it and not before.

**Two levels, delivered in order, the cheap one first.** Sprite *already
holds* a structured grid of every terminal program's screen — that is what
the terminal is. **Level 0** applies the theme to that existing grid: font,
line height, cell padding, and colour remapping by token, for every program,
with no protocol and no adapter. Its colour tokens already exist: `Colors`'s
`background`, `foreground`, `cursor`, and sixteen `palette` slots
(`config.rs:107–115`) become the built-in tokens `terminal.background`,
`terminal.foreground`, `terminal.cursor`, and `ansi.0`–`ansi.15`, and the
existing `[colors]` TOML keys keep working as their overrides; what Level 0
adds is line height and cell padding. It is a small change inside
`grid_paint.rs` and `config.rs` and it makes Neovim, Helix, Croft, `htop`,
and everything else look like the theme immediately. **Level 1** is semantic: a program
streams a grid surface with highlight ids, and styling follows highlight
groups. Level 0 ships first because it is nearly free and because it is the
right fallback when Level 1 is absent.

**The Neovim side lives in `sprite.nvim`, out of process.** `nvim_ui_attach`
is available only over RPC, so the adapter is a process: it runs
`nvim --embed`, attaches as Neovim's UI, translates the redraw stream
(`grid_line`, `win_pos`, `hl_attr_define`, cursor and mode events) into grid
surface updates, and translates Sprite's input events back into
`nvim_input`. This is what Neovide's core does; it is well-trodden.
`sprite.nvim` also holds the Lua API plugins call to describe Surfaces and the
`nvim` launcher that starts the adapter when a person types `nvim .`. That
launcher inherits the takeover PRD's fail-open rule — no socket, refusal, or
unknown protocol means `exec` the real Neovim in text mode — and its PATH and
alias concerns, which are now the Neovim distribution's to solve rather than
Sprite's. The "Sprite-friendly Neovim distribution" the project owner
intends is `sprite.nvim` plus a configuration.

**Sprite keeps one small hook the launcher wants.** `shell.rs` already exports
`SPRITE_SHELL_INTEGRATION_DIR` to children and notes it is "not yet injected"
into PATH. Closing that gap — prepending the directory when it names one
present — lets a distribution place its launcher where the shell inside
Sprite finds it first, on every platform, without touching a system `bin`.
It is generic, a few lines, and stays optional: plain `sprite` sets no
directory and nothing changes.

**The brief and the plurality PRD are amended.** One paragraph in
`terminal-project-brief.md` records the supersession: one pane type; programs
describe UI and Sprite draws it; the pane interface from PR #27 is the
contract a *hosted surface* satisfies, not a pane type; editors integrate as
clients of the surface protocol from their own repositories. The plurality
PRD gets a status line pointing here. The dependency invariant and the three
editor repositories are restated, not removed — with the note that a Helix or
Croft *fork* is no longer required for them to look native; Level 0 covers
appearance, and a Level 1 adapter is theirs to write if they want semantic
styling or Surfaces.

## What ends up where

### In Sprite (`crates/sprite-app`)

- **`surface/` module (new):** the interpreter. Parses a versioned
  description; resolves tokens through the registry; maps utility tokens to
  `gpui::Styled` calls; builds GPUI elements for each kind; owns the grid
  widget, which shares its cell painter with `grid_paint.rs`.
- **Token registry (new, in `config.rs` alongside `Settings`):** built-in
  tokens — the existing `Colors` fields under token names — each with one
  default and a description; `register` from programs; theme overrides by
  name; `resolve(name) -> Rgba` with a documented fallback for an unknown
  token. One active theme; no theme kinds.
- **Surface Channel (`surface/channel.rs`, new):** a second `Endpoint`, its
  socket path exported to children as `SPRITE_SURFACE_SOCKET`, sharing the
  observation key, runtime directory, and authentication code. Its own
  newline-delimited-JSON grammar: `open { pane, position: fill|dock|overlay,
  side, focus, version }`, `update`, `close`, `focus`, `token register`, and the
  event direction (`input`, `resize`, `event`, `focus`, `blur`) on the same
  long-lived connection. Refusals
  are distinct: unknown pane, pane not a terminal, unsupported version,
  malformed description, unknown element kind or token, position occupied.
  **`observation/` is
  not modified**; its read-only-by-construction grammar and tests are
  untouched.
- **`TerminalView` (`terminal_view.rs`):** hosts surfaces by position — at
  most one fill, one dock per side (left or right), and stacked overlays.
  *Fill* renders the surface instead of the grid and forwards
  focus, size, and input. *Dock* splits the pane's rectangle, renders both,
  and resizes the PTY. *Overlay* paints above. A surface takes focus when it
  opens unless its `open` said `focus: false`; a `focus` request moves it; an
  overlay returns focus to the previous holder when it closes; one workspace
  keybinding cycles focus. When a surface's connection closes, its space
  returns to the grid. The `sprite-pane` interface from
  PR #27 is what a hosted surface presents to the pane.
- **Level 0 (`grid_paint.rs`, `config.rs`):** theme-driven font, line height,
  cell padding, and token-remapped colours applied to the existing terminal
  grid.
- **`shell.rs` (`crates/sprite-term`):** prepend `SPRITE_SHELL_INTEGRATION_DIR`
  to the child's PATH when it names a present directory.
- **Docs:** the two amendments above.

### Outside Sprite (later projects, named so the seam is visible)

~~~
sprite.nvim/                      separate repository — Neovim's side of the wire
  adapter/                        runs `nvim --embed`, nvim_ui_attach, redraw -> grid surface
  lua/sprite/                     the API plugins call to describe Surfaces and register tokens
  bin/nvim                        the launcher: fails open to text-mode Neovim
svgtree.nvim, scm.nvim            send Surface Descriptions; register their tokens; pure Lua
~~~

## Verification

The capability is proven **without Neovim**, on purpose: a shell script is
the first client, so nothing about Project 1 depends on a repository that does
not exist yet, and program-agnosticism is demonstrated rather than claimed.

1. **Unit, in Sprite.** The parser accepts each element kind and rejects an
   unknown kind, an unknown utility token, and an unsupported version with
   distinct reasons. Every supported utility token maps to a `Styled` call
   (table-driven; the table *is* the supported vocabulary). The registry returns a built-in default, a program-registered default, a theme
   override in that order of precedence, and the documented fallback for an
   unknown name; for the grid, a program's own OSC colour wins over all of
   them while it runs. The grid widget applies an incremental line update without
   repainting untouched rows. `TerminalView` hosts a fill, a dock (with the
   PTY told its new size), and an overlay, and returns the space when the
   connection closes. A surface opened with the default takes focus; one
   opened with `focus: false` leaves it where it was; `update` never moves
   focus; a `focus` request moves it; an overlay closing returns focus to
   the previous holder; `focus` and `blur` events are delivered. Two docks on
   different sides coexist; a second dock on an occupied side, or a second
   fill, is refused as `position occupied`; overlays stack in open order. Level 0 changes a cell's drawn colour when the theme
   remaps its token. `shell.rs` prepends a present integration directory and
   ignores an absent one. The observation grammar's own tests pass without
   change, and `observation/request.rs` still constructs no mutating variant.
2. **Invariant, in Sprite.** The `sprite-pane` manifest test still finds
   `gpui` alone; no Sprite `Cargo.toml` names an editor. The full suite,
   `fmt`, `clippy -D warnings`, and the `--locked --offline` build pass.
   Plain `sprite` with no surface open behaves as before, byte for byte.
3. **End to end, by hand, with a script.** From a shell in a Sprite pane, a
   script opens a *dock* surface describing a docked Surface: a title, a list of
   three rows each with an SVG icon and a label, styled with utility tokens
   and colours referenced by token name. The Surface appears beside the
   terminal; `tput cols` in the shell reports the narrower width. The Surface has
   focus the moment it opens. Clicking a row delivers an `event` to the
   script, which prints it; the script then sends `focus` and the next
   keystroke reaches the shell. Editing the
   theme's value for a token the Surface uses restyles the Surface live. The
   script exits; the Surface disappears and `tput cols` is restored. A second
   script opens a *fill* surface streaming a small grid with two highlight
   groups; the theme's flat highlight map colours them; closing it returns
   the shell.
4. **End to end, by hand, Level 0.** With `nvim` and then `htop` running in
   text mode, changing the theme's font size and one token colour restyles
   the live grid without restarting either program.

Steps 3 and 4 are written into the TSP as checkbox steps; automating them is
named below as absent, not deferred by accident.

## Out of scope

- **The Neovim adapter, the Lua API, and the launcher** — `sprite.nvim`, its
  own repository and PRD. This document gives it a protocol to speak.
- **`svgtree.nvim` and `scm.nvim` changes** — they follow the API.
- **The Neovim distribution** — `sprite.nvim` plus configuration; roadmap 2.3
  in spirit, no distribution *crate* required any more.
- **A CSS cascade or selector engine**, and **TextMate-style scope selectors**
  — declined above, with reasons.
- **Webviews or arbitrary HTML** — declined above; the enumerated schema is
  the design.
- **Inline widgets inside a Neovim buffer** at a specific text position.
  They need window geometry only the Level 1 adapter knows; a *fill* surface
  driven by the adapter can position them later without a Sprite change.
- **Helix and Croft forks.** No longer required for appearance. An adapter
  for either is that project's choice.
- **A light/dark theme toggle.** Wanted later; it selects which theme is
  active and sits above the registry, changing no registration.
- **Automating the by-hand verification.**
