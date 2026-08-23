//! Shared test scaffolding for Terminal Session integration tests.
//!
//! Every helper here talks only to the public `sprite-term` interface. The
//! taken stream moves into a helper thread that blocks in `next_blocking` and
//! forwards each item through `std::sync::mpsc`, so a hung worker fails the
//! test on the watchdog instead of hanging the suite.

#![allow(dead_code)]

use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use sprite_term::{EventStream, SessionError, SnapshotBundle, SnapshotStream, TerminalEvent};

/// Every blocking wait in the suite fails after this long.
pub const WATCHDOG: Duration = Duration::from_secs(5);

/// Drives an `EventStream` from a helper thread.
pub struct EventPump {
    receiver: Receiver<Result<TerminalEvent, SessionError>>,
}

impl EventPump {
    pub fn new(mut stream: EventStream) -> Self {
        let (sender, receiver) = mpsc::channel();
        thread::Builder::new()
            .name("event-pump".to_owned())
            .spawn(move || {
                loop {
                    let item = stream.next_blocking();
                    let ended = item.is_err();
                    if sender.send(item).is_err() || ended {
                        break;
                    }
                }
            })
            .expect("spawn event pump");
        Self { receiver }
    }

    /// The next event, or a panic describing which wait timed out.
    pub fn next(&self) -> TerminalEvent {
        match self.receiver.recv_timeout(WATCHDOG) {
            Ok(Ok(event)) => event,
            Ok(Err(error)) => panic!("event stream ended early: {error}"),
            Err(RecvTimeoutError::Timeout) => {
                panic!("watchdog: no terminal event within {WATCHDOG:?}")
            }
            Err(RecvTimeoutError::Disconnected) => panic!("event pump thread died"),
        }
    }

    /// Waits for `Ready`, failing on any other event.
    pub fn expect_ready(&self) {
        match self.next() {
            TerminalEvent::Ready => {}
            other => panic!("expected Ready, got {other:?}"),
        }
    }
}

/// Drives a `SnapshotStream` from a helper thread.
///
/// Snapshots are latest-only, so a test cannot assume the first bundle it sees
/// carries the output it is waiting for. `wait_for` keeps pulling until a
/// bundle satisfies the predicate or the watchdog fires.
pub struct SnapshotPump {
    receiver: Receiver<Result<Arc<SnapshotBundle>, SessionError>>,
}

impl SnapshotPump {
    pub fn new(mut stream: SnapshotStream) -> Self {
        let (sender, receiver) = mpsc::channel();
        thread::Builder::new()
            .name("snapshot-pump".to_owned())
            .spawn(move || {
                loop {
                    let item = stream.next_blocking();
                    let ended = item.is_err();
                    if sender.send(item).is_err() || ended {
                        break;
                    }
                }
            })
            .expect("spawn snapshot pump");
        Self { receiver }
    }

    /// The next bundle, whatever it holds.
    pub fn next(&self) -> Arc<SnapshotBundle> {
        match self.receiver.recv_timeout(WATCHDOG) {
            Ok(Ok(bundle)) => bundle,
            Ok(Err(error)) => panic!("snapshot stream ended early: {error}"),
            Err(RecvTimeoutError::Timeout) => {
                panic!("watchdog: no snapshot within {WATCHDOG:?}")
            }
            Err(RecvTimeoutError::Disconnected) => panic!("snapshot pump thread died"),
        }
    }

    /// The first bundle satisfying `predicate`, within the watchdog.
    pub fn wait_for(
        &self,
        what: &str,
        predicate: impl Fn(&SnapshotBundle) -> bool,
    ) -> Arc<SnapshotBundle> {
        let deadline = Instant::now() + WATCHDOG;
        let mut seen = 0_u32;
        while Instant::now() < deadline {
            let bundle = self.next();
            seen += 1;
            if predicate(&bundle) {
                return bundle;
            }
        }
        panic!("watchdog: no snapshot matched {what} after {seen} bundles");
    }
}

/// The visible pane text of a bundle, one row per line, trailing blanks
/// removed.
pub fn pane_text(bundle: &SnapshotBundle) -> String {
    bundle
        .pane
        .rows
        .iter()
        .map(|row| row.text.trim_end().to_owned())
        .collect::<Vec<_>>()
        .join("\n")
}

impl EventPump {
    /// Looks for a clipboard write within a short window.
    ///
    /// A denied write is silence, so this cannot block indefinitely; but pure
    /// `try_recv` would race the pump thread that forwards events, so it waits
    /// briefly and only then concludes nothing arrived.
    pub fn try_next_clipboard(&self) -> Option<String> {
        let deadline = Instant::now() + Duration::from_millis(750);
        while Instant::now() < deadline {
            match self.receiver.recv_timeout(Duration::from_millis(50)) {
                Ok(Ok(TerminalEvent::ClipboardWrite(text))) => return Some(text),
                Ok(_) => {}
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return None,
            }
        }
        None
    }
}

// ---------------------------------------------------------------------------
// Kitty graphics
// ---------------------------------------------------------------------------

/// Base64 as the Kitty protocol wants it, without a dependency for it.
pub fn base64(bytes: &[u8]) -> String {
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

/// One Kitty escape sequence, written as printf escapes.
///
/// `q=2` silences the protocol's reply. The terminal answers a transmission by
/// writing back to the child, which is correct Kitty behaviour — but these
/// tests drive a shell, and a shell reading a protocol reply on its standard
/// input tries to run it as a command and mangles whatever came next.
pub fn kitty(control: &str, payload: &str) -> String {
    format!("\\033_G{control},q=2;{payload}\\033\\\\")
}

/// A PNG of one flat colour, built by the same crate the decoder reads with.
pub fn png_bytes(width: u32, height: u32, value: u8) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().expect("write the header");
        let pixels = vec![value; (width * height * 4) as usize];
        writer.write_image_data(&pixels).expect("write the pixels");
    }
    out
}
