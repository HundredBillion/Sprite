//! What a pane will and will not accept in the way of images.
//!
//! Every test here is about a limit on untrusted input. Image data arrives as
//! escape-sequence bytes from an arbitrary child, so "a program printed
//! something" must not become "the terminal read a file nobody named" or "the
//! terminal held a gigabyte".

mod support;

use std::ffi::OsString;
use std::time::{Duration, Instant};

use sprite_term::{
    GraphicsPolicy, GraphicsSnapshot, SessionConfig, TerminalCommand, TerminalEvent,
    TerminalSession,
};

use support::{EventPump, SnapshotPump, pane_text};

fn args(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}

/// Waits for the answer to one graphics probe, ignoring unrelated events.
fn wait_for_graphics(events: &EventPump) -> GraphicsSnapshot {
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        match events.next() {
            TerminalEvent::Graphics(graphics) => return (*graphics).clone(),
            TerminalEvent::Error(error) => panic!("graphics probe failed: {error}"),
            _ => {}
        }
    }
    panic!("watchdog: no graphics answer arrived");
}

/// A Kitty transmit-and-display sequence for raw RGBA pixels.
///
/// Raw rather than PNG on purpose: it needs no decoder, so these tests measure
/// the *policy* rather than whatever decoder happens to be installed.
///
/// `q=2` silences the protocol's reply. The terminal answers a transmission by
/// writing back to the child — a refusal arrives as `EINVAL: unsupported
/// medium` — which is correct Kitty behaviour, but these tests drive a shell,
/// and a shell reading a protocol reply on its standard input tries to run it
/// as a command and mangles whatever was typed next.
fn transmit_rgba(id: u32, width: u32, height: u32) -> String {
    let pixels = vec![0xa0_u8; (width * height * 4) as usize];
    let payload = base64(&pixels);
    format!("\\033_Ga=T,f=32,s={width},v={height},i={id},q=2;{payload}\\033\\\\")
}

/// A transmission that asks the terminal to read the image out of the
/// filesystem, by one of the three mediums that can do so.
///
/// `medium` is Kitty's `t=` parameter: `f` a named file, `t` a temporary file,
/// `s` shared memory.
fn transmit_from_path(id: u32, medium: char, path: &str) -> String {
    let payload = base64(path.as_bytes());
    format!("\\033_Ga=T,t={medium},f=32,s=1,v=1,i={id},q=2;{payload}\\033\\\\")
}

fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(ALPHABET[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// A shell that reads commands, so the tests can make the *child* print.
///
/// The distinction matters: `TerminalCommand::Input` writes to the child's
/// standard input, where an escape sequence is just bytes the shell may echo.
/// Only what a child writes to its output reaches the terminal's parser, so an
/// image has to be printed by the child to exist at all.
fn session(policy: GraphicsPolicy) -> (TerminalSession, EventPump, SnapshotPump) {
    let mut config = SessionConfig::command("/bin/sh", args(&[]));
    config.graphics = policy;
    let mut session = TerminalSession::spawn(config).expect("spawn session");
    let events = EventPump::new(session.take_event_stream().expect("take event stream"));
    let snapshots = SnapshotPump::new(session.take_snapshot_stream().expect("take snapshots"));
    events.expect_ready();
    session
        // `\\n` so printf receives the two characters backslash-n. A real
        // newline inside the quotes would leave the shell waiting for the rest
        // of an unterminated string.
        .send(TerminalCommand::Input(b"printf 'READY\\n'\n".to_vec()))
        .expect("ask the shell to announce itself");
    snapshots.wait_for("the shell to start", |bundle| {
        pane_text(bundle).contains("READY")
    });
    (session, events, snapshots)
}

/// Distinguishes one marker from the last, since the screen still holds them.
static NEXT_MARK: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Has the child print `escapes`, then waits until the terminal has read them.
///
/// The marker is printed by a second command, so seeing it means the shell has
/// finished the first — which is a fact about the terminal's parser rather than
/// a guess about timing.
fn feed(
    session: &mut TerminalSession,
    snapshots: &SnapshotPump,
    escapes: String,
) -> std::sync::Arc<sprite_term::SnapshotBundle> {
    // Sent verbatim: the sequence is already written as printf escapes, and
    // doubling the backslashes here would make printf emit the text `\033`
    // rather than an escape character.
    session
        .send(TerminalCommand::Input(
            format!("printf '{escapes}'\n").into_bytes(),
        ))
        .expect("ask the child to print the payload");

    let mark = NEXT_MARK.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let (command, marker) = support::marker_command(mark);
    session
        .send(TerminalCommand::Input(command))
        .expect("ask the child to print the marker");
    snapshots.wait_for("the payload to be processed", |bundle| {
        pane_text(bundle).contains(&marker)
    })
}

fn probe(session: &mut TerminalSession, events: &EventPump) -> GraphicsSnapshot {
    session
        .send(TerminalCommand::CaptureGraphics)
        .expect("request graphics");
    wait_for_graphics(events)
}

#[test]
fn a_raw_image_is_accepted_and_held() {
    let (mut session, events, snapshots) = session(GraphicsPolicy::default());

    feed(&mut session, &snapshots, transmit_rgba(1, 4, 4));
    let graphics = probe(&mut session, &events);

    assert!(
        graphics.holds(1),
        "the pane holds the image it was sent: {graphics:?}"
    );
    let image = graphics.images.iter().find(|i| i.id == 1).expect("image 1");
    assert_eq!((image.width, image.height), (4, 4));
    assert!(image.byte_len >= 4 * 4 * 4, "the pixels are held");
    assert_eq!(graphics.placements.len(), 1, "and it is placed once");
}

/// The rule that matters most here: a transmission naming a path must not make
/// the terminal read that path.
///
/// The proof is that a readable file and a path that does not exist produce
/// *identical* outcomes. If the path were ever consulted, a real file would
/// behave differently from a missing one.
#[test]
fn an_image_is_never_read_from_a_file() {
    let directory = std::env::temp_dir().join(format!("sprite-graphics-{}", std::process::id()));
    std::fs::create_dir_all(&directory).expect("a directory for the fixture");
    let readable = directory.join("readable.rgba");
    std::fs::write(&readable, vec![0x7f_u8; 4 * 4 * 4]).expect("write the fixture");
    let missing = directory.join("does-not-exist.rgba");

    let (mut session, events, snapshots) = session(GraphicsPolicy::default());

    feed(
        &mut session,
        &snapshots,
        transmit_from_path(10, 'f', &readable.to_string_lossy()),
    );
    let with_file = probe(&mut session, &events);

    feed(
        &mut session,
        &snapshots,
        transmit_from_path(11, 'f', &missing.to_string_lossy()),
    );
    let without_file = probe(&mut session, &events);

    assert!(
        !with_file.holds(10),
        "a file transmission is refused: {with_file:?}"
    );
    assert!(!without_file.holds(11));
    assert_eq!(
        with_file.images, without_file.images,
        "a readable file and a missing one are indistinguishable, which is what \
         shows the path was never consulted"
    );

    // The pane is still working afterwards: a refusal is not a wedge.
    feed(&mut session, &snapshots, transmit_rgba(12, 2, 2));
    assert!(probe(&mut session, &events).holds(12));

    let _ = std::fs::remove_dir_all(&directory);
}

/// None of the three filesystem mediums may turn terminal output into a file
/// read. Two are denied explicitly; the temporary-file medium cannot be, because
/// the binding's setter for it aborts the process — so all three are asserted
/// here by behaviour, which is the property that actually matters.
#[test]
fn no_filesystem_medium_can_load_an_image() {
    let directory = std::env::temp_dir().join(format!("sprite-mediums-{}", std::process::id()));
    std::fs::create_dir_all(&directory).expect("a directory for the fixture");
    let fixture = directory.join("payload.rgba");
    std::fs::write(&fixture, vec![0x11_u8; 4 * 4 * 4]).expect("write the fixture");
    let path = fixture.to_string_lossy().into_owned();

    let (mut session, events, snapshots) = session(GraphicsPolicy::default());

    for (id, medium, name) in [
        (20, 'f', "a named file"),
        (21, 't', "a temporary file"),
        (22, 's', "shared memory"),
    ] {
        feed(
            &mut session,
            &snapshots,
            transmit_from_path(id, medium, &path),
        );
        let graphics = probe(&mut session, &events);
        assert!(
            !graphics.holds(id),
            "an image was loaded from {name}, which turns printing into a file read: {graphics:?}"
        );
    }

    // Direct transmission still works, so this is a refusal of the medium
    // rather than of images.
    feed(&mut session, &snapshots, transmit_rgba(23, 2, 2));
    assert!(probe(&mut session, &events).holds(23));

    let _ = std::fs::remove_dir_all(&directory);
}

/// A pane with graphics turned off holds nothing at all — the image is dropped
/// as it arrives rather than accumulated and then ignored.
#[test]
fn a_pane_with_graphics_disabled_holds_no_images() {
    let (mut session, events, snapshots) = session(GraphicsPolicy::disabled());

    feed(&mut session, &snapshots, transmit_rgba(1, 8, 8));
    let graphics = probe(&mut session, &events);

    assert!(
        graphics.images.is_empty(),
        "nothing was stored: {graphics:?}"
    );
    assert_eq!(graphics.stored_bytes(), 0);

    // Text still works: disabling images disables images, not the terminal.
    //
    // The bundle `feed` already waited for is the one to assert on. Waiting
    // again would require a *further* snapshot, and once the screen has settled
    // there is no reason for one to arrive — which made this test fail on a
    // loaded machine and pass on an idle one.
    let bundle = feed(&mut session, &snapshots, "still-here\\n".to_owned());
    assert!(pane_text(&bundle).contains("still-here"));
}

/// The configured storage limit is what is enforced, not the library's default.
///
/// Proved by comparison rather than by assuming how eviction orders itself: the
/// same image goes to two panes that differ only in their limit. Asserting
/// which image survives would be a test of Ghostty's eviction policy, which is
/// not Sprite's to promise.
#[test]
fn the_storage_limit_is_the_one_configured() {
    let generous = GraphicsPolicy {
        storage_bytes: 1024 * 1024,
        ..GraphicsPolicy::default()
    };
    let (mut roomy, roomy_events, roomy_snapshots) = session(generous);
    feed(&mut roomy, &roomy_snapshots, transmit_rgba(1, 32, 32));
    let held = probe(&mut roomy, &roomy_events);
    assert!(
        held.holds(1),
        "a pane with room for the image keeps it: {held:?}"
    );

    let tight = GraphicsPolicy {
        storage_bytes: 1024,
        ..GraphicsPolicy::default()
    };
    let (mut small, small_events, small_snapshots) = session(tight);
    feed(&mut small, &small_snapshots, transmit_rgba(1, 32, 32));
    let refused = probe(&mut small, &small_events);
    assert!(
        !refused.holds(1),
        "the very same image does not fit a smaller limit, so what is enforced \
         is the configured value rather than a default: {refused:?}"
    );
    assert!(refused.stored_bytes() <= 1024);
}

/// An oversized payload is refused rather than accumulated, and the terminal
/// keeps working afterwards.
#[test]
fn a_payload_beyond_the_bound_is_refused_without_wedging_the_pane() {
    let policy = GraphicsPolicy {
        apc_max_bytes: 4 * 1024,
        ..GraphicsPolicy::default()
    };
    let (mut session, events, snapshots) = session(policy);

    // A 32x32 image is about 5.5 KiB of base64, comfortably past the 4 KiB
    // bound and comfortably inside what one write may carry.
    feed(&mut session, &snapshots, transmit_rgba(1, 32, 32));
    let graphics = probe(&mut session, &events);
    assert!(
        !graphics.holds(1),
        "a payload past the bound is refused: {graphics:?}"
    );

    // Under the bound, the same pane still accepts an image.
    feed(&mut session, &snapshots, transmit_rgba(2, 8, 8));
    assert!(
        probe(&mut session, &events).holds(2),
        "the bound refuses one payload, it does not disable the pane"
    );
}

/// An image too large for a pane's limits leaves no image and no damage.
///
/// **A wart worth knowing about.** When a transmission exceeds either bound —
/// the storage limit or the APC byte cap — the pinned Ghostty abandons the
/// escape sequence and prints the rest of it as ordinary text, so a refused
/// image can spray thousands of characters of base64 across the screen. That is
/// upstream behaviour rather than something Sprite does, and it is why the
/// default limits are generous enough that ordinary images never reach it. This
/// test deliberately asserts only what must be true, so that an upstream fix
/// does not fail it.
#[test]
fn an_image_beyond_a_limit_leaves_no_image_and_a_working_pane() {
    for policy in [
        GraphicsPolicy {
            storage_bytes: 1024,
            ..GraphicsPolicy::default()
        },
        GraphicsPolicy {
            apc_max_bytes: 4096,
            ..GraphicsPolicy::default()
        },
    ] {
        let (mut session, events, snapshots) = session(policy);
        feed(&mut session, &snapshots, transmit_rgba(1, 32, 32));

        assert!(
            !probe(&mut session, &events).holds(1),
            "the image was refused: {policy:?}"
        );

        // The pane is still a terminal afterwards, which is the property that
        // matters: a limit degrades one image, it does not end the session.
        let bundle = feed(&mut session, &snapshots, "still-a-terminal\\n".to_owned());
        assert!(pane_text(&bundle).contains("still-a-terminal"));

        // And a smaller image still works, so the limit refused one image
        // rather than switching graphics off.
        feed(&mut session, &snapshots, transmit_rgba(2, 4, 4));
        assert!(probe(&mut session, &events).holds(2));
    }
}

/// A program that transmits images forever must reach a steady state.
///
/// This is the pathological case the limits exist for: without them a loop like
/// this is unbounded growth, and the pane eventually takes the machine with it.
#[test]
fn transmitting_images_in_a_loop_reaches_a_steady_state() {
    let policy = GraphicsPolicy {
        storage_bytes: 16 * 1024,
        ..GraphicsPolicy::default()
    };
    let (mut session, events, snapshots) = session(policy);

    let mut readings = Vec::new();
    for id in 1..=40 {
        feed(&mut session, &snapshots, transmit_rgba(id, 32, 32));
        if id % 10 == 0 {
            readings.push(probe(&mut session, &events).stored_bytes());
        }
    }

    for reading in &readings {
        assert!(
            *reading <= 16 * 1024,
            "storage stayed inside its limit through the loop: {readings:?}"
        );
    }
    // Steady rather than merely bounded: the last reading is no larger than the
    // first, so nothing is quietly accumulating beneath the limit.
    let first = *readings.first().expect("a reading");
    let last = *readings.last().expect("a reading");
    assert!(
        last <= first,
        "storage settled instead of creeping: {readings:?}"
    );

    // And the pane still works afterwards.
    let bundle = feed(&mut session, &snapshots, "still-alive\\n".to_owned());
    assert!(pane_text(&bundle).contains("still-alive"));
}
