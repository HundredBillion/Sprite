//! The owned graphics projection: what a renderer receives, what it costs, and
//! what it is guaranteed to agree with.

mod support;

use std::ffi::OsString;
use std::sync::Arc;
use std::time::{Duration, Instant};

use sprite_term::{
    GraphicsPolicy, Layer, SessionConfig, SnapshotBundle, TerminalCommand, TerminalSession,
};

use support::{EventPump, SnapshotPump, base64, kitty, pane_text, png_bytes};

fn args(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}

fn session(policy: GraphicsPolicy) -> (TerminalSession, EventPump, SnapshotPump) {
    let mut config = SessionConfig::command("/bin/sh", args(&[]));
    config.graphics = policy;
    let mut session = TerminalSession::spawn(config).expect("spawn session");
    let events = EventPump::new(session.take_event_stream().expect("take event stream"));
    let snapshots = SnapshotPump::new(session.take_snapshot_stream().expect("take snapshots"));
    events.expect_ready();
    session
        .send(TerminalCommand::Input(b"printf 'READY\\n'\n".to_vec()))
        .expect("ask the shell to announce itself");
    snapshots.wait_for("the shell to start", |bundle| {
        pane_text(bundle).contains("READY")
    });
    (session, events, snapshots)
}

static NEXT_MARK: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Has the child print `escapes`, and returns the bundle that shows the result.
fn feed(
    session: &mut TerminalSession,
    snapshots: &SnapshotPump,
    escapes: String,
) -> Arc<SnapshotBundle> {
    session
        .send(TerminalCommand::Input(
            format!("printf '{escapes}'\n").into_bytes(),
        ))
        .expect("ask the child to print the payload");

    let mark = NEXT_MARK.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let marker = format!("MARK{mark}");
    session
        .send(TerminalCommand::Input(
            format!("printf '{marker}\\n'\n").into_bytes(),
        ))
        .expect("ask the child to print the marker");
    snapshots.wait_for("the payload to be processed", |bundle| {
        pane_text(bundle).contains(&marker)
    })
}

/// A pane showing no images must carry no graphics at all — not an empty frame,
/// which would still be an allocation on every capture.
#[test]
fn a_pane_with_no_images_carries_no_graphics() {
    let (mut session, _events, snapshots) = session(GraphicsPolicy::default());

    let bundle = feed(&mut session, &snapshots, "plain-text\\n".to_owned());

    assert!(
        bundle.graphics.is_none(),
        "text-only panes pay nothing for graphics"
    );
    assert!(pane_text(&bundle).contains("plain-text"));
}

#[test]
fn an_image_arrives_with_its_pixels_and_its_placement() {
    let (mut session, _events, snapshots) = session(GraphicsPolicy::default());

    let bundle = feed(
        &mut session,
        &snapshots,
        // Comfortably larger than one 8x16 cell: an image smaller than a cell
        // covers no whole cells, and this test is about geometry rather than
        // about that edge.
        kitty("a=T,f=100,i=1", &base64(&png_bytes(32, 32, 0x55))),
    );

    let graphics = bundle.graphics.as_ref().expect("a frame");
    let image = graphics.image(1).expect("the image");
    assert_eq!((image.width, image.height), (32, 32));
    assert_eq!(
        image.pixels.len(),
        32 * 32 * image.bytes_per_pixel(),
        "the pixels are owned and complete"
    );
    assert!(
        image.bytes_per_pixel() >= 3,
        "colour, not a palette index: {} bytes per pixel",
        image.bytes_per_pixel()
    );

    let placement = graphics
        .placements
        .iter()
        .find(|placement| placement.image == 1)
        .expect("the placement");
    assert!(!placement.is_virtual);
    assert!(placement.visible, "it is on screen");
    assert_eq!(
        placement.source.width, 32,
        "the whole image, since none was cropped"
    );
    assert_eq!(placement.source.height, 32);
    assert_eq!(
        (placement.pixel_width, placement.pixel_height),
        (32, 32),
        "drawn at its own size, since no scale was asked for"
    );
    // Cells are 8x16 here, so 32 pixels square covers four columns and two
    // rows. Grid geometry is computed from the cell metrics, which is why a
    // pane that never set them would report an image covering nothing.
    assert_eq!((placement.columns, placement.rows), (4, 2));
    // A placement with no `z` given has z-index 0, and Ghostty classifies
    // z >= 0 as above the text. Images therefore default to drawing *over*
    // text rather than under it, which is worth knowing before the renderer
    // decides its painting order.
    assert_eq!(placement.layer, Layer::AboveText);
}

/// The projection and the text come from one capture, so a renderer never draws
/// an image against a screen it never accompanied.
#[test]
fn graphics_and_rows_share_one_generation() {
    let (mut session, _events, snapshots) = session(GraphicsPolicy::default());

    let bundle = feed(
        &mut session,
        &snapshots,
        kitty("a=T,f=32,s=4,v=4,i=7", &base64(&[0x33_u8; 4 * 4 * 4])),
    );

    let graphics = bundle.graphics.as_ref().expect("a frame");
    assert_eq!(bundle.generation, bundle.render.generation);
    assert_eq!(bundle.generation, bundle.pane.generation);
    assert!(
        graphics.image(7).is_some(),
        "the image belongs to the same bundle as the rows"
    );
}

/// The claim this task exists to make: pixels are copied once per image
/// generation, not once per capture.
///
/// Proved by identity rather than by timing — every capture must hand back the
/// *same allocation*, which a copy would not.
#[test]
fn pixels_are_copied_once_per_generation_not_once_per_capture() {
    let (mut session, _events, snapshots) = session(GraphicsPolicy::default());

    let first = feed(
        &mut session,
        &snapshots,
        kitty("a=T,f=32,s=8,v=8,i=1", &base64(&[0x99_u8; 8 * 8 * 4])),
    );
    let original = Arc::clone(
        first
            .graphics
            .as_ref()
            .expect("a frame")
            .image(1)
            .expect("the image"),
    );

    // Several more captures, caused by ordinary text rather than by anything
    // to do with images.
    for _ in 0..3 {
        let bundle = feed(&mut session, &snapshots, "more-text\\n".to_owned());
        let again = bundle
            .graphics
            .as_ref()
            .expect("the image is still shown")
            .image(1)
            .expect("the image");
        assert!(
            Arc::ptr_eq(&original, again),
            "the very same allocation, so nothing was copied again"
        );
    }

    // Replacing the image's content under the same id is a new generation, and
    // that must be a new copy rather than the stale one.
    let replaced = feed(
        &mut session,
        &snapshots,
        kitty("a=T,f=32,s=8,v=8,i=1", &base64(&[0x11_u8; 8 * 8 * 4])),
    );
    let new = replaced
        .graphics
        .as_ref()
        .expect("a frame")
        .image(1)
        .expect("the image");
    assert!(
        !Arc::ptr_eq(&original, new),
        "new content is a new copy, not the old pixels under a new name"
    );
    assert_ne!(original.pixels, new.pixels, "and the pixels really differ");
}

/// Adding images must not make capturing *text* more expensive. Checkpoint 2
/// lost most of a 30% latency regression to a per-cell FFI call, so this
/// measures rather than assumes.
#[test]
fn a_pane_with_images_captures_text_as_fast_as_one_without() {
    fn measure(with_image: bool) -> Duration {
        let (mut session, _events, snapshots) = session(GraphicsPolicy::default());
        if with_image {
            feed(
                &mut session,
                &snapshots,
                kitty("a=T,f=32,s=32,v=32,i=1", &base64(&[0x22_u8; 32 * 32 * 4])),
            );
        }

        // Warm up, then time ordinary text captures.
        feed(&mut session, &snapshots, "warm\\n".to_owned());
        let started = Instant::now();
        for _ in 0..10 {
            feed(&mut session, &snapshots, "line-of-text\\n".to_owned());
        }
        started.elapsed()
    }

    let without = measure(false);
    let with = measure(true);

    // Generous, because this is a shell round trip rather than a micro
    // benchmark: what it catches is a per-cell cost, which would be far worse
    // than this bound.
    assert!(
        with < without * 3,
        "text capture with an image on screen took {with:?} against {without:?} \
         without one; graphics must not add work proportional to cells"
    );
}

/// A pane that had images and no longer shows any goes back to carrying none.
#[test]
fn deleting_the_last_image_returns_the_pane_to_carrying_nothing() {
    let (mut session, _events, snapshots) = session(GraphicsPolicy::default());

    let shown = feed(
        &mut session,
        &snapshots,
        kitty("a=T,f=32,s=4,v=4,i=1", &base64(&[0x40_u8; 4 * 4 * 4])),
    );
    assert!(shown.graphics.is_some());

    // `a=d,d=A` deletes every placement and its images.
    let cleared = feed(&mut session, &snapshots, kitty("a=d,d=A", ""));

    assert!(
        cleared.graphics.is_none(),
        "no placements means no frame, so nothing is held for a pane showing \
         no images: {:?}",
        cleared.graphics
    );
}
