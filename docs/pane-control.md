# Pane control

Any program you run can list, read, create, label, move, focus, resize, close
and type into Sprite panes, in every Sprite window, with no prompt and no
access file. This includes panes a person is using, panes in background tabs,
and panes hosting native Surfaces. An orchestrator such as an agent manager uses
this to drive the panes it works in.

## Who can use it

Each window listens on a local socket:

- macOS: `$TMPDIR/sprite/window-<window>.sock`
- Linux: `$XDG_RUNTIME_DIR/sprite/window-<window>.sock`

The directory is mode 0700 and the socket 0600, so only processes running as
your user can connect. That is the whole check: there is no key and no Allow
button. Anything running as you can drive every pane, including typing commands
into a shell, just as it could already run those commands itself.

`<window>` is 24 hex characters, chosen at random when the window opens. A
pane's address is `<window>:<pane>`, and a tab's is `<window>:t<tab>`. A
restarted window gets a new id, so an old address never reaches a new pane.

Every pane's session gets `SPRITE_PANE=<window>:<pane>`, so a program knows
which pane and window it is running in.

## Use the CLI

Requests print JSON on stdout; diagnostics go to stderr. The examples use `jq`
to pull out addresses.

```sh
sprite panes list
human=$(sprite panes list | jq -r '.result.panes[0].id')
pane=$(sprite panes create --beside "$human" --side right | jq -r '.result.id')
sprite panes read --pane "$pane" --lines 100
sprite panes rename --pane "$pane" --label 'Agent work'
sprite panes resize --pane "$human" --side right --pixels 400
sprite panes move --pane "$pane"
sprite panes focus --pane "$pane"
sprite panes input --pane "$pane" --text 'echo hello'
sprite panes input --pane "$pane" --key enter
sprite panes close --pane "$pane"
```

Which window a command talks to:

- `panes list` asks every running window and merges their panes into one
  answer. Sockets left by a window that was killed are skipped.
- A command that names a pane, or `create --beside`, talks to that pane's
  window.
- `panes create` without `--beside`, `config reload` and `config print` use the
  window named by `--window <id>`, else the window in `SPRITE_PANE`, else the
  only window running. With several running and neither given, they fail and ask
  for `--window`.

`create` without `--beside` opens a new tab. `move` without `--beside` moves the
pane to a new tab. With `--beside ADDRESS --side left|right|up|down`, each works
beside the addressed pane. Moves keep the pane's identity, Terminal Session,
content and Surface connections. They don't switch to a background tab; if the
selected tab disappears, the usual tab fallback applies. Moving a pane beside
itself is invalid. An empty tab disappears as usual. Moving a pane to another
window returns `unsupported`.

`rename --clear` removes a Pane Label. Labels, Tab Names and child-provided Pane
Titles are separate fields; setting one never overwrites another, and a label
doesn't change the child's terminal title.

`resize --side SIDE --pixels N` puts the divider N logical pixels from the start
of its split. It is not a cell count or the pane's width. Sprite applies its
usual minimum sizes, returns the accepted position and geometry, and keeps the
ratio for later redraws. A side with no divider returns `unsupported`. A resize,
create, move, focus or close during a person's divider drag returns `busy`.

`focus` selects the pane's tab and makes it the Current Pane, asks for the
window to be activated, and reports whether it was active when it answered.
Desktop focus rules can delay or prevent activation.

## Terminal input

Send either text or one named key. Text is written as typed, wrapped in
bracketed-paste markers when the program has turned that on. Sprite does not
check it or hold it back: a newline in text runs the line.

Supported keys: `enter`, `tab`, `escape`, `backspace`, `up`, `down`, `left`,
`right`, `ctrl-c`, `ctrl-d`. Each is one key press, encoded with the terminal's
current modes. `ok` means Sprite accepted the input, not that a command ran.
Input always goes to the terminal, even when a Surface has keyboard focus.

## Content and pane kinds

`list` reports, for every pane: address, tab, Pane Label, child title, Tab Name,
whether it is the Current Pane, visibility, geometry in logical pixels,
lifecycle, content support and capabilities.

| Pane | Management | Read | Input |
| --- | --- | --- | --- |
| Terminal | All operations | Terminal text and history | Text and named keys |
| Terminal with a Surface | All operations; the Surface stays | The terminal text underneath; native content unavailable | Terminal only |
| Other pane kinds | Layout, label, focus and close | Metadata only | `unsupported` |

Read content is untrusted terminal output. The nested terminal snapshot keeps its
own `complete` flag and failures, so a top-level `ok` doesn't mean an ended or
slow terminal supplied everything.

## Close and outcomes

`close` removes the pane at once, even when a program is running in it. The
program is ended by the usual bounded Terminal Session cleanup. The answer says
`removed: true` and `cleanup: scheduled`; cleanup may still be running. Closing a
window's last pane sends the answer before the window's socket closes, and Sprite
still waits for cleanup before quitting.

| Outcome | Meaning |
| --- | --- |
| `ok` | Accepted; the result has the details. |
| `gone` | The pane has closed, or never existed in this window. |
| `unsupported` | The pane kind, divider side or cross-window move isn't supported. |
| `invalid_request` | Bad grammar, protocol version, fields, sizes, or an address for another window. |
| `busy` | A limit or a person's divider drag prevents it. |
| `closing` | The window or session is closing. |
| `outcome_unknown` | It may have been applied, but the answer was lost. Check the state before trying again. |

CLI exit codes: 0 for `ok`, 2 for argument errors, 4 when no window could be
reached (including an address whose window isn't running), 5 for any other
outcome. The CLI never retries an operation.

Limits per window: 16 connections at once, 16 queued operations, 8 KiB request
lines, 256-byte labels, 4 KiB input and 16 MiB responses. `create` is refused at
128 open panes or 16 unfinished cleanups, and `close` at 16 unfinished cleanups.
Reads return at most 5,000 lines of history.

## Configuration

```sh
sprite config reload [--window <id>]
sprite config print [--window <id>] [--config <path>]
```

`config reload` re-reads the window's configuration file and prints what
changed; a file the window couldn't apply exits 5. `config print` prints the
settings a window is actually using. With `--config`, or with no window running,
it prints what Sprite would load from the file.

## Direct protocol

Connect to the window's socket and send one UTF-8 JSON line:

```text
{"protocol":"sprite-pane-control/2","id":"unique-request","window":"<window>","operation":{"op":"list"}}
```

Sprite answers with one JSON line and closes the connection:

```json
{"protocol":"sprite-pane-control/2","id":"unique-request","outcome":"ok","result":{"window":"<window>","panes":[]}}
```

Request IDs are at most 64 bytes, and unknown fields are rejected. A request with
another protocol version gets `invalid_request` with
`{"supported":"sprite-pane-control/2"}`. Operations:

```json
{"op":"list"}
{"op":"read","pane":"<window>:0","lines":100}
{"op":"create","beside":"<window>:0","side":"right"}
{"op":"create"}
{"op":"rename","pane":"<window>:0","label":"work"}
{"op":"rename","pane":"<window>:0","label":null}
{"op":"move","pane":"<window>:0","beside":"<window>:1","side":"down"}
{"op":"move","pane":"<window>:0"}
{"op":"focus","pane":"<window>:0"}
{"op":"resize","pane":"<window>:0","side":"right","pixels":400}
{"op":"close","pane":"<window>:0"}
{"op":"input","pane":"<window>:0","text":"hello"}
{"op":"input","pane":"<window>:0","key":"enter"}
{"op":"config_reload"}
{"op":"config_print"}
```

`config_reload` answers `{"applied":true|false,"message":"…"}` and
`config_print` answers `{"settings":"<TOML>"}`.
