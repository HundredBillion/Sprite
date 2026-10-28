# Pane control

Sprite Terminal lets a local agent manage existing and newly created panes when
you approve a Pane Grant. This includes panes used by a person, panes in
background tabs, and panes hosting native Surfaces.

## Grant access

Run inside the Sprite window you want to authorize:

```sh
sprite panes access request --all-panes --manage --output /tmp/my-sprite-access.json
```

Sprite displays the requested scope and rights. Choose Allow or Deny. A request
expires after 60 seconds; another pending request receives `busy`. The displayed
requester pane ID is a claim, not verified identification of a program.

The output file is created exclusively with private permissions. Choose a new
path for each request. Approval stores the credential there; stdout reports the
file path, window and grant ID without printing the token. Never share the file
unless you intend to delegate its rights. A failed save requires revoking that
grant in Pane Access. Credentials are not automatically inherited by new panes.

For selected existing panes, replace `--all-panes` with one or more `--pane N`
options using the numeric IDs from `sprite panes snapshot --window`. An
all-panes grant includes future human and agent panes. A selected-pane grant
includes panes created with that grant, but excludes unrelated future panes.
Its scope holds at most 128 open panes. A closed pane leaves the scope, so a
long-lived grant can keep creating and closing panes.

Rights are separate:

| Requested flags | Rights |
| --- | --- |
| Neither | Read metadata and supported terminal content. |
| `--manage` | Read plus create, rename, move, focus, resize and close. |
| `--input` | Read plus terminal text/key input, which can execute commands. |
| Both | Both management and input. |

Creating a pane starts the configured shell, without accepting an arbitrary
command. Manage alone cannot type into that shell. Existing observation and
Surface credentials gain no control rights. Observation must be enabled to
request a new grant; previously approved grants retain their independently
managed rights until revoked or their window closes.

Click **Pane Access** in Sprite to revoke a grant, remove management, or remove
input. Restoring rights needs another request and approval. Revocation checks
also apply to queued operations and pending human confirmations. Work already
committed before revocation remains committed.

This is a local bearer-capability interface, not an OS sandbox against hostile
code running as the same user.

## Use the public CLI

The examples use `jq` to extract returned addresses. Replace the access path as
needed. Requests return JSON on stdout; diagnostics go to stderr.

```sh
access=/tmp/my-sprite-access.json
sprite panes list --access "$access"
human=$(sprite panes list --access "$access" | jq -r '.result.panes[0].id')
pane=$(sprite panes create --access "$access" --beside "$human" --side right | jq -r '.result.id')
sprite panes read --access "$access" --pane "$pane" --lines 100
sprite panes rename --access "$access" --pane "$pane" --label 'Agent work'
sprite panes resize --access "$access" --pane "$human" --side right --pixels 400
sprite panes move --access "$access" --pane "$pane"
sprite panes focus --access "$access" --pane "$pane"
sprite panes close --access "$access" --pane "$pane"
```

`create` without `--beside` opens a new tab. `move` without `--beside` moves the
existing pane to a new tab. With `--beside ID --side left|right|up|down`, each
operates beside an addressed pane. Moves preserve pane and Terminal Session
identity, content and Surface connections. They do not silently select a
background tab; if the selected tab disappears, the usual tab fallback applies.
Moving beside itself is invalid. A selected-pane grant also needs access to its
destination anchor. An empty tab disappears through normal tab lifecycle.

`rename --clear` removes a Pane Label. Labels, user Tab Names and child-provided
Pane Titles are separate response fields; naming one never overwrites another.
Labels are available in the control inventory; a label does not rewrite the
child's terminal title.

`resize --side SIDE --pixels N` positions the divider N logical pixels from the
start of its containing split. It is not a cell count or an absolute width of
the addressed pane. Sprite applies its usual minimum allocation and clamping,
returns the accepted position and geometry, and stores the split ratio so the
next redraw preserves it. A side without a divider returns `unsupported`.
Background tabs use their owning window's viewport; their PTY allocation is
applied when shown. Later window-size changes adjust pane allocation normally.
A resize, create, move, focus or close during a human divider drag returns `busy`.

`focus` selects the target tab and Current Pane, requests window activation and
reports actual window activation at response time. Desktop focus policies can
prevent or delay activation; accepted focus is not proof of foreground input.

For another authorized window, repeat the access request inside that window and
supply its access file. Multiple `--access` files are supported: `list` collects
their inventories; other commands require a single matching descriptor, selected
with `--window ID` when necessary. Pane/tab addresses contain their window's
random incarnation ID, so restarting cannot make an old address refer to a new
pane. Cross-window moves return `unsupported` without transferring ownership.

## Terminal input

Request `--input` explicitly, then use either text or one named key:

```sh
sprite panes input --access "$access" --pane "$pane" --text 'echo hello'
sprite panes input --access "$access" --pane "$pane" --key enter
```

Supported keys: `enter`, `tab`, `escape`, `backspace`, `up`, `down`, `left`,
`right`, `ctrl-c`, `ctrl-d`. Each named key is one key press; remote key-release
or hold events are unsupported. Keys use the existing terminal encoder and its live
terminal modes. Text uses terminal paste encoding. Admission means Sprite
accepted input; it does not report command execution or completion.

Potentially unsafe paste requires an explicit human decision tied to that grant,
pane and exact payload. This check is conservative even when bracketed paste is
currently enabled. Repeating an input request never confirms it. Approval
rechecks authorization before submitting the confirmed payload. Input always
targets the terminal, even when a Surface has keyboard focus; synthetic Surface
input is unsupported. No permission exposes an arbitrary host-process PID signal.

## Content and pane kinds

`list` reports metadata for every authorized pane: address, tab, Pane Label,
child title, Tab Name, Current Pane, visibility, logical-pixel geometry,
lifecycle, content support and capabilities.

| Presentation | Pane management | Read | Input |
| --- | --- | --- | --- |
| Terminal | All listed operations | Bounded terminal text/history | Terminal text and named keys. |
| Terminal with fill/dock/overlay Surface | All listed operations, retaining Surface | Underlying terminal text; native content explicitly unavailable | Terminal target only. |
| Other hosted PaneHandle implementation | Layout, label, focus and close | Metadata; unsupported terminal content | `unsupported`. |

Surface metadata identifies elements/grid/list bodies and placement. It does not
expose Surface documents, editor buffers, raster pixels or arbitrary native UI.
An unknown create kind is invalid: only the configured terminal factory exists.
Read content is untrusted terminal output. The nested terminal snapshot keeps
its existing `complete` and failure information; top-level `ok` does not imply
that an ended, failed or slow terminal supplied complete content.

## Close and outcomes

Idle panes can close unattended. A pane with a foreground-job warning returns
`confirmation_required` and displays Confirm/Cancel in Sprite. Repeated close
requests cannot answer it. Confirmation expires after 30 seconds, and approval
checks the original target and current permission. Denial, expiry or revocation
performs no operation. The requester may inspect pane state after a human
confirms; there is no command/event stream or automatic retry.

Successful close reports `removed: true` and `cleanup: scheduled`. It uses the
existing bounded Terminal Session cleanup and foreground-job safeguards. It is
not a promise that cleanup has already finished. Closing the last pane sends its
reply before canceling its control endpoint; Sprite still waits for owned
pending cleanup before quitting.

| Outcome | Meaning |
| --- | --- |
| `ok` | Operation accepted; inspect the result for admission, focus and cleanup details. |
| `denied` | Missing grant/rights or a target outside scope. Does not reveal existence. |
| `gone` | An authorized, previously allocated target has closed. |
| `unsupported` | Target content/operation, divider side or cross-window move is unsupported. |
| `invalid_request` | Invalid grammar, version, fields, sizes or option combination. |
| `busy` | A request/resource bound or active human gesture prevents admission. |
| `confirmation_required` | A human decision is pending; no unattended confirmation is accepted. |
| `closing` | Window/session is closing. |
| `outcome_unknown` | A claimed/sent operation might have applied but its result could not be recovered. Inspect state before acting again. |

CLI exits: 0 for `ok`, 2 for argument errors, 4 for local file/transport errors,
5 for protocol refusals and unknown outcomes. Legacy `panes snapshot` retains its
existing schema and partial-success exit behavior. Control clients never retry
a non-idempotent operation automatically. Request IDs correlate answers; no
replay cache or durable deduplication is promised.

Limits: 16 grants/window, 16 active connections/grant, 16 queued control
operations/window, 8 KiB authenticated request lines, 256-byte labels, 4 KiB input
payloads and 16 MiB responses. Agent creation refuses at 128 live panes or 16
unfinished cleanups/window; agent close refuses at 16 unfinished cleanups.
Keyboard behavior remains available and keyboard-created resources count toward
later agent admission. Read history keeps the observation bound of 5,000 lines
and its single broker deadline. Invalid and saturated requests never silently
succeed or leave a late mutation queued after abandonment.

## Direct protocol

A descriptor is a private JSON document with `window`, `grant`, `socket` and
`key`. Connect to its Unix socket and send one UTF-8 line:

```text
<key> {"protocol":"sprite-pane-control/1","id":"unique-request","window":"<window>","operation":{"op":"list"}}
```

Sprite replies with one JSON line and closes the connection:

```json
{"protocol":"sprite-pane-control/1","id":"unique-request","outcome":"ok","result":{"window":"<window>","panes":[]}}
```

Request IDs are at most 64 bytes. Unknown fields are rejected. Operation shapes:

```json
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
```

Access requests travel on the existing authenticated observation endpoint as
`<observation-key> panes access <JSON>`. JSON contains `all_panes`, `panes`
(an array of numeric IDs), `manage`, `input` and optional unverified `from`.
Choose either all panes or a nonempty fixed list. Only the UI can approve it.
The response uses ID `access` and returns the descriptor on approval. Missing or
incorrect control authentication returns `denied` with an empty request ID;
malformed input may also have an empty ID because no valid request was decoded.
