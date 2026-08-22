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

- [ ] Each pane owns exactly one Terminal Session; splitting creates a new one.
  Test that closing, moving, or resizing one pane leaves another's child
  running.
- [ ] Normalised `x`, `y`, `width`, `height` per pane within its tab, so clients
  learn left/right and above/below without pixels or DPI.
- [ ] A launch creates one fresh tab with one fresh session and restores
  nothing.
- [ ] Verify with several panes running Croft and shells simultaneously that
  per-pane shutdown still reaps only its own descendants.

### Task 3: Tabs

**Files:** `sprite-app/src/workspace.rs`

- [ ] A window owns ordered tabs, each owning one pane tree.
- [ ] Tab and pane identity is stable for the lifetime of the window, because
  the observation schema exposes it.
- [ ] Test that closing a tab shuts down every session it owns and no others.

### Task 4: History extraction for observation

**Files:** `sprite-term/src/snapshot.rs`, `sprite-term/src/lib.rs`

**Threat model:** none directly; this is the data source the broker will expose.

- [ ] Add a request for the active screen plus up to N history lines, answered
  once. It must **not** widen the render bundle — see the note above.
- [ ] Clamp N to 0..=5000, defaulting to 500. Test the boundary and that a
  request beyond it is clamped rather than refused.
- [ ] Return only the **active** screen's history. When an alternate-screen
  application is running, return that screen and its history, never the hidden
  normal-screen buffer. Test with a real full-screen program.
- [ ] Preserve Unicode rows, whitespace, and line-wrap markers exactly.

### Task 5: The window socket and key

**Files:** new `sprite-app/src/observation/endpoint.rs`

**Threat model:** anything that can reach the socket and present the key can read
every pane in the window. The key is therefore unguessable, per-window, injected
only into sessions Sprite launches, and destroyed with the window.

- [ ] Create a private Unix-domain socket per window, with restrictive
  permissions, in a per-user runtime directory.
- [ ] Generate an unguessable key per window from a cryptographically secure
  source. Test that two windows never share one.
- [ ] Inject key, socket path, and the session's own pane identity into every
  Terminal Session the window launches.
- [ ] Reject any request with a missing or incorrect key **without returning
  pane data**, and without revealing whether the key or the pane was wrong.
- [ ] Closing the window destroys socket and key. Test that a captured key stops
  working afterwards.
- [ ] No TCP port is opened. Assert this in the forbidden-state scan.

### Task 6: The broker

**Files:** new `sprite-app/src/observation/broker.rs`

**Threat model:** a client is authorised but untrusted. It must not be able to
mutate anything, escape its window, or use a slow pane to stall the window.

- [ ] Requests are pull-based and read-only. There are no subscriptions, no
  continuous output, and no keystroke access; assert the request enum admits
  nothing that mutates.
- [ ] Default scope is the requesting pane's tab, excluding the requester;
  `--include-self`, `--pane`, and `--window` adjust it. **Never** across windows.
- [ ] Capture panes concurrently under **one** 500 ms deadline for the whole
  request. A slow pane must not extend anyone else's. Test with a deliberately
  stalled pane.
- [ ] A pane that closes or fails mid-collection does not discard healthy
  snapshots: `complete` becomes false and the failure is named.

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
