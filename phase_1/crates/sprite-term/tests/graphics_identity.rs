//! What an image *is*, and every way one stops existing.
//!
//! Identity is (image id, generation). The renderer caches textures against it,
//! so getting it wrong shows a stale picture — the kind of defect that looks
//! like a rendering bug and is really a bookkeeping one.
//!
//! Each way an image disappears gets its own test rather than one test for all
//! of them, because they are separate mechanisms in libghostty and a single
//! test passing would say nothing about the other four.

mod support;

use std::sync::Arc;

use sprite_term::{GraphicsPolicy, SnapshotBundle};

use support::{GraphicsSession, base64, kitty, pane_text};

/// Transmits and places a solid image of `size` square under `id`.
fn place(pane: &mut GraphicsSession, id: u32, size: u32, value: u8) -> Arc<SnapshotBundle> {
    let pixels = vec![value; (size * size * 4) as usize];
    pane.feed(&kitty(
        &format!("a=T,f=32,s={size},v={size},i={id}"),
        &base64(&pixels),
    ))
}

/// The images a bundle is carrying, by id.
fn images(bundle: &SnapshotBundle) -> Vec<u32> {
    bundle
        .graphics
        .as_ref()
        .map(|frame| frame.images.iter().map(|image| image.id).collect())
        .unwrap_or_default()
}

// ---- identity --------------------------------------------------------------

/// Replacing an image's content under the same id must advance its generation,
/// or a renderer caching by id alone would keep showing the old picture.
#[test]
fn replacing_content_under_one_id_advances_the_generation() {
    let mut pane = GraphicsSession::start(GraphicsPolicy::default());

    let first = place(&mut pane, 1, 8, 0xa0);
    let before = Arc::clone(
        first
            .graphics
            .as_ref()
            .expect("a frame")
            .image(1)
            .expect("image 1"),
    );

    let second = place(&mut pane, 1, 8, 0x0b);
    let after = second
        .graphics
        .as_ref()
        .expect("a frame")
        .image(1)
        .expect("image 1");

    assert_eq!(after.id, before.id, "the same id");
    assert_ne!(
        after.generation, before.generation,
        "but a different generation, which is what makes it a different image"
    );
    assert_ne!(after.pixels, before.pixels);
}

/// Two images of identical dimensions are still two images. Caching by size
/// would collapse them into one.
#[test]
fn two_images_of_the_same_size_are_never_confused() {
    let mut pane = GraphicsSession::start(GraphicsPolicy::default());

    place(&mut pane, 1, 8, 0x11);
    let bundle = place(&mut pane, 2, 8, 0x99);
    let frame = bundle.graphics.as_ref().expect("a frame");

    let first = frame.image(1).expect("image 1");
    let second = frame.image(2).expect("image 2");
    assert_eq!(
        (first.width, first.height),
        (second.width, second.height),
        "the same shape"
    );
    assert_ne!(first.pixels, second.pixels, "and different content");
    assert!(
        !Arc::ptr_eq(first, second),
        "held separately, not shared by shape"
    );
}

// ---- the five ways an image goes ------------------------------------------

#[test]
fn deleting_one_image_leaves_the_others() {
    let mut pane = GraphicsSession::start(GraphicsPolicy::default());
    place(&mut pane, 1, 8, 0x11);
    let both = place(&mut pane, 2, 8, 0x22);
    assert_eq!(images(&both), vec![1, 2]);

    // `a=d,d=I,i=1` deletes image 1 and its placements.
    let after = pane.feed(&kitty("a=d,d=I,i=1", ""));

    assert_eq!(
        images(&after),
        vec![2],
        "one image went and the other did not"
    );
}

#[test]
fn switching_to_the_alternate_screen_hides_the_images_behind_it() {
    let mut pane = GraphicsSession::start(GraphicsPolicy::default());
    let shown = place(&mut pane, 1, 8, 0x33);
    assert_eq!(images(&shown), vec![1]);

    // The alternate screen is its own screen, with its own images.
    let alternate = pane.feed("\\033[?1049h");
    assert!(
        alternate.graphics.is_none(),
        "the normal screen's images are not shown over an alternate screen: {:?}",
        alternate.graphics
    );

    let back = pane.feed("\\033[?1049l");
    assert_eq!(
        images(&back),
        vec![1],
        "and they return when the program leaves"
    );
}

#[test]
fn a_reset_clears_every_image() {
    let mut pane = GraphicsSession::start(GraphicsPolicy::default());
    place(&mut pane, 1, 8, 0x44);
    let both = place(&mut pane, 2, 8, 0x55);
    assert_eq!(images(&both), vec![1, 2]);

    // RIS: a full terminal reset.
    let after = pane.feed("\\033c");

    assert!(
        after.graphics.is_none(),
        "a reset takes the images with it: {:?}",
        after.graphics
    );
}

#[test]
fn storage_eviction_removes_an_image_from_the_projection() {
    // Room for one 32x32 image (4 KiB) and not two.
    let policy = GraphicsPolicy {
        storage_bytes: 6 * 1024,
        ..GraphicsPolicy::default()
    };
    let mut pane = GraphicsSession::start(policy);

    place(&mut pane, 1, 32, 0x66);
    let after = place(&mut pane, 2, 32, 0x77);

    let held = images(&after);
    assert!(
        held.len() < 2,
        "two images of that size cannot both be held: {held:?}"
    );
    assert!(
        pane.probe().stored_bytes() <= 6 * 1024,
        "and what remains is inside the limit"
    );
}

/// Closing the session must release the pixels it was holding.
///
/// Proved by reference count: once the worker is gone, the only reference left
/// is the one this test holds. A worker that kept its cache alive would show up
/// as a second reference.
#[test]
fn closing_the_session_releases_the_pixels_it_held() {
    let mut pane = GraphicsSession::start(GraphicsPolicy::default());
    let bundle = place(&mut pane, 1, 16, 0x88);
    let pixels = Arc::clone(
        bundle
            .graphics
            .as_ref()
            .expect("a frame")
            .image(1)
            .expect("image 1"),
    );
    // The bundle holds one reference and the worker's cache another.
    assert!(Arc::strong_count(&pixels) >= 2);
    drop(bundle);

    if let Ok(Some(handle)) = pane.session.begin_shutdown() {
        let _ = handle.wait();
    }
    drop(pane);

    assert_eq!(
        Arc::strong_count(&pixels),
        1,
        "the worker let go of its copy when the session ended"
    );
}

// ---- placement versus image ------------------------------------------------

/// A placement scrolled out of view stops being *visible*, but the image it
/// refers to has not gone anywhere. Conflating the two would either drop
/// pixels that are about to be needed again or keep pixels forever.
#[test]
fn scrolling_a_placement_out_of_view_does_not_delete_its_image() {
    let mut pane = GraphicsSession::start(GraphicsPolicy::default());
    let shown = place(&mut pane, 1, 16, 0x99);
    let placement = shown
        .graphics
        .as_ref()
        .expect("a frame")
        .placements
        .first()
        .copied()
        .expect("a placement");
    assert!(placement.visible, "it starts on screen");

    // Enough lines to push it well off the top of a 24-row screen.
    for _ in 0..3 {
        pane.feed("scroll\\nscroll\\nscroll\\nscroll\\nscroll\\nscroll\\nscroll\\nscroll\\nscroll\\nscroll\\n");
    }
    let scrolled = pane.feed("after\\n");

    let frame = scrolled
        .graphics
        .as_ref()
        .expect("the image is still held even though it is off screen");
    assert!(frame.image(1).is_some(), "the image survives scrolling");
    let placement = frame
        .placements
        .iter()
        .find(|placement| placement.image == 1)
        .expect("the placement is still known");
    assert!(
        !placement.visible,
        "but it is no longer on screen: {placement:?}"
    );
    assert!(pane_text(&scrolled).contains("after"));
}
