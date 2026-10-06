use sprite_term::{SessionConfig, TerminalCommand, TerminalSession};
use std::sync::mpsc;
use std::time::Duration;

#[test]
fn ui_submission_returns_with_events_and_worker_queue_full() {
    let sprite_term::Spawned { mut session, events, mut snapshots } = TerminalSession::spawn(
        SessionConfig::command("/bin/sh", vec!["-c".into(), "i=0; while [ $i -lt 100 ]; do printf '\\033]2;TITLE%s\\007' $i; i=$((i+1)); done; head -c 1048576 /dev/zero; sleep 30".into()]),
    ).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    snapshots.next_blocking().unwrap();
    let sender = session.commands();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let result = sender.try_send(TerminalCommand::Input(b"a".to_vec()));
        let _ = tx.send(result);
    });
    let result = rx.recv_timeout(Duration::from_secs(1));
    let handle = session.begin_shutdown().unwrap().unwrap();
    drop(events);
    handle.wait().unwrap();
    assert!(
        result.is_ok(),
        "UI submission blocked on a full worker queue: {result:?}"
    );
}

#[test]
fn shutdown_retains_events_and_final_outcome_without_consumption() {
    let titles: String = (0..100)
        .map(|index| format!("\x1b]2;TITLE{index}\x07"))
        .collect();
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
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    let mut saturated_since = None;
    loop {
        if session
            .try_send(TerminalCommand::Capture)
            .is_err_and(|error| error.message.contains("queue is full"))
        {
            let since = saturated_since.get_or_insert_with(std::time::Instant::now);
            if since.elapsed() >= Duration::from_millis(100) {
                break;
            }
        } else {
            saturated_since = None;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "worker did not reach sustained event pressure"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    let handle = session.begin_shutdown().unwrap().unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(handle.wait());
    });
    let joined = rx.recv_timeout(Duration::from_secs(7));
    let mut titles = Vec::new();
    let mut exited = false;
    while let Ok(event) = events.next_blocking() {
        match event {
            sprite_term::TerminalEvent::TitleChanged(Some(title)) => {
                assert!(!exited, "events remain ordered before exit");
                titles.push(title);
            }
            sprite_term::TerminalEvent::Exited(_) => exited = true,
            _ => {}
        }
    }
    assert!(
        joined.is_ok(),
        "shutdown depends on event consumption: {joined:?}"
    );
    assert!(exited, "final outcome remains readable after join");
    assert!(
        titles.len() >= 32,
        "the first title beyond the Ready plus normal title slots survives: {}",
        titles.len()
    );
    for (index, title) in titles.iter().enumerate() {
        assert_eq!(title, &format!("TITLE{index}"));
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
    let sprite_term::Spawned { mut session, mut events, snapshots: _snapshots } = TerminalSession::spawn(
        SessionConfig::command("/bin/sh", vec!["-c".into(), "i=0; while [ $i -lt 100 ]; do printf '\\033]2;TITLE%s\\007' $i; i=$((i+1)); done; exit 7".into()]),
    ).unwrap();
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
    while let Ok(event) = events.next_blocking() {
        if let sprite_term::TerminalEvent::Exited(exit) = event {
            outcome = Some(exit);
        }
    }
    let outcome = outcome.expect("reserved exit outcome");
    assert_eq!(outcome.code, Some(7));
    assert!(!outcome.requested);
    session.begin_shutdown().unwrap().unwrap().wait().unwrap();
}
