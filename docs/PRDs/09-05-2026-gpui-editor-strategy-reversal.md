# GPUI Editor Strategy Reversal

**Date:** 2026-09-05
**Type:** Documentation amendment (decision reversal)
**Target:** `terminal-project-brief.md`
**Status:** Draft

## Problem Statement

The project brief's Phases 2–3 commit the Croft fork to a TUI-first strategy:
visual parity is pursued on the ratatui/terminal-cell path, with a native GPUI
renderer held as "a last resort, not the default Phase 3 plan" (§3.5), behind a
grid-ceiling gate that requires screenshot-test failure before any renderer
work is permitted.

A 2026-09-05 analysis of the Phase 1 codebase (`grid_paint.rs`,
`terminal_view.rs`, `grid.rs`) and the terminal rendering model concluded that
the grid ceiling is not a gate to be tested but a structural property of the VT
protocol: cell-quantized text addressing, whole-row scrolling, one cell height
per pane, and cell-anchored graphics placement. Pixel-level visual parity with
VS Code cannot pass through a PTY, no matter how well the terminal renders.
Waiting for Phase 3 screenshot tests to prove this would spend the most
expensive phase of the plan discovering a conclusion that is already available.

The brief must be amended to make the GPUI view-layer fork the plan rather
than the escape hatch, and to record the reversal honestly in Addendum A.

## Decision Being Recorded

**The Croft fork becomes a GPUI-native editor: Croft's model (editor state,
LSP, DAP, Git, testing, tasks) is retained; its ratatui view layer is replaced
with GPUI. No dual-renderer seam is built or maintained — the fork is
GPUI-only.** (Referred to in session notes as "option (c).")

Supporting decisions:

- **Remote/SSH editing** is served by Neovim or unmodified upstream Croft
  running in an ordinary terminal pane. The fork owes no TUI fallback. The
  project owner confirmed SSH-based editing is rare in his workflow and Neovim
  is his preferred tool for it.
- **A third product, working name `sprite-studio`,** hosts the forked editor
  as a native pane type alongside terminal panes. It depends on `sprite-term`
  and on UI components extracted from `sprite-app`. Sprite Terminal itself
  remains pure: the Phase 1 boundary ("neither Croft nor any future IDE may
  become a Sprite dependency") is preserved — the fork links into studio,
  never into the terminal.
- **The maintenance trade is accepted:** a GPUI-only fork diverges heavily
  from upstream Croft, and upstream syncs become harder over time. In
  exchange, the fork avoids both the hardest surgery (extracting a stable
  dual-renderer seam from a ~34k-line App module with no existing renderer
  abstraction) and a permanent second-renderer maintenance obligation.

## Scope: the seven edits

All edits target `terminal-project-brief.md`. Style follows the document's own
convention: the main text describes only what is being built; superseded
decisions move to Addendum A.

1. **§1 Thesis.** Redefine product 2: the Croft fork is a GPUI-native editor
   (Croft model + GPUI view), hosted as a pane type in `sprite-studio`. The
   product list becomes three: Sprite Terminal (unchanged), the fork, studio.
   Replace the "standards-based TUI fallback" line with the Neovim/upstream-
   Croft answer for remote editing.
2. **§2 Core problems.** Rescope "the terminal grid ceiling" to TUI programs
   running in terminal panes (accepted, not fought). Replace the four-step
   progressive-enhancement escape path: the GPUI view fork is the plan. Record
   that the nested-terminal boundary problem largely dissolves — the fork
   drops its internal `alacritty_terminal` engine because studio provides
   terminal panes via `sprite-term`.
3. **§3 Architecture.** Introduce `sprite-studio`: depends on `sprite-term`
   plus a UI crate extracted from `sprite-app`; hosts two pane types
   (Terminal, Editor). State explicitly that Sprite Terminal's no-IDE-
   dependency boundary survives.
4. **§5 Ecosystem.** Croft's role becomes "model/logic foundation for the
   native editor pane." Update the risk list: heavy upstream divergence,
   harder syncs. Supersede the "Deferred research: native Neovim and Helix
   panels" section: a native editor pane is now scheduled (the Croft-fork
   pane); Neovim/Helix panels remain unscheduled.
5. **Phase 2 rewrite.** Keep 2.1 (freeze baseline), 2.2 (separate fork repo,
   now consumed by studio as a Cargo dependency), 2.3 (characterize before
   changing), 2.6 (branding). Replace 2.4–2.5 with model/view separation:
   carve Croft's model out of ratatui incrementally; GPUI view replaces TUI;
   no dual-renderer seam. Add a studio-foundation item: extract the shared UI
   crate from `sprite-app`, generalize the pane tree to pane types, stand up
   the `sprite-studio` crate.
6. **Phase 3 rewrite.** Goal unchanged — VS Code visual parity, measured by
   screenshot comparison — but against the GPUI renderer. Keep 3.1 (reference
   corpus), 3.2 (design tokens, mapped to GPUI), 3.4 (visual regression
   harness). Delete 3.3 (Kitty-graphics TUI workarounds) and 3.5 (grid-ceiling
   gate) as moot.
7. **Addendum A.15.** "TUI-first Croft strategy and grid-ceiling gate:
   SUPERSEDED (2026-09-05)." Records the grid-physics analysis, the GPUI-only
   decision, the SSH answer, and the accepted maintenance trade.

Additional minor edits: one line under "Ongoing / cross-cutting" for the
terminal quality-of-life items (configurable line height, configurable pane
padding, smooth scrolling) as Phase 1 maintenance work; sweep §7–§11 for
sentences that assume the TUI fallback and update only those.

## Acceptance Criteria

- The main text of `terminal-project-brief.md` describes only the new plan;
  no section still presents the TUI-parity path or grid-ceiling gate as
  current.
- Addendum A.15 exists and states what was superseded, when, and why, at a
  level of detail consistent with entries A.6–A.14.
- Phases 0, 1, 4, and 5 are unchanged except where they referenced the TUI
  fallback.
- The Phase 1 product boundary sentence ("the IDE never becomes a dependency
  of the terminal") survives verbatim or strengthened.
- `sprite-studio` is consistently used as the working name, marked as a
  working name.
- No text claims literal pixel-for-pixel identity with VS Code; parity claims
  stay tied to the screenshot-comparison standard already defined in §1 and
  A.10.

## Out of Scope

- The agent-first product vision (Claude-Code-style layout, source-control
  review UI, inline-output annotations). Deferred to its own future PRD.
- Implementation of the terminal QoL features, the shared-UI-crate
  extraction, the pane-tree generalization, or any `sprite-studio` code
  (sub-projects A, B, C — each gets its own PRD/TSP cycle).
- Any change to Phase 1 code or its documentation under `phase_1/`.
- Renaming or renumbering the build-plan phases.
