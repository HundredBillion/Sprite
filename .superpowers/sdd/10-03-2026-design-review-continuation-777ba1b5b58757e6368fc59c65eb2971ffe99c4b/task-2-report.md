# Task 2 — pooled RAII PTY output

Status: implemented and locally verified; independent review belongs to the parent.
Base: `b30550a9e8a54bd57d03e8f0d24fd53e7a90d387`.

## Changes and decisions

- `Message::PtyOutput` owns `OutputChunk`, exposing only `as_bytes()`.
  Its private `Permit` owns an optional buffer and the pool sender/wake socket.
  `Drop` takes the allocation, uses bounded `try_send`, then wakes the pump.
  Disconnected returns free their buffer; a full wake socket cannot block Drop.
- Allocate sixteen 16 KiB buffers at pump startup (262,144 payload bytes).
  A checked-out buffer is the read permit, preventing token/buffer divergence.
  Read directly into it and transfer ownership instead of allocating a copy.
  The pool stores plain buffers and has no thread/receiver ownership cycle.
- Worker parsing drops the chunk immediately after applying its mutation.
  Ordinary discard arms need no explicit permit handling. No `return_permit`
  symbol remains anywhere in `crates/sprite-term/src`.
- Poll uses a fixed stack array, since allocator measurement also covers polling
  and the old temporary poll vector allocated on every iteration.
- The Unix module denies unsafe code, with exceptions only at the existing
  descriptor duplication and macOS process-name boundaries. No new unsafe code
  enters the pump. Test-only allocator forwarding lives in a separate module.
- Tests use Unix socket pairs, bounded receive/write deadlines, and disconnect
  inboxes before joining. Inbox bindings are deliberately moved after Pump
  construction so assertion unwinding drops the inbox before Pump::drop joins.
  The allocation consumer also disconnects and cancels before returning its count.
- Updated ADR 0015 and the performance record. No dependencies changed.

## Red and control evidence

An initial invocation with `--exact` and an unqualified test name selected zero
 tests; it is not counted as evidence. The corrected command was:

```sh
TERM=xterm-ghostty cargo test -p sprite-term --lib \
  pump_delivers_more_than_forty_chunks_when_consumers_only_drop_messages -- --nocapture
```

On the old implementation this ran one test and failed in 2.00 seconds:
`left: 16`, `right: 64`; 0 passed, 1 failed, 37 filtered out. The test dropped
its inbox before shutdown, so the red test joined without hanging. Temporarily
adding the old `pump.return_permit()` after each drop made the same single test
pass. Removing that control and introducing RAII made the drop-only test pass.

This proves the requested drop-only ownership contract. The normal old worker
explicitly returned permits in its parsing and closing loops; these results do
not establish a normal-worker stall.

## Actual allocation measurement

The test allocator forwards `alloc`, `alloc_zeroed`, `realloc`, and `dealloc` to
`System`, counting allocation calls and requested bytes only on the measuring
thread. An allocator control test observes one actual 29-byte allocation and
excludes a 13-byte allocation made outside the scope.

The measurement wraps the real production `run(Ends)` loop. Pool, socket,
channel, and consumer-thread setup are outside measurement; the consumer runs
on another thread. All pump polling, reads, chunk construction and message
sends are inside measurement. The consumer makes 64 separate 15-byte writes,
receives every message, validates bytes, and drops it before the next write.
No parser/snapshot/app allocation claim is made.

```sh
TERM=xterm-ghostty cargo test -p sprite-term --lib \
  steady_state_pump_delivers_sixty_four_chunks_without_allocating -- --nocapture
```

A deliberate temporary `black_box(buffer[..count].to_vec())` in the real read
path caused the test to fail: **64 deliveries, 64 allocations, 960 requested
bytes**, expected zero. After removing the mutation, the same production loop
measured **64 deliveries, 0 allocations, 0 requested bytes**. The separate
pointer-reuse test supplements, rather than substitutes for, this measurement.

## Verification

```sh
TERM=xterm-ghostty cargo test -p sprite-term --lib \
  --test lifecycle --test session_output --test input_backpressure \
  --test graphics_transfer -- --nocapture
```

Passed 69 tests: 45 library, 7 lifecycle, 8 session_output, 1 input_backpressure,
and 8 graphics_transfer; zero failures or ignored tests. Library tests include
9 pump tests and the allocator positive control.

After strengthening the capacity test to drop one exhausted permit and verify
that the pending read resumes, re-ran:

```sh
TERM=xterm-ghostty cargo test -p sprite-term --lib pty_unix::tests -- --nocapture
```

Passed all 9 pump tests (36 filtered out). Cases cover drop-only 64-message
consumption, zero allocations, all sixteen permits held, resumption after one
Drop, sixteen output messages plus the reserved command slot, queued messages
dropped on inbox teardown, rejected sends without explicit cancellation,
chunks surviving pump shutdown, allocation identity returned before wake, a
full nonblocking wake socket, descriptor ownership, and close-on-exec.

```sh
TERM=xterm-ghostty cargo clippy -p sprite-term --all-targets --locked --offline -- -D warnings
cargo fmt --all -- --check
git diff --check
```

All pass. Clippy was rerun after the final capacity-test change. The unrestricted
source census finds zero `return_permit` occurrences and exactly the intended
two unsafe blocks in `pty_unix.rs`, beneath the module deny and narrow allowances.

## Self-review and limits

Checked pool conservation: a buffer is either queued, held by the pump, or owned
by one output message; moving output into a failed send drops its permit. Every
run exit drops the held permit automatically, with no paired manual return.
`try_send` publishes before wake. Pool exhaustion removes read readiness while
leaving input/cancellation readiness intact; dropping output wakes read polling.
Worker queue capacity remains 17, with at most 16 outstanding output chunks.
Worker drain-before-join behavior and lifecycle deadlines remain unchanged.

Linux was exercised. The macOS FFI allowance is configuration-gated and was
reviewed in source; no native macOS execution was available. The requested
focused term suites ran, not a new whole-workspace test gate. Parent-owned plan
and progress files were intentionally excluded from the implementation commit.
