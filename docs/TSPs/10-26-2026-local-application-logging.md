# Local Application Logging Technical Spec

> **For agentic workers:** REQUIRED SUB-SKILL: Use dmi-superpowers:executing-plans to implement this TSP task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give Sprite window processes safe, persistent, filterable local diagnostics.

**Architecture:** Install a tracing subscriber only for a window invocation. A bounded lossy queue feeds a background writer with size-based rotation and per-process locking; lifecycle and request events carry only explicitly selected, non-secret fields.

**Tech Stack:** Rust 1.97.1, tracing, tracing-subscriber, bounded standard-library channel, Unix file locks, Sprite application and terminal benchmarks.

## Global Constraints

- Preserve CLI and protocol output; no application diagnostic output to stdout.
- No raw terminal input/output, credentials, tokens, or full environment values by default.
- Keep application operation independent of logging errors and slow destinations.
- Bound queue bytes, per-process files and completed-log retention; do not delete active logs.
- No per-character or per-frame logging. Timings are monotonic elapsed durations, not CPU time.
- Document native-platform limitations and measure representative idle and high-output overhead.

## Design investigation

`tracing` 0.1.44 is already resolved in Cargo.lock. `tracing-subscriber` 0.3.20 is selected and locked; `tracing-appender` is not used. The [nonblocking builder](https://docs.rs/tracing-appender/0.2.4/tracing_appender/non_blocking/struct.NonBlockingBuilder.html) bounds queued lines and drops when lossy; this alone does not bound bytes per line. Its [rolling appender](https://docs.rs/tracing-appender/latest/tracing_appender/rolling/) rotates by time, so it does not enforce the approved size cap. A bounded formatter and size-limited writer supply the missing guarantees. The standard [file locking API](https://doc.rust-lang.org/std/fs/struct.File.html#method.try_lock) permits nonblocking advisory locks and releases locks on descriptor close; deletion must coordinate via a shared directory lock so a new owner cannot race cleanup.

The existing `sprite-term-bench` measures the terminal core without running `sprite-app` logging initialization. It is not adequate evidence of logger overhead on its own; the measurement must launch a real app window and record comparable idle and high-output runs.

## Implementation tasks

The tasks below build the sink first, then instrument it, and finally document and measure it.

### Updated-base findings

`crates/sprite-app/src/control/mod.rs::open_grant` now uses the shared `LocalSocket` transport for pane control. Correlation must use a locally generated numeric connection ID, not the client's request ID, socket key, request body, pane contents, command, or environment. The shared listener already generates a per-listener connection ID in `LocalSocket::open_in`; pair it with the transport policy name to identify a request without exposing client-supplied data. Surface and observation use the same transport, but their application-level failure paths still need separate instrumentation.

### Retention protocol to validate

Use a private root and a distinct PID-and-timestamp run directory per process; retain an exclusively locked marker file for the lifetime of the writer. Serialize run-directory creation and cleanup with a separate shared root lock. Cleanup must acquire the root lock before attempting each marker lock, and must skip any run whose marker lock cannot be acquired or whose identity cannot be verified. Delete only closed run directories while holding both locks. Never unlink the shared root lock. Crash-released marker locks allow a later cleanup to reclaim abandoned runs. Both lock acquisition and filesystem work belong on the writer thread, not a terminal or UI hot path. Fail closed for diagnostics if locking or private-directory creation fails.

Within each run directory, use fixed-size segment files and a fixed segment count, enforce the byte limit even on unusually large events, and cap the formatted event before enqueueing. The implementation caps events at 4 KiB, the queue at 512 events, and each run at four 1 MiB segments; ten completed runs are retained. Test multiple processes, cleanup races, crash recovery, oversized events, full queue and failed writes. Avoid lock-dependent cleanup on filesystems whose locking semantics cannot be trusted.

### Event and privacy constraints

- Use numeric IDs assigned by Sprite for pane, tab, and transport connection context. Client-supplied IDs and names may contain secrets and must not be emitted directly.
- Record operation class and outcome from a closed set of labels; do not record protocol requests or replies. `control::open_grant` may return terminal content, so its response must never be formatted into a log event.
- Never format arbitrary error or panic payload strings unless their origin and contents have been audited. Record error category and panic source location where available, and chain the prior panic hook.
- Existing diagnostics printed to stderr remain in place when they are part of the CLI or user-facing behavior.

### Verification matrix

1. Level and target directives: capture a WARN, reject DEBUG by default, admit DEBUG for one selected target, and reject it for a second target.
2. Bounded formatting and queue: submit an event larger than the per-event ceiling, saturate a deliberately stalled writer, check fixed memory/queue limits and dropped-event accounting, and verify that the producer returns without waiting for the writer.
3. Rotation: write past the segment cap and verify each segment and the aggregate per-run cap, including a single oversized event and a partially failed write.
4. Retention: run two processes, retain a live marker while cleanup runs, terminate a child without graceful shutdown, then reclaim the abandoned run on the next cleanup. Exercise simultaneous startup and cleanup.
5. Failure: use a deliberately unwritable destination and an injected writer that returns disk-full; verify normal operations and no protocol output contamination.
6. CLI: compare stdout, stderr and exit status for help, version, configuration and representative Surface/control commands with logging settings present and absent. Include binary/protocol stdout assertions.
7. Security: emit requests with sentinel values in commands, tokens, bodies, pane content and client IDs; assert the sentinel never appears in diagnostic files.
8. Native workloads: launch a real app on macOS and Linux where available, run paired disabled/enabled idle and sustained terminal output tests, record elapsed time, CPU, RSS, versions, machine and repetitions. Record unavailable platforms explicitly.

### Appender decision

`cargo info` downloaded tracing-appender 0.2.5 and tracing-subscriber 0.3.20. Sprite declares and locks tracing-subscriber 0.3.20. The installed `tracing-appender-0.2.5/src/non_blocking.rs` copies each complete event into its queue without a byte cap and its `WorkerGuard::drop` calls `println!` when the shutdown queue times out. The associated `worker.rs` prints flush errors to stderr. Do not use this worker: provide a small lossy bounded channel with a capped event representation and a detached worker, so stdout remains untouched and a blocked destination cannot delay shutdown. The worker must report dropped events through its own output only when output becomes available. `tracing-subscriber` provides filtering; the diagnostics layer formats directly into a capped event buffer.


### Task 1: Bounded diagnostics sink

**Blocked by:** None

**Files:** Create `crates/sprite-app/src/diagnostics.rs`; modify `crates/sprite-app/src/lib.rs`, `crates/sprite-app/src/main.rs`, `crates/sprite-app/Cargo.toml`, and `Cargo.lock`.

**Interfaces:** Produce `pub fn init() -> Option<Guard>` for the window branch, and `tracing` events filtered through `SPRITE_LOG`. The guard signals shutdown without waiting indefinitely. The writer caps event bytes, queue capacity, segment count, and segment size; run directories hold a locked marker and cleanup holds the root lock. A failed destination drops events without blocking or writing diagnostics to stdout.

- [x] Verify persistence, filtering, bounded queue delivery, rotation and an unavailable destination. The initial persistence and rotation tests were run before implementation.
- [x] Implement the bounded sink and subscriber; use a capped formatter and nonblocking `try_send`. Keep all filesystem I/O and cleanup on its worker. Validate the lock protocol using concurrent processes.
- [x] Run `cargo test --locked -p sprite-app --lib diagnostics` and `cargo fmt --all --check`; record output and fix failures.

### Task 2: Context and timings

**Blocked by:** 1

**Files:** Modify `crates/sprite-app/src/main.rs`, `crates/sprite-app/src/workspace/pane_factory.rs`, `crates/sprite-app/src/workspace/close_gate.rs`, `crates/sprite-app/src/local_socket.rs`, `crates/sprite-app/src/surface/channel.rs`, and `crates/sprite-app/src/control/mod.rs`.

**Interfaces:** Consume `tracing` event macros; use Sprite-created numeric pane, tab and connection identifiers. Never emit client-supplied IDs, payloads, environment values, secrets or pane contents. Startup and request timings use `std::time::Instant`, reported as elapsed milliseconds.

- [x] Compare CLI and Surface command output with detailed logging off and on, including missing-endpoint errors.
- [x] Instrument configuration, startup and shutdown, pane lifecycle and transport failures; add opt-in monotonic timings for startup and request handling. Log only classified error categories and preserve the default panic hook.
- [x] Run `cargo test --locked -p sprite-app` and verify output and failure cases.

### Task 3: Documentation and measurements

**Blocked by:** 1, 2

**Files:** Create `docs/local-logging.md` and `docs/performance/local-logging.md`; modify `README.md`.

**Interfaces:** Document `SPRITE_LOG`, platform paths, active-file discovery, follow/filter commands, limits, dropped-event reporting, crash boundaries and privacy. Measure the actual window application, rather than a core-only benchmark, for paired idle and high-output workloads with detailed logging off and on. Report sample count, commands, OS and hardware, elapsed time, CPU and RSS; label unavailable native evidence.

- [x] Run paired workloads and record raw results and methodology; do not make performance claims beyond evidence.
- [x] Run formatting, relevant tests, CLI output separation, filesystem failure and concurrency tests. Review the full diff against the PRD and issue acceptance criteria.
