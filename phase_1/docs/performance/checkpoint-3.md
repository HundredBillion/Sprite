# Checkpoint 3 performance baselines and budgets

**Status: Arch Linux only.** Under the Linux-first posture, macOS baselines join
the macOS acceptance milestone. See [checkpoint-1.md](checkpoint-1.md), whose
machine description still applies — same hardware, same toolchain, same pinned
Ghostty commit.

## Budgets

Thirty samples, release build, on an otherwise idle machine. Budgets are 110% of
this run's p95. Machine-readable copy: `checkpoint-3-arch.json`. Produced by
`sprite-observation-bench`, which measures the observation paths only.

| Metric | Median (ms) | p95 (ms) | Budget (ms) |
|---|---:|---:|---:|
| `collect_four_panes` | 0.001 | 0.001 | 0.001 |
| `collect_sixteen_panes` | 0.003 | 0.003 | 0.003 |
| `collect_with_one_stalled_pane` | 500.196 | 500.205 | 550.225 |
| `encode_default_request` | 1.239 | 1.288 | 1.417 |
| `encode_maximum_history` | 15.472 | 22.559 | 24.815 |

Checkpoint 1's and Checkpoint 2's budgets carry forward unchanged and still
pass; nothing in this checkpoint touches the render path.

## What these measure, and what they deliberately do not

The panes in this harness are stand-ins that answer immediately, not real
terminals. That is on purpose: the cost of an actual capture belongs to
`sprite-term`'s own budgets — `capture_with_full_scrollback` and the history
measurements in the Checkpoint 3 TSP — and folding it in here would hide the
broker's and the encoder's contribution behind it. What these numbers isolate is
the machinery Checkpoint 3 added.

**`collect_four_panes` and `collect_sixteen_panes`** are the broker's own
overhead: dispatching a request to every pane, waiting on the answers, and
ordering the report. At a few microseconds it is nothing next to a real capture,
which is the point — a request costs what the panes cost, not what brokering
them costs. Quadrupling the panes roughly triples the cost, so nothing here is
quadratic in pane count.

**`collect_with_one_stalled_pane`** is the number worth watching. It lands
within a fifth of a millisecond of the 500 ms deadline, which is the evidence
that the deadline bounds the *whole request* rather than each pane: with three
healthy panes and one that never answers, a per-pane deadline would take four
times as long and blow this budget immediately. It also confirms the request
gives up close to the deadline rather than comfortably after it.

**`encode_default_request`** is a typical answer — four panes, 500 lines of
history each — at about 1.2 ms. **`encode_maximum_history`** is four panes each
holding the maximum 5,000 lines, at about 15 ms.

Neither encoding measurement crosses the 16 MiB response limit, so neither
includes the cost of shedding. That path is measured separately in
`the_real_limit_holds_at_the_scale_it_exists_for`: about **80 ms** in release to
bring a 24 MiB response down to 16,246,273 bytes, which is inside the request
deadline that produced it.

## Reproducing

~~~bash
cargo build --release -p sprite-app --locked --offline
./target/release/sprite-observation-bench --samples 30 \
  --output docs/performance/checkpoint-3-arch.json
~~~

Run on an idle machine. Checkpoint 2 recorded a case where a benchmark taken
while a background build was running reported a fix as a regression; the same
caution applies here.
