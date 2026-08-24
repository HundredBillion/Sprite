//! The 256-colour palette, which is how a terminal is actually coloured.
//!
//! A cell's colour is usually an *index*: `\x1b[31m` is entry 1, not red. A
//! snapshot that carries the index without the palette leaves a renderer able
//! to draw only the default foreground — which is how `ls --color`, git diffs
//! and shell prompts all come out the same shade.

mod support;

use std::ffi::OsString;

use sprite_term::{SessionConfig, SnapshotColor, TerminalSession};

use support::{EventPump, SnapshotPump, pane_text};

fn args(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}

/// Runs one script and returns the bundle once `marker` is on screen.
fn shown(script: &str, marker: &str) -> std::sync::Arc<sprite_term::SnapshotBundle> {
    let config = SessionConfig::command("/bin/sh", args(&["-c", script]));
    let mut session = TerminalSession::spawn(config).expect("spawn session");
    let events = EventPump::new(session.take_event_stream().expect("take event stream"));
    let snapshots = SnapshotPump::new(session.take_snapshot_stream().expect("take snapshots"));
    events.expect_ready();
    snapshots.wait_for("the coloured text", |bundle| {
        pane_text(bundle).contains(marker)
    })
}

/// The style of the first cell of the row containing `needle`.
fn first_cell_style(bundle: &sprite_term::SnapshotBundle, needle: &str) -> sprite_term::CellStyle {
    let row = bundle
        .render
        .rows
        .iter()
        .zip(bundle.pane.rows.iter())
        .find(|(_, pane)| pane.text.contains(needle))
        .map(|(render, _)| render)
        .expect("the row is on screen");
    row.cells
        .iter()
        .find(|cell| !cell.text.trim().is_empty())
        .expect("a cell with text")
        .style
}

#[test]
fn the_snapshot_carries_a_palette() {
    let bundle = shown("printf 'plain\\n'; sleep 30", "plain");

    // Entry 1 is red and entry 2 is green in every standard palette, so a
    // palette that is present and sane is one where they differ from each other
    // and from the default foreground.
    let palette = &bundle.render.palette;
    assert_ne!(
        palette[1], palette[2],
        "red and green are different colours"
    );
    assert!(palette[1].r > palette[1].g, "entry 1 is reddish");
    assert!(palette[2].g > palette[2].r, "entry 2 is greenish");
    assert_ne!(
        palette[1], bundle.render.default_foreground,
        "and none of them is simply the default"
    );
}

#[test]
fn a_foreground_colour_is_reported_as_a_palette_index() {
    let bundle = shown("printf '\\033[31mRED\\033[0m\\n'; sleep 30", "RED");

    assert_eq!(
        first_cell_style(&bundle, "RED").foreground,
        SnapshotColor::Palette(1),
        "an SGR colour is an index into the palette, not an RGB value"
    );
}

#[test]
fn every_standard_colour_is_a_distinct_index() {
    let script = (0..8)
        .map(|index| format!("printf '\\033[3{index}mC{index}\\033[0m '"))
        .collect::<Vec<_>>()
        .join("; ");
    let bundle = shown(&format!("{script}; printf 'END\\n'; sleep 30"), "END");

    let row = bundle
        .render
        .rows
        .iter()
        .zip(bundle.pane.rows.iter())
        .find(|(_, pane)| pane.text.contains("C0"))
        .map(|(render, _)| render)
        .expect("the row is on screen");

    let indices: Vec<u8> = row
        .cells
        .iter()
        .filter_map(|cell| match cell.style.foreground {
            SnapshotColor::Palette(index) => Some(index),
            _ => None,
        })
        .collect();
    for index in 0..8_u8 {
        assert!(
            indices.contains(&index),
            "colour {index} is on the row: {indices:?}"
        );
    }
}

#[test]
fn a_bright_colour_and_a_background_colour_are_also_indices() {
    let bundle = shown(
        "printf '\\033[91mBRIGHT\\033[0m \\033[44mONBLUE\\033[0m\\n'; sleep 30",
        "BRIGHT",
    );

    assert_eq!(
        first_cell_style(&bundle, "BRIGHT").foreground,
        SnapshotColor::Palette(9),
        "bright red is entry 9"
    );

    let row = bundle
        .render
        .rows
        .iter()
        .zip(bundle.pane.rows.iter())
        .find(|(_, pane)| pane.text.contains("ONBLUE"))
        .map(|(render, _)| render)
        .expect("the row is on screen");
    assert!(
        row.cells
            .iter()
            .any(|cell| cell.style.background == SnapshotColor::Palette(4)),
        "a background colour is an index too"
    );
}

/// An indexed colour beyond the sixteen standard ones, which is what 256-colour
/// applications use.
#[test]
fn an_extended_index_is_carried_and_has_a_colour() {
    let bundle = shown(
        "printf '\\033[38;5;208mORANGE\\033[0m\\n'; sleep 30",
        "ORANGE",
    );

    assert_eq!(
        first_cell_style(&bundle, "ORANGE").foreground,
        SnapshotColor::Palette(208)
    );
    let colour = bundle.render.palette[208];
    assert!(
        colour.r > colour.g && colour.g > colour.b,
        "entry 208 is an orange: {colour:?}"
    );
}
