# Describe surfaces over a socket; keep editor code out of Sprite's process

A program running in a pane **describes** user interface — a versioned document
of element kinds, utility tokens for style, and token names for colour — over
an authenticated Unix socket, and Sprite Terminal draws it with GPUI. The code
that understands an editor (Neovim's UI protocol, a plugin's tree) never
touches GPUI and never links into Sprite; it runs beside the editor and sends
descriptions.

Taken while brainstorming and grilling the native-surfaces PRD on 2026-09-07
(`docs/PRDs/09-07-2026-native-surfaces.md`), after two earlier designs for the
same goal were written and rejected the same day.

## Why

The goal was native, stylesheet-controlled rendering for Neovim's editing area
and for plugins such as `svgtree.nvim` and `scm.nvim`, without teaching Sprite
Terminal any editor's name. Three shapes were tried.

**An editor pane type**, chosen from a picker when a pane opens, was the
roadmap's plan. It asks a question the shell already answers — nobody picks
"Neovim" from a menu; they type `nvim .` — and it makes Sprite's source name
every editor it can open. Rejected.

**Program takeover** kept one pane type and let `nvim .` borrow the pane for a
native renderer. But it assumed the renderer was Rust linked into Sprite, which
forced a separate renderer repository, a distribution crate to bundle it, a
PATH-shadowing helper, and a live edge case around shell aliases. Rejected once
the assumption was noticed.

**A description boundary** dissolves the assumption. Once what crosses the
boundary is a document rather than a call, the code that produces it needs no
access to GPUI and so need not share Sprite's process. Neovim already emits
such a document — its UI protocol streams every line, highlight group, window
position, and cursor on each keystroke — so an adapter beside it forwards that
stream, and a Lua plugin sends its own. Zed reached the neighbouring
conclusion for its extensions (WASM extensions get no rendering access,
because in-process rendering across an ABI is unsafe); Sprite arrives from the
other side, giving programs a language for rendering instead of denying it.

## What follows from it

**The dependency invariant becomes structural.** Sprite's manifest names no
editor because nothing about an editor is compiled into Sprite: there is
nothing to bundle and no distribution crate. Two repositories — Sprite and
`sprite.nvim` — plus the plugins.

**Sprite owns a small UI description language and must keep it small.**
Element kinds, utility tokens that map one-for-one onto GPUI's `Styled`
methods, and a token registry; versioned; grown only when a real plugin needs
a kind. A UI description language is a browser engine if nobody says no; the
schema is enumerated so that someone has.

**Styling is CSS-shaped, not CSS-syntax.** GPUI lays out with Taffy — the CSS
flexbox and grid algorithms — and styles through Tailwind-shaped utilities;
Sprite translates tokens into calls it already has and builds no cascade. The
one idea taken from VS Code is the extensible token registry: a plugin can add
a colour role by name, which Zed's fixed theme struct cannot express.

**ADR 0016 stands, re-aimed.** Panes are still held as `Rc<dyn PaneHandle>`,
but the handle now describes what a pane *hosts* — a Surface — rather than a
pane of another type. Its motivating case, "an editor pane from another
repository," is superseded by this record; its storage decision is not.

**Level 0 comes free.** Because Sprite already holds a structured grid of every
terminal program's screen, the theme can style that grid for every program
with no protocol at all; Helix and Croft need no fork to look right.
