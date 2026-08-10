# Separate lifecycle events from latest-only snapshots

Terminal lifecycle events and render snapshots have different delivery rules,
so each Terminal Session exposes a separate bounded stream for each. Ready,
exit, and error events are ordered and lossless. Render snapshots use one
latest-only slot because an intermediate generation is obsolete as soon as a
newer complete generation exists. After the consumer drains that slot, it asks
the terminal owner to capture again only if the generation advanced. This
coalesces both snapshot construction and delivery without polling, while a slow
renderer can never discard a lifecycle event.
