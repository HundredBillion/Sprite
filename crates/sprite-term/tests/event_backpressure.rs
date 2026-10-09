use sprite_term::{SessionConfig, TerminalCommand, TerminalEvent, TerminalSession};
use std::sync::mpsc;
use std::time::Duration;

/// Standard base64, enough to spell an OSC 52 payload.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for group in bytes.chunks(3) {
        let at = |index: usize| u32::from(group.get(index).copied().unwrap_or(0));
        let bits = (at(0) << 16) | (at(1) << 8) | at(2);
        for index in 0..4 {
            if index <= group.len() {
                out.push(char::from(
                    ALPHABET[(bits >> (18 - 6 * index)) as usize & 63],
                ));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// `count` OSC 52 clipboard writes of `CLIP0`, `CLIP1`, …, as one string,
/// sixteen bytes each.
///
/// Clipboard writes are the one parser notice the worker never coalesces — a
/// title keeps only its latest value per pass — so they are how these tests
/// put many lossless events into one pass.
fn clipboard_writes(count: usize) -> String {
    (0..count)
        .map(|index| {
            format!(
                "\x1b]52;c;{}\x07",
                base64(format!("CLIP{index}").as_bytes())
            )
        })
        .collect()
}

/// A child that waits for one line before writing `writes`, then runs `then`.
fn on_cue(writes: &str, then: &str) -> SessionConfig {
    SessionConfig::command(
        "/bin/sh",
        vec![
            "-c".into(),
            format!("stty -echo; read _; printf '%s' '{writes}'; {then}").into(),
        ],
    )
}

/// Focuses the pane, then releases the child: an unfocused pane is denied the
/// clipboard and the writes would raise nothing.
fn focus_and_release(session: &mut TerminalSession) {
    session.send(TerminalCommand::Focus(true)).unwrap();
    session
        .send(TerminalCommand::Input(b"\n".to_vec()))
        .unwrap();
}

#[test]
fn shutdown_retains_accepted_clipboard_writes_without_consumer_progress() {
    // One write, small enough for one PTY read even where a read returns at
    // most 1 KiB, and more events than the mailbox's thirty-two ordinary slots.
    let writes = clipboard_writes(60);
    assert!(writes.len() <= 1024);
    let sprite_term::Spawned {
        mut session,
        mut events,
        snapshots: _snapshots,
    } = TerminalSession::spawn(on_cue(&writes, "sleep 30")).unwrap();
    focus_and_release(&mut session);
    std::thread::sleep(Duration::from_millis(300));
    let handle = session.begin_shutdown().unwrap().unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(handle.wait());
    });
    let joined = rx.recv_timeout(Duration::from_secs(7));
    let mut retained = Vec::new();
    let mut exited = false;
    while let Ok(event) = events.next_blocking() {
        match event {
            TerminalEvent::ClipboardWrite(text) => {
                assert!(!exited);
                retained.push(text);
            }
            TerminalEvent::Exited(_) => exited = true,
            _ => {}
        }
    }
    assert!(
        joined.is_ok(),
        "shutdown depends on consumer progress: {joined:?}"
    );
    assert!(exited);
    assert_eq!(
        retained.len(),
        60,
        "one accepted parser pass survives cancellation"
    );
    for (i, text) in retained.iter().enumerate() {
        assert_eq!(text, &format!("CLIP{i}"));
    }
}

#[test]
fn every_submission_method_bounds_variable_sized_input() {
    use sprite_term::{KeyAction, KeyEvent, KeyModifiers};
    let sprite_term::Spawned {
        mut session,
        events: _events,
        snapshots: _snapshots,
    } = TerminalSession::spawn(SessionConfig::command(
        "/bin/sh",
        vec!["-c".into(), "sleep 30".into()],
    ))
    .unwrap();
    let sender = session.commands();
    let oversized = vec![
        TerminalCommand::Input(vec![b'x'; 16 * 1024 + 1]),
        TerminalCommand::Paste("x".repeat(1024 * 1024 + 1)),
        TerminalCommand::PasteConfirmed("x".repeat(1024 * 1024 + 1)),
        TerminalCommand::CommitText("x".repeat(16 * 1024 + 1)),
        TerminalCommand::Key(KeyEvent {
            logical_key: "x".into(),
            text: Some("x".repeat(16 * 1024 + 1)),
            action: KeyAction::Press,
            composing: false,
            modifiers: KeyModifiers {
                shift: false,
                alt: false,
                control: false,
                platform: false,
                function: false,
            },
        }),
    ];
    for command in oversized {
        for result in [
            session.send(command.clone()),
            session.try_send(command.clone()),
            sender.send(command.clone()),
            sender.try_send(command),
        ] {
            let error = result.expect_err("oversized command must be refused before queuing");
            assert!(error.message.contains("limit"), "{error}");
        }
    }
    session.begin_shutdown().unwrap().unwrap().wait().unwrap();
}

#[test]
fn resumed_consumer_receives_all_clipboard_writes_in_order() {
    let writes = clipboard_writes(100);
    let sprite_term::Spawned {
        mut session,
        mut events,
        snapshots: _snapshots,
    } = TerminalSession::spawn(on_cue(&writes, "sleep 0.2; exit 7")).unwrap();
    focus_and_release(&mut session);
    std::thread::sleep(Duration::from_millis(100));
    let mut received = Vec::new();
    while let Ok(event) = events.next_blocking() {
        if let TerminalEvent::ClipboardWrite(text) = event {
            received.push(text);
        }
    }
    assert_eq!(received.len(), 100);
    for (index, text) in received.iter().enumerate() {
        assert_eq!(text, &format!("CLIP{index}"));
    }
    session.begin_shutdown().unwrap().unwrap().wait().unwrap();
}

#[test]
fn dropping_the_consumer_releases_event_pressure() {
    let writes = clipboard_writes(100);
    let sprite_term::Spawned {
        mut session,
        events,
        snapshots: _snapshots,
    } = TerminalSession::spawn(on_cue(&writes, "sleep 30")).unwrap();
    focus_and_release(&mut session);
    std::thread::sleep(Duration::from_millis(300));
    drop(events);
    let handle = session.begin_shutdown().unwrap().unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(handle.wait());
    });
    rx.recv_timeout(Duration::from_secs(7))
        .expect("drop releases producer")
        .unwrap();
}

#[test]
fn natural_exit_releases_event_pressure_before_shutdown_is_requested() {
    // One write: separate small writes exhaust the output permits while the consumer
    // stalls, and macOS PTYs then block the child before it can exit.
    let writes = clipboard_writes(100);
    let sprite_term::Spawned {
        mut session,
        mut events,
        snapshots: _snapshots,
    } = TerminalSession::spawn(on_cue(&writes, "exit 7")).unwrap();
    focus_and_release(&mut session);
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    loop {
        let ended = session
            .try_send(TerminalCommand::Capture)
            .is_err_and(|error| error.message == "the terminal worker ended");
        if ended {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "natural cleanup depended on consuming events"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let mut outcome = None;
    let mut received = Vec::new();
    while let Ok(event) = events.next_blocking() {
        match event {
            TerminalEvent::ClipboardWrite(text) => {
                assert!(
                    outcome.is_none(),
                    "retained writes precede the final outcome"
                );
                received.push(text);
            }
            TerminalEvent::Exited(exit) => outcome = Some(exit),
            _ => {}
        }
    }
    assert!(received.len() >= 32);
    for (index, text) in received.iter().enumerate() {
        assert_eq!(text, &format!("CLIP{index}"));
    }
    let outcome = outcome.expect("reserved exit outcome");
    assert_eq!(outcome.code, Some(7));
    assert!(!outcome.requested);
    session.begin_shutdown().unwrap().unwrap().wait().unwrap();
}
