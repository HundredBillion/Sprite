# Checkpoint 4 security review

**Status: PERFORMED, 2026-09-05.** This is the review
[checkpoint-4-security-review-request.md](checkpoint-4-security-review-request.md)
asked for. It answers that document's four areas and every "push hardest on"
question in them, against the code as it stands on `phase_1` at `5806e45`.

**Scope and method.** A read of `sprite-term/src/png_decoder.rs`,
`graphics.rs`, `apply_graphics_policy` in `worker.rs`, `GraphicsPolicy` in
`lib.rs`, and `sprite-app/src/graphics_cache.rs`, with the suite run locally
(green). Reviewed by Claude Opus 5 at the project owner's direction. **A code
review, not a penetration test:** no malformed PNGs were fired at the decoder,
and no fuzzing was run. Given that this is the one place Sprite parses an
untrusted binary format, that limit matters more here than in the Checkpoint 3
review, and it is restated at the end.

**Verdict: the denials hold and the decoder is bounded on both sides of the
allocation.** One finding is recorded: an unbounded-in-practice scratch buffer
that makes the real per-pane memory ceiling about twice what the configuration
implies.

---

## Findings

### F1 — The decoder's scratch buffer is retained at peak size, for the life of the pane

`PngDecoder::buffer` is a `Vec<u8>` reused between images (`png_decoder.rs:28`),
grown by `resize(needed, 0)` on every decode and **never shrunk**. Reuse is
deliberate and the comment says so — a pane showing many images should not
reallocate for each. The consequence it does not mention is that one large image
raises the pane's floor permanently: decode a 60 MiB PNG once and that pane
holds 60 MiB of scratch until it closes, whether or not the image was kept, and
whether or not anything is on screen.

`needed` is bounded by `self.limit`, which is `storage_bytes`. So the honest
per-pane ceiling at the defaults is:

| | |
|---|---:|
| `graphics.storage_bytes` (libghostty's decoded images) | 64 MiB |
| decoder scratch, retained at peak | up to 64 MiB |
| `graphics.texture_bytes` (`GraphicsCache`) | 128 MiB |
| **per pane** | **up to 256 MiB** |

which makes the sixteen-pane window in the request document's §3 up to 4 GiB
rather than the 3 GiB it estimates. Reaching it needs the user's own programs to
transmit large images in every pane, so this is a ceiling, not a leak — but the
configuration does not currently let anyone see it, because the scratch is
governed by a limit named for something else.

**Recommended:** shrink the buffer after each decode above some retained size,
or `shrink_to` a fixed reuse budget — a few MiB keeps the reuse benefit for the
images anyone actually transmits. Whatever is chosen, say the scratch exists in
the `graphics.storage_bytes` documentation, because today it is a second
allocation of the same size with no name.

---

## 1. Image-source denial

**Is behaviour-asserted denial enough for the temporary-file medium?** Yes,
because the alternative is worse and there is no third option. I verified the
reason: `set_kitty_image_from_temp_file_allowed` takes a `bool` while the option
it writes expects a string, so the Zig side `@alignCast`s a one-byte pointer to
an eight-byte-aligned type and **aborts the process**. That is not a refusal
Sprite can catch — it is not an error return, it is the end of the program.
Calling it defensively would trade a denial that currently holds for a crash
that certainly does.

So the denial rests on Ghostty's default limits being `.direct`, asserted by
`tests/graphics_policy.rs` sending a transmission on each medium and requiring
that no image appears. The exposure is real and correctly stated: a future
libghostty that changed the default would open a path from terminal output to
the filesystem, and the tests would catch it only when someone runs them. Since
the submodule is pinned to an exact commit and CI runs the suite on every
change, "when someone runs them" is in practice "on the pull request that bumps
the pin", which is the right moment.

**Recommended:** name the pinned commit in the failure message of those three
tests. When one fails after a bump, the cause should be legible without reading
this document.

**Is the file-medium proof the right one?** Yes. Showing that a readable file
and a missing one produce identical outcomes proves the path was never
consulted, and it proves it without depending on how a failure is reported.
Observing the `open` directly would be stronger — `strace`, or an `LD_PRELOAD`
shim — but it would be a platform-specific test for a property the equivalence
already establishes. I would not add it.

**Can a `t=f` payload reach a filesystem call before the medium is refused?**
Not on the path I can see. Sprite passes the payload to libghostty as-is and
never interprets it; the medium check is Ghostty's, and it happens before the
payload is treated as a path. This is the one answer in this review that rests
on upstream code I did not read, and it is worth saying plainly: the guarantee
is Ghostty's, and Sprite's contribution is not to add a second path of its own.
It does not.

## 2. Payload bounds

**Are there PNG shapes where `output_buffer_size` under-reports what
`next_frame` writes? Not reachable ones, and the code is belt-and-braces.**
Three things stack:

- `needed = reader.output_buffer_size()?` is checked against the limit **before**
  `resize`, so an attacker-declared size cannot cause the allocation.
- `next_frame(&mut self.buffer)` receives a slice whose length is `needed`. It
  cannot write past it; a decoder that wanted to would get a bounds check.
- After the decode, `produced = info.buffer_size()` is re-checked against *both*
  the limit and `self.buffer.len()` (`png_decoder.rs:70`), so an under-reporting
  `output_buffer_size` is caught rather than trusted.

That third check is the one that answers the question, and it is easy to leave
out. The `ALPHA | STRIP_16` transformations are also applied before
`output_buffer_size` is asked, so the size accounts for the expansion rather
than the on-disk form — which is the failure mode I went looking for.

Every failure is `None`, and I found no `unwrap`, no indexing, and no
arithmetic that could panic on this path. That matters more than usual: it runs
on the thread that owns the terminal, and a panic there takes the pane's session
with it.

**Is there any other unbounded accumulation?** Chunked transmissions are bounded
by `apc_max_bytes`. Decoded images are bounded by `storage_bytes`. Textures are
bounded by `texture_bytes`. The scratch buffer is F1. I did not find an
unbounded placement list or image-id map on Sprite's side — placements are read
from the terminal per snapshot rather than accumulated by Sprite — but the
storage of ids and placements inside libghostty is upstream's, bounded by its
own storage limit, and I did not review it.

**The base64 spray on refusal: a display wart, not the escalation the request
document suspected.** When a transmission exceeds a bound the pinned Ghostty
abandons the sequence and prints the remainder as text, which lands in
scrollback and is then faithfully reported by `panes snapshot`. So yes, a
program can force text it controls into observable output — but it could
already do that by printing the text, which is what a terminal is for. It gains
nothing: the text lands in **its own** pane, and a program has always been able
to write anything to its own pane. No pane boundary is crossed, and observation
reports it as exactly what it is. It remains an ugly display failure worth
fixing for its own sake.

## 3. Memory limits

**Should there be a per-window ceiling? Eventually, and not in Phase 1.** The
per-pane limits are correct as the enforcement point — the caches are per pane
and per session, and that is what keeps panes from affecting each other — but
they compose by multiplication, and F1 makes the multiplier bigger than it
looks. A window-level ceiling is a real feature (it needs a policy for which
pane gives up memory, which is a product decision, not a bounds check), and
Phase 1 is not where it belongs. Record it as known.

**Can a program make another pane's images be evicted? No.** `GraphicsCache` is
owned per pane and `GraphicsPolicy` is applied per worker thread, one per pane.
The PNG decoder is installed in thread-local storage on the pane's own worker,
so even the decoder is not shared. Eviction is least-recently-placed *within one
cache*. I looked for a shared allocator or a process-wide budget that would
couple two panes and did not find one.

**A wedged pane holds its textures indefinitely — acceptable.** `retain` is
driven by arriving snapshots, so a pane that stops producing them keeps whatever
it last had. The hold is bounded by `texture_bytes`, so the worst case is the
budget rather than growth, and the alternative — evicting the textures of a pane
that is merely slow — would make a stalled pane repaint wrongly the moment it
recovered. Holding is the better failure.

`an_image_larger_than_the_whole_budget_is_refused_rather_than_emptying_it` is
the right rule and is tested: refusing beats evicting everything for something
that still would not fit.

## 4. Observation exclusions

**Is any property of an image a leak?** Not across a boundary that matters. Any
pane in a window may already read any other pane's *rows*, so learning that a
neighbour is displaying a 256×256 image is strictly less than what the caller is
already permitted. The question would matter under a per-pane boundary; ADR 0013
does not draw one. Within the model, pixel size and cell coverage are safe.

**Does `transmission_format` narrow down what produced the image?** It says how
the image arrived, not what made it, and the only surviving medium is direct
transmission — so it carries close to one bit. Harmless.

**Is there an encoding the exclusion tests would miss?** Possibly, and the
construction is what makes it not matter. The tests search the finished JSON for
the transmission's base64, the pixels as decimal, hex and escapes, and the
fixture's filename — a good list, but a search-based test can only find
encodings someone thought of. The actual guarantee is structural:
`PlacementMetadata` has no field that could hold bytes, pixels, or a filename,
and the path that fills it never calls `Image::data`. The type is the defence;
the searches are a regression net over it. That is the right way round, and it
is why I am not proposing more encodings to search for.

---

## What this review did not do

- **No fuzzing of the PNG decoder**, which is the gap I would close first if
  this code is ever exposed to anything beyond a local child. `cargo-fuzz`
  against `decode_png` with a corpus of malformed PNGs is a day of work and
  would cover the shapes a read cannot.
- No review of libghostty's own image storage, id handling, or medium
  enforcement — several answers above rest on upstream behaviour that is pinned
  and behaviourally asserted, but not read.
- The general reviews of Checkpoints 1, 2, 3 and 4 remain owed. This covers the
  security review only.
