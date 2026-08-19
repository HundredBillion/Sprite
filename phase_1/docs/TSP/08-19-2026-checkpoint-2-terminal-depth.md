# Sprite Terminal Checkpoint 2 Technical Spec

> **Status: implemented on Linux, not reviewed.** Tasks 1-10 are complete
> except for the deferrals each records. Human review is still owed.
>
> **Original status: DRAFT — not reviewed.** Checkpoint 1's TSP contained two commands
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

- [x] Add a `Scroll` command carrying an ordered row delta. Test that it changes
  which rows a snapshot reports without disturbing the child. (Landed in Task 2.)
- [x] Hold fractional offset as application state layered over libghostty's
  row-based viewport. `ScrollAccumulator` keeps the sub-row remainder so trackpad
  gestures accumulate instead of being rounded away, and emits whole-row
  `Scroll::Delta` commands.
- [x] A viewport at the live bottom follows new output. A viewport reading older
  scrollback stays anchored and reports an unseen-line count. libghostty already
  anchors; the test pins the behaviour so a future change cannot lose it.
- [x] Keyboard input returns the Pane to the live bottom; raw `Input` does not,
  being a transport rather than a keystroke. Both tested.
- [ ] **Unverified by machine:** the GPUI wheel-to-accumulator wiring. The
  accumulator has unit tests and `Scroll` has integration tests, but no scroll
  injection tool is available here, so the few lines joining them have only been
  read, not exercised. Confirm by hand, or with a compositor that can synthesise
  wheel events.
- [ ] Paste returns to the bottom — deferred with paste itself to Task 6.
- [ ] Selection and search preserve the viewport — deferred to Task 4, which
  introduces selection.

### Task 4: Select and copy

**Files:** new `sprite-app/src/selection.rs`, `sprite-term/src/snapshot.rs`

- [x] Model selection in viewport cell coordinates. `Select { anchor, head,
  mode, rectangle }` and `ClearSelection` install it through libghostty, which
  anchors it over the whole screen including scrollback.
- [x] Support character, word, and line granularity, delegating word and line
  boundaries to libghostty so Sprite agrees with Ghostty on what a word is.
- [x] Render the selection overlay from snapshot state: `RenderCell::selected`
  comes from the same traversal as the text, so there is no second copy.
- [x] Copy yields the selected text through `SelectionCopied`, extracted by
  `format_selection` because the terminal is what knows which rows were
  soft-wrapped. Empty when nothing is selected. User-initiated copy writes
  straight to the clipboard; OSC 52 policy remains Task 7.
- [ ] **Not yet driven by gestures.** Selection is complete at the seam and
  rendered, but nothing produces the commands: mouse drag arrives in Task 5.
  Until then it is reachable only through the API.
- [ ] Table-test word boundaries against wide characters and combining marks.

### Task 5: Mouse

**Files:** `sprite-term/src/worker.rs`, `sprite-app/src/input.rs`

- [x] Add an owned `Mouse` command: position in cells, button, action,
  modifiers. `sprite-term` encodes it through libghostty against live mouse
  modes; no encoded bytes cross the seam. Position is carried in cells rather
  than pixels so a font or scale change cannot desynchronise the two sides.
- [x] Test that a child enabling mouse reporting receives events, and that one
  disabling it does not. Both assert against the encoded bytes the child reads
  back, not against internals.
- [x] Exclusivity: Terminal Core alone decides, returning `None` when the child
  is not reporting or the override is held. `RenderSnapshot::mouse_tracking`
  tells the application whether a drag is its own gesture, but never decides
  delivery — both sides read the same terminal state. A test asserts a child
  that never enabled reporting receives nothing at all.
- [x] Drag-to-select wired: press anchors, motion extends, release copies.
- [ ] **Unverified by machine:** drag-to-select in the window. The hit test and
  routing have unit and integration tests, but no mouse-injection tool is
  available here, so the GPUI listeners themselves have been read, not
  exercised. Same gap as Task 3's scroll wheel, which turned out to work.
- [ ] Make the override modifier configurable. It is Shift, but hardcoded;
  configuration arrives with the TOML work.

### Task 6: Complete the key protocol

**Files:** `sprite-term/src/worker.rs`, `sprite-app/src/input.rs`

- [x] Kitty keyboard flags **already worked** — the encoder refreshes from
  terminal state before every encode, so negotiation needed no code. A test now
  pins it: `a` is `61` legacily and `1b 5b 39 37 75` once the child sends
  `CSI > 8 u`.
- [x] Bracketed paste, chunked through the 16 KiB write bound. libghostty's
  `paste::encode` does the dangerous part: it strips control bytes, so a payload
  containing `ESC [ 201 ~` cannot close the bracket early and inject a command.
  Tested with exactly that payload.
- [x] Focus reporting driven by terminal state: `CSI I` reaches only a child
  that enabled mode 1004.
- [x] Route application shortcuts before the terminal, with explicit precedence.
  Ctrl+Shift+C and Ctrl+Shift+V are the whole table; everything else is the
  child's, and a claimed binding is never also typed.
- [ ] **IME is not done.** GPUI's `InputHandler` wiring is substantial and
  deserves its own pass rather than being tacked onto this task.
  `KeyEvent::composing` therefore remains always false, as Checkpoint 1 left it.
- [ ] **Paste protection is owed.** An unbracketed paste containing newlines
  still executes, and no conversion can prevent it: Sprite writes a carriage
  return, but the line discipline rewrites it back to a newline unless `icrnl`
  is off. That is inherent to terminals and is why bracketed paste exists.
  `paste::is_safe` is available and detects exactly this case; what is missing
  is the confirmation step before performing such a paste.

### Task 7: Clipboard and OSC 52

**Files:** `sprite-term/src/worker.rs`, `sprite-app/src/terminal_view.rs`

- [x] Accepted clipboard writes arrive as a typed `ClipboardWrite` event and the
  application performs them. Terminal Core never touches the system clipboard.
- [x] Secure defaults enforced and each denial path tested: unfocused panes
  denied, payloads over 1 MiB denied, non-text representations denied, and the
  selection clipboard governed by the same policy.
- [x] **All terminal-initiated reads denied** — by libghostty, which drops OSC 52
  `?` requests before they reach any callback. A test pins that nothing is
  written back in answer to one.
- [x] **No base64 dependency needed.** libghostty delivers clipboard content
  already decoded and binary-safe, so there is nothing to add to the ledger.
- [x] Focus defaults to *denied* rather than focused. A child can emit OSC 52 the
  instant it starts, so a pane whose focus the application has not yet declared
  must not be able to write. Found while testing: the original default of
  focused made two denial tests race the child's first output.

### Task 8: Shell integration and observation metadata

**Files:** new `sprite-term/src/shell_integration.rs`, bundled scripts

- [~] Versioned scripts for Bash, Zsh, and Fish exist in
  `crates/sprite-term/shell-integration/` and no user dotfile is touched. Only
  the Bash script's syntax has been checked — zsh and fish are not installed
  here, so those two are **unverified**.
- [x] OSC 133 prompt marks are reported per row on `PaneRow::prompt`, rather
  than as events. libghostty already tracks them per row, so an observer can
  tell a prompt from its output without parsing text and without a second
  source of truth. Command exit status is **not** yet surfaced.
- [x] Working directory reported from OSC 7, and title from OSC 2, both as
  snapshot fields and typed events. A pane-scoped OS process query as a fallback
  is not implemented; without integration the value stays `None`.
- [x] Unavailable metadata stays unknown. A test asserts that a shell which says
  nothing yields no title, no directory, and no row claiming to be a prompt.
- [x] Bell arrives as a typed event rather than a character to notice in text.
- [ ] **Automatic loading is not implemented**, and deliberately so. Each shell
  needs a different mechanism — zsh a generated `ZDOTDIR` that re-sources the
  real one, fish a vendor conf directory, bash no clean interactive hook at all
  — and getting any of them wrong leaves someone without their shell
  configuration. Sprite exports `SPRITE_SHELL_INTEGRATION_DIR` and stops there.
- [ ] Test with integration disabled, with an unsupported shell, and with a
  deliberately broken script. Only the "says nothing" case is covered so far.

### Task 9: Terminal lifecycle metadata

**Files:** `sprite-term/src/worker.rs`

- [x] Title, bell, and working-directory changes emit as typed events (landed
  with Task 8).
- [x] Hyperlinks are **resolved on demand rather than carried on snapshot
  cells.** A link lookup is per cell, so resolving a full screen every capture
  would mean thousands of calls a second for information almost never used.
  `ResolveHyperlink` asks about one cell and `Hyperlink` answers.
- [x] Only `https` and `http` are allowed. `file:`, bare paths, `data:`,
  `javascript:`, and application schemes are refused, and a refusal is
  indistinguishable from "no link" so a caller cannot act on the difference.
  Nine cases table-tested.
- [x] Opening requires Ctrl+Click. The parsed URI goes straight to the platform
  opener; Sprite never builds a command line from terminal-provided text.
- [x] A hostile label cannot influence what is opened: the test uses a link
  whose visible text impersonates a bank while targeting somewhere else, and
  asserts the parsed target is what resolves.

### Task 10: Freeze Checkpoint 2 budgets and review

**Files:** `sprite-term/src/bin/sprite-term-bench.rs`, `docs/performance/checkpoint-2.md`, `.github/workflows/phase-1.yml`

- [x] Three metrics added — `capture_with_full_scrollback`, `scroll_round_trip`,
  `select_full_screen` — alongside all five from Checkpoint 1, so the old budgets
  stay comparable. Scroll is measured as a command-to-snapshot round trip rather
  than a ten-second cadence, which is the same property in a form that does not
  take ten seconds per sample.
- [x] Arch budgets re-frozen at 110% of p95, in `docs/performance/checkpoint-2.md`
  and `checkpoint-2-arch.json`.
- [x] **A regression was found and mostly fixed.** Adding `is_selected()` per
  cell cost ~1,900 FFI calls per capture and pushed three metrics over
  Checkpoint 1's budgets. Skipping the query while nothing is selected recovered
  idle latency from 0.168 to 0.147 ms. Two metrics remain marginally over (3.5%
  and 0.6%) and are accepted with reasons recorded.
- [x] Croft gate re-run and **passed** against upstream `cf805f29`, after a
  checkpoint that substantially changed the rendering path.
- [x] Forbidden-state and provenance inspections re-run, both clean.
- [ ] **Human review still owed**, focused on selection/viewport coherence across
  generations, mouse routing exclusivity, OSC 52 denial paths, and paste safety.
  Outstanding for Checkpoint 1 as well.

---

## Open questions for review

1. ~~**Does selection belong in `sprite-app` or `sprite-term`?**~~ **Resolved in
   Task 4: `sprite-term`, against this draft's guess.** libghostty already
   implements `select_word`, `select_line`, `select_output` (semantic, OSC
   133-aware), `select_all`, and `format_selection`, which rejoins soft-wrapped
   rows. Decisively, the render iterator's `is_selected()` reports a cell as
   selected only when the *terminal* holds the selection, so an application-side
   model could not have used the existing render path at all. Putting it in the
   app would have meant reimplementing word boundaries and wrap handling, and
   diverging from Ghostty on both.
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
