use super::*;
use std::time::Duration;

/// Runs `body` on its own thread, failing rather than hanging the suite if
/// it has not finished in twenty seconds.
fn within_watchdog(body: impl FnOnce() + Send + 'static) {
    let (done, finished) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        body();
        done.send(()).ok();
    });
    match finished.recv_timeout(Duration::from_secs(20)) {
        Ok(()) => {}
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            panic!("watchdog: the worker test made no progress for twenty seconds")
        }
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
            panic!("the worker test failed; its assertion is printed above")
        }
    }
}

/// A real worker over a silent child whose inbox already holds output when
/// it starts.
///
/// Events are published with no spare room, so the worker waits at each
/// publication until the test takes it. The test therefore decides when the
/// snapshot slot is empty, and sees every snapshot the worker publishes.
struct Fixture {
    commands: SyncSender<Message>,
    /// Held so the lane stays open, as a live session's would.
    _control: SyncSender<Message>,
    mailbox: Arc<crate::event_mailbox::Mailbox>,
    events: crate::event_mailbox::Receiver,
    snapshots: async_channel::Receiver<Arc<SnapshotBundle>>,
    shutdown: Arc<AtomicBool>,
    worker: JoinHandle<Option<pty_unix::SessionProcesses>>,
}

impl Fixture {
    fn start(chunks: &[Vec<u8>]) -> Self {
        Self::start_with(
            chunks
                .iter()
                .map(|chunk| Message::PtyOutput(pty_unix::OutputChunk::detached(chunk)))
                .collect(),
        )
    }

    fn start_with(queued: Vec<Message>) -> Self {
        Self::start_with_control(queued, Vec::new())
    }

    /// As `start_with`, with `control` already waiting on the control lane.
    fn start_with_control(queued: Vec<Message>, control: Vec<Message>) -> Self {
        let (commands, inbox) = std::sync::mpsc::sync_channel(crate::WORKER_QUEUE_CAPACITY);
        for message in queued {
            assert!(
                commands.try_send(message).is_ok(),
                "the inbox holds every queued message"
            );
        }
        let (control_tx, control_rx) = std::sync::mpsc::sync_channel(crate::CONTROL_CAPACITY);
        for message in control {
            assert!(
                control_tx.try_send(message).is_ok(),
                "the control lane holds every queued command"
            );
        }
        let (events, receiver) = crate::event_mailbox::bounded(0);
        let mailbox = Arc::clone(&events);
        let (published, snapshots) = async_channel::bounded(1);
        let shutdown = Arc::new(AtomicBool::new(false));
        let worker = std::thread::spawn({
            let commands = commands.clone();
            let shutdown = Arc::clone(&shutdown);
            move || {
                run(
                    SessionConfig::command("/bin/sh", vec!["-c".into(), "exec sleep 30".into()]),
                    commands,
                    inbox,
                    control_rx,
                    events,
                    published,
                    shutdown,
                    Arc::new(crate::ForegroundWatch::default()),
                )
            }
        });
        Self {
            commands,
            _control: control_tx,
            mailbox,
            events: receiver,
            snapshots,
            shutdown,
            worker,
        }
    }

    fn next_event(&self) -> TerminalEvent {
        self.events
            .next_blocking()
            .expect("the worker is still publishing")
    }

    fn next_generation(&self) -> u64 {
        self.snapshots
            .recv_blocking()
            .expect("the worker is still capturing")
            .generation
    }

    /// Shuts the worker down; returns every event published after this
    /// point and the generation of any snapshot left unread.
    fn finish(self) -> (Vec<TerminalEvent>, Option<u64>) {
        self.shutdown.store(true, Ordering::SeqCst);
        self.commands.send(Message::Shutdown).ok();
        if let Some(processes) = self.worker.join().expect("the worker did not panic") {
            finish_shutdown(processes, std::time::Instant::now());
        }
        let mut remaining = Vec::new();
        while let Ok(event) = self.events.next_blocking() {
            remaining.push(event);
        }
        let unread = self
            .snapshots
            .try_recv()
            .ok()
            .map(|bundle| bundle.generation);
        (remaining, unread)
    }
}

fn title_of(event: TerminalEvent) -> String {
    let TerminalEvent::TitleChanged(Some(title)) = event else {
        panic!("expected a title, got {event:?}");
    };
    title
}

/// Sixteen chunks already waiting are one pass: one snapshot and one title.
/// The seventeenth is the next pass.
#[test]
fn sixteen_queued_chunks_produce_one_snapshot() {
    within_watchdog(|| {
        let chunks: Vec<Vec<u8>> = (0..17)
            .map(|index| format!("\x1b]2;title-{index}\x07line {index}\r\n").into_bytes())
            .collect();
        let fixture = Fixture::start(&chunks);
        assert!(matches!(fixture.next_event(), TerminalEvent::Ready));
        assert_eq!(fixture.next_generation(), 0, "the frame before any output");
        assert_eq!(title_of(fixture.next_event()), "title-15");
        assert_eq!(
            fixture.next_generation(),
            16,
            "one snapshot for the first sixteen chunks"
        );
        assert_eq!(title_of(fixture.next_event()), "title-16");
        assert_eq!(fixture.next_generation(), 17);
        let (remaining, unread) = fixture.finish();
        assert!(
            !remaining
                .iter()
                .any(|event| matches!(event, TerminalEvent::TitleChanged(_))),
            "{remaining:?}"
        );
        assert_eq!(unread, None, "nothing was left to capture");
    });
}

/// A pass also ends once it has parsed 16 KiB of output.
#[test]
fn a_pass_stops_at_sixteen_kibibytes_of_output() {
    within_watchdog(|| {
        let chunks: Vec<Vec<u8>> = (0..3)
            .map(|index| {
                let mut chunk = format!("\x1b]2;bytes-{index}\x07").into_bytes();
                chunk.resize(8 * 1024, b'x');
                chunk
            })
            .collect();
        let fixture = Fixture::start(&chunks);
        assert!(matches!(fixture.next_event(), TerminalEvent::Ready));
        assert_eq!(fixture.next_generation(), 0);
        assert_eq!(title_of(fixture.next_event()), "bytes-1");
        assert_eq!(fixture.next_generation(), 2, "two 8 KiB chunks fill a pass");
        assert_eq!(title_of(fixture.next_event()), "bytes-2");
        assert_eq!(fixture.next_generation(), 3);
        let (_, unread) = fixture.finish();
        assert_eq!(unread, None);
    });
}

/// A helper's report taken into a pass is recorded even when the pass then
/// ends because its notices cannot be delivered: each helper reports only
/// once, and closing waits for that report.
///
/// The report is the pump's, carrying a failure the real pump never has:
/// once the session closes, the real pump reports only that it was
/// cancelled, which carries no failure. So the error can reach the
/// outcomes only through the report taken into the pass, whichever
/// order the real reports arrive in.
#[test]
fn a_report_taken_into_a_refused_pass_is_still_recorded() {
    within_watchdog(|| {
        let fixture = Fixture::start_with(vec![
            Message::PtyOutput(pty_unix::OutputChunk::detached(b"\x1b]2;unread\x07")),
            Message::PumpStopped(PumpOutcome::ReadError("injected read failure".into())),
        ]);
        assert!(matches!(fixture.next_event(), TerminalEvent::Ready));
        assert_eq!(fixture.next_generation(), 0);
        // What the waiter does once the child has exited: the title nobody
        // takes is refused two seconds from now, ending the pass. Nothing
        // is read until the worker has finished, so the refusal happens.
        fixture.mailbox.begin_natural_drain();
        if let Some(processes) = fixture.worker.join().expect("the worker did not panic") {
            finish_shutdown(processes, std::time::Instant::now());
        }
        let mut failures = Vec::new();
        while let Ok(event) = fixture.events.next_blocking() {
            if let TerminalEvent::Error(error) = event {
                failures.push(error);
            }
        }
        assert!(
            failures.iter().any(|error| error.operation == "pty_read"
                && error.message.contains("injected read failure")),
            "the report taken into the refused pass was dropped: {failures:?}"
        );
    });
}

/// Commands waiting on the control lane are applied before output that was
/// queued on the inbox earlier: both when the worker picks its first message
/// and when a pass continues. A hyperlink answer carries the generation it was
/// resolved against, so an answer given after the output was parsed says so.
#[test]
fn a_queued_command_is_applied_before_output_that_arrived_first() {
    within_watchdog(|| {
        let output = |text: &str| {
            Message::PtyOutput(pty_unix::OutputChunk::detached(
                format!("{text}\r\n").as_bytes(),
            ))
        };
        let resolve = |request_id| {
            Message::Command(TerminalCommand::ResolveHyperlink {
                position: crate::CellPosition { row: 0, column: 0 },
                request_id,
            })
        };
        let fixture = Fixture::start_with_control(
            vec![output("first"), output("second")],
            vec![resolve(1), resolve(2)],
        );
        assert!(matches!(fixture.next_event(), TerminalEvent::Ready));
        assert_eq!(fixture.next_generation(), 0, "the frame before any output");
        for expected in [1, 2] {
            match fixture.next_event() {
                TerminalEvent::Hyperlink {
                    request_id,
                    generation,
                    ..
                } => assert_eq!(
                    (request_id, generation),
                    (expected, 0),
                    "a command waited behind output that was already queued"
                ),
                other @ (TerminalEvent::Ready
                | TerminalEvent::Exited(_)
                | TerminalEvent::SelectionCopied(_)
                | TerminalEvent::UnsafePaste(_)
                | TerminalEvent::Bell
                | TerminalEvent::TitleChanged(_)
                | TerminalEvent::History { .. }
                | TerminalEvent::HistoryFailed { .. }
                | TerminalEvent::Graphics(_)
                | TerminalEvent::WorkingDirectoryChanged(_)
                | TerminalEvent::ClipboardWrite(_)
                | TerminalEvent::Error(_)) => panic!("expected a hyperlink answer, got {other:?}"),
            }
        }
        assert_eq!(
            fixture.next_generation(),
            2,
            "the output is parsed after the commands"
        );
        fixture.finish();
    });
}
