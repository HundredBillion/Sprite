# Separate Render Snapshots from Pane Snapshots

One coherent Terminal Core generation produces two owned projections: a rich
internal Render Snapshot for GPUI and a reduced text-focused Pane Snapshot for
Pane Observation. `sprite-app` enriches the latter with tab and layout metadata;
neither projection is derived from pixels or converted from the other, so the
stable shell-facing JSON does not expose graphics internals or freeze the
renderer interface.

Pane Snapshots may describe Kitty placements that intersect the returned range
using identity, format, dimensions, cell bounds, and z-order, but never contain
image bytes, pixels, filenames, or inferred content.
