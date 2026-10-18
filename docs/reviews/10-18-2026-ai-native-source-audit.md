# AI-native workflow source inventory

Audited 18 October 2026 at `3fbdfedf30a5ee752375c2d94c8a556ba50e706c`
(v0.2.8), verified against remote HEAD at audit time. Parent: #94.
This is source evidence, not a fresh performance certification.

| Subsystem | Classification | Existing evidence | Remaining work |
|---|---|---|---|
| Terminal engine / PTY lifecycle | Implemented; preserve | `crates/sprite-term/src/session.rs:328`, `pty_unix.rs:1`, `worker/closing.rs`; lifecycle/session-jobs tests | Managed command identity and controlled external access |
| GPUI rendering / redraw | Implemented; needs profiling for new loads | `crates/sprite-app/src/terminal_view/render.rs:423`, `grid_paint.rs`, `paint_benchmark.rs:1` | Native multi-pane presentation measurements |
| Pane management / application state | Partial for this initiative | `crates/sprite-app/src/tabs.rs:108`, `pane_tree.rs:415`, `workspace/mod.rs:87` | External controls, pane labels/move, agent ownership |
| Internal channels / tasks | Implemented | `crates/sprite-term/src/session.rs:1`, `event_mailbox.rs`, `crates/sprite-app/src/surface/lanes.rs:12` | Subscriber isolation; reuse bounded dispatch |
| Observation / authentication | Partial | `crates/sprite-app/src/observation/request.rs:24`, `schema.rs:352`, `endpoint.rs:1` | Principal scopes, deltas, replay and subscriptions |
| IPC / serialization | Implemented foundation | `crates/sprite-app/src/local_socket.rs:145`, `surface/wire.rs` | New privileged contract; no transport rewrite justified |
| Surface Channel / native UI | Implemented primitives | `crates/sprite-app/src/surface/description.rs:20`, `host.rs:1`, `wire.rs:351` | Workflow producers and trusted approval semantics |
| Output buffering / scrollback | Implemented bounded terminal state; journal missing | `crates/sprite-term/src/pty_unix.rs:7`, `snapshot.rs:88`, `pane.rs:24` | Raw output retention and stable command/output sequence |
| External pane control | Missing | `crates/sprite-app/src/cli.rs:181` accepts snapshot only for panes | Typed authorized lifecycle/input/process operations |
| Extension / automation | Partial | Surface clients, configuration reload relay, Bash/Zsh/Fish integration | Agent adapters, structured results, optional model/language bridges |

## Architectural facts that affect scope

- `Workspace` is a Sprite Window root, not an agent ownership domain.
- Pane IDs are monotonic within a window and start at zero; an external identity must also identify its workspace/lifetime.
- `Tabs` already creates/splits/closes/focuses panes and adjusts dividers. Naming currently applies to tabs; pane titles come from children. Move/reparent is not already a public layout operation.
- The pane factory copies a window-wide command; Terminal Session configuration already supports individual program/arguments/cwd/environment.
- Terminal geometry is derived from allocated pixels. A direct PTY rows/columns update alone can be undone by layout.
- Observation is read-only for pane operations, with a configuration-reload write exception. Its window key and self-reported source pane are not principal-scoped authority.
- Surface credentials are also shared per window (`surface/channel.rs:763`), and routing selects the supplied pane (`workspace/surface_routing.rs:47`). Foreground PID checks provide lifecycle/focus eligibility, not a client grant model.
- Bash/Zsh/Fish emit OSC 133 completion status. `TerminalEvent` has direct-child exit, CWD/title and other events, but no per-shell-command completion contract.
- The pinned libghostty-rs interface has no exposed command-start/completion callback. Shell reporting or a narrow supported binding extension requires design; screen scraping is not an equivalent source.
- `EventStream` is single-owner; render snapshots are latest-only. External replay must not block or compete with the installed UI consumer.
- Existing Surface descriptions support box/text/list/image/button/grid/virtual-list data and native event handling. A second AI-specific renderer is unnecessary.

## Architectural decisions to preserve or deliberately revisit

- ADR 0001: supported protected observation remains available without an LLM.
- ADR 0003: Ghostty changes expose existing behavior; no undocumented parser fork.
- ADR 0006 / 0010: reduced observation and rich rendering projections have different contracts; lifecycle and latest-only snapshots have different delivery rules.
- ADR 0008: one terminal owner per session.
- ADR 0013: current observation scope is the window. Dedicated agent windows preserve this decision; mixed privilege in one window reopens it.
- ADR 0017 / 0018: descriptions, not executable UI code; Surface and observation grammars remain separate while sharing transport mechanics.
- ADR 0024 / 0025 / 0026: bounded cleanup, ordinary session job ownership, and event/admission recovery.
- ADR 0028 / 0029 / 0030: request correlation and claim-before-apply; blocking I/O off GPUI; bounded control lanes.

## Measurement inventory and limits

Existing binaries measure terminal, graphics, observation, paint and startup paths.
`crates/sprite-term/tests/benchmark.rs:14` lists 11 required terminal metrics;
its test at line 69 validates report shape, not production performance budgets.
`crates/sprite-app/src/bin/sprite-observation-bench.rs:9` uses stand-in panes.
`crates/sprite-app/src/paint_benchmark.rs:1` excludes window layout, glyph
rasterization and GPU submission. Historical performance reports are useful
reproduction references, not evidence about the new workflow or today's machine.

Phase 0 must add/reconcile live socket and multi-pane measurements, workload
identity, CPU/RAM/native frame behavior, output/token volume and task outcomes.
Actual model prefill/speculation measurements belong to the external runtime.
Unsupported energy or native display measurements remain explicitly unverified.

## Checks run during the source audit

- `cargo test -p sprite-term --test integration_metadata --locked --offline`: five tests passed.
- `python3 -m unittest scripts.test_shell_integration -v`: five tests passed; four Fish tests skipped because Fish was unavailable.
- Local HEAD matched remote HEAD; original working tree was clean.
- No broad suite or fresh performance comparison was run for this source audit.

First-party Rust inventory: 76,194 physical lines in 157 files across the three
crates, including inline and integration tests. Vendors/build output excluded.
Planning estimates are recorded in the PRD; they are not measured diffs.
