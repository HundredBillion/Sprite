# Checkpoint 3 security review request

**Status: PERFORMED, 2026-09-05.** The review this document prepared is
[checkpoint-3-security-review.md](checkpoint-3-security-review.md), which
answers every question below. This file is kept as the brief it was.

The observation surface is the most security-sensitive code in Phase 1: it
brokers read-only access to the contents of other panes. The TSP requires a
security review separate from general review, covering key handling, scope
enforcement, the deadline, and the exclusion list. Those four areas are set out
below with what was built, what it is meant to guarantee, and where a reviewer
should push hardest.

Two other reviews are also still owed and are **not** covered here: the general
review of Checkpoints 1 and 2, and this checkpoint's own general review.

## 1. Key handling

**What exists.** Each window mints a 32-byte key from `/dev/urandom` and a
separate random socket name, in `$XDG_RUNTIME_DIR/sprite` created `0700` before
the socket exists, with the socket itself `0600`. The key is injected into the
environment of every session that window starts. It is compared in constant time
over all 32 bytes; a malformed candidate is compared against zeroes so a wrong
length costs the same as a wrong key. Closing the window unlinks the socket and
stops the key being accepted, including for a request already in flight.
Re-enabling observation creates a new key at a new path rather than reviving the
old.

**Push hardest on.**

- The key is in the environment of every child, so it is readable from
  `/proc/<pid>/environ` by this user, and inherited by every process a pane
  starts. Is environment injection the right mechanism at all, or should a
  session receive a descriptor instead?
- `/dev/urandom` is read directly rather than through a crate. Is that
  acceptable here, and is the failure mode right — `Endpoint::open` returns an
  error and the window runs with no observation rather than a weak key?
- The wipe on drop is `fill(0)` plus `black_box`, chosen to avoid introducing
  `unsafe`. Is that sufficient, given the key also exists in the environment of
  every child anyway?
- Sweeping dead sockets on open connects to each `.sock` in the directory and
  unlinks the ones that refuse. Can that race a window that is starting?

## 2. Scope enforcement

**What exists.** Scope resolves only against `source.panes()`, which lists one
window, so a pane in another window cannot appear whatever is asked. A pane that
does not exist and a pane that may not be seen produce the same `denied`. The
default scope is the requester's own tab excluding itself; `--include-self`,
`--pane`, and `--window` adjust it.

**The model is decided, and is not what this review is for.** Any pane may read
any other pane in its window: the key separates windows, not panes. That is a
deliberate product decision recorded in
[ADR 0013](../adr/0013-scope-observation-to-the-window-not-the-pane.md), taken so
that tools can coordinate across panes. It follows that a program in one pane
can read a secret visible in another, and that the window is the unit of trust.
Peer credentials are not wanted: they would make `--from` honest without making
it meaningful, since the same key already authorises `--window`.

Please review the implementation of that model, not the model. Specifically:

**Push hardest on.**

- Does the window boundary actually hold? Scope resolves only against one
  window's panes, which is meant to make "never across windows" structural
  rather than a check. Is there any path — `--pane` with an id from another
  window, a stale registry entry, a shared runtime directory — by which one
  window's request reaches another window's pane?
- The refusal is identical for wrong key, unknown pane, and forbidden pane — but
  a *malformed* request is distinguishable. Does that distinction leak anything?
- `WindowPanes` is populated by views registering themselves and removed on
  drop. Is there a window in which a closed pane is still listed?
- Given the decision, is there anywhere the code still implies a per-pane
  boundary it does not enforce? Half a boundary is worse than none.

## 3. The deadline

**What exists.** One 500 ms deadline for a whole request, not per pane. Every
pane is asked before any answer is awaited. After the deadline each remaining
pane gets a non-blocking chance to hand over an answer that arrived while
another was stalling. Connections are served one thread each, capped at 16 in
flight, with a 2 s client timeout.

**Push hardest on.**

- TSP open question 3, unresolved: a pane under sustained output may always miss
  the deadline, making a loud pane permanently unobservable. Is that acceptable?
- 16 concurrent connections each spawn a thread. A local caller can open all 16
  and hold them for 2 s, making the endpoint refuse others. Is that denial of
  service worth closing, given the caller is already inside the window?
- The response is encoded *after* the deadline, and encoding a 16 MiB answer
  takes about 80 ms. So the wall-clock cost of a request exceeds the deadline.
  Is the deadline meant to bound collection or the whole exchange?

## 4. The exclusion list

**What exists.** Every field of the JSON is written out by hand; nothing is
derived from a Rust type, so a field added to a snapshot for the renderer cannot
appear on the wire. Tests assert both that forbidden words are absent from the
rendered text and that a pane object's key set equals the agreed list exactly.
Every pane declares `content_trust: "untrusted_terminal_output"`. The foreground
executable is read from `/proc/<pid>/comm`, never `cmdline` or `environ`.

**Push hardest on.**

- `working_directory` comes from OSC 7, which the child controls, and `title`
  from OSC 2. Both are attacker-controlled strings in an attacker-controlled
  terminal. They are declared untrusted, but are they safe to include at all?
- Row text is the terminal's *rendered* content. Does anything reach it that the
  exclusion list means to keep out — for instance a password echoed to screen,
  or the contents of a file a pane happened to `cat`? The PRD's exclusions are
  about metadata; the rows are the point of the feature. Is that boundary drawn
  where a reviewer would draw it?
- `content_trust` is a declaration, not a defence. Sprite does not neutralise
  prompt-injection text. Is saying so sufficient?

## How to exercise it

~~~bash
cd phase_1
cargo test --workspace --locked --offline
./target/release/sprite                      # open a window
# inside a pane:
sprite panes snapshot --pretty                # its tab, excluding itself
sprite panes snapshot --window --lines 5000   # everything this window has
sprite panes snapshot --pane 999              # denied, as for any unseeable pane
~~~

Turning it off entirely, and confirming nothing is injected:

~~~toml
# $XDG_CONFIG_HOME/sprite/config.toml
[pane_observation]
enabled = false
~~~
