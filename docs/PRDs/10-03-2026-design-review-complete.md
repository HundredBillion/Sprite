# Complete design review implementation

Source: https://github.com/HundredBillion/Sprite/issues/48 at cee803a.
The user explicitly selected **all three waves in one PR**. The Wave 1 PRD and
TSP remain the first execution segment; their earlier initial scope is superseded
by this document. No release, deployment or merge is requested.

## Outcomes and acceptance

Implement every proposed type-level move (2.1–2.21), the performance proposals
other than the explicitly not-recommended caches, and the seam-based file splits.
Confirm or refute inferred bugs with executable evidence. Preserve the existing
terminal, input, Surface, observation, configuration and shutdown contracts except
where the review explicitly requests a correction (such as rejecting invalid dock
sizes consistently). Ship one branch with independently reviewable commits.

Wave 1 is recorded in its existing documents. The continuation adds:

1. A paint benchmark using real layout and Window-free GridPaint colour/cursor
   resolution with a counting allocator. Commit measured baseline budgets before
   optimising; do not present GPU glyph submission as a headless measurement.
2. RAII output chunks: dropping a message returns its PTY permit and wakes the
   pump, including drop/discard/error paths. Pool the existing 16 KiB buffers so
   steady-state chunk delivery allocates zero buffers. Dropping 40 chunks without
   manual returns must keep delivery live; preserve the command queue reserve.
3. One local_socket owner for private runtime directory, random names, 0600 bind,
   bounded accepts, capped authenticated first-line reads, cancellation and unlink.
   Observation and Surface are protocol adapters with separate grammars. Preserve
   existing protocol-specific limits as named policy values, documenting justified
   differences instead of silently changing authentication or macOS path lengths.
4. Surface routing through PaneHandle, with an actual second (placeholder) adapter
   returning NotATerminal. No workspace downcast. Keep protocol types in their
   existing domain where possible; an associated request type avoids a dependency
   from sprite-pane back to sprite-app. Trait defaults must answer refusals rather
   than silently dropping reply channels. Cycle-focus travels through the seam.
5. Parsed Element and Ownership enums that cannot represent text on a stack,
   grid dimensions on a label, or owned fill with a return target. Parsing retains
   current diagnostics and root-only grid/list restrictions. Shared CellMetrics
   measured in one constructor; snapped geometry and row/column types at geometry
   edges. ValidTerminalSize is the only terminal size admitted to sessions/resizes.
6. Row text and row ownership shared across snapshots and layout. Use compact
   inline text for blanks/single chars and Arc text for graphemes, or shared row
   storage when measurements favour it. Projector reuses clean Arc<RenderRow>s;
   style, selection, palette/default changes and resize invalidate appropriately.
   Layout reuses clean rows and is cached for generation plus hovered span. Palette
   Arc is shared while equal. Image split passes share immutable positioned rows.
7. Pane title updates are pushed. Workspace observation layout publishes only on
   layout mutations, not idle renders. Counts prove publication follows mutations;
   terminal foreground fallback must remain correct when no OSC title exists.
8. Compact Surface grid cells with shared rendered rows, dirty tracking and
   palette/theme invalidation; list configs/visible rows shared, one ID index per
   list operation. One event write per wheel gesture and tuple-based resize dedupe.
9. PaneTree owns each leaf's payload; remove PaneRegistry's parallel map. Preserve
   split geometry, stable IDs, focus, divider semantics and exactly-once shutdown.
10. Raw serde config conversion into validated settings and metrics newtypes,
    canonical palette indexing and typed Settings::diff. Preserve permissive
    recovery/diagnostics for invalid file fields and current defaults. Generated
    property tests prove serialisation round trips and drawable metrics.
11. Split term public types, worker operation seams, workspace interactions and
    wire parsing/serialisation by responsibility as specified in issue section 5.
    Avoid pure forwarding modules and retain stable public re-exports.

## Measurement contract

The paint benchmark measures the Sprite-owned preparation and Window-free
GridPaint::draw decision path used by the live app. It reports first layout,
unchanged-generation/blink, hover and one-row-change cases for 200x60. The unchanged
blink preparation/decision path must allocate zero times after warmup. Window/GPUI
layout, GPU command submission and font shaping are explicitly outside that
headless measurement; do not label those costs zero.

Record allocations per terminal capture as well as timings; prove row sharing
reduces them and a one-row change rebuilds one row. Do not equate an arithmetic
estimate with measurement. Surface cells target <=8 bytes by using row text
references/interned handles with an inline common case; prove the actual layout.
List truncation work is bounded by visible rows plus 16 for a 100k-row model.

## Architectural decisions

Retain deep existing modules, separate grammars and latest-only snapshots. Shared
transport centralises authentication mechanics while explicit adapter policies
preserve their different stream behaviour. Generic pane requests avoid leaking
application protocol types into the lower crate or adding a reverse dependency.

Use serde already resolved in Cargo.lock as a direct dependency for typed raw
configuration; add proptest as a test-only dependency for the requested generated
properties, pin it and fetch before the offline gate. No other dependency is
planned. These are explicitly requested by the complete issue and supersede Wave
1's no-new-dependency constraint for this continuation.

The changes are linked Rust interface migrations within one workspace/binary,
not a live database or network-version rollout. Rebuild every source consumer in
the same branch, preserve wire formats and public re-export paths, and roll back
by reverting the branch. No stored-data backfill or dual-write deployment applies.

## Verification and reporting

Run interface tests per task, independent reviews, and the full locked offline
fmt/clippy/test/build plus forbidden-state gates before PR creation. Use
TERM=xterm-ghostty for command-session tests; inherited TERM=dumb caused the two
initial tmux failures and both passed unchanged with the correct identity.
Run new benchmark budget checks and record hardware/build profile/sample counts.
Report any unverified native macOS or visual UI coverage explicitly. No claims of
zero allocations outside measured seams, no issue checkbox marked done on code
inspection alone. The final PR accounts for every issue item with its proof.
