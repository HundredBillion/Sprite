# Return GPU Memory and Idle CPU Technical Spec

> **For agentic workers:** REQUIRED SUB-SKILL: Use dmi-superpowers:subagent-driven-development (recommended) or dmi-superpowers:executing-plans to implement this TSP task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Sprite releases the GPU copy of every picture it stops using, stops
asking for frames while a window has nothing to draw, and forgets closed panes
in Pane Control grant scopes (issue #140; PRD
`docs/PRDs/10-27-2026-return-gpu-memory-and-idle-cpu.md`).

**Architecture:** A new `picture` module owns a picture's GPU lifetime. Every
image cache holds an `Arc<Picture>`, and dropping the last handle queues the
image for `App::drop_image` before the window's next frame. One new GPUI patch
does four things:
- a test-only atlas tile count;
- a Metal atlas that frees a removed tile;
- release of the `NSScreenNumber` strings;
- frame delivery that pauses after 1 s idle and is woken through the
  `WindowInvalidator`.

`Workspace::refresh_layout`, which every close path runs, prunes closed panes
from grant scopes.

**Tech Stack:** Rust 1.98.1 (pinned), GPUI 0.2.2 vendored with exact patches,
`etagere` 0.2.15, Python 3 patch checker.

## Global Constraints

- Build, test and lint with the pinned toolchain. Plain `cargo` on this Mac is
  Homebrew's and ignores `rust-toolchain.toml`. Prefix every cargo command with
  `PATH=$HOME/.cargo/bin:$PATH DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer`.
  The steps below write that prefix as `$RUST`. Set it once with:
  `export RUST="env PATH=$HOME/.cargo/bin:$PATH DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer"`.
- Vendored GPUI must equal the published 0.2.2 archive plus the recorded
  patches (`python3 scripts/check_gpui_patch.py`). Every change under
  `vendor/gpui` goes into one new patch,
  `patches/gpui-0.2.2-release-and-idle.patch`, which is regenerated after each
  task that touches GPUI (see "Regenerating the GPUI patch").
- `RenderImage::new` is called only in `crates/sprite-app/src/picture.rs`.
- The pause threshold is 1 s (`FRAME_IDLE_PAUSE`).
- The pane cap stays `control::MAX_PANES` = 128 and now counts open panes.
- `SPRITE_PANE` stays. `SPRITE_TAB` goes. Historical TSPs that mention it are
  not edited.
- Comments explain why in plain prose and never cite issues, ADRs, PRDs or
  TSPs (`dmi-superpowers:writing-code-comments`).
- Commit messages end with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- Branch: `issue-140-release-gpu-idle-cpu`. Do not push until Task 8.

### Regenerating the GPUI patch

Run this from the repository root after every task that edits `vendor/gpui`.
`master`'s tree already contains the four earlier patches, so the diff against
`master` is exactly the new patch, at the line numbers of the tree those four
produce.

```bash
cd /Users/davidlee/Projects/Sprite
git diff --relative=vendor/gpui master -- vendor/gpui \
  | grep -v -e '^diff --git ' -e '^index ' \
  > patches/gpui-0.2.2-release-and-idle.patch
python3 scripts/check_gpui_patch.py
python3 -m unittest scripts.test_gpui_patch -v
```

Expected: `GPUI archive checksum and exact patched source verified`, and every
unittest `ok`.

---

## File Structure

| File | Responsibility |
| --- | --- |
| `crates/sprite-app/src/picture.rs` (new) | `Picture`, its release queue, `release_dropped`, and the test that only it makes a `RenderImage` |
| `crates/sprite-app/src/lib.rs` | declare `mod picture;` |
| `crates/sprite-app/src/workspace/mod.rs` | drain the queue at the top of `Workspace::render`; call `prune_closed_panes` |
| `crates/sprite-app/src/graphics_cache.rs` | Kitty cache holds `Arc<Picture>` |
| `crates/sprite-app/src/surface/svg_decode.rs` | `SvgState::Ready(Arc<Picture>)` (covers Surface elements, tab icons, list icons, Markdown icons) |
| `crates/sprite-app/src/surface/render.rs`, `terminal_view/list_view.rs`, `terminal_view/native_tabs.rs`, `terminal_view/native_markdown.rs`, `terminal_view/surfaces.rs` | hold `Arc<Picture>`; draw `picture.image()` |
| `crates/sprite-app/src/terminal_view.rs` | correct the texture-lifetime comment |
| `crates/sprite-app/src/workspace/control.rs` | `prune_closed_panes`; drop the duplicate label removal |
| `crates/sprite-app/src/workspace/control_tests.rs` | grant-scope pruning tests |
| `crates/sprite-app/src/surface/channel.rs`, `observation/endpoint.rs`, `workspace/pane_factory.rs`, `workspace/reload.rs` | remove `SPRITE_TAB` and the now-unused `tab` argument |
| `vendor/gpui/src/platform/test/window.rs`, `vendor/gpui/src/app/test_context.rs` | test-only atlas tile count |
| `vendor/gpui/src/platform/mac/metal_atlas.rs` | free a removed tile; make removal safe to repeat |
| `vendor/gpui/src/platform/mac/window.rs` | release `NSScreenNumber`; pause and resume the display link |
| `vendor/gpui/src/platform.rs`, `vendor/gpui/src/window.rs` | `set_frame_delivery`, `frame_waker`, invalidator wake, idle pause |
| `patches/gpui-0.2.2-release-and-idle.patch`, `scripts/check_gpui_patch.py`, `.gitattributes`, `vendor/gpui.provenance.md` | record the patch |
| `docs/adr/0035-a-pictures-gpu-copy-ends-with-the-picture.md` (new), `docs/adr/0022-…`, `docs/adr/0033-…`, `docs/adr/0034-…` | decisions |
| `docs/pane-control.md`, `docs/local-logging.md` | user docs |

---

### Task 1: A Picture releases its GPU copy before the next frame

**Blocked by:** None

**Files:**
- Modify: `vendor/gpui/src/platform/test/window.rs` (`TestWindowState.sprite_atlas`, `TestWindow`, `TestAtlas`)
- Modify: `vendor/gpui/src/app/test_context.rs` (`impl VisualTestContext`, beside `simulate_scale_factor`)
- Create: `patches/gpui-0.2.2-release-and-idle.patch`
- Modify: `scripts/check_gpui_patch.py:15-20`, `.gitattributes`, `vendor/gpui.provenance.md`
- Create: `crates/sprite-app/src/picture.rs`
- Modify: `crates/sprite-app/src/lib.rs` (add `mod picture;` after `mod pane_tree;`)
- Modify: `crates/sprite-app/src/workspace/mod.rs:494-495` (`Workspace::render`)
- Test: `crates/sprite-app/src/picture.rs` (`mod tests`), `crates/sprite-app/src/workspace/layout_tests.rs`

**Interfaces:**
- Produces:
  - `gpui::VisualTestContext::atlas_tile_count(&self) -> usize`
  - `crate::picture::Picture`: `Picture::new(pixels: image::RgbaImage) -> Arc<Picture>`, `Picture::image(&self) -> Arc<gpui::RenderImage>`, `Deref<Target = RenderImage>`
  - `crate::picture::release_dropped(window: &mut gpui::Window, cx: &mut gpui::App)`
  - `#[cfg(test)] crate::picture::released_by(run: impl FnOnce()) -> usize`, which returns how many pictures `run` let go of
  - `#[cfg(test)] crate::picture::queued() -> usize`

- [ ] **Step 1: Add the test-only atlas tile count to GPUI**

In `vendor/gpui/src/platform/test/window.rs`, give the test window its concrete atlas, so tests can count tiles:

```rust
// in struct TestWindowState
    sprite_atlas: Arc<TestAtlas>,
```

`TestWindow::new` already writes `sprite_atlas: Arc::new(TestAtlas::new()),`
and `fn sprite_atlas` returns `self.0.lock().sprite_atlas.clone()`, which
coerces to `Arc<dyn PlatformAtlas>`. Leave both unchanged. Add to `impl TestWindow`, after `simulate_scale_factor`:

```rust
    /// How many images the window's texture atlas holds.
    pub fn atlas_tile_count(&self) -> usize {
        self.0.lock().sprite_atlas.tile_count()
    }
```

Add to `impl TestAtlas` (the inherent impl with `pub fn new`):

```rust
    pub(crate) fn tile_count(&self) -> usize {
        self.0.lock().tiles.len()
    }
```

In `vendor/gpui/src/app/test_context.rs`, add to `impl VisualTestContext`, directly after `simulate_scale_factor`:

```rust
    /// How many images the window's texture atlas holds.
    pub fn atlas_tile_count(&self) -> usize {
        self.test_window(self.window).atlas_tile_count()
    }
```

If `TestWindow` is missing an import (`Arc` is already in scope as `sync::Arc` in this file), use the path the file already uses.

- [ ] **Step 2: Record the patch**

In `scripts/check_gpui_patch.py`, append to `PATCHES`:

```python
PATCHES = (
    "gpui-0.2.2-native-text.patch",
    "gpui-0.2.2-should-terminate.patch",
    "gpui-0.2.2-test-scale-factor.patch",
    "gpui-0.2.2-test-block-wakeup.patch",
    "gpui-0.2.2-release-and-idle.patch",
)
```

Append to `.gitattributes`:

```
patches/gpui-0.2.2-release-and-idle.patch -whitespace
```

Then run "Regenerating the GPUI patch". Expected: both checks pass.

Append to `vendor/gpui.provenance.md`, before `## Execution limits`:

```markdown
## Releasing pictures and pausing idle frames

`patches/gpui-0.2.2-release-and-idle.patch` applies to the tree the
test-block-wakeup patch produces; its line numbers are that tree's.

Test atlas tile count: the `test-support` `TestWindow` keeps its concrete
`TestAtlas` and reports how many tiles it holds, through
`VisualTestContext::atlas_tile_count`. Sprite uses it to test that a picture it
stops using leaves the texture atlas.
```

(Tasks 3 and 4 add paragraphs to this section.)

- [ ] **Step 3: Write the failing tests**

Create `crates/sprite-app/src/picture.rs` with only the type, functions that
do nothing yet, and the tests, so the tests compile and fail:

```rust
//! A picture Sprite draws, and the GPU copy that goes with it.
//!
//! GPUI uploads an image to the window's texture atlas the first time it is
//! drawn and keeps it there until told to drop it, keyed by an identity every
//! `RenderImage::new` mints afresh. A cache that replaces or forgets a picture
//! would leave that copy behind for the life of the window. A `Picture`
//! releases its own: when the last handle goes, its image is queued, and the
//! window drops everything queued before it draws again. That covers every way
//! a cache lets go of a picture, including caches not yet written.

use std::ops::Deref;
use std::sync::Arc;

use gpui::{App, RenderImage, Window};

/// One decoded image, drawn through `image()`.
#[derive(Debug)]
pub(crate) struct Picture(Arc<RenderImage>);

impl Picture {
    pub(crate) fn new(pixels: image::RgbaImage) -> Arc<Self> {
        Arc::new(Self(Arc::new(RenderImage::new(vec![image::Frame::new(
            pixels,
        )]))))
    }

    /// What an `img` element draws. Hold the `Picture`, not this, between
    /// frames: only the `Picture` releases the GPU copy.
    pub(crate) fn image(&self) -> Arc<RenderImage> {
        Arc::clone(&self.0)
    }
}

impl Deref for Picture {
    type Target = RenderImage;

    fn deref(&self) -> &RenderImage {
        &self.0
    }
}

/// Drops the GPU copy of every picture let go of since the last call.
pub(crate) fn release_dropped(_window: &mut Window, _cx: &mut App) {}

#[cfg(test)]
pub(crate) fn queued() -> usize {
    0
}

#[cfg(test)]
pub(crate) fn released_by(run: impl FnOnce()) -> usize {
    run();
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Context, IntoElement, ParentElement, Render, Styled, div, img, px};

    /// A window that draws at most one picture, releasing as Sprite's does.
    struct Shows(Option<Arc<Picture>>);

    impl Render for Shows {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            release_dropped(window, cx);
            div().size_full().children(
                self.0
                    .as_ref()
                    .map(|picture| img(picture.image()).size(px(4.0))),
            )
        }
    }

    fn square() -> Arc<Picture> {
        Picture::new(image::RgbaImage::new(4, 4))
    }

    fn draw(cx: &mut gpui::VisualTestContext) {
        cx.update(|window, cx| {
            window.refresh();
            window.draw(cx).clear();
        });
    }

    #[gpui::test]
    fn a_picture_let_go_leaves_the_atlas_before_the_next_frame(cx: &mut gpui::TestAppContext) {
        let (view, cx) = cx.add_window_view(|_, _| Shows(Some(square())));
        draw(cx);
        assert_eq!(cx.atlas_tile_count(), 1, "drawing uploads the picture");
        view.update(cx, |view, _| view.0 = None);
        draw(cx);
        assert_eq!(cx.atlas_tile_count(), 0);
    }

    #[gpui::test]
    fn a_picture_still_held_keeps_its_gpu_copy(cx: &mut gpui::TestAppContext) {
        let held = square();
        let (view, cx) = cx.add_window_view({
            let held = Arc::clone(&held);
            move |_, _| Shows(Some(held))
        });
        draw(cx);
        view.update(cx, |view, _| view.0 = None);
        draw(cx);
        assert_eq!(cx.atlas_tile_count(), 1, "another holder still has it");
        drop(held);
        draw(cx);
        assert_eq!(cx.atlas_tile_count(), 0);
    }

    #[gpui::test]
    fn dropping_an_image_already_dropped_is_harmless(cx: &mut gpui::TestAppContext) {
        let first = square();
        let image = first.image();
        let (view, cx) = cx.add_window_view(move |_, _| Shows(Some(first)));
        draw(cx);
        cx.update(|window, cx| {
            cx.drop_image(image.clone(), Some(window));
            cx.drop_image(image, Some(window));
        });
        view.update(cx, |view, _| view.0 = Some(square()));
        draw(cx);
        assert_eq!(cx.atlas_tile_count(), 1, "only the new picture");
    }

    #[test]
    fn a_picture_let_go_on_a_thread_that_never_draws_is_not_kept() {
        std::thread::spawn(|| {
            drop(square());
            assert_eq!(queued(), 0, "nothing to release, so its pixels go now");
        })
        .join()
        .unwrap();
    }
}
```

Add `mod picture;` to `crates/sprite-app/src/lib.rs` after `mod pane_tree;`.

Add to `crates/sprite-app/src/workspace/layout_tests.rs` (it already `use`s
`super::*` and `test_support`; follow the file's existing imports):

```rust
#[gpui::test]
fn the_window_releases_pictures_let_go_before_it_draws(cx: &mut gpui::TestAppContext) {
    let (_workspace, cx) = test_support::test_workspace(cx);
    test_support::draw_workspace(cx);
    drop(crate::picture::Picture::new(image::RgbaImage::new(1, 1)));
    assert_eq!(crate::picture::queued(), 1);
    test_support::draw_workspace(cx);
    assert_eq!(crate::picture::queued(), 0);
}
```

- [ ] **Step 4: Run the tests and confirm they fail for the right reason**

Run: `$RUST cargo test -p sprite-app --lib picture:: the_window_releases_pictures`

Expected:
- `a_picture_let_go_leaves_the_atlas_before_the_next_frame` FAILS with `left: 1, right: 0` after its first assertion passed. The first assertion passing proves the test window really uploads pictures. If that assertion fails instead, the test seam is wrong: fix it before going on.
- `a_picture_still_held_keeps_its_gpu_copy` fails at the final `0`.
- `the_window_releases_pictures_let_go_before_it_draws` fails at `queued() == 1`.
- The repeat-drop and thread tests pass already. They pin behaviour that must not regress.

- [ ] **Step 5: Implement the release queue**

Replace the stubs in `picture.rs` (above `mod tests`) with:

```rust
use std::cell::{Cell, RefCell};

thread_local! {
    /// Images whose last `Picture` has gone, waiting for the window's next
    /// frame. Thread-local, like the Surface image ledger, so parallel tests
    /// never share one.
    static RELEASED: RefCell<Vec<Arc<RenderImage>>> = const { RefCell::new(Vec::new()) };
    /// Whether this thread draws windows. Only a drawing thread has GPU
    /// copies to release; a picture let go anywhere else (a decode whose
    /// result nobody wanted) never had one, and holding its pixels in a queue
    /// nobody empties would leak them.
    static DRAWS: Cell<bool> = const { Cell::new(false) };
}

impl Drop for Picture {
    fn drop(&mut self) {
        // `try_with`: a picture can be dropped while its thread is exiting,
        // after these locals are gone.
        let draws = DRAWS.try_with(Cell::get).unwrap_or(false);
        if draws {
            RELEASED
                .try_with(|released| released.borrow_mut().push(Arc::clone(&self.0)))
                .ok();
        }
    }
}

/// Drops the GPU copy of every picture let go of since the last call, from
/// every window. Called by each window before it paints, so a copy never
/// outlives its picture by more than a frame.
pub(crate) fn release_dropped(window: &mut Window, cx: &mut App) {
    DRAWS.with(|draws| draws.set(true));
    let released = RELEASED.with(|released| std::mem::take(&mut *released.borrow_mut()));
    for image in released {
        cx.drop_image(image, Some(window));
    }
}

#[cfg(test)]
pub(crate) fn queued() -> usize {
    RELEASED.with(|released| released.borrow().len())
}

/// How many pictures `run` let go of, counted as a drawing thread would.
#[cfg(test)]
pub(crate) fn released_by(run: impl FnOnce()) -> usize {
    DRAWS.with(|draws| draws.set(true));
    RELEASED.with(|released| released.borrow_mut().clear());
    run();
    RELEASED.with(|released| std::mem::take(&mut *released.borrow_mut()).len())
}
```

In `crates/sprite-app/src/workspace/mod.rs`, make this the first statement of `Workspace::render`:

```rust
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Before anything paints, so a picture let go since the last frame
        // gives back its GPU copy now, and an empty window still releases.
        crate::picture::release_dropped(window, cx);
        let (_, height, strip) = self.cached_pane_area();
```

- [ ] **Step 6: Run the tests and confirm they pass**

Run: `$RUST cargo test -p sprite-app --lib picture:: the_window_releases_pictures`
Expected: all 5 PASS.

- [ ] **Step 7: Commit**

```bash
git add vendor/gpui patches/gpui-0.2.2-release-and-idle.patch scripts/check_gpui_patch.py .gitattributes vendor/gpui.provenance.md crates/sprite-app/src/picture.rs crates/sprite-app/src/lib.rs crates/sprite-app/src/workspace/mod.rs crates/sprite-app/src/workspace/layout_tests.rs
git commit -m "feat: release a picture's GPU copy before the next frame

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 2: Every image cache holds Pictures

**Blocked by:** Task 1

**Files:**
- Modify: `crates/sprite-app/src/graphics_cache.rs:14-17,30,66-104,113-118`
- Modify: `crates/sprite-app/src/surface/svg_decode.rs:18,85-97,309,442,510,556,560,567`
- Modify: `crates/sprite-app/src/surface/render.rs:16,62,141,505`
- Modify: `crates/sprite-app/src/terminal_view/list_view.rs:9,56,155,581,587,827-830,844,868,945`
- Modify: `crates/sprite-app/src/terminal_view/native_tabs.rs:178`
- Modify: `crates/sprite-app/src/terminal_view/native_markdown.rs:14,36,153,342-358,671`
- Modify: `crates/sprite-app/src/terminal_view/surfaces.rs:2621`
- Modify: `crates/sprite-app/src/terminal_view.rs:93-97`
- Test: `graphics_cache.rs` tests, `surface/svg_decode.rs` tests, `picture.rs` tests

**Interfaces:**
- Consumes: `Picture::new`, `Picture::image`, `released_by` (Task 1)
- Produces:
  - `SvgState::Ready(Arc<Picture>)`
  - `SvgState::picture(&self) -> Option<Arc<Picture>>`
  - `ElementImageCache::picture(..) -> Option<Arc<Picture>>`
  - `GraphicsCache::texture` and `GraphicsCache::get` still return `Option<Arc<RenderImage>>`. That clone lives only for one frame's elements; the cache holds the `Picture`.

**Rule for this task:** anything that keeps a picture between frames holds
`Arc<Picture>`. An `img(...)` element is given `picture.image()`. A stored
`Arc<RenderImage>` would be repainted after its `Picture` released it and would
then never be released again. The compiler finds every site: `img` does not
accept `Arc<Picture>`.

- [ ] **Step 1: Write the failing tests**

Add to `graphics_cache.rs` `mod tests`:

```rust
    #[test]
    fn every_way_the_cache_lets_go_of_a_texture_releases_its_gpu_copy() {
        use crate::picture::released_by;
        // Room for two 1×1 images.
        let mut cache = GraphicsCache::with_budget(8);
        cache.texture(&image(1, 1, 1, 1)).unwrap();
        assert_eq!(
            released_by(|| {
                cache.texture(&image(1, 2, 1, 1)).unwrap();
            }),
            1,
            "a new generation replaces the old picture"
        );
        cache.texture(&image(2, 1, 1, 2)).unwrap();
        assert_eq!(
            released_by(|| {
                cache.texture(&image(3, 1, 1, 3)).unwrap();
            }),
            1,
            "the least recently placed picture is evicted for room"
        );
        assert_eq!(
            released_by(|| {
                cache.refresh(&[Arc::new(image(3, 1, 1, 3))]);
            }),
            1,
            "a picture no longer shown"
        );
        assert_eq!(released_by(|| cache.clear()), 1, "cleared");
        cache.texture(&image(4, 1, 1, 4)).unwrap();
        cache.texture(&image(4, 1, 1, 4)).unwrap();
        assert_eq!(released_by(|| drop(cache)), 1, "dropped with its pane, once");
    }
```

Add to `surface/svg_decode.rs` `mod tests`, after
`every_way_a_cache_lets_go_of_pictures_returns_them_to_the_process_budget`:

```rust
    #[gpui::test]
    fn every_way_a_cache_lets_go_of_a_picture_releases_its_gpu_copy(
        cx: &mut gpui::TestAppContext,
    ) {
        use crate::picture::released_by;
        let owner = cx.new(|_| Owner);
        let cache = SvgDecodes::new(None);
        let svgs = [square(4, 0), square(4, 1), square(4, 2)];
        want_all(cx, &owner, &cache, &svgs);
        cx.run_until_parked();
        let kept = Arc::clone(&svgs[0]);
        assert_eq!(released_by(|| cache.retain(|svg| *svg == kept)), 2, "retain");
        assert_eq!(released_by(|| cache.reset(None)), 1, "reset");
        want_all(cx, &owner, &cache, &svgs[..1]);
        cx.run_until_parked();
        assert_eq!(released_by(|| drop(cache)), 1, "dropped with its Surface");
    }
```

These cover every cache built on `SvgDecodes`: Surface element pictures
(`ElementImageCache`), tab icons, list icons and Markdown icons. A closed pane
drops its `GraphicsCache` and its Surfaces' `SvgDecodes`, which the two "dropped"
assertions cover.

Add to `picture.rs` `mod tests`:

```rust
    #[test]
    fn only_a_picture_makes_a_render_image() {
        let mut offenders = Vec::new();
        let mut stack = vec![std::path::PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src"
        ))];
        while let Some(path) = stack.pop() {
            if path.is_dir() {
                for entry in std::fs::read_dir(&path).unwrap() {
                    stack.push(entry.unwrap().path());
                }
            } else if path.extension().is_some_and(|extension| extension == "rs")
                && !path.ends_with("picture.rs")
                && std::fs::read_to_string(&path)
                    .unwrap()
                    .contains("RenderImage::new(")
            {
                offenders.push(path);
            }
        }
        assert!(
            offenders.is_empty(),
            "{offenders:?} make a RenderImage whose GPU copy nothing releases; use Picture::new"
        );
    }
```

- [ ] **Step 2: Run them and confirm they fail**

Run: `$RUST cargo test -p sprite-app --lib every_way_the_cache_lets_go every_way_a_cache_lets_go_of_a_picture only_a_picture_makes`
Expected:
- The two cache tests FAIL with `left: 0, right: 1` (or `2`), because the caches still hold bare `RenderImage`s.
- `only_a_picture_makes_a_render_image` FAILS, listing `graphics_cache.rs`, `svg_decode.rs` and `native_markdown.rs`.

- [ ] **Step 3: Kitty cache**

In `graphics_cache.rs`:

```rust
use crate::picture::Picture;
// ...
struct Entry {
    generation: u64,
    texture: Arc<Picture>,
    // (rest unchanged)
}
```

In `texture`: return `Some(entry.texture.image())` on a hit. Replace the
construction with `let texture = Picture::new(buffer);`. Keep
`texture: Arc::clone(&texture)` in the `Entry`, and end with
`Some(texture.image())`. In `get`: `.map(|entry| entry.texture.image())`.

Replace the comment above the replacement (`// Replacing an entry releases the old texture, …`) with:

```rust
        // Replacing an entry lets go of the old picture, whose GPU copy is then
        // released before the next frame: a generation change reclaims rather
        // than accumulates.
```

- [ ] **Step 4: SVG decodes and their four users**

In `surface/svg_decode.rs`:

```rust
use gpui::Context;
use crate::picture::Picture;
// ...
    Ready(Arc<Picture>),
// ...
    pub(crate) fn picture(&self) -> Option<Arc<Picture>> {
```

- Change `finish`'s `outcome`, `decode`, `rasterise` and `render_svg` to `Result<Arc<Picture>, RefusalKind>` and `Option<Arc<Picture>>`.
- The last line of `rasterise` becomes `Ok(Picture::new(buffer))`.
- `raster_bytes(&picture)` keeps compiling through `Deref`.

Then fix every compile error the same way:

- `surface/render.rs`:
  - `ElementImageCache::picture` and `image_for` return `Option<Arc<Picture>>`.
  - The element becomes `style::apply_all(img(picture.image()), &style.utilities)`.
  - Drop the unused `RenderImage` import.
- `terminal_view/list_view.rs`:
  - every `Option<Arc<RenderImage>>` becomes `Option<Arc<Picture>>` (lines 56, 155, 827-830, 844, 945);
  - the draws become `img(leading.image())`, `img(icon.image())`, `img(icon.image())`;
  - drop the unused `RenderImage` import.
- `terminal_view/native_tabs.rs:178`: `.children(icon.map(|picture| img(picture.image()).size_full()))`.
- `terminal_view/native_markdown.rs`:
  - `images: HashMap<(usize, usize), Arc<Picture>>`;
  - construction becomes `Picture::new(image.to_rgba8())`;
  - the block image becomes `img(image.image())`;
  - pass `picture.image()` to `inline_image`, which keeps its `Arc<RenderImage>` parameter because it only builds the element;
  - the tinted icon becomes `img(picture.image())`;
  - drop the unused `RenderImage` import.
- `terminal_view/surfaces.rs:2621`: the test helper returns `Option<std::sync::Arc<crate::picture::Picture>>`. Callers that read sizes keep working through `Deref`.

- [ ] **Step 5: Correct the pane comment**

Replace `terminal_view.rs:93-97` with:

```rust
    /// Textures for the images this pane is showing.
    ///
    /// Dropped with the view. Each texture's GPU copy is released before the
    /// window's next frame, so closing a pane or a tab gives back its pictures
    /// without a separate teardown path to forget to call.
    textures: crate::graphics_cache::GraphicsCache,
```

- [ ] **Step 6: Run the whole crate's tests**

Run: `$RUST cargo test -p sprite-app`
Expected: everything passes, including the three new tests.

- [ ] **Step 7: Commit**

```bash
git add crates/sprite-app/src
git commit -m "feat: every image cache releases the GPU copies it lets go of

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 3: The Metal atlas frees a removed image's space; NSScreenNumber strings are released

**Blocked by:** Task 1 (the patch file exists)

**Files:**
- Modify: `vendor/gpui/src/platform/mac/metal_atlas.rs:60-88` (`MetalAtlas::remove`)
- Modify: `vendor/gpui/src/platform/mac/window.rs:1066,2523`
- Modify: `patches/gpui-0.2.2-release-and-idle.patch` (regenerated), `vendor/gpui.provenance.md`

**Interfaces:** none new. `MetalAtlas::remove` is safe to call more than once for the same key.

The vendored GPUI is outside the workspace (`Cargo.toml` `exclude`), and no CI
job runs its tests. Verification for this task is:
- the exact-apply patch check;
- the repeat-drop test from Task 1;
- the by-hand measurements in Task 8.

- [ ] **Step 1: Rewrite `MetalAtlas::remove`**

```rust
    fn remove(&self, key: &AtlasKey) {
        let mut lock = self.0.lock();
        // The key leaves first, whatever else happens, so removing an image
        // twice cannot count it out of its texture twice.
        let Some(tile) = lock.tiles_by_key.remove(key) else {
            return;
        };
        let id = tile.texture_id;

        let textures = match id.kind {
            AtlasTextureKind::Monochrome => &mut lock.monochrome_textures,
            AtlasTextureKind::Polychrome => &mut lock.polychrome_textures,
        };

        let Some(texture_slot) = textures
            .textures
            .iter_mut()
            .find(|texture| texture.as_ref().is_some_and(|v| v.id == id))
        else {
            return;
        };

        if let Some(mut texture) = texture_slot.take() {
            texture.decrement_ref_count();

            if texture.is_unreferenced() {
                textures.free_list.push(id.index as usize);
            } else {
                // Its space goes back to the texture for the next image, rather
                // than waiting for every other image on it to go too.
                texture.allocator.deallocate(tile.tile_id.into());
                *texture_slot = Some(texture);
            }
        }
    }
```

- [ ] **Step 2: Release the `NSScreenNumber` keys**

In `vendor/gpui/src/platform/mac/window.rs`, `ns_string` (already imported)
makes an autoreleased string:

```rust
            let screen_number: id = NSDictionary::valueForKey_(
                device_description,
                ns_string("NSScreenNumber"),
            );
```

```rust
        let screen_number_key: id = ns_string("NSScreenNumber");
```

- [ ] **Step 3: Build, regenerate, check**

Run: `$RUST cargo build -p sprite-app`, then "Regenerating the GPUI patch".
Expected: build succeeds; both checks pass.

Append to the provenance section from Task 1:

```markdown
Metal atlas removal: `MetalAtlas::remove` took the key out of `tiles_by_key`
only when its whole texture emptied, and never returned the tile to the
texture's allocator. A removed image's space was therefore reused only once
every other image on that texture had gone, and removing the same image twice
lowered the texture's count twice. It now always takes the key out, returns
the tile to the `etagere` allocator, and frees the texture when it empties, as
before. Replacing a 512×512 Kitty image 60 times took `IOAccelerator
(graphics)` from 11.5 to 66.7 MB before Sprite released pictures; the
measurement after this patch is recorded in the pull request.

`NSScreenNumber`: both dictionary keys were `alloc`/`init` strings that were
never released, about 30 × 32 B per session under `leaks`. They are now
autoreleased through `ns_string`.
```

- [ ] **Step 4: Commit**

```bash
git add vendor/gpui patches/gpui-0.2.2-release-and-idle.patch vendor/gpui.provenance.md
git commit -m "fix(gpui): free a removed image's atlas space and release screen-number keys

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 4: A window with nothing to draw stops asking for frames

**Blocked by:** Task 3 (same patch file; avoid regenerating it concurrently)

**Files:**
- Modify: `vendor/gpui/src/platform.rs:547-558` (`trait PlatformWindow`)
- Modify: `vendor/gpui/src/window.rs:94-150` (`WindowInvalidator`), `:989` (`Window::new`), `:1018-1066` (frame callback), `:1644-1646` (`on_next_frame`)
- Modify: `vendor/gpui/src/platform/mac/window.rs:394` (`MacWindowState` fields), `:477-499` (`start_display_link`, `stop_display_link`), `:692` (initialiser), `:939` (close), `impl PlatformWindow for MacWindow`
- Modify: `patches/gpui-0.2.2-release-and-idle.patch`, `vendor/gpui.provenance.md`

**Interfaces:**
- Produces, as `PlatformWindow` defaults that do nothing:
  - `fn set_frame_delivery(&self, active: bool)`
  - `fn frame_waker(&self) -> Option<Rc<dyn Fn()>>`
- Produces, on `WindowInvalidator`: `set_waker`, `pause`, `wake`.

GPUI's test platform keeps the defaults, so Sprite's tests can't observe the
pause. It is checked by hand in Task 8.

- [ ] **Step 1: Platform hooks**

In `trait PlatformWindow`, after `fn completed_frame(&self) {}`:

```rust
    /// Stops or restarts the platform's frame callbacks. A window with
    /// nothing to draw stops them, so an idle window costs no CPU.
    fn set_frame_delivery(&self, _active: bool) {}
    /// Restarts stopped frame callbacks without needing the window, for the
    /// invalidator to call when anything asks for a redraw. `None` where they
    /// never stop.
    fn frame_waker(&self) -> Option<Rc<dyn Fn()>> {
        None
    }
```

(`Rc` is in scope in `platform.rs`. If not, use `std::rc::Rc`.)

- [ ] **Step 2: Invalidator wake**

```rust
struct WindowInvalidatorInner {
    pub dirty: bool,
    pub draw_phase: DrawPhase,
    pub dirty_views: FxHashSet<EntityId>,
    /// Frame callbacks are stopped because nothing needed drawing.
    paused: bool,
    waker: Option<Rc<dyn Fn()>>,
}
```

`new()` initialises `paused: false, waker: None`. Then:

```rust
    pub fn invalidate_view(&self, entity: EntityId, cx: &mut App) -> bool {
        let mut inner = self.inner.borrow_mut();
        inner.dirty_views.insert(entity);
        if inner.draw_phase == DrawPhase::None {
            inner.dirty = true;
            cx.push_effect(Effect::Notify { emitter: entity });
            drop(inner);
            self.wake();
            true
        } else {
            false
        }
    }

    pub fn set_dirty(&self, dirty: bool) {
        self.inner.borrow_mut().dirty = dirty;
        if dirty {
            self.wake();
        }
    }

    pub fn set_waker(&self, waker: Option<Rc<dyn Fn()>>) {
        self.inner.borrow_mut().waker = waker;
    }

    /// Records that frame callbacks are stopped.
    pub fn pause(&self) {
        self.inner.borrow_mut().paused = true;
    }

    /// Restarts stopped frame callbacks. Every redraw request passes through
    /// here, so none can be lost while they are stopped.
    pub fn wake(&self) {
        let waker = {
            let mut inner = self.inner.borrow_mut();
            if !std::mem::replace(&mut inner.paused, false) {
                return;
            }
            inner.waker.clone()
        };
        if let Some(waker) = waker {
            waker();
        }
    }
```

The waker is called after the `RefCell` borrow ends, so a waker that redraws
can't hit a double borrow.

- [ ] **Step 3: Wire the window**

In `Window::new`, directly after `let invalidator = WindowInvalidator::new();`:

```rust
        invalidator.set_waker(platform_window.frame_waker());
```

`on_next_frame`:

```rust
    pub fn on_next_frame(&self, callback: impl FnOnce(&mut Window, &mut App) + 'static) {
        RefCell::borrow_mut(&self.next_frame_callbacks).push(Box::new(callback));
        self.invalidator.wake();
    }
```

Add near the other window constants in `window.rs`:

```rust
/// How long a window may go with nothing to draw before it stops asking the
/// platform for frames. It also covers the second of presentation after input
/// that keeps a variable-refresh display from lowering its rate.
const FRAME_IDLE_PAUSE: Duration = Duration::from_secs(1);
```

In the `on_request_frame` closure (`window.rs:1018`):
- add `let mut idle_since: Option<Instant> = None;` to the captured state, next to `let last_input_timestamp = last_input_timestamp.clone();`;
- inside it, record `let had_callbacks = !next_frame_callbacks.is_empty();` right after `take()`;
- record `let drew = invalidator.is_dirty() || request_frame_options.force_render;` before the draw branch, and use `if drew {` for that branch.

After the `complete_frame` update, at the end of the closure:

```rust
                // After a second with nothing to draw, nothing to present and
                // nothing waiting for a frame, stop asking for frames. Anything
                // that redraws restarts them through the invalidator.
                if drew || needs_present || had_callbacks {
                    idle_since = None;
                } else if idle_since
                    .get_or_insert_with(Instant::now)
                    .elapsed()
                    >= FRAME_IDLE_PAUSE
                {
                    idle_since = None;
                    invalidator.pause();
                    handle
                        .update(&mut cx, |_, window, _| {
                            window.platform_window.set_frame_delivery(false)
                        })
                        .log_err();
                }
```

The closure must be `move` and own `idle_since`. It already is `move`.

- [ ] **Step 4: macOS display link pause and resume**

`DisplayLink::start` and `stop` resume and suspend a dispatch source, so they
must alternate, and `DisplayLink`'s `Drop` calls `stop` once more. The window
tracks a pause so that every stop it adds is matched.

In `MacWindowState`, after `display_link: Option<DisplayLink>,`:

```rust
    /// The display link is stopped because the window had nothing to draw.
    frame_delivery_paused: bool,
```

Initialise it as `frame_delivery_paused: false,` next to `display_link: None,` (around line 692).

```rust
    fn start_display_link(&mut self) {
        self.stop_display_link();
        // (body unchanged)
    }

    fn stop_display_link(&mut self) {
        if let Some(mut display_link) = self.display_link.take()
            && self.frame_delivery_paused
        {
            // Dropping stops the link again; restart it first so that stop
            // is matched.
            display_link.start().log_err();
        }
        self.frame_delivery_paused = false;
    }

    fn set_frame_delivery(&mut self, active: bool) {
        if self.frame_delivery_paused != active {
            return;
        }
        let Some(display_link) = self.display_link.as_mut() else {
            return;
        };
        if active {
            display_link.start().log_err();
        } else {
            display_link.stop().log_err();
        }
        self.frame_delivery_paused = !active;
    }
```

`set_frame_delivery`'s early return reads: paused and asked to stop, or running
and asked to start, means there is nothing to do. With no link (the window is
hidden), nothing changes. The occlusion handler makes a fresh, running link
when the window shows again, and `start_display_link` clears the flag through
`stop_display_link`.

At line 939 (window close), replace `this.display_link.take();` with
`this.stop_display_link();`. Keep whatever binding `this` is there (a lock
guard), for example `lock.stop_display_link();`.

In `impl PlatformWindow for MacWindow`:

```rust
    fn set_frame_delivery(&self, active: bool) {
        self.0.lock().set_frame_delivery(active);
    }

    fn frame_waker(&self) -> Option<Rc<dyn Fn()>> {
        // GPUI's macOS window code releases this lock before it calls into the
        // app, so a redraw request made from app code can take it.
        let state = Arc::downgrade(&self.0);
        Some(Rc::new(move || {
            if let Some(state) = state.upgrade() {
                state.lock().set_frame_delivery(true);
            }
        }))
    }
```

- [ ] **Step 5: Build, test, regenerate, check**

Run:
- `$RUST cargo build -p sprite-app`
- `$RUST cargo test -p sprite-app`
- `$RUST cargo clippy --workspace --all-targets --locked -- -D warnings`
- "Regenerating the GPUI patch"

Expected: all pass. A Linux build is checked by CI in Task 8. The defaults
compile on every platform.

Append to the provenance section:

```markdown
Idle frame pause: a visible macOS window ran its `CVDisplayLink` sixty times a
second whether or not anything changed, about 85% of an idle Sprite's 0.5% of
one core. After a second in which a frame callback found nothing to draw,
nothing to present and no next-frame callbacks, `Window` calls
`PlatformWindow::set_frame_delivery(false)`, and macOS stops the existing link.
Every redraw request already passes through `WindowInvalidator`
(`invalidate_view`, `set_dirty`) or `on_next_frame`, so those call the
platform's `frame_waker`, which restarts it. `DisplayLink` start and stop
suspend and resume a dispatch source and must alternate; the window records
the pause so each added stop is matched, including when the link is replaced
or dropped. Other platforms keep the default hooks, which do nothing.
```

- [ ] **Step 6: Commit**

```bash
git add vendor/gpui patches/gpui-0.2.2-release-and-idle.patch vendor/gpui.provenance.md
git commit -m "feat(gpui): stop asking for frames while a window has nothing to draw

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 5: Closed panes leave every grant scope

**Blocked by:** None

**Files:**
- Modify: `crates/sprite-app/src/workspace/control.rs:81-85` (`prune_control_labels`), `:528` (Close), plus a test-only accessor
- Modify: `crates/sprite-app/src/workspace/mod.rs:383` (`refresh_layout`)
- Modify: `docs/pane-control.md:29-30`, `docs/adr/0034-authorize-pane-control-separately-from-observation.md`
- Test: `crates/sprite-app/src/workspace/control_tests.rs`

**Interfaces:**
- Produces: `Workspace::prune_closed_panes(&mut self)`, replacing `prune_control_labels`
- Produces: `#[cfg(test)] Workspace::scoped_panes(&self) -> std::collections::BTreeSet<u64>`, every pane any grant's scope names

- [ ] **Step 1: Test-only accessor**

Append to `workspace/control.rs`:

```rust
#[cfg(test)]
impl Workspace {
    /// Every pane any selected-pane grant's scope names.
    pub(super) fn scoped_panes(&self) -> std::collections::BTreeSet<u64> {
        self.control
            .grants
            .values()
            .flat_map(|grant| grant.scope.panes.iter().copied())
            .collect()
    }
}
```

- [ ] **Step 2: Write the failing tests**

Add to `control_tests.rs`:

```rust
/// The numeric pane ID in a created pane's address (`window:id`).
fn created_pane(response: &Value) -> u64 {
    assert_eq!(response["outcome"], "ok", "{response}");
    let id = response["result"]["id"].as_str().unwrap();
    id.rsplit_once(':').unwrap().1.parse().unwrap()
}

#[gpui::test]
fn every_way_of_closing_a_pane_takes_it_out_of_grant_scopes(cx: &mut gpui::TestAppContext) {
    if !private_runtime(module_path!(), cx) {
        return;
    }
    let (workspace, cx) = test_support::test_workspace(cx);
    let fixed = approve_scope(&workspace, cx, &[0]);
    let human = format!("{}:0", fixed.window);
    let beside = || Operation::Create {
        beside: Some(human.clone()),
        side: Some(crate::control::Side::Right),
    };
    let by_control = created_pane(&ask(&fixed, beside(), cx));
    let exited = created_pane(&ask(&fixed, beside(), cx));
    let focused = created_pane(&ask(&fixed, beside(), cx));
    let own_tab = created_pane(&ask(
        &fixed,
        Operation::Create {
            beside: None,
            side: None,
        },
        cx,
    ));
    let scoped = |cx: &mut gpui::VisualTestContext| {
        workspace.read_with(cx, |workspace, _| workspace.scoped_panes())
    };
    assert_eq!(
        scoped(cx),
        [0, by_control, exited, focused, own_tab].into_iter().collect()
    );

    let closed = ask(
        &fixed,
        Operation::Close {
            pane: format!("{}:{by_control}", fixed.window),
        },
        cx,
    );
    assert_eq!(closed["outcome"], "ok", "{closed}");
    assert!(!scoped(cx).contains(&by_control), "Pane Control close");

    workspace.update(cx, |workspace, cx| {
        let tab = workspace.tabs.tab_of(PaneId(exited)).unwrap();
        workspace.close_exited_pane(tab, PaneId(exited), cx);
    });
    assert!(!scoped(cx).contains(&exited), "a pane whose program exited");

    workspace.update(cx, |workspace, cx| {
        assert!(workspace.tabs.focus_pane(PaneId(focused)));
        workspace.close_focused_pane(false, cx);
    });
    assert!(!scoped(cx).contains(&focused), "the close-pane key");

    workspace.update(cx, |workspace, cx| {
        let tab = workspace.tabs.tab_of(PaneId(own_tab)).unwrap();
        assert!(workspace.tabs.focus_tab(tab));
        workspace.close_active_tab(false, cx);
    });
    assert!(!scoped(cx).contains(&own_tab), "the close-tab key");
    assert_eq!(scoped(cx), [0].into_iter().collect());
}

#[gpui::test]
fn a_fixed_grant_counts_only_its_open_panes_toward_the_cap(cx: &mut gpui::TestAppContext) {
    if !private_runtime(module_path!(), cx) {
        return;
    }
    let (workspace, cx) = test_support::test_workspace(cx);
    let fixed = approve_scope(&workspace, cx, &[0]);
    let human = format!("{}:0", fixed.window);
    for _ in 0..control::MAX_PANES {
        let pane = created_pane(&ask(
            &fixed,
            Operation::Create {
                beside: Some(human.clone()),
                side: Some(crate::control::Side::Right),
            },
            cx,
        ));
        let closed = ask(
            &fixed,
            Operation::Close {
                pane: format!("{}:{pane}", fixed.window),
            },
            cx,
        );
        assert_eq!(closed["outcome"], "ok", "{closed}");
    }
    created_pane(&ask(
        &fixed,
        Operation::Create {
            beside: Some(human),
            side: Some(crate::control::Side::Right),
        },
        cx,
    ));
    let _ = workspace;
}
```

If `Operation::Close` has more fields than `pane`, fill them as the existing
`public_close_requires_targeted_confirmation_and_revocation_cancels_it` test
does. If a close answers `confirmation_required`, the pane's test session is
treated as running. Then close it the way that test does. Don't weaken the
assertion.

- [ ] **Step 3: Run them and confirm they fail**

Run: `$RUST cargo test -p sprite-app --lib every_way_of_closing_a_pane_takes_it_out a_fixed_grant_counts_only`
Expected:
- The first FAILS at `"Pane Control close"`.
- The second FAILS on the final create with `outcome` `busy`, after 128 successful create/close rounds.

- [ ] **Step 4: Prune in the one place every close path runs**

Replace `prune_control_labels` in `workspace/control.rs`:

```rust
    /// Forgets closed panes everywhere Pane Control recorded them: their
    /// labels and every selected-pane grant's scope. Every way of closing a
    /// pane ends in a layout refresh, which calls this, so no close path has
    /// to remember it. A pane's identity is never reused, so a forgotten one
    /// can't authorize anything again, and a scope holds only open panes,
    /// which is what its limit counts.
    pub(super) fn prune_closed_panes(&mut self) {
        let tabs = &self.tabs;
        self.control
            .labels
            .retain(|pane, _| tabs.get(*pane).is_some());
        for grant in self.control.grants.values_mut() {
            grant
                .scope
                .panes
                .retain(|pane| tabs.get(PaneId(*pane)).is_some());
        }
    }
```

In `workspace/mod.rs` `refresh_layout`, replace `self.prune_control_labels();` with `self.prune_closed_panes();`.

In the `Operation::Close` arm, delete `self.control.labels.remove(&pane);`.
`after_close` runs `refresh_layout`, which now forgets the label.

- [ ] **Step 5: Run the control tests**

Run: `$RUST cargo test -p sprite-app --lib workspace::`
Expected: all pass. If the 128-round test takes more than about 20 s, report
the time in the task summary. Don't shrink the loop.

- [ ] **Step 6: Docs**

In `docs/pane-control.md`, replace:

```
Its scope retains at most 128 identities over its lifetime; request a fresh
grant if that limit is reached.
```

with:

```
Its scope holds at most 128 open panes. A closed pane leaves the scope, so a
long-lived grant can keep creating and closing panes.
```

Append to `docs/adr/0034-authorize-pane-control-separately-from-observation.md`:

```markdown
Amended 2026-10-27: a selected-pane grant's scope holds only open panes. The
approved design bounded agent-created resources at 128 live panes, but the
first implementation kept every pane a grant had ever created in its scope, so
a long-lived grant was refused as `busy` after 128 creations even with none
open. The workspace's layout refresh, which every way of closing a pane runs,
now removes closed panes from every grant's scope. This can't widen a grant:
pane identities are never reused, so a removed identity can never name a pane
again. The 128 limit still bounds a scope's size and what one grant can hold
open.
```

- [ ] **Step 7: Commit**

```bash
git add crates/sprite-app/src/workspace docs/pane-control.md docs/adr/0034-authorize-pane-control-separately-from-observation.md
git commit -m "fix: closed panes leave Pane Control grant scopes

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 6: Stop exporting SPRITE_TAB

**Blocked by:** None

**Files:**
- Modify: `crates/sprite-app/src/surface/channel.rs:768-787` (`SurfaceEndpoint::environment`) and its test at `:2390-2403`
- Modify: `crates/sprite-app/src/observation/endpoint.rs:138-157` (`Endpoint::environment`) and its test at `:575-593`
- Modify: `crates/sprite-app/src/workspace/pane_factory.rs:85,134-150` (`session_environment`)
- Modify: `crates/sprite-app/src/workspace/reload.rs:405`

**Interfaces:**
- Produces:
  - `SurfaceEndpoint::environment(&self, pane: PaneId)`
  - `Endpoint::environment(&self, pane: PaneId)`
  - `session_environment(endpoint, surfaces, pane)`

  Each loses its `tab: TabId` argument, which only `SPRITE_TAB` used.

- [ ] **Step 1: Change the two tests to expect no `SPRITE_TAB`**

In `surface/channel.rs`, replace `assert_eq!(get("SPRITE_TAB").as_deref(), Some("2"));` with:

```rust
        assert_eq!(get("SPRITE_TAB"), None, "a pane can move tabs; only its pane id is stable");
```

In `observation/endpoint.rs`, the `lookup` helper panics on a missing
variable. Replace `assert_eq!(lookup("SPRITE_TAB"), "3");` with:

```rust
        assert!(
            !environment.iter().any(|(key, _)| key == "SPRITE_TAB"),
            "a pane can move tabs; only its pane id is stable"
        );
```

- [ ] **Step 2: Run them and confirm they fail**

Run: `$RUST cargo test -p sprite-app --lib a_session_is_told_the_socket_the_key_and_its_own_identity surface::channel::`
Expected: both changed assertions FAIL, because `SPRITE_TAB` is present.

- [ ] **Step 3: Remove the export and the unused argument**

- In both `environment` functions, delete the `SPRITE_TAB` tuple and the `tab: TabId` parameter.
- In the `channel.rs` doc comment, replace "`SPRITE_TAB` and `SPRITE_PANE` repeat what the observation endpoint exports" with "`SPRITE_PANE` repeats what the observation endpoint exports".
- Remove `tab` from `session_environment` and its two callers (`pane_factory.rs:85`, `reload.rs:405`).
- Update the two tests' calls to `endpoint.environment(PaneId(5))` and `endpoint.environment(PaneId(7))`.
- Remove any `TabId` import this leaves unused.

- [ ] **Step 4: Run the tests and confirm they pass**

Run: `$RUST cargo test -p sprite-app`
Expected: all pass. `grep -rn SPRITE_TAB crates docs --include='*.rs' --include='*.md' | grep -v docs/TSPs/ | grep -v docs/PRDs/` prints nothing.

- [ ] **Step 5: Commit**

```bash
git add crates/sprite-app/src
git commit -m "fix: stop exporting SPRITE_TAB, which went stale when a pane moved

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 7: Record the decisions and fix the launch advice

**Blocked by:** Tasks 2, 3, 4

**Files:**
- Create: `docs/adr/0035-a-pictures-gpu-copy-ends-with-the-picture.md`
- Modify: `docs/adr/0022-bound-surface-raster-memory-at-the-decoder.md`, `docs/adr/0033-bound-surface-raster-memory-across-all-surfaces.md`
- Modify: `docs/local-logging.md:22`

- [ ] **Step 1: Write ADR 0035**

```markdown
# A picture's GPU copy ends with the picture

Approved for issue #140 on 2026-10-27. Amends ADRs 0022 and 0033.

GPUI uploads an image to a window's texture atlas the first time it is drawn
and keeps it until the application drops it, keyed by an identity that every
`RenderImage::new` mints afresh. Sprite never dropped one. Replacing a
512×512 Kitty image 60 times took `IOAccelerator (graphics)` from 11.5 to
66.7 MB, and none of it came back after the image was deleted. Every image
cache had the same gap: Kitty graphics, Surface element pictures, tab icons,
list icons and Markdown images.

Every cache now holds a `Picture`, the only code that makes a `RenderImage`
(a test enforces this). When the last handle to a picture goes, its image joins
a queue, and each window empties the queue through `App::drop_image` at the
start of its next render, before anything paints. Every way a cache lets go
(replacement, eviction, clearing, a closed pane or Surface) is covered without
each path remembering to release, and a cache added later inherits it.

The queue is thread-local, like the ADR 0033 ledger. Only a thread that has
drawn queues anything: a picture let go elsewhere never had a GPU copy, and
holding its pixels in a queue nobody empties would leak them.

GPUI's Metal atlas also kept a removed image's space until its whole texture
emptied. The GPUI patch returns the tile to the texture's allocator and makes
removal safe to repeat.

Consequences: a picture's GPU copy outlives it by at most one frame of the
window that drew it. ADRs 0022 and 0033 bound decoded pixels and excluded
"native texture copies". That exclusion no longer holds for retained pictures;
it still holds for usvg parse allocations and CPU time.

Rejected: calling `drop_image` at each eviction site. It is the scattered
pattern that caused the leak, and the next cache would forget it.
```

- [ ] **Step 2: Point 0022 and 0033 at it**

Append to each of `0022-…` and `0033-…`:

```markdown
Amended 2026-10-27 by ADR 0035: a retained picture's native texture copy is
now released with the picture, so that copy is no longer outside this bound.
```

- [ ] **Step 3: Fix the launch advice**

In `docs/local-logging.md:22`, replace the sentence that begins "For a macOS Dock launch, `launchctl setenv`" (through `launchctl unsetenv SPRITE_LOG`.) with:

```
On macOS 26 a value set with `launchctl setenv` does not reach an app the Dock starts. To log a desktop launch, quit Sprite completely first, then start it with `open -a Sprite --env SPRITE_LOG='sprite_app=debug'`: if Sprite is already running, `open` only brings the running copy forward and the value is ignored.
```

- [ ] **Step 4: Commit**

```bash
git add docs/adr docs/local-logging.md
git commit -m "docs: record that a picture's GPU copy ends with it; fix macOS logging launch advice

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 8: Verify, measure by hand, open the PR

**Blocked by:** Tasks 1-7

- [ ] **Step 1: Full checks with the pinned toolchain**

```bash
cd /Users/davidlee/Projects/Sprite
$RUST cargo fmt --all --check
$RUST cargo clippy --workspace --all-targets --locked -- -D warnings
$RUST cargo test --workspace --locked --no-fail-fast
python3 scripts/check_gpui_patch.py
python3 -m unittest scripts.test_gpui_patch -v
```

Expected: no formatting diff, no Clippy warnings, every test passes (1235 before this branch plus the new ones), and both patch checks pass. Report exact counts.

- [ ] **Step 2: Build a release binary for measurement**

Run: `$RUST cargo build --release -p sprite-app`. Measure with
`target/release/sprite`, never the installed app. Before driving any window
with keystrokes:
- check the screen is unlocked;
- check that the target window exists and is frontmost.

- [ ] **Step 3: GPU memory after 60 picture replacements**

Save as
`/private/tmp/claude-502/-Users-davidlee-Projects-Sprite/85a7fe9b-351d-4dc1-9330-b04329890ed1/scratchpad/kitty_replace.py`:

```python
import base64, sys, time

def send(payload, **keys):
    data = base64.b64encode(payload).decode()
    chunks = [data[i:i + 4096] for i in range(0, len(data), 4096)]
    head = ",".join(f"{k}={v}" for k, v in keys.items())
    for n, chunk in enumerate(chunks):
        more = 1 if n < len(chunks) - 1 else 0
        prefix = f"{head},m={more}" if n == 0 else f"m={more}"
        sys.stdout.write(f"\x1b_G{prefix};{chunk}\x1b\\")
    sys.stdout.flush()

side = 512
for n in range(60):
    pixel = bytes([n * 4 % 256, 255 - n * 4 % 256, 128, 255])
    send(pixel * side * side, a="T", f=32, s=side, v=side, i=1, q=2)
    time.sleep(0.1)
input("replaced 60 times; press Enter to delete")
sys.stdout.write("\x1b_Ga=d,d=I,i=1,q=2\x1b\\")
sys.stdout.flush()
input("deleted; press Enter to exit")
```

In a Sprite pane, run `python3 <that path>`. Record
`vmmap --summary <pid> | grep 'IOAccelerator (graphics)'` at three points:
- before the run;
- after the 60 replacements;
- 5 s after the delete.

Expected: after the delete, it returns near the before value (before this
branch: 11.5 → 66.7 MB, never returned).

- [ ] **Step 4: Idle CPU**

With one visible window, an idle shell, and the cursor not blinking: start
Sprite with a scratch config containing `[cursor]\nblink = false` (see the
`sprite --help` config flag). Then sample in the background for 5 minutes:

```bash
for i in $(seq 60); do ps -o %cpu= -p <pid>; sleep 5; done > idle.txt
sort -n idle.txt | awk '{a[NR]=$1} END {print "median", a[int(NR/2)+1]}'
```

Repeat with `blink = true`. Expected:
- not blinking: the median is close to the hidden-window figure of about 0.1%. Before this branch it was about 0.5% visible.
- blinking: about the old figure, as the PRD states.

- [ ] **Step 5: Resume checks (no freezes)**

After 5 s idle, check by hand that each of these appears at once:
- typed text;
- output from `sleep 3; echo late` arriving while idle;
- resizing;
- switching tabs;
- moving the window to another display;
- hiding and showing the window (Cmd-H);
- a Surface update;
- a minimap scroll.

Measure first-frame delay after idle in a scratch build that is not committed:
- temporarily add `eprintln!("wake {:?}", std::time::Instant::now())` in `WindowInvalidator::wake` after the early return;
- add `eprintln!("drew {:?}", std::time::Instant::now())` in the frame callback's draw branch;
- type a key after 5 s idle and record the difference;
- remove both lines and rebuild before continuing.

- [ ] **Step 6: Leaks**

Run `leaks <pid>` after a few window moves between displays. Expected: no
`NSScreenNumber` `CFString` entries.

- [ ] **Step 7: Open the PR**

Use `dmi-superpowers:creating-a-pull-request`. Before pushing, show the user
the PR title and body. Push only with their go-ahead:

```bash
git push -u origin issue-140-release-gpu-idle-cpu
gh pr create --title "Give back GPU memory and idle CPU that Sprite no longer needs" --body-file <body>
```

The body:
- includes the before/after numbers from Steps 3-6;
- says Linux follow-ups are #141;
- ends with `🤖 Generated with [Claude Code](https://claude.com/claude-code)`.
