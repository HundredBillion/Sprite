# Sprite Terminal Checkpoint 2 Technical Spec

> **Status: DRAFT — not reviewed.** Checkpoint 1's TSP contained two commands
> that did not work against the pinned source and one uncompilable signature,
> each found only during execution. This document deserves the same grilling
> before anyone starts Task 1.

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> dmi-superpowers:subagent-driven-development (recommended) or
> dmi-superpowers:executing-plans to implement this TSP task-by-task. Steps use
> checkbox (- [ ]) syntax for tracking.

**Goal:** Turn the Checkpoint 1 spine into a terminal that is correct enough for
daily use: a real cell grid, scrollback with a stable viewport, selection and
clipboard, the full key and mouse protocols, and shell integration that reports
what the child is actually doing.

**Architecture:** Unchanged. `sprite-term` remains the sole owner of the PTY,
libghostty, and the terminal-owner worker; `sprite-app` remains the sole owner of
GPUI. Checkpoint 2 deepens both sides of the existing seam and adds no new
module boundary. Every new capability arrives as additional `TerminalCommand`
variants, additional `TerminalEvent` variants, and richer owned snapshot fields.

**Tech Stack:** Unchanged from Checkpoint 1, with two expected additions
requiring dependency ledger entries: a clipboard path (GPUI's, if sufficient)
and base64 decoding for OSC 52.

## Global Constraints

Checkpoint 1's constraints carry forward unchanged and are not restated. In
particular: no async runtime, no `unsafe impl Send/Sync`, libghostty values live
and die on the terminal-owner worker, bounded queues, offline locked builds, and
tests that exercise only the public interface.

Additional constraints for this checkpoint:

- **Linux first.** Checkpoint 2 is accepted on Arch Linux. macOS is held to
  compile-and-test parity in CI, and its interactive gates join the macOS
  acceptance milestone. See the PRD's platform posture.
- **Accessibility is out of scope**, and this is not a choice. GPUI `=0.2.2`
  provides no accessibility surface at all. PRD story 62 and the focused-pane
  accessibility state the PRD assigns to this checkpoint are deferred to a GPUI
  release that ships AccessKit. See ADR 0012.
- **One event never reaches two consumers.** Mouse input goes to the child or to
  Sprite selection, never both.
- **Terminal state decides, never the application.** Mouse modes, Kitty keyboard
  flags, bracketed paste, and focus reporting are read from libghostty at the
  moment of use, exactly as Checkpoint 1's key encoder already does.

## Checkpoint boundary

Checkpoint 2 includes:

- a cell-positioned renderer with per-cell colour, style, and wide-cell layout;
- scrollback history, a viewport that anchors or follows, and fractional scroll;
- Sprite text selection, copy, paste, and an OSC 52 policy;
- the full key protocol including Kitty keyboard, bracketed paste, and IME;
- terminal mouse reporting with a Shift override for selection;
- shell integration for Bash, Zsh, and Fish, with prompt marks and working
  directory;
- title, bell, working-directory, and hyperlink metadata as lifecycle events;
- observation metadata on `PaneSnapshot` for Checkpoint 3 to expose;
- refreshed performance budgets covering scrollback capture and scroll cadence.

Checkpoint 2 does not claim tabs, splits, Pane Observation IPC, Kitty graphics,
packaging, or accessibility.

## PRD traceability

| PRD requirement | Checkpoint 2 evidence |
|---|---|
| Correct text rendering | Task 1; cell-positioned renderer and the Croft regression case |
| Active-screen history, scrolling, viewport anchoring | Tasks 2 and 3 |
| Selection, copy, paste, OSC 52 policy | Tasks 4 and 7 |
| Full key protocol, Kitty keyboard, bracketed paste, IME | Task 6 |
| Mouse reporting and the Shift override | Task 5 |
| Shell integration, working directory, prompt marks | Task 8 |
| Title, bell, hyperlink metadata | Task 9 |
| Observation metadata for Checkpoint 3 | Task 8; `PaneSnapshot` fields |
| Story 62, focused-pane accessibility | **Deferred.** GPUI 0.2.2 has no accessibility surface; ADR 0012 |
| Numerical budgets before Checkpoint 3 | Task 10 |

## Known inputs from Checkpoint 1

Three findings from Checkpoint 1 that this checkpoint must act on:

1. **The renderer flows text rather than positioning cells.** Rows are joined
   into strings and handed to GPUI, so each glyph renders at its natural advance.
   Confirmed against Croft: box-drawing and Nerd Font glyphs shift everything
   after them on a row. Task 1 exists to fix exactly this.
2. **Snapshot capture allocates a `String` per cell.** Harmless at 24x80, but
   scrollback capture multiplies it. Task 2 should measure before optimising, and
   Task 10 re-freezes budgets either way.
3. **Debug builds are ~25x slower than release** on heavy output. Any new
   benchmark keeps the reduced-volume schema test that Checkpoint 1 introduced.

---

### Task 1: Position cells on a grid

**Files:** `sprite-app/src/terminal_view.rs`, new `sprite-app/src/grid.rs`

**Interfaces:** Consumes `RenderSnapshot`. Produces a drawn grid. No change to
`sprite-term`.

- [ ] Write a failing test for the pure layout helper: given cell metrics and a
  `RenderRow`, it yields one positioned run per contiguous span sharing a style,
  with a `Wide` cell occupying two columns and its `SpacerTail` occupying none.
- [ ] Draw each row as positioned runs rather than one string. Every run carries
  its own foreground, background, and style flags from `CellStyle`.
- [ ] Stop trimming trailing blanks so a row's background reaches the right edge.
- [ ] Draw the cursor from `CursorSnapshot`, honouring visibility and blink.
- [ ] Keep rendering damage driven: an unchanged, unfocused terminal with no
  animation must not repaint. Re-run Checkpoint 1's no-poll scan.
- [ ] Verify against Croft: box-drawing borders align, the status bar's Nerd Font
  glyphs sit on their cells, and a CJK line occupies the expected columns.

### Task 2: Expose scrollback history

**Files:** `sprite-term/src/snapshot.rs`, `sprite-term/src/lib.rs`

- [x] ~~Extend both projections with the scrollback rows above the viewport~~ —
  superseded. Both projections carry `Viewport { total_rows, offset,
  visible_rows }` instead; history is reached by moving the viewport, not by
  copying it. See resolved open question 3.
- [x] Test that a child printing more than one screen of output leaves earlier
  rows retrievable at the same generation as the visible ones.
- [x] Respect the configured scrollback budget; prove that zero keeps nothing
  and that a larger budget retains more.
- [ ] Measure capture cost before optimising. If per-cell `String` allocation
  dominates, replace it with a compact representation in this task, not later.
  Deferred to Task 10's budgets: capture is unchanged in size by this task.

### Task 3: Scroll the viewport

**Files:** `sprite-term/src/worker.rs`, `sprite-app/src/terminal_view.rs`

- [ ] Add a `Scroll` command carrying an ordered row delta. Test that it changes
  which rows a snapshot reports without disturbing the child.
- [ ] Hold fractional offset as application state layered over libghostty's
  row-based viewport: accumulated deltas cross row boundaries by issuing ordered
  scroll commands while the remainder translates rendering.
- [ ] A viewport at the live bottom follows new output. A viewport reading older
  scrollback stays anchored and reports an unseen-line count.
- [ ] Keyboard input and paste return the Pane to the live bottom. Selection,
  copy, and search preserve the viewport. Test both.

### Task 4: Select and copy

**Files:** new `sprite-app/src/selection.rs`, `sprite-term/src/snapshot.rs`

- [ ] Model selection in cell coordinates anchored to a generation, so a
  selection survives new output arriving beneath it.
- [ ] Support character, word, and line granularity. Table-test word boundaries
  against wide characters and combining marks.
- [ ] Render the selection overlay from snapshot state, not from a second copy of
  the text.
- [ ] Copy yields the selected text with trailing blanks stripped per row and
  wrapped rows rejoined without an inserted newline.

### Task 5: Mouse

**Files:** `sprite-term/src/worker.rs`, `sprite-app/src/input.rs`

- [ ] Add an owned `Mouse` command: position in cells, button, action,
  modifiers. `sprite-term` encodes it through libghostty against live mouse
  modes; no encoded bytes cross the seam.
- [ ] Test that a child enabling mouse reporting receives events, and that one
  disabling it does not.
- [ ] When reporting is inactive, drag performs Sprite selection. When active,
  events go exclusively to the child; Shift overrides for selection. Assert no
  event ever reaches both paths.
- [ ] Make the override modifier configurable, defaulting to Shift.

### Task 6: Complete the key protocol

**Files:** `sprite-term/src/worker.rs`, `sprite-app/src/input.rs`

- [ ] Negotiate Kitty keyboard flags from terminal state; test that the same
  keystroke encodes differently as flags change, as Checkpoint 1 did for
  cursor-application mode.
- [ ] Implement bracketed paste: wrap pasted text per terminal state, chunk it
  through the existing 16 KiB `Input` limit, and never let paste content be
  interpreted as commands.
- [ ] Wire GPUI's `InputHandler` for IME. Composition displays at the cursor
  without mutating terminal state until commit; `KeyEvent::composing` becomes
  true only for events genuinely part of a composition.
- [ ] Add focus reporting driven by terminal state.
- [ ] Route application shortcuts before the terminal, with explicit precedence.

### Task 7: Clipboard and OSC 52

**Files:** `sprite-term/src/worker.rs`, `sprite-app/src/terminal_view.rs`

- [ ] Deliver terminal clipboard requests as typed events; the application
  answers with an explicit command. No implicit clipboard access.
- [ ] Enforce the PRD's secure defaults: OSC 52 writes accepted only from the
  focused Pane and only up to 1 MiB decoded; hidden or unfocused writes,
  malformed or oversized payloads, and **all** terminal-initiated reads denied.
- [ ] Test each denial path explicitly. Secure defaults apply whenever
  configuration is absent or invalid.
- [ ] Add base64 decoding to the dependency ledger, or justify hand-rolling it.

### Task 8: Shell integration and observation metadata

**Files:** new `sprite-term/src/shell_integration.rs`, bundled scripts

- [ ] Bundle versioned integration scripts for Bash, Zsh, and Fish and load them
  into launched shells. Never edit or append to a user dotfile.
- [ ] Parse OSC 133 prompt marks into typed events: prompt start, command start,
  command end with exit status.
- [ ] Report the working directory from integration or a pane-scoped OS process
  query. Unavailable metadata stays unknown; never infer it from displayed text.
- [ ] Add title, working directory, and prompt state to `PaneSnapshot` so
  Checkpoint 3 can expose them without a second source of truth.
- [ ] Test with integration disabled, with an unsupported shell, and with a
  deliberately broken script.

### Task 9: Terminal lifecycle metadata

**Files:** `sprite-term/src/worker.rs`

- [ ] Emit title changes, bells, and working-directory changes as typed events.
- [ ] Surface hyperlink metadata on snapshot cells. Allow only configured
  schemes, defaulting to `https` and `http`; `file`, bare paths, and custom
  schemes stay disabled unless explicitly trusted.
- [ ] Opening requires Ctrl+Click on Linux. Pass the parsed URI to the platform
  opener; never build or execute a shell command from terminal-provided text.
- [ ] Test that a hostile label cannot influence what is opened.

### Task 10: Freeze Checkpoint 2 budgets and review

**Files:** `sprite-term/src/bin/sprite-term-bench.rs`, `docs/performance/checkpoint-2.md`, `.github/workflows/phase-1.yml`

- [ ] Add metrics for scrollback capture at the configured maximum, scroll
  cadence over ten seconds, and selection over a full screen. Keep the existing
  metrics so Checkpoint 1's budgets stay comparable.
- [ ] Re-freeze Arch budgets at 110% of p95 from a release build.
- [ ] Run the Croft gate again. From Checkpoint 4 the full matrix is
  merge-blocking; here, any regression in a capability Checkpoint 1 or 2 claims
  blocks acceptance.
- [ ] Re-run the forbidden-state and provenance inspections unchanged.
- [ ] Request human review focused on selection/viewport coherence across
  generations, mouse routing exclusivity, OSC 52 denial paths, and paste safety.

---

## Open questions for review

1. **Does selection belong in `sprite-app` or `sprite-term`?** This draft puts it
   in the app, since it is a presentation concern layered over snapshots. The
   counter-argument is that Checkpoint 3's Pane Observation wants selection state
   in `PaneSnapshot`, which implies the terminal side owns it.
2. **Is GPUI's clipboard sufficient**, or does OSC 52's policy need direct
   platform access? Unknown until Task 7 is attempted.
3. ~~**Scrollback in every snapshot may be too expensive.**~~ **Resolved in
   Task 2: snapshots carry no history.** libghostty already models a viewport
   over the scrollable area, so a bundle reports `Viewport { total_rows, offset,
   visible_rows }` from `Terminal::scrollbar()` and scrolling changes which rows
   the next capture returns. Carrying history would have meant rebuilding tens
   of thousands of rows, each allocating a `String` per cell, many times a
   second. Capture stays proportional to what is visible.
4. **How much of Checkpoint 2 is worth doing before accessibility exists?**
   Selection and viewport state are exactly what an accessibility tree would
   expose. Building them without that consumer risks designing the wrong
   interface.
