# Open pane control

Status: design approved by the owner on 2026-11-01, sections 1–3 in chat; the owner
asked to go straight to implementation.

Base: `39c6ffc` (`master` after PR #153). Branch: `feat/open-pane-control`.

## Problem

Agents that the owner starts can't read or drive Sprite panes without the owner
clicking Allow. A pane's own shell gets an observation key, but agents usually run
under a long-lived orchestrator (for example `herdr server`) whose processes were
not started by a Sprite window, so they have no key and must ask for a grant
(ADR 0034). The owner wants an orchestrator that pairs with Sprite, and does not
want any Sprite code that asks a person to click Allow before an action proceeds.

## Outcome

Any process running as the same OS user can list, read, create, rename, move,
focus, resize, close and type into any pane of any Sprite window, and reload or
print a window's configuration, with no prompt and no key. Sprite stays a
standalone terminal and owns the terminals; orchestration lives in a separate
client.

The owner's decisions:

- Access is given to every process of the same user ("option A"). The socket's
  file permissions are the only check.
- Sprite has no paste-safety logic and no prompts that gate an action: the
  unsafe-paste hold and the close-a-running-program confirmation are removed
  everywhere, for keyboard actions as well as for agents.
- The Cmd-Q quit toast (`quit_gesture.rs`) stays, unchanged.

## Design

### 1. Endpoint and discovery

- Each window opens `<runtime>/sprite/window-<window-id>.sock`. `<runtime>` is
  `$TMPDIR` on macOS and `$XDG_RUNTIME_DIR` on Linux, as today. The directory is
  0700 and the socket 0600; that is the whole authentication.
- The window id is 24 hex characters (it was 64), so the path fits macOS's
  103-byte socket limit.
- Protocol `sprite-pane-control/2`: the client sends one JSON line
  `{"protocol","id","window","operation"}` with no key prefix, receives one JSON
  line, and the connection closes.
- `SPRITE_PANE=<window>:<pane>` is set in every pane's environment (it was a bare
  number). `SPRITE_OBSERVATION_SOCKET` and `SPRITE_OBSERVATION_KEY` are no longer
  set.
- The Surface socket, its key and `SPRITE_SURFACE_*` are unchanged.

### 2. What is removed, moved and kept

Removed:

- Grants: `sprite panes access request`, `--access` descriptor files, rights,
  scopes, `MAX_GRANTS`, the Allow/Deny prompt and the Pane Access panel.
- The observation socket and its key, the `sprite panes snapshot` command, the
  `sprite-observation-bench` binary and the `[pane_observation] enabled` setting.
- The unsafe-paste hold, for keyboard paste and control input alike. Text is sent
  as typed, wrapped in bracketed-paste markers when the program has enabled them.
- The busy-pane close confirmation for panes, tabs and windows, from keyboard and
  control alike. `close_warning`, `Mode::ConfirmingClose`, `PendingClose` and the
  control `confirmation_required` outcome go with it.

Moved:

- `config reload` and `config print` become operations on the window socket.
- Pane Labels stay window state; they move out of the deleted grant state.

Kept:

- The Cmd-Q quit toast and shutdown cleanup (ADR 0032).
- `ForegroundWatch`, still used for titles and Surface ownership.
- The broker, schema and `WindowPanes` code that builds `read` responses.

Compatibility: a leftover `[pane_observation]` section in the config is accepted,
ignored, and logged as one warning.

### 3. Requests, CLI and errors

Request flow: the socket thread parses one line, relays it to the window through
the existing control lane (ADR 0030), and writes one reply. `read` content is
built on the socket thread, as now.

Operations: `list`, `read`, `create`, `rename`, `move`, `focus`, `resize`,
`close`, `input` with their current shapes, plus `config_reload` and
`config_print`.

- `close` removes the pane at once; its program is ended by the normal bounded
  cleanup.
- `input` text is sent unchecked.
- Outcomes: `ok`, `gone`, `unsupported`, `invalid_request`, `busy`, `closing`,
  `outcome_unknown`. `denied` and `confirmation_required` are removed.
- A `/1` request is answered `invalid_request`, naming `/2`.

CLI:

- `sprite panes list` asks every `window-*.sock` in the folder and merges the
  answers, skipping sockets nobody is listening on.
- Commands that take a pane address connect to that address's window.
- `sprite panes create` without `--beside`, `sprite config reload` and
  `sprite config print` pick a window: the window in `SPRITE_PANE`, else the
  only open window, else they fail and ask for `--window <id>`.
- Exit codes are unchanged: 0 ok, 2 usage, 4 file or connection error, 5 refusal
  or unknown outcome.

Errors and limits:

- A leftover socket (window not running) is exit 4; `list` skips it silently.
- A pane address whose window is not open is exit 4.
- Kept: 16 connections per window, 16 queued operations, 8 KiB requests, 4 KiB
  input, 16 MiB responses, 5,000 lines per read, creation refused at 128 open
  panes or 16 unfinished cleanups.

### 4. Tests

Through the public interfaces, red before green:

- The window socket answers a keyless `list` and `read`; a `/1` or keyed request
  is `invalid_request`.
- The socket file is 0600 in a 0700 directory and is removed when the window
  closes.
- `sprite panes list` merges two windows and skips a dead socket.
- A pane address for a window that is not running exits 4.
- `create` with no `--beside` resolves the window from `SPRITE_PANE`, then the
  single open window, and otherwise asks for `--window`.
- `close` of a pane running a program removes it at once and schedules cleanup.
- `input` of multi-line text with no bracketed paste is delivered as typed.
- A keyboard close of a busy pane, tab or window closes it with no banner.
- A keyboard paste of multi-line text is written to the program, with no hold.
- `config_reload` and `config_print` work over the socket.
- A config containing `[pane_observation]` loads with one warning.
- A pane's environment holds `SPRITE_PANE=<window>:<pane>` and no observation
  variables.

Removed tests that asserted the deleted behaviour are deleted, not rewritten.

## Documentation

A new ADR supersedes ADR 0034 and amends ADRs 0001 and 0013. `crates/CONTEXT.md`,
`docs/pane-control.md`, `README.md` and `docs/technical-reference.md` are updated
to the new model.

## Risks

- Any process of the same user can drive every pane, including typing commands.
  That is the owner's chosen trust boundary, the same one a shell already has.
- Existing scripts using `--access` files or `sprite panes snapshot` break. The
  owner is the only user of both.
