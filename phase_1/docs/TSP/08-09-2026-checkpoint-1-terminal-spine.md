# Sprite Terminal Checkpoint 1 Technical Spec

> **For agentic workers:** REQUIRED SUB-SKILL: Use dmi-superpowers:subagent-driven-development (recommended) or dmi-superpowers:executing-plans to implement this TSP task-by-task. Steps use checkbox (- [ ]) syntax for tracking.

**Goal:** Deliver one runnable Sprite desktop window whose keystrokes travel through the owned Terminal Session interface to a real PTY/login shell and whose libghostty state returns as coherent owned snapshots for GPUI rendering.

**Architecture:** sprite-app owns GPUI and one Terminal Session; sprite-term owns the PTY, child, terminal-owner worker, libghostty objects, and snapshot conversion. A bounded standard-library worker queue preserves command/output order and reserves capacity for non-output work; bounded async-channel streams bridge the worker to GPUI. On Linux and macOS, a PTY pump blocks in the OS readiness primitive on the PTY and a cancellation socket, while a separate child waiter reports one exit status; neither helper touches libghostty. Lossless lifecycle events and one-slot latest-only snapshots travel on separate streams. Checkpoint 1 creates the permanent seam and runnable path; later checkpoints deepen it without replacing it.

**Tech Stack:** Rust 1.97.1, Zig 0.16.0, Cargo resolver 3, GPUI =0.2.2, libghostty-vt =0.2.1, Ghostty source commit ab0b9da9e88fcb4b0533a1854e84628f663930af, portable-pty =0.9.0, async-channel =2.5.0, and the already-transitive nix =0.28.0 with Sprite directly enabling poll/process/signal on Unix (plus portable-pty's existing term/fs feature requests).

## Global Constraints

- Work on branch phase_1 inside the existing Sprite repository. phase_1/ is a workspace directory, not another Git repository.
- Keep exactly two product crates: sprite-term and sprite-app. Benchmark binaries live in those crates.
- Do not depend on gpui-ghostty, tty7, Croft, Omarchy, an async runtime, a logging facade, anyhow, or thiserror.
- Do not add unsafe Send or Sync. Every libghostty value is created, used, and dropped on the terminal-owner worker.
- Test behavior through TerminalSession; do not assert FFI calls, private channels, private worker messages, or borrowed Ghostty types.
- Use bounded queues. PTY bytes and keyboard input are ordered and lossless.
  Snapshot construction and delivery may coalesce to the newest complete
  generation; lifecycle events may not.
- Keep RenderSnapshot and PaneSnapshot as separate owned projections created during one capture. Neither projection is made from the other.
- Make dependency acquisition an explicit bootstrap step. After that step, all
  local Cargo checks and tests run `--locked --offline`; no test may download
  source. `.cargo/config.toml` must force libghostty-vt-sys to use
  `phase_1/vendor/ghostty`.
- Linux builds enable GPUI native Wayland and X11. macOS builds use GPUI's native macOS backend.
- Each red-green cycle ends green before refactoring. Commit after every task using the exact commit subject shown.
- Stop if the pinned Ghostty source requires changes to parsing, terminal semantics, allocator ownership, or thread ownership. Such a change requires architectural review under ADR 0003.

---

---

## Checkpoint 1 acceptance status

**NOT ACCEPTED.** Tasks 1-8 are complete and verified on Arch Linux. Tasks 9 and
10 are partially complete: everything that can be done without macOS hardware or
a Croft build is done, and the rest is marked **OUTSTANDING** in place.

Blocking items, all requiring resources unavailable to the workspace that wrote
this:

- Real-macOS build, test, product smoke, and idle inspection (Tasks 8, 9, 10).
- Ghostty comparison at the identical pinned commit (Task 9).
- Croft moving-main capability smoke, never executed (Task 10).
- Human review of ownership, shutdown, and platform parity (Task 10).
- Native X11 smoke is only partial, and two upstream GPUI defects were found
  there: an unterminated `WM_CLASS`, and `HasWindowHandle` being
  `unimplemented!()`. Neither is fixable from Sprite; see Task 8 for detail.

Checkpoint 2 may not begin until these are satisfied.

---

## Checkpoint boundary

Checkpoint 1 includes:

- a reproducible two-crate workspace and exact upstream pins;
- one Terminal Session, real PTY, explicit child command, resolved login shell, and child reaping;
- ordered byte input and ordered cell/pixel resize;
- libghostty output ingestion and terminal-generated PTY replies;
- coherent owned render and pane projections from one generation;
- one GPUI window with basic monospaced text, basic key input, resize, and event-driven invalidation;
- deterministic Terminal Session integration tests;
- a repeatable benchmark harness and recorded Arch/macOS baselines and budgets.

Checkpoint 1 does not claim the text shaping, full key/mouse protocol, selection, scrolling, search, shell integration, accessibility, tabs/splits, Pane Observation IPC, Kitty graphics, or packaging acceptance assigned to later checkpoints.

## PRD traceability

| PRD requirement | Checkpoint 1 evidence |
|---|---|
| Stories 1-2: launch a real resolved login shell and expose failures | Tasks 2 and 6; lifecycle and identity tests |
| Story 9: resize reaches the PTY | Task 5; real stty integration test |
| Story 10: reap children | Task 7; PID liveness and join tests |
| Story 54: hide PTY/libghostty details | Architectural interface plus forbidden-import gate in Task 10 |
| Story 55: owned worker messages | Tasks 2-4; public-interface integration tests and ownership review |
| Story 56: deterministic snapshots | Task 3; coherent render/pane projection test |
| Story 57: event-driven invalidation | Tasks 3 and 8; awaitable event/snapshot streams, demand-triggered capture, and no-poll scan |
| Stories 58-59: locked inputs and both operating systems | Tasks 1, 8, and 10; exact pins, lockfile, Arch/macOS gates |
| Checkpoint 1 end-to-end path | Tasks 1-8; one GPUI window through TerminalSession, worker, libghostty, PTY, and shell |
| Checkpoint 1 numerical budgets before Checkpoint 2 | Task 9; committed Arch/macOS and Ghostty comparison evidence |
| ADRs 0003, 0006, 0008, 0009, 0010, and 0011 | Global constraints, snapshot types, worker ownership, exact source pin, separate delivery semantics, and cancelable PTY reads |

The remaining user stories stay assigned to Checkpoints 2-5 exactly as the PRD
states. This TSP creates no alternate path for them.

## Architectural interfaces

Create this public interface in sprite-term. Names, field types, and ownership are fixed for this TSP; implementation-only types remain private.

~~~rust
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerminalSize {
    pub rows: u16,
    pub cols: u16,
    pub cell_width_px: u32,
    pub cell_height_px: u32,
}

impl TerminalSize {
    pub const DEFAULT: Self = Self {
        rows: 24,
        cols: 80,
        cell_width_px: 8,
        cell_height_px: 16,
    };

    pub fn pixel_width(self) -> u16;
    pub fn pixel_height(self) -> u16;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionConfig {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub working_directory: Option<PathBuf>,
    pub environment: Vec<(OsString, OsString)>,
    pub size: TerminalSize,
    pub max_scrollback: usize,
}

impl SessionConfig {
    pub fn command(program: impl Into<PathBuf>, args: Vec<OsString>) -> Self;
    pub fn login_shell() -> Result<Self, SessionError>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KeyModifiers {
    pub shift: bool,
    pub alt: bool,
    pub control: bool,
    pub platform: bool,
    pub function: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyAction {
    Press,
    Repeat,
    Release,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KeyEvent {
    pub logical_key: String,
    pub text: Option<String>,
    pub modifiers: KeyModifiers,
    pub action: KeyAction,
    pub composing: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TerminalCommand {
    Key(KeyEvent),
    Input(Vec<u8>),
    Resize(TerminalSize),
    Capture,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScreenKind {
    Primary,
    Alternate,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SnapshotColor {
    Default,
    Palette(u8),
    Rgb(Rgb),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnderlineStyle {
    None,
    Single,
    Double,
    Curly,
    Dotted,
    Dashed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CellWidth {
    Narrow,
    Wide,
    SpacerTail,
    SpacerHead,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CellStyle {
    pub foreground: SnapshotColor,
    pub background: SnapshotColor,
    pub underline_color: SnapshotColor,
    pub bold: bool,
    pub italic: bool,
    pub faint: bool,
    pub blink: bool,
    pub inverse: bool,
    pub invisible: bool,
    pub strikethrough: bool,
    pub overline: bool,
    pub underline: UnderlineStyle,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RenderCell {
    pub text: String,
    pub width: CellWidth,
    pub style: CellStyle,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RenderRow {
    pub cells: Vec<RenderCell>,
    pub wrapped: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CursorSnapshot {
    pub row: u16,
    pub column: u16,
    pub visible: bool,
    pub blinking: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RenderSnapshot {
    pub generation: u64,
    pub size: TerminalSize,
    pub rows: Vec<RenderRow>,
    pub cursor: CursorSnapshot,
    pub default_foreground: Rgb,
    pub default_background: Rgb,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PaneRow {
    pub text: String,
    pub wrapped: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PaneSnapshot {
    pub generation: u64,
    pub size: TerminalSize,
    pub screen: ScreenKind,
    pub rows: Vec<PaneRow>,
    pub cursor: CursorSnapshot,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SnapshotBundle {
    pub generation: u64,
    pub render: Arc<RenderSnapshot>,
    pub pane: Arc<PaneSnapshot>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChildExit {
    pub code: Option<u32>,
    pub signal: Option<String>,
    pub requested: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TerminalEvent {
    Ready,
    Exited(ChildExit),
    Error(SessionError),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionError {
    pub operation: &'static str,
    pub message: String,
}

pub struct EventStream;

impl EventStream {
    pub async fn next(&mut self) -> Result<TerminalEvent, SessionError>;
    pub fn next_blocking(&mut self) -> Result<TerminalEvent, SessionError>;
}

pub struct SnapshotStream;

impl SnapshotStream {
    pub async fn next(&mut self) -> Result<Arc<SnapshotBundle>, SessionError>;
    pub fn next_blocking(&mut self) -> Result<Arc<SnapshotBundle>, SessionError>;
}

pub struct ShutdownHandle;

impl ShutdownHandle {
    pub fn wait(self) -> Result<(), SessionError>;
}

pub struct TerminalSession;

impl TerminalSession {
    pub fn spawn(config: SessionConfig) -> Result<Self, SessionError>;
    pub fn take_event_stream(&mut self) -> Result<EventStream, SessionError>;
    pub fn take_snapshot_stream(&mut self) -> Result<SnapshotStream, SessionError>;
    pub fn send(&mut self, command: TerminalCommand) -> Result<(), SessionError>;
    pub fn begin_shutdown(&mut self) -> Result<Option<ShutdownHandle>, SessionError>;
}
~~~

Interface invariants:

- TerminalSession::spawn returns after the worker starts; Ready means the PTY, child, and terminal are live.
- EventStream is single-owner and lossless for Ready, Exited, and Error.
  SnapshotStream is separately single-owner and carries only the newest complete
  snapshot. Each take method succeeds once and then returns an error. Stream
  reads and command sends require mutable access, making the single consumer and
  single application-side command order explicit in Rust's type system.
- Ordering is defined within each stream, not across the two independent
  streams; consumers may render a coherent snapshot before their Ready task is
  scheduled and must not infer lifecycle state from snapshot arrival.
- GPUI uses async next methods; synchronous tests and the benchmark use
  next_blocking.
- Every bundle is immutable and internally coherent: bundle.generation,
  render.generation, and pane.generation are equal.
- A generation increments once after each terminal mutation batch, never once per projection.
- TerminalSize rejects zero rows, columns, or cell dimensions at spawn/resize.
  It also rejects grids whose `u64::from(rows) * u64::from(cols)` exceeds
  1,000,000 cells, before allocating or mutating either backend.
  Its PTY total-pixel accessors multiply in u64 and saturate only the final value
  to u16::MAX; `cell_width_px` and `cell_height_px` are physical device pixels,
  and libghostty always receives those original per-cell u32 metrics.
- The 17-slot worker queue reserves one slot that PTY output cannot consume.
  Sixteen 16-KiB output permits structurally limit output waiting ahead of a
  later input command to 256 KiB. An accepted raw Input command is at most
  16 KiB; larger commands fail before enqueueing and Checkpoint 2 chunks paste
  through this same limit. Accepted input never discards bytes and has a
  measured latency budget under sustained output.
- Key carries owned platform-neutral input. sprite-term maps the logical key to
  libghostty, refreshes encoder options from the current terminal, and writes
  the resulting bytes; no libghostty input type crosses into sprite-app.
- Resize preserves ordering in Checkpoint 1; coalescing arrives only with a test in Checkpoint 2.
- begin_shutdown is idempotent and non-blocking: the first call returns Some,
  later calls return None. ShutdownHandle::wait may block and must run off the
  GPUI thread.
- TerminalSession and the worker share one atomic shutdown flag. begin_shutdown
  and Drop set it and make a best-effort nonblocking Shutdown send: a full queue
  means the worker is active and checks the flag after its next message; an idle
  worker has room for the wake-up. Dropping never joins on the dropping thread.

## File structure

~~~text
Sprite/
├── .github/workflows/phase-1.yml
├── .gitignore
├── .gitmodules
└── phase_1/
    ├── .cargo/config.toml
    ├── Cargo.lock
    ├── Cargo.toml
    ├── CONTEXT.md
    ├── LICENSE-APACHE
    ├── LICENSE-MIT
    ├── rust-toolchain.toml
    ├── crates/
    │   ├── sprite-term/
    │   │   ├── Cargo.toml
    │   │   ├── src/
    │   │   │   ├── bin/sprite-term-bench.rs
    │   │   │   ├── lib.rs
    │   │   │   ├── pty_unix.rs
    │   │   │   ├── shell.rs
    │   │   │   ├── snapshot.rs
    │   │   │   └── worker.rs
    │   │   └── tests/
    │   │       ├── benchmark.rs
    │   │       ├── croft_smoke.rs
    │   │       ├── lifecycle.rs
    │   │       ├── session_io.rs
    │   │       ├── session_output.rs
    │   │       └── support/mod.rs
    │   └── sprite-app/
    │       ├── Cargo.toml
    │       └── src/
    │           ├── input.rs
    │           ├── lib.rs
    │           ├── main.rs
    │           └── terminal_view.rs
    ├── docs/
    │   ├── performance/checkpoint-1.md
    │   └── TSP/08-09-2026-checkpoint-1-terminal-spine.md
    ├── scripts/test-croft-main.sh
    └── vendor/ghostty/
~~~

sprite-term is the deep module. worker.rs, snapshot.rs, and shell.rs are private implementation files, not extra seams. sprite-app learns only the public types above.

---

### Task 1: Establish the reproducible workspace and source pin

**Files:**

- Create: phase_1/Cargo.toml
- Create: phase_1/rust-toolchain.toml
- Create: phase_1/.cargo/config.toml
- Create: phase_1/crates/sprite-term/Cargo.toml
- Create: phase_1/crates/sprite-term/src/lib.rs
- Create: phase_1/crates/sprite-app/Cargo.toml
- Create: phase_1/crates/sprite-app/src/lib.rs
- Create: phase_1/crates/sprite-app/src/main.rs
- Create: phase_1/LICENSE-MIT
- Create: phase_1/LICENSE-APACHE
- Modify: .gitignore
- Modify: .gitmodules
- Modify: phase_1/DEPENDENCIES.md
- Create: phase_1/Cargo.lock

**Interfaces:** Produces a two-package Cargo workspace, a network-explicit
bootstrap, and an offline locked verification contract. No product behavior is
introduced.

- [x] Add the exact source submodule from the Sprite repository root:

~~~bash
git submodule add https://github.com/ghostty-org/ghostty.git phase_1/vendor/ghostty
git -C phase_1/vendor/ghostty checkout ab0b9da9e88fcb4b0533a1854e84628f663930af
git add .gitmodules phase_1/vendor/ghostty
~~~

Expected: git -C phase_1/vendor/ghostty rev-parse HEAD prints exactly ab0b9da9e88fcb4b0533a1854e84628f663930af.

- [x] Create phase_1/Cargo.toml:

~~~toml
[workspace]
members = ["crates/sprite-app", "crates/sprite-term"]
resolver = "3"

[workspace.package]
version = "0.1.0"
edition = "2024"
rust-version = "1.97.1"
license = "MIT OR Apache-2.0"

[workspace.dependencies]
async-channel = "=2.5.0"
gpui = { version = "=0.2.2", default-features = false }
libghostty-vt = { version = "=0.2.1", default-features = false }
nix = { version = "=0.28.0", default-features = false, features = ["poll", "process", "signal"] }
portable-pty = { version = "=0.9.0", default-features = false }
sprite-term = { path = "crates/sprite-term" }
~~~

- [x] Create phase_1/.cargo/config.toml:

~~~toml
[env]
GHOSTTY_SOURCE_DIR = { value = "vendor/ghostty", relative = true, force = true }
SPRITE_TERMINFO_DIR = { value = "target/terminfo", relative = true, force = true }
ZIG_GLOBAL_CACHE_DIR = { value = "target/zig-global-cache", relative = true, force = true }
ZIG_LOCAL_CACHE_DIR = { value = "target/zig-local-cache", relative = true, force = true }
~~~

- [x] Pin Rust 1.97.1 with minimal profile, clippy, and rustfmt. Add the unmodified standard MIT and Apache-2.0 texts. Give both crates workspace package metadata. In sprite-app's manifest, declare `[[bin]]` with `name = "sprite"` and `path = "src/main.rs"`; the shipped and development executable is never named `sprite-app`.
- [x] Add `/phase_1/target/` to the root .gitignore. Do not ignore Cargo.lock,
  the Ghostty submodule entry, licenses, performance evidence, or docs.
- [x] Add async-channel, libghostty-vt, and portable-pty to sprite-term. On Unix,
  declare exact nix 0.28.0 and directly request only poll/process/signal;
  portable-pty already resolves that package with term/fs, so Cargo's resolved
  union has all five features. This adds audited APIs/features rather than
  another package. Add sprite-term to sprite-app. Add target-specific GPUI
  dependencies: wayland and x11 features on Linux; no features on macOS.
- [x] Run zig version and require exactly 0.16.0 before the native check. Record
  Zig as a build tool in DEPENDENCIES.md, not a runtime dependency. Stop with an
  actionable prerequisite error if Zig is absent or is not exactly 0.16.0.
- [x] Run `tic -V` and require an extended-capability ncurses implementation.
  Record `tic` as a build/packaging tool, not a runtime dependency.
- [x] Expand DEPENDENCIES.md into full entries for all five direct external
  crates. Record capability, rejected std/existing option, direct and resolved
  features,
  license/source, and update policy. Explain that async-channel replaces an app
  polling loop or extra bridge thread, while the already-transitive nix package
  makes PTY reads cancelable and process-group shutdown bounded without an async
  runtime or detached thread.
- [x] Run one explicit network bootstrap, then prove that compilation no longer
  needs the network. Two corrections to the original plan, found while executing
  it against the pinned commit: `zig build --fetch` alone does not resolve the
  lazily-fetched packages the libghostty-vt build needs (notably `aro`), so the
  Cargo build script would reach the network during an offline `cargo check`;
  and `zig run src/main_build_data.zig` fails at this commit because that entry
  point imports the `help_strings` module only Ghostty's own `build.zig`
  constructs. `scripts/gen-terminfo.zig` imports the pinned
  `src/terminfo/ghostty.zig` directly, which depends on nothing but `std` and
  its sibling `Source.zig`. No Ghostty source is patched, so ADR 0003 review is
  not triggered.

~~~bash
cd phase_1
cargo generate-lockfile
cargo fetch --locked
export ZIG_GLOBAL_CACHE_DIR="$PWD/target/zig-global-cache"
export ZIG_LOCAL_CACHE_DIR="$PWD/target/zig-local-cache"
(cd vendor/ghostty && zig build --fetch=all -Demit-lib-vt=true -Demit-xcframework=false -Dapp-runtime=none)
mkdir -p target/terminfo
zig build-exe -lc -femit-bin=target/gen-terminfo \
  --dep ghostty_terminfo \
  -Mroot=scripts/gen-terminfo.zig \
  -Mghostty_terminfo=vendor/ghostty/src/terminfo/ghostty.zig
./target/gen-terminfo > target/ghostty.terminfo
tic -x -o target/terminfo target/ghostty.terminfo
infocmp -A target/terminfo xterm-ghostty >/dev/null
cargo metadata --locked --offline --format-version 1 --no-deps
cargo check --workspace --locked --offline
~~~

Expected: metadata lists only sprite-app and sprite-term as workspace members;
the lock/fetch commands are the only network-enabled steps, the terminfo check finds
the entry generated from the pinned Ghostty source, and the offline check ends
with Finished. A clean CI job uses the same fetch-then-offline boundary.

- [x] Commit:

~~~bash
git add .gitignore .gitmodules phase_1/.cargo phase_1/Cargo.toml phase_1/Cargo.lock phase_1/rust-toolchain.toml phase_1/crates/sprite-app phase_1/crates/sprite-term phase_1/DEPENDENCIES.md phase_1/LICENSE-APACHE phase_1/LICENSE-MIT phase_1/vendor/ghostty
git commit -m "build(phase_1): establish pinned terminal workspace"
~~~

### Task 2: Prove real PTY spawn and child exit

**Files:**

- Modify: phase_1/crates/sprite-term/src/lib.rs
- Create: phase_1/crates/sprite-term/src/worker.rs
- Create: phase_1/crates/sprite-term/tests/support/mod.rs
- Create: phase_1/crates/sprite-term/tests/lifecycle.rs

**Interfaces:** Consumes SessionConfig. Produces TerminalSession, the single-owner
EventStream and SnapshotStream, Ready, Exited, Error, and ShutdownHandle.
TerminalCommand is declared completely, but no public command is handled here;
lifecycle remains exclusively behind begin_shutdown rather than exposing a
second shutdown path.

- [x] Write one external-interface test: spawn /bin/sh with arguments -c and
  exit 7, take EventStream once, await Ready, then Exited, and assert ChildExit
  has code Some(7), no signal, and requested false. tests/support/mod.rs moves the taken stream
  into a helper thread that calls next_blocking, returns the event through
  std::sync::mpsc, and uses recv_timeout for a five-second watchdog.
- [x] Add a launch-failure test for a nonexistent absolute executable. It must
  receive Error with operation spawn_child, never receive Ready, and terminate
  its worker cleanly.
- [x] Run:

~~~bash
cd phase_1
cargo test -p sprite-term --test lifecycle --locked --offline child_exit_is_reported -- --exact
~~~

Expected RED: unresolved Terminal Session types.

- [x] Add the exact public types above. Implement Display and Error for SessionError without a dependency.
- [x] Implement the worker queue with `std::sync::mpsc::sync_channel(17)` and
  lifecycle/latest-snapshot streams with async-channel capacities 32 and 1. The
  seventeenth worker slot is reserved for application/lifecycle work by the
  output-permit rule introduced in Task 3. The standard queue supplies
  `recv_timeout` only during bounded shutdown; normal operation blocks in
  `recv` and never polls. For Task 2, the worker opens portable-pty with the
  configured size, constructs the command, applies cwd/environment, spawns the
  child, drops the parent slave, and sends Ready. Task 3 adds libghostty
  initialization and the I/O pump before Ready without changing the public
  lifecycle.
- [x] On the supported Unix targets, require both Child::process_id and
  MasterPty::as_raw_fd immediately after spawn. A missing/invalid value emits a
  startup Error and cleans up rather than weakening process-group or pump
  cancellation guarantees.
- [x] Move the Child handle into one named waiter thread with a 256 KiB stack
  and block in Child::wait. The waiter sends one
  private ChildExited message containing the owned status, so quiet exits are
  reaped without any timer and descendants holding the PTY open cannot hide the
  shell's exit. Convert portable-pty status without inventing two causes: when
  `signal()` is Some, set ChildExit.signal to that owned name and code to None;
  otherwise set code to Some(`exit_code()`) and signal to None. The terminal
  owner keeps only the validated process/session IDs; it does not duplicate
  child ownership.
- [x] Share one `Arc<AtomicBool>` shutdown flag. The worker checks it after every
  message and immediately before a blocking receive. begin_shutdown and Drop set
  it and `try_send(Shutdown)` only as a wake-up; `Full` is safe because the active
  worker will observe the flag, and `Disconnected` means it already ended.
  begin_shutdown takes the join handle once even when the queue is disconnected:
  the first call returns Some(handle), later calls return None. Reject new sends
  after the flag is set and map a worker panic to operation join_worker.
- [x] Run:

~~~bash
cargo test -p sprite-term --test lifecycle --locked --offline
~~~

Expected GREEN: the child exit test passes.

- [x] Commit:

~~~bash
git add phase_1/crates/sprite-term/src phase_1/crates/sprite-term/tests
git commit -m "feat(term): own PTY and child lifecycle"
~~~

### Task 3: Publish coherent owned Ghostty projections

**Files:**

- Modify: phase_1/crates/sprite-term/src/lib.rs
- Modify: phase_1/crates/sprite-term/src/worker.rs
- Create: phase_1/crates/sprite-term/src/pty_unix.rs
- Create: phase_1/crates/sprite-term/src/snapshot.rs
- Create: phase_1/crates/sprite-term/tests/session_output.rs

**Interfaces:** Consumes ordered PTY output. Produces the complete owned snapshot
types through latest-only SnapshotStream, independently of lifecycle events.

- [x] Add a test that spawns /bin/sh and prints red ANSI text, the wide character 界, and e plus U+0301. Await a Pane Snapshot containing red wide:界 combining:é. Assert equal generations, a red styled render cell, Wide followed by SpacerTail, and no ANSI bytes in pane text.
- [x] Run the focused test. Expected RED: SnapshotStream yields no snapshot.
- [x] Create Terminal, RenderState, RowIterator, and CellIterator on the terminal-owner worker before Ready. Use SessionConfig dimensions and scrollback.
- [x] Immediately after Ready, publish one coherent blank generation-0 bundle
  before processing PTY output. A silent long-running child must still provide
  dimensions and cursor state without a timer or synthetic mutation.
- [x] In pty_unix.rs, start one named PTY I/O-pump thread with a 256 KiB stack.
  Give it the stable master raw descriptor and one end of a
  `UnixStream::pair`; the worker retains the master and the cancellation writer
  until after the pump joins. Create a standard-library synchronous permit
  channel seeded with sixteen tokens. Before waiting for PTY readiness, the pump
  takes one token, then blocks with nix `poll` on PTY readability and the
  cancellation socket with no timeout. It handles cancellation first when both
  descriptors are ready. Retry `poll` and `read` on EINTR. Treat PTY POLLIN and
  POLLHUP as readable: a hangup may still have trailing bytes, so read and
  deliver them, then report EOF only when read returns zero. Treat POLLERR or
  POLLNVAL as ReadError unless cancellation won. The worker returns the token
  only after it finishes
  applying or discarding the corresponding PtyOutput message, so the sixteen-
  token limit includes the chunk currently being processed. The held token
  covers one read of at most 16 KiB. If cancellation, EOF, or a read error wins
  instead, the pump returns the unused token before reporting its stop outcome.
  Thus a pump waiting for a permit is
  released when Closing drains one output, then observes the already-readable
  cancellation socket; it can always join. At most sixteen output chunks
  (256 KiB) can occupy the 17-slot worker queue, structurally reserving one slot
  for input or lifecycle work. Exhausted permits apply PTY backpressure without
  depending on sender fairness. Every exit path sends exactly one private
  PumpStopped message whose owned outcome is Canceled, Eof, or ReadError before
  the thread returns. Running treats Eof as session close and ReadError as one
  Error followed by close. Keep any raw-descriptor borrowing in this one audited Unix
  module, with a safety invariant proving the master outlives the pump and is
  not closed before join; never add unsafe Send/Sync and never touch libghostty.
- [x] Each output chunk calls Terminal::vt_write once and increments generation
  once. Capture immediately only when the one-slot snapshot channel is empty;
  otherwise mark the worker snapshot-dirty without building an obsolete
  projection. After SnapshotStream returns a snapshot to its consumer, the
  stream uses nonblocking `try_send` for a private CaptureRequested message. If
  the worker queue is full, a queued mutation is guaranteed to wake the worker,
  which checks dirty-plus-empty after every message; if it is idle, the request
  fits and wakes it. The worker then captures the newest dirty generation, if
  any. This coalesces both construction and delivery without ever blocking GPUI
  or using a timer.
- [x] Snapshot capture uses:

~~~rust
fn capture(
    generation: u64,
    size: TerminalSize,
    terminal: &libghostty_vt::Terminal<'_, '_>,
    render_state: &mut libghostty_vt::RenderState<'_>,
    rows: &mut libghostty_vt::render::RowIterator<'_>,
    cells: &mut libghostty_vt::render::CellIterator<'_>,
) -> Result<SnapshotBundle, SessionError>;
~~~

- [x] Call `RenderState::update`, keep its returned borrowed Snapshot alive
  while updating RowIterator and CellIterator, and copy all borrowed fields.
  Map Ghostty raw/style colors, underline, `Cell::wide`, `Row::is_wrapped`,
  cursor, active screen, dimensions, and defaults. Construct render and pane
  rows during one traversal but allocate independent owned fields.
- [x] After the complete owned bundle is built, set every visited row dirty flag
  to false and the borrowed render Snapshot to `Dirty::Clean`. Send into the one-slot
  snapshot channel; never place a snapshot in EventStream and never block the
  terminal owner on an obsolete projection.
- [x] Run:

~~~bash
cargo test -p sprite-term --test session_output --locked --offline
cargo test -p sprite-term --locked --offline
~~~

Expected GREEN: projection and lifecycle tests pass.

- [x] Commit:

~~~bash
git add phase_1/crates/sprite-term/src phase_1/crates/sprite-term/tests/session_output.rs
git commit -m "feat(term): publish coherent Ghostty snapshots"
~~~

### Task 4: Round-trip ordered input and terminal replies

**Files:**

- Modify: phase_1/crates/sprite-term/src/worker.rs
- Modify: phase_1/crates/sprite-term/src/pty_unix.rs
- Create: phase_1/crates/sprite-term/tests/session_io.rs

**Interfaces:** Consumes Key events and raw Input bytes. Produces state-aware
libghostty key encodings, ordered PTY writes, and terminal-generated PTY replies.

- [x] Test /bin/sh -c with a read followed by printf got:%s. Send the letters
  in sprite as Key events plus an Enter Key event and await got:sprite. Add a
  second assertion that ArrowUp differs between normal and cursor-application
  mode after the child changes that terminal mode.
- [x] Test a child that prints CSI 5 n, reads exactly four response bytes, and
  pipes them through `/usr/bin/od -An -tx1`. Assert a final pane row contains
  `1b 5b 30 6e` (CSI 0 n). Do not print the raw reply for the assertion, because
  the terminal parser would correctly consume it as another control sequence.
- [x] Run both and confirm RED.
- [x] Keep the sole `MasterPty::take_writer` result on the terminal-owner worker
  in worker-local `Rc<RefCell<Box<dyn Write + Send>>>`; it never crosses a
  thread or public interface. Before Ready, register `Terminal::on_pty_write`
  with a clone of that `Rc` and have the synchronous callback call `write_all`
  for each terminal-generated reply. The callback cannot return an I/O error,
  so store its first failure in a second worker-local `Rc<RefCell<Option<
  SessionError>>>`. Immediately after every `Terminal::vt_write`, take and
  handle that error by emitting one Error and beginning pane-local shutdown.
  Keyboard and trusted raw Input writes use the same writer, preserving one
  ordered PTY-write path without `Arc`, a writer thread, or another channel.
- [x] Create one libghostty key Encoder on the owner worker. Map owned logical
  GPUI names `enter`, `tab`, `space`, `backspace`, `delete`, `escape`, `up`,
  `down`, `left`, `right`, `home`, `end`, `pageup`, `pagedown`, `insert`, and
  `f1` through `f25` to their libghostty Key variants; map single ASCII
  letters, digits, and punctuation to the corresponding physical-key variants.
  Use Unidentified plus UTF-8 text as the safe fallback. Map
  modifiers/action/composition, set the unshifted codepoint
  when the logical key is one Unicode scalar, call set_options_from_terminal
  immediately before every encode, and write the resulting vector to the PTY.
  Pass text to `set_utf8` only when it contains no C0/DEL control codepoint and
  no macOS private-use function-key codepoint, as required by libghostty; named
  control/function keys rely on their mapped Key value.
  GPUI's function modifier remains preserved in the owned event but has no
  libghostty `Mods` bit and therefore is not invented as a terminal modifier;
  GPUI's resulting key name/text still determine the encoded key.
  This mapper is extended in place as GPUI platform tests reveal more key names;
  it is not replaced by an application encoder.
- [x] Handle raw Input with write_all and flush for trusted pre-encoded bytes
  and deterministic byte-level tests. Reject payloads over 16 KiB before
  enqueueing; test that rejection leaves the live session usable. Each accepted
  Key or Input command is one ordered write operation. Do not route clipboard
  text through Input in Checkpoint 1; Checkpoint 2 adds state-aware Paste
  encoding and chunks it through this same limit.
- [x] Add a public-session stress test whose child produces sustained output in
  one process while another reads input. Send a marker after output starts,
  stop the producer when the marker arrives, and assert the marker reaches a
  visible snapshot within the five-second watchdog with no lost output or input.
  This proves the 16-output-message/256-KiB permit bound prevents starvation;
  Task 9 gives the same path a numerical latency budget.
- [x] Run:

~~~bash
cargo test -p sprite-term --test session_io --locked --offline
cargo test -p sprite-term --locked --offline
~~~

Expected GREEN: input, terminal reply, output, and lifecycle tests pass.

- [x] Commit:

~~~bash
git add phase_1/crates/sprite-term/src/worker.rs phase_1/crates/sprite-term/tests/session_io.rs
git commit -m "feat(term): route ordered terminal input"
~~~

### Task 5: Apply one ordered resize to PTY and terminal

**Files:**

- Modify: phase_1/crates/sprite-term/src/lib.rs
- Modify: phase_1/crates/sprite-term/src/worker.rs
- Modify: phase_1/crates/sprite-term/tests/session_io.rs

**Interfaces:** Consumes Resize. Produces one snapshot at the new cell/pixel dimensions after both PTY and libghostty accept it.

- [x] Test u64 multiplication followed by u16 saturation at the exact boundary.
  Reject zero rows, columns, or cell dimensions and grids above 1,000,000 cells
  before either backend allocates or mutates. Test the exact 1,000,000-cell
  acceptance boundary and the first larger grid.
- [x] Spawn a shell that reports stty size after input. Resize to 40x100 cells at 9x18 pixels, send newline, and await both pane text 40 100 and the exact TerminalSize.
- [x] Run focused test and confirm RED.
- [x] Validate at the public seam. Call MasterPty::resize first with total
  pixels, then Terminal::resize with per-cell pixels. Only after both succeed
  publish the new current size, increment generation, and capture. The two
  external mutations cannot be rolled back atomically: on either failure keep
  the prior *published* size, emit Error, and begin pane-local shutdown so an
  uncertain PTY/terminal pair is never presented as coherent.
- [x] Run:

~~~bash
cargo test -p sprite-term --test session_io --locked --offline resize_updates_pty_and_snapshot -- --exact
cargo test -p sprite-term --locked --offline
~~~

Expected GREEN: PTY output and snapshot dimensions agree.

- [x] Commit:

~~~bash
git add phase_1/crates/sprite-term/src/lib.rs phase_1/crates/sprite-term/src/worker.rs phase_1/crates/sprite-term/tests/session_io.rs
git commit -m "feat(term): order PTY and terminal resize"
~~~

### Task 6: Resolve and identify the login shell

**Files:**

- Modify: phase_1/crates/sprite-term/src/lib.rs
- Create: phase_1/crates/sprite-term/src/shell.rs
- Modify: phase_1/crates/sprite-term/tests/lifecycle.rs

**Interfaces:** Produces SessionConfig::login_shell. Produces TERM=xterm-ghostty,
the bootstrapped local TERMINFO directory when available, TERM_PROGRAM=Sprite,
TERM_PROGRAM_VERSION=0.1.0, and a PATH beginning with the running executable
directory.

- [x] Table-test a private pure resolver accepting Option<&OsStr>. An absolute executable SHELL wins. Invalid SHELL falls back to /bin/zsh on macOS, /bin/bash on Linux, then /bin/sh. The result must be absolute and executable.
- [x] Test identity variables, `infocmp xterm-ghostty`, and the first PATH entry
  through a real Terminal Session. Cargo supplies SPRITE_TERMINFO_DIR from the
  workspace config; SessionConfig sets the child's TERMINFO to that directory.
- [x] Run and confirm RED.
- [x] Implement login_shell with -l, current directory, TerminalSize::DEFAULT, and 10,000 scrollback lines. Do not inspect or modify dotfiles.
- [x] Apply Sprite identity after user environment entries. Prepend PATH with split_paths and join_paths, never a literal colon.
- [x] If SPRITE_TERMINFO_DIR names the bootstrapped directory, override any user
  TERMINFO for this child with that exact path. If it is absent, rely on the
  packaged/system terminfo search; Checkpoint 5 supplies the packaged path.
- [x] Run:

~~~bash
cargo test -p sprite-term --locked --offline
cargo clippy -p sprite-term --all-targets --locked --offline -- -D warnings
~~~

Expected GREEN with no warnings.

- [x] Commit:

~~~bash
git add phase_1/crates/sprite-term/src/lib.rs phase_1/crates/sprite-term/src/shell.rs phase_1/crates/sprite-term/tests/lifecycle.rs
git commit -m "feat(term): launch an identified login shell"
~~~

### Task 7: Prove idempotent shutdown and child reaping

**Files:**

- Modify: phase_1/crates/sprite-term/src/worker.rs
- Modify: phase_1/crates/sprite-term/tests/lifecycle.rs

**Interfaces:** Consumes Shutdown through begin_shutdown. Produces the approved
bounded HUP/TERM/KILL process-group policy, a canceled pump, a reaped child, and
a joined owner worker.

- [x] Spawn a shell that prints its PID then execs sleep 60. Capture the PID,
  begin shutdown twice, wait on the first returned handle, and verify /bin/kill
  -0 PID exits nonzero. The second call returns None.
- [x] Add a five-second test for dropping after a shutdown request.
- [x] Add a regression test whose shell starts a background descendant that
  inherits the PTY and ignores HUP and TERM, prints the shell PID plus `$!`, and
  then exits. Assert cleanup reaches KILL, Exited arrives only afterward, the
  pump and waiter have joined, `/bin/kill -0` fails for the descendant, and the
  five-second watchdog does not time out even though PTY EOF alone would not
  have arrived.
- [x] Run and confirm RED.
- [x] Implement one explicit Running-to-Closing worker transition. Closing
  rejects application commands, drops libghostty and its PTY callback, writes
  the pump cancellation byte, and drains/discards already queued PtyOutput so a
  pump blocked on the bounded queue can send PumpStopped and return.
- [x] Record the Unix session/process-group ID at spawn. Immediately before
  shutdown, also query MasterPty::process_group_leader for the current foreground
  group. Deduplicate and signal both known groups so an interactive foreground
  program and the original shell group receive the policy. On requested shutdown,
  signal the groups with HUP and use worker `recv_timeout` against one absolute
  deadline while processing only owned exit/pump outcomes. Check Instant before
  every receive/message so continuous output cannot postpone escalation. Send
  TERM at two seconds and KILL one second later. Treat an already-gone group as
  success. On natural child exit, send HUP once to any remaining recorded group
  members and enter the same Closing state. Wait until both PumpStopped and
  ChildExited have arrived, then join the known-finished pump and waiter, drop
  writer/master, and finish the worker.
  Use nix's safe process/signal API; platform calls remain private to
  pty_unix.rs. Checkpoint 2 adds foreground-process identification and the
  close-confirmation UI, not a replacement lifecycle.
- [x] Normal shutdown emits no Error. Exited occurs at most once. All helper
  threads and libghostty objects end before the terminal-owner worker returns.
  Publish Exited only after the pump and waiter are joined and the descendant
  cleanup policy has finished, then return from the worker. Exited sets
  requested true whenever the atomic shutdown flag caused the close, so its
  eventual signal is never presented as an unexpected child failure.
- [x] Run:

~~~bash
cargo test -p sprite-term --test lifecycle --locked --offline
cargo test -p sprite-term --locked --offline
~~~

Expected GREEN without watchdog timeout.

- [x] Commit:

~~~bash
git add phase_1/crates/sprite-term/src/pty_unix.rs phase_1/crates/sprite-term/src/worker.rs phase_1/crates/sprite-term/tests/lifecycle.rs
git commit -m "fix(term): reap children on idempotent shutdown"
~~~

### Task 8: Open one interactive GPUI terminal window

**Files:**

- Modify: phase_1/crates/sprite-app/src/lib.rs
- Modify: phase_1/crates/sprite-app/src/main.rs
- Create: phase_1/crates/sprite-app/src/input.rs
- Create: phase_1/crates/sprite-app/src/terminal_view.rs

**Interfaces:** Consumes TerminalSession, the separate EventStream and
SnapshotStream, and GPUI events. Produces one event-driven window displaying the
latest RenderSnapshot and sending owned keys/resizes through the Terminal
Session seam.

- [x] Unit-test the private helper
  `gpui_key_event(&gpui::Keystroke, sprite_term::KeyAction) ->
  sprite_term::KeyEvent`. KeyDownEvent chooses Press or Repeat from `is_held`;
  KeyUpEvent chooses Release, then both handlers call this helper. Cover
  printable key/key_char, Enter, arrows, held-repeat, release, Shift, Alt,
  Control, platform/Super, and function. It must copy strings and must not
  generate terminal bytes.
- [x] Unit-test `grid_size`, a pure conversion from logical content bounds,
  measured logical cell metrics, and the current positive display scale factor
  to a nonzero TerminalSize. Divide logical bounds by logical cell width/height
  and round row/column counts down. Convert each logical cell metric to physical
  `cell_*_px` by multiplying by the scale factor and rounding to the nearest
  device pixel, with a minimum of one; do not multiply rows or columns by scale.
  Cover 1.0, 1.25, and 2.0 scale, the 1,000,000-cell cap, zero/invalid inputs,
  and duplicate-size suppression.
- [x] Run `cargo test -p sprite-app --locked --offline gpui_key_event`.
  Expected RED.
- [x] Implement only GPUI-to-owned-event normalization in input.rs. Terminal
  bytes are always produced by libghostty on the terminal-owner worker. GPUI's
  KeyDownEvent/KeyUpEvent do not expose IME composition state, so this direct
  key path sets composing false; Checkpoint 2 adds GPUI InputHandler/IME wiring
  and sets true only for events that actually belong to a composition.
- [x] Implement TerminalView with one session, newest bundle, FocusHandle,
  measured logical cell width/height, current size, optional error text, and two
  retained GPUI tasks. Its constructor receives the Window, shapes the initial
  cell before spawning, and replaces the login configuration's default physical
  cell metrics using the current scale factor while retaining the 24x80 initial
  grid. Thus a child cannot observe scale-1 metrics during Retina/HiDPI startup.
- [x] In the constructor spawn that configured login shell, take each stream exactly once,
  and use one cx.spawn task per stream with no timer or polling. The snapshot task
  applies only newer generations and calls cx.notify. The lifecycle task stores
  Error/Exited status and notifies. After normal Exited, both tasks treat stream
  closure as completion rather than presenting a new error.
- [x] Render one full-size dark focused div. Display rows in a 14 px monospace family at 16 px line height. Build strings from RenderRow, omitting spacer cells. Add no renderer abstraction or cache.
- [x] Measure `"M"` in the constructor with GPUI's
  `Window::text_system().shape_line` using
  the exact 14-logical-pixel font run rendered above; use its shaped logical
  width and the 16-logical-pixel line height as grid geometry. On every layout,
  pass those values plus `Window::scale_factor()` to grid_size and send Resize
  only when the resulting valid size changes. This also updates physical cell
  metrics after a window moves between displays with different scale factors.
  Never mix fixed 8x16 resize math with a differently measured rendered font,
  and never report logical pixels as physical pixels.
- [x] main.rs opens one centered 960x640 window, focuses TerminalView, and
  activates the app. Install `Window::on_window_should_close`; its first call
  takes the ShutdownHandle, starts `wait` on GPUI's background executor, detaches
  a foreground continuation, and returns true immediately so the native window
  closes. The continuation calls `App::quit` only after the wait finishes. Do
  not also quit immediately from `on_window_closed`, because that could tear
  down the executor before the child and helper threads join.
- [x] Run:

~~~bash
cargo fmt --all -- --check
cargo test --workspace --locked --offline
cargo clippy --workspace --all-targets --locked --offline -- -D warnings
cargo build -p sprite-app --locked --offline
~~~

Expected: every command exits zero.

- [~] Smoke on native Arch Wayland and X11 (Wayland complete; X11 PARTIAL):

~~~bash
cargo run -p sprite-app --locked --offline
env -u WAYLAND_DISPLAY cargo run -p sprite-app --locked --offline
~~~

The first command requires a nonempty WAYLAND_DISPLAY and must report Wayland;
the second requires a nonempty DISPLAY and must report X11. In each, type
printf 'sprite-check\n', run stty size before/after resize, then exit. Text must
appear, size must change, exit must not leave a zombie, and idle must not redraw
continuously.

**Wayland: verified.** Window class `sprite`, title `Sprite`, live `/usr/bin/bash
-l` child in state `Ss+`, named worker/waiter/pump threads present. Typed input
round-tripped (`printf 'sprite-check\n'` produced `sprite-check`). `stty size`
reported 35x112 from the measured font rather than the 24x80 default, proving the
layout-driven resize reaches the kernel. Monospace confirmed by equal-width
`iiiii` and `MMMMM`.

**X11: PARTIAL.** Launches under Xwayland with a live `bash -l` child and a mapped
window, and exits without leaving a zombie. Text rendering, typing, and resize
were *not* verified under X11, and the before/after resize comparison was not
completed on either backend (the window manager refused the scripted resize;
only the initial computed grid was checked). Idle redraw behaviour is unmeasured
on both.

**Two upstream GPUI 0.2.2 defects found on X11. Neither is fixable from
Sprite; both need an upstream change.**

1. *Unterminated `WM_CLASS`.* ICCCM requires two consecutive NUL-**terminated**
   strings, `instance\0class\0`. `X11Window::set_app_id` writes the separator
   but omits the final terminator:

   ~~~rust
   let mut data = Vec::with_capacity(app_id.len() * 2 + 1);
   data.extend(app_id.bytes()); // instance
   data.push(b'\0');
   data.extend(app_id.bytes()); // class   <- no trailing NUL
   ~~~

   Lenient readers cope — `xprop` reports `"sprite", "sprite"` correctly — but
   readers that trust the terminator drop the final byte. Hyprland/wlroots
   therefore reports the class as `sprite\x00sprit`, and no window rule matching
   `sprite` applies to a Sprite window under X11 or XWayland. Wayland is
   unaffected: it sets the app id through the toplevel protocol. The upstream
   fix is one line.

2. *`HasWindowHandle` panics.* `impl rwh::HasWindowHandle for X11Window` is
   `unimplemented!()` (`src/platform/linux/x11/window.rs:316`), as is
   `HasDisplayHandle`. Any call panics the process. Wayland implements both.

**A local workaround was attempted and reverted.** Reading the X11 window id
through `raw-window-handle` to rewrite the property ourselves is the obvious
fix, but defect 2 makes it impossible: the call panics before it can return an
id, turning a cosmetic class problem into a hard crash on X11. The alternative —
enumerating `_NET_CLIENT_LIST` and matching `_NET_WM_PID` — needs a retry loop
because the window is not listed immediately, and `thread::sleep` in
`sprite-app` is a forbidden state under Task 10. Both routes are closed, so
Sprite carries no workaround and the defects stay recorded here.

Neither has been reported upstream yet.

- [ ] **OUTSTANDING — needs real macOS hardware.** On real macOS repeat offline
  locked workspace test, sprite-app build/run, typing, resize, exit, and Activity
  Monitor idle inspection. Never attempted; no macOS machine available to this
  workspace. Checkpoint 1 cannot be accepted until this is run by hand.
- [x] Commit:

~~~bash
git add phase_1/crates/sprite-app
git commit -m "feat(app): open the first Sprite terminal window"
~~~

### Task 9: Freeze Checkpoint 1 performance budgets

**Files:**

- Create: phase_1/crates/sprite-term/src/bin/sprite-term-bench.rs
- Create: phase_1/crates/sprite-term/tests/benchmark.rs
- Create: phase_1/docs/performance/checkpoint-1.md

**Interfaces:** Consumes only TerminalSession. Produces repeatable measurements and committed numerical budgets.

- [x] Add CLI sprite-term-bench --samples 30 --output PATH. Measure
  spawn-to-Ready, one-byte input-to-visible-snapshot both idle and during
  sustained output, 10 MiB output-to-final-snapshot, and full capture of a
  100-by-100 visible grid (10,000 cells). Scrollback-history capture is measured
  when Checkpoint 2 adds that capability.
  Write stable JSON with sample count, median, p95, max, units, and budget equal
  to 110% of p95 using only std.
- [x] Integration-test three samples into a unique temp path. Assert fixed JSON keys and finite nonnegative values, then remove it.
- [x] Run focused test RED, implement, then GREEN.
- [x] On Arch:

~~~bash
cargo run --release -p sprite-term --bin sprite-term-bench --locked --offline -- --samples 30 --output target/checkpoint-1-arch.json
/usr/bin/time -v cargo run --release -p sprite-app --locked --offline
~~~

- [ ] **OUTSTANDING — needs real macOS hardware.** On real macOS:

~~~bash
cargo run --release -p sprite-term --bin sprite-term-bench --locked --offline -- --samples 30 --output target/checkpoint-1-macos.json
/usr/bin/time -l cargo run --release -p sprite-app --locked --offline
~~~

- [~] Write checkpoint-1.md (written; Arch section complete, macOS and Ghostty
  sections marked outstanding inside it) with date, OS/kernel, CPU, RAM, GPU, backend, refresh, tool versions, Ghostty commit, benchmark JSON, app-to-prompt, 60-second idle CPU/RSS, 10-second resize cadence, and each numerical regression budget. Record graphics retention and budget as 0 MiB because Checkpoint 1 allocates no images; replace that metric in Checkpoint 4.
- [ ] **OUTSTANDING.** Packaged Ghostty here is 1.3.1, which is 1,443 commits
  behind the pinned `ab0b9da9`, and Sprite has no CLI yet to drive an
  application-level workload. Run the same workloads in Ghostty built at the identical source commit on both machines. Record commands/results. Do not claim Sprite is faster. Explain any Sprite p95 over 110% of Ghostty before accepting the budget.
- [x] Verify:

~~~bash
cargo test --workspace --locked --offline
cargo run -p sprite-term --bin sprite-term-bench --locked --offline -- --samples 3 --output target/checkpoint-1-smoke.json
~~~

Expected: tests pass and smoke JSON contains every metric and numeric budget.

- [ ] Commit:

~~~bash
git add phase_1/crates/sprite-term/src/bin/sprite-term-bench.rs phase_1/crates/sprite-term/tests/benchmark.rs phase_1/docs/performance/checkpoint-1.md
git commit -m "perf(phase_1): freeze checkpoint one budgets"
~~~

### Task 10: Run the Checkpoint 1 review gate

**Files:**

- Create: .github/workflows/phase-1.yml
- Create: phase_1/crates/sprite-term/tests/croft_smoke.rs
- Create: phase_1/scripts/test-croft-main.sh
- Modify only other files required by review findings.

**Interfaces:** Verifies the complete runnable path and permanent seams consumed by later checkpoints.

- [x] Create phase-1.yml for pull requests that touch phase_1, .gitmodules, or
  its workflow,
  pushes to phase_1, nightly schedule, and manual dispatch. Pin every third-party
  action to a full reviewed commit. Its Arch Linux and macOS jobs check out
  submodules recursively, use an Arch container pinned by digest, install the
  exact Rust/Zig toolchains and required native build packages, record the macOS
  runner image, run the one explicit Cargo/Ghostty fetch stage, generate
  pinned terminfo, and then run the same offline locked format/lint/test/build
  gate below. A separate network-enabled step runs test-croft-main.sh and uploads
  `croft-main-commit.txt` plus logs even on failure. Hosted CI is compile/headless
  coverage; the native Wayland, X11, and interactive real-macOS gates below
  remain required because emulation or a headless compositor is not equivalent.

- [x] Verify provenance:

~~~bash
test "$(git -C phase_1/vendor/ghostty rev-parse HEAD)" = "ab0b9da9e88fcb4b0533a1854e84628f663930af"
git submodule status phase_1/vendor/ghostty
cargo metadata --manifest-path phase_1/Cargo.toml --locked --offline --format-version 1 >/dev/null
infocmp -A phase_1/target/terminfo xterm-ghostty >/dev/null
test -s phase_1/LICENSE-MIT
test -s phase_1/LICENSE-APACHE
~~~

Expected: test exits zero, submodule status begins with a space rather than - or +, and offline metadata succeeds.

- [x] Run the local gate without network access:

~~~bash
cd phase_1
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked --offline -- -D warnings
cargo test --workspace --locked --offline
cargo build --workspace --locked --offline
cargo tree --locked --offline --duplicates
cargo tree --locked --offline --edges features
~~~

Expected: all exit zero with no warnings; every duplicate and enabled feature is
either required by GPUI/platform integration or explained in DEPENDENCIES.md.

- [ ] **OUTSTANDING — needs real macOS hardware.** Repeat the explicit fetch step followed by offline locked test/build and
  product smoke on real macOS. Repeat native Wayland/X11 smoke on Arch. Attach
  commands, versions, and results to review. A real macOS result is mandatory;
  a cross-compile or mock is not a substitute and Checkpoint 2 may not begin
  without it.
- [x] Add ignored external test `croft_checkpoint_one_capabilities`. It reads an
  absolute executable from SPRITE_CROFT_BIN, creates its entire fixture in a
  unique temporary directory, launches Croft through public TerminalSession with
  `--open-file fixture.txt --zen`, and supplies the same TERM, TERMINFO,
  TERM_PROGRAM, and TERM_PROGRAM_VERSION identity that a Sprite login shell
  passes to its descendants. It
  asserts: a nonempty Alternate-screen
  snapshot; typed marker text becomes visible; a 40x100 Resize is reflected in
  a newer coherent snapshot; and begin_shutdown joins within the watchdog. It
  never imports Croft or private sprite-term types and removes the fixture.
- [x] Add `scripts/test-croft-main.sh`. It uses `mktemp -d` plus a cleanup trap,
  enables `set -euo pipefail`, resolves and changes to the phase_1 workspace
  from its own script location,
  shallow-clones `https://github.com/vitali87/croft.git` branch `main`, writes
  the resolved SHA to `target/croft-main-commit.txt`, builds Croft unmodified
  with its committed lockfile, then exports the absolute binary path and runs
  only the ignored test above with Sprite's `--locked --offline` flags. It tees
  build/test output to `target/croft-main.log` without masking the command exit
  status and fails if Croft has any tracked diff after the run. The
  clone/build is the explicit network-enabled external phase; ordinary Rust
  tests never call this script.
- [ ] **OUTSTANDING.** The script and the ignored test exist and are wired into CI,
  but neither has been executed: Croft has never been cloned or built here, so
  the capability matrix is entirely unmeasured. Run that wrapper on the Arch and real-macOS validation machines. Record
  Kitty graphics, mouse, embedded-terminal, and richer rendering cases as
  expected missing capabilities assigned to later checkpoints, not false
  Checkpoint 1 passes. Any regression in a capability Checkpoint 1 claims blocks
  acceptance. From Checkpoint 4 onward the complete Croft matrix is
  merge-blocking.
- [x] Inspect forbidden states:

~~~bash
rg -n "unsafe impl.*(Send|Sync)|gpui[-_]ghostty|tty7|tokio|async_std|smol|crossbeam" phase_1 --glob '*.rs' --glob 'Cargo.toml'
rg -n "\bunsafe\b" phase_1/crates --glob '*.rs'
rg -n "libghostty|portable_pty" phase_1/crates/sprite-app --glob '*.rs'
rg -n "thread::sleep|Timer::after|request_animation_frame" phase_1/crates/sprite-app --glob '*.rs'
~~~

Expected: zero unexplained matches. The only local unsafe operation is the
audited raw-descriptor borrow in pty_unix.rs with its lifetime proof; all other
unsafe code belongs to reviewed dependencies.

- [ ] **OUTSTANDING — human review not yet requested.** Request review focused on libghostty ownership, I/O-pump and child-waiter
  shutdown, separate lossless/latest-only streams, lossless bytes, bounded input
  latency, newest-generation coalescing, idle behavior, provenance, and platform
  parity.
- [ ] **OUTSTANDING — depends on the review above.** Fix findings one red-green cycle at a time, rerun the entire gate, then commit:

~~~bash
git add .github/workflows/phase-1.yml phase_1/crates/sprite-term/tests/croft_smoke.rs phase_1/scripts/test-croft-main.sh
git add -u phase_1/crates phase_1/Cargo.toml phase_1/Cargo.lock phase_1/DEPENDENCIES.md
git diff --cached --check
git commit -m "test(phase_1): close checkpoint one review"
~~~

Checkpoint 1 is accepted only after hosted CI, the moving-main Croft capability
smoke, Arch Wayland/X11 smokes, real-macOS smoke, Ghostty comparison, and
committed numerical budgets pass. Checkpoint 2 planning starts from accepted
code and does not replace the Terminal Session seam.
