//! What the suite's snapshot watchdog is for.
//!
//! `SnapshotPump::wait_for` exists to fail a wait that has stopped making
//! progress, so a hung worker fails one test instead of wedging the suite. A
//! child that is still producing output has not stopped making progress,
//! however long it takes to reach what the test is waiting for — and a child
//! that finishes in two seconds on an idle machine can take three times that
//! while the rest of the workspace's tests compete for the same cores.

mod support;

use std::ffi::OsString;
use std::time::Instant;

use sprite_term::{SessionConfig, TerminalSession};

use support::{SnapshotPump, WATCHDOG, pane_text};

/// A child that streams for longer than the watchdog, and only then arrives.
///
/// The filler is what makes this a regression test rather than a slow one: the
/// stream never goes quiet, so there is nothing here for a hang detector to
/// catch. Against a watchdog measured from the start of the wait this fails at
/// five seconds with the marker still on its way — which is what the oversized
/// clipboard payload did on CI, where its 2 MiB of base64 outran the budget
/// that fits it locally.
#[test]
fn a_child_still_producing_output_is_not_timed_out() {
    let ticks = (WATCHDOG.as_secs() + 2) * 10;
    let script = format!(
        "i=0; while [ $i -lt {ticks} ]; do printf 'filler %d\\n' \"$i\"; \
         i=$((i+1)); sleep 0.1; done; printf 'ARRIVED\\n'; sleep 30"
    );
    let mut session = TerminalSession::spawn(SessionConfig::command(
        "/bin/sh",
        vec![OsString::from("-c"), OsString::from(script)],
    ))
    .expect("spawn session");
    let snapshots = SnapshotPump::new(session.take_snapshot_stream().expect("take snapshots"));

    let started = Instant::now();
    let bundle = snapshots.wait_for("the child to arrive", |bundle| {
        pane_text(bundle).contains("ARRIVED")
    });

    assert!(
        pane_text(&bundle).contains("ARRIVED"),
        "the bundle that satisfied the wait is the one that shows the marker"
    );
    assert!(
        started.elapsed() > WATCHDOG,
        "this test only proves anything if the wait outlived the {WATCHDOG:?} \
         watchdog; it took {:?}",
        started.elapsed()
    );
}
