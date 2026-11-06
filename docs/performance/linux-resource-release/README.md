# Linux resource release measurements

Measured on 2026-11-05 on the physical Omarchy laptop for issue #141.
Baseline: `41ecab3` (v0.6.0). Both release builds use Rust 1.98.1, Zig 0.17.0,
the locked dependencies, and the same temporary instrumentation.
The final GPUI patch does not contain that instrumentation.

## Scope and method

The desktop is Hyprland 0.56.2 on Linux 7.2.15, x86-64. Blade used the Intel
Tiger Lake i915 GPU at PCI `0000:00:02.0`; the laptop also has an NVIDIA GPU,
which was not used for these measurements. The X11 run uses Xwayland in this
Wayland session. A separate native Xorg session was not tested.

`measure.py` launches a separate Sprite Terminal window with a steady cursor
and a controlled child. It addresses only that window's panes, leaving existing
windows alone. It uses Hyprland's Lua dispatcher to hide its own window on a
special workspace, then restore it; it changes no desktop configuration.

Each idle sample waits 2.5 seconds, then reads process CPU ticks across 15 seconds.
CPU is a percentage of one core, with a 10 ms clock-tick resolution, so a zero
means no CPU tick was charged in that interval. The same instrumented builds
write a timestamp per delivered GPUI callback and after Blade submits each rendered frame.
This adds timestamp and stderr-write overhead, so the CPU percentages include
measurement overhead. Callback counts independently distinguish periodic wakeups
from useful drawing. The laptop was not otherwise isolated from concurrent work.

Eight wake trials each wait 2.5 seconds, then measure from sending a request to
the controlled child until Blade has submitted the first subsequent frame.
This includes IPC, scheduling, terminal output processing and rendering. It is
not a GPU-completion or display-photon measurement or a statistically strong latency bound.

The image workload displays a persistent 512×512 Kitty image, transmits 60 successive versions of a second
512×512 RGBA Kitty image, then deletes the second and finally all images.
The two images occupy separate screen rows. GPU counters come from
`/proc/PID/fdinfo`, deduplicated by DRM client ID. They describe driver allocations,
not individual atlas tiles. Moving between workspaces can temporarily resize the
window and change framebuffer allocations; image comparisons begin after it has
returned to its original workspace and settled.

The pane workload opens four extra panes running `sleep`, waits, then closes
them and waits for cleanup. The final runs also repeat this four-pane cycle five
more times to distinguish a persistent RSS high-water mark from continued growth.
RSS includes allocator caches; it is not a count of live Rust allocations.

## Reproduce

Apply `instrumentation.patch` to either the baseline or fixed GPUI tree, build
with `cargo build --release --locked --offline -p sprite-app --bin sprite`, and
copy the binary somewhere it will not be overwritten by the other build.
The instrumentation only touches the shared frame callback and Blade renderer;
it applies to both versions. Restore those two files before running source
integrity checks or committing. Keep both backend runs sequential so their
windows do not resize each other.

```sh
python3 docs/performance/linux-resource-release/measure.py \
  /tmp/sprite-before wayland /tmp/before-wayland.json
python3 docs/performance/linux-resource-release/measure.py \
  /tmp/sprite-before x11 /tmp/before-x11.json
```

Repeat with the fixed binary. The baseline commands finish collecting their
measurements, then fail the idle-callback assertion. The fixed commands must
pass it. The harness refuses an uninstrumented binary so a missing trace cannot
masquerade as zero callbacks. JSON files retain individual measurements; frame
trace logs are local, ignored artifacts.

## Allocator regression and dependency evidence

`python3 -m unittest scripts.test_gpui_patch -v` checks the exact patched GPUI
source and compiles its actual `BladeAtlas::remove` method against etagere.
The regression failed on the baseline with "a removed image's space must be
reusable" and passes with the allocator fix. It exercises both texture kinds,
a surviving allocation, 60 replacements, repeated deletion and final texture
release. GPU destruction is represented by a counter in this portable harness;
the desktop measurements exercise the actual driver.

The manifest declares etagere `0.2`; `Cargo.lock` resolves 0.2.15. The
[versioned allocator documentation](https://docs.rs/etagere/0.2.15/etagere/struct.BucketedAtlasAllocator.html)
requires `deallocate` and explains that space is reclaimed when the containing
bucket becomes empty. The existing Metal removal path is the local precedent.
The patch keeps that policy and returns space within a still-live texture.

The Wayland source trace tests the production pause/resume methods with recorded
surface requests: pending callback drain, resize-throttle release, coalesced
wakes and waking with a callback already pending. Removing the pause guard makes
the same trace fail. The live runs additionally exercise the real compositor,
foreground-executor wakers and X11 calloop timer removal/restart.

## Logging

A shell launch with `SPRITE_LOG=sprite_app::startup=debug` and a desktop entry
whose `Exec` begins `env SPRITE_LOG=sprite_app::startup=debug` both produced the
`window activated` debug event. The temporary desktop entry passed
`desktop-file-validate` and was launched through `gio launch`, with no inherited
`SPRITE_LOG`. Each run used a private temporary `XDG_STATE_HOME`; log files were
mode 0600. `docs/local-logging.md` now documents an explicit desktop-entry value
because an environment assignment in an unrelated shell cannot configure an
already-running desktop launcher.

## Desktop results

All idle samples are 15 seconds. The number after the slash is the largest of
eight wake trials, not a p95 estimate.

| Build | Backend | Visible CPU | Visible callbacks | Hidden CPU | Hidden callbacks | Wake median / max |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| before | wayland | 0.533% | 871 | 0.000% | 0 | 5.17 / 24.07 ms |
| before | x11 | 0.400% | 897 | 0.333% | 900 | 16.82 / 19.36 ms |
| after | wayland | 0.000% | 0 | 0.000% | 0 | 5.62 / 21.39 ms |
| after | x11 | 0.067% | 0 | 0.000% | 0 | 1.15 / 2.97 ms |

Both baseline commands fail the idle assertion; both fixed commands pass.
Wayland's compositor already stops delivering callbacks to the hidden surface.
On this Xwayland session, moving the window offscreen did not stop the original
X11 refresh timer. The fixed X11 idle pause stops it in both states. A zero CPU
sample means below the tick resolution, not literally no work.

For all four runs, GPU `drm-total-system0` was **249,100 KiB (243.26 MiB)** before
the image workload, after the 60 transmissions, after deleting the changing
image, and after deleting all images. The fixed run stays at its starting value,
but the same driver counter also stays flat on the baseline. Consequently these
driver measurements do **not** demonstrate an additional reduction in GPU
residency from this patch. The production-method allocator regression is the
evidence for the unreclaimed tile space. Native screenshots were inspected to
confirm both pictures were actually displayed; they remain local artifacts.

### Panes

| Fixed backend | Threads: before / +4 panes / closed | FDs: before / +4 panes / closed | RSS KiB: before / +4 panes / closed / 5 more cycles |
| --- | --- | --- | --- |
| wayland | 28 / 40 / 28 | 72 / 100 / 72 | 97,556 / 100,384 / 100,324 / 107,636 |
| x11 | 31 / 43 / 31 | 72 / 100 / 72 | 96,064 / 98,260 / 98,064 / 98,740 |

Each pane adds three threads and seven descriptors; those return completely.
RSS does not fully return after closing panes. These process-level measurements
alone cannot establish whether the retained pages are live objects or allocator
caches, and this PR does not claim to eliminate the RSS high-water mark.

### Allocator check after repeated pane closure

`memory.py` repeats the four-pane open/close workload ten times in a fresh fixed
Wayland process. GDB reads glibc `malloc_stats()` before and after each cycle,
then calls `malloc_trim(0)` once as a diagnostic. Only this test process permits
its parent (and the parent's GDB child) to trace it, using `PR_SET_PTRACER` from
`/usr/include/linux/prctl.h`; system ptrace policy is unchanged.

```sh
python3 docs/performance/linux-resource-release/memory.py /tmp/sprite-after
```

| Point | glibc system bytes | glibc in-use bytes |
| --- | ---: | ---: |
| Before panes | 15,948,432 | 12,798,384 |
| After first cycle | 18,664,080 | 13,241,648 |
| After tenth cycle | 26,696,336 | 13,352,224 |

After forty extra panes have closed, glibc's in-use total is only
553,840 bytes above the initial value, while its system allocation has grown
by 10,747,904 bytes. Most of the additional heap capacity is therefore free
space retained by the allocator. RSS is 88,148 KiB initially,
100,200 KiB before trimming and 95,916 KiB afterwards.
The small in-use increase is not proof that every pane allocation was released;
these samples do not establish a long-run bound. No production trimming or
additional pane-memory policy is introduced by this patch.

## Verification notes

The initial full baseline suite had one five-second selection watchdog timeout
while the release build was running. The focused selection rerun passed all
23 tests with `SPRITE_TEST_WATCHDOG_SECS=20`, the value already used by macOS CI
for scheduling delays. The final full workspace run uses the same value.
The unchanged GPUI taffy source emits two future-compatibility float warnings,
and Cargo reports the existing `proc-macro-error2` future-compatibility warning.

Independent review checked the atlas removal path, actual Wayland configure and
callback ordering, X11 native event paths and calloop timer removal, deferred
weak wakers, and the portable regressions. It found no important issues.
Native macOS execution and a separate native Xorg session were not performed.

Final local checks passed: workspace formatting; rustfmt of the touched GPUI and
Rust fixture files; exact GPUI archive/patch verification; all six Python patch
regressions; `cargo clippy --workspace --all-targets --locked --offline -- -D warnings`;
`SPRITE_TEST_WATCHDOG_SECS=20 cargo test --workspace --locked --offline --no-fail-fast`;
`cargo build --workspace --locked --offline`; findings-ledger validation; and
`git diff --check`. Both instrumented release builds also completed successfully.
