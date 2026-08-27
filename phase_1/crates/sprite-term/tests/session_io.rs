//! Ordered input into the PTY, state-aware key encoding, and terminal-generated
//! replies travelling back out.

mod support;

use std::ffi::OsString;

use sprite_term::{
    KeyAction, KeyEvent, KeyModifiers, SessionConfig, TerminalCommand, TerminalSession,
};

use support::{EventPump, SnapshotPump, pane_text};

fn args(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}

fn plain(logical_key: &str, text: Option<&str>) -> TerminalCommand {
    TerminalCommand::Key(KeyEvent {
        logical_key: logical_key.to_owned(),
        text: text.map(str::to_owned),
        modifiers: KeyModifiers {
            shift: false,
            alt: false,
            control: false,
            platform: false,
            function: false,
        },
        action: KeyAction::Press,
        composing: false,
    })
}

fn session(script: &str) -> TerminalSession {
    TerminalSession::spawn(SessionConfig::command("/bin/sh", args(&["-c", script])))
        .expect("spawn session")
}

#[test]
fn key_events_reach_the_child_in_order() {
    let mut session = session("read line; printf 'got:%s\\n' \"$line\"");
    let events = EventPump::new(session.take_event_stream().expect("take event stream"));
    let snapshots = SnapshotPump::new(session.take_snapshot_stream().expect("take snapshots"));
    events.expect_ready();

    for letter in ["s", "p", "r", "i", "t", "e"] {
        session
            .send(plain(letter, Some(letter)))
            .expect("send letter");
    }
    session.send(plain("enter", None)).expect("send enter");

    let bundle = snapshots.wait_for("the child's echo of our keys", |bundle| {
        pane_text(bundle).contains("got:sprite")
    });
    assert!(pane_text(&bundle).contains("got:sprite"));
}

/// The same logical key encodes differently once the child turns on
/// cursor-application mode, which proves the encoder reads live terminal state
/// rather than a fixed table.
#[test]
fn arrow_up_follows_cursor_key_mode() {
    let normal = arrow_up_encoding("");
    assert!(
        normal.contains("1b 5b 41"),
        "normal mode encodes ArrowUp as CSI A, got: {normal}"
    );

    let application = arrow_up_encoding(r"printf '\033[?1h';");
    assert!(
        application.contains("1b 4f 41"),
        "application mode encodes ArrowUp as SS3 A, got: {application}"
    );
}

fn arrow_up_encoding(set_mode: &str) -> String {
    let script = format!(
        "{set_mode} stty -icanon -echo min 3 time 0; printf 'READY\\n'; head -c 3 | od -An -tx1"
    );
    let mut session = session(&script);
    let events = EventPump::new(session.take_event_stream().expect("take event stream"));
    let snapshots = SnapshotPump::new(session.take_snapshot_stream().expect("take snapshots"));
    events.expect_ready();

    // The mode change only counts once the terminal has parsed it, so wait for
    // the marker the child prints afterwards.
    snapshots.wait_for("the child's ready marker", |bundle| {
        pane_text(bundle).contains("READY")
    });

    session.send(plain("up", None)).expect("send arrow up");

    let bundle = snapshots.wait_for("the decoded key bytes", |bundle| {
        let text = pane_text(bundle);
        text.contains("1b 5b 41") || text.contains("1b 4f 41")
    });
    pane_text(&bundle)
}

/// A device status report is answered by the terminal itself, not the child.
/// The reply is read back as hex so the parser cannot consume its own answer.
#[test]
fn terminal_answers_device_status_report() {
    let mut session =
        session("stty -icanon -echo min 4 time 0; printf '\\033[5n'; head -c 4 | od -An -tx1");
    let events = EventPump::new(session.take_event_stream().expect("take event stream"));
    let snapshots = SnapshotPump::new(session.take_snapshot_stream().expect("take snapshots"));
    events.expect_ready();

    let bundle = snapshots.wait_for("the terminal's own reply", |bundle| {
        pane_text(bundle).contains("1b 5b 30 6e")
    });
    assert!(
        pane_text(&bundle).contains("1b 5b 30 6e"),
        "the terminal replies CSI 0 n through the PTY"
    );
}

#[test]
fn oversized_input_is_rejected_and_the_session_survives() {
    let mut session = session("read line; printf 'got:%s\\n' \"$line\"");
    let events = EventPump::new(session.take_event_stream().expect("take event stream"));
    let snapshots = SnapshotPump::new(session.take_snapshot_stream().expect("take snapshots"));
    events.expect_ready();

    let too_big = vec![b'x'; 16 * 1024 + 1];
    let error = session
        .send(TerminalCommand::Input(too_big))
        .expect_err("an oversized payload is refused");
    assert_eq!(error.operation, "send");

    // The refusal is not fatal: the same session still accepts real input.
    session
        .send(TerminalCommand::Input(b"sprite\n".to_vec()))
        .expect("the session still works");

    let bundle = snapshots.wait_for("input after the rejection", |bundle| {
        pane_text(bundle).contains("got:sprite")
    });
    assert!(pane_text(&bundle).contains("got:sprite"));
}

/// Sustained output must not starve input: the permit bound reserves worker
/// queue space for commands no matter how loud the child is.
#[test]
fn input_survives_sustained_output() {
    let mut session = session(
        "stty -echo; yes sprite-flood-line & producer=$!; \
         read line; kill $producer 2>/dev/null; wait $producer 2>/dev/null; \
         printf '\\nMARKER:%s\\n' \"$line\"",
    );
    let events = EventPump::new(session.take_event_stream().expect("take event stream"));
    let snapshots = SnapshotPump::new(session.take_snapshot_stream().expect("take snapshots"));
    events.expect_ready();

    // Only send once the flood is genuinely under way.
    snapshots.wait_for("sustained output", |bundle| {
        pane_text(bundle).contains("sprite-flood-line")
    });

    session
        .send(TerminalCommand::Input(b"ping\n".to_vec()))
        .expect("input is accepted mid-flood");

    let bundle = snapshots.wait_for("the marker echoed back", |bundle| {
        pane_text(bundle).contains("MARKER:ping")
    });
    assert!(pane_text(&bundle).contains("MARKER:ping"));
}

/// One resize reaches both backends: the kernel's window size (which the child
/// reads with `stty size`) and the terminal grid the snapshot describes.
#[test]
fn resize_updates_pty_and_snapshot() {
    let mut session = session("stty -echo; while read line; do stty size; done");
    let events = EventPump::new(session.take_event_stream().expect("take event stream"));
    let snapshots = SnapshotPump::new(session.take_snapshot_stream().expect("take snapshots"));
    events.expect_ready();

    let resized = sprite_term::TerminalSize {
        rows: 40,
        cols: 100,
        cell_width_px: 9,
        cell_height_px: 18,
    };
    session
        .send(TerminalCommand::Resize(resized))
        .expect("send resize");
    session
        .send(TerminalCommand::Input(b"\n".to_vec()))
        .expect("ask the child for its size");

    let bundle = snapshots.wait_for("the child's view of the new size", |bundle| {
        pane_text(bundle).contains("40 100")
    });

    assert_eq!(
        bundle.render.size, resized,
        "the render projection reports the new size"
    );
    assert_eq!(
        bundle.pane.size, resized,
        "the pane projection reports the same size"
    );
    assert_eq!(bundle.render.rows.len(), 40, "the grid actually grew");
}

#[test]
fn degenerate_and_oversized_resizes_are_refused() {
    let mut session = session("sleep 30");
    let events = EventPump::new(session.take_event_stream().expect("take event stream"));
    events.expect_ready();

    let base = sprite_term::TerminalSize::DEFAULT;

    for (label, size) in [
        ("zero rows", sprite_term::TerminalSize { rows: 0, ..base }),
        (
            "zero columns",
            sprite_term::TerminalSize { cols: 0, ..base },
        ),
        (
            "zero cell width",
            sprite_term::TerminalSize {
                cell_width_px: 0,
                ..base
            },
        ),
        (
            "too many cells",
            sprite_term::TerminalSize {
                rows: 1000,
                cols: 1001,
                ..base
            },
        ),
    ] {
        let error = session
            .send(TerminalCommand::Resize(size))
            .expect_err(label);
        assert_eq!(error.operation, "resize", "{label} is refused at the seam");
    }

    // The exact acceptance boundary is one million cells.
    session
        .send(TerminalCommand::Resize(sprite_term::TerminalSize {
            rows: 1000,
            cols: 1000,
            ..base
        }))
        .expect("a one-million-cell grid is allowed");
}

/// The Kitty keyboard protocol changes how the same keystroke encodes. The
/// encoder refreshes from terminal state before every encode, so this follows
/// the child's negotiation with no application involvement at all.
#[test]
fn kitty_keyboard_flags_change_the_encoding() {
    fn encoding_of(setup: &str, bytes: usize, marker: &str) -> String {
        let script = format!(
            "stty -icanon -icrnl -echo min {bytes} time 0; {setup} printf 'READY\\n'; \
             head -c {bytes} | od -An -tx1"
        );
        let mut session = session(&script);
        let events = EventPump::new(session.take_event_stream().expect("take event stream"));
        let snapshots = SnapshotPump::new(session.take_snapshot_stream().expect("take snapshots"));
        events.expect_ready();
        snapshots.wait_for("ready", |b| pane_text(b).contains("READY"));

        session.send(plain("a", Some("a"))).expect("send a key");

        let bundle = snapshots.wait_for("the encoded key", |b| pane_text(b).contains(marker));
        pane_text(&bundle)
    }

    // Without negotiation, `a` is the bare byte 0x61.
    let legacy = encoding_of("", 1, "61");
    assert!(legacy.contains("61"), "legacy is one byte, got:\n{legacy}");

    // Flag 8 asks for every key as an escape code, so the same key becomes
    // CSI 97 u — 1b 5b 39 37 75.
    let kitty = encoding_of("printf '\\033[>8u';", 5, "1b 5b");
    assert!(
        kitty.contains("1b 5b 39 37 75"),
        "the Kitty protocol encodes `a` as CSI 97 u, got:\n{kitty}"
    );
}

/// Text committed by an input method is typed, not pasted: it reaches the child
/// verbatim and carries no bracketing.
#[test]
fn committed_input_method_text_reaches_the_child() {
    let mut session = session("stty -echo; read line; printf 'got:%s\\n' \"$line\"");
    let events = EventPump::new(session.take_event_stream().expect("take event stream"));
    let snapshots = SnapshotPump::new(session.take_snapshot_stream().expect("take snapshots"));
    events.expect_ready();

    session
        .send(TerminalCommand::CommitText("日本語".to_owned()))
        .expect("commit composed text");
    session
        .send(TerminalCommand::Input(b"\n".to_vec()))
        .expect("end the line");

    let bundle = snapshots.wait_for("the committed text", |bundle| {
        pane_text(bundle).contains("got:日本語")
    });
    assert!(pane_text(&bundle).contains("got:日本語"));
}

/// A commit returns the reader to live output, because it is typing.
#[test]
fn committing_returns_the_viewport_to_live_output() {
    let mut session = session("stty -echo; seq 1 200; cat");
    let events = EventPump::new(session.take_event_stream().expect("take event stream"));
    let snapshots = SnapshotPump::new(session.take_snapshot_stream().expect("take snapshots"));
    events.expect_ready();
    snapshots.wait_for("the output", |b| pane_text(b).contains("200"));

    session
        .send(TerminalCommand::Scroll(sprite_term::Scroll::Top))
        .expect("scroll into history");
    snapshots.wait_for("history", |b| !b.render.viewport.at_bottom());

    session
        .send(TerminalCommand::CommitText("x".to_owned()))
        .expect("commit");

    let back = snapshots.wait_for("the live bottom", |b| b.render.viewport.at_bottom());
    assert!(back.render.viewport.at_bottom());
}
