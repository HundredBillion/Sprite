//! A picture Sprite draws, and the GPU copy that goes with it.
//!
//! GPUI uploads an image to the window's texture atlas the first time it is
//! drawn and keeps it there until told to drop it, keyed by an identity every
//! `RenderImage::new` mints afresh. A cache that replaces or forgets a picture
//! would leave that copy behind for the life of the window. A `Picture`
//! releases its own: when the last handle goes, its image is queued, and the
//! window drops everything queued before it draws again. That covers every way
//! a cache lets go of a picture, including caches not yet written.

use std::cell::{Cell, RefCell};
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
