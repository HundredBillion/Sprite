//! The Croft Compatibility Gate.
//!
//! Croft is unmodified upstream software used as an external acceptance
//! application: if a real full-screen program behaves correctly inside a
//! Terminal Session, the seam is doing its job. Croft is never a Sprite runtime
//! dependency, and nothing here imports it or reaches into private
//! `sprite-term` types.
//!
//! Ignored by default because it needs an externally built binary. Run it
//! through `phase_1/scripts/test-croft-main.sh`, which supplies
//! `SPRITE_CROFT_BIN`.

mod support;

use std::ffi::OsString;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use sprite_term::{ScreenKind, SessionConfig, TerminalCommand, TerminalSession, TerminalSize};

use support::{EventPump, SnapshotPump, pane_text};

const MARKER: &str = "sprite-croft-marker";

fn unique_directory() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after epoch")
        .as_nanos();
    let directory =
        std::env::temp_dir().join(format!("sprite-croft-{}-{nanos}", std::process::id()));
    fs::create_dir_all(&directory).expect("create the fixture directory");
    directory
}

#[test]
#[ignore = "needs an externally built Croft; run scripts/test-croft-main.sh"]
fn croft_checkpoint_one_capabilities() {
    let binary = PathBuf::from(
        std::env::var_os("SPRITE_CROFT_BIN").expect("SPRITE_CROFT_BIN names the Croft binary"),
    );
    assert!(
        binary.is_absolute(),
        "SPRITE_CROFT_BIN must be an absolute path, got {}",
        binary.display()
    );

    let directory = unique_directory();
    let fixture = directory.join("fixture.txt");
    fs::write(&fixture, "sprite checkpoint one fixture\nsecond line\n").expect("write the fixture");

    // Croft is launched with exactly the terminal identity a Sprite login shell
    // hands its descendants, so this exercises the same environment a user's
    // program would actually see.
    let identity = SessionConfig::login_shell()
        .expect("resolve a login shell for its identity")
        .environment;

    let mut config = SessionConfig::command(
        &binary,
        vec![
            OsString::from("--open-file"),
            fixture.clone().into_os_string(),
            OsString::from("--zen"),
        ],
    );
    config.working_directory = Some(directory.clone());
    config.environment = identity;

    let mut session = TerminalSession::spawn(config).expect("spawn Croft");
    let events = EventPump::new(session.take_event_stream().expect("take event stream"));
    let snapshots = SnapshotPump::new(session.take_snapshot_stream().expect("take snapshots"));
    events.expect_ready();

    // A full-screen editor takes the alternate screen and draws something.
    let bundle = snapshots.wait_for("Croft's alternate screen", |bundle| {
        bundle.pane.screen == ScreenKind::Alternate && !pane_text(bundle).trim().is_empty()
    });
    assert_eq!(bundle.pane.screen, ScreenKind::Alternate);

    // Typed text reaches the application and comes back on screen.
    session
        .send(TerminalCommand::Input(MARKER.as_bytes().to_vec()))
        .expect("send the marker");
    snapshots.wait_for("the typed marker", |bundle| {
        pane_text(bundle).contains(MARKER)
    });

    // A resize is reflected in a newer, still-coherent snapshot.
    let resized = TerminalSize {
        rows: 40,
        cols: 100,
        cell_width_px: 8,
        cell_height_px: 16,
    };
    session
        .send(TerminalCommand::Resize(resized))
        .expect("send the resize");
    let after = snapshots.wait_for("the resized grid", |bundle| bundle.render.size == resized);
    assert_eq!(
        after.pane.size, resized,
        "both projections agree on the size"
    );
    assert_eq!(
        after.generation, after.render.generation,
        "the bundle stays coherent across a resize"
    );

    let handle = session
        .begin_shutdown()
        .expect("begin_shutdown succeeds")
        .expect("the first call owns the worker");
    handle.wait().expect("the worker joins within the watchdog");

    fs::remove_dir_all(&directory).expect("remove the fixture directory");
}
