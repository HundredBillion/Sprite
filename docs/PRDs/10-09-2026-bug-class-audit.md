# Eliminate the bug classes found in the October whole-repository review

## Outcome

Repair every confirmed finding from the read-only whole-repository review of
`master` at `a62247e` (2026-10-08), and where a finding is one instance of a
recurring failure class, change the owning structure so the class cannot recur.
Each finding below has an ID (BCA-nn) that tests, the TSP and the PR cite.

Most findings are long-standing: BCA-01, 05, 06, 07, 08 and 11 predate the
repository split (`e69d28b`); BCA-09 dates from the first Surface socket
(`bb53506`); BCA-15 is a regression introduced by the previous audit's fix
(`8010540`, which replaced "skip a failed accept" with "stop the listener").

## Constraints

- Rust 1.97.1, pinned dependencies, no new crates, no async runtime.
- One libghostty-owning worker thread per Terminal Session (ADR 0008).
- Surface Channel wire protocol version 1, its authentication, event ordering
  and existing refusal semantics are unchanged on the wire.
- **New threads are allowed only where this PRD names them:** one writer thread
  per live Surface connection (R-S1), and one short-lived thread per pane
  cleanup in progress (R-W6).
- Linux and macOS source compatibility.
- Work happens on `fix/bug-class-audit` in `.worktrees/bug-class-audit`; the
  original checkout stays unchanged. No release, version bump, publishing or
  external messages.
- Every fix is test-first: the regression test fails against `a62247e` and
  passes after the change. Tests cross the real failing seam (worker, socket,
  GPUI view) rather than a mock of it, as earlier audits required.

## Bug classes and structural changes

### C1. An answer is delivered to whoever is first in line (BCA-06)

Observation pairs `TerminalEvent::History` and failures with waiters by arrival
order, and every `TerminalEvent::Error` — including unrelated selection, key
encoding and write errors — fails the oldest waiter, shifting every later answer.

- R-C1.1 `TerminalCommand::CaptureHistory` carries a `Ticket`; the answers are
  `TerminalEvent::History { ticket, snapshot }` and
  `TerminalEvent::HistoryFailed { ticket, error }`.
- R-C1.2 `WindowPanes` keeps waiters keyed by ticket. An answer reaches only
  the waiter holding that ticket; an answer for an unknown or expired ticket is
  dropped. A waiter that times out removes its own ticket.
- R-C1.3 A general `TerminalEvent::Error` updates the status line only and can
  never resolve a waiter.
- R-C1.4 `ResolveHyperlink` already carries a `request_id` and is the model
  for R-C1.1; it is unchanged except as BCA-14 requires.

### C2. The asker gives up but the work still happens (BCA-08)

`relay` times out after 2 s and the endpoint replies "nothing was changed"
while the request is still queued and later applies.

- R-C2.1 Every relayed request carries shared state
  `Waiting | Claimed | Abandoned`. The window applies a request only after it
  atomically moves `Waiting → Claimed`.
- R-C2.2 On timeout the endpoint attempts `Waiting → Abandoned`. If that
  succeeds, the reply "nothing was changed" is true and the window later
  discards the request. If it fails (already claimed), the endpoint waits up
  to 10 s more for the window's real answer; if none arrives it replies "the
  window accepted this request and is still applying it". No timeout path ever
  says "nothing was changed" about a claimed request.

### C3. A confirmation is answered by stale or accidental input (BCA-02, BCA-05, BCA-31)

Holding Ctrl+Shift+W (or close-tab / quit) answers its own confirmation via key
auto-repeat; a held unsafe paste never expires and a later paste sends the old
text unbracketed without reading the clipboard.

- R-C3.1 One `Confirmation<T>` type in `sprite-app`: `arm(subject)`,
  `answer(subject, is_held) -> bool`, `disarm()`. It confirms only for a
  non-repeat press on the same subject.
- R-C3.2 Deliberate input disarms it: any non-modifier key press other than
  the confirming gesture, any mouse button press, an IME text commit, and loss
  of Pane Focus. Mouse motion, hover, wheel/scroll, modifier-only presses,
  resizes and terminal output do not. Disarming is called from the key, mouse
  and IME handlers — not from `TerminalView::send`, which also carries hover
  lookups and resizes.
- R-C3.3 Close/quit confirmation uses it with the close scope as subject.
- R-C3.4 Unsafe paste uses it with the held text as subject. The confirming
  paste reads the clipboard; if the clipboard text differs from the held text,
  the hold is dropped and the new text goes through the normal safety check;
  if the clipboard cannot be read, the hold is dropped and nothing is pasted.
- R-C3.5 `WorkspaceAction::repeats()` is true only for focus movement, divider
  nudge and font zoom. All other workspace actions ignore `is_held` events
  (BCA-31: holding the split key makes one split).
  Known limit: a key consumed by a workspace binding (e.g. font zoom) never
  reaches the pane and so does not drop a held paste.

### C4. Pane state the application holds but never tells the terminal (BCA-01)

No production code sends `TerminalCommand::Focus`, so the worker treats every
pane as unfocused: OSC 52 writes are always denied (violating the Phase-1 PRD's
"accepted only from the focused Pane") and DECSET 1004 focus reports never
reach programs.

- R-C4.1 A pane is focused when its focus handle is focused and its window is
  active. `TerminalView` observes GPUI focus-in/out and window activation and
  sends `Focus(bool)` on each change (deduplicated) and once at session start.
  A `Focus` refused by a full queue is retained as the latest desired value
  and resubmitted when admission recovers (Command Admission), never dropped.
- R-C4.2 The same state drives OSC 52 policy, focus reporting, cursor blinking
  (focused only, R-R2) and a hollow, steady cursor in unfocused panes.

### C5. Untrusted plugin input makes the UI thread block or do unbounded work (BCA-03, BCA-09, BCA-18)

Surface event writes are blocking `write_all` calls on the GPUI thread with a
2 s timeout; one highlight relink scans every stored name (O(N·M)); every
`active_guides` patch rescans all rows.

- R-S1 `SurfaceConnection` no longer exposes the stream to the UI thread. Each
  live connection owns one writer thread and a pending queue that admits an
  event only while fewer than 4 MiB are already waiting (the bound is 4 MiB
  plus one event, so a single large paste behaves as today). `send`,
  `send_batch` and `establish` only enqueue and never block. Reaching the
  bound marks the connection dead and shuts the socket down. The
  writer keeps the existing 2 s `SO_SNDTIMEO`; a timeout or failure marks the
  connection dead. Closing or dropping the connection stops the thread; joining
  it never happens on the GPUI thread. A graceful close first lets the writer
  send what is queued (including `closed`) within the write timeout. Existing ordering (`opened` first,
  contiguous batches and gestures) is preserved because all writes share the
  one queue.
- R-S2 Grid highlights keep a name → id reverse index so relinking one name is
  O(1). A grid holds at most 262,144 defined attribute ids and 262,144 group
  names; a message that would exceed either is refused whole as `Malformed`
  and changes nothing.
- R-S3 The set of guide ids is computed once per `apply_rows` and kept;
  `active_guides` validates against it in O(k).

### C6. Each frame or message recomputes what did not change (BCA-04, BCA-10, BCA-16, BCA-23, BCA-29)

- R-R1 Shaped text is cached per row beside `LayoutCache`: a lazily filled
  per-cell slot naming its drawn foreground and a shape held in a per-pane pool
  of shared shapes capped at 4,096 entries (a GPUI `ShapedLine` is about 3 KB,
  so one per cell would cost tens of MB per pane). A cell reshapes only when its drawn
  foreground differs from the cached one. The cache is invalidated by any change
  of font family, font size, theme or scale factor. A laid-out cell keeps its
  compact `CellText`; the text system's `SharedString` is built only when a
  shape is built, on a shape-cache miss.
- R-R2 One 530 ms clock per Sprite Window replaces the per-pane timers. Each
  tick, every pane refreshes its Pane Title discovery and notifies only if the
  title changed; only the pane with Pane Focus toggles its blink phase and
  repaints. Unfocused panes draw a steady cursor: a block cursor becomes a
  hollow block, bar and underline keep their shape. A grid Surface's cursor
  follows the same rule. (No blink-phase reset on input exists today and none
  is added.)
- R-R3 Box drawing strokes use fixed-size arrays, not per-cell `Vec`s (BCA-29).
- R-T1 After handling a message the worker drains already-queued messages
  (bounded: stop at 16 messages or 16 KiB of output, whichever comes first) before capturing, so a burst
  produces one snapshot. The pump reads until `EAGAIN` or a full buffer per
  permit on every platform (one extra non-blocking read per wake on Linux,
  which lets Linux CI test it) (BCA-16).
- R-T2 Title and working-directory events within one batch are coalesced to
  the latest of each, as the bell already is (BCA-23).
  Tests that used title floods to create event pressure move to OSC 52
  clipboard writes, the only parser event that is never coalesced.
- R-S4 Element Surface images are cached by their full SVG text (the decode
  does not depend on pixel size) and survive `update`; entries the new
  description no longer references are dropped, so the 16 MiB per-bitmap and
  64 MiB per-Surface budgets still hold (BCA-10). The text is looked up once
  per description; later frames find each image's decode by its tree
  position.

### C7. Registry state coupled to endpoint lifetime (BCA-11)

- R-W1 Panes register with `WindowPanes` at creation regardless of whether an
  observation endpoint exists; enabling observation by reload exposes every
  existing pane.

### C8. A background loop dies silently (BCA-15)

- R-L1 The local-socket accept loop never exits on an accept error. Transient
  errors (`EMFILE`, `ENFILE`, `ECONNABORTED`, `ENOBUFS`, `ENOMEM`) wait about
  100 ms in the cancellable `poll` and retry; no error busy-spins. Only
  cancellation ends the loop.

## Local fixes

| ID | Defect | Required behaviour |
|---|---|---|
| BCA-07 | `Transformations::ALPHA` turns Grayscale into GrayscaleAlpha (2 B/px); libghostty requires RGBA8, so every grayscale Kitty PNG is rejected. | The decoder emits RGBA8 for every PNG colour type and bit depth; an exhaustive colour-type × bit-depth test proves it. |
| BCA-12 | Faint (SGR 2) is recorded but never drawn. | Faint foreground draws at 50% alpha (Ghostty's default `faint-opacity`). |
| BCA-13 | Cursor on a wide character's tail column matches no cell and disappears. | The snapshot maps `at_wide_tail` to the lead column; the cursor is drawn over the wide cell, which already spans two columns. |
| BCA-14 | Hover link re-requested on every snapshot; every reply notifies. | Request only when the hovered cell or that row's content changes; notify only when the result changes. |
| BCA-17 | Virtual-list rows keyed by index; press and release can hit different rows. | Row element ids derive from the row key. |
| BCA-19 | Config reload resets font zoom and reports a font change. | Zoom is held separately from `settings.font.size`; reload diffs file values and keeps zoom. |
| BCA-20 | An unreadable explicit `--config` silently uses defaults. | An explicit path that cannot be read prints the same complaint reload uses to stderr, then continues with defaults. Discovery of an absent default file stays silent. |
| BCA-21 | Splits smaller than twice the floor snap to 0.5 and cannot move. | The floor is `min(120 px, extent / 4)`, so a split's travel grows with its size and no split is pinned; splits of 480 px and wider are unchanged. |
| BCA-22 | 1 MiB input backlog equals the 1 MiB paste limit; a near-limit bracketed paste always fails with a misleading error. | The backlog bound is derived as max clipboard bytes plus bracket overhead. |
| BCA-24 | `ForegroundWatch` keeps a duplicate PTY master open after the session ends. | The duplicate closes when the session ends, so the slave hangs up. |
| BCA-25 | PNG scratch buffer grows to the largest image and never shrinks. | The scratch buffer is released after each decode. |
| BCA-26 | Selection anchor is re-resolved in viewport space on each drag event, so it drifts while output scrolls. | The press that starts a selection gesture is marked as the start; the worker keeps the anchor as a libghostty `TrackedGridRef` for the rest of the gesture, so it follows its content through scrolling and scrollback eviction. If the anchored content is evicted, the selection is cleared rather than re-anchored elsewhere. The gesture starts with a new `BeginSelection { anchor }` command sent on press. Known gap: with zero scrollback libghostty rotates rows instead of evicting a page, so loss is never reported and the anchor can move to the next line. |
| BCA-27 | On Darwin a member dying mid-scan aborts the scan, yet escalation advances, consuming TERM and the single HUP. | A vanished member is skipped; escalation advances only after a signal was actually attempted. |
| BCA-28 | Invisible attribute applied before selection inversion; selected hidden text shows no highlight. | Selected hidden cells show the selection background; glyphs stay hidden. |
| BCA-30 | Kitty placeholder tiles are fractionally positioned layout nodes and can seam. | Tiles snap to the same device-pixel grid as cells. |

## Pane cleanup off the shared executor (BCA-32)

- R-W6 Pane cleanup (HUP/TERM/KILL escalation and joins, up to several
  seconds) currently blocks a GPUI background-executor thread. On Linux that
  executor is a fixed pool of `available_parallelism()` threads
  (`vendor/gpui/src/platform/linux/dispatcher.rs:30`), so closing or quitting
  with as many busy panes as cores starves all other background work. Each
  pane's cleanup runs on its own short-lived named thread and reports
  completion through a channel that the existing pending-cleanup tracking
  awaits. Cleanups stay parallel, so quit time does not grow with pane count;
  threads exist only while a pane is shutting down. No blocking cleanup runs on
  the GPUI thread or the shared background executor.

## Explicitly out of scope

- Cached GPUI views per pane: with R-R1 an unchanged pane's repaint is cheap.
  Revisit only if the count tests or timings show otherwise.
- Divider-drag per-move layout work: cheap at realistic pane counts.
- Kitty Unicode-placeholder implicit row/column inheritance: a spec gap, not a
  defect in implemented behaviour.

## Evidence

- Gate: deterministic regression tests per ID, plus count-based tests at
  Sprite's single `shape_line` call site and worker capture point: an unchanged
  frame shapes nothing; a blink-only frame shapes at most one cell; a one-row
  change shapes only that row; font/theme change reshapes all; a blink tick
  notifies only the focused pane; 16 queued chunks produce one snapshot.
- Record (not enforced): `sprite-paint-bench` gains a mode that includes
  shaping through the platform text system, run before and after, written to
  `docs/performance/`. If the platform text system cannot be built headless,
  the record says timing was not measured; no speed-up is claimed without it.
  (Established while planning: GPUI's text systems are crate-private and the
  test platform's is a no-op, so shaping is counted, not timed.) Paint
  allocation budgets are re-frozen by the documented 30-sample procedure,
  because layout now allocates once per non-ASCII cell while paint stops
  allocating per glyph per frame.
- Before the PR: `cargo test --workspace --locked --offline`, all-target
  `clippy -D warnings`, `cargo fmt --check`. Native macOS desktop behaviour that
  CI cannot run is listed as unexecuted, not claimed.
- ADRs: 0028 (correlate answers by ticket; claim before applying) and 0029
  (dedicated threads for Surface writes and pane cleanup, amending ADR 0018's
  blocking write and ADR 0024's executor choice).

## Grilling decisions (2026-10-09)

- 2026-10-09: R-R1 revised after whole-branch review — the text-system string
  is built on a shape-cache miss rather than at row layout, removing a
  per-non-ASCII-cell allocation.
- BCA-21 revised to a smooth floor after review found the piecewise rule
  pinned 240–280 px splits (2026-10-09).
- Cleanup threads: one short-lived thread per pane cleanup, not one serial
  thread — quit time must not become the sum of per-pane deadlines.
- Pane Focus requires the Sprite Window to be active (glossary term added);
  a background app switch denies OSC 52 and sends focus-out, as other terminals do.
- Confirmations disarm on deliberate input only (keys, button presses, IME
  commits, focus loss) — never on hover, motion, scroll or resize, which also
  pass through `TerminalView::send`.
- Claimed relays wait at most 10 s more and then report "still applying".
- Selection anchors use libghostty `TrackedGridRef`, because `Point::Screen`
  still drifts once full scrollback evicts lines.
- Highlight caps are 262,144 entries each: sprite.nvim forwards Neovim's
  monotonically allocated attribute ids, and a refusal mid-session would
  break that grid's highlighting.
- The window clock still drives title discovery for every pane; only the
  focused pane's blink repaints.
- Hyperlink resolution already carries a request id; only BCA-14's
  request-on-change remains.
