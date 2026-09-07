# Write PTY input through the pump, never from the worker

The terminal-owner worker used to write input to the PTY master inline, with a
blocking call, from the same thread that applies output and publishes
snapshots. A kernel's PTY input queue is finite — 1024 bytes on macOS — and a
write into a full one blocks until the child reads. A child that has paused
reading while it writes (a line editor re-echoing a long paste, a shell still
starting) then blocks on an output queue that only the worker drains, and the
pane freezes for good: worker on input, child on output, pump on a worker that
no longer takes its chunks. Linux hid the structure behind larger queues and by
discarding an over-long canonical line rather than blocking on it.

Sprite therefore makes the existing PTY pump the pane's single I/O helper in
both directions rather than adding a writer thread. The worker queues input and
returns immediately; the pump waits in one `poll` on PTY readability (only while
it holds an output permit), PTY writability (only while input is queued), a wake
socket the worker pokes, and the cancellation socket, and writes with the master
non-blocking so no call it makes can stall. Queued input is bounded at one
megabyte; beyond that the worker refuses the write and reports it as a session
error the application shows, because that much unread input means the program
has stopped reading and hoarding more would only hide that from the person
typing. This keeps one helper thread per pane (ADR 0008), the pump joinable
without periodic polling (ADR 0011), and input ordered exactly as before.
