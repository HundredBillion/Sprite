# Prompt startup and throughput review

Reviewed master `afece5293271b410d83c63e236938ff2a7e0a250` on 2026-10-11.
Production code is unchanged. The accompanying prompt tests establish a baseline;
they do not implement the recommendations below.

The first optimization to pursue is font discovery before session creation.
Sprite enumerates installed fonts for every new pane, twice with default font
settings, before starting the shell. The engine itself delivered a clean shell's
first directory prompt in about 5 ms at p95 in this release-build measurement.
The user's actual shell and native frame presentation have not been measured.

## Findings, in recommended order

### P2: Font enumeration delays every pane before its shell starts

`TerminalView::new` calls `chosen_family` before `TerminalSession::spawn`
([constructor](../../crates/sprite-app/src/terminal_view.rs#L283)).
`chosen_family` enumerates fonts, then its default/missing-font fallback enumerates
again ([selection](../../crates/sprite-app/src/terminal_view/theme.rs#L100)).
GPUI's wrapper sorts and deduplicates the result; its macOS implementation creates
a CoreText collection and walks its descriptors on each call
([platform implementation](../../vendor/gpui/src/platform/mac/text_system.rs#L100)).
This work runs on the UI thread for startup, new tabs, and splits.

A standalone macOS probe using the same CoreText collection/descriptor operations
found 187 families and took 54.8 ms for its first enumeration, then 33.7–36.6 ms
per enumeration. This is a platform microbenchmark, not an instrumented Sprite
constructor. The proxy estimate for two warm enumerations is approximately 68 ms.
That is not a measured Sprite constructor cost or end-to-end saving.

Recommendation: first reuse one enumeration within selection, then cache font
resolution across panes with unchanged font settings. Define invalidation for
font registration/installation. Preserve physical font metrics before starting
the child. Add a counting test for default, installed, and missing families and
an actual action-to-first-paint benchmark before claiming the improvement.

### P2: The existing startup benchmark cannot detect a slow prompt

`spawn_to_ready` launches `sleep 30`, waits for `Ready`, then shuts down
([benchmark](../../crates/sprite-term/src/bin/sprite-term-bench.rs#L269)).
`Ready` is published before initial capture and before processing child output
([worker](../../crates/sprite-term/src/worker/mod.rs#L319)).
It measures session machinery, not an initialized interactive shell or visible
prompt. The paint benchmark also excludes native window presentation.

The added `prompt_latency` test waits for an interactive Bash directory prompt
in an actual terminal snapshot. A controlled 250 ms child-start delay yielded
`Ready` at 3.8 ms but the prompt at 272.9 ms. Replacing the measured prompt time
with `Ready` time made the control fail, as intended. This closes the engine-side
measurement gap; it does not cover the GPUI or compositor part.

### P2: Output can trigger more snapshot work than the display needs

The worker output batch limit and maximum PTY chunk are both 16 KiB. Each pass
attempts capture; a full-size chunk can therefore end a pass by itself
([batch/handler](../../crates/sprite-term/src/worker/mod.rs#L46)).
An empty latest-snapshot slot permits projection. `SnapshotStream::next` frees
that slot and asks for another capture; the UI consumer keeps draining it without
a presentation-rate gate ([consumer](../../crates/sprite-app/src/terminal_view.rs#L404)).
A fast consumer can repeatedly project scrolling output between displayed frames.
This competes with parsing/input on the owner thread and with other UI updates.
Backpressure avoids the work when the receiver stays full; this is conditional,
not one capture for every chunk under every workload.

Recommendation: benchmark live output with immediate, display-paced, and paused
snapshot consumption. Count captures and parsed bytes, and measure input and
new-prompt latency. Then consider presentation-driven capture demand while keeping
first/quiet output immediate and guaranteeing delivery of the final dirty state.
Do not simply enlarge parsing batches without checking latency. Current throughput
impact and an appropriate cadence have not been measured here.

### P2: Refused images still consume conversion time on the UI thread

`GraphicsCache::texture` converts the full image to BGRA before checking its
memory budget ([cache](../../crates/sprite-app/src/graphics_cache.rs#L77)).
Refused images are not cached. Subsequent text snapshots retaining the same image
repeat the conversion. `refresh_textures` happens before installing/notifying the
text snapshot ([delivery](../../crates/sprite-app/src/terminal_view.rs#L417)).
A graphical shell banner or an image-heavy background pane can therefore delay
unrelated text/UI work. Plain text without images does not incur this cost.

Recommendation: validate dimensions and checked output byte size against the
budget before conversion. A focused allocation/call-count test should demonstrate
that an over-budget image is rejected without conversion on repeated calls.
Also benchmark multiple individually valid images that exceed the cache together.

### P2: Graphics capture has quadratic image-ID bookkeeping

For each placement, `capture_frame` scans the accumulated image vector, then cache
retention scans that vector again for each image
([graphics capture](../../crates/sprite-term/src/graphics.rs#L400)).
With P placements and I distinct images, these lookups cost O(P×I + I²) even when
pixels are unchanged. Text mutations can repeat them.

Recommendation: collect referenced IDs in a set for deduplication and retention,
keeping deterministic output order and pixel sharing. Test 1/32/128/512 distinct
small images and repeated placements of one image. Algorithmic cost is confirmed;
wall-clock impact at real image counts is unmeasured.

## Other opportunities requiring targeted evidence

- Startup socket cleanup scans the runtime directory and synchronously connects
  to existing sockets before creating the first pane (`local_socket.rs:488`,
  `workspace/mod.rs:211`). Connects have no deadline. Reproduce backlog pressure
  in a private test directory before claiming a platform-specific startup hang;
  bound or move housekeeping without delaying endpoint credentials.
- New panes start at 24×80 and receive their real size during render
  (`terminal_view.rs:309`, `terminal_view/geometry.rs:137`). This creates extra
  resize/reflow work; prompt-redraw impact depends on scheduling. Test the child's
  initial observed dimensions before changing initialization order.
- Snapshots without an OSC title query foreground state before notifying text
  (`terminal_view.rs:872`). Cosmetic title discovery could use the existing clock;
  close confirmation still needs fresh process information.
- Expired history requests still execute expensive traversal and publication
  (`observation/panes.rs:248`, `worker/mod.rs:716`). Carry cancellation to the worker
  and test expiry before traversal, preserving ticket correlation.
- Workspace title/placement reconciliation includes quadratic membership scans
  (`workspace/tab_strip.rs:5`, `workspace/mod.rs:368`). Measure realistic pane
  counts before prioritizing this over the unconditional font scans.

## Baseline and tests

Native macOS, optimized Rust/Cargo 1.97.1, 20 samples per clean-shell scenario.
The shell is `/bin/bash --noprofile --norc -i`, directory `/`, and a controlled PS1.
No personal shell startup files were inspected or changed.

| Engine scenario | Ready p95 | Directory prompt in snapshot p95 |
| --- | ---: | ---: |
| No busy sessions | 2.20 ms | 4.76 ms |
| Four output-producing sessions | 4.31 ms | 7.80 ms |

The four background sessions were verified producing output and then held with
paused snapshot/event receivers. Their PTYs and parsing remained active, but
this is not the full GUI's continuously consuming/rendering workload. Timing
starts at session spawn, excluding configuration/font resolution, pane layout,
GPU work, and native presentation. Twenty warm process-local samples are not a
cold-application distribution. The 100 ms check used below is an illustrative
engine budget, not an agreed product target or a claim of imperceptible latency.

```sh
PATH="$HOME/.cargo/bin:$PATH" cargo +1.97.1 test -p sprite-term \
  --test prompt_latency --release --locked --offline

SPRITE_PROMPT_BUDGET_MS=100 PATH="$HOME/.cargo/bin:$PATH" \
  cargo +1.97.1 test -p sprite-term --test prompt_latency \
  --release --locked --offline -- --include-ignored --nocapture
```

The ordinary regression proves `Ready` plus a blank snapshot is not a prompt:
a controlled shell must receive permission before it emits the directory.
The ignored performance test uses a real interactive prompt and reports p95.
The two tests pass together. Each runs in a subprocess with an independent
watchdog covering startup and cleanup. HOME and history paths are controlled. A mutation returning `Ready` duration in place of
prompt duration failed the 250 ms control; the source was restored and tests
rerun. Timing assertions stay opt-in to avoid turning shared CI load into failures.
Focused Clippy, workspace formatting, and whitespace checks pass. No production
changes or commits were made during this review.

The actual visible-prompt target should be measured from launch/action dispatch
through font resolution, shell output, matching snapshot, and native presentation.
Use both a controlled shell and the user's normal shell. Sprite defaults to a
login shell (`shell.rs:70`); initialization and prompt hooks can add time before
any terminal can draw its real prompt. The controlled delay demonstrates this
boundary but does not diagnose the user's shell. Start with font-discovery reuse
and that end-to-end trace before changing shell semantics or introducing warm
spare shell processes.
