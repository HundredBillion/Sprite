# Local application diagnostics

Sprite writes window-process diagnostics to private files. CLI commands retain their normal stdout and stderr; diagnostic events never go to stdout or pane-observation replies. The default filter records warnings and errors from Sprite. Events include a Unix timestamp in milliseconds, severity, subsystem and process ID; pane and connection IDs are included where available.

On macOS the directory is `~/Library/Logs/Sprite`. On Linux it is `$XDG_STATE_HOME/sprite/logs` when `XDG_STATE_HOME` is an absolute path, otherwise `~/.local/state/sprite/logs`. Each window process creates a directory named `run-PID-TIMESTAMP` with a `sprite.log` file. For example, choose the right PID from `pgrep -fl sprite`, then follow its file:

```sh
# macOS; on Linux set this to the Linux path described above
logroot="$HOME/Library/Logs/Sprite"
ls -td "$logroot"/run-*/
tail -F "$logroot"/run-PID-TIMESTAMP/sprite.log
```

Replace `PID-TIMESTAMP` with the directory shown by `ls`. `tail -F` follows the active filename across rotation on macOS and GNU/Linux. To filter existing logs, for example, use `grep 'subsystem=sprite_app::ipc' "$logroot"/run-PID-TIMESTAMP/sprite.log`. Several Sprite processes have separate logs; use the PID to select the one being investigated.

Set `SPRITE_LOG` before launching a window for more detail. This uses tracing-subscriber filter directives:

```sh
SPRITE_LOG='sprite_app=info,sprite_app::ipc=debug,sprite_app::startup=debug' sprite
```

Levels are `error`, `warn`, `info`, `debug`, and `trace`. An invalid filter falls back to `sprite_app=warn`. Timings at debug level are monotonic elapsed milliseconds: they include scheduling and waiting and are not CPU time. Use a profiler or the existing benchmarks to investigate CPU or memory; log messages alone do not demonstrate a performance improvement. For a macOS Dock launch, `launchctl setenv SPRITE_LOG 'sprite_app=debug'` sets the value for future GUI launches in the current login session; remove it afterwards with `launchctl unsetenv SPRITE_LOG`.

Each event is capped at 4 KiB and the queue holds at most 512 events. Producers do not wait for the writer; when the queue is full, events are dropped and a count is recorded when the writer next succeeds. The worker maintains up to four 1 MiB segments per running process (`sprite.log`, `sprite.1.log`, etc.) and retains up to ten completed runs. Active runs are protected with file locks, so total usage can grow with the number of active processes. Cleanup runs when a logger starts or exits; after an abrupt termination, the next launch can reclaim old runs. A locked, slow, full, or unwritable destination can lose logs or delay cleanup; it does not block terminal activity. Shutdown waits at most 200 ms for the writer, and abrupt termination, process aborts, or native crashes may lose buffered events. Rust panics are recorded best-effort with source location, and the previous panic hook still runs.

Sprite does not deliberately log raw terminal input, terminal output, tokens, credentials, full environment values, protocol bodies, or client-provided request IDs. Treat logs as potentially sensitive nonetheless. Displaying a log inside a pane makes it readable to clients authorized for that pane; filesystem-capable agents may separately read log files. More verbose filtering can enable events from dependencies, so review logs before sharing them.
