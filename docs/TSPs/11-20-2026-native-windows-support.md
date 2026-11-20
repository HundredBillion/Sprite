# Native Windows Support Technical Spec

> **For agentic workers:** REQUIRED SUB-SKILL: Use dmi-superpowers:subagent-driven-development (recommended) or dmi-superpowers:executing-plans to implement this TSP task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build and test a native Windows Sprite executable without weakening its local-only Pane Control and Surface Channel boundary.

**Architecture:** Keep the application and terminal-domain APIs platform-neutral. Replace Unix-specific PTY I/O and local-socket internals with narrow platform implementations: `portable-pty`/ConPTY for a Terminal Session and named pipes protected by the current logon SID for local application protocols. Preserve existing Unix behavior behind its current implementations.

**Tech Stack:** Rust 1.98.1, `portable-pty` 0.9.0, Windows ConPTY, Windows named pipes, `windows-sys`, GPUI 0.2.2, GitHub Actions Windows runner.

## Global Constraints

- PRD: `docs/PRDs/11-20-2026-native-windows-support.md`.
- Supported Windows target: `x86_64-pc-windows-msvc`, Windows 10 version 1809+ and Windows 11.
- No TCP listener, WSL launcher, default named-pipe security descriptor, or new PTY dependency.
- Windows local endpoints must permit only the current logon SID and reject network clients.
- The Surface Channel remains key-authenticated; Pane Control remains same-user access without a key.
- Unix process-group cleanup remains Unix-only. Windows explicitly owns and terminates the direct child only.
- Linux and macOS packaging, Unix-socket names, terminal behavior, and tests stay unchanged.
- Every kept Windows behavior added to `docs/capabilities.toml` names a real test.

---

## File Structure

| File group | Responsibility |
|---|---|
| `crates/sprite-term/src/pty.rs`, `pty_unix.rs`, `pty_windows.rs` | Platform-selected bounded PTY input/output pumps. |
| `crates/sprite-term/src/worker/{mod.rs,start.rs,closing.rs}` | Spawn, resize, exit, and shutdown with platform-specific ownership. |
| `crates/sprite-term/src/shell.rs` | Default shell discovery and environment behavior per platform. |
| `crates/sprite-app/src/transport/{mod.rs,unix.rs,windows.rs}` | Private, cancellation-aware byte-stream endpoint transport. |
| `crates/sprite-app/src/{local_socket.rs,control,surface,timeouts}.rs` | Protocol adapters migrated to the transport abstraction. |
| `Cargo.toml`, crate manifests, `Cargo.lock` | Windows-only dependency/features. |
| `.github/workflows/ci.yml`, `README.md`, `packaging/README.md` | Windows CI artifact and user build instructions. |

### Task 1: Establish the Windows build and shell contract

**Files:**
- Modify: `Cargo.toml`, `crates/sprite-app/Cargo.toml`, `crates/sprite-term/src/lib.rs`, `crates/sprite-term/src/shell.rs`
- Test: `crates/sprite-term/src/shell.rs`

**Interfaces:**
- Produces `login_shell() -> Result<SessionConfig, SessionError>` on all three supported targets.
- Produces a Windows target dependency set with GPUI's `windows-manifest` and only the Windows API features used by Sprite.

- [ ] Add the Windows target to the local toolchain and run `cargo check --target x86_64-pc-windows-msvc --workspace` to capture the baseline compiler failures.
- [ ] Write platform-gated tests that resolve `pwsh.exe` from `PATH`, fall back to `%SystemRoot%\\System32\\WindowsPowerShell\\v1.0\\powershell.exe`, and preserve current Unix fallback order.
- [ ] Split executable validation and shell discovery into Unix and Windows helpers; Windows accepts an existing absolute executable and never tests Unix permission bits.
- [ ] Remove the Terminal Core's Linux/macOS-only `compile_error!`, keeping unsupported-target rejection explicit.
- [ ] Enable the Windows GPUI manifest feature and add an exact, Windows-target-only `windows-sys` dependency after confirming the resolved version and required feature modules.
- [ ] Run `cargo fmt --all`, the focused shell tests on Linux, and the Windows target check. Commit: `Add the Windows shell and build contract`.

### Task 2: Make PTY I/O a platform-selected Terminal Core seam

**Files:**
- Create: `crates/sprite-term/src/pty.rs`, `crates/sprite-term/src/pty_windows.rs`
- Modify: `crates/sprite-term/src/lib.rs`, `crates/sprite-term/src/pty_unix.rs`, `crates/sprite-term/src/worker/mod.rs`, `crates/sprite-term/src/worker/start.rs`
- Test: `crates/sprite-term/tests/session_io.rs`, platform-gated unit tests in the PTY modules

**Interfaces:**
- Produces `Pump`, `InputQueue`, `OutputChunk`, and `PumpOutcome` with the existing worker message contract.
- Consumes `Box<dyn MasterPty + Send>` rather than a Unix raw descriptor.

- [ ] Write tests at the existing `InputQueue`/worker-message seam for bounded input, output delivery, EOF, cancellation, and a blocked writer; retain the sixteen-output-permit contract.
- [ ] Move platform-independent permits, channels, output chunks, and input admission into `pty.rs`.
- [ ] Retain Unix `poll` behavior in `pty_unix.rs` behind the same public-in-crate interface.
- [ ] Implement `pty_windows.rs` with independent blocking reader and writer workers obtained from `MasterPty::try_clone_reader` and `take_writer`; cancellation closes their handles and exactly one `PumpStopped` is sent.
- [ ] Change startup to pass the `MasterPty` object to the selected pump and keep `MasterPty::resize` as the resize path.
- [ ] Run Unix session I/O tests, Windows target check, and native Windows PowerShell smoke tests in CI. Commit: `Use a portable PTY pump`.

### Task 3: Preserve Terminal Session lifecycle semantics on Windows

**Files:**
- Modify: `crates/sprite-term/src/{foreground.rs,session.rs}`, `crates/sprite-term/src/worker/{mod.rs,start.rs,closing.rs}`, `crates/sprite-term/tests/{lifecycle.rs,session_jobs.rs}`
- Test: new `cfg(windows)` terminal lifecycle integration tests

**Interfaces:**
- Produces the existing `TerminalEvent` and `ChildExit` results for natural exit and explicit shutdown.
- Windows `Started` stores a direct-child killer; Unix retains `SessionProcesses` and process-group escalation.

- [ ] Write Windows tests that start PowerShell, receive output, accept stdin, resize, exit naturally, and are terminated on Pane close.
- [ ] Gate foreground-program discovery and Ordinary Session Job cleanup to Unix; Windows returns no Unix foreground metadata rather than fabricating it.
- [ ] Give Windows explicit shutdown a bounded direct-child kill/wait sequence and retain the current Unix HUP/TERM/KILL path unchanged.
- [ ] Run all portable Terminal Core tests on Windows and Unix-specific lifecycle tests only where their OS contracts apply. Commit: `Run Terminal Sessions through ConPTY`.

### Task 4: Introduce a private local transport abstraction

**Files:**
- Create: `crates/sprite-app/src/transport/{mod.rs,unix.rs,windows.rs}`
- Modify: `crates/sprite-app/src/{lib.rs,local_socket.rs,timeouts.rs}`, `crates/sprite-app/src/control/{client.rs,watch.rs}`, `crates/sprite-app/src/surface/{channel.rs,client.rs}`
- Test: existing local-socket, control, and surface-channel tests plus Windows transport tests

**Interfaces:**
- Produces a private bidirectional stream type with read/write timeouts, cancellation, connection cap accounting, and peer closure detection.
- Produces a platform endpoint locator: filesystem path on Unix and an opaque named-pipe name on Windows.

- [ ] Characterize existing local-socket tests by behavior: authentication, max connection count, cancellation, bounded writes, endpoint removal, and refusal responses.
- [ ] Extract stream and endpoint operations from protocol code without changing Unix wire bytes, path layout, timeouts, or tests.
- [ ] Move the existing Unix-domain socket implementation behind `transport::unix`.
- [ ] Implement `transport::windows` with `CreateNamedPipe`/`ConnectNamedPipe`, an explicit logon-SID DACL, remote-client rejection, and one pipe instance per accepted connection.
- [ ] Generate endpoint and Surface keys with the OS CSPRNG (`BCryptGenRandom` on Windows); keep their 32-byte representation and constant-time comparison unchanged.
- [ ] Adapt CLI clients and Surface Channel state to opaque endpoints, then run the existing protocol tests on both platforms.
- [ ] Add capabilities for kept Windows terminal and local-interface behavior. Commit: `Serve local Sprite protocols on Windows`.

### Task 5: Qualify the Windows application and publish CI evidence

**Files:**
- Modify: `crates/sprite-app/src/{diagnostics.rs,config.rs,input.rs,workspace/keymap.rs}`, `.github/workflows/ci.yml`, `README.md`, `packaging/README.md`, `docs/capabilities.toml`
- Test: affected Rust tests and `scripts.test_ci_policy`

**Interfaces:**
- Produces `sprite.exe` artifact for the pinned Windows target and documented manual qualification steps.

- [ ] Gate Unix-only diagnostics and configuration permission checks; add Windows equivalents only when their observable output is part of Sprite's promise.
- [ ] Verify Windows key labels and default shortcuts use GPUI's Windows platform mappings while preserving compatibility bindings.
- [ ] Add a time-limited GitHub Actions Windows job that installs the pinned Rust/Zig toolchains, initializes Ghostty, builds the target, runs portable tests, and uploads `sprite.exe`.
- [ ] Document source build, artifact limitations, system-version floor, and native manual checks for live input, clipboard, IME, rendering, Pane Control, and Surface Channel.
- [ ] Run formatting, Clippy, Linux workspace tests, Windows build/test job, capability checks, and CI-policy tests. Commit: `Build Sprite for Windows`.

## Verification Handoff

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --locked --offline -- -D warnings` on Linux
- `cargo test --workspace --locked --offline --no-fail-fast` on Linux
- Windows CI: `cargo check --workspace --target x86_64-pc-windows-msvc`, native Windows portable tests, and `cargo build -p sprite-app --bin sprite --target x86_64-pc-windows-msvc --locked`
- Native Windows manual qualification: run `sprite.exe`; use PowerShell, copy/paste, resize, close, Pane Control, Surface Channel, and a non-ASCII IME input path.
