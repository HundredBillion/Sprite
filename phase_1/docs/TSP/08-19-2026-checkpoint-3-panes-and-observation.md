# Sprite Terminal Checkpoint 3 Technical Spec

> **Status: DRAFT — Task 1 implemented, the rest not started.** Task 1 is pure
> logic with no security surface, taken on deliberately while review is
> outstanding. Tasks 5 onward touch the observation surface and should not begin
> before Checkpoints 1 and 2 are reviewed.
>
> **Original status: DRAFT — not reviewed, and not ready to start.** Checkpoint 2 is
> implemented but unaccepted: human review is still owed and four items are
> deferred. This document plans the next checkpoint; beginning it before those
> close would repeat the pattern of carrying debt forward.

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> dmi-superpowers:subagent-driven-development (recommended) or
> dmi-superpowers:executing-plans to implement this TSP task-by-task. Steps use
> checkbox (- [ ]) syntax for tracking.

**Goal:** Compose many instances of the existing Terminal Session into tabs and
recursive splits, then expose their snapshot model through a protected,
local, read-only observation interface and the `sprite panes snapshot` client.

**Architecture:** Still two crates. `sprite-term` gains a history-extraction
path and nothing else structural; `sprite-app` gains the pane tree, the
observation broker, and the socket. **No second terminal model is created**, and
no LLM-specific path exists anywhere: observation is a general terminal
capability that happens to be useful to agents.

**Tech Stack:** Unchanged, plus JSON serialisation for the versioned response
(a dependency ledger entry is owed) and the existing `nix` for Unix-domain
socket credentials if peer checking proves necessary.

## Global Constraints

Checkpoints 1 and 2 constraints carry forward. Additionally:

- **The observation surface is the most security-sensitive code in Phase 1.**
  It brokers read-only access to other panes' content. Every task below that
  touches it states its threat model, and none may be marked done on the
  strength of a happy-path test alone.
- **Never hand out a handle.** A client receives owned text and metadata. Never
  a PTY, a libghostty object, a keystroke stream, mutable state, or a route to
  another child process.
- **Snapshots are untrusted data.** Every response declares
  `content_trust: "untrusted_terminal_output"`. Sprite does not classify,
  redact, or neutralise prompt-injection text, and says so in the schema rather
  than implying safety it does not provide.
- **One window, one key.** The observation key and socket are scoped to a single
  Sprite window and destroyed with it.
- **Linux first**, as the PRD's platform posture records.

## Checkpoint boundary

Checkpoint 3 includes: tabs; recursive binary split trees; geometric focus
movement; per-pane sessions that are never shared; a history-extraction path for
observation; the private per-window socket and key; the versioned JSON schema;
partial-result and deadline handling; response size limiting; and the
`sprite panes snapshot` client.

Checkpoint 3 does **not** include Kitty graphics (Checkpoint 4), packaging
(Checkpoint 5), session persistence across restarts (explicitly out of scope in
the PRD), or accessibility (see below).

## Carried forward from Checkpoint 2

These are open and should close before or during this checkpoint:

1. Human review of Checkpoints 1 and 2.
2. IME, which needs GPUI's `InputHandler`.
3. Paste protection for unbracketed pastes containing newlines.
4. Shell-integration auto-loading, whose per-shell mechanisms are risky.
5. Word-boundary tests for selection against wide characters and combining
   marks.
6. Drag-to-select is unverified by machine; no mouse-injection tool was
   available.

## Accessibility, again

The PRD assigns Checkpoint 3 "tab/pane names and focus through the same
accessibility tree". **GPUI `=0.2.2` still has no accessibility surface**, so
this remains impossible on the pinned version. ADR 0012 records the decision and
its revisit criteria. Nothing in this checkpoint should be designed as though an
accessibility tree exists; when a GPUI release ships AccessKit, tab and pane
identity is already in `PaneSnapshot` and can feed it.

## An architectural consequence of Checkpoint 2

Checkpoint 2 deliberately decided that **snapshots carry no scrollback**, because
rebuilding history on every capture would cost thousands of allocations a second
for data the renderer never draws. That decision stands for rendering.

Observation has the opposite requirement: the PRD promises "up to 5,000 of its
most recent scrollback lines", defaulting to 500. So Checkpoint 3 must add a
**separate, on-demand history extraction path** in `sprite-term` — a request for
N lines of history, answered once, not carried in the render bundle. Task 4
below covers it. Getting this wrong by widening the render snapshot would
reintroduce exactly the cost Checkpoint 2 removed.

---

### Task 1: The pane tree

**Files:** new `sprite-app/src/pane_tree.rs`

- [x] A tab is a recursive binary tree: leaves own a `PaneId`, internal nodes own
  orientation and a ratio. Split, close, and collapse are tested, including that
  closing a sibling returns the survivor to the whole tab rather than leaving a
  split node with one child.
- [x] Closing chooses the nearest surviving pane by centre distance, ties broken
  on id. The test computes the expected survivor from rectangles independently,
  so it cannot simply mirror the implementation's traversal.
- [x] Focus moves geometrically. A neighbour must lie in the requested direction
  *and* share extent on the perpendicular axis, so a diagonal pane cannot steal
  focus from one directly beside you — tested with a layout where tree shape and
  geometry disagree.
- [x] Pane identity is stable across splits, which is what ties a pane to its
  session; a rearrangement that renumbered panes would silently reattach
  terminals to the wrong ones.
- [ ] Sessions are not attached yet, so "moving a pane does not recreate its
  PTY" is only proved at the identity level. Task 2 attaches sessions and can
  assert it end to end.

### Task 2: Many sessions in one window

**Files:** `sprite-app/src/terminal_view.rs`, new `sprite-app/src/workspace.rs`

- [x] Each pane owns exactly one Terminal Session; splitting creates a new one.
  Test that closing, moving, or resizing one pane leaves another's child
  running. `PaneRegistry<T>` is generic over the payload so this is asserted
  headlessly with a drop-recording spy: closing one pane ends exactly one
  session, focus movement ends none, and a closed pane is handed back to the
  caller rather than dropped at an unpredictable moment.
- [x] Normalised `x`, `y`, `width`, `height` per pane within its tab, so clients
  learn left/right and above/below without pixels or DPI. `layout()` returns
  them, and a test asserts the panes tile the tab exactly.
- [x] A launch creates one fresh tab with one fresh session and restores
  nothing.
- [x] Each pane is told its own allocation before it lays out its grid.
  `TerminalView` previously sized itself from the window viewport, which would
  have told every pane in a split it was the full window's size.
- [x] Verify with several panes running Croft and shells simultaneously that
  per-pane shutdown still reaps only its own descendants. Two panes, Croft
  running in one: the pane's tree was `croft`, its `bash`, and two
  `rust-analyzer` processes. Closing that pane left exactly the other pane's
  shell, with the whole Croft tree reaped.

**Defect found and fixed during verification: focus did not follow a split.**
Splitting builds a view whose element does not exist in the dispatch tree until
the frame that draws it, and GPUI discards a focus request naming a handle that
is not yet there. The keyboard silently stayed with the previous pane, so the
second split divided the wrong pane and typing went to the wrong child. The
workspace now records the pane that should hold the keyboard and applies it
while rendering, once the element exists.

This was caught by making the running application report *which child PID*
received a keystroke, rather than by reading a screenshot: the layout looked
plausible precisely because the bug produced a believable arrangement. Focus
*movement* (`Ctrl+Shift+<arrow>`) was confirmed correct at the same time, which
is what localised the fault to newly created panes.

**Second defect found and fixed: a binding reached two consumers.** A pane
encodes every key it does not claim as an application shortcut and writes it to
its child. The workspace's bindings were bound during the *bubble* phase, so the
focused pane saw `Ctrl+Shift+D` first, wrote it to the shell, and only then did
the workspace split — the split key was also typed into the child. This breaks
the input rule that one event never reaches two consumers, the same rule the
double-letter defect broke. The workspace now binds during the **capture**
phase, which runs from the root down to the focused element, and calls
`stop_propagation` on a claimed key so no pane ever sees it.

The symptom that exposed it was small: `Ctrl+Shift+Up` at the top pane, where
focus cannot move, left the shell unable to run the next command — the escape
sequence had been typed into it.

**How Task 2 was verified.** The application was driven end to end, with each
step reporting *which child PID owned the keyboard*, so a claim about focus is
backed by which process received the keystroke:

| step | result |
| --- | --- |
| launch | keyboard on the only pane |
| split right, split down | keyboard on the newly created pane each time |
| focus left / right | keyboard on the pane in that direction |
| focus up, no neighbour | keyboard unmoved, shell still usable |
| close focused pane | exactly one session ended; the others kept running |
| close a pane running Croft | `croft`, its shell, and both `rust-analyzer` processes reaped; the other pane untouched |

Leakage was tested directly by running `cat` in a pane and pressing the
bindings: zero bytes reached the child, against a control run in which ordinary
text did arrive. The control matters — an earlier version of this test reported
"no bytes" for *everything*, because the tty is in canonical mode and the line
discipline holds bytes until Enter. Without the flush and the control, the test
would have "passed" while proving nothing.

### Task 3: Tabs

**Files:** `sprite-app/src/workspace.rs`

- [x] A window owns ordered tabs, each owning one pane tree. `Tabs<T>` is
  generic over the payload, like `PaneRegistry<T>`, so its ownership rules are
  asserted headlessly. Only the active tab is laid out; the rest keep running.
- [x] Tab and pane identity is stable for the lifetime of the window, because
  the observation schema exposes it. **This required moving identity out of the
  tree.** `PaneTree` minted its own IDs from a counter starting at zero, so
  every tab would have held a `PaneId(0)` and one ID would have named several
  panes in one window. A window-scoped `PaneIds` now mints them and the tree
  accepts them, with tests that no two panes in a window share an ID and that a
  closed tab's IDs are never handed out again.
- [x] Test that closing a tab shuts down every session it owns and no others.
  Asserted headlessly with drop-recording payloads, and confirmed in the running
  application: closing a tab holding two panes ended exactly those two children
  and left the other tabs' shells running.

**Bindings.** `Ctrl+Shift+T` opens a tab, `Ctrl+Shift+Q` closes the active one,
`Ctrl+Shift+PageUp`/`PageDown` move between them, wrapping at each end. All
workspace bindings require `Ctrl+Shift` so they cannot collide with what a child
expects to receive, and all are claimed in the capture phase so none reaches a
child. The tab strip appears only when a second tab exists, so a single-tab
window loses no height to it.

**How Task 3 was verified.** The same method as Task 2 — each step reports which
child PID owns the keyboard:

| step | result |
| --- | --- |
| new tab, twice | a new session each time, focused |
| previous tab / next tab | keyboard on that tab's own shell |
| split inside a tab | new session in that tab, focused |
| close the active tab | exactly its two sessions ended; the other tabs' shells kept running, and the keyboard moved to a neighbouring tab |

### Task 4: History extraction for observation

**Files:** `sprite-term/src/snapshot.rs`, `sprite-term/src/lib.rs`

**Threat model:** none directly; this is the data source the broker will expose.

- [x] Add a request for the active screen plus up to N history lines, answered
  once. It must **not** widen the render bundle — see the note above.
  `TerminalCommand::CaptureHistory` is answered once with
  `TerminalEvent::History`. It does not touch the render path at all: the rows
  are read straight from the scrollback in screen coordinates, so the render
  bundle keeps exactly the shape Checkpoint 2 measured. Every committed
  benchmark budget still passes.
- [x] Clamp N to 0..=5000, defaulting to 500. Test the boundary and that a
  request beyond it is clamped rather than refused. `HistoryLines::new` clamps;
  `usize::MAX` yields 5,000, 4,999 stays 4,999, and 0 is a real request meaning
  "the screen only". Asking for more history than exists returns what exists
  rather than an error.
- [x] Return only the **active** screen's history. When an alternate-screen
  application is running, return that screen and its history, never the hidden
  normal-screen buffer. Tested with `less`, a real full-screen program: the
  answer names `Alternate`, reports **zero** rows of available scrollback, and
  does not contain text the shell printed before `less` started.
- [x] Preserve Unicode rows, whitespace, and line-wrap markers exactly.
  Formatting is done with unwrap and trim both off, so a soft-wrapped row stays
  its own row with `wrapped` set, and trailing spaces a child actually wrote
  survive. A combining mark is not normalised away.

**A difference between the two projections, made deliberate.** History rows are
not padded out to the screen width, while `PaneSnapshot` rows report one entry
per cell and so are. An observer reading thousands of rows should not receive
thousands of columns of invented spaces, and whitespace a child *wrote* is
preserved either way — but the difference is real, so it is documented on the
type and pinned by a test rather than left to be discovered later by whoever
writes the schema.

**Continuity.** History and the active screen are one continuous run of rows: a
test prints 200 numbered lines and asserts no number is skipped or repeated
across the seam, with the last history row `line-177` immediately followed by
the first screen row `line-178`.

**Cost**, measured on this machine with a 6,000-row scrollback, request to
answer including the channel round trip:

| requested | rows returned | time |
| --- | --- | --- |
| 0 (screen only) | 24 | 0.32 ms |
| 500 (the default) | 524 | 2.7 ms |
| 5,000 (the maximum) | 5,024 | 20.4 ms |

Linear in rows returned, about 4 µs a row, with no page-traversal blowup at the
maximum. Task 6's deadline handling can be sized against these numbers. The
measurement is kept as an ignored test so it can be re-run rather than trusted.

### Task 5: The window socket and key

**Files:** new `sprite-app/src/observation/endpoint.rs`

**Threat model:** anything that can reach the socket and present the key can read
every pane in the window. The key is therefore unguessable, per-window, injected
only into sessions Sprite launches, and destroyed with the window.

- [x] Create a private Unix-domain socket per window, with restrictive
  permissions, in a per-user runtime directory. `$XDG_RUNTIME_DIR/sprite`, made
  `0700` **before** the socket exists so there is no moment in which the path is
  reachable, with the socket itself `0600`. There is deliberately no fall back
  to a world-writable temporary directory: with no private directory, the window
  has no observation surface at all, which is better than one another user can
  reach.
- [x] Generate an unguessable key per window from a cryptographically secure
  source. Test that two windows never share one. 32 bytes read from
  `/dev/urandom`; 64 draws produce no repeat, and two endpoints share neither
  key nor socket path. The socket filename is separately random and **not**
  derived from the key, because a path appears in the environment and in process
  listings.
- [x] Inject key, socket path, and the session's own pane identity into every
  Terminal Session the window launches. `Tabs` now hands a pane's identity to
  whatever builds its payload, so a session is told which pane it is at the
  moment it is created. Confirmed in the running application: the first child
  carries `SPRITE_PANE=0 SPRITE_TAB=0`, and a pane created by a split carries
  `SPRITE_PANE=1`.
- [x] Reject any request with a missing or incorrect key **without returning
  pane data**, and without revealing whether the key or the pane was wrong. One
  fixed refusal for all three cases. The key is compared in constant time over
  all 32 bytes — a comparison that stopped at the first wrong byte would leak
  how much of a guess was right, which is enough to recover a key a byte at a
  time — and a malformed candidate is compared against zeroes rather than
  returning early, so a wrong length costs the same as a wrong key. A spy proves
  the handler is never reached at all for an unauthorised request.
- [x] Closing the window destroys socket and key. Test that a captured key stops
  working afterwards. Verified live: closing the window removed the socket and
  the runtime directory. A request already in flight when the window closes is
  refused too, so the key stops being accepted at the moment of closing rather
  than whenever the last serving thread finishes.
- [x] No TCP port is opened. Assert this in the forbidden-state scan. Asserted
  by **measurement** rather than by inspection: a test opens an endpoint, reads
  the process's own descriptor table for socket inodes, and intersects them with
  `/proc/net/tcp` and `/proc/net/tcp6`. The intersection must be empty, and the
  test first asserts the endpoint is open and that its Unix socket *is* visible,
  so it cannot pass by measuring nothing. Confirmed against the running
  application as well.

**A defect found by a test that was meant to be a formality.** Connections were
served on the accepting thread, so one client that connected and said nothing
held the whole window's endpoint for the client timeout — five seconds at the
time. That is the same failure Task 6 forbids for a slow pane, arriving early.
Each connection now gets its own thread, capped at 16 in flight so that removing
the stall does not simply move it, with connections past the cap dropped
unread. The timeout is down to two seconds, which is generous for a local client
that connects and writes immediately.

**No `unsafe` was added.** The key is wiped on drop with a plain fill plus
`black_box`, not `write_volatile`: the latter would have been the only `unsafe`
outside the one audited descriptor borrow in `sprite-term`, and that is a poor
trade for wiping 32 bytes.

**The handler refuses everything until Task 6.** A window that answered requests
before anything decided what a caller may see would be an observation surface
with no policy behind it.

### Task 6: The broker

**Files:** new `sprite-app/src/observation/broker.rs`

**Threat model:** a client is authorised but untrusted. It must not be able to
mutate anything, escape its window, or use a slow pane to stall the window.

- [x] Requests are pull-based and read-only. There are no subscriptions, no
  continuous output, and no keystroke access; assert the request enum admits
  nothing that mutates. The grammar is one verb — `panes snapshot` — and
  anything unrecognised is refused rather than ignored, so a lenient parser
  cannot let a verb through. A test tries `send-keys`, `input`, `paste`,
  `subscribe`, `watch`, `stream`, `--write`, `--exec` and `kill`, and asserts
  none of them parse. The narrowness is structural as well: `PaneSource` can
  list this window's panes and ask one for a snapshot, and has no method that
  writes or hands back a session — a broker cannot do what its source cannot
  express.
- [x] Default scope is the requesting pane's tab, excluding the requester;
  `--include-self`, `--pane`, and `--window` adjust it. **Never** across windows.
  Never-across-windows is structural rather than a check: every address comes
  from `source.panes()`, which lists one window, so a pane elsewhere cannot
  appear whatever the caller asks. A pane in another window and a pane that
  never existed produce the same `denied`.
- [x] Capture panes concurrently under **one** 500 ms deadline for the whole
  request. A slow pane must not extend anyone else's. Test with a deliberately
  stalled pane. Every pane is asked before any answer is waited for; per-pane
  deadlines would let a request grow without limit as panes are added, so a
  caller could stall the endpoint just by opening more of them.
- [x] A pane that closes or fails mid-collection does not discard healthy
  snapshots: `complete` becomes false and the failure is named. Four separate
  failures are covered: a pane that stalls, one that fails, one that closes
  after being asked, and one that cannot be asked at all.

**An answer that arrived during someone else's stall is still collected.** Once
the deadline is spent, each remaining pane is still given a non-blocking chance
to hand over an answer that arrived while a neighbour was holding things up.
Discarding it would punish a healthy pane for a slow one, and it costs nothing.

**Answers are matched to callers in order.** Two clients may ask one pane at
once, so the registry keeps a queue of waiters rather than a single slot. The
worker handles commands in order and answers each exactly once, and the view
forwards answers in arrival order, so pairing the oldest waiter with the next
answer is correct. A closing pane releases its waiters immediately rather than
letting them wait out the deadline for something already known to be gone.

**An honest limit, recorded rather than implied.** The key is a boundary between
this window and everything else; it is **not** a boundary between the panes
inside it. Every session the window launches is told the same key, so a caller
can already ask for `--window`, and `--from` is self-reported. The requester's
identity therefore shapes the default scope for convenience — it is not a
privilege, and the module says so instead of implying a separation it does not
enforce. Peer credentials over the socket could make identity real; the TSP
already anticipates that as "if peer checking proves necessary".

**Verified against the running application**, with three panes across two tabs:

| request from pane 0 | answer |
| --- | --- |
| default | pane 1 only — its own tab, without itself |
| `--include-self` | panes 0 and 1 |
| `--window` | panes 0, 1 and 2, including the other tab's |
| `--pane 2` | that pane alone |
| `--pane 999` | `denied` |
| `panes send-keys rm -rf /` | `malformed`, never parsed as a request |
| no key | `denied` |

All answered in under a millisecond. After closing a pane it disappeared from
the listing and asking for it by name returned `denied`, indistinguishable from
a pane that never existed.

**The response is provisional.** It carries counts, not content: shipping pane
text through a format nothing has specified yet would create a second,
accidental schema for clients to depend on. Task 7 replaces it.

### Task 7: The versioned JSON schema

**Files:** new `sprite-app/src/observation/schema.rs`

- [ ] One versioned object with `schema_version` and a `panes` array, built from
  typed Rust data. Pretty printing changes whitespace only and never creates a
  second schema.
- [ ] Deterministic ordering: tabs by window order, panes by normalised top
  edge, then left edge, then stable pane ID. Test that concurrent completion
  order cannot change serialisation.
- [ ] Every snapshot declares `content_trust: "untrusted_terminal_output"`.
- [ ] Exclude screenshots, colours, fonts, raw control sequences, clipboard
  data, environment values, image bytes, and filenames. Test the exclusions
  explicitly — this is a promise about what cannot leak.
- [ ] Foreground executable basename only when obtainable safely from platform
  process state; never arguments or environment; `null` rather than a guess.

### Task 8: Response limiting

**Files:** `sprite-app/src/observation/schema.rs`

- [ ] Bound one response to 16 MiB of encoded JSON.
- [ ] Drop the oldest history first, preserve complete Unicode rows, and mark
  affected snapshots truncated.
- [ ] Metadata and complete current screens outrank history. If they still do
  not fit, omit whole snapshots rather than half a screen, set `complete` false,
  and report `response_limit` per omission.
- [ ] **Never emit malformed or partially cut JSON.** Test at the boundary with
  a pane whose history alone exceeds the limit.

### Task 9: The `sprite panes snapshot` client

**Files:** `sprite-app/src/main.rs`, new `sprite-app/src/cli.rs`

- [ ] Sprite gains argument parsing, which it has never had. Preserve current
  behaviour: no arguments still opens a window.
- [ ] `sprite panes snapshot` sends a bounded request and writes JSON to stdout,
  diagnostics to stderr.
- [ ] Exit zero for a syntactically valid response **even when `complete` is
  false**, because healthy snapshots remain usable. Nonzero only when no valid
  response could be produced.
- [ ] Test outside a Sprite window: no key, so a clear diagnostic and nonzero
  exit, never a hang.
- [ ] This also unblocks the Ghostty performance comparison, which Checkpoint 1
  could not run because Sprite had no way to be given a workload.

### Task 10: Configuration, budgets, and review

- [ ] `pane_observation.enabled = false` disables live: close the socket,
  destroy the key, reject new requests, stop injecting into new sessions. Test
  that an in-flight session keeps running without observation access.
- [ ] Re-enabling creates a **new** endpoint and key rather than reviving the
  old. Test that the destroyed key stays dead.
- [ ] Budgets for multi-pane capture, the deadline path, and response encoding.
- [ ] Re-run Croft, forbidden-state, and provenance gates.
- [ ] **Security review**, separately from general review, covering key
  handling, scope enforcement, the deadline, and the exclusion list.

---

## Open questions for review

1. **Where does the broker live?** This draft puts it in `sprite-app`, since it
   needs the pane tree and tab order. But it also needs history extraction from
   `sprite-term`, and the PRD calls the snapshot model "tested" — meaning it
   should be testable without a GUI. Splitting it may be right.
2. **How is the socket authenticated beyond the key?** Unix peer credentials
   could confirm the same user, which the key alone does not. Worth deciding
   deliberately rather than defaulting to key-only.
3. **What is a "pane timeout" when a pane is merely busy?** The 500 ms deadline
   is per request, but a pane under sustained output may always miss it. That
   would make a loud pane permanently unobservable, which is a bad property.
4. **Does the CLI belong in `sprite-app`?** A separate binary would keep the GUI
   crate from growing a second front door, at the cost of another artifact to
   package in Checkpoint 5.
5. **How much of this can be tested without a window?** Tasks 5 through 8 are
   the security-critical ones and would benefit from headless tests; Checkpoint
   1 and 2 both showed that GUI-side code is where verification gets thin.
