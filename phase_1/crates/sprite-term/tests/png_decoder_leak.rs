//! Does one pane's graphics policy reach another pane?
//!
//! `set_png_decoder` keeps the decoder in thread-local storage but registers the
//! callback with `ghostty_sys_set`, which is a library-wide option. Clearing it
//! for a disabled pane therefore clears it for every pane in the process.

mod support;

use std::ffi::OsString;

use sprite_term::{
    GraphicsPolicy, GraphicsSnapshot, SessionConfig, TerminalCommand, TerminalEvent,
    TerminalSession,
};

use support::{EventPump, SnapshotPump, base64, kitty, pane_text, png_bytes};

fn start(policy: GraphicsPolicy) -> (TerminalSession, EventPump, SnapshotPump) {
    let mut config = SessionConfig::command("/bin/sh", Vec::<OsString>::new());
    config.graphics = policy;
    let mut session = TerminalSession::spawn(config).expect("spawn session");
    let events = EventPump::new(session.take_event_stream().expect("events"));
    let snapshots = SnapshotPump::new(session.take_snapshot_stream().expect("snapshots"));
    events.expect_ready();
    session
        .send(TerminalCommand::Input(b"printf 'READY\\n'\n".to_vec()))
        .expect("announce");
    snapshots.wait_for("the shell", |b| pane_text(b).contains("READY"));
    (session, events, snapshots)
}

fn probe(session: &mut TerminalSession, events: &EventPump) -> GraphicsSnapshot {
    session
        .send(TerminalCommand::CaptureGraphics)
        .expect("request graphics");
    loop {
        if let TerminalEvent::Graphics(g) = events.next() {
            return (*g).clone();
        }
    }
}

/// A second pane that shows no images must not stop the first one decoding PNGs.
#[test]
fn a_disabled_pane_does_not_disable_png_for_another_pane() {
    let (mut shown, shown_events, shown_snapshots) = start(GraphicsPolicy::default());

    // The disabled pane starts *after* the one that wants images, which is the
    // order that matters: its worker clears the decoder during its own setup.
    let (_hidden, _hidden_events, _hidden_snapshots) = start(GraphicsPolicy::disabled());

    let png = png_bytes(6, 5, 0x40);
    shown
        .send(TerminalCommand::Input(
            format!("printf '{}'\n", kitty("a=T,f=100,i=1", &base64(&png))).into_bytes(),
        ))
        .expect("payload");
    let (command, marker) = support::marker_command(1);
    shown.send(TerminalCommand::Input(command)).expect("marker");
    shown_snapshots.wait_for("the payload", |b| pane_text(b).contains(&marker));

    let graphics = probe(&mut shown, &shown_events);
    assert_eq!(
        graphics
            .images
            .iter()
            .find(|i| i.id == 1)
            .map(|i| (i.width, i.height)),
        Some((6, 5)),
        "the enabled pane still decodes PNG after a disabled pane started: {graphics:?}"
    );
}
