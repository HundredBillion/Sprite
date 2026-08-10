# Test Croft compatibility against moving main

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
