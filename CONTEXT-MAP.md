# Context Map

## Contexts

- [Sprite Project](./CONTEXT.md) — project-wide vocabulary: the
  product, its boundaries, and the pane-first identity
- [Terminal Core](./crates/CONTEXT.md) — Sprite Terminal's internal
  vocabulary: sessions, panes, snapshots, observation

## Relationships

- **Sprite Project → Terminal Core**: "Sprite Terminal" in the project
  context is the product whose internals the Terminal Core context describes.
- **Terminal Core's "Croft Compatibility Gate"** refers to what the project
  context calls "Croft (upstream)" in its acceptance-application role.
- Documents written during Phase 1 predate the current vocabulary and use
  "Sprite" to mean Sprite Terminal; they are grandfathered, not errors.
