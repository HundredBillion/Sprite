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

Levels are `error`, `warn`, `info`, `debug`, and `trace`. An invalid filter falls back to `sprite_app=warn`. Timings at debug level are monotonic elapsed milliseconds: they include scheduling and waiting and are not CPU time. Use a profiler or the existing benchmarks to investigate CPU or memory; log messages alone do not demonstrate a performance improvement. On macOS 26 a value set with `launchctl setenv` does not reach an app the Dock starts. To log a desktop launch, quit Sprite completely first, then start it with `open -a Sprite --env SPRITE_LOG='sprite_app=debug'`: if Sprite is already running, `open` only brings the running copy forward and the value is ignored.

Each event is capped at 4 KiB and the queue holds at most 512 events. Producers do not wait for the writer; when the queue is full, events are dropped and a count is recorded when the writer next succeeds. The worker maintains up to four 1 MiB segments per running process (`sprite.log`, `sprite.1.log`, etc.) and retains up to ten completed runs. Active runs are protected with file locks, so total usage can grow with the number of active processes. Cleanup runs when a logger starts or exits; after an abrupt termination, the next launch can reclaim old runs. A locked, slow, full, or unwritable destination can lose logs or delay cleanup; it does not block terminal activity. Shutdown waits at most 200 ms for the writer, and abrupt termination, process aborts, or native crashes may lose buffered events. Rust panics are recorded best-effort with source location, and the previous panic hook still runs.

Sprite does not deliberately log raw terminal input, terminal output, tokens, credentials, full environment values, protocol bodies, or client-provided request IDs. Treat logs as potentially sensitive nonetheless. Displaying a log inside a pane makes it readable to clients authorized for that pane; filesystem-capable agents may separately read log files. More verbose filtering can enable events from dependencies, so review logs before sharing them.

## Performance events

`SPRITE_LOG='sprite_app::perf=debug' sprite` records where time and resources go during ordinary use. Nothing is recorded at the default level, and the checks cost one filter test per event site when disabled.

| Message | When | Fields |
| --- | --- | --- |
| `pane first output` | The first screen update with text after a pane's program starts | `pane`, `elapsed_ms` |
| `pane closed` | A closed pane's cleanup (hangup, terminate, kill) has finished | `elapsed_ms` |
| `configuration reloaded` | A reload was applied, timed until every pane has followed it, font listing included; a file that does not parse records nothing | `elapsed_ms` |
| `resource sample` | Within one clock beat (530 ms) of the pane count changing, right after each applied reload, otherwise once a minute | `reason` (`panes`, `reload` or `interval`), `panes`, `rss_mb`, `cpu_user_ms`, `cpu_system_ms`, `threads`, `open_files` |
| `slow paint` | One terminal grid took 16 ms or longer to paint, a whole 60 Hz frame | `pass`, `rows`, `elapsed_ms` |

A reload whose file matches the running configuration, and whose configured font families are all installed, is answered without reading the system's font list again, so it records a near-zero `elapsed_ms`. A font with no `family` still reads the list on every reload, because its fallback can change when a font is installed.

A resource field the platform does not report is omitted rather than written as zero. `cpu_user_ms` and `cpu_system_ms` are cumulative for the process, so compare consecutive samples. `pane first output` measures the program as well as Sprite: a shell that spends two seconds in its startup files reports two seconds. To tell the two apart, time the shell outside Sprite, for example `time zsh -i -c exit`.

## Startup events

`SPRITE_LOG='sprite_app::startup=debug' sprite` records how far a window got and when. Every `elapsed_ms` is measured from the same moment, just before the configuration file is read, so consecutive events subtract to give each phase.

| Message | When |
| --- | --- |
| `configuration read` | The configuration file has been read and checked |
| `platform ready` | The windowing platform has started and handed over control |
| `window created` | The native window and its renderer exist; the workspace has not been built |
| `window opened` | The workspace, its first pane and its listeners exist |
| `window activated` | The window has been brought to the front |

The window process also records `application stopped` (`sprite_app::shutdown`, with `elapsed_ms` since launch) as it quits, and writes any queued events before the process ends.

These events show where to look, not whether a change helped. Confirm a refactor with a profiler or the existing benchmarks (`sprite-startup-bench`, `sprite-paint-bench`, `sprite-term-bench`) before and after.
