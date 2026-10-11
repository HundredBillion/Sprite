//! Many sessions starting at the same moment must all come up.

mod support;

use std::ffi::OsString;
use std::sync::{Arc, Barrier};
use std::thread;

use sprite_term::{SessionConfig, TerminalSession};

use support::EventPump;

const SESSIONS: usize = 16;
const ROUNDS: usize = 12;

/// Concurrent PTY allocation can transiently fail on macOS; a pane starting
/// alongside others must not surface that as a start error.
#[test]
fn simultaneous_session_starts_all_reach_ready() {
    // One round fails against an unfixed start only some of the time, so repeat.
    for _ in 0..ROUNDS {
        start_all_at_once();
    }
}

fn start_all_at_once() {
    let barrier = Arc::new(Barrier::new(SESSIONS));
    let starters: Vec<_> = (0..SESSIONS)
        .map(|_| {
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                let spawned = TerminalSession::spawn(SessionConfig::command(
                    "/bin/sh",
                    vec![OsString::from("-c"), OsString::from("sleep 30")],
                ))
                .expect("spawn session");
                let events = EventPump::new(spawned.events);
                events.expect_ready();
                spawned.session
            })
        })
        .collect();

    let sessions: Vec<_> = starters
        .into_iter()
        .map(|starter| starter.join().expect("a session failed to reach Ready"))
        .collect();

    for mut session in sessions {
        if let Some(handle) = session.begin_shutdown().expect("begin_shutdown") {
            handle.wait().expect("shutdown completes");
        }
    }
}
