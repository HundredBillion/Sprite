# Make the PTY pump explicitly cancelable

A blocking PTY read does not necessarily finish when the direct child exits,
because a descendant can retain the slave descriptor. Sprite therefore waits on
both PTY readiness and a cancellation socket in its private Unix adapter. The
terminal owner keeps the master descriptor alive until it signals cancellation
and joins the pump. Sprite declares the already-transitive nix 0.28.0 package
directly, requesting poll/process/signal in addition to portable-pty's existing
term/fs requests. Those APIs also support the bounded process-group
HUP/TERM/KILL policy. This is event-driven OS waiting, not periodic polling, and
avoids both an async runtime and a detached reader-thread leak.
