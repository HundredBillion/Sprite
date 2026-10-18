# Sprite Terminal: secure, observable AI-native coding workflows

Parent initiative: [GitHub issue #94](https://github.com/HundredBillion/Sprite/issues/94).

Status: implementation requirements synthesized from the strategy, the source audit of
v0.2.8 (`3fbdfedf`), and the project owner's workflow instructions on 18 October
2026. The owner confirmed the public CLI/protocol workflow as the primary testing
seam. Detailed TSPs are prepared immediately before their corresponding work begins.

## Problem Statement

Local coding agents can observe Sprite Terminal panes, but they cannot securely
manage their own work through a supported control interface. Repeated snapshots
make agents consume output they have already seen, while command completion,
diagnostics, and test outcomes are not available as a coherent structured result.
People cannot yet follow and authorize those workflows through dedicated native
views. Existing native rendering, shell integration, process management, and local
transport already solve much of the infrastructure problem and must be preserved.

Success means completing representative coding tasks with less repeated context
and lower measured time/resource cost, while maintaining terminal correctness,
responsiveness, and independence from any AI runtime. Higher terminal FPS or model
tokens per second alone does not establish success.

## Solution

Give an explicitly authorized agent its own workspace, allow it to launch and
control managed work there, and expose bounded incremental observations with
reliable identity, lifecycle, provenance, and recovery. Present diagnostics, tests,
diffs, task progress, and optional inference telemetry using existing Surfaces.

Deliver one useful workflow first: authorize a client, launch a managed command in
a dedicated agent Sprite Window, observe its output and exit, and show structured
results. Extend pane control and conventional shell integration after that path
works. Mixed human/agent groups inside a single Sprite Window are a later optional
scope requiring a deliberate change to the existing window trust model.

## User Stories

1. As a developer, I want Sprite Terminal to remain useful without an agent or model, so that ordinary terminal work remains independent.
2. As a developer, I want existing terminal input, graphics, selection, scrollback, and process cleanup preserved, so that automation does not regress daily work.
3. As a developer, I want current behavior and resource use measured before changes, so that improvements have a credible baseline.
4. As a developer, I want working terminal and Surface modules reused, so that this initiative does not create duplicate infrastructure.
5. As a developer, I want to grant an agent explicit workspace authority, so that it can work without receiving control of my sessions.
6. As a developer, I want to revoke that authority, so that new requests stop promptly and active work follows a documented lifecycle policy.
7. As an agent, I want stable client, workspace, pane, and command identities, so that results cannot be confused across sessions or reconnects.
8. As an agent, I want to list only the resources I may see, so that discovery respects my permissions.
9. As an agent, I want to create panes with a program, arguments, working directory, and environment, so that each task starts in the intended context.
10. As an agent, I want to rename, move, focus, resize, and close authorized panes, so that I can organize ongoing work.
11. As a developer, I want pane labels, tab names, and child-provided titles to remain distinct, so that automation does not overwrite unrelated naming.
12. As an agent, I want resize requests to have clear layout semantics, so that the next frame does not silently undo them.
13. As an agent, I want separate permission for terminal input, execution, and signaling, so that read or layout access does not grant arbitrary execution.
14. As a developer, I want consequential actions tied to explicit grants or trusted approvals, so that an untrusted Surface cannot approve its own action.
15. As an agent, I want a managed command's start, completion, exit status, and output identified together, so that I can decide the next action reliably.
16. As an agent, I want incremental output with a continuation cursor, so that I do not repeatedly retrieve the entire pane.
17. As an agent, I want replay gaps and retention limits reported explicitly, so that lost output is never presented as a complete result.
18. As a developer, I want slow subscribers bounded and isolated, so that they cannot stall input or redraw.
19. As an agent, I want cancellation, disconnect, timeout, and shutdown outcomes stated precisely, so that retrying does not duplicate an accepted action.
20. As a developer, I want a bounded audit trail of mutations, so that I can understand which client performed an action and its outcome.
21. As an existing observation client, I want supported snapshot behavior preserved, so that upgrades do not break my tools.
22. As a Surface client, I want existing descriptions and event behavior preserved, so that editor and plugin integration keeps working.
23. As an agent, I want conventional shell command metadata where integration is available, so that I can distinguish command status from the shell process exiting.
24. As an agent, I want missing or untrusted metadata identified honestly, so that I do not treat text heuristics as reliable process facts.
25. As an agent, I want structured compiler diagnostics and test results with raw-output references, so that I can act on concise results and inspect the source evidence.
26. As an agent, I want stable diagnostic and repository/context identities, so that unchanged context can be reused without hiding changed facts.
27. As a developer, I want clickable diagnostic/test results and readable diffs, so that I can follow an agent's work in native UI.
28. As a developer, I want task progress and approval controls to stay responsive during terminal output, so that supervision remains practical.
29. As a local-model user, I want optional inference telemetry, so that I can observe throughput, memory, cache reuse, and draft acceptance when the runtime supplies them.
30. As an agent-runtime developer, I want a versioned optional integration contract, so that context assembly and model execution remain outside Sprite Terminal.
31. As a future shell or CPR tooling author, I want to integrate through typed results and existing Surfaces, so that my implementation need not link to GPUI or alter the terminal parser.
32. As a maintainer, I want native macOS and Linux coverage, so that platform-specific lifecycle and display behavior is not assumed from headless tests.
33. As a maintainer, I want comparative task success, time, token, and resource measurements, so that a faster but less correct workflow is not called an improvement.
34. As a maintainer, I want each work item to include its own tests and documentation, so that a closed issue represents a complete usable increment.

## Implementation Decisions

### Global constraints

- Support macOS and Linux.
- Use Rust 1.97.1 and the repository's pinned dependency versions.
- Sprite Terminal must remain fully functional without an agent, inference engine, replacement shell, or CPR tooling.
- Reuse the existing Terminal Session, pane layout, local Unix-socket transport, bounded channels, and Surface modules unless measured evidence establishes a deficiency.
- Keep model execution, model weights, inference kernels, context assembly, and compiler semantics outside Sprite Terminal's rendering hot path.
- Preserve supported observation CLI/JSON and Surface Channel compatibility; new authority must not be inferred from legacy credentials.
- Never block the GPUI thread on socket I/O, process cleanup, model execution, or subscriber consumption.
- Bound retained output, subscriptions, queues, audit records, and connection resources; expose saturation, truncation, and replay gaps explicitly.
- Retain one libghostty-owning worker per Terminal Session and the existing safe ownership rules.
- Do not add HTTP or binary serialization without evidence that the existing Unix-socket and JSON approach is insufficient.

### Required outcomes

**AN-01 — Audit and measurement.** Persist the source-grounded capability inventory
and establish reproducible baselines for terminal activity and agent observation.
Record commit, platform, hardware, workload, sample count, and method. Separate
report-schema checks, headless CPU paint, native frame presentation, and actual
end-to-end task measurements. Missing measurements remain explicitly unverified.

**AN-02 — Workspace authority.** Establish authenticated client identity, stable
workspace identity, scoped grants, revocation, and a documented lifetime policy.
First delivery uses dedicated agent Sprite Windows. A client receives no automatic
authority over human windows. Separate read, pane management, input, execution,
signal, and Surface authority. Address credential inheritance and existing
observation/configuration/Surface paths together; a new interface alone is not an
isolation model. Identifiers are stable for their stated lifetime and stale
identifiers cannot target a replacement resource. This is application authority,
not an operating-system sandbox against arbitrary same-user processes.

**AN-03 — Managed commands.** Launch an explicitly specified program and arguments
with per-command cwd/environment in an authorized workspace. Use existing Terminal
Session process ownership and bounded shutdown. Report accepted, started,
completed, cancelled, or failed outcomes with command identity and provenance.
Define timeout/retry semantics so a request reported as unapplied cannot execute
later. Distinguish direct-child exit from commands inside an interactive shell.
Input and signaling require their own grants. Retain terminal functionality when
no control client is connected.

**AN-04 — Incremental observations.** Add a bounded output/event journal with
monotonic sequence, versioned envelopes, command/output correlation, continuation
cursors, reconnect/replay rules, and explicit retention gaps. Preserve raw output
within stated limits. Keep the existing render snapshot path independent;
terminal generations are not durable output sequence numbers. Slow external
subscribers must not block the session's existing lifecycle consumer or rendering.

**AN-05 — Pane operations.** Expose authorized create/list/read/rename/move/focus/
resize/close operations through a typed control interface and bundled CLI. Reuse
internal operations; define labels versus child titles, address background panes,
and make resize consistent with the UI's pixel-derived terminal geometry. Preserve
session identity and cleanup during moves and closure; define cross-workspace
moves against ownership and grants before supporting them.

**AN-06 — Conventional shell metadata.** Reuse Bash, Zsh, and Fish integration to
report command boundaries and completion status. Metadata absent from a shell is
unknown, not reconstructed from visible prose. Distinguish shell-reported facts
from managed-process facts and test fragmented protocol input and shell lifecycle
edge cases. Respect the existing restriction on Ghostty changes to exposing
already implemented behavior; do not create a second terminal parser.

**AN-07 — Structured results and context.** Provide a versioned diagnostic/test
result envelope and bounded adapters for a deliberately limited initial set of
machine-readable producers. Each fact identifies its producer, command, source
location where present, and raw-output reference or retention gap. Malformed
records must not erase raw evidence or invent success. Provide stable identities
and repository/changed-file provenance suitable for an external context assembler.
Stable prefix placement and actual KV cache reuse remain agent/runtime concerns.

**AN-08 — Native supervision.** Use existing Surface descriptions and events for
initial diagnostics, tests, diffs and task progress. Add kinds only for a real view
that cannot be expressed with current primitives. Trusted approvals bind to the
exact action and authority; externally supplied content never grants approval or
executes UI code. Preserve accessibility, focus, input, and Surface ownership
behavior while high-volume output is active.

**AN-09 — Optional inference telemetry.** Define a small adapter for metrics a real
runtime actually supplies: request identity, time, token counts, memory, cache
reuse and speculative acceptance. Unsupported metrics remain absent. Disconnects
and model stalls cannot interrupt terminal use. No inference implementation is
introduced into Sprite Terminal.

**AN-10 — Optional language integration evaluation.** Evaluate typed shell and CPR
integration against actual external protocol availability. Reuse command/result
and Surface contracts where sufficient. Record supported grammar/completion,
diagnostic/test, and artifact lifecycle capabilities; implement only a bounded
adapter when the producer exists. Do not invent a compiler or an engine API to
finish a terminal issue. An evaluated, documented deferral is a valid result of
this exploratory work, not a claim of a working integration.

**AN-11 — Acceptance evidence.** Compare full snapshots against incremental and
structured results on fixed workloads. Track success, median completion time,
input tokens, CPU/RAM and frame behavior; measure energy where the platform makes
it available and label missing measurements. Benchmark idle, high-output,
multiple-pane, create/close, large-history, concurrent-observer, and Surface-update
scenarios. Define numerical budgets from Phase 0 evidence before evaluating the
changed implementation; do not move thresholds to conceal regressions. Do not
claim superiority to Ghostty without a matched comparison.

**AN-12 — Later mixed-workspace option.** Separate human/agent groups within one
Sprite Window are optional follow-up scope. Revisit the window-scoped observation
ADR explicitly and test all legacy and privileged paths together before claiming
isolation. Dedicated-window delivery does not imply this outcome is implemented.

### Delivery process

One PRD owns product scope. GitHub sub-issues own deliverable scope, dependencies,
acceptance criteria, and evidence. One focused TSP is written just before each
sub-issue enters implementation; it references this PRD and actual prerequisite
interfaces. Each TSP is self-reviewed and stress-tested with DMI-superpowers,
then implemented, independently reviewed, and verified in an isolated worktree.
Use one reviewable PR per deliverable unless a dependency genuinely requires a
stack. No release/version bump or automatic merge is implied by this initiative.

Do not create all detailed TSPs up front. Do not split implementation from its
required tests, documentation, or migration compatibility. Parallel work is
appropriate only after prerequisite interfaces are stable and file ownership is
independent. Optional research is not a blocker for the first usable workflow.

## Testing Decisions

The owner confirmed the public CLI/protocol as the primary acceptance seam: an
authorized client launches a command, consumes incremental results, and cannot
reach human panes. Tests exercise actual authorization, dispatch, and process
results through that seam, rather than proving that internal helpers call each
other. Existing Terminal Session and Surface interfaces remain the test seams for
their respective behavior.

- Reuse real local socket and CLI fixtures, pane/layout behavior tests, Terminal Session lifecycle tests, event-pressure tests, and GPUI Surface/input tests.
- Add adversarial public-workflow tests for wrong scope, inherited legacy credentials, revoked grants, stale IDs, spoofed targets and exact-action approval binding.
- Exercise disconnects, slow subscribers, saturation, timeout before/after admission, concurrent close/control, shutdown, and replay after retention loss.
- Test command facts against actual spawned processes; test shell facts against shell integration. Never use screen text as the completion oracle.
- Test diagnostic/test adapters using representative producer fixtures, malformed records, partial input, unknown fields and retained raw evidence.
- Verify wire compatibility through existing clients and version/refusal behavior, not only serializer snapshots.
- Measure native UI behavior separately from headless tests; document any platform/display limitation and keep the corresponding acceptance evidence open.
- Preserve baseline task outcomes and success rate while comparing token/resource/time metrics. Performance claims require repeatable release-build measurements.
- Use regression-first tests for new behavior and fixes where the test can distinguish the required outcome; avoid tests that merely mirror implementation.

## Out of Scope

- Implementing the Rust/Mojo inference engine, model kernels, MTP/EAGLE/System One or MoE research.
- Building a new shell or CPR compiler/language server within Sprite Terminal.
- Replacing the terminal parser, PTY pump, pane tree, or Surface renderer without an identified deficiency.
- OS/filesystem sandboxing, remote network control, arbitrary executable UI plugins, or general-purpose browser rendering.
- Persisted/restored agent process sessions across application restarts in the initial delivery.
- Broad language/diagnostic coverage beyond explicitly selected adapters in their TSPs.
- Mixed human/agent groups within one window as a requirement of the first milestone.

## Further Notes

The source audit found a mature terminal and native UI foundation but no public
pane-mutation protocol, principal-scoped workspace authorization, or bounded
command/output journal. Existing prompt marks and working-directory metadata are
reusable; shell completion status is emitted but not exposed as a command result.
Both observation and Surface credentials currently use a window trust scope.

Estimated scope for the initial bounded feature set is 8–16k production lines
added/reworked, 8–16k test/measurement lines, and 2–5k external adapter/client lines.
These are planning judgments, not patch commitments, and exclude external engine
and language implementations. Approximately 2–4k production lines may rework
existing code; most work is additive. The first vertical milestone is smaller.

The companion source inventory supplies exact anchors without freezing volatile
file paths inside these product implementation decisions. The delivery map links
requirements to GitHub sub-issues and records readiness rather than pretending
that future TSPs or unavailable measurements are complete.
