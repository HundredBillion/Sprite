# Checkpoint 4 performance baselines and budgets

**Status: Arch Linux only.** Under the Linux-first posture, macOS baselines join
the macOS acceptance milestone. See [checkpoint-1.md](checkpoint-1.md), whose
machine description still applies — same hardware, same toolchain, same pinned
Ghostty commit.

## Budgets

Twenty samples, release build, on an otherwise idle machine. Budgets are 110% of
this run's p95. Machine-readable copy: `checkpoint-4-arch.json`. Produced by
`sprite-graphics-bench`, which measures through the public `TerminalSession`
interface.

| Metric | Median (ms) | p95 (ms) | Budget (ms) |
|---|---:|---:|---:|
| `transmit_to_placement` | 3.294 | 3.569 | 3.926 |
| `transmit_to_placement_large` | 11.262 | 17.302 | 19.032 |
| `text_capture_without_images` | 0.143 | 0.900 | 0.990 |
| `text_capture_with_an_image` | 0.204 | 0.222 | 0.244 |

Checkpoints 1 to 3 budgets carry forward and still pass.

## What these measure

**`transmit_to_placement`** is the question an application cares about: from the
child printing an image to a placement a renderer could draw. A 32×32 image
takes about 3.3 ms and a 256×256 one — 256 KiB of pixels, 350 KiB of base64 —
about 11 ms. Most of that is moving the transmission through the PTY and the
parser rather than decoding.

**`text_capture_with_an_image` against `text_capture_without_images`** is the
regression Checkpoint 2 taught this project to fear: a capture that grows work
per cell. Showing an image costs an ordinary text capture about 0.06 ms more at
the median, which is the cost of walking the placements — proportional to how
many images are on screen, never to how many cells are.

The two numbers are directly comparable because both print a line and wait for
it before the measurement starts. **An earlier version of this benchmark did
not**, and reported that showing an image made text capture *faster* — the
no-image case was carrying the shell's start-up cost and the image case was not.
A benchmark that flatters a change is worse than none.

## Steady state under a transmit loop

Not a latency, so not budgeted: a program transmitting images forever must
settle rather than grow. Sixty 4 KiB images through a pane limited to 64 KiB:

| after 30 images | after 60 images |
|---:|---:|
| 65,536 bytes | 65,536 bytes |

Both readings are taken *past* saturation on purpose. An earlier version sampled
at ten images, before the pane had filled, and called the rise to the limit a
leak — growth towards a bound is the bound working. The harness now fails only
if the later reading exceeds the earlier one, and exits non-zero if it does.

## Reproducing

~~~bash
cargo build --release -p sprite-term --locked --offline
./target/release/sprite-graphics-bench --samples 20 \
  --output docs/performance/checkpoint-4-arch.json
~~~

Run on an idle machine. Checkpoint 2 recorded a case where a benchmark taken
while a background build was running reported a fix as a regression; the same
caution applies here.
