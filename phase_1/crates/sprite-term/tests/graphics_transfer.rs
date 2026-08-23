//! Getting an image into a pane: the formats, the compression, the chunking,
//! and what happens when the payload is nonsense.
//!
//! Everything here goes through a real terminal and a real child, because the
//! transfer path is the protocol parser, the decoder, and image storage acting
//! together. A decoder that works in isolation and never receives a byte in
//! practice would pass a unit test and show nothing on screen.

mod support;

use std::ffi::OsString;
use std::time::{Duration, Instant};

use sprite_term::{
    GraphicsPolicy, GraphicsSnapshot, SessionConfig, TerminalCommand, TerminalEvent,
    TerminalSession,
};

use support::{EventPump, SnapshotPump, base64, kitty, pane_text, png_bytes};

fn args(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}

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

fn session(policy: GraphicsPolicy) -> (TerminalSession, EventPump, SnapshotPump) {
    let mut config = SessionConfig::command("/bin/sh", args(&[]));
    config.graphics = policy;
    let mut session = TerminalSession::spawn(config).expect("spawn session");
    let events = EventPump::new(session.take_event_stream().expect("take event stream"));
    let snapshots = SnapshotPump::new(session.take_snapshot_stream().expect("take snapshots"));
    events.expect_ready();
    session
        .send(TerminalCommand::Input(b"printf 'READY\\n'\n".to_vec()))
        .expect("ask the shell to announce itself");
    snapshots.wait_for("the shell to start", |bundle| {
        pane_text(bundle).contains("READY")
    });
    (session, events, snapshots)
}

static NEXT_MARK: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Has the child print `escapes`, then waits until the terminal has read them.
fn feed(session: &mut TerminalSession, snapshots: &SnapshotPump, escapes: String) {
    session
        .send(TerminalCommand::Input(
            format!("printf '{escapes}'\n").into_bytes(),
        ))
        .expect("ask the child to print the payload");

    let mark = NEXT_MARK.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let marker = format!("MARK{mark}");
    session
        .send(TerminalCommand::Input(
            format!("printf '{marker}\\n'\n").into_bytes(),
        ))
        .expect("ask the child to print the marker");
    snapshots.wait_for("the payload to be processed", |bundle| {
        pane_text(bundle).contains(&marker)
    });
}

fn probe(session: &mut TerminalSession, events: &EventPump) -> GraphicsSnapshot {
    session
        .send(TerminalCommand::CaptureGraphics)
        .expect("request graphics");
    wait_for_graphics(events)
}

/// The image as the schema will eventually report it.
fn held(graphics: &GraphicsSnapshot, id: u32) -> Option<(u32, u32)> {
    graphics
        .images
        .iter()
        .find(|image| image.id == id)
        .map(|image| (image.width, image.height))
}

#[test]
fn a_png_transmission_is_decoded() {
    let (mut session, events, snapshots) = session(GraphicsPolicy::default());
    let png = png_bytes(6, 5, 0x40);

    feed(
        &mut session,
        &snapshots,
        kitty("a=T,f=100,i=1", &base64(&png)),
    );
    let graphics = probe(&mut session, &events);

    assert_eq!(
        held(&graphics, 1),
        Some((6, 5)),
        "the PNG's own dimensions, which means it really was decoded rather \
         than stored as bytes: {graphics:?}"
    );
}

/// A PNG that reaches a pane with no decoder must be refused, not stored raw.
#[test]
fn a_png_without_a_decoder_is_refused() {
    let (mut session, events, snapshots) = session(GraphicsPolicy::disabled());
    let png = png_bytes(6, 5, 0x40);

    feed(
        &mut session,
        &snapshots,
        kitty("a=T,f=100,i=1", &base64(&png)),
    );

    assert!(probe(&mut session, &events).images.is_empty());
}

#[test]
fn raw_rgb_and_rgba_are_both_accepted() {
    let (mut session, events, snapshots) = session(GraphicsPolicy::default());

    let rgba = vec![0x30_u8; 4 * 4 * 4];
    feed(
        &mut session,
        &snapshots,
        kitty("a=T,f=32,s=4,v=4,i=1", &base64(&rgba)),
    );

    let rgb = vec![0x60_u8; 5 * 3 * 3];
    feed(
        &mut session,
        &snapshots,
        kitty("a=T,f=24,s=5,v=3,i=2", &base64(&rgb)),
    );

    let graphics = probe(&mut session, &events);
    assert_eq!(held(&graphics, 1), Some((4, 4)), "RGBA: {graphics:?}");
    assert_eq!(held(&graphics, 2), Some((5, 3)), "RGB: {graphics:?}");
}

#[test]
fn a_zlib_compressed_payload_is_accepted() {
    use flate2::Compression;
    use flate2::write::ZlibEncoder;
    use std::io::Write;

    let (mut session, events, snapshots) = session(GraphicsPolicy::default());

    let rgba = vec![0x77_u8; 8 * 8 * 4];
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&rgba).expect("compress");
    let compressed = encoder.finish().expect("finish");
    assert!(
        compressed.len() < rgba.len(),
        "the fixture really is smaller"
    );

    feed(
        &mut session,
        &snapshots,
        kitty("a=T,f=32,s=8,v=8,o=z,i=1", &base64(&compressed)),
    );

    let graphics = probe(&mut session, &events);
    assert_eq!(held(&graphics, 1), Some((8, 8)), "{graphics:?}");
}

/// A payload too large for one escape sequence arrives in pieces. The pane must
/// hold one image at the end, not three fragments.
#[test]
fn a_chunked_transmission_reassembles() {
    let (mut session, events, snapshots) = session(GraphicsPolicy::default());

    let rgba = vec![0x21_u8; 16 * 16 * 4];
    let encoded = base64(&rgba);
    // Chunk boundaries must fall on whole base64 groups, as the protocol
    // requires; 256 characters is 192 bytes.
    let chunks: Vec<&str> = encoded
        .as_bytes()
        .chunks(256)
        .map(|chunk| std::str::from_utf8(chunk).expect("base64 is ASCII"))
        .collect();
    assert!(chunks.len() > 2, "the fixture really is chunked");

    for (index, chunk) in chunks.iter().enumerate() {
        let control = if index == 0 {
            "a=T,f=32,s=16,v=16,i=1,m=1".to_owned()
        } else if index + 1 == chunks.len() {
            "m=0".to_owned()
        } else {
            "m=1".to_owned()
        };
        feed(&mut session, &snapshots, kitty(&control, chunk));
    }

    let graphics = probe(&mut session, &events);
    assert_eq!(held(&graphics, 1), Some((16, 16)), "{graphics:?}");
    assert_eq!(graphics.images.len(), 1, "one image, not one per chunk");
}

/// A chunk sequence that never finishes must not become an image.
///
/// It also must not be *escaped* by simply starting another transmission: the
/// protocol says everything after the first chunk belongs to the sequence until
/// `m=0`, so a would-be second image is swallowed as more of the first. That is
/// the protocol working, not a defect — and it is why the accumulation is
/// bounded by `apc_max_bytes` rather than by hoping a sequence ends.
#[test]
fn an_unfinished_chunk_sequence_is_discarded() {
    let (mut session, events, snapshots) = session(GraphicsPolicy::default());

    let rgba = vec![0x21_u8; 16 * 16 * 4];
    let encoded = base64(&rgba);
    feed(
        &mut session,
        &snapshots,
        kitty("a=T,f=32,s=16,v=16,i=1,m=1", &encoded[..256]),
    );

    assert!(
        probe(&mut session, &events).images.is_empty(),
        "an unfinished transmission is not an image"
    );

    // Ending the sequence delivers less data than the header promised, so the
    // image is refused rather than shown half-filled.
    feed(&mut session, &snapshots, kitty("m=0", ""));
    assert!(
        probe(&mut session, &events).images.is_empty(),
        "a sequence that delivered less than it declared is refused"
    );

    // And with the sequence closed, the pane accepts images again.
    let small = vec![0x44_u8; 2 * 2 * 4];
    feed(
        &mut session,
        &snapshots,
        kitty("a=T,f=32,s=2,v=2,i=2", &base64(&small)),
    );
    assert_eq!(
        held(&probe(&mut session, &events), 2),
        Some((2, 2)),
        "the pane recovers once the abandoned sequence is closed"
    );
}

/// Nonsense must be refused without taking the pane with it. This runs on the
/// thread that owns the terminal, so a panic here would end the session.
#[test]
fn malformed_payloads_are_refused_and_the_pane_survives() {
    let (mut session, events, snapshots) = session(GraphicsPolicy::default());
    let png = png_bytes(4, 4, 0x90);

    let cases: Vec<(&str, String, String)> = vec![
        (
            "a PNG that is not a PNG",
            "a=T,f=100,i=10".to_owned(),
            base64(b"not a png at all, just some bytes"),
        ),
        (
            "a truncated PNG",
            "a=T,f=100,i=11".to_owned(),
            base64(&png[..png.len() / 2]),
        ),
        (
            "raw pixels that do not fill the declared size",
            "a=T,f=32,s=64,v=64,i=12".to_owned(),
            base64(&[0x10_u8; 16]),
        ),
        (
            "a zero-sized image",
            "a=T,f=32,s=0,v=0,i=13".to_owned(),
            base64(&[]),
        ),
        (
            "payload that is not base64",
            "a=T,f=32,s=2,v=2,i=14".to_owned(),
            "!!!!not base64!!!!".to_owned(),
        ),
        (
            "compression claimed but absent",
            "a=T,f=32,s=4,v=4,o=z,i=15".to_owned(),
            base64(&[0x00_u8; 64]),
        ),
    ];

    for (name, control, payload) in cases {
        feed(&mut session, &snapshots, kitty(&control, &payload));
        let graphics = probe(&mut session, &events);
        assert!(
            graphics.images.is_empty(),
            "{name} produced an image: {graphics:?}"
        );
    }

    // The pane still works: every refusal above was a refusal, not damage.
    feed(
        &mut session,
        &snapshots,
        kitty("a=T,f=100,i=20", &base64(&png)),
    );
    assert_eq!(
        held(&probe(&mut session, &events), 20),
        Some((4, 4)),
        "a good image after six bad ones"
    );

    feed(&mut session, &snapshots, "text-still-works\\n".to_owned());
    let bundle = snapshots.wait_for("text after malformed images", |bundle| {
        pane_text(bundle).contains("text-still-works")
    });
    assert!(pane_text(&bundle).contains("text-still-works"));
}

/// Decoding is bounded by the same limit that bounds storage: an image too
/// large to keep is never decoded, because decoding is where the memory goes.
#[test]
fn a_png_beyond_the_storage_limit_is_not_decoded() {
    let policy = GraphicsPolicy {
        storage_bytes: 2 * 1024,
        ..GraphicsPolicy::default()
    };
    let (mut session, events, snapshots) = session(policy);

    // 64x64 RGBA is 16 KiB decoded, from a PNG of a few hundred bytes: the
    // compressed size is no guide to what decoding will cost.
    let png = png_bytes(64, 64, 0x11);
    assert!(png.len() < 2 * 1024, "the transmission itself is small");

    feed(
        &mut session,
        &snapshots,
        kitty("a=T,f=100,i=1", &base64(&png)),
    );

    let graphics = probe(&mut session, &events);
    assert!(
        graphics.images.is_empty(),
        "a small payload that decodes large is still refused: {graphics:?}"
    );
}
