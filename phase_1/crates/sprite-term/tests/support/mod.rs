//! Shared test scaffolding for Terminal Session integration tests.
//!
//! Every helper here talks only to the public `sprite-term` interface. The
//! taken stream moves into a helper thread that blocks in `next_blocking` and
//! forwards each item through `std::sync::mpsc`, so a hung worker fails the
//! test on the watchdog instead of hanging the suite.

#![allow(dead_code)]

use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::Duration;

use sprite_term::{EventStream, SessionError, TerminalEvent};

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
