//! Configured colours, and the rule that a running program outranks them.
//!
//! A preference decides what a fresh shell looks like. A program that sets its
//! own colours — `vim`'s theme, a full-screen installer, anything that sends
//! OSC 10, 11, 12 or 4 — must win for as long as it is running, and what it
//! leaves behind when it resets must be the *preference* rather than
//! libghostty's built-in colour. That is the whole reason configured colours
//! are written into the terminal's default slot instead of applied by the
//! renderer.

mod support;

use std::ffi::OsString;
use std::sync::Arc;

use sprite_term::{ColorDefaults, Rgb, SessionConfig, SnapshotBundle, TerminalSession};

use support::{EventPump, SnapshotPump, pane_text};

fn args(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}

fn color(r: u8, g: u8, b: u8) -> Rgb {
    Rgb { r, g, b }
}

/// Runs one script under the given colours and returns the bundle once
/// `marker` is on screen.
fn shown(colors: ColorDefaults, script: &str, marker: &str) -> Arc<SnapshotBundle> {
    let mut config = SessionConfig::command("/bin/sh", args(&["-c", script]));
    config.colors = colors;
    let mut session = TerminalSession::spawn(config).expect("spawn session");
    let events = EventPump::new(session.take_event_stream().expect("take event stream"));
    let snapshots = SnapshotPump::new(session.take_snapshot_stream().expect("take snapshots"));
    events.expect_ready();
    snapshots.wait_for("the marker", |bundle| pane_text(bundle).contains(marker))
}

#[test]
fn configured_colours_are_what_a_pane_starts_with() {
    let colors = ColorDefaults {
        foreground: Some(color(0x11, 0x22, 0x33)),
        background: Some(color(0x44, 0x55, 0x66)),
        cursor: Some(color(0x77, 0x88, 0x99)),
        palette: vec![(1, color(0xaa, 0xbb, 0xcc))],
    };
    let bundle = shown(colors, "printf 'PLAIN\\n'; sleep 30", "PLAIN");

    assert_eq!(bundle.render.default_foreground, color(0x11, 0x22, 0x33));
    assert_eq!(bundle.render.default_background, color(0x44, 0x55, 0x66));
    assert_eq!(bundle.render.cursor_color, Some(color(0x77, 0x88, 0x99)));
    assert_eq!(bundle.render.palette[1], color(0xaa, 0xbb, 0xcc));
}

/// Sparse on purpose: changing one entry must not flatten the other 255.
#[test]
fn an_unlisted_palette_entry_keeps_the_colour_it_had() {
    let plain = shown(ColorDefaults::default(), "printf 'A\\n'; sleep 30", "A");
    let patched = shown(
        ColorDefaults {
            palette: vec![(1, color(0xaa, 0xbb, 0xcc))],
            ..ColorDefaults::default()
        },
        "printf 'A\\n'; sleep 30",
        "A",
    );

    assert_eq!(patched.render.palette[1], color(0xaa, 0xbb, 0xcc));
    assert_eq!(
        patched.render.palette[2], plain.render.palette[2],
        "entry 2 was not configured, so it is untouched"
    );
    assert_eq!(
        patched.render.default_foreground, plain.render.default_foreground,
        "and a palette preference is not a foreground preference"
    );
}

/// A cursor colour nobody asked for is `None`, not a colour Sprite invented —
/// that is what lets the renderer fall back to inverting the cell.
#[test]
fn an_unconfigured_cursor_has_no_colour_of_its_own() {
    let bundle = shown(ColorDefaults::default(), "printf 'A\\n'; sleep 30", "A");
    assert_eq!(bundle.render.cursor_color, None);
}

#[test]
fn a_program_that_sets_its_own_colours_wins() {
    let colors = ColorDefaults {
        foreground: Some(color(0x11, 0x22, 0x33)),
        background: Some(color(0x44, 0x55, 0x66)),
        cursor: Some(color(0x77, 0x88, 0x99)),
        palette: vec![(1, color(0xaa, 0xbb, 0xcc))],
    };
    // OSC 10, 11, 12 and 4: foreground, background, cursor, palette entry.
    let script = "printf '\\033]10;#010203\\007\\033]11;#040506\\007\
                  \\033]12;#070809\\007\\033]4;1;#0a0b0c\\007THEIRS\\n'; sleep 30";
    let bundle = shown(colors, script, "THEIRS");

    assert_eq!(bundle.render.default_foreground, color(0x01, 0x02, 0x03));
    assert_eq!(bundle.render.default_background, color(0x04, 0x05, 0x06));
    assert_eq!(bundle.render.cursor_color, Some(color(0x07, 0x08, 0x09)));
    assert_eq!(bundle.render.palette[1], color(0x0a, 0x0b, 0x0c));
}

/// The other half of the rule, and the reason the preference goes in the
/// default slot: a program that resets falls back to what was configured, not
/// to libghostty's built-in colour.
#[test]
fn a_reset_returns_to_the_configured_colour() {
    let configured = color(0x44, 0x55, 0x66);
    let colors = ColorDefaults {
        // Both, because libghostty reports the pair only when it knows both.
        foreground: Some(color(0x11, 0x22, 0x33)),
        background: Some(configured),
        palette: vec![(1, color(0xaa, 0xbb, 0xcc))],
        ..ColorDefaults::default()
    };
    // Set both, then reset both: OSC 111 and OSC 104.
    let script = "printf '\\033]11;#040506\\007\\033]4;1;#0a0b0c\\007'; \
                  printf '\\033]111\\007\\033]104;1\\007RESET\\n'; sleep 30";
    let bundle = shown(colors, script, "RESET");

    assert_eq!(
        bundle.render.default_background, configured,
        "a reset background is the configured one"
    );
    assert_eq!(
        bundle.render.palette[1],
        color(0xaa, 0xbb, 0xcc),
        "and a reset palette entry is the configured one"
    );
}
