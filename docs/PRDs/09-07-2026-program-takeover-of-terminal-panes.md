# Program Takeover of Terminal Panes

**Date:** 2026-09-07
**Type:** Architecture (composition seam) plus a proof in a separate repository
**Target:** `crates/sprite-app` (`lib.rs`, `main.rs`, `terminal_view.rs`,
`workspace.rs`, `observation/`), `crates/sprite-term/src/shell.rs`,
`terminal-project-brief.md`; a new repository, `sprite-nvim-pane`
**Status:** Designed 2026-09-07 (brainstorming session). Supersedes the
"Terminal pane / Editor pane" framing of
`09-07-2026-pane-trait-and-editor-plurality.md` for editors; that PRD's
dependency invariant is preserved unchanged.

## Summary

Sprite's roadmap planned editors as a second *kind of pane* beside the
terminal, chosen when a pane is opened. That is not how anyone uses an editor
from a terminal. A person opens a pane, gets a shell, and types `nvim .`. They
can do that in Sprite today, and Neovim appears — drawn as text, the way a
terminal draws everything.

This PRD makes that exact gesture the way editors join Sprite, and changes
nothing about it from the person's side. **There is one kind of pane: a
terminal.** A program running in it may *borrow* the pane's rectangle and draw
itself natively for as long as it runs; when it exits, the shell is back, in
the same directory, with the program's exit code. The pane never changed type.
A program used it for a while.

The mechanism does not know what an editor is. Sprite keeps a registry —
*program name → how to draw it* — that is **empty in the default `sprite`
binary** and filled by whichever build bundles an editor. Sprite itself never
learns the name `nvim`, `hx`, or `croft`. That is what keeps the roadmap's
invariant, *Sprite's manifest names no editor*, true by construction, and what
makes the design equally shaped for Neovim, Helix, and Croft: adding one is one
more entry in the registry, supplied from that editor's own repository.

This document covers the seam in Sprite and the smallest thing that proves it
end to end: a **hollow** Neovim renderer, in its own repository, that draws a
labelled box in place of the terminal and hands the pane back when dismissed.
The real Neovim renderer, the `sprite.nvim` API for plugins, and the Neovim
distribution are later projects (see *Out of scope*).

## User outcome

- In a build that bundles the Neovim renderer, a person opens a Sprite pane,
  types `nvim .`, and the pane becomes Neovim drawn natively. They quit, and
  their shell prompt returns in the same directory; `$?` is Neovim's exit
  code. Nothing about opening panes, splitting, tabs, or dividers changed.
- In plain `sprite`, `nvim .` runs text-mode Neovim exactly as it does today.
  There is nothing to configure and nothing to opt out of.
- In a bundled build, a program with no registered renderer — or a takeover
  Sprite refuses — falls through to the real program, drawn as text. The
  fallback is what happens when nothing native answers, not a mode.
- The same gesture is available to every editor with a command line. A
  future Helix or Croft bundle adds an entry; it does not add a mechanism.

## Decisions, and why

**One pane type; programs borrow it.** The pane tree only ever holds terminal
panes. A native renderer is not a peer of the terminal in the tree; it is
something a terminal pane *hosts* while a program runs, then releases. The
alternative — distinct pane kinds, chosen from a picker when a pane opens —
was designed and rejected in this session: it inserts a decision ("Terminal or
Neovim?") that the shell already answers, and it would have taught Sprite a
list of editor names to put in the picker. The `Rc<dyn PaneHandle>` slot the
workspace already holds (`workspace.rs`, from PR #27) remains for panes that
genuinely are not terminals — the agent-chat and diff-viewing panes on the
roadmap's horizon — but editors do not enter through it. They enter through
the shell, like every other program.

**The renderer contract is the existing `sprite-pane` interface, reused as
"the thing a pane hosts."** `PaneHandle` already exposes everything a hosted
renderer needs to offer — `view() -> AnyView`, `title`, `focus_handle`,
`set_allocated`, `begin_shutdown`, `close_warning` (`crates/sprite-pane/src/lib.rs`).
A terminal pane in native mode renders the hosted `view()` in place of its
grid, hands it focus, and forwards its allocated size. No new trait. The
interface built for "pane types" finds its real job here, and the work in
PR #27 is not undone; it is re-aimed.

**The takeover request rides the observation socket, not a terminal escape.**
Every child process Sprite spawns already receives `SPRITE_PANE` (its pane's
id — `observation/endpoint.rs:308`), `SPRITE_OBSERVATION_SOCKET`, and
`SPRITE_OBSERVATION_KEY`, and the `Workspace` already serves authenticated
requests on that socket (`workspace.rs:608`; `sprite panes snapshot` and
`sprite config reload` use it). A takeover is one more verb on that channel,
beside `config reload`. The alternative — a private OSC sequence through the
PTY — was rejected because `sprite-term` surfaces only specific OSCs (7, 10,
11, 12, 4, 52, 133) and has no passthrough for unknown ones; adding one would
be new parser surface for a message that already has a better, authenticated
door. A request must present the key, so a stray program on the machine
cannot seize a pane.

**The takeover connection stays open for the life of the program, and its
response is the exit code.** The helper sends one request and blocks reading
the reply. Sprite answers only when the hosted renderer finishes, with the
program's exit status. The helper then exits with that status, so the shell
sees `nvim .` complete like any foreground command: `$?`, job control, `&&`
chains, and the prompt's redraw all behave. The endpoint's existing short
request timeout ("this window did not answer in time") does not apply to a
takeover connection; it is explicitly long-lived.

**The helper lives in `SPRITE_SHELL_INTEGRATION_DIR`, which Sprite now
prepends to the child's PATH.** Sprite already exports that variable to
children and already prepends its *executable* directory to PATH
(`shell.rs`, `prepend_path`; tested by
`the_executable_directory_becomes_the_first_path_entry`). Its comment records
that the integration directory is "not yet injected automatically" — a known
gap, closed here: when the variable names a present directory, Sprite puts it
first on the child's PATH. A file named `nvim` there shadows the real Neovim
for the shell *inside Sprite only*. The alternative — installing the helper
beside the binary in the executable directory — works inside a macOS bundle
(`Contents/MacOS` is private to the app) but on a Linux prefix install would
put a file named `nvim` in `/usr/bin` and clobber the system package. A
Sprite-owned directory avoids that on every platform. Plain `sprite` sets no
integration directory, so nothing is shadowed and nothing changes.

**The helper is a tiny program that fails open.** It reads `SPRITE_PANE`, the
socket path, and the key from its environment; sends a takeover request
naming its program, its working directory, and its arguments; and blocks for
the reply. If the socket variables are absent (plain Sprite, or not inside
Sprite at all), if the connection is refused, or if Sprite answers that no
renderer is registered for that name, the helper `exec`s the real program
found on PATH *with its own directory removed*, so it never re-executes
itself. Every failure path ends in text-mode Neovim, which is the correct
fallback and costs the person nothing to reach.

**The registry is filled by the caller of Sprite's entry point, never by
Sprite.** `sprite-app` is already a library with a thin `main.rs`. The library
gains a `run` entry that takes the parsed invocation and a registry of
takeover renderers; `main.rs` passes an empty registry. The dev harness in
`sprite-nvim-pane` passes `{ "nvim" → the renderer }`. The default binary
therefore contains no editor code — not compiled-and-unused, but never
compiled — because the renderer is a dependency of the harness, not of
Sprite. A Cargo feature on `sprite-app` was rejected: it would name the
renderer crate in Sprite's manifest, which is exactly what the invariant
forbids. Runtime plugin loading was rejected on the grounds the roadmap
already recorded: GPUI requires one process and one set of versions, and
Rust has no stable ABI to make that safe.

**The proof is hollow on purpose.** The first renderer draws a coloured box
labelled "Neovim" and finishes on a keypress, returning exit code 0. It
launches no process and speaks no protocol. It exists to answer one question
before the expensive work starts: *does a renderer built in another
repository, bundled by a harness, integrate with Sprite — same tab tree, same
dividers, same shutdown, shell restored?* If it does, the Neovim renderer
has a proven home. If it does not, we learn it in a day, not after a
renderer exists that fits nothing.

**The Neovim repository depends on Sprite by git, pinned to an exact commit.**
Sprite is not published to crates.io, so the harness names Sprite's
repository and a full commit hash — the same discipline as Sprite's own
`gpui = "=0.2.2"` — so a change in Sprite can never silently break the pane.
A one-line local `[patch]` points at a sibling checkout during development.
The dependency arrow points from the pane to Sprite and never back.

**Two repositories now; a third when there is a product.** The socket lives in
Sprite. The hollow renderer, the `nvim` helper, and the dev harness live
together in `sprite-nvim-pane`, because a library needs a way to run itself
and the harness is that. The "Sprite-friendly Neovim distribution" — Sprite
plus the real renderer plus a Neovim configuration and branding — is the
roadmap's distribution crate (2.3) grown into a product; it gets its own
repository when the renderer does something worth shipping, and it depends
on `sprite-nvim-pane` rather than duplicating it.

**The brief is amended so the next reader is not sent to the wrong frame.**
`terminal-project-brief.md` describes editors as pane types and the pane
interface as their door. One paragraph records the supersession: one pane
type, programs borrow it, the registry, and that the `sprite-pane` interface
is the hosted-renderer contract. The dependency invariant and the three
editor repositories are unchanged and are restated, not removed.

## What ends up where

### In Sprite (`crates/sprite-app`, `crates/sprite-term`)

- `sprite_app::run(invocation, takeovers)` — the library entry point.
  `takeovers` is a registry keyed by program name; each entry is a
  constructor that, given the request's working directory and arguments and
  the pane's window and app context, returns an `Rc<dyn PaneHandle>`.
  `main.rs` becomes a thin caller that passes an empty registry.
- `TerminalView` gains a **native mode**: while it hosts a renderer it renders
  the hosted `view()` in place of the grid, forwards focus and allocated size
  to it, and reports the hosted `title()` as its own. When the hosted
  renderer's `begin_shutdown` completes — or it signals exit with a status —
  the pane returns to the shell and the pending takeover connection is
  answered with that status. The PTY and shell stay alive throughout; the
  program was the helper, which is blocking on the socket.
- The observation protocol gains a `takeover` verb: `{ program, cwd, args }`
  from a connection that presents the key and a `SPRITE_PANE` that names a
  live terminal pane. Responses: the exit status on completion; a refusal
  when the program is unregistered, the pane is not a terminal in shell
  mode, or the pane is already hosting. The connection is exempt from the
  short request timeout.
- `shell.rs`: when `SPRITE_SHELL_INTEGRATION_DIR` names a present directory,
  it is prepended to the child's PATH ahead of the executable directory.
- `terminal-project-brief.md`: the supersession paragraph.

### In the new repository `sprite-nvim-pane`

~~~
sprite-nvim-pane/
  Cargo.toml            depends on sprite-app + sprite-pane by git, pinned
  src/lib.rs            the renderer: implements sprite_pane::Pane; hollow
  src/bin/nvim.rs       the helper: takeover request, block, exit; fails open
  src/bin/harness.rs    sprite_app::run(..., { "nvim" -> renderer })
~~~

The harness installs the `nvim` helper into a directory it names as
`SPRITE_SHELL_INTEGRATION_DIR` for the Sprite it launches.

## Verification

The Project 1 finish line is all of the following, in this order:

1. **Unit, in Sprite.** The registry resolves a registered name and refuses
   an unregistered one. A `takeover` request parses; a missing key, an
   unknown pane, a non-terminal pane, and an already-hosting pane are each
   refused with a distinct reason. `TerminalView` enters native mode on a
   hosted handle and returns to shell mode when it finishes, reporting the
   status. `shell.rs` prepends a present integration directory and ignores
   an absent one; the existing PATH tests keep passing.
2. **Invariant, in Sprite.** The `sprite-pane` manifest test still finds
   `gpui` alone; a grep of every Sprite `Cargo.toml` finds no editor name.
   The full suite, `fmt`, `clippy -D warnings`, and the `--locked --offline`
   build pass unchanged. Plain `sprite`'s behaviour is identical: with an
   empty registry every `takeover` is refused and no PATH entry is
   added.
3. **End to end, by hand, in the harness.** Open a pane; `nvim .` → the pane
   becomes the labelled box, in the same tree, resizable by the same
   dividers, its title in the tab; press a key → the shell prompt returns,
   `pwd` is unchanged, `echo $?` prints `0`. Then run a program whose helper
   is present but unregistered → text-mode program runs (the refusal
   fallback). Then quit Sprite while a box is hosted → shutdown completes
   through the same path as a terminal.
4. **End to end, by hand, in plain `sprite`.** `nvim .` opens text-mode
   Neovim exactly as before this change.

Steps 3 and 4 are written into the TSP as checkbox steps; they have no
automated seam yet, and inventing one is not this project.

## Out of scope

- **The real Neovim renderer** — launching `nvim --embed`, `nvim_ui_attach`,
  drawing grids and highlights. Project 2. This document gives it a home.
- **The `sprite.nvim` API** for plugins to draw native widgets. Project 3.
- **The Neovim distribution** and its repository. Project 4 / roadmap 2.3.
- **Helix and Croft renderers.** Each is an entry in the same registry when
  its repository exists; nothing here is specific to Neovim beyond the
  proof's label.
- **Non-terminal panes** (agent chat, diff viewers). They keep the
  `Rc<dyn PaneHandle>` slot in the tree for when they are designed.
- **Runtime plugin loading.** Rejected above and in the roadmap.
- **Automating the by-hand verification.** Named as absent, not deferred by
  accident.
