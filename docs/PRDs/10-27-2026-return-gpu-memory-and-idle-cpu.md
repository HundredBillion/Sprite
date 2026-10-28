# Return GPU memory and idle CPU that Sprite no longer needs

Status: approved by the owner on 2026-10-27; hardened against repository interfaces.

Issue: https://github.com/HundredBillion/Sprite/issues/140
Linux follow-up: https://github.com/HundredBillion/Sprite/issues/141

Base: `6ec9590` (`master`). Branch: `issue-140-release-gpu-idle-cpu`.

## Outcome and scope

Sprite stops holding GPU memory for pictures it no longer shows, stops waking
the CPU sixty times a second while nothing on screen changes, and forgets a
closed pane everywhere it was recorded. One PR resolves every acceptance
criterion of #140 on macOS. The Linux work it leaves out is #141.

In scope:

1. A picture's GPU copy is released when Sprite stops using the picture.
2. GPUI pauses frame delivery after about 1 s with nothing to draw.
3. A GPUI patch releases the `NSScreenNumber` string and frees an image's space
   in the texture cache when the image is removed.
4. Closed panes are pruned from Pane Control grant scopes, so the 128-pane cap
   counts open panes.
5. `SPRITE_TAB` is no longer exported. `SPRITE_PANE` stays.
6. ADR and documentation fixes listed below.

Out of scope: Linux frame timing, the Linux texture cache and Linux
measurements (#141); a shared exit-waiter thread to reduce per-pane threads;
the usvg parse allocations and CPU time that ADRs 0022 and 0033 already
exclude.

## Evidence this rests on

- Replacing a 512×512 Kitty image 60 times took `IOAccelerator (graphics)`
  resident memory from 11.5 to 66.7 MB, and it was not returned after the image
  was deleted.
- Nothing in `crates/` calls GPUI's `drop_image`. GPUI frees an image's texture
  space only through `Window::drop_image` or `App::drop_image`, and
  `RenderImage::new` gives every picture a new `ImageId`, so each replaced
  picture stays in the texture cache.
- `MetalAtlas::remove` (`vendor/gpui/src/platform/mac/metal_atlas.rs`) lowers
  the texture's reference count but never deallocates the tile, so space is
  reused only when a whole texture empties.
- Idle CPU is about 0.5% of a core with the window visible and 0.1% hidden.
  About 85% comes from GPUI's `CVDisplayLink`, which runs whenever the window is
  visible (`vendor/gpui/src/platform/mac/window.rs`).
- `leaks` finds about 30 × 32 B `CFString`s per session from
  `NSString::alloc(nil).init_str("NSScreenNumber")`, which is never released
  (`vendor/gpui/src/platform/mac/window.rs` near lines 1066 and 2523).
- Pane Control inserts a created pane into its grant's scope
  (`workspace/control.rs`, `Operation::Create`) and never removes it. PaneIds
  are never reused, so after 128 creations through one grant, `create` is
  refused as `busy` even with no panes open.
- `SPRITE_TAB` is exported at spawn (`surface/channel.rs`,
  `observation/endpoint.rs`). Sprite never reads it, and it goes stale after a
  pane moves to another tab.

## Design

### 1. Pictures release their GPU copy (`Picture`)

A new module `crates/sprite-app/src/picture.rs` owns the lifetime of a picture
that GPUI draws.

- `Picture` wraps one `Arc<RenderImage>`. It is created only through
  `Picture::new(pixels: image::RgbaImage) -> Arc<Picture>` and exposes the image to elements
  through `Picture::image() -> Arc<RenderImage>`.
- Dropping the last `Arc<Picture>` pushes its `RenderImage` onto a release
  queue. The queue is thread-local, like the ADR 0033 memory ledger, so
  parallel tests never share one.
- `picture::release_dropped(window, cx)` empties the queue with
  `cx.drop_image(image, Some(window))`, which removes the image from every
  window's texture cache. The Workspace calls it at the start of each render,
  before any element paints.

Every cache that today holds an `Arc<RenderImage>` holds an `Arc<Picture>`
instead:

| Cache | Today |
| --- | --- |
| Kitty `GraphicsCache` | `graphics_cache.rs` |
| Surface element pictures and tab icons | `surface/svg_decode.rs` (`render_svg`, decode results), `surface/render.rs` |
| List icons | `terminal_view/list_view.rs` |
| Markdown pictures | `terminal_view/native_markdown.rs` |

Because release happens on `Drop`, every way a picture goes away is covered:
replaced, evicted by the 128 MB Kitty budget, cleared, its pane or Surface
closed, or its cache dropped. A cache added later gets the same behaviour by
using `Picture`.

Only a thread that has drawn queues anything. A picture dropped on a
background thread is not queued, because only the UI thread paints, so it
never had a GPU copy, and queuing it on a thread that never empties its queue
would hold its pixels forever. A picture dropped while no window renders is
released at the window's next render.

`RenderImage::new` is called only inside `picture.rs`. A test pins this, so a
new call site can't bypass the release.

### 2. Frame delivery pauses when idle (GPUI patch)

- GPUI's `PlatformWindow` gains `set_frame_delivery(active: bool)`. It does
  nothing on platforms other than macOS.
- On macOS, the window stops the existing `CVDisplayLink` and starts it again.
  It does not recreate the link. Its occlusion handling still decides whether
  the link may run at all, so a hidden window stays paused.
- GPUI's `Window` frame callback pauses delivery once it has found, for about
  1 s, nothing to draw, nothing to present and no next-frame callbacks. The 1 s
  also covers GPUI's existing rule of presenting for 1 s after input, which
  keeps ProMotion displays from lowering their refresh rate.
- Resume goes through one place, the `WindowInvalidator`. When GPUI creates a
  window, it asks the platform for a wake function and gives it to the
  invalidator. The invalidator calls it when the window goes from clean to
  dirty (`invalidate_view`, `set_dirty`), and registering a next-frame callback
  calls it too. Anything that redraws the window (`cx.notify`, input, a resize,
  an appearance change, Sprite's minimap `on_next_frame`) therefore resumes
  delivery. A missed path can't freeze the window, because no path bypasses
  the invalidator.
- On macOS the wake function holds a weak handle to the window's state and
  restarts the existing display link. It takes the state lock, as every
  `PlatformWindow` method does. GPUI's macOS code releases that lock before it
  calls into the app, so a redraw request can't deadlock on it. Display link
  start and stop must alternate, so the window records the pause and keeps
  them matched, including when the link is replaced or dropped. On other
  platforms it does nothing.
- Rejected: keeping the display link running and skipping only the hop to the
  main thread while idle. It needs no resume wiring, but the timer thread would
  still wake 60 times a second, so it can't reach the idle-CPU target.

Consequence: while the focused pane's cursor blinks, Sprite redraws every
530 ms, so the window never stays idle for 1 s and the link keeps running as it
does today. Measurements are recorded with blinking on and off.

### 3. NSScreenNumber and texture-cache space (same GPUI patch)

- Both `NSScreenNumber` strings are released after use (or replaced with a
  static `NSString`).
- `MetalAtlas::remove` always takes the key out of `tiles_by_key`, deallocates
  its tile from the texture's `etagere` allocator (`TileId` converts back to
  `AllocId`), then lowers the texture's count and frees the texture if it is
  empty. A removed image's space is reused without waiting for the whole
  texture to empty.
- Today the key is removed only when the whole texture empties, so removing
  the same image twice lowers the count twice. That can free a texture other
  pictures still use, and a re-added image gets a stale tile. After the patch,
  removing an image twice is harmless.

The patch goes in `patches/` and is recorded in `scripts/check_gpui_patch.py`
(`PATCHES`) and `vendor/gpui.provenance.md`, alongside the existing four
patches (ADRs 0027 and 0032).

A second, test-only change to GPUI lets tests count the tiles in
`TestAtlasState` (`platform/test/window.rs`), so a test can show that a
dropped picture leaves the texture cache. It is part of the same patch.

### 4. One place forgets a closed pane

`Workspace::refresh_layout` already prunes control labels and titles against
the live layout, and every close path (`close_focused_pane`,
`close_exited_pane`, `close_active_tab`, Pane Control `close`) runs it.

- `prune_control_labels` becomes `prune_closed_panes`. It also removes closed
  panes from every grant's `scope.panes`.
- Its doc comment names it as the single place that forgets closed panes.
- The duplicate `control.labels.remove` in Pane Control `close` is deleted.

The 128-pane check in `Operation::Create` therefore counts the grant's open
panes.

### 5. Remove `SPRITE_TAB`

Remove both exports, the two test assertions that expect it, and the
`channel.rs` doc comment that mentions it. No user doc lists it, and
`sprite.nvim` doesn't read it. Historical TSPs that mention it are left as
written. `SPRITE_PANE` is unchanged.

## Decisions to record

- **New ADR 0035, "A picture's GPU copy ends with the picture."** It records
  the `Picture` handle, the render-time drain, and the GPUI tile patch. It
  amends ADR 0022 and ADR 0033, whose "not native texture copies" exclusion no
  longer holds for retained pictures. Both ADRs get a one-line pointer to 0035.
- **ADR 0034 amendment.** The approved pane-control PRD bounded agent-created
  resources at "128 live panes". The implementation instead kept closed panes in
  a grant's scope, and `docs/pane-control.md` then documented a lifetime limit
  of 128 identities per grant. Pruning restores the live-pane bound. It is safe
  because pane IDs are never reused, so a pruned ID can't authorize anything
  again, and the scope's memory stays bounded by open panes.
- **Comments.** Fix the misleading comments at `terminal_view.rs:94-96` about
  when picture textures are freed.

## Documentation

- `docs/local-logging.md`: replace the `launchctl setenv SPRITE_LOG` advice
  with `open -a Sprite --env SPRITE_LOG=…`. Note that Sprite must be fully
  quit first, because `open` otherwise brings forward the running copy.
- `docs/pane-control.md:29`: replace "retains at most 128 identities over its
  lifetime; request a fresh grant if that limit is reached" with a statement
  that a selected-pane grant can have at most 128 open panes in scope, and that
  closed panes leave the scope.

## Acceptance and evidence

Automated tests (TDD, through public interfaces):

- Replacing, evicting and clearing a Kitty picture, closing its pane, and
  closing a Surface each leave no tile for the old picture in the test atlas.
  The tests also cover a second replacement and a cleared cache refilled.
- A picture still in use is not released.
- Releasing the same picture twice is harmless (test atlas).
- The Metal half of the texture-cache patch is verified by the exact-apply
  patch check and the by-hand measurements below. Vendored GPUI is outside the
  workspace, and no CI job runs its tests.
- Only `picture.rs` calls `RenderImage::new` (source-scan test).
- After 129 Pane Control creates and closes through one grant, the next create
  succeeds. Every close path prunes the grant scope.
- A spawned pane has `SPRITE_PANE` and no `SPRITE_TAB`.
- The GPUI patch check passes with the new patch listed.
- The full suite, Clippy and rustfmt pass with the pinned toolchain (Rust
  1.98.1).

By-hand measurements on macOS, recorded in the PR:

- Replace a 512×512 Kitty image 60 times, then delete it. `IOAccelerator
  (graphics)` returns near its starting value. Before: 11.5 → 66.7 MB, not
  returned.
- Measure idle CPU with the window visible and the cursor not blinking. The
  target is close to the hidden-window figure of about 0.1%. Also record the
  figure with a blinking cursor.
- Measure the time from a keystroke to the first frame after an idle pause.
- `leaks` shows no `NSScreenNumber` strings.

Linux: the shared code (`Picture`, grant pruning, `SPRITE_TAB`) builds and runs
on Linux in CI. Linux measurements and the Blade texture cache are #141.
