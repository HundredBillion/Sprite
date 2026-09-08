# Native Surfaces

**Date:** 2026-09-07
**Type:** Architecture (a generic capability of Sprite Terminal)
**Target:** `crates/sprite-app` (`observation/`, a new `surface` module,
`terminal_view.rs`, `grid_paint.rs`, `config.rs`), `terminal-project-brief.md`,
`docs/PRDs/09-07-2026-pane-trait-and-editor-plurality.md` (status note)
**Status:** Designed 2026-09-07 (brainstorming session). Supersedes
`09-07-2026-program-takeover-of-terminal-panes.md`, which is removed in the
same commit, and supersedes the "Terminal pane / Editor pane" framing of the
plurality PRD for editors. That PRD's dependency invariant — *Sprite's
manifest names no editor* — is preserved unchanged and is made true by
construction here.

## Summary

Sprite gains one capability: **a program running in a pane can describe user
interface, and Sprite draws it natively.** A panel with a list. A tree with
full-colour SVG icons. A whole text grid. The description travels over the
authenticated socket every child process already knows how to reach, and Sprite
renders it with GPUI, styled by name against a registry of semantic tokens that
programs can extend and themes can override.

The capability knows nothing about editors. Neovim is simply its first user,
and it uses it **entirely from its own side of the wire**: Neovim's UI protocol
already emits a complete, structured description of the editing screen on every
keystroke — every line, every highlight group, every window's position, the
cursor — so a small adapter beside Neovim forwards that stream as a grid
surface, and plugins such as `svgtree.nvim` and `scm.nvim` send panels of their
own. All of that code lives in a Neovim repository. Sprite never learns the
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
  full-colour vector icons, hover, smooth scrolling. `scm.nvim` shows a real
  source-control panel. Clicking a file in either opens it in Neovim. Neither
  plugin contains a line of Rust.
- Any program — a shell script, `htop`, an agent's CLI — can put up a native
  panel in its pane the same way, because nothing in the mechanism is
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
socket with per-theme-kind defaults and a description; the theme in
`config.rs` overrides by name; descriptions reference tokens by name and
Sprite resolves them at draw time, so changing the theme restyles every
surface at once. A literal colour is accepted as an escape hatch and is
discouraged in the documentation.

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
Zed. Nothing about tabs, splits, or dividers changes.

**Transport: the observation socket, one long-lived connection per surface.**
Every child already receives `SPRITE_PANE` (`observation/endpoint.rs:308`),
`SPRITE_OBSERVATION_SOCKET`, and `SPRITE_OBSERVATION_KEY`, and the `Workspace`
already serves authenticated requests on that socket (`workspace.rs:608`;
`sprite panes snapshot` and `sprite config reload` use it). Surfaces add verbs
beside them. A `surface` connection stays open: the program streams
incremental updates (a changed grid line, a re-rendered panel) and receives
input and events (a click, a key while the surface has focus, a resize) on
the same connection, then closes it to remove the surface. The endpoint's
short request timeout does not apply to surface connections. A private OSC
sequence through the PTY was rejected, as before: `sprite-term` surfaces only
specific OSCs (7, 10, 11, 12, 4, 52, 133) and has no passthrough, and the
socket is already authenticated so a stray program on the machine cannot
draw into someone's pane.

**A small, versioned schema, grown only under demand.** The single largest
risk in this design is scope: a description language for UI is a browser
engine if nobody says no. VS Code's answer to "extensions want arbitrary UI"
is the **webview** (`src/vs/workbench/contrib/webview/browser/`, a sandboxed
iframe running `pre/index.html`) — an actual embedded browser, which VS Code
can afford because it *is* one. Sprite declines that route. The schema is
enumerated: a handful of element kinds (box, text, list, tree, image/SVG,
button, grid), the utility tokens the interpreter maps, the token registry
verbs, and the events. It carries a protocol version; the endpoint already
refuses `UnsupportedProtocol`. A kind or token is added when a real plugin
needs it and not before.

**Two levels, delivered in order, the cheap one first.** Sprite *already
holds* a structured grid of every terminal program's screen — that is what
the terminal is. **Level 0** applies the theme to that existing grid: font,
line height, cell padding, and colour remapping by token, for every program,
with no protocol and no adapter. It is a small change inside `grid_paint.rs`
and `config.rs` and it makes Neovim, Helix, Croft, `htop`, and everything
else look like the theme immediately. **Level 1** is semantic: a program
streams a grid surface with highlight ids, and styling follows highlight
groups. Level 0 ships first because it is nearly free and because it is the
right fallback when Level 1 is absent.

**The Neovim side lives in `sprite.nvim`, out of process.** `nvim_ui_attach`
is available only over RPC, so the adapter is a process: it runs
`nvim --embed`, attaches as Neovim's UI, translates the redraw stream
(`grid_line`, `win_pos`, `hl_attr_define`, cursor and mode events) into grid
surface updates, and translates Sprite's input events back into
`nvim_input`. This is what Neovide's core does; it is well-trodden.
`sprite.nvim` also holds the Lua API plugins call to describe panels and the
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
styling or panels.

## What ends up where

### In Sprite (`crates/sprite-app`)

- **`surface/` module (new):** the interpreter. Parses a versioned
  description; resolves tokens through the registry; maps utility tokens to
  `gpui::Styled` calls; builds GPUI elements for each kind; owns the grid
  widget, which shares its cell painter with `grid_paint.rs`.
- **Token registry (new, in `config.rs` alongside `Settings`):** built-in
  tokens with per-theme-kind defaults and descriptions; `register` from
  programs; theme overrides by name; `resolve(name) -> Rgba` with a
  documented fallback for an unknown token.
- **Observation protocol (`observation/request.rs`, `endpoint.rs`):** verbs
  `surface open { pane, position: fill|dock|overlay, version }`,
  `surface update`, `surface close`, `token register`, and the event
  direction (`input`, `resize`, `event`) on the same connection. Refusals
  are distinct: unknown pane, pane not a terminal, unsupported version,
  malformed description, unknown element kind or token.
- **`TerminalView` (`terminal_view.rs`):** hosts zero or more surfaces by
  position. *Fill* renders the surface instead of the grid and forwards
  focus, size, and input. *Dock* splits the pane's rectangle, renders both,
  and resizes the PTY. *Overlay* paints above. When a surface's connection
  closes, its space returns to the grid. The `sprite-pane` interface from
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
  lua/sprite/                     the API plugins call to describe panels and register tokens
  bin/nvim                        the launcher: fails open to text-mode Neovim
svgtree.nvim, scm.nvim            send panel descriptions; register their tokens; pure Lua
~~~

## Verification

The capability is proven **without Neovim**, on purpose: a shell script is
the first client, so nothing about Project 1 depends on a repository that does
not exist yet, and program-agnosticism is demonstrated rather than claimed.

1. **Unit, in Sprite.** The parser accepts each element kind and rejects an
   unknown kind, an unknown utility token, and an unsupported version with
   distinct reasons. Every supported utility token maps to a `Styled` call
   (table-driven; the table *is* the supported vocabulary). The registry
   returns a built-in default, a program-registered default, a theme
   override in that order of precedence, and the documented fallback for an
   unknown name. The grid widget applies an incremental line update without
   repainting untouched rows. `TerminalView` hosts a fill, a dock (with the
   PTY told its new size), and an overlay, and returns the space when the
   connection closes. Level 0 changes a cell's drawn colour when the theme
   remaps its token. `shell.rs` prepends a present integration directory and
   ignores an absent one.
2. **Invariant, in Sprite.** The `sprite-pane` manifest test still finds
   `gpui` alone; no Sprite `Cargo.toml` names an editor. The full suite,
   `fmt`, `clippy -D warnings`, and the `--locked --offline` build pass.
   Plain `sprite` with no surface open behaves as before, byte for byte.
3. **End to end, by hand, with a script.** From a shell in a Sprite pane, a
   script opens a *dock* surface describing a panel: a title, a list of
   three rows each with an SVG icon and a label, styled with utility tokens
   and colours referenced by token name. The panel appears beside the
   terminal; `tput cols` in the shell reports the narrower width. Clicking a
   row delivers an `event` to the script, which prints it. Editing the
   theme's value for a token the panel uses restyles the panel live. The
   script exits; the panel disappears and `tput cols` is restored. A second
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
- **Automating the by-hand verification.**
