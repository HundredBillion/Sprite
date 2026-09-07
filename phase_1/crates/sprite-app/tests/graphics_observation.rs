//! What an observer learns about an image, and what it must never learn.
//!
//! The exclusion list bans transmitted bytes, decoded pixels, and source
//! filenames. Graphics are the first feature that could breach it by accident,
//! so this drives a real terminal, transmits a recognisable picture, and
//! searches the finished JSON for it.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use sprite_app::{Failure, PaneAddress, PaneReport, Report, render_schema};
use sprite_app::{PaneId, Rect, TabId};
use sprite_term::{
    HistoryLines, HistorySnapshot, SessionConfig, TerminalCommand, TerminalEvent, TerminalSession,
};

/// A byte that is easy to spot and unlikely to occur by chance.
const MARKER_BYTE: u8 = 0xab;

/// A directory of this test's own, which no other test will delete.
///
/// The process id alone does not distinguish them: the tests in a binary run as
/// threads of one process, so every caller here built the same path — and each
/// removed it on the way out. Whichever test was still waiting for its child to
/// `cat` the fixture then found it gone, showed no image, and spent the whole
/// twenty-second deadline discovering that. It failed on CI as a flake that
/// moved between the two tests, because either one can lose the race.
fn fixture_directory() -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    std::env::temp_dir().join(format!(
        "sprite-observe-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
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

/// Captures the history of a pane that has been shown one recognisable image.
///
/// The escape sequence is written to a file and `cat`ed rather than typed, so
/// the payload never appears on screen as an echoed command — otherwise the
/// search below would find it as legitimate screen text and prove nothing.
fn pane_showing_an_image() -> (HistorySnapshot, String) {
    let pixels = vec![MARKER_BYTE; 32 * 32 * 4];
    let payload = base64(&pixels);
    let sequence = format!("\x1b_Ga=T,f=32,s=32,v=32,i=1,q=2;{payload}\x1b\\");

    let directory = fixture_directory();
    std::fs::create_dir_all(&directory).expect("a directory for the fixture");
    let path = directory.join("image.esc");
    std::fs::write(&path, &sequence).expect("write the fixture");

    let mut session = TerminalSession::spawn(SessionConfig::command(
        "/bin/sh",
        vec![
            "-c".into(),
            format!("cat {}; sleep 300", path.display()).into(),
        ],
    ))
    .expect("spawn session");
    let mut events = session.take_event_stream().expect("take event stream");

    // Give the child time to print, then ask. The history answer is what the
    // schema is built from.
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut snapshot = None;
    while Instant::now() < deadline && snapshot.is_none() {
        session
            .send(TerminalCommand::CaptureHistory(HistoryLines::new(500)))
            .expect("request history");
        while let Ok(event) = events.next_blocking() {
            match event {
                TerminalEvent::History(history) if !history.placements.is_empty() => {
                    snapshot = Some((*history).clone());
                    break;
                }
                TerminalEvent::History(_) => break,
                _ => {}
            }
        }
    }

    let _ = std::fs::remove_dir_all(&directory);
    (
        snapshot.expect("the pane showed an image within the watchdog"),
        payload,
    )
}

fn report_of(snapshot: HistorySnapshot) -> Report {
    Report {
        complete: true,
        panes: vec![PaneReport {
            address: PaneAddress {
                tab: TabId(0),
                tab_order: 0,
                pane: PaneId(0),
                rect: Rect::FULL,
                focused: true,
            },
            snapshot: Arc::new(snapshot),
        }],
        failures: Vec::<Failure>::new(),
    }
}

#[test]
fn an_observer_is_told_an_image_is_there() {
    let (snapshot, _) = pane_showing_an_image();
    let json = render_schema(&report_of(snapshot), true);
    let value: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");

    let placement = &value["panes"][0]["placements"][0];
    assert_eq!(placement["image"], serde_json::json!(1));
    assert_eq!(
        placement["pixel_size"]["width"],
        serde_json::json!(32),
        "its size, so a client knows how much space it takes: {placement}"
    );
    assert_eq!(placement["transmission_format"], serde_json::json!("rgba"));
    assert_eq!(placement["visible"], serde_json::json!(true));
}

/// The promise this task exists to keep.
#[test]
fn the_image_itself_never_reaches_the_response() {
    let (snapshot, payload) = pane_showing_an_image();
    let json = render_schema(&report_of(snapshot), true);

    // The transmitted bytes, as they were sent.
    assert!(
        !json.contains(&payload[..64]),
        "the transmission's own bytes appear in the response"
    );

    // The decoded pixels, in the forms they could plausibly take: a run of the
    // marker byte as numbers, as hex, or re-encoded.
    for shape in [
        "171, 171, 171",
        "171,171,171",
        "abababab",
        "ABABABAB",
        "\\u00ab",
    ] {
        assert!(
            !json.contains(shape),
            "decoded pixels appear in the response as {shape:?}"
        );
    }

    // And nothing that names a file. The fixture lived in a temporary
    // directory, whose name would show up if a path were ever carried.
    assert!(!json.contains("sprite-observe-"), "a filename leaked");
    assert!(!json.contains(".esc"), "a filename leaked");

    // A response describing a 4 KiB image is a few hundred bytes: the pixels
    // are not in there under any encoding, because there is no room for them.
    let pane = &json[json.find("\"placements\"").expect("placements")..];
    assert!(
        pane.len() < 4096,
        "the placement section is metadata-sized, not image-sized: {} bytes",
        pane.len()
    );
}

/// Two fixtures never collide, so no test can delete another's.
///
/// This is the property the flake violated. It is asserted directly rather than
/// by racing the tests, because the race is what made the failure rare enough
/// to survive several merges.
#[test]
fn every_fixture_directory_is_its_own() {
    let paths: Vec<PathBuf> = (0..8).map(|_| fixture_directory()).collect();
    let mut unique = paths.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(
        unique.len(),
        paths.len(),
        "each caller gets a directory no other caller will remove, got {paths:?}"
    );
}
