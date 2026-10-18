# Scoped agent workspace authority Technical Spec

> **For agentic workers:** REQUIRED SUB-SKILL: Use dmi-superpowers:executing-plans to implement this TSP inline, with an independent whole-change review. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let a person launch one explicitly authorized agent in a dedicated Sprite Window, inspect its workspace identity, and revoke its access without granting access to human windows.

**Architecture:** Reuse window-local pane registries, Terminal Session environment sanitization, LocalSocket, observation and Surface protocols. A small shared authority object owns immutable identity/grants and bounded audit state; the GPUI workspace remains the authority for applying Surface operations and revocation. Add a status-only agent endpoint now; execution and pane mutation are separate later issues.

**Tech Stack:** Rust 1.97.1; pinned GPUI 0.2.2, serde/serde_json, async-channel and standard library. No new dependencies or transport.

**Status:** Owner approved the dedicated-window, single-principal launch model and revocation policy: remove Sprite permissions and disconnect Surfaces while terminal processes continue. Design decisions are resolved; implementation has not started. This TSP covers #103, not the command/output implementation of #104/#105.

**References:** [PRD AN-02](../PRDs/10-18-2026-ai-native-workflows.md), [#103](https://github.com/HundredBillion/Sprite/issues/103), [ADR 0033](../adr/0033-enroll-one-agent-principal-per-dedicated-window.md), [baseline evidence](../performance/ai-native-workflow-baseline.md).

## Global Constraints

- Support macOS and Linux.
- Use Rust 1.97.1 and the repository's pinned dependency versions.
- Sprite Terminal must remain fully functional without an agent, inference engine, replacement shell, or CPR tooling.
- Sprite Terminal should target the lowest practical CPU, RAM, and energy use during idle and active workloads, with measured improvements that preserve correctness, responsiveness, and required functionality.
- Reuse the existing Terminal Session, pane layout, local Unix-socket transport, bounded channels, and Surface modules unless measured evidence establishes a deficiency.
- Keep model execution, model weights, inference kernels, context assembly, and compiler semantics outside Sprite Terminal's rendering hot path.
- Preserve supported observation CLI/JSON and Surface Channel compatibility; new authority must not be inferred from legacy credentials.
- Never block the GPUI thread on socket I/O, process cleanup, model execution, or subscriber consumption.
- Bound retained output, subscriptions, queues, audit records, and connection resources; expose saturation, truncation, and replay gaps explicitly.
- Retain one libghostty-owning worker per Terminal Session and the existing safe ownership rules.
- Do not add HTTP or binary serialization without evidence that the existing Unix-socket and JSON approach is insufficient.

---

## Approved behavior and boundaries

```sh
sprite agent start --allow read --allow surface -- <program> [args...]
sprite agent start --allow read --config /path/to/config.toml -- <program> [args...]
sprite agent status
```

- `start` creates a new dedicated window/process using the existing native startup
  path. `--` and a nonempty program are required. No grants are implicit; omitting
  `--allow` permits identity/status inspection only. Duplicate grants are harmless.
  Accept `read` and `surface` now; reject `pane-management`, `input`, `execute`,
  `signal`, unknown options and unknown grants until their implementation exists.
  The startup command is an explicit human launch, not remotely managed execution.
- One random workspace ID and one independent random client ID per window lifetime;
  32 random bytes encoded as 64 lowercase hex characters, using the existing OS
  randomness helper. They are public identities, not bearer secrets. All child
  processes and manually added panes in this dedicated window share the principal.
- The new status credential is independent of observation and Surface credentials.
  Never accept an observation/Surface key as an agent key, or a client-supplied
  `--from` as identity. No global window enumeration or human-window conversion.
- Read uses existing `sprite panes snapshot`/JSON and the window's own registry.
  Grant `surface` permits existing Surface open/update/focus/close, capabilities
  and token registration within that dedicated window. It does not grant terminal
  input, process signaling or pane management. Both config print and reload via
  observation are denied in agent windows; native human config controls may still
  update settings without changing grants.
- New pane environments use the existing `configure_child_environment` stripping
  of **all** inherited `SPRITE_` variables. Inject only this window's active,
  explicitly granted endpoints plus status identity. Do not mutate process-global
  environment or add another environment scrubber.
- A native tab-strip control shows “Agent workspace” and the granted categories.
  “Revoke access” is mouse and keyboard reachable and changes to “Access revoked”.
  Keep the strip visible even for one tab. Terminal/Surface content cannot cover
  or replace this control. No re-enrollment within the same window in this increment.
- Revocation is an irreversible transition for this principal: deny new status
  and snapshot admissions; refuse queued Surface mutations before application;
  abort active Surface connections, discard their unsent events and remove those
  Surfaces, restoring terminal focus. Leave terminal processes and visible terminal
  output alive. Already admitted snapshot replies may finish; completed actions and
  bytes already delivered cannot be retracted. Do not claim revocation terminates
  programs or prevents direct filesystem/network access.
- Disconnecting a CLI connection does not revoke a grant; reconnect is allowed
  until revocation/window shutdown. Closing the window revokes before existing
  endpoint/session cleanup. No principal/grant restoration across application restart.
- Revoke authorization synchronously on the GPUI thread, without waiting for socket
  writes or worker joins. Existing endpoint listeners may remain bounded and return
  refusals until normal window shutdown; they cannot reauthorize the principal.
  Active Surface connections use nonblocking shutdown, not graceful event draining.
- Bound audit to 256 entries, containing only sequence, operation and allowed/denied;
  no commands, terminal text, keys, arguments or environment. On eviction, expose
  the earliest retained sequence and dropped count. Sequence exhaustion fails closed.
  Record enrollment/revocation and authenticated operation decisions. Failed bearer
  handshakes retain the transport's fixed refusal and are not attributed to a client.
  No background audit writer, file persistence, timers or per-pane authority objects.

## Existing seams and file ownership

| File | Change |
|---|---|
| `crates/sprite-app/src/agent.rs` | Public grant value, shared authority, bounded audit, status endpoint/client; keep under about 500 lines plus tests |
| `crates/sprite-app/src/cli.rs`, `main.rs`, `lib.rs` | Parse/start/status dispatch and exports |
| `crates/sprite-app/src/workspace/mod.rs` | Agent-aware constructor, shared authority/status endpoint, strip geometry and shutdown ordering |
| `crates/sprite-app/src/workspace/agent.rs` | Native grant indicator/revoke control and direct Surface revocation dispatch |
| `crates/sprite-app/src/workspace/pane_factory.rs` | Inject only active own-window credentials on every creation path |
| `crates/sprite-app/src/workspace/reload.rs` | Preserve agent read gate when observation toggles; no grant creation by config |
| `crates/sprite-app/src/observation/request.rs` | Snapshot-only responder that never relays config mutations |
| `crates/sprite-app/src/workspace/surface_routing.rs` | Grant/revocation check at application point, retaining closed-connection cleanup |
| `crates/sprite-app/src/surface/channel.rs` | Abort queued output; internal-only revoke request |
| `crates/sprite-app/src/terminal_view/surfaces.rs` | Abort/remove active Surfaces through existing close/focus paths |
| `crates/sprite-app/src/workspace/close_gate.rs` | Revoke principal before existing shutdown |
| `crates/sprite-app/src/workspace/agent_tests.rs`, `tests/client.rs` | Real protocol, native view and CLI checks |
| `docs/agent-workspaces.md` | Usage, grant/lifetime/revocation contract and limits |

`LocalSocket`, Terminal Session, `WindowPanes`, Surface lanes and worker code remain
owners of their current behavior. No generic policy engine, additional transport,
principal database or per-pane isolation is introduced.

### Task 1: Implement explicit identity, grants and bounded revocation state

**Blocked by:** None.

**Files:** Create `crates/sprite-app/src/agent.rs`; modify `crates/sprite-app/src/lib.rs`.

**Interfaces:**
- Consumes `ObservationKey::generate`, `crate::locked::locked`, serde_json.
- Produces `AgentGrants { read: bool, surface: bool }`,
  `AgentAuthority::new(AgentGrants) -> io::Result<Arc<AgentAuthority>>`,
  `permits(Operation) -> bool`, `active() -> bool`, `revoke()`,
  `status() -> serde_json::Value`. IDs are immutable `workspace_id: String` and
  `client_id: String`. `Operation` is `Status`, `Read`, `Surface`, `Config`,
  `PaneManagement`, `Input`, `Execute`, `Signal`.

- [ ] **Step 1: Write the authority behavior tests in `agent.rs`.**

```rust
#[test]
fn grants_do_not_escalate_and_revocation_is_permanent() {
    let authority = AgentAuthority::new(AgentGrants { read: true, surface: false }).unwrap();
    assert!(authority.permits(Operation::Read));
    for operation in [Operation::Surface, Operation::Config, Operation::PaneManagement,
                      Operation::Input, Operation::Execute, Operation::Signal] {
        assert!(!authority.permits(operation));
    }
    authority.revoke();
    assert!(!authority.active());
    assert!(!authority.permits(Operation::Status));
    assert!(!authority.permits(Operation::Read));
    authority.revoke();
    assert!(!authority.active());
}

#[test]
fn identities_are_incarnation_scoped_and_audit_is_bounded() {
    let first = AgentAuthority::new(AgentGrants::default()).unwrap();
    let second = AgentAuthority::new(AgentGrants::default()).unwrap();
    assert_ne!(first.workspace_id, second.workspace_id);
    assert_ne!(first.client_id, second.client_id);
    assert_ne!(first.workspace_id, first.client_id);
    for _ in 0..300 { assert!(!first.permits(Operation::Read)); }
    let status = first.status();
    assert_eq!(status["audit"].as_array().unwrap().len(), 256);
    assert!(status["audit_dropped"].as_u64().unwrap() > 0);
    assert_eq!(status["grants"]["execute"], false);
    assert!(!status.to_string().contains("key"));
}
```

Run `PATH="$HOME/.cargo/bin:$PATH" cargo test --locked --offline -p sprite-app --lib agent::tests`.
First run must fail because the module/types are absent. Add `mod agent;` in lib.rs
so tests are actually discovered; never count a zero-test result as the red/green loop.

- [ ] **Step 2: Implement the authority state.**

```rust
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use crate::local_socket::ObservationKey;
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AgentGrants {
    pub read: bool,
    pub surface: bool,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum Operation {
    Status, Read, Surface, Config, PaneManagement, Input, Execute, Signal,
}
impl Operation {
    fn name(self) -> &'static str {
        match self {
            Self::Status => "status", Self::Read => "read", Self::Surface => "surface",
            Self::Config => "config", Self::PaneManagement => "pane-management",
            Self::Input => "input", Self::Execute => "execute", Self::Signal => "signal",
        }
    }
}

struct AuditEntry { sequence: u64, operation: &'static str, allowed: bool }
struct AuthorityState {
    revoked: bool,
    next: u64,
    dropped: u64,
    audit: VecDeque<AuditEntry>,
}
impl AuthorityState {
    fn record(&mut self, operation: &'static str, allowed: bool) -> bool {
        let Some(next) = self.next.checked_add(1) else {
            self.revoked = true;
            return false;
        };
        if self.audit.len() == 256 {
            self.audit.pop_front();
            self.dropped = self.dropped.saturating_add(1);
        }
        self.audit.push_back(AuditEntry { sequence: self.next, operation, allowed });
        self.next = next;
        allowed
    }
}

pub(crate) struct AgentAuthority {
    pub(crate) workspace_id: String,
    pub(crate) client_id: String,
    pub(crate) grants: AgentGrants,
    state: Mutex<AuthorityState>,
}
impl AgentAuthority {
    pub(crate) fn new(grants: AgentGrants) -> std::io::Result<Arc<Self>> {
        let mut state = AuthorityState {
            revoked: false, next: 1, dropped: 0, audit: VecDeque::with_capacity(256),
        };
        state.record("enroll", true);
        Ok(Arc::new(Self {
            workspace_id: ObservationKey::generate()?.to_hex(),
            client_id: ObservationKey::generate()?.to_hex(),
            grants, state: Mutex::new(state),
        }))
    }
    pub(crate) fn active(&self) -> bool { !crate::locked::locked(&self.state).revoked }
    pub(crate) fn permits(&self, operation: Operation) -> bool {
        let mut state = crate::locked::locked(&self.state);
        let allowed = !state.revoked && match operation {
            Operation::Status => true,
            Operation::Read => self.grants.read,
            Operation::Surface => self.grants.surface,
            Operation::Config | Operation::PaneManagement | Operation::Input
            | Operation::Execute | Operation::Signal => false,
        };
        state.record(operation.name(), allowed)
    }
    pub(crate) fn revoke(&self) {
        let mut state = crate::locked::locked(&self.state);
        if !state.revoked {
            state.revoked = true;
            state.record("revoke", true);
        }
    }
    pub(crate) fn status(&self) -> Value {
        let state = crate::locked::locked(&self.state);
        json!({
            "schema_version": 1, "workspace_id": self.workspace_id,
            "client_id": self.client_id, "active": !state.revoked,
            "grants": {"read": self.grants.read, "surface": self.grants.surface,
                "pane-management": false, "input": false, "execute": false, "signal": false},
            "audit_first": state.audit.front().map(|entry| entry.sequence),
            "audit_dropped": state.dropped,
            "audit": state.audit.iter().map(|entry| json!({
                "sequence": entry.sequence, "operation": entry.operation,
                "allowed": entry.allowed,
            })).collect::<Vec<_>>()
        })
    }
}
```

Extend tests for no grants, Surface-only, audit sequence exhaustion (set `next` to
`u64::MAX` within the module test), and concurrent revoke/read decisions. Keep
status serialization bounded by the fixed audit capacity. Export only `AgentGrants`
from lib.rs; authority remains crate-private.

- [ ] **Step 3: Verify and commit.** Run the focused command and `cargo fmt --all -- --check`.
Expect nonzero discovered tests and no failures. Commit the module/tests/exports
as `feat: define explicit agent workspace grants and revocation`.

### Task 2: Expose only authenticated workspace status and explicit launch syntax

**Blocked by:** Task 1.

**Files:** Extend `agent.rs`; modify `cli.rs`, `main.rs`, `lib.rs`; extend `tests/client.rs`.

**Interfaces:**
- Produces `AgentEndpoint::open(Arc<AgentAuthority>) -> io::Result<Self>`,
  `open_in(PathBuf, Arc<AgentAuthority>)`, `environment() -> Vec<(OsString, OsString)>`;
  `run_agent_status(&mut dyn Write, &mut dyn Write) -> u8`.
- `WindowArgs` adds `agent: Option<AgentGrants>`; `Invocation` adds `AgentStatus`.
- Status wire line: `<key> sprite-agent/1 status <workspace_id> <client_id>\n`.
  Reply is one JSON line or the fixed `denied\n` refusal. Unknown versions, stale
  identity, unsupported verbs and legacy keys cannot disclose workspace details.

- [ ] **Step 1: Add real-socket and CLI parser tests before implementation.**

In `agent.rs` tests use `UnixStream` with the actual `AgentEndpoint`, not a mock
of `permits`. The fixture directory is `temp_dir()/format!("sa-{}-{}", pid, counter)`
with a process-local `AtomicUsize`; remove it after dropping the endpoints.

```rust
fn ask(endpoint: &AgentEndpoint, key: &str, body: &str) -> String {
    use std::io::{Read, Write};
    let mut socket = std::os::unix::net::UnixStream::connect(endpoint.transport.socket_path()).unwrap();
    crate::timeouts::bound_client(&socket).unwrap();
    writeln!(socket, "{key} {body}").unwrap();
    socket.shutdown(std::net::Shutdown::Write).unwrap();
    let mut answer = String::new();
    socket.read_to_string(&mut answer).unwrap();
    answer
}

#[test]
fn an_agent_key_is_bound_to_its_client_and_workspace_incarnation() {
    let root = std::env::temp_dir().join(format!("sa-{}-identity", std::process::id()));
    let authority = AgentAuthority::new(AgentGrants::default()).unwrap();
    let endpoint = AgentEndpoint::open_in(root.clone(), authority.clone()).unwrap();
    let key = endpoint.transport.key_hex();
    let valid = format!("sprite-agent/1 status {} {}", authority.workspace_id, authority.client_id);
    let status: Value = serde_json::from_str(&ask(&endpoint, &key, &valid)).unwrap();
    assert_eq!(status["workspace_id"], authority.workspace_id);
    assert_eq!(ask(&endpoint, &"0".repeat(64), &valid), "denied\n");
    assert_eq!(ask(&endpoint, &key, "sprite-agent/1 status stale stale"), "denied\n");
    assert_eq!(ask(&endpoint, &key, &valid.replace("status", "execute")), "denied\n");
    authority.revoke();
    assert_eq!(ask(&endpoint, &key, &valid), "denied\n");
    drop(endpoint);
    std::fs::remove_dir_all(root).unwrap();
}
```

Parser assertions (place in existing CLI tests):

```rust
assert!(matches!(parse_arguments(["agent", "status"]), Ok(Invocation::AgentStatus)));
assert!(parse_arguments(["agent", "start", "--allow", "execute", "--", "sh"]).is_err());
assert!(parse_arguments(["agent", "start", "--allow", "read"]).is_err());
let Invocation::Window(args) = parse_arguments([
    "agent", "start", "--allow", "read", "--allow", "surface", "--", "sh", "-l"
]).unwrap() else { panic!("agent start must open a window") };
assert_eq!(args.agent, Some(AgentGrants { read: true, surface: true }));
assert_eq!(args.command, Some(vec!["sh".into(), "-l".into()]));
```

Run focused `agent::tests` and `cli::tests`; expect missing endpoint/invocation
errors before implementation, then exercise actual denial assertions.

- [ ] **Step 2: Add the status endpoint using the existing transport.**

```rust
use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::PathBuf;
use crate::local_socket::{LocalSocket, TransportPolicy};

pub(crate) struct AgentEndpoint {
    transport: LocalSocket,
    authority: Arc<AgentAuthority>,
}
impl AgentEndpoint {
    pub(crate) fn open(authority: Arc<AgentAuthority>) -> std::io::Result<Self> {
        Self::open_in(crate::local_socket::runtime_directory()?, authority)
    }
    pub(crate) fn open_in(directory: PathBuf, authority: Arc<AgentAuthority>) -> std::io::Result<Self> {
        let serving = authority.clone();
        let transport = LocalSocket::open_in(
            directory, Arc::new(ObservationKey::generate()?),
            TransportPolicy {
                name: "sprite-agent", filename_hex: 24, suffix: ".agent.sock",
                max_connections: 16, max_first_line: 8192,
                handshake_timeout: crate::timeouts::SILENT_CLIENT_TIMEOUT,
                write_timeout: crate::timeouts::SILENT_CLIENT_TIMEOUT,
            },
            |stream| { writeln!(stream, "denied").ok(); },
            move |mut connection| {
                let words: Vec<_> = connection.body.split_whitespace().collect();
                let response = match words.as_slice() {
                    ["sprite-agent/1", verb, workspace, client]
                        if *workspace == serving.workspace_id && *client == serving.client_id => {
                        let operation = match *verb {
                            "status" => Some(Operation::Status),
                            "pane-management" => Some(Operation::PaneManagement),
                            "input" => Some(Operation::Input),
                            "execute" => Some(Operation::Execute),
                            "signal" => Some(Operation::Signal),
                            _ => None,
                        };
                        match operation {
                            Some(operation) if serving.permits(operation) && *verb == "status" => serving.status().to_string(),
                            _ => "denied".to_owned(),
                        }
                    }
                    _ => "denied".to_owned(),
                };
                writeln!(connection.stream, "{response}").ok();
            },
        )?;
        Ok(Self { transport, authority })
    }
    pub(crate) fn environment(&self) -> Vec<(OsString, OsString)> {
        if !self.authority.active() { return Vec::new(); }
        vec![
            ("SPRITE_AGENT_SOCKET".into(), self.transport.socket_path().as_os_str().to_owned()),
            ("SPRITE_AGENT_KEY".into(), self.transport.key_hex().into()),
            ("SPRITE_AGENT_WORKSPACE".into(), self.authority.workspace_id.clone().into()),
            ("SPRITE_AGENT_CLIENT".into(), self.authority.client_id.clone().into()),
        ]
    }
}

pub fn run_agent_status(out: &mut dyn Write, errors: &mut dyn Write) -> u8 {
    let result = (|| -> Result<String, String> {
        let value = |name| std::env::var(name).ok().filter(|s| !s.is_empty())
            .ok_or_else(|| "not running in an enrolled agent workspace".to_owned());
        let socket = value("SPRITE_AGENT_SOCKET")?;
        let key = value("SPRITE_AGENT_KEY")?;
        let workspace = value("SPRITE_AGENT_WORKSPACE")?;
        let client = value("SPRITE_AGENT_CLIENT")?;
        if [&key, &workspace, &client].iter().any(|s| s.len() != 64 || !s.bytes().all(|b| b.is_ascii_hexdigit())) {
            return Err("invalid agent credentials".into());
        }
        let mut stream = std::os::unix::net::UnixStream::connect(socket).map_err(|e| e.to_string())?;
        crate::timeouts::bound_client(&stream).map_err(|e| e.to_string())?;
        writeln!(stream, "{key} sprite-agent/1 status {workspace} {client}").map_err(|e| e.to_string())?;
        stream.shutdown(std::net::Shutdown::Write).map_err(|e| e.to_string())?;
        let mut response = String::new();
        stream.take(131073).read_to_string(&mut response).map_err(|e| e.to_string())?;
        if response.len() > 131072 { return Err("agent response is too large".into()); }
        let json: Value = serde_json::from_str(&response).map_err(|_| "agent access refused".to_owned())?;
        if json["schema_version"] != 1 || json["workspace_id"] != workspace
            || json["client_id"] != client || json["active"] != true {
            return Err("incompatible agent status".into());
        }
        Ok(json.to_string())
    })();
    match result {
        Ok(response) => if writeln!(out, "{response}").is_ok() { 0 } else { 1 },
        Err(error) => { writeln!(errors, "sprite: {error}").ok(); 1 }
    }
}
```

Keys never appear in diagnostics. The read is size bounded and uses the existing
socket timeout policy; do not claim a total wall deadline from inactivity timeouts.

- [ ] **Step 3: Wire the CLI without starting GPUI for status.**

In `cli.rs` import `AgentGrants`, add `agent: Option<AgentGrants>` to defaultable
`WindowArgs`, `AgentStatus` to `Invocation`, and an `agent` match arm:

```rust
Some("agent") => match arguments.next().as_deref().and_then(text).as_deref() {
    Some("status") if arguments.next().is_none() => Ok(Invocation::AgentStatus),
    Some("start") => agent_start(arguments).map(Invocation::Window),
    _ => Err(UsageError("use: sprite agent start [--allow read|surface] -- <program> [args], or sprite agent status".into())),
},
```

Add this parser; values after `--` remain `OsString`, including non-UTF-8 arguments:

```rust
fn agent_start(mut arguments: impl Iterator<Item = OsString>) -> Result<WindowArgs, UsageError> {
    let mut parsed = WindowArgs::default();
    let mut grants = AgentGrants::default();
    while let Some(argument) = arguments.next() {
        match argument.to_str() {
            Some("--allow") => match arguments.next().as_deref().and_then(OsStr::to_str) {
                Some("read") => grants.read = true,
                Some("surface") => grants.surface = true,
                _ => return Err(UsageError("available agent grants: read, surface".into())),
            },
            Some("--config") => parsed.config = Some(arguments.next().ok_or_else(|| UsageError("--config needs a path".into()))?.into()),
            Some("--") => {
                let command: Vec<_> = arguments.collect();
                if command.is_empty() || command[0].is_empty() { break; }
                parsed.command = Some(command);
                parsed.agent = Some(grants);
                return Ok(parsed);
            }
            _ => return Err(UsageError("unknown agent start option".into())),
        }
    }
    Err(UsageError("agent start requires -- <program> [args]".into()))
}
```

Export `run_agent_status` and `AgentGrants`. `main` dispatches AgentStatus with
stdout/stderr and returns its exit code, before Application creation. Add usage
lines matching the examples. Update explicit `WindowArgs` test literals with
`agent: None`; do not change ordinary `-e` or `--config` parsing.

- [ ] **Step 4: Verify socket/CLI refusal and commit.** Add CLI integration cases
using the existing subprocess fixtures in `tests/client.rs`: absent credentials,
wrong key, mismatched identity, unsupported schema, bounded oversized response,
non-UTF-8 startup argv and `--help`. No GUI is required for status.

Run `cargo test --locked --offline -p sprite-app --lib agent::tests`, `cargo test
--locked --offline -p sprite-app --lib cli::tests`, and `cargo test --locked
--offline -p sprite-app --test client` with the pinned PATH. Commit as
`feat: enroll agent windows through explicit launch and status commands`.

### Task 3: Apply grants to window creation, observation and every pane environment

**Blocked by:** Tasks 1–2.

**Files:** `workspace/mod.rs`, `workspace/pane_factory.rs`, `workspace/reload.rs`,
`observation/request.rs`, `main.rs`; create `workspace/agent_tests.rs`.

**Interfaces:**
- Preserve `Workspace::new(...) -> Self` for human windows.
- Add `Workspace::new_agent(command: Vec<OsString>, settings: Settings,
  config_path: Option<PathBuf>, grants: AgentGrants, window: &mut Window,
  cx: &mut Context<Self>) -> io::Result<Self>`.
- Shared private constructor takes the existing arguments plus
  `Option<Arc<AgentAuthority>>` and returns `io::Result<Self>`.
- Workspace fields `agent: Option<Arc<AgentAuthority>>` and
  `agent_endpoint: Option<AgentEndpoint>`; pane services add borrowed equivalents.
- `respond_agent(panes: &dyn PaneSource, body: &str, authority: &AgentAuthority) -> String`.

- [ ] **Step 1: Add protocol tests that distinguish agent mode from human mode.**

Extend observation responder tests using their existing PaneSource fixtures and
real Endpoint tests. The critical negative is a valid agent-window observation
key sending `sprite-observation/1 config reload`: the reload receiver must remain
empty. Config print is denied too. A snapshot with a read grant still returns the
unchanged snapshot JSON. A Surface-only or revoked principal receives `denied`.

Add these direct state assertions to the responder tests (the existing fixture's
`PaneSource` is passed as `panes`):

```rust
let authority = AgentAuthority::new(AgentGrants { read: true, surface: false }).unwrap();
assert_eq!(respond_agent(panes, "sprite-observation/1 config reload", &authority), DENIED);
assert_eq!(respond_agent(panes, "sprite-observation/1 config print", &authority), DENIED);
authority.revoke();
assert_eq!(respond_agent(panes, "sprite-observation/1 panes snapshot --window", &authority), DENIED);
```

The real socket test sends the same lines through the endpoint and checks no
reload was enqueued; do not replace that public-path test with helper assertions.
First run should expose the current config-relay behavior before the agent gate.

- [ ] **Step 2: Split only the snapshot response body out of `respond`.**

Move the existing `parse(body)`/`broker::collect`/`schema::render` tail verbatim to
`fn respond_snapshot(panes: &dyn PaneSource, body: &str) -> String`; have the human
`respond` call it after its unchanged version/config handling. Add:

```rust
pub(crate) fn respond_agent(
    panes: &dyn PaneSource,
    body: &str,
    authority: &crate::agent::AgentAuthority,
) -> String {
    let Ok(body) = protocol_check(body) else { return DENIED.to_owned(); };
    if config_request(body).is_some() {
        authority.permits(crate::agent::Operation::Config);
        return DENIED.to_owned();
    }
    if !authority.permits(crate::agent::Operation::Read) { return DENIED.to_owned(); }
    respond_snapshot(panes, body)
}
```

Preserve `open_endpoint(panes, reload)` as a human-mode wrapper for existing tests.
Its shared opener takes an optional authority, capturing it in the real endpoint:

```rust
pub(super) fn open_scoped_endpoint(
    panes: &Arc<WindowPanes>,
    reload: &async_channel::Sender<ReloadRequest>,
    authority: Option<Arc<crate::agent::AgentAuthority>>,
) -> Option<Endpoint> {
    if authority.as_ref().is_some_and(|a| !a.active() || !a.grants.read) { return None; }
    let panes = Arc::clone(panes);
    let reload = reload.clone();
    Endpoint::open(move |request| match &authority {
        Some(authority) => crate::observation::request::respond_agent(panes.as_ref(), &request.body, authority),
        None => crate::observation::request::respond(panes.as_ref(), &reload,
            &request.body, Some(request.reply_connection)),
    }).ok()
}
```

Both constructor and `change_observation` call the shared opener with the
workspace's actual authority. Reloading configuration must never create an
unrestricted endpoint in an agent window, grant read, or revive a revoked key.

- [ ] **Step 3: Construct all required endpoints before launching the program.**

In the private constructor, retain the existing panes/reload/surface lanes, then:

```rust
let agent_endpoint = agent.as_ref().map(|a| crate::agent::AgentEndpoint::open(a.clone())).transpose()?;
let endpoint = settings.pane_observation.enabled
    .then(|| open_scoped_endpoint(&panes, &reload_tx, agent.clone())).flatten();
let allow_surface = agent.as_ref().is_none_or(|a| a.active() && a.grants.surface);
let surfaces = if allow_surface {
    crate::observation::endpoint::ObservationKey::generate().ok().map(Arc::new)
        .and_then(|key| SurfaceEndpoint::open(key, surface_lanes).ok())
} else { None };
if let Some(authority) = &agent {
    if (authority.grants.read && (!settings.pane_observation.enabled || endpoint.is_none()))
        || (authority.grants.surface && surfaces.is_none()) {
        return Err(std::io::Error::other("could not establish requested agent grants"));
    }
}
```

A requested read grant with observation disabled is a clear startup error, not an
implicit config change. Only after this block call `Tabs::new(make_pane(...))`.
Store `agent` and `agent_endpoint` with the workspace. Human constructor delegates
with `None`; its endpoint degradation behavior remains unchanged. Agent constructor
creates authority once and delegates with `Some(authority)`. In main, choose the
constructor using `WindowArgs.agent`; report any agent startup error on stderr and
exit 1 before an agent child is spawned. Never fall back to an ordinary human window.

Extend `PaneServices` with `agent: Option<&AgentAuthority>` and
`agent_endpoint: Option<&AgentEndpoint>`, filling them in initial/new-tab/split call
sites. In `session_environment`, build credentials only while active:

```rust
if agent.is_some_and(|authority| !authority.active()) { return Vec::new(); }
let endpoint = endpoint.filter(|_| agent.is_none_or(|authority| authority.grants.read));
let surfaces = surfaces.filter(|_| agent.is_none_or(|authority| authority.grants.surface));
let mut environment = endpoint.map(|e| e.environment(tab, pane)).unwrap_or_default();
environment.extend(surfaces.into_iter().flat_map(|e| e.environment(tab, pane)));
environment.extend(agent_endpoint.into_iter().flat_map(|e| e.environment()));
environment
```

Human call sites pass `None` for both new options. The already existing worker
strips parent-window credentials before overlaying this vector. Test initial,
split and new-tab paths, including after revocation. Do not add Rust environment
mutation to tests: use a subprocess for polluted ambient-environment cases.

- [ ] **Step 4: Verify through real endpoints and preserve compatibility.**

Run observation tests, existing `tests/client.rs`, native workspace startup/layout
tests, and the existing Terminal Session environment-isolation tests. Add an agent
startup test with deliberately failing endpoint creation and assert the requested
program's sentinel file was not created. Human `config reload` remains reachable
with its existing observation credentials; agent config relay remains empty.
Commit as `feat: enforce agent grants across observation and credential inheritance`.

### Task 4: Revoke pending Surface actions and queued events at the real application seam

**Blocked by:** Task 3.

**Files:** `surface/channel.rs`, `terminal_view/surfaces.rs`,
`workspace/surface_routing.rs`, `workspace/agent.rs`, `workspace/agent_tests.rs`.

**Interfaces:**
- `SurfaceConnection::abort(&self)` discards pending events and shuts down both
  directions without joining or draining the writer.
- Internal `SurfaceRequest::Revoke { pane: PaneId }`; no wire-parser mapping.
- `Workspace::revoke_agent(&mut self, window: &mut Window, cx: &mut Context<Self>)`.

- [ ] **Step 1: Prove queued output cannot survive revocation.**

Add this real UnixStream-pair test in `surface/channel.rs`, before implementation:

```rust
#[test]
fn abort_discards_events_before_establishment_and_refuses_new_events() {
    use std::io::Read;
    let (server, mut client) = std::os::unix::net::UnixStream::pair().unwrap();
    client.set_read_timeout(Some(std::time::Duration::from_secs(2))).unwrap();
    let connection = SurfaceConnection::new(&server).unwrap();
    assert!(connection.send("sensitive event"));
    connection.abort();
    assert_eq!(connection.pending_bytes(), 0);
    assert!(!connection.send("late event"));
    assert!(!connection.establish("opened"));
    let mut bytes = Vec::new();
    client.read_to_end(&mut bytes).unwrap();
    assert!(bytes.is_empty());
}
```

- [ ] **Step 2: Reuse the existing nonblocking kill/close paths.**

```rust
pub(crate) fn abort(&self) {
    let shared = &self.handle.shared;
    shared.kill(crate::locked::locked(&shared.queue), true);
}
```

Add `Revoke { pane: PaneId }` to `SurfaceRequest`. Include it in `pane()`'s
`Some(*pane)` alternatives, `claim()`'s true alternatives and `refuse_with()`'s
no-op alternatives. No `first_line`/stream parser or client JSON may construct it.
Add this arm to TerminalView's Surface request handler:

```rust
SurfaceRequest::Revoke { .. } => {
    let ids: Vec<_> = self.surfaces.iter().map(|surface| {
        surface.connection.abort();
        surface.id
    }).collect();
    for id in ids { self.close_surface(id, window, cx); }
}
```

At `Workspace::serve_surface_request`, retain `request.claim()` and `stopping`
handling. Before the existing dispatch match, check authority on the GPUI thread:

```rust
if !matches!(&request, SurfaceRequest::Closed { .. })
    && self.agent.as_ref().is_some_and(|a| !a.permits(crate::agent::Operation::Surface)) {
    request.refuse_with(Refusal::Ineligible);
    return;
}
```

`Closed` is cleanup from a dead connection, never a new authority grant. Native
revocation bypasses that queued ingress entirely and calls the owning pane handle:

```rust
pub(super) fn revoke_agent(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    let Some(authority) = &self.agent else { return; };
    authority.revoke();
    for (_, pane, handle) in self.tabs.all_panes() {
        handle.surface_request(SurfaceRequest::Revoke { pane }, window, cx);
    }
    cx.notify();
}
```

Add the internal Revoke variant to the existing routing match for exhaustiveness,
but never expose it through the Surface protocol. Keep existing unknown-pane and
ownership checks: a numeric target is resolved only in this window's registry.
Revoking must not close a terminal, invoke shutdown escalation or send a signal.

- [ ] **Step 3: Exercise queued work and Surface ownership using GPUI tests.**

In `workspace/agent_tests.rs`, use actual SurfaceEndpoint/SurfaceLanes and the
native Workspace (not a replacement fake policy handler). Queue Open/FocusPane
without draining GPUI, revoke via the native method, then drain: the reply must
be refused and no Surface/focus change applied. For an already open Surface,
queue events without reading them, revoke, assert connection death, zero pending
bytes and terminal focus restored. Subsequent Update/Grid/List/Focus/TokenRegister
requests must not change UI state. Deliver Closed afterward and confirm cleanup
still completes. A granted Surface request before revoke must succeed.

Run the new abort test, all `surface::channel::tests`, native Surface routing and
existing `terminal_view::surfaces` tests. Commit as
`feat: revoke agent Surface access without stopping terminal sessions`.

### Task 5: Make grants and native revocation visible without changing human layouts

**Blocked by:** Tasks 3–4.

**Files:** `workspace/agent.rs`, `workspace/mod.rs`, `workspace/keymap.rs`,
`workspace/pane_factory.rs`, `workspace/close_gate.rs`, `workspace/agent_tests.rs`.

**Interfaces:** `agent_control(&self, &mut Context<Self>) -> Option<gpui::Div>`;
`layout_tab_count(tabs: usize, agent: bool) -> usize` reserves the existing strip
for an agent window even with one tab. No new UI overlay or Surface kind.

- [ ] **Step 1: Add geometry and input checks.**

```rust
#[test]
fn the_agent_strip_reserves_space_without_changing_human_windows() {
    assert_eq!(layout_tab_count(1, false), 1);
    assert_eq!(layout_tab_count(1, true), 2);
    assert_eq!(layout_tab_count(3, true), 3);
    let viewport = gpui::size(gpui::px(960.0), gpui::px(640.0));
    assert_eq!(pane_area(viewport, layout_tab_count(1, false)).2, 0.0);
    assert_eq!(pane_area(viewport, layout_tab_count(1, true)).2, TAB_STRIP_HEIGHT);
}
```

Native GPUI test: show one agent tab, inspect `agent-access` element, click revoke,
verify authority inactive and session still alive. Repeat via Ctrl+Shift+A, with
pending terminal text, and prove that the shortcut is not sent to the child.
Human-window Ctrl+Shift+A must retain its previous terminal behavior. Repeated
keypress/click is idempotent and must not allocate cleanup threads.

- [ ] **Step 2: Add the native indicator and reuse the existing tab-strip geometry.**

```rust
fn layout_tab_count(tabs: usize, agent: bool) -> usize {
    if agent { tabs.max(2) } else { tabs }
}
```

Use that count at every `pane_area`/`pane_allocation` production call: initial
allocation, split, new tab, cached pane area and refresh_layout. Keep actual Tabs
membership/count unchanged. Human paths pass false and preserve exact geometry.
Inside the existing strip div, append `.children(self.agent_control(cx))` after tab
children. Add `mod agent;` to workspace modules. The control implementation:

```rust
use super::*;
impl Workspace {
    pub(super) fn agent_control(&self, cx: &mut Context<Self>) -> Option<gpui::Div> {
        let authority = self.agent.as_ref()?;
        let label = if !authority.active() { "Agent workspace · Access revoked" }
        else { match (authority.grants.read, authority.grants.surface) {
            (true, true) => "Agent workspace · Read + Surfaces · Revoke access (Ctrl+Shift+A)",
            (true, false) => "Agent workspace · Read · Revoke access (Ctrl+Shift+A)",
            (false, true) => "Agent workspace · Surfaces · Revoke access (Ctrl+Shift+A)",
            (false, false) => "Agent workspace · No grants · Revoke access (Ctrl+Shift+A)",
        }};
        Some(div().debug_selector(|| "agent-access".into()).px(px(12.0)).h_full()
            .text_size(px(12.0)).text_color(rgb(TAB_ACTIVE_FG))
            .on_mouse_down(gpui::MouseButton::Left, cx.listener(|workspace, _, window, cx| {
                workspace.revoke_agent(window, cx);
                cx.stop_propagation();
            })).child(label))
    }
}
```

Add `RevokeAgentAccess` to `WorkspaceAction`, with `repeats()` false and
Ctrl+Shift+A parsed using the existing modifier rules. In `key_down`, filter that
action to `None` when `self.agent.is_none()`, then use the existing cancellation,
held-key suppression and capture path. Apply it with
`self.revoke_agent(window, cx)`. If rename/confirmation modes intercept it, handle
revocation before those modes while retaining the shared pending-input cancellation
and stop-propagation behavior. Native control is outside pane content, so a
Surface cannot register or trigger this action through JSON.

At the start of `begin_shutdown`, after its already-stopping guard, revoke the
shared authority before existing endpoint/session cleanup. Drop the status endpoint
as part of that existing endpoint lifetime; do not allow a status listener to outlive
the window. `revoke_agent` itself never joins listeners or process workers.

- [ ] **Step 3: Run layout, shortcut, Surface and shutdown checks; commit.**

Run `cargo test --locked --offline -p sprite-app --lib workspace::` with pinned
PATH. Require the new agent cases and existing startup/layout/quit/rename tests to
run. Commit as `feat: show and revoke agent grants through native window controls`.

### Task 6: Prove the full authority boundary and document the usable increment

**Blocked by:** Tasks 1–5.

**Files:** `workspace/agent_tests.rs`, `tests/client.rs`, `docs/agent-workspaces.md`,
`docs/TSPs/10-18-2026-ai-native-delivery-map.md`; existing ADR/glossary entries.

**Interfaces:** Public `sprite agent start/status`, observation CLI and Surface
Channel, backed by the actual window registry and GPUI dispatcher.

- [ ] **Step 1: Complete the adversarial workflow matrix.**

Run real authenticated socket requests against a human window and an agent window
with distinct sentinel pane content. Use the agent's issued credentials only;
substitute target socket paths/claimed IDs while keeping those credentials. This
proves isolation rather than merely testing an invalid-key helper. Exercise the
actual Surface endpoints and native dispatch before asserting no mutation.

| Case | Required observation |
|---|---|
| No grant | Status identifies only its own workspace; observation/Surface env absent; no content/mutation authority |
| Read only | Own-window snapshot succeeds; config relay is empty; Surface mutation fails |
| Surface only | Own-window Surface succeeds; observation absent; no terminal input authority |
| Human observation endpoint + agent read key | Fixed refusal; human sentinel absent |
| Human Surface endpoint + agent Surface key | Fixed refusal; no human draw/focus/token change |
| Legacy observation or Surface key + agent status endpoint | Fixed refusal, including otherwise correct IDs |
| Correct agent key + stale workspace/client ID | Refused, including after a new window reuses pane number 0 |
| Wrong/missing Surface target in own window | Existing target/ownership refusal; no other pane changes |
| Revoked principal | Status/read refused; queued Surface work denied; established Surface events aborted |
| Config toggle after revoke | No new agent credentials or resurrected endpoints/grants |
| New tab/split after revoke | Terminal still works, no active agent/observation/Surface credentials injected |
| Parent process has human `SPRITE_*` credentials | Agent child sees only issued own-window credentials; existing worker sanitizer tested through child env output |
| Human compatibility | Existing snapshot/config/Surface behavior and JSON remain supported |
| Agent disconnect/reconnect | Grant survives short-lived client connections; close/restart invalidates identities and keys |
| Execution/input/signal/pane control verbs | Refused; no program marker, terminal bytes, signal or layout mutation |

Use bounded waits and sentinels, never timing sleeps as proof of rejection. Negative
requests must be followed by inspection of actual human pane state/focus, not just
a client nonzero exit. Do not use terminal text as the oracle for process completion.

- [ ] **Step 2: Write usage and policy documentation.**

`docs/agent-workspaces.md` must contain the exact start/status commands, explicit
read/Surface grant semantics, separate human windows, shared-principal descendants,
Ctrl+Shift+A/native revoke, no in-place re-enrollment, stable workspace/client-qualified
pane identity, audit retention/gaps, configuration denial, disconnect/shutdown policy,
and startup failure behavior. State that already admitted reads/completed operations
may finish and that revocation does not terminate processes. Do not advertise remote
execution until #104, full pane operations until #106, or mixed scopes until #113.

Describe real socket/GPUI tests separately from native display tests. David owns
native Linux execution on his separate machine; keep that evidence explicitly open
in #102/#112. Linux build/test CI remains active. Never claim a same-user OS sandbox.

- [ ] **Step 3: Run checks and collect review evidence.**

```sh
PATH="$HOME/.cargo/bin:$PATH" cargo fmt --all -- --check
PATH="$HOME/.cargo/bin:$PATH" cargo clippy --workspace --all-targets --locked --offline -- -D warnings
PATH="$HOME/.cargo/bin:$PATH" cargo test --workspace --locked --offline --no-fail-fast
python3 -m unittest scripts.test_workflow_baseline scripts.test_ci_policy -v
git diff --check
```

Use the exact toolchain and existing terminfo setup. Record failures by name; do
not weaken watchdogs or mark unrelated failures passed. The previous CI macOS
`event_backpressure::input_is_admitted_while_every_output_permit_is_outstanding`
timeout did not reproduce in ten targeted local reruns; its CI rerun remains a
separate check. The earlier Linux dependency fetch failed with network unreachable.
Neither result is evidence for or against new authority behavior.

After committing source, capture the existing release component benchmarks with
the #102 driver into fresh directories and compare allocation invariants. Report
added idle listener/state costs separately from terminal hot-path metrics. Native
macOS smoke: explicitly launch a harmless agent program, inspect native grant
indicator, run own status/snapshot, revoke, confirm refusal and continuing terminal
use. Record actual evidence or leave native interaction unverified.

Commit tests/docs as `test: prove agent authority across legacy and native paths`.
Request independent whole-change review with this TSP, source PRD, ADR 0033 and
execution ledger. Resolve substantive findings and verify affected behavior. Use
one feature PR, stacked on PR #114 if its baseline branch remains unmerged. Never
merge either PR automatically. Map all six #103 acceptance criteria to evidence;
only close the issue if every required criterion is actually established.

## Self-review and design stress test

- Coverage: Tasks 1–2 provide explicit enrollment/identity; Task 3 covers inheritance,
  observation and configuration; Tasks 4–5 cover queued mutations, outgoing events,
  native revoke and terminal continuity; Task 6 covers the public boundary and
  human compatibility. Execution/input/signal/pane verbs remain unavailable.
- Trust test: A human Surface key is not intentionally given to an agent; guessing
  another pane ID or swapping socket paths cannot change the registry/key owning
  the request. An arbitrary same-user process that steals human secrets is outside
  this application-authority boundary, as the approved PRD states.
- Revocation test: Surface admission before revoke is insufficient; the actual
  GPUI application point checks current authority. Closed cleanup remains valid.
  Active event writers abort queued output rather than draining it to the agent.
- Lifetime test: No configuration reload re-enrolls, revives old credentials or
  assigns an old workspace/client identity to a replacement window.
- Efficiency: One authority object and one additional bounded status endpoint per
  enrolled window; fixed audit ring; no per-pane tokens, extra periodic tasks,
  generic ACL framework, actor runtime or new dependency.
- Scope: One principal per dedicated window was explicitly approved. Separate
  principals in one window require a future design; #113 is not silently included.
- Integration: Native Linux validation is owner-run on a separate host, not waived.
  Existing CI failures are tracked separately; they are not a reason to invent
  green native evidence.
