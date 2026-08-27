# Use one terminal-owner worker thread per Pane

Each Terminal Session owns one in-process terminal-owner worker so libghostty
remains on its required thread and Sprite can exchange owned messages without a
second serialization and process-lifecycle layer. Because `portable-pty` exposes
a blocking reader, each Pane also owns one small I/O-pump thread that copies PTY
bytes into the bounded worker queue but never touches libghostty or terminal
state. The pump blocks in Unix readiness on both the PTY and a cancellation
socket, so it remains joinable when a descendant keeps the PTY open without
periodic polling or an async runtime. Sixteen standard-library output permits
reserve one slot in the 17-slot worker queue for commands and lifecycle work, so
sustained output cannot win every newly freed slot. One small child-waiter
thread blocks until the child exits and returns one owned status, so quiet exits
do not rely on PTY EOF. Both helpers use explicitly small stacks and never own
terminal state.
Ordinary errors and supervised worker termination stay pane-local, but Phase 1
accepts that a native memory fault may terminate the application; safe bindings,
fuzzing, and compatibility tests mitigate that risk instead of a helper process
per Pane.
