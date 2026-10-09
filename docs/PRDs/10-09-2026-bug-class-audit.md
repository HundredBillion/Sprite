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
  per live Surface connection (R-S1), and conditionally one cleanup thread
  (R-W6).
- Linux and macOS source compatibility.
- Work happens on `fix/bug-class-audit` in `.worktrees/bug-class-audit`; the
  original checkout stays unchanged. No release, version bump, publishing or
  external messages.
- Every fix is test-first: the regression test fails against `a62247e` and
  passes after the change. Tests cross the real failing seam (worker, socket,
  GPUI view) rather than a mock of it, as earlier audits required.

## Bug classes and structural changes

### C1. An answer is delivered to whoever is first in line (BCA-06, BCA-14)

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
- R-C1.4 `ResolveHyperlink` and its `Hyperlink` answer carry a ticket; the view
  ignores any answer that is not for its latest request.

### C2. The asker gives up but the work still happens (BCA-08)

`relay` times out after 2 s and the endpoint replies "nothing was changed"
while the request is still queued and later applies.

- R-C2.1 Every relayed request carries shared state
  `Waiting | Claimed | Abandoned`. The window applies a request only after it
  atomically moves `Waiting → Claimed`.
- R-C2.2 On timeout the endpoint attempts `Waiting → Abandoned`. If that
  succeeds, the reply "nothing was changed" is true and the window later
  discards the request. If it fails (already claimed), the endpoint waits for
  the window's real answer.

### C3. A confirmation is answered by stale or accidental input (BCA-02, BCA-05, BCA-31)

Holding Ctrl+Shift+W (or close-tab / quit) answers its own confirmation via key
auto-repeat; a held unsafe paste never expires and a later paste sends the old
text unbracketed without reading the clipboard.

- R-C3.1 One `Confirmation<T>` type in `sprite-app`: `arm(subject)`,
  `answer(subject, is_held) -> bool`, `disarm()`. It confirms only for a
  non-repeat press on the same subject.
- R-C3.2 Any other input reaching the pane or workspace (key, mouse, IME
  commit) disarms it, as does focus loss. Disarming happens at the single input
  funnel, not per call site.
- R-C3.3 Close/quit confirmation uses it with the close scope as subject.
- R-C3.4 Unsafe paste uses it with the held text as subject. The confirming
  paste reads the clipboard; if the clipboard text differs from the held text,
  the hold is dropped and the new text goes through the normal safety check.
- R-C3.5 `WorkspaceAction::repeats()` is true only for focus movement, divider
  nudge and font zoom. All other workspace actions ignore `is_held` events
  (BCA-31: holding the split key makes one split).

### C4. Pane state the application holds but never tells the terminal (BCA-01)

No production code sends `TerminalCommand::Focus`, so the worker treats every
pane as unfocused: OSC 52 writes are always denied (violating the Phase-1 PRD's
"accepted only from the focused Pane") and DECSET 1004 focus reports never
reach programs.

- R-C4.1 A pane is focused when its focus handle is focused and its window is
  active. `TerminalView` observes GPUI focus-in/out and window activation and
  sends `Focus(bool)` on each change (deduplicated) and once at session start.
- R-C4.2 The same state drives OSC 52 policy, focus reporting, cursor blinking
  (focused only, R-R2) and a hollow, steady cursor in unfocused panes.

### C5. Untrusted plugin input makes the UI thread block or do unbounded work (BCA-03, BCA-09, BCA-18)

Surface event writes are blocking `write_all` calls on the GPUI thread with a
2 s timeout; one highlight relink scans every stored name (O(N·M)); every
`active_guides` patch rescans all rows.

- R-S1 `SurfaceConnection` no longer exposes the stream to the UI thread. Each
  live connection owns one writer thread and a byte-bounded queue (4 MiB
  pending). `send`, `send_batch` and `establish` only enqueue and never block.
  Exceeding the bound marks the connection dead and shuts the socket down. The
  writer keeps the existing 2 s `SO_SNDTIMEO`; a timeout or failure marks the
  connection dead. Closing or dropping the connection stops the thread; joining
  it never happens on the GPUI thread. Existing ordering (`opened` first,
  contiguous batches and gestures) is preserved because all writes share the
  one queue.
- R-S2 Grid highlights keep a name → id reverse index so relinking one name is
  O(1). A grid holds at most 65,536 defined attribute ids and 65,536 group
  names; a message that would exceed either is refused whole as `Malformed`
  and changes nothing.
- R-S3 The set of guide ids is computed once per `apply_rows` and kept;
  `active_guides` validates against it in O(k).

### C6. Each frame or message recomputes what did not change (BCA-04, BCA-10, BCA-16, BCA-23, BCA-29)

- R-R1 Shaped text is cached per row beside `LayoutCache`: a lazily filled
  per-cell `(drawn foreground, ShapedLine)`. A cell reshapes only when its drawn
  foreground differs from the cached one. The cache is invalidated by any change
  of font family, font size, theme or scale factor. `PositionedCell.text` is a
  `SharedString` created at row layout.
- R-R2 One window-level 530 ms blink clock owned by the workspace notifies only
  the focused pane; input resets the phase as today.
- R-R3 Box drawing strokes use fixed-size arrays, not per-cell `Vec`s (BCA-29).
- R-T1 After handling a message the worker drains already-queued messages
  (bounded: stop at 16 messages or 16 KiB of output, whichever comes first) before capturing, so a burst
  produces one snapshot. On macOS the pump reads until `EAGAIN` or a full
  buffer per permit (BCA-16).
- R-T2 Title and working-directory events within one batch are coalesced to
  the latest of each, as the bell already is (BCA-23).
- R-S4 Element Surface images are cached by a hash of SVG text plus scaled
  pixel size and survive `update`; entries the new description no longer
  references are dropped, so the 16 MiB per-bitmap and 64 MiB per-Surface
  budgets still hold (BCA-10).

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
| BCA-13 | Cursor on a wide character's tail column matches no cell and disappears. | The snapshot maps `at_wide_tail` to the lead column; the cursor draws two cells wide. |
| BCA-14 | Hover link re-requested on every snapshot; every reply notifies. | Request only when the hovered cell or that row's content changes; notify only when the result changes. |
| BCA-17 | Virtual-list rows keyed by index; press and release can hit different rows. | Row element ids derive from the row key. |
| BCA-19 | Config reload resets font zoom and reports a font change. | Zoom is held separately from `settings.font.size`; reload diffs file values and keeps zoom. |
| BCA-20 | An unreadable explicit `--config` silently uses defaults. | An explicit path that cannot be read prints the same complaint reload uses to stderr, then continues with defaults. Discovery of an absent default file stays silent. |
| BCA-21 | Splits smaller than twice the floor snap to 0.5 and cannot move. | The effective floor is `min(floor, extent / 4)`, so small splits move within [0.25, 0.75]. |
| BCA-22 | 1 MiB input backlog equals the 1 MiB paste limit; a near-limit bracketed paste always fails with a misleading error. | The backlog bound is derived as max clipboard bytes plus bracket overhead. |
| BCA-24 | `ForegroundWatch` keeps a duplicate PTY master open after the session ends. | The duplicate closes when the session ends, so the slave hangs up. |
| BCA-25 | PNG scratch buffer grows to the largest image and never shrinks. | The scratch buffer is released after each decode. |
| BCA-26 | Selection anchor is re-resolved in viewport space on each drag event, so it drifts while output scrolls. | The anchor is pinned in `Point::Screen` space at gesture start. |
| BCA-27 | On Darwin a member dying mid-scan aborts the scan, yet escalation advances, consuming TERM and the single HUP. | A vanished member is skipped; escalation advances only after a signal was actually attempted. |
| BCA-28 | Invisible attribute applied before selection inversion; selected hidden text shows no highlight. | Selected hidden cells show the selection background; glyphs stay hidden. |
| BCA-30 | Kitty placeholder tiles are fractionally positioned layout nodes and can seam. | Tiles snap to the same device-pixel grid as cells. |

## Conditional requirement

- R-W6 Pane cleanup waits (`ShutdownHandle::wait`) run on GPUI's background
  executor. First establish from the vendored GPUI source whether the Linux
  executor has a fixed worker count. If it does, move cleanup waits to one
  workspace-owned cleanup thread; if not, record the evidence in the TSP and
  make no change.

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
- Before the PR: `cargo test --workspace --locked --offline`, all-target
  `clippy -D warnings`, `cargo fmt --check`. Native macOS desktop behaviour that
  CI cannot run is listed as unexecuted, not claimed.
- Two ADRs: correlated requests (tickets and claim-or-abandon), and Surface
  writer threads (amending ADR 0018's blocking-write decision).
