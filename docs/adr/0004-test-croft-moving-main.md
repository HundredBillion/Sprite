# Keep Croft compatibility testing optional

## Current decision (2026-10-04)

Croft is an optional external smoke test, not a CI job or a required merge,
checkpoint, or release acceptance gate. Building its moving `main` on every PR
adds upstream maintenance work without being necessary for Sprite's development.
This supersedes the mandatory Croft requirements in earlier PRDs and TSPs.

Keep `scripts/test-croft-main.sh` and the ignored `croft_smoke` test available for
manual investigation. The script downloads and builds upstream Croft explicitly;
ordinary offline tests do not run it. Its current checks cover alternate-screen
output, typed text, resize coherence, and shutdown. Opening Croft in Sprite can
also reveal visual problems that the headless smoke test cannot observe, but it
is one optional real-program check rather than a commitment to support Croft's
entire feature set.

## Original decision (superseded)

Sprite's Croft acceptance suite resolves upstream Croft `main` at the start of
every run instead of keeping a permanent pinned baseline. This intentionally
trades day-to-day test reproducibility for immediate compatibility pressure and
lower staleness risk; every run records the exact resolved Croft commit so a
failure can still be reproduced and diagnosed, and Sprite-specific Croft patches
remain forbidden in the acceptance gate.

The moving suite is required for pull requests, merges, checkpoints, release
candidates, and a nightly schedule. Before Checkpoint 4 it is staged to the
capabilities Sprite already claims, while all known missing cases are reported
explicitly. The complete Croft matrix becomes merge-blocking at Checkpoint 4,
when Sprite first claims Kitty graphics and the required richer interactions.
Local `cargo test` remains offline; Croft is an explicit external acceptance
command rather than an implicit test download.
