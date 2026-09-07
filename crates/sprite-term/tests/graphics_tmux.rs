//! Images through tmux.
//!
//! tmux does not forward escape sequences it does not understand unless it is
//! told to, by its own documented `allow-passthrough` option. **Sprite does not
//! patch, override, or detect and work around tmux.** When passthrough is off
//! an image does not appear, and that is tmux behaving as documented rather
//! than a Sprite defect — so this file asserts both halves, and the "off" half
//! is as much a promise as the "on" half.
//!
//! Skipped with a clear message when tmux is absent, in the same shape as the
//! Croft gate: a machine without tmux should say so rather than fail.

mod support;

use std::ffi::OsString;
use std::time::{Duration, Instant};

use sprite_term::{
    GraphicsPolicy, GraphicsSnapshot, SessionConfig, TerminalCommand, TerminalEvent,
    TerminalSession,
};

use support::{EventPump, SnapshotPump, base64, pane_text};

/// Where tmux is, if this machine has it.
fn tmux() -> Option<String> {
    let output = std::process::Command::new("sh")
        .args(["-c", "command -v tmux"])
        .output()
        .ok()?;
    let path = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    (!path.is_empty()).then_some(path)
}

/// Wraps a sequence the way an application must for tmux to pass it through.
///
/// tmux's own format: the payload sits inside a DCS `tmux;` sequence with every
/// escape doubled, so tmux can tell the payload's terminators from its own.
fn tmux_wrapped(inner: &str) -> String {
    let doubled = inner.replace('\x1b', "\x1b\x1b");
    format!("\x1bPtmux;{doubled}\x1b\\")
}

/// A Kitty transmit-and-display sequence for a small solid image.
fn kitty_image(id: u32, size: u32) -> String {
    let pixels = vec![0x5a_u8; (size * size * 4) as usize];
    format!(
        "\x1b_Ga=T,f=32,s={size},v={size},i={id},q=2;{}\x1b\\",
        base64(&pixels)
    )
}

/// Runs tmux with passthrough on or off, showing it an image, and reports what
/// the pane ends up holding.
fn images_through_tmux(passthrough: bool) -> GraphicsSnapshot {
    let tmux = tmux().expect("checked by the caller");
    let directory =
        std::env::temp_dir().join(format!("sprite-tmux-{}-{passthrough}", std::process::id()));
    std::fs::create_dir_all(&directory).expect("a directory for the fixtures");

    // The image is `cat`ed from a file so the payload is never typed as a
    // command, where a shell would echo it as ordinary text.
    let image = directory.join("image.esc");
    std::fs::write(&image, tmux_wrapped(&kitty_image(1, 8))).expect("write the image");

    // A configuration file of our own, so this test says nothing about the
    // machine's tmux configuration and the machine's says nothing about it.
    let config = directory.join("tmux.conf");
    let setting = if passthrough { "on" } else { "off" };
    std::fs::write(&config, format!("set -g allow-passthrough {setting}\n"))
        .expect("write the configuration");

    let script = format!(
        "{tmux} -L sprite-test-{}-{passthrough} -f {} new-session -- sh -c 'cat {}; printf DONE; sleep 300'",
        std::process::id(),
        config.display(),
        image.display(),
    );
    let mut config_session = SessionConfig::command(
        "/bin/sh",
        vec![OsString::from("-c"), OsString::from(&script)],
    );
    config_session.graphics = GraphicsPolicy::default();

    let mut session = TerminalSession::spawn(config_session).expect("spawn session");
    let events = EventPump::new(session.take_event_stream().expect("take event stream"));
    let snapshots = SnapshotPump::new(session.take_snapshot_stream().expect("take snapshots"));
    events.expect_ready();

    // tmux takes a moment to start; the marker is how we know the inner shell
    // has run rather than how long we waited.
    snapshots.wait_for("tmux to run the command", |bundle| {
        pane_text(bundle).contains("DONE")
    });

    session
        .send(TerminalCommand::CaptureGraphics)
        .expect("request graphics");
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut answer = None;
    while Instant::now() < deadline && answer.is_none() {
        if let TerminalEvent::Graphics(graphics) = events.next() {
            answer = Some((*graphics).clone());
        }
    }

    // Leave no server behind on the machine running the tests.
    let _ = std::process::Command::new(&tmux)
        .args([
            "-L",
            &format!("sprite-test-{}-{passthrough}", std::process::id()),
            "kill-server",
        ])
        .output();
    let _ = std::fs::remove_dir_all(&directory);

    answer.expect("a graphics answer arrived")
}

#[test]
fn an_image_survives_tmux_when_passthrough_is_enabled() {
    let Some(_) = tmux() else {
        eprintln!("skipping: this machine has no tmux, so passthrough cannot be exercised");
        return;
    };

    let graphics = images_through_tmux(true);

    assert!(
        graphics.holds(1),
        "tmux forwarded the image with allow-passthrough on: {graphics:?}"
    );
}

/// With passthrough off the image does not arrive, and Sprite does nothing to
/// change that. Asserting it keeps anyone from later "fixing" tmux from inside
/// Sprite.
#[test]
fn an_image_does_not_survive_tmux_without_passthrough() {
    let Some(_) = tmux() else {
        eprintln!("skipping: this machine has no tmux, so passthrough cannot be exercised");
        return;
    };

    let graphics = images_through_tmux(false);

    assert!(
        !graphics.holds(1),
        "tmux withheld the sequence, which is its documented behaviour and not \
         something Sprite works around: {graphics:?}"
    );
}
