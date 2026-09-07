# Checkpoint 4 security review request

**Status: PERFORMED, 2026-09-05.** The review this document prepared is
[checkpoint-4-security-review.md](checkpoint-4-security-review.md), which
answers every question below. This file is kept as the brief it was.

Graphics turn "a program printed something" into "the terminal decoded an
untrusted binary format and put it on a GPU". The TSP requires a security review
covering image-source denial, payload bounds, memory limits, and the observation
exclusions. Each is set out below with what was built and where to push hardest.

Still owed and **not** covered here: the general review of Checkpoints 1 and 2,
the general review of Checkpoint 3, the Checkpoint 3 security review, and this
checkpoint's general review. That is five reviews outstanding.

## 1. Image-source denial

**What exists.** Kitty can be told to load an image from a named file, a
temporary file, or shared memory. All three are denied, which is what stops a
printed escape sequence becoming a file read. The file and shared-memory
mediums are refused explicitly; the temporary-file medium **cannot** be, because
the binding's setter for it takes a `bool` while the option expects a string and
the Zig side aborts the process when called. It is denied by Ghostty's default
limits (`.direct`), and all three are asserted by *behaviour* — a transmission
on each medium must leave no image.

**Push hardest on.**

- Is behaviour-asserted denial enough for the temporary-file medium, given it
  rests on an upstream default that a future version could change? The tests
  would catch it, but only when someone runs them.
- The file-medium test proves the path was never consulted by showing that a
  readable file and a missing one produce identical outcomes. Is that the right
  proof, or is there a way to observe an `open` directly?
- Sprite passes the payload of a `t=f` transmission to libghostty as-is. Is
  there any path by which that payload — an attacker-chosen string — reaches a
  filesystem call before the medium is refused?

## 2. Payload bounds

**What exists.** `apc_max_bytes` caps what one unterminated sequence may
accumulate; `storage_bytes` caps decoded image bytes. Sprite's own PNG decoder
checks the declared output size against the storage limit **before** allocating,
because a PNG's declared size is chosen by whoever printed it. Every decode
failure is a `None`; nothing panics, because this runs on the thread that owns
the terminal.

**Push hardest on.**

- **A known wart.** When a transmission exceeds either bound, the pinned Ghostty
  abandons the sequence and prints the remainder as text, so a refused image
  sprays thousands of characters of base64 onto the screen. It is a display
  problem rather than a memory one, but is it worse than that — can a program
  use it to force text it controls into a pane's scrollback, which observation
  then reports as terminal output?
- The decoder expands paletted and grayscale PNGs and strips 16-bit channels.
  Are there PNG shapes where `output_buffer_size` under-reports what
  `next_frame` writes?
- Chunked transmissions accumulate until `m=0`. The bound is `apc_max_bytes`.
  Is there any other accumulation — placement lists, image ids — that a program
  could grow without bound?

## 3. Memory limits

**What exists.** Two independent limits: `graphics.storage_bytes` for what the
terminal holds and `graphics.texture_bytes` for what the renderer holds. The
renderer refuses an image larger than its whole budget rather than emptying the
cache for something that still would not fit, and evicts least-recently-placed
otherwise. A pane that has shown a thousand images holds textures for the few on
screen. A transmit-forever loop settles at the limit.

**Push hardest on.**

- The limits are per pane. A window of sixteen panes at the defaults is 1 GiB of
  terminal-side storage and 2 GiB of textures. Should there be a per-window
  ceiling as well?
- Eviction is by least-recently-placed within one pane. Can a program make
  another pane's images be evicted? (It should not be able to: the caches are
  per pane and per session.)
- `GraphicsCache::retain` is driven by each arriving snapshot. If snapshots stop
  arriving — a wedged pane — textures are held indefinitely. Is that acceptable?

## 4. Observation exclusions

**What exists.** A placement is reported as metadata: identity, transmission
format, pixel size, cells covered, z-order, visibility. `PlacementMetadata` has
no field that could hold bytes, pixels, or a filename, and the path that fills it
never calls `Image::data`. A test transmits an image of a recognisable byte and
searches the finished JSON for the transmission's base64, the pixels as decimal,
as hex and as escapes, and the fixture's filename.

**Push hardest on.**

- Pixel dimensions and cell coverage are reported. Is *any* property of an image
  a leak — could a program that cannot see a pane infer something from the size
  of an image in it?
- The `transmission_format` field says how an image arrived. Harmless, or does
  it narrow down what produced it?
- The exclusion tests search for the byte patterns a leak would most likely
  take. Is there an encoding they would miss — a length, a checksum, a
  base64 variant?

## How to exercise it

~~~bash
cd phase_1
cargo test --workspace --locked --offline

# An image, on screen, in a real window:
./target/release/sprite
# then, inside a pane, print any Kitty graphics payload.
~~~

Turning images off entirely, and the two limits:

~~~toml
# $XDG_CONFIG_HOME/sprite/config.toml
[graphics]
enabled = false
storage_bytes = 67108864
texture_bytes = 134217728
~~~
