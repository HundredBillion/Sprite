//! Does one pane's graphics policy reach another pane?
//!
//! libghostty has one PNG decoder for the whole process, registered with
//! `ghostty_sys_set`. Clearing it for a disabled pane would clear it for every
//! pane, so Sprite registers one decoder and gives each pane its own behind it;
//! a pane that shows no images simply has none.
#![expect(
    clippy::expect_used,
    reason = "integration-test helpers run only under test, where a failed setup stops the test with its cause"
)]

mod support;

use sprite_term::{
    GraphicsPolicy, GraphicsSnapshot, TerminalCommand, TerminalEvent, TerminalSession,
};

use support::{EventPump, SnapshotPump, base64, kitty, png_bytes};

fn start(policy: GraphicsPolicy) -> (TerminalSession, EventPump, SnapshotPump) {
    support::repl_session(policy)
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
    support::repl_feed(
        &mut shown,
        &shown_snapshots,
        &kitty("a=T,f=100,i=1", &base64(&png)),
        1,
    );

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
