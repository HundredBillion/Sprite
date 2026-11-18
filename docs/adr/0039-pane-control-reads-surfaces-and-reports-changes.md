# Pane control reads Surfaces and reports changes

Requested by the owner on 2026-11-17 (issue #174). Amends ADR 0036 and reverses
one line of the 2026-10-26 pane control design, which said a read reports
native content as unavailable.

Pane Control now does three more things, all inside
`sprite-pane-control/2`. Each is an addition an older client never sees.

1. **A read returns what a pane's Surfaces show.** A grid gives its rows as
   text and its cursor, a list its rows, an element tree its text, button
   labels and where its pictures sit. The 10-26 design withheld this because
   reading then needed a grant. ADR 0036 removed grants: a process of the same
   user can already read the terminal under a Surface and type into it, so
   withholding the Surface protected nothing and hid an editor's whole screen
   from the orchestrator the owner runs beside Sprite.
2. **A connection can `watch`.** The window sends one line when a watched pane
   changes, naming what changed. Without it an orchestrator reads every pane
   on a timer, which keeps two processes awake while nothing happens.
3. **A connection can stay open** (`keep_open`) and send request after
   request, so a client pays for connecting once.

## Why Sprite and not the orchestrator

Sprite already parses every pane's output and holds every Surface's
description. A read copies state it has. An orchestrator that kept its own
copy would parse all terminal output a second time and still could not see a
Surface. Agent state, messages between agents and task routing stay in the
orchestrator: ADR 0036's "orchestration lives in a separate client" stands.

## What keeps a long-lived connection from hanging or crowding out

Pane Control connections were one request long, so two earlier findings about
long-lived Surface connections did not apply to it. They do now, and both are
closed off here:

- **Issue #79** (on macOS a thread asleep in `recv` can miss the peer closing
  its end): a connection that stays open never sleeps in a read. It waits in
  `poll`, looks again every second, and reads only what `poll` reported. The
  `sprite panes watch` client waits the same way.
- **Issue #67** (a slow reader keeps a connection slot): every answer and
  every notice is written within the drain deadline the Surface Channel uses,
  which a reader cannot stretch by taking a few bytes at a time. This now
  covers one-request answers too, which had only a per-write timeout.
- At most 8 of a window's 16 connections may stay open. The ninth is answered
  `busy` before its request is applied. A one-request client such as
  `sprite panes list` can therefore always be served.
- What a watcher has not yet been sent is one merged entry per pane, so a
  watcher that reads slowly costs a bounded amount of memory until its write
  deadline drops it.
- A request line that starts and does not finish within the handshake timeout
  closes its connection.

A quiet connection is not closed for being quiet: an orchestrator is meant to
hold one for as long as it runs. Its place comes back when the peer goes.

## What a read is bounded by

All of a pane's Surfaces share 2 MiB of text in one read, counted as it is sent,
with the quotes and escapes JSON adds, and every text an entry sends is counted.
The terminal's text gets what is left of the 16 MiB response after that is set
aside. A program therefore cannot make its own pane unreadable by showing large
rows or opening many Surfaces.

## Where notices come from

Content changes are marked by the pane's view. Everything `list` reports
besides content (geometry, tab, visible, current, label, tab name, and which
panes exist) is compared by the window whenever its layout is refreshed and
after each Pane Control request, over the window's own panes. A pane that isn't
a terminal, or whose program never started, is told of like any other. A watch
of a pane the window doesn't have is refused with `gone`.

## Cost when unused

With no watcher, marking a content change is one atomic load. Nothing is timed
and nothing is added to a frame. The comparison above runs only when the layout
is refreshed or a request is served, never per frame.

## Considered and rejected

- **A new protocol version.** Nothing an existing client sends or reads
  changes meaning. `list` reports `features`, and an older Sprite answers
  `invalid_request` to the new fields, so a client can tell either way.
- **Sending the changed content in the notice.** A notice would then be as
  large as a read and a slow watcher's backlog unbounded. A notice says where
  to look; the read stays the one place content comes from.
- **Highlights and pixels.** A read returns text. Colour and images are for
  drawing, and an agent reading a pane wants the words.
