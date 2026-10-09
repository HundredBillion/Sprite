use sprite_term::{SessionConfig, TerminalCommand, TerminalEvent, TerminalSession};
use std::sync::mpsc;
use std::time::Duration;

#[test]
fn shutdown_retains_accepted_titles_without_consumer_progress() {
    // One parser batch is one PTY read, which macOS caps at 1 KiB; 80 titles fit.
    let titles: String = (0..80).map(|i| format!("\x1b]2;TITLE{i}\x07")).collect();
    let script = format!("printf '%s' '{titles}'; sleep 30");
    let sprite_term::Spawned {
        mut session,
        mut events,
        snapshots: _snapshots,
    } = TerminalSession::spawn(SessionConfig::command(
        "/bin/sh",
        vec!["-c".into(), script.into()],
    ))
    .unwrap();
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
            TerminalEvent::TitleChanged(Some(title)) => {
                assert!(!exited);
                retained.push(title);
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
        80,
        "one accepted parser batch survives cancellation"
    );
    for (i, title) in retained.iter().enumerate() {
        assert_eq!(title, &format!("TITLE{i}"));
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
fn resumed_consumer_receives_all_titles_in_order() {
    let sprite_term::Spawned { mut session, mut events, snapshots: _snapshots } = TerminalSession::spawn(
        SessionConfig::command("/bin/sh", vec!["-c".into(), "i=0; while [ $i -lt 100 ]; do printf '\\033]2;TITLE%s\\007' $i; i=$((i+1)); done; sleep 0.2; exit 7".into()]),
    ).unwrap();
    std::thread::sleep(Duration::from_millis(100));
    let mut titles = Vec::new();
    while let Ok(event) = events.next_blocking() {
        if let sprite_term::TerminalEvent::TitleChanged(Some(title)) = event {
            titles.push(title);
        }
    }
    assert_eq!(titles.len(), 100);
    for (index, title) in titles.iter().enumerate() {
        assert_eq!(title, &format!("TITLE{index}"));
    }
    session.begin_shutdown().unwrap().unwrap().wait().unwrap();
}

#[test]
fn dropping_the_consumer_releases_event_pressure() {
    let sprite_term::Spawned { mut session, events, snapshots: _snapshots } = TerminalSession::spawn(
        SessionConfig::command("/bin/sh", vec!["-c".into(), "i=0; while [ $i -lt 100 ]; do printf '\\033]2;TITLE%s\\007' $i; i=$((i+1)); done; sleep 30".into()]),
    ).unwrap();
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
    let titles: String = (0..100).map(|i| format!("\x1b]2;TITLE{i}\x07")).collect();
    let script = format!("printf '%s' '{titles}'; exit 7");
    let sprite_term::Spawned {
        mut session,
        mut events,
        snapshots: _snapshots,
    } = TerminalSession::spawn(SessionConfig::command(
        "/bin/sh",
        vec!["-c".into(), script.into()],
    ))
    .unwrap();
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
    let mut titles = Vec::new();
    while let Ok(event) = events.next_blocking() {
        match event {
            TerminalEvent::TitleChanged(Some(title)) => {
                assert!(
                    outcome.is_none(),
                    "retained titles precede the final outcome"
                );
                titles.push(title);
            }
            TerminalEvent::Exited(exit) => outcome = Some(exit),
            _ => {}
        }
    }
    assert!(titles.len() >= 32);
    for (index, title) in titles.iter().enumerate() {
        assert_eq!(title, &format!("TITLE{index}"));
    }
    let outcome = outcome.expect("reserved exit outcome");
    assert_eq!(outcome.code, Some(7));
    assert!(!outcome.requested);
    session.begin_shutdown().unwrap().unwrap().wait().unwrap();
}
