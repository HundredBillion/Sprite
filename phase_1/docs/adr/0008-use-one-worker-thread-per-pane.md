# Use one terminal worker thread per Pane

Each Terminal Session owns one in-process worker thread so libghostty remains on
its required owner thread and Sprite can exchange owned messages without a
second serialization and process-lifecycle layer. Ordinary errors and supervised
worker termination stay pane-local, but Phase 1 accepts that a native memory
fault may terminate the application; safe bindings, fuzzing, and compatibility
tests mitigate that risk instead of a helper process per Pane.
