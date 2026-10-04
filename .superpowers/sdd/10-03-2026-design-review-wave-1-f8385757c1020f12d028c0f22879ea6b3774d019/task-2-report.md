# Task 2 report

Status: DONE. Base: 8f83cf5. Scope: Wave 1 bells and terminal resource ownership.

## Changes

- The existing `Rc<RefCell<_>>` callback notices now own one `bell_pending` boolean alongside ordered title/directory notices. Each parsed output chunk drains at most one Bell; the next silent chunk emits none. No new callback ownership primitive.
- `Pump::start` synchronously duplicates the PTY descriptor and moves an `OwnedFd` into the pump thread. Polls and writes use safe `AsFd` borrows. Reads and nonblocking setup accept `BorrowedFd`, converting to raw only at the pinned nix safe function calls that require it. Removed the cloned boxed reader and caller-outlives-pump contract.
- Duplication uses `F_DUPFD_CLOEXEC`, so later child execs do not inherit the new pump descriptor. Reused the existing duplication boundary, also preserving that property for foreground tracking.
- Private `Owned` fields declare projector, key encoder, mouse encoder, terminal in destruction order. The terminal's heap-backed callback table supports moving it into the owner. Normal Closing drops one owner; returns and unwinds receive the same field ordering. During partial initialization, reverse local order already releases newly created scratch before terminal state.
- Corrected the direct Projector ownership comment in snapshot.rs. Public session interfaces and TerminalEvent::Bell remain unchanged; the private Pump start operation no longer needs a separate reader argument.

## Red/green evidence

All cargo test commands used `TERM=xterm-ghostty`.

1. `cargo test -p sprite-term --locked --offline --test session_output bell_burst -- --nocapture`: RED, 5-second snapshot watchdog while the event consumer was deliberately paused. The test drops the event receiver before joining even on failure, so reproduction exits rather than hanging shutdown. GREEN after coalescing (one public regression).
2. `cargo test -p sprite-term --locked --offline bell_tests --lib`: RED with the original one-event-per-BEL callback, GREEN with the boolean callback. Uses the production callback registration and drain seam to feed exactly 16 KiB of BEL, then a silent chunk, then another BEL.
3. `cargo test -p sprite-term --locked --offline --lib pty_unix::tests`: RED when closing the caller descriptor (UnexpectedEof on attempted pump write); GREEN with owned duplication. Final test also verifies pump output reads and clean canceled stop after original closure.
4. `cargo test -p sprite-term --locked --offline --lib duplicated_endpoint`: RED with the existing plain dup helper, GREEN with atomic close-on-exec duplication.

Final verification after all production changes:

- `cargo test -p sprite-term --locked --offline --lib`: 37 passed, 0 failed (includes both pump tests and Bell test).
- `cargo test -p sprite-term --locked --offline --test lifecycle --test session_output --test input_backpressure`: 17 passed, 0 failed (8 lifecycle, 8 output, 1 backpressure). Includes descendant shutdown escalation and child exit.
- `cargo clippy -p sprite-term --locked --offline --all-targets -- -D warnings`: passed.
- `cargo fmt --all --check`: passed after initial formatter check identified formatting changes.
- `git diff --check`: passed.
- `rg -n 'unsafe|borrow_raw' crates/sprite-term/src/pty_unix.rs`: exactly two unsafe sites, macOS proc_name and OwnedFd::from_raw_fd at the duplication boundary; no borrow_raw.

## Source grounding

Manifest Cargo.toml pins nix 0.28.0, libghostty-vt 0.2.1, portable-pty 0.9.0; Cargo.lock resolves the same versions for sprite-term. Host rustc: 1.97.1 (8bab26f4f 2026-07-14).

Versioned official docs were attempted first (`https://docs.rs/nix/0.28.0/nix/unistd/fn.dup.html`, `https://docs.rs/libghostty-vt/0.2.1/libghostty_vt/key/struct.Encoder.html`); web tool reported both inaccessible. Used installed version-matched primary source instead, under `/home/hundredbillion/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/`:

- nix-0.28.0/src/unistd.rs:426 returns RawFd from dup; :1095 read takes RawFd, :1105 write takes AsFd. nix-0.28.0/src/fcntl.rs:508,600-605 supports F_DUPFD_CLOEXEC and returns a raw descriptor. Therefore one FromRawFd ownership boundary is necessary with this pinned interface; no dependency upgrade needed. Existing unsafe repeated borrowed descriptors were removable.
- nix-0.28.0/src/poll.rs defines PollFd::new using BorrowedFd. Watch now carries those safe lifetime-bound borrows.
- libghostty-vt-0.2.1/src/key.rs:27,31,136 and mouse.rs:32,36,128: encoder lifetimes represent allocator lifetime, constructors use default allocation, set_options_from_terminal takes only a temporary terminal reference.
- libghostty-vt-0.2.1/src/render.rs:352: update takes a terminal whose allocator lifetime agrees with the render state; returned Snapshot borrows the render state. Projector::new already constructs Projector<'static>, and capture drops transient iterators before returning owned application snapshots.
- libghostty-vt-0.2.1/src/terminal.rs:230-234: callbacks live in a boxed VTable explicitly to retain a stable address when Terminal moves; :744 frees the terminal. No self-referential Rust borrow prevents Owned.

Compilation plus the focused runtime tests verify these choices against the actual pinned dependencies.

## Self-review and limits

Checked callback sharing, chunk reset, terminal initialization failure order, all changed command borrows, FD ownership on spawn failure, nonblocking setup, EINTR/EAGAIN/EIO/EOF branches, and cancellation joining. The socket ownership test exercises both pump directions; public lifecycle/backpressure tests exercise real PTYs.

Coalesced Bell is delivered at the end of the chunk's title/directory notice batch; title/directory relative ordering remains intact. Coalescing bounds BEL amplification per chunk, not an indefinitely undrained lifecycle stream across arbitrarily many chunks. That broader lifecycle backpressure policy is unchanged.

No app changes, no plan/checklist edits, no Wave 2 RAII permit changes. Linux tests were run; macOS-specific proc_name remains unchanged and was not executed. No full workspace test claim.
