# Shared Surface rows and event batches

Baseline: `08f7c37`, measured by the harness committed in `fe8ad69`.
Measurements below use the same Linux x86_64 debug test harness and fixtures as
[the baseline](design-review-surface-baseline.md). Rust 1.97.1; locked/offline dependencies.

| Measurement | Before | After |
| --- | ---: | ---: |
| Stored grid cell bytes | 32 | 8 |
| 200×60 grid creation allocations / requested bytes | 12,061 / 397,440 | 68 / 98,816 |
| First `render_grid` allocations / bytes | 187 / 2,205,656 | 125 / 1,627,720 |
| Idle `render_grid` allocations / bytes | 122 / 579,528 | 1 / 632 |
| One-row update plus `render_grid` allocations / bytes | 186 / 1,157,139 | 7 / 10,960 |
| Parse/apply 100k list rows allocations / bytes | 400,019 / 45,471,414 | 300,018 / 40,160,012 |
| Replace 100k rows with live anchor allocations / bytes | 400,021 / 47,699,660 | 300,019 / 40,160,018 |
| State-only update on 100k rows allocations / bytes | 15 / 792 | 15 / 792 |
| Five-event wheel gesture socket syscalls | 10 | 1 |

Allocation scopes count allocation/reallocation requests on the current Rust
thread. Fixture JSON creation is outside the list scope; parsing, applying,
index construction and anchor preservation are inside it. Grid scopes call
production `render_grid`, including its element construction, but exclude GPUI
window drawing/font shaping/GPU submission. The first render includes lazy
process initialization. These are deterministic work counts, not timing or a
whole-GPUI zero-allocation claim. The idle Surface render still allocates its
632-byte element. The new test budgets cover idle/one-row rendering and list
parse/replace/state work; no historical budget was changed.

The real GPUI `uniform_list` path, including both headers' `truncate_line`
calls, executes **51 truncations for 47 visible rows**, below `visible + 16`.
This holds at top rows 0, 49,954 and 99,953 after revealing stable IDs `r0`,
`r50000` and `r99999`. The baseline also executed 51 calls for its first frame;
virtualization was already present. The improvement is shared config/text and
removing repeated index construction, with a regression guard for bounded
render work. A 100k-row replacement builds one index; state-only updates build
none and preserve both the existing index storage and row Arc.

Grid `Cell` stores two `u32`s. The text pool counts live cell references,
recycles vacated slots and removes unused lookup keys. A blank sentinel keeps
one permanent reference. The pool has at most grid-area plus two slots,
including transient incoming text; tests also bound entries/free-list capacity
and map capacity relative to grid area. Clear releases strings; resize rebuilds
the pool so downsizing releases old high-water metadata. Already-issued render
rows own `CellText` independently of recycled pool IDs. The 10k-distinct-write
test retains old frames across reuse, checks immutable text, then verifies
clear and shrink reclamation. A 64-seed, 100-transition-per-seed oracle checks
writes, scroll copies, clear, resize, malformed dimensions and Unicode/spacers.
Failure messages name the seed and shortest failing operation prefix.

The wire buffer lives under the same mutex as the stream. `send_batch` appends
all newline-delimited messages before `write_all`; queued batches follow
`opened`. The normal five-event gesture is 399 bytes in one `sendto`. Partial
writes still retry through Rust's `write_all`; errors preserve the already-sent
prefix, mark the connection dead and shut it down. Tests cover four concurrent
senders, queued opening, buffer reuse, closed peers, a 4 MB batch drained in
1024-byte reads, and a backpressured timeout with a nonempty exact prefix.
The one-syscall budget applies to the ordinary empty-buffer socket-pair probe,
not to arbitrary backpressure or payload sizes.

## Reproduction

```sh
TERM=xterm-ghostty cargo test --locked --offline -p sprite-app --lib surface_performance -- --nocapture
TERM=xterm-ghostty cargo test --locked --offline -p sprite-app --lib surface_list_100k_virtualization_probe -- --nocapture
TERM=xterm-ghostty cargo test --locked --offline -p sprite-app -p sprite-pane --all-targets
TERM=xterm-ghostty cargo clippy --locked --offline --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
TERM=xterm-ghostty /tmp/sprite-syscall-tools.B9rhrh/usr/bin/strace -f -e trace=write,writev,sendto,sendmsg -s 2048 -o /tmp/task8-wheel-after.trace target/debug/deps/sprite_app-b1d7a61a7846d134 surface_wheel_syscall_probe --nocapture --test-threads=1
python scripts/check_surface_wheel_trace.py /tmp/task8-wheel-after.trace
```

The test executable hash is build-specific; use the path printed by `cargo test`.
The tracer was privately extracted from the signed official Arch package and
made no system installation. The probe uses a local socket pair and artificial
Surface ID 987; it never reads real authentication secrets. The checker rejects
the captured baseline with `wheel syscall budget exceeded: 10 != 1` and accepts
the optimized trace: `wheel gesture: 1 syscall, 399 bytes, 5 complete JSON lines`.

Verification: 540 tests across app/pane all-targets (511 app library/GPUI tests,
3 pane tests, remaining binary/integration tests), zero failures; workspace
all-target clippy and format checks pass. The existing proc-macro-error2 2.0.1
future-incompatibility notice remains. Intentionally restoring a second
`id_index` call in apply made the index regression fail (2 builds instead of 1);
restoring the implementation passed the 125-test Surface filter.

## Dependency contracts

- Manifest and lock pin GPUI 0.2.2. Its installed primary source
  `src/elements/uniform_list.rs:456` invokes the callback for the visible range;
  `:656` also invokes it for a measurement row. `src/shared_string.rs:116` accepts
  `Arc<str>` through `ArcCow` without copying text. Tests instrument the actual
  callback's `truncate_line` calls rather than a simulated range loop.
- Manifest and lock pin serde_json 1.0.151 with `preserve_order`. Existing
  `event_mouse` and resize encoders remain the grammar/field-order authority.
  Only transport aggregation and resize tuple comparison changed.
- Installed Rust 1.97.1 standard-library source `std/src/io/mod.rs:1858` documents
  and implements `write_all` retrying partial writes and interruptions;
  `std/src/os/unix/net/stream.rs:656` delegates UnixStream writes to the socket.
  No new dependency or undocumented GPUI API was introduced.
