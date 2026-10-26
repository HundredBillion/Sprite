# Authorized pane control

Status: approved by the owner on 2026-10-26; hardened against repository interfaces.

Issue: https://github.com/HundredBillion/Sprite/issues/106

Base: `921db1b` (`origin/master` when inspected on 2026-10-26).
Branch: `feature/issue-106-pane-control` in `.worktrees/issue-106-pane-control`.

## Outcome and scope

An agent can create, list, read, label, move, focus, resize and close authorized
Sprite panes, including panes already used by a person and panes in background
tabs. Being created by an agent is never an eligibility condition.

Deliver a documented public CLI and versioned protocol over local Unix sockets.
Reuse Terminal Sessions, Tabs/PaneTree, the shared local transport and the
workspace's admission and cleanup mechanisms. The canceled managed-command and
event roadmap is outside this change. A design alone does not close the issue.

## Approaches considered

1. **Separate pane-control adapter over existing local transport (recommended).**
   Its own credentials and grammar keep the existing read-only promise intact.
   Workspace operations serve both keyboard actions and addressed control.
   This adds an authorization flow and a small public protocol.
2. **Extend observation with mutation verbs and stronger credentials.** Fewer
   endpoints, but changes the deliberately read-only interface and requires
   multi-key authorization in transport that currently authenticates one key.
3. **Automate keyboard actions or extend Surface commands into pane control.**
   Reuses visible actions, but does not reliably address background panes and
   confuses pane management with a program's own Surface ownership.

## Authorization and owner experience

Control is unavailable until the owner approves a grant in Sprite's UI.
`sprite panes access request` asks for access; possession of observation or
Surface credentials never approves the request. A bounded pending request shows
the claimed requesting pane, requested targets and permissions. The requester
identity is self-reported and displayed as such; the observation credential
authenticates the containing window, not an individual program. It has explicit Allow
and Deny actions and expires after 60 seconds. Only one pending access request
per window is retained; another receives `busy`. Repeated requests never count
as approval. An access request must come through the containing window's
authenticated observation channel; observation being disabled prevents requests.

The owner chooses either an explicit set of pane identities or **all panes in
this window**, and separately enables:

- `read`: inventory, metadata and supported content.
- `manage`: create, label, move, focus, resize and close.
- `input`: explicit terminal input, independent of `manage`.

Manage and input grants include read. A read-only grant is also supported.
The approval UI shows that input can execute commands and that pane creation
starts a configured shell. Creation accepts no arbitrary executable or shell
command; an agent with manage alone can allocate a shell, but cannot type into it.
Existing observation credentials retain exactly their current read-only access.

Approval returns a fresh, opaque control credential to the waiting requester.
Do not inject it automatically into existing or newly created child environments.
A bearer may deliberately delegate it. This follows Sprite's existing local
capability model; it is not an OS sandbox against hostile code running as the
same user. Display scope and grant IDs in a Pane Access UI; never display tokens
in logs, configuration output or audit entries. The public client can store its
descriptor in an owner-only file supplied explicitly with `--access <file>`.

The owner can revoke a grant or remove manage/input rights in Pane Access.
Re-enabling rights after revocation requires a fresh approval and credential.
Rights are checked at the UI execution point, including work queued before
revocation. Work already committed before revocation remains committed.

All-panes grants cover future panes in that window. A fixed-pane grant excludes
unrelated future panes, and includes panes created using that grant. Closing a
pane never transfers its permission to a later pane. A newly opened window needs
its own approval. Grants expire when their window closes or Sprite restarts.

## Identity and windows

Public addresses combine a random window incarnation ID and a monotonically
allocated PaneId; tab addresses carry that same window ID. IDs are opaque to
clients. A restarted window cannot reuse an earlier public address.

Clients can retain multiple grant descriptors and explicitly select a window
with `--window <id>`. Each descriptor points to its authorized window's endpoint;
there is no global cross-window mutation broker. Listing returns only the
windows/panes covered by the supplied descriptors. A wrong window or forbidden
pane produces `denied` without leaking its existence. An authorized address that
has closed produces `gone`; nonexistent IDs that were never granted are denied.

Move supports reordering and reparenting within a tab and between tabs in the
same window, including a new tab. Cross-window moves return `unsupported` before
changing anything. Control of addressed panes in other authorized windows is
supported through their own descriptors; it does not imply transferring a
Terminal Session between windows.

Moves require permission for the moved pane and the destination pane/tab layout.
In fixed-pane scope, insertion beside an authorized destination pane is allowed;
an unrelated pane is not a valid anchor. No move may silently activate a
background tab; activation requires an explicit focus operation. If moving the
current pane removes its tab, the existing tab-selection fallback applies.

## Public operations

The CLI uses `sprite panes` subcommands. Every operation returns JSON with a
protocol version, request ID, outcome, and operation-specific result. A documented
`sprite-pane-control/1` JSON-lines grammar is the direct protocol counterpart.
Read requests return owned data; clients never receive a PTY or live UI handle.

| Command | Behavior |
| --- | --- |
| `list` | Window/tab/pane addresses, kind, label, child title, tab name, current pane, visibility, geometry, capabilities and lifecycle state. |
| `read --pane ID` | Metadata and bounded terminal text/history; explicit content availability for Surface-backed presentation. |
| `create --window ID` | Start the configured shell in a new tab of an authorized window. |
| `create --beside ID --side SIDE` | Split beside an addressed pane without changing unrelated focus. |
| `rename --pane ID --label TEXT` | Change a pane label; an explicit clear operation removes it. |
| `move --pane ID --beside ID --side SIDE` | Transfer the existing pane payload, retaining its identity and session. A new-tab destination is also supported. |
| `focus --pane ID` | Select the containing tab and its Current Pane and request focus for the target window. Return actual focus state; platform refusal is explicit. |
| `resize --pane ID --side SIDE --pixels N` | Set the addressed divider's position in logical pixels within its containing split. Return its clamped position and resulting pane allocations. |
| `close --pane ID` | Close through the addressed close gate and owned cleanup path. |
| `input --pane ID --text TEXT` | Submit bounded text as terminal paste with the input permission; unsafe paste requires human confirmation. |
| `input --pane ID --key KEY` | Encode one documented terminal key through existing terminal input semantics, with input permission. |

Pane labels, user Tab Names and child-provided Pane Titles are separate stored
values and separate response fields. A label can affect presentation without
overwriting the title reported by the child or the name assigned to the tab.

Resize modifies the persistent split ratio using the same allocation, divider
floor and clamping rules as mouse/keyboard resize. It never merely sends a PTY
resize that the next layout overwrites. Edge panes with no divider on the
requested side return `unsupported`. Background-tab geometry uses its owning
window's current viewport and the same tab-strip allocation rules. Window-size
changes subsequently resize panes normally; fixed pixel sizes are not promised.

## Supported content and input

The current production pane implementation is TerminalView. A hosted fill,
dock, overlay, grid or element Surface is content inside that pane, not another
pane kind. Inventory identifies this presentation and advertises capabilities.

Terminal panes support CRUD, terminal text reads and terminal input. Surface
presentations support the same containing-pane CRUD and underlying terminal
text reads. Arbitrary Surface trees, editor documents and raster content are not
represented as terminal text: reads explicitly report that native content is
unavailable through this channel. Surface ownership/update protocols remain
unchanged. Input always names the terminal target; it does not send synthetic
Surface events or imply that the Surface received the input.

For a hosted implementation without a content or input capability, list and
layout/label/focus/close still work through PaneHandle; read content and input
return `unsupported`. Do not silently drop unsupported requests. Create supports
the currently installed terminal pane factory, and rejects unknown kind names.

## Close, concurrency and bounded work

Addressed close inspects the actual target's `close_warning`, including when
the target is in a background tab. A busy target returns
`confirmation_required` and asks the person in Sprite. No unattended `--force`
or repeated close request answers the question. Human confirmation is tied to
that grant, target and operation, expires, and rechecks authorization and target
liveness before applying. Denial, expiry, revocation and target exit cannot
close a different pane. Idle panes close without a foreground-job question.

Closing removes the pane once and uses existing `begin_shutdown`, pending
cleanup ownership and bounded Terminal Session escalation. It grants no API to
signal an arbitrary host PID. The final-pane response must be sent before its
endpoint is canceled; existing pending cleanups still finish before shutdown.
The result distinguishes layout removal from cleanup completion.

Mutation uses a bounded lane drained fairly on GPUI. Reuse claim/abandon reply
semantics: timeout before claim means nothing changed; timeout after claim means
`outcome_unknown`, never a false refusal. Clients must inspect state rather than
automatically retry non-idempotent operations. A reply lost after commit also
has an unknown outcome. Request IDs correlate responses; they do not promise
durable deduplication or replay storage.

Initial bounds: 16 active grants/window, 16 connections/grant, 16 queued
control operations/window, 8 KiB request lines, 256-byte labels and 4 KiB input
payloads. Reads retain existing history and broker deadline bounds. Agent-created
resources count against a limit of 128 live panes and 16 pending cleanups/window;
agent creation/close refuses with `busy` before exceeding those bounds. Ordinary
keyboard creation keeps existing behavior, but counts toward the admission check
for later agent operations. No unbounded event stream, worker pool or audit log.

Return `ok`, `denied`, `gone`, `unsupported`, `invalid_request`, `busy`,
`confirmation_required`, `closing`, or `outcome_unknown` as appropriate.
Only successful commands exit zero; refusals have documented nonzero exits.
Legacy snapshot output and exit behavior remain unchanged. Operation responses
serve as observable mutation records without logging terminal text or credentials.

## Repository evidence and required implementation seams

- `cli.rs::Invocation` currently exposes snapshots but no pane CRUD commands.
- `observation/broker.rs::PaneSource` deliberately exposes only list/capture.
- `local_socket.rs::LocalSocket` provides private sockets, authentication,
  connection limits, cancellation and bounded initial reads. Reuse it.
- `workspace/reload.rs::Relayed` and `relay` implement claim/abandon semantics.
  Reuse them rather than inventing a managed-command subsystem.
- `tabs.rs` supports close-by-identity but split/focus/divider methods primarily
  act on the active tab. Add addressed variants shared with existing actions.
- `workspace/close_gate.rs::may_close` currently checks `CloseScope::Pane` through
  the focused pane. Background close needs an explicit target, not temporary
  focus switching or the natural-exit path as a safeguard bypass.
- `terminal_view.rs::PaneExit` captures its original `(TabId, PaneId)` and the
  event task sends it after exit. Resolve the live tab from PaneId on delivery
  so a moved pane still exits correctly.
- `observation/panes.rs::register` stores the original tab; `set_layout` updates
  geometry but not tab identity. Move must publish current membership without
  losing outstanding capture tickets or changing the session command sender.
- `workspace/mod.rs::refresh_layout` publishes persistent split allocation.
  Resize must update that source of truth and verify the next redraw.
- Surface requests already locate panes by PaneId in
  `workspace/surface_routing.rs`; same-window movement should preserve the
  existing handle, Surface connection and renderer state.

## Acceptance and stress-test cases for the TSP

The primary seam is the public client/protocol, backed by a real Workspace under
GPUI test support. Use existing internal tests for Terminal Session, Surface and
tree mechanics rather than reproducing those suites.

1. Owner approves all panes; client lists a pre-existing human pane and a
   background-tab pane, then creates, reads, labels, moves, focuses, resizes and
   closes a pane through public commands. Verify returned state, not just exit.
2. Observation-only credentials and denied access requests cannot mutate.
   Manage without input cannot type. Input without manage cannot close or create.
3. All-panes grants include a new human-created pane. Fixed-pane grants exclude
   it, retain their own newly created panes, and cannot select unrelated anchors.
4. Two granted windows contain the same local PaneId; explicit addresses act only
   in the named window. Wrong-window and restarted-window addresses fail safely.
5. Move a background pane between tabs while a capture is pending and a Surface
   is attached. Session identity/content persists, observation reports the new
   tab, and subsequent child exit closes that pane exactly once.
6. Resize, redraw, change active tab and redraw again. The accepted split ratio
   persists; no-divider and constrained allocations return documented results.
7. Busy background close requires a person. Retrying cannot confirm. Revoke
   while confirmation or queued control is pending; subsequent mutation fails.
8. Target exits between request and execution. Close a pane twice. Keep another
   pane running. No wrong target, extra shutdown or orphaned cleanup results.
9. Final-pane close replies before endpoint teardown and waits for pending
   cleanup. Shutdown refuses queued creates and control requests.
10. Exhaust grants, requests, payload sizes, pane creation and pending cleanup
    admission. Bounds hold, overflow is observable, keyboard behavior remains
    responsive, and abandoned operations never apply later.
11. Verify terminal-only and each supported Surface presentation; unsupported
    native content/input and unknown pane kinds return explicit errors.
12. Preserve ordinary keyboard split, focus, tab rename, divider dragging,
    unsafe-paste confirmation, pane close and quit behavior.

Delivery record: the owner approved this design on 2026-10-26. The focused TSP,
ADR 0034, implementation, public workflow checks and independent review are
complete in the isolated feature worktree. See
`docs/reviews/10-26-2026-pane-control.md` for verification and review evidence,
and `docs/pane-control.md` for the public contract. Desktop interaction is tested
under GPUI's test platform; no manual desktop run is claimed.
