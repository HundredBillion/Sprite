# Pane control

Any program you run can list, read, watch, create, label, move, focus, resize,
close and type into Sprite panes, in every Sprite window, with no prompt and no
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
sprite panes read --pane "$pane" --surface 4
sprite panes watch --panes "$pane"
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
- `panes create` without `--beside`, `panes watch` without `--panes`,
  `config reload` and `config print` use the window named by `--window <id>`, else the window in `SPRITE_PANE`, else the
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
| Terminal with a Surface | All operations; the Surface stays | The terminal text underneath, and what each Surface shows | Terminal only |
| Other pane kinds | Layout, label, focus and close | Metadata only | `unsupported` |

Read content is untrusted terminal output. The nested terminal snapshot keeps its
own `complete` flag and failures, so a top-level `ok` doesn't mean an ended or
slow terminal supplied everything.

## Surface content

A program such as an editor can draw a pane itself, through Surfaces. The
terminal underneath then shows little, so `read` also returns what each Surface
shows, as text, in `result.surfaces`. Every entry has the Surface's `id`,
`position` (`fill`, `dock` or `overlay`), `kind`, `content_trust` and
`truncated`, and then:

| Kind | Content |
| --- | --- |
| `grid` (an editor) | `size`, `cursor` and `rows`, each with its `text`, without the blanks at the end of a row. `cursor` has `row`, `column`, `text_index` and `visible`. Editor tabs, if any, are in `tabs` with `id`, `label`, `modified` and `active`. |
| `list` (a tree or a panel) | `revision`, `total`, `selected`, `status` and `rows`. A row has `id`, `text` and `indent`, plus whichever of `kind` (`input` or `button`), `icon`, `leading`, `description`, `badge`, `count`, `placeholder` and `selected` apply. |
| `elements` | `nodes` in drawing order: each text and button label with its `kind` and `depth`, and each picture as `{"kind":"image","svg_bytes":N}`. |

A cursor's `column` counts cells, and a wide character such as `界` fills two.
`text_index` is the number of characters of the row's `text` before the cursor,
so use it, not `column`, to find the cursor in the text. It can be past the end
of the text when the cursor sits in the blanks after it.

`read --surface ID` returns that one Surface and leaves the terminal out. An id
the pane doesn't have returns `gone`. `--lines N` also limits how many rows of
each Surface come back. All the Surfaces in one read share 2 MiB of text, in the
order `list` gives them. `truncated` on a Surface says that rows, tabs, or its
`selected` or `status` text were left out; a Surface reached after the 2 MiB is
used still appears, with no rows. A read returns
text only: no colours and no pixels. Like terminal text, it is whatever a
program chose to show, so treat it as untrusted.

`list` marks a Surface `"content": "readable"`, gives a terminal pane the
`surface_read` capability, and names what the window supports in
`result.features`. An older Sprite reports `"content": "unavailable"` and has
no `features`. `sprite panes list` asks every window, so its `features` are the
ones every running window supports: empty if any of them is an older Sprite.

## Watching for changes

`sprite panes watch` stays connected and prints one line each time a pane
changes, so a program doesn't have to read every pane on a timer:

```sh
sprite panes watch                    # every pane of the window
sprite panes watch --panes "$a,$b"    # these panes, all in one window
```

The first line is the window's answer. Each later line is a notice:

```json
{"protocol":"sprite-pane-control/2","event":"changed","pane":"<window>:0","changed":["terminal","surface:4"],"generation":578}
```

| In `changed` | Meaning |
| --- | --- |
| `terminal` | The terminal drew new content. `generation` is the one a read would now return. |
| `surface:<id>` | That Surface opened, changed or closed. An operation Sprite refused changes nothing and sends no notice. |
| `title` | The program set or cleared its title. |
| `lifecycle` | The pane's session ended. |
| `layout` | The pane's `geometry`, `tab`, `visible` or `current` changed: it moved, it or the window was resized, its tab was shown or hidden, or it gained or lost being the Current Pane. |
| `label` | The pane's `label` or its tab's `tab_name` changed. |
| `created`, `closed` | The pane opened or closed. |

Every pane `list` reports is watched, whatever kind it is. Watching a pane the
window doesn't have, such as one that already closed, returns `gone`.

A notice says where to look; `read` returns the content. Changes to one pane
less than 16 ms apart arrive as one notice, so a pane printing fast sends at
most about 60 a second. The watch ends when the window closes, or when you close
the connection. A watcher that stops reading is dropped. While nobody watches,
watching costs the window nothing.

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

Limits per window: 16 connections at once, of which 8 may stay open (watching,
or kept open for further requests); 16 queued operations, 8 KiB request lines,
256-byte labels, 4 KiB input and 16 MiB responses. A read returns at most 5,000
rows from each Surface and 2 MiB of text from all of a pane's Surfaces together. `create` is refused at
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

Sprite answers with one JSON line and closes the connection, unless the request
asks otherwise (see [Keeping a connection open](#keeping-a-connection-open)):

```json
{"protocol":"sprite-pane-control/2","id":"unique-request","outcome":"ok","result":{"window":"<window>","panes":[]}}
```

Request IDs are at most 64 bytes, and unknown fields are rejected. A request with
another protocol version gets `invalid_request` with
`{"supported":"sprite-pane-control/2"}`. Operations:

```json
{"op":"list"}
{"op":"read","pane":"<window>:0","lines":100}
{"op":"read","pane":"<window>:0","surface":4}
{"op":"watch"}
{"op":"watch","panes":["<window>:0","<window>:2"]}
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

### Keeping a connection open

A program that sends many requests can send them on one connection. Add
`"keep_open":true` beside `operation`:

```text
{"protocol":"sprite-pane-control/2","id":"r1","window":"<window>","operation":{"op":"read","pane":"<window>:0"},"keep_open":true}
```

Sprite answers and waits for the next line on the same connection. Answers come
in the order the requests were sent. The connection closes after a request
without `keep_open`, when you close it, or when the window closes. It may stay
quiet for as long as you like. Once a request line has started, the rest must
arrive within 5 seconds.

A `watch` request turns its connection into a stream of notices; it takes no
further requests.

A window lets 8 connections stay open at once. One more is answered `busy`,
before its request is applied, and closed. Send that request again without
`keep_open`, or close a connection you no longer need. An older Sprite answers
`invalid_request` to `keep_open` and to `watch`.

Sprite never waits on a connection in a way that can hang. It notices a peer
that closed, or half-closed, within a second, and it drops a peer that reads an
answer or a notice too slowly (slower than 64 KiB a second, after a 2-second
allowance).
