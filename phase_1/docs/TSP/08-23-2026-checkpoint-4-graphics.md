# Sprite Terminal Checkpoint 4 Technical Spec

> **Status: DRAFT — not reviewed, not started.** Checkpoint 3 is implemented but
> unaccepted: three reviews are owed, including the security review of the
> observation surface. This document plans the next checkpoint. The project
> owner has chosen to continue building and to take the outstanding reviews at
> the end; that debt is carried forward knowingly rather than forgotten.

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> dmi-superpowers:subagent-driven-development (recommended) or
> dmi-superpowers:executing-plans to implement this TSP task-by-task. Steps use
> checkbox (- [ ]) syntax for tracking.

**Goal:** Show images. Extend the existing owned-snapshot contract with Kitty
graphics — transmitted, decoded, placed, layered, scrolled, and deleted in step
with the text — and validate Croft as an unmodified external application that
uses them.

**Architecture:** Still two crates, still one worker thread per pane. Graphics
follow the same rule as every other terminal value: libghostty owns them, the
worker copies what it needs into owned data before the next mutation, and no
handle crosses a thread. `sprite-app` gains a GPU texture cache; `sprite-term`
gains an owned graphics projection.

**Tech Stack:** Unchanged, plus two feature flags on the existing
`libghostty-vt` dependency — `kitty-graphics` and `png` — which bring in that
crate's own `RustPngDecoder`. No new direct dependency is expected. A
`DEPENDENCIES.md` update is owed for the changed feature set.

## Global Constraints

Checkpoints 1 through 3 constraints carry forward. Additionally:

- **Image data is untrusted input from an arbitrary program.** A payload arrives
  as escape-sequence bytes; a decoder turns it into pixels. Every limit below
  exists because a program that can print can reach this code.
- **Never widen the render bundle for panes with no images.** Checkpoint 2
  measured what happens when capture grows: an FFI call per cell cost most of a
  30% latency regression. A pane showing no images must pay nothing.
- **Textures are cached by identity and generation, never by dimensions or
  placement.** Two different images of the same size are not the same image, and
  one image whose content changed is not the old one.
- **Degrade, never terminate.** Exceeding a memory limit spoils one image and
  says so. It does not kill the pane, the window, or the session.
- **Linux first**, as the PRD's platform posture records.

## Checkpoint boundary

Checkpoint 4 includes: enabling and bounding Ghostty's Kitty image storage; PNG
and raw transfer; the owned graphics projection; the GPU texture cache and its
eviction; placement geometry including source rectangles, scaling, clipping, and
the three z-layer classes; independent terminal-side and GPU-side memory limits;
placement metadata in the observation schema; tmux passthrough; and the Croft
graphics validation.

Checkpoint 4 does **not** include packaging (Checkpoint 5), configuration beyond
the graphics limits it needs, macOS acceptance, or accessibility.

## Carried forward

Open at the end of Checkpoint 3 and still open:

1. Human review of Checkpoints 1 and 2.
2. General review of Checkpoint 3.
3. Security review of the observation surface.
4. Shell-integration auto-loading, deliberately not attempted.
5. Drag-to-select verified by machine; no mouse-injection tool is available here.
6. TSP 3 open question 3: a pane under sustained output may always miss the
   observation deadline, making a loud pane permanently unobservable.

## What the pinned libghostty gives us

Verified against `libghostty-vt =0.2.1` before writing this plan, because a
checkpoint built on an assumed API is a checkpoint that stops halfway:

- `Terminal::kitty_graphics()` → `Graphics`, with `image(id)` and `generation()`.
- `Image`: `id`, `number`, `width`, `height`, `generation`, `format`,
  `compression`, `data`.
- `ImageFormat`: `Rgb`, `Rgba`, `Png`, `GrayAlpha`, `Gray`.
  `Compression`: `None`, `ZlibDeflate`.
- `PlacementIterator` with `update`/`next`, and per placement: `set_layer`,
  `pixel_size`, `grid_size`, `viewport_pos`, `source_rect`, `rect`,
  `placement_render_info`, `image_id`, `placement_id`, `is_virtual`,
  `x_offset`, `y_offset`.
- `Layer`: `All`, `BelowBg`, `BelowText`, `AboveText` — the three z-layer
  classes the PRD names, already classified for us.
- `set_png_decoder` taking a `DecodePng`, and a ready-made `RustPngDecoder`
  behind the `png` feature.
- Storage controls: `set_kitty_image_storage_limit`,
  `set_kitty_image_from_file_allowed`, `set_kitty_image_from_temp_file_allowed`,
  `set_kitty_image_from_shared_mem_allowed`, `set_apc_max_bytes_kitty`.

**The dependency is currently `default-features = false`, which means Kitty
graphics are compiled out today.** Task 1 turns them on deliberately rather than
by accident.

## What GPUI 0.2.2 gives us, and one trap

`RenderImage::new(frames)` plus `img(ImageSource::Render(...))` renders raw
pixels, so no image file ever has to touch the disk.

Two facts shape the design:

- **GPUI wants BGRA; libghostty produces RGBA.** A conversion happens somewhere.
  It must happen once per image generation, not once per frame.
- **`RenderImage::new` mints a fresh `ImageId` every call.** GPUI's own identity
  is therefore useless for caching. Sprite keeps its own map from
  (image id, generation) to `Arc<RenderImage>`, which is what the PRD's "stable
  image identity plus content generation" means in practice.

---

### Task 1: Enable and bound Kitty graphics

**Files:** `Cargo.toml`, `sprite-term/src/worker.rs`, `sprite-term/src/lib.rs`,
`DEPENDENCIES.md`

**Threat model:** every byte here came from a program that can print. Ghostty's
image storage can be told to read images from paths, temporary files, and shared
memory. Those turn "a program printed something" into "the terminal read a file
you did not name", which is a capability no image protocol needs in Phase 1.

- [ ] Enable the `kitty-graphics` and `png` features on `libghostty-vt` and
  record the changed feature set in `DEPENDENCIES.md`. Confirm the lock file
  change and report it: no new crate should appear that is not already there.
- [ ] **Deny `from_file`, `from_temp_file`, and `from_shared_mem` explicitly**,
  rather than relying on a default. Test each: a transmission naming a path Sprite
  can read must be refused, and the file must not be opened.
- [ ] Set an explicit terminal-side storage limit, configurable, with a
  documented default. Test that the limit is what is enforced, not the default.
- [ ] Bound APC payload bytes so a single unterminated sequence cannot grow
  without limit. Test with a payload larger than the bound.
- [ ] A pane with graphics disabled by configuration behaves exactly as today:
  no storage, no decoder, and transmitted images ignored rather than buffered.

### Task 2: PNG and raw transfer

**Files:** `sprite-term/src/graphics.rs` (new), `sprite-term/src/worker.rs`

**Threat model:** a decoder is a parser for hostile input. The one here is
`libghostty-vt`'s own, which uses the `png` crate; the risk owned locally is
what Sprite does with the result.

- [ ] Install the PNG decoder through the binding's supported callback, on the
  worker thread that owns the terminal. Document why it cannot be installed
  once globally: the binding requires the terminal's own thread.
- [ ] Accept raw `Rgb`, `Rgba`, `Gray`, and `GrayAlpha` transfers as well as
  `Png`, and `ZlibDeflate` compression. Fixtures for each.
- [ ] Chunked transmission reassembles correctly; a chunk sequence that never
  completes is discarded rather than retained.
- [ ] **Malformed payloads are refused without a panic and without terminating
  the pane**: truncated PNG, wrong declared dimensions, zero-size, and a
  declared size that disagrees with the data. Each produces a diagnostic.
- [ ] Decoding cost is bounded by the storage limit, so a decode cannot exceed
  what transmission was already allowed to hold.

### Task 3: The owned graphics projection

**Files:** `sprite-term/src/graphics.rs`, `sprite-term/src/snapshot.rs`,
`sprite-term/src/lib.rs`

- [ ] The worker copies decoded pixels and placement metadata into owned values
  **before the next terminal mutation**, exactly as the render projection does.
  No borrowed handle survives the call.
- [ ] Graphics ride alongside the existing bundle without widening it: a pane
  with no images allocates nothing new and adds no FFI calls per cell. Measure
  this, do not assume it — Checkpoint 2's regression came from exactly this
  shape of change.
- [ ] Pixels are copied **once per image generation**, not once per capture. A
  still image on screen through a thousand frames is copied once.
- [ ] The projection carries: image id, generation, pixel dimensions, format
  after decoding, and for each placement its id, virtual flag, source rectangle,
  cell and pixel geometry, viewport position, and z-layer class.
- [ ] Test that a snapshot's graphics and its rows come from one generation, so
  an image is never drawn against text it never accompanied.

### Task 4: Identity, generation, and invalidation

**Files:** `sprite-term/src/graphics.rs`, `sprite-term/src/worker.rs`

- [ ] Identity is (image id, generation). Test that replacing an image's content
  under the same id produces a new generation, and that a different image with
  identical dimensions is never mistaken for it.
- [ ] Deletion, screen switch, `reset`, storage eviction, and session close each
  make the affected images disappear from the projection deterministically —
  each with its own test, not one test for all five.
- [ ] A placement that scrolls out of the viewport stops being placed; the image
  it referred to is not necessarily gone. Test both halves.
- [ ] Test the alternate screen: images on the normal screen are not visible
  while an alternate-screen program runs, and return afterwards.

### Task 5: The GPU texture cache

**Files:** `sprite-app/src/graphics_cache.rs` (new),
`sprite-app/src/terminal_view.rs`

- [ ] Cache `Arc<RenderImage>` by (image id, generation). A generation change
  replaces the entry; the old texture is released.
- [ ] Convert RGBA to the BGRA GPUI expects **once per generation**, at the point
  of caching. Test the channel order against a known pixel, because a silent
  red/blue swap is the kind of defect that survives review.
- [ ] Entries for images no longer in any snapshot are evicted. Test that a
  long-running pane that shows a thousand images does not hold a thousand
  textures.
- [ ] Closing a pane releases its textures; closing a tab releases every pane's.
- [ ] A GPU-side memory ceiling, independent of the terminal-side limit, with
  eviction by least-recently-placed.

### Task 6: Placement rendering

**Files:** `sprite-app/src/terminal_view.rs`, `sprite-app/src/grid.rs`

- [ ] Draw each placement at its cell geometry, honouring source rectangle,
  scale and aspect behaviour, and clipping at the pane's edges.
- [ ] The three z-layer classes render in order: below background, below text,
  above text. Test that text over an image is legible and an above-text image
  covers it.
- [ ] Virtual placements are filtered out of ordinary rendering.
- [ ] Images move with the text when the pane scrolls, and are clipped rather
  than stretched at the viewport edge.
- [ ] Resizing re-places without re-decoding: a resize changes geometry, not
  generation. Assert no re-decode happens.

### Task 7: Memory limits and degradation

**Files:** `sprite-term/src/graphics.rs`, `sprite-app/src/graphics_cache.rs`

- [ ] Terminal-side and GPU-side limits are independent and separately
  configurable, as the PRD requires.
- [ ] Exceeding either limit degrades **that image** with a diagnostic naming it,
  and leaves the pane, its text, and every other image working. Test at both
  limits.
- [ ] A single image larger than the whole limit is refused at transmission
  rather than accepted and then evicting everything else.
- [ ] Test the pathological case deliberately: a program that transmits images in
  a loop forever must reach a steady state, not unbounded growth.

### Task 8: Placement metadata for observation

**Files:** `sprite-app/src/observation/schema.rs`, `sprite-term/src/snapshot.rs`

**Threat model:** the observation surface's exclusion list explicitly bans image
bytes, decoded pixels, and filenames. Graphics are the first feature that could
violate it by accident.

- [ ] For placements intersecting the returned screen and history range, include
  stable placement identity, transmission format, pixel dimensions, cell bounds,
  and z-order — enough to know an image occupies terminal space.
- [ ] **Prove the JSON never contains transmitted bytes, decoded pixels, source
  filenames, or inferred content.** Extend the existing key-set assertion so a
  field added to the graphics projection cannot reach the wire unnoticed, and
  add a test that transmits a recognisable image and asserts its bytes appear
  nowhere in the response.
- [ ] Decide and record whether adding `placements` bumps `schema_version`. It is
  additive, so probably not — but a client pinning the version deserves an
  explicit answer rather than an inference.

### Task 9: tmux passthrough

**Files:** `docs/`, `sprite-term/tests/`

- [ ] Graphics through the current stable tmux work when tmux's documented
  passthrough option is enabled. Sprite documents the setting.
- [ ] **Sprite does not patch, override, or detect-and-work-around tmux.** If
  passthrough is off, images do not appear, and that is tmux's documented
  behaviour rather than a Sprite defect.
- [ ] The test is skipped with a clear message when tmux is absent, in the same
  shape as the Croft gate, rather than failing.

### Task 10: Fixtures, budgets, Croft, and review

- [ ] Graphics fixtures covering: PNG and raw transfer, chunking, replacement,
  crop, scale, placement IDs, negative and positive z, scrolling, screen
  switches, deletion, generation changes, storage limits, malformed payloads,
  and texture reclamation. Compare owned snapshots, never GPU internals.
- [ ] Budgets: decode-to-placement latency, snapshot cost for a pane with images
  against one without, and steady-state memory under a transmit loop.
- [ ] Croft runs unmodified and its graphics behaviour is validated, extending
  the existing capability gate rather than replacing it.
- [ ] Re-run forbidden-state and provenance gates; the whole locked offline gate
  passes.
- [ ] **Security review** of image-source denial, payload bounds, memory limits,
  and the observation exclusions.

---

## Open questions for review

1. **Where do graphics ride?** Alongside the render bundle, or on their own
   stream? A separate stream keeps text latency independent of a large decode,
   but two streams can disagree about which generation is current, and
   Checkpoint 1 chose one coherent generation deliberately.
2. **Can the renderer be tested at all?** The PRD asks for renderer tests in "an
   offscreen or controlled GPUI harness". No such harness exists, and GPUI 0.2.2
   may not offer one. If it does not, image rendering is verified by hand and by
   owned-snapshot tests only — and this checkpoint should say so plainly rather
   than implying pixel coverage it does not have.
3. **What is the default storage limit?** Ghostty's own default is a starting
   point, but Sprite's panes are many per window, and the limit is per terminal.
   A window of sixteen panes at Ghostty's default may be a lot of memory.
4. **Does an image belong in scrollback at all?** Placements scroll with text, so
   history extraction may return rows whose images are long evicted. An observer
   should probably see "an image was here" rather than nothing, but that is a
   product decision.
5. **Should graphics be configurable off?** Task 1 assumes yes, following the
   observation setting's shape. It is the only sure defence against a decoder
   bug in a dependency.
