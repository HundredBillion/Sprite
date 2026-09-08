# Pane Trait and Editor Plurality

**Date:** 2026-09-07
**Type:** Documentation amendment (decision reversal)
**Target:** `terminal-project-brief.md`, `CONTEXT.md`
**Status:** Implemented (commit `0698154d`); partially superseded by
`09-07-2026-native-surfaces.md` — one pane type; programs describe UI over the
Surface Channel and Sprite draws it; the pane interface is what a hosted
Surface satisfies, not a second pane type.

## Problem Statement

The 2026-09-05 reversal (`09-05-2026-gpui-editor-strategy-reversal.md`,
Addendum A.15) made the Croft fork a GPUI-native editor and introduced a
third product, `sprite-studio`, to host it. That amendment solved the right
problem — the editor must leave the terminal grid — but committed to two
structures that a 2026-09-07 review found unnecessary and, in one case,
actively constraining:

**One editor, named in the architecture.** §3 item 4 names exactly two pane
types, "Terminal (Studio's own panes)" and "Editor (the fork)," and §3's
closing boundary paragraph states that the fork is "deliberately *not* a
process boundary — it is a crate dependency." The editor pane and the Croft
fork are the same thing by name. Nothing in the document distinguishes the
socket from the plug. The project owner intends to build three editors —
a Neovim distribution, a Helix fork, and the Croft fork — each in its own
repository, each drawing through GPUI. The current architecture has no place
to put the second one.

**A second product to carry the boundary.** Studio exists to protect the
Phase 1 invariant that no editor code links into Sprite Terminal. That
invariant is worth protecting, but a second product is an expensive way to
express it: two release trains, two version numbers, two packaging paths,
and a shared UI crate extracted solely to let them both use the pane tree.
An interface expresses the same invariant at a fraction of the cost.

A code review the same day found the refactor far smaller than the brief
assumes. `pane_tree.rs`, `tabs.rs`, and `pane_registry.rs` contain zero
references to `TerminalView`; the pane tree is already pure geometry
(`PaneId`, `Orientation`, `Divider`, `Rect`) and `Tabs` is already generic.
All seven `TerminalView` references live in `workspace.rs`, and the load-
bearing one is a single type-parameter instantiation at `workspace.rs:50`:
`tabs: Tabs<gpui::Entity<TerminalView>>`. Generalizing the pane tree to pane
types is not a refactor of the pane tree. Studio was scheduled to pay for
work that is largely already done.

Separately, A.15 scheduled the rename of `sprite-term` to `sprite-engine` on
the grounds that "the `-term` name misleads — it is the engine library, not
the terminal product." Reviewing the Rust ecosystem convention, a crate name
names the crate's *domain*, not its rank in a dependency graph; `-term` reads
as the terminal domain library exactly as `-app` reads as the application.
The rename buys no clarity and costs a mechanical sweep of every import,
document, and package reference. It is reverted before it is executed.

## Decision Being Recorded

**Sprite Terminal defines a pane interface and remains the only product.
Editors are separate repositories that implement that interface. Sprite
never depends on a concrete editor.** (Referred to in session notes as the
"socket and plugs" shape.)

Supporting decisions:

- **`sprite-studio` is dissolved.** There is one product: Sprite Terminal.
  The pane-first identity, the workshop analogy, and the designated home of
  future workspace features all move onto Sprite Terminal itself, which is
  where the pane tree already lives.

- **The product boundary becomes a dependency invariant.** The Phase 1 rule
  ("neither Croft nor any future IDE may become a Sprite dependency") is
  preserved and made mechanically checkable: *Sprite's `Cargo.toml` never
  names a concrete editor.* Dependency arrows point into Sprite and never
  out. Editor repositories know about Sprite; Sprite knows only the trait.

- **A new crate, `sprite-pane`, holds the interface.** It depends on `gpui`
  and `sprite-term` types and nothing else. It is the socket.

- **Sprite's own terminal implements the same trait.** `TerminalView` is a
  first-class consumer of `sprite-pane`, not a privileged special case. This
  is a correctness requirement, not a nicety: an interface whose only in-repo
  implementation is unused rots undetected, and one written solely against an
  editor will silently encode editor assumptions. The terminal exercising it
  every frame is what keeps it honest and keeps its scope at *pane* rather
  than *editor*.

- **Three editor repositories are scheduled, in ascending order of
  difficulty:**
  1. **Neovim pane** — Neovim is a child process speaking msgpack-RPC
     (`nvim_ui_attach`); the pane is a GPUI client that draws what Neovim
     reports. No fork, no editor code linked, architecturally the same kind
     of protocol client Sprite already is for VT. It proves the trait against
     a real editor at the lowest cost.
  2. **Helix fork** — Helix already separates `helix-core` / `helix-view`
     from its TUI in `helix-term`. The GPUI frontend replaces `helix-term`
     against an existing seam rather than carving a new one.
  3. **Croft fork** — the most capable model (editor, LSP, DAP, Git,
     testing, tasks; ~3,100 tests) and by a wide margin the hardest surgery:
     a ~34k-line `App` module, ~50 modules referencing `ratatui`, no renderer
     abstraction. It goes last, by which point the trait has been proven
     twice and cannot be shaped by any single editor.

- **Composition is at build time, not runtime.** GPUI rendering requires
  sharing Sprite's `App`, entity arena, and allocator, which requires the
  same process, which in Rust — absent a stable ABI — requires linking.
  A runtime plugin system loading editor `.so` files that pass GPUI entities
  across the boundary is not safely available today; version, feature, or
  toolchain skew is undefined behavior rather than an error. Zed reached the
  same wall and answered it by giving WASM extensions no rendering access at
  all. Sprite therefore ships a default binary with terminal panes only, and
  builds that include editors are produced by a small distribution crate that
  names its chosen editors. That crate is a build manifest, not a product.

- **The `sprite-term` → `sprite-engine` rename is reverted.** The crate keeps
  the name `sprite-term`. A.15's scheduling of the rename is superseded.

- **Studio's deferred inheritance moves to Sprite Terminal.** The agent-first
  product vision (agent chat, source-control review, diff viewing, inline-
  output annotations), when it gets its own PRD, lands in Sprite Terminal as
  pane types and layouts. There is no second product to receive it.

## Scope: the eight edits

Seven edits target `terminal-project-brief.md`; the eighth targets
`CONTEXT.md` at the project root. Style follows the documents' own
convention: main text describes only what is being built; superseded
decisions move to Addendum A.

1. **§1 Thesis.** The product list returns to one product plus a family of
   editor repositories: Sprite Terminal, and the Neovim/Helix/Croft editor
   panes that implement its pane interface. Delete Studio as product 3.
   The pane-first restatement and the workshop-benches analogy survive
   verbatim in meaning, reattributed from Studio to Sprite Terminal — the
   workshop *is* the terminal, most benches hold terminals, and an editor is
   one optional bench among them. Add the plurality point in the same
   high-level register: the workshop does not care which editor is on the
   bench, and can hold more than one. Update the terminology block: remove
   Studio, restore `sprite-term` as the engine crate name, replace the
   singular "the fork" with "editor panes" (each naming its own product at
   its own branding step), and add `sprite-pane`. Because only one product
   now exists, the note that unqualified "Sprite" names the project rather
   than a product may be relaxed; keep it, but stop treating phase_1's usage
   as an exception needing a grandfather clause.

2. **§2 Core problems.** Rewrite "The nested-terminal boundary dissolves":
   terminal panes are Sprite Terminal's own, powered by `sprite-term`, so any
   forked editor drops its duplicated engine — the point generalizes from the
   Croft fork to all three. Rewrite the four-step build path: extract the
   interface, prove it with the Neovim pane, fork Helix, fork Croft. Add a
   short paragraph naming the composition constraint (GPUI ⟹ same process ⟹
   build-time linking) so no later reader proposes runtime plugins without
   finding the reasoning already recorded.

3. **§3 Architecture.** Restore item 1 to `sprite-term` and delete the
   "named `sprite-term` until the rename lands" parenthetical. Insert
   `sprite-pane` as a new item: the pane interface, depending on `gpui` and
   `sprite-term` only. Rewrite item 2 (`sprite-app`) to state that
   `TerminalView` implements `Pane` like any other pane type, and to drop the
   shared-UI-crate extraction, which existed only to serve Studio. Replace
   item 3 (the Croft fork) with a plural item covering all editor
   repositories and the dependency invariant. Delete item 4
   (`sprite-studio`) entirely. Rescope item 5: the VS Code compatibility
   subsystem belongs to the Croft fork specifically, not to every editor
   pane. Rewrite the closing two-boundaries paragraph: the product boundary
   is now the dependency invariant (Sprite names no editor), and the process
   boundary for extension hosts is owned by whichever editor pane runs one.

4. **§5 Ecosystem and stack.** Restore `sprite-term` in the PTY-dependency
   line. Replace the "Croft: fork after Phase 1 validation" entry with an
   entry covering the three editor sources and their differing integration
   shapes (Neovim: child process + RPC; Helix and Croft: forked Rust models
   with GPUI views). Add Helix and Neovim to the upstream inventory alongside
   Croft, each with its role. Rewrite "Native editor panes: scheduled via the
   fork" — native editor panes are scheduled as three repositories, and the
   Neovim panel that A.8 deferred and A.15 declared unscheduled is now
   scheduled first. Note in the Croft risk list that a heavily diverged fork
   is now one of three implementations rather than the single editor answer,
   which lowers its blast radius.

5. **Phase 2 rewrite and renumber.** The phase becomes "Pane interface and
   editor panes." Items are renumbered because the ordering changes; phase
   *numbers* (0–5) are unchanged.
   - 2.1 Extract `sprite-pane`; make `TerminalView` implement it; generalize
     `Workspace`'s pane type parameter. In-repo, no external dependency.
   - 2.2 Neovim pane repository: GPUI client over `nvim_ui_attach`. First
     external implementation; the trait's acceptance test.
   - 2.3 Distribution crate and packaging story: default `sprite` ships
     terminal panes only; editor-bearing builds are composed explicitly.
   - 2.4 Helix fork repository: GPUI frontend replacing `helix-term`.
   - 2.5 Croft qualification and fork — absorbing the previous 2.1 (freeze a
     baseline), 2.2 (separate fork repository), 2.3 (characterize before
     changing), and 2.4 (model/view separation) unchanged in substance.
   - 2.6 Branding and configuration: retained, now plural — each editor pane
     chooses its own product name and assets; design tokens centralize in
     `sprite-pane` or a shared token crate; Croft, Helix, and Neovim
     attribution and license notices are retained per repository.
   Delete the previous 2.5 (Studio foundation) — the shared UI crate,
   the pane-tree generalization it scheduled, and the `sprite-engine` rename
   are respectively unnecessary, absorbed into 2.1, and reverted.

6. **Phase 3 rewrite.** VS Code visual parity is measured against the Croft
   fork's GPUI renderer specifically, since it is the pane pursuing VS Code
   parity; the Neovim and Helix panes have their own visual references and
   are not held to the VS Code corpus. 3.3's harness renders deterministic
   workspaces in Sprite Terminal rather than Studio.

7. **Addendum A.16.** "Studio as a separate product, the single-editor
   architecture, and the `sprite-engine` rename: SUPERSEDED (2026-09-07)."
   Records the socket-and-plugs decision, the dependency invariant that
   replaces the product boundary, the code finding that the pane tree is
   already generic, the build-time composition constraint and why runtime
   plugins were rejected, the three-editor ordering and its rationale, and
   the rename reversal with the Rust naming-convention reasoning. Written at
   the level of detail of A.10–A.15. Cross-reference A.15, which it partially
   supersedes: A.15's grid-physics analysis and GPUI-only decision stand
   unchanged; only its Studio, single-editor, and rename provisions fall.

8. **`CONTEXT.md` (project root).** Delete the **Studio** entry. Restore the
   **Sprite Engine** entry to **`sprite-term`**, removing the rename note and
   the `_Avoid_: sprite-term (post-rename)` line. Replace **The Fork** with
   **Editor Pane** (an implementation of `sprite-pane`'s trait, living in its
   own repository, never a Sprite dependency). Add **Pane** as the primitive
   and **`sprite-pane`** as the interface crate. Rewrite **Pane-first** to
   describe Sprite Terminal rather than Studio. Rewrite **Croft (upstream)**
   to note it is one of three model sources, and add Helix and Neovim
   entries. `CONTEXT-MAP.md` was checked and references none of the deleted
   terms, so it needs no edit.

Additional minor edits: correct the stale `Status: Draft` header on
`09-05-2026-gpui-editor-strategy-reversal.md`, whose seven edits landed in
commit `0c088436`; sweep §§7–11 for sentences that assume Studio hosts the
editor or that "the fork" is singular, and update only those.

## Design sketch (non-normative)

Recorded so the follow-on TSP starts from a shared picture. Nothing here is
binding; the implementation PRD may choose differently.

The interface is `Pane`, not `Editor`. Nothing a pane must do is
editor-specific:

```
render · focus · title · handle input · save/restore session state · can_close
```

The mechanism that makes it object-safe across GPUI's entity system is an
open implementation question with at least three candidates — a trait object,
a handle wrapper in the shape Zed uses for `ItemHandle`, or an enum of known
pane types — and the choice depends on gpui 0.2.2 API details this amendment
does not attempt to settle.

The change at the call site is small. Today:

```rust
// crates/sprite-app/src/workspace.rs:50
tabs: Tabs<gpui::Entity<TerminalView>>,
```

`Tabs` is already generic and `pane_tree.rs` is already pure geometry, so the
work is instantiating that parameter with the pane handle instead, and
following the six other `TerminalView` mentions in `workspace.rs` (`:22`,
`:247`, `:325`, `:616`, `:1072`).

## Acceptance Criteria

- The main text of `terminal-project-brief.md` describes only the new plan;
  no section still presents Studio as a product, "the fork" as the single
  editor, or the `sprite-engine` rename as scheduled.
- No occurrence of `sprite-studio` or `sprite-engine` remains outside
  Addendum A.
- Addendum A.16 exists and states what was superseded, when, and why, at a
  level of detail consistent with A.10–A.15, and is explicit about which
  parts of A.15 survive.
- The Phase 1 no-editor-dependency invariant survives, strengthened into the
  checkable form: Sprite's `Cargo.toml` never names a concrete editor.
- The requirement that `TerminalView` implement the same trait as editor
  panes appears in the main text, with its rationale, not only in the
  addendum.
- All three editors (Neovim, Helix, Croft) appear in §5 and Phase 2 with
  their differing integration shapes and their ordering rationale.
- Phases 0, 1, 4, and 5 are unchanged except where they referenced Studio,
  the singular fork, or `sprite-engine`.
- The build-time composition constraint is recorded in the main text with its
  reasoning, so runtime plugin loading is not re-proposed without cause.
- No text claims literal pixel-for-pixel identity with VS Code; parity claims
  stay tied to the screenshot-comparison standard in §1 and A.10, and apply
  to the Croft fork pane specifically.
- `CONTEXT.md` and the brief agree on every term.

## Out of Scope

- Implementation of any of it: the `sprite-pane` extraction, the
  `TerminalView` trait implementation, the `Workspace` type-parameter change,
  the distribution crate, or any editor repository. Each gets its own
  PRD/TSP cycle.
- Choosing the object-safety mechanism for the pane trait. That is a
  technical decision for the 2.1 TSP, informed by gpui 0.2.2's API.
- Product names for the three editor panes, which remain a Phase 2.6
  decision per repository.
- The agent-first product vision. Still deferred to its own future PRD; it
  now lands in Sprite Terminal rather than Studio.
- Any change to Phase 1 code or its documentation under `phase_1/`, including
  `phase_1/CONTEXT.md`.
- Renaming or renumbering the build-plan phases themselves. Only the item
  numbering *within* Phase 2 changes.
