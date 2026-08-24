//! The protocol features the other graphics tests do not reach.
//!
//! Crop, scale, explicit placement identity, and the z bands, each asserted
//! against the owned projection rather than against anything a GPU did. The
//! rest of the fixture list lives with the behaviour it belongs to: transfer
//! and chunking in `graphics_transfer`, deletion and screen switches in
//! `graphics_identity`, limits in `graphics_policy`, texture reclamation in
//! `sprite-app`'s cache tests.

mod support;

use sprite_term::{GraphicsPolicy, Layer, Placement};

use support::{GraphicsSession, base64, kitty};

/// Transmits a solid image, with whatever extra control data a fixture needs.
fn place(pane: &mut GraphicsSession, control: &str, size: u32) -> Vec<Placement> {
    let pixels = vec![0x5a_u8; (size * size * 4) as usize];
    let bundle = pane.feed(&kitty(
        &format!("a=T,f=32,s={size},v={size},{control}"),
        &base64(&pixels),
    ));
    bundle
        .graphics
        .as_ref()
        .map(|frame| frame.placements.clone())
        .unwrap_or_default()
}

/// `x`, `y`, `w`, `h` select a region of the image; everything outside it is
/// not drawn.
#[test]
fn a_crop_selects_a_region_of_the_image() {
    let mut pane = GraphicsSession::start(GraphicsPolicy::default());

    let placements = place(&mut pane, "i=1,x=8,y=4,w=16,h=12", 32);
    let placement = placements.first().expect("a placement");

    assert_eq!(
        (
            placement.source.x,
            placement.source.y,
            placement.source.width,
            placement.source.height
        ),
        (8, 4, 16, 12),
        "the requested region, resolved by the terminal"
    );
}

/// A crop larger than the image is clamped rather than reaching outside it.
#[test]
fn a_crop_beyond_the_image_is_clamped_to_it() {
    let mut pane = GraphicsSession::start(GraphicsPolicy::default());

    let placements = place(&mut pane, "i=1,x=24,y=24,w=999,h=999", 32);
    let placement = placements.first().expect("a placement");

    assert!(
        placement.source.x + placement.source.width <= 32,
        "the region stays inside the image: {:?}",
        placement.source
    );
    assert!(placement.source.y + placement.source.height <= 32);
}

/// `c` and `r` ask for a size in cells, and the pixel size follows from it.
#[test]
fn a_scaled_placement_reports_the_size_it_will_be_drawn_at() {
    let mut pane = GraphicsSession::start(GraphicsPolicy::default());

    // Cells are 8x16 here, so six columns by three rows is 48x48 pixels — from
    // a 32x32 image, which is a scale up rather than a crop.
    let placements = place(&mut pane, "i=1,c=6,r=3", 32);
    let placement = placements.first().expect("a placement");

    assert_eq!((placement.columns, placement.rows), (6, 3));
    assert_eq!(
        (placement.pixel_width, placement.pixel_height),
        (48, 48),
        "the pixels it covers follow from the cells it was given"
    );
    assert_eq!(
        (placement.source.width, placement.source.height),
        (32, 32),
        "and the whole image is still the source: scaling is not cropping"
    );
}

/// An application that gives a placement its own identity gets it back, which
/// is how it addresses that placement later.
#[test]
fn an_explicit_placement_id_is_reported() {
    let mut pane = GraphicsSession::start(GraphicsPolicy::default());

    let placements = place(&mut pane, "i=7,p=42", 8);
    let placement = placements.first().expect("a placement");

    assert_eq!(placement.image, 7);
    assert_eq!(placement.placement, 42);
}

/// One image can be placed more than once, and the placements are distinct.
#[test]
fn one_image_can_hold_several_placements() {
    let mut pane = GraphicsSession::start(GraphicsPolicy::default());

    place(&mut pane, "i=1,p=1", 8);
    pane.feed("\\033[10;1H");
    let placements = place(&mut pane, "i=1,p=2", 8);

    let ids: Vec<u32> = placements
        .iter()
        .filter(|placement| placement.image == 1)
        .map(|placement| placement.placement)
        .collect();
    assert_eq!(
        ids,
        vec![1, 2],
        "two placements of one image: {placements:?}"
    );
}

/// The three z bands, as the projection reports them.
///
/// The boundaries are Ghostty's: below the background is far negative, below
/// the text is negative, and zero or above is over the text. A renderer paints
/// in this order, so a mistake here is a picture in the wrong place rather than
/// a missing one.
#[test]
fn z_order_lands_in_the_right_band() {
    let mut pane = GraphicsSession::start(GraphicsPolicy::default());

    for (z, expected) in [
        ("z=-2000000000", Layer::BelowBackground),
        ("z=-1", Layer::BelowText),
        ("z=0", Layer::AboveText),
        ("z=1", Layer::AboveText),
    ] {
        let mut pane_for_case = GraphicsSession::start(GraphicsPolicy::default());
        let placements = place(&mut pane_for_case, &format!("i=1,{z}"), 8);
        let placement = placements.first().expect("a placement");
        assert_eq!(placement.layer, expected, "{z} belongs in {expected:?}");
    }

    // And with no `z` at all, which is the case almost every application hits.
    let placements = place(&mut pane, "i=1", 8);
    assert_eq!(
        placements.first().expect("a placement").layer,
        Layer::AboveText,
        "no z given means z=0, which is above the text"
    );
}
