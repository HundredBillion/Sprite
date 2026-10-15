# Socket and test reliability verification

Scope: GitHub issues #79, #81, #83 and #85, against `7ad4bc4`.

## Behavior and implementation

- Surface connections keep their existing protocol and authentication deadlines.
  Only their unbounded readers use `SocketReader`. On macOS it polls with a
  one-second retry interval before reading; an idle connection has no expiry.
  The server chains authentication read-ahead ahead of the socket. Linux keeps
  ordinary reads. Existing pipelined-open coverage guards the handoff.
- The tmux producer starts detached and waits for the client-attached hook before
  emitting the image. Ordinary text is replayed on attach, but an earlier
  passthrough image is not. Fixture cleanup runs on panic as well as success.
- Local-socket test coordination uses the shared watchdog. The child process
  gets three watchdog periods for startup, reply and housekeeping. Transport
  policy limits and the listener's separate prompt-shutdown assertion stay intact.
- `scripts/stress_tests.py` builds the selected test binary before starting one
  `yes` process per logical CPU, then runs the full parallel binary sequentially.
  It records every exit status and log, bounds stalled runs, and cleans up its
  own load workers, process groups and private tmux servers. It refuses to count
  absent tmux as coverage.

## Source evidence

`Cargo.toml` and `Cargo.lock` pin nix 0.28.0. Its installed `src/poll.rs`
documents readiness, millisecond timeouts, EINTR and draining data before EOF.
The local precedent is `surface/client.rs::CancellableInput`.

The local runtime is tmux 3.6a. Its [versioned manual](https://github.com/tmux/tmux/blob/3.6a/tmux.1)
defines `client-attached` and `wait-for -S`; these coordinate the fixture without
sleeping to guess when attachment finishes.

## Baseline evidence

On macOS 26.5.2 / Darwin 25.5.0 arm64, the unmodified workspace suite passed
using Homebrew Rust 1.98.1 and pinned Zig 0.17.0
(1,064 passing tests summed across 49 reported results, including child test
summaries; this is not a count of unique tests).

A standalone PTY probe starts tmux detached, waits for the producer, then attaches:

| Producer | DONE visible | Image forwarded |
|---|---|---|
| Emits before attachment | yes | no |
| Waits for attachment | yes | yes |

The original tmux binary passed 30 loaded runs and the two reported local-socket
tests passed 30 loaded runs in this session. Those runs do not reproduce the
reported flakes; the deterministic tmux probe does reproduce image loss.
The deadline changes follow the reported timeout traces and preserve assertions.
The unmodified client binary also passed 500 loaded runs; the rare OS hang was
not reproduced in this sample. Final verification uses the pinned Rust 1.97.1;
the initial PATH selected Homebrew ahead of rustup, which was corrected after
Clippy 1.98 reported new lints in unchanged PNG decoder code.

## Acceptance commands

From the repository root, with the pinned Rust and Zig on PATH:

```sh
python3 scripts/stress_tests.py graphics_tmux --runs 100 --output stress/graphics_tmux
python3 scripts/stress_tests.py app --runs 30 --output stress/app
python3 scripts/stress_tests.py client --runs 3000 --output stress/client
```

For native x86-64 Linux, run `client`, `app`, `graphics_policy` and `graphics_tmux`
with `--runs 100`. The script defaults `SPRITE_TEST_WATCHDOG_SECS` to 20.

The existing `ci` workflow accepts `workflow_dispatch` input `stress=true`.
It runs the native Arch container on an x86-64 hosted runner and the macOS loops,
then uploads `stress-linux` and `stress-macos`. Normal PR CI keeps its shorter
job limits. Acceptance results will be recorded after execution, not inferred
from a passing build or a single suite run.

## Bounded variant review

The local-socket search starts with the reported two-second `recv_timeout`,
then includes condition deadlines and socket read deadlines in the same test
module: 18 two-second channel waits, three two-second condition deadlines,
six one-second socket reads, one two-second socket read, and the eight-second
child-process deadline. These are coordination limits, not assertions about production timing;
they now share `watchdog()`. The transport policy's 200 ms deadlines, explicit
30-second handshake setup and one-second prompt-shutdown assertion are separate
contracts and remain unchanged.

Surface-reader review covers `first_line`, the client events loop and
`serve_surface`. Short one-shot exchanges retain their existing receive timeout;
stdin already waits with poll. No other surface-read workaround is added.

## Local verification of the change

With Rust 1.97.1 and Zig 0.17.0:

- `cargo fmt --all -- --check`: passed.
- `cargo clippy --workspace --all-targets --locked --offline -- -D warnings`: passed.
- `cargo test --workspace --locked --offline --no-fail-fast`: passed, including
  740 app unit tests, 21 client integration tests and all three tmux tests.
- `python3 -m unittest scripts.test_ci_policy scripts.test_findings scripts.test_stress_tests -v`:
  22 passed, including killing a detached tmux server after its test times out.
- `python3 scripts/check_findings.py`: passed.
- Ruby's YAML parser accepted the workflow.

Independent review checked socket timeout separation, buffered-byte preservation,
tmux setup and cleanup, and the stress runner. Its server-cleanup finding was
fixed and re-reviewed with no remaining important findings. The idle-reader test
is behavioral coverage, not a deterministic reproduction of the Darwin kernel
bug; the long client loop provides the statistical acceptance evidence.

## Additional failure found by the acceptance loop

The first loaded app loop passed 29/30 full runs. Both reported local-socket
regressions passed all 30; run 23 failed only
`a_silent_handshake_is_refused_after_the_timeout`.

That test started its stopwatch after `connect`, but the server can start its
handshake deadline after `accept`. Scheduling between those operations made
the measured remainder shorter than the handshake budget. Adding a temporary
500 ms pause after connect reproduced the same assertion failure (0/1 passed).
Starting the stopwatch before connect passed with the same pause (1/1), then
passed again after removing the probe. Production handshake behavior is unchanged.
Clippy and the 21 client tests also passed after the test-only correction.

Variant search: `elapsed() >=` has three matches in sprite-app source/tests.
The handshake test was affected; the reload test starts before sending its
request, and the startup benchmark measures its own loop, so both are safe.

The repeat app loop then exposed a different test setup race: run 2 failed in
`silent_handshakes_are_counted_against_the_connection_cap` while setting the
excess connection's read timeout (`EINVAL`). Darwin rejects socket options
after the peer has closed. A socket-pair test with the peer already dropped
reproduced this deterministically. Both connection-cap tests now use a bounded
readiness wait followed by the same EOF assertion, avoiding the socket option.
The new regression and all 18 local-socket tests pass. The remaining five
read-timeout setups precede the operation that permits peer closure; they do not
have this immediate-rejection race. The interrupted repeat loop is retained as
failed evidence, not counted as a clean acceptance run.

The first native Arch normal gate passed, but stress startup failed before any
iteration because Git rejected the container checkout's ownership. The Linux
stress step now registers only `$GITHUB_WORKSPACE` as a safe directory before
collecting commit metadata. This setup failure supplies no Linux stress evidence.
Both follow-ups received an independent review with no findings.
