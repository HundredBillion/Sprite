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
#![expect(
    clippy::expect_used,
    reason = "integration-test helpers run only under test, where a failed setup stops the test with its cause"
)]

mod support;

use std::ffi::OsString;
use std::time::{Duration, Instant};

use sprite_term::{
    GraphicsPolicy, GraphicsSnapshot, SessionConfig, TerminalCommand, TerminalEvent,
    TerminalSession,
};

use support::{EventPump, SnapshotPump, base64, pane_text, wait_until};

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

struct TmuxFixture<'a> {
    binary: &'a str,
    socket: &'a str,
    directory: &'a std::path::Path,
}

impl Drop for TmuxFixture<'_> {
    fn drop(&mut self) {
        std::process::Command::new(self.binary)
            .args(["-L", self.socket, "kill-server"])
            .output()
            .ok();
        std::fs::remove_dir_all(self.directory).ok();
    }
}

/// Runs tmux with passthrough on or off, showing it an image, and reports what
/// the pane ends up holding.
fn images_through_tmux(passthrough: bool, special_path: bool) -> GraphicsSnapshot {
    let tmux = tmux().expect("checked by the caller");
    let directory = std::env::temp_dir().join(format!(
        "sprite-tmux-{}-{passthrough}-{special_path}{}",
        std::process::id(),
        if special_path {
            " space ; ' $ ` (café)"
        } else {
            ""
        }
    ));
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

    let socket = format!(
        "sprite-test-{}-{passthrough}-{special_path}",
        std::process::id()
    );
    let _fixture = TmuxFixture {
        binary: &tmux,
        socket: &socket,
        directory: &directory,
    };
    // Start detached so the producer waits before tmux can forward the image.
    // Passthrough output sent without an attached client is lost, unlike ordinary text.
    let started = std::process::Command::new(&tmux)
        .args(["-L", &socket, "-f"])
        .arg(&config)
        .args([
            "new-session", "-d", "--", "sh", "-c",
            "printf WAITING; \"$2\" -L \"$3\" wait-for sprite-ready && cat \"$1\"; printf DONE; sleep 300",
            "sprite-image-fixture",
        ])
        .arg(&image)
        .arg(&tmux)
        .arg(&socket)
        .output()
        .expect("start the detached fixture");
    assert!(started.status.success(), "{started:?}");
    wait_until("the detached producer to start", || {
        let pane = std::process::Command::new(&tmux)
            .args(["-L", &socket, "capture-pane", "-p"])
            .output()
            .expect("inspect the detached pane");
        String::from_utf8_lossy(&pane.stdout)
            .contains("WAITING")
            .then_some(())
    });
    let mut config_session = SessionConfig::terminal_command(
        &tmux,
        vec![
            OsString::from("-L"),
            OsString::from(&socket),
            OsString::from("attach-session"),
        ],
    );
    config_session.graphics = GraphicsPolicy::default();

    let sprite_term::Spawned {
        mut session,
        events,
        snapshots,
    } = TerminalSession::spawn(config_session).expect("spawn session");
    let events = EventPump::new(events);
    let snapshots = SnapshotPump::new(snapshots);
    events.expect_ready();

    // The attachment hook runs before tmux finishes its initial redraw.
    // Passthrough is discarded during a pending redraw, so wait for visible output.
    snapshots.wait_for("tmux to draw the attached pane", |bundle| {
        pane_text(bundle).contains("WAITING")
    });
    let released = std::process::Command::new(&tmux)
        .args(["-L", &socket, "wait-for", "-S", "sprite-ready"])
        .output()
        .expect("release the image producer after the initial redraw");
    assert!(released.status.success(), "{released:?}");

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

    answer.expect("a graphics answer arrived")
}

#[test]
fn an_image_survives_tmux_when_passthrough_is_enabled() {
    let Some(_) = tmux() else {
        eprintln!("skipping: this machine has no tmux, so passthrough cannot be exercised");
        return;
    };

    let graphics = images_through_tmux(true, false);

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

    let graphics = images_through_tmux(false, false);

    assert!(
        !graphics.holds(1),
        "tmux withheld the sequence, which is its documented behaviour and not \
         something Sprite works around: {graphics:?}"
    );
}

#[test]
fn an_image_survives_tmux_with_shell_metacharacters_in_fixture_paths() {
    let Some(_) = tmux() else {
        eprintln!("skipping: this machine has no tmux, so passthrough cannot be exercised");
        return;
    };
    let graphics = images_through_tmux(true, true);
    assert!(graphics.holds(1), "tmux forwarded the image: {graphics:?}");
}
