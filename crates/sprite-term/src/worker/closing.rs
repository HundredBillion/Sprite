use super::*;
use crate::pty_unix::GroupSignal;
use std::sync::mpsc::RecvTimeoutError;
use std::time::{Duration, Instant};
/// The bounded shutdown policy, measured from the moment shutdown is actually
/// requested — not from the start of Closing. A pane whose child exits on its
/// own may not be asked to shut down until much later, and starting the clock
/// at Closing would spend the whole budget before the request arrives.
const TERM_AFTER: Duration = Duration::from_secs(2);
const KILL_AFTER: Duration = Duration::from_secs(3);

/// Cleanup stops waiting here even if a group somehow survives KILL, so a
/// worker can never hang forever.
const GIVE_UP_AFTER: Duration = Duration::from_secs(6);

/// Short enough that escalation deadlines are re-checked promptly even under
/// continuous output.
const CLOSING_SLICE: Duration = Duration::from_millis(50);

/// The next signal escalation owes. It advances only once a signal was
/// actually attempted against every member a complete scan found: a scan that
/// could not finish signalled nobody, and counting it would spend the hangup
/// or TERM on no one and leave only KILL for a program that would have
/// honoured a politer request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Escalation {
    /// The hangup has not reached every group of the session yet. Each
    /// group is hung up once, however many attempts that takes.
    Hangup,
    /// The hangup was attempted; TERM is owed two seconds after a request.
    Terminate,
    /// TERM was attempted; only KILL remains, three seconds after a request.
    Kill,
}

impl Escalation {
    /// The signal due now, given how long ago shutdown was requested, if it was.
    ///
    /// KILL is due on every pass once its deadline passes, so a group created
    /// after the previous scan is still reached before the budget ends.
    fn due(self, requested: Option<Duration>) -> Option<GroupSignal> {
        match (self, requested) {
            (_, Some(waited)) if waited >= KILL_AFTER => Some(GroupSignal::Kill),
            (Self::Hangup, _) => Some(GroupSignal::Hangup),
            (Self::Terminate, Some(waited)) if waited >= TERM_AFTER => Some(GroupSignal::Terminate),
            _ => None,
        }
    }

    /// Where escalation stands after `signal`: past it only if it was attempted.
    fn after(self, signal: GroupSignal, attempted: bool) -> Self {
        match (attempted, signal) {
            (false, _) => self,
            (true, GroupSignal::Hangup) => Self::Terminate,
            (true, GroupSignal::Terminate | GroupSignal::Kill) => Self::Kill,
        }
    }
}

/// Sends whatever is due and reports where escalation then stands.
fn escalate(
    processes: &mut pty_unix::SessionProcesses,
    escalation: Escalation,
    requested: Option<Duration>,
    signal_processes: &mut impl FnMut(&mut pty_unix::SessionProcesses, &GroupSignal) -> bool,
) -> Escalation {
    let Some(signal) = escalation.due(requested) else {
        return escalation;
    };
    let attempted = signal_processes(processes, &signal);
    escalation.after(signal, attempted)
}

pub(super) fn close(runtime: Runtime) -> Option<pty_unix::SessionProcesses> {
    close_with_signal(runtime, pty_unix::SessionProcesses::signal)
}

fn close_with_signal(
    runtime: Runtime,
    signal_processes: impl FnMut(&mut pty_unix::SessionProcesses, &GroupSignal) -> bool,
) -> Option<pty_unix::SessionProcesses> {
    close_with_signal_since(runtime, signal_processes, Instant::now())
}

fn close_with_signal_since(
    runtime: Runtime,
    mut signal_processes: impl FnMut(&mut pty_unix::SessionProcesses, &GroupSignal) -> bool,
    closing_started: Instant,
) -> Option<pty_unix::SessionProcesses> {
    let Runtime {
        started:
            Started {
                master,
                master_fd: _,
                process_group: _,
                mut processes,
                waiter,
            },
        mut pump,
        inbox,
        control: _,
        events,
        shutdown,
        foreground,
        mut exit_status,
        mut pump_stopped,
        mut fatal,
    } = runtime;
    // The pump may be parked on a PTY that a descendant keeps open forever, so
    // it is woken now rather than waited on.
    if let Some(pump) = &pump {
        pump.cancel();
    }

    // The first pass below sends the hangup every well-behaved program
    // honours; escalation moves past a step only once it reached the members.
    let mut escalation = Escalation::Hangup;

    // Set the first time the flag is observed, so every escalation deadline is
    // relative to the request rather than to the child's exit.
    let mut requested_at: Option<Instant> = None;

    loop {
        // Read the flag every pass: a session may be told to shut down after
        // its child has already exited on its own.
        let requested = shutdown.load(Ordering::SeqCst);
        if requested && requested_at.is_none() {
            requested_at = Some(Instant::now());
        }

        // Checked before every receive, so continuous output cannot postpone
        // escalation past its deadline.
        if let Some(processes) = &mut processes {
            escalation = escalate(
                processes,
                escalation,
                requested_at.map(|since| since.elapsed()),
                &mut signal_processes,
            );
        }

        let settled = exit_status.is_some() && pump_stopped;
        let hangup_pending = processes.is_some() && escalation == Escalation::Hangup;
        // Natural exit retries an incomplete HUP before publishing its outcome.
        // Explicit shutdown also waits for owned jobs to exit.
        let descendants_gone = if requested {
            processes
                .as_mut()
                .is_none_or(|processes| !processes.is_alive())
        } else {
            !hangup_pending
        };
        // A pane that was never asked to shut down still may not hang forever,
        // so an unrequested close keeps its own deadline from Closing.
        let exhausted = match requested_at {
            Some(since) => since.elapsed() >= GIVE_UP_AFTER,
            None => closing_started.elapsed() >= GIVE_UP_AFTER,
        };
        if (settled && descendants_gone) || exhausted {
            break;
        }

        match inbox.recv_timeout(CLOSING_SLICE) {
            Ok(Message::ChildExited(status)) => exit_status = Some(status),
            Ok(Message::PumpStopped(outcome)) => {
                if let Some(error) = pump_failure(outcome) {
                    fatal.get_or_insert(error);
                }
                pump_stopped = true;
            }
            Ok(_) => {}
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) if !requested && hangup_pending => {
                // Reports can settle before HUP succeeds; avoid spinning on a closed inbox.
                std::thread::sleep(CLOSING_SLICE);
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }

    // The loop above can exit on its deadline without having seen PumpStopped.
    // In that case the pump may be blocked sending into a full worker queue,
    // and joining it while nothing drains would deadlock: its send waits for
    // room, our join waits for its send.
    //
    // Joining ensures no I/O helper outlives the session's shutdown report.
    // The queue must be drained before joining so the pump can finish its send.
    if !pump_stopped {
        if let Some(pump) = &pump {
            pump.cancel();
        }
        let drain_deadline = Instant::now() + GIVE_UP_AFTER;
        while !pump_stopped && Instant::now() < drain_deadline {
            match inbox.recv_timeout(CLOSING_SLICE) {
                Ok(Message::PumpStopped(outcome)) => {
                    if let Some(error) = pump_failure(outcome) {
                        fatal.get_or_insert(error);
                    }
                    pump_stopped = true;
                }
                Ok(_) => {}
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
    }

    if let Some(pump) = &mut pump {
        pump.shutdown();
    }
    if exit_status.is_some() {
        let _ = waiter.join();
    }

    // Only now, with every helper thread finished and the descendant policy
    // complete, does the application hear how the session ended.
    let mut outcomes = Vec::with_capacity(2);
    if let Some(error) = fatal {
        outcomes.push(TerminalEvent::Error(error));
    }
    let requested = shutdown.load(Ordering::SeqCst);
    match exit_status {
        Some(Ok(status)) => outcomes.push(TerminalEvent::Exited(child_exit(&status, requested))),
        Some(Err(error)) => {
            outcomes.push(TerminalEvent::Error(SessionError::new("wait_child", error)))
        }
        None if requested => outcomes.push(TerminalEvent::Exited(ChildExit {
            code: None,
            signal: None,
            requested: true,
        })),
        None => {}
    }
    // Every descriptor this session holds on the PTY master closes before the
    // outcome is published: the worker's own and the duplicate kept for
    // foreground questions. A descendant still holding the terminal then sees
    // it hang up, and whoever is told the session ended can rely on that.
    drop(master);
    foreground.detach();
    events.seal(outcomes);
    // Natural completion leaves ordinary jobs for a later explicit owner;
    // an attempted explicit cleanup must never spend its budget twice.
    if requested_at.is_none() {
        processes
    } else {
        None
    }
}

pub(crate) fn finish_shutdown(processes: pty_unix::SessionProcesses, requested_at: Instant) {
    finish_shutdown_with(
        processes,
        requested_at,
        pty_unix::SessionProcesses::signal,
        pty_unix::SessionProcesses::is_alive,
    );
}

fn finish_shutdown_with(
    mut processes: pty_unix::SessionProcesses,
    requested_at: Instant,
    mut signal_processes: impl FnMut(&mut pty_unix::SessionProcesses, &GroupSignal) -> bool,
    mut is_alive: impl FnMut(&mut pty_unix::SessionProcesses) -> bool,
) {
    // Natural close may have retained an incomplete hangup attempt.
    // SessionProcesses remembers reached groups, so retrying skips them.
    let mut escalation = Escalation::Hangup;
    while is_alive(&mut processes) {
        let waited = requested_at.elapsed();
        escalation = escalate(
            &mut processes,
            escalation,
            Some(waited),
            &mut signal_processes,
        );
        if waited >= GIVE_UP_AFTER {
            break;
        }
        std::thread::sleep(CLOSING_SLICE.min(GIVE_UP_AFTER.saturating_sub(waited)));
    }
}

/// Reports one cause, never two: a signalled child has no exit code.
fn child_exit(status: &ExitStatus, requested: bool) -> ChildExit {
    match status.signal() {
        Some(signal) => ChildExit {
            code: None,
            signal: Some(signal.to_owned()),
            requested,
        },
        None => ChildExit {
            code: Some(status.exit_code()),
            signal: None,
            requested,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settled_runtime() -> Runtime {
        let pair = portable_pty::native_pty_system()
            .openpty(portable_pty::PtySize::default())
            .unwrap();
        let (_, inbox) = std::sync::mpsc::sync_channel(1);
        let (_, control) = std::sync::mpsc::sync_channel(1);
        let (events, _) = crate::event_mailbox::bounded(0);
        Runtime {
            started: Started {
                master: pair.master,
                master_fd: -1,
                process_group: None,
                processes: pty_unix::SessionProcesses::capture(i32::MAX as u32),
                waiter: std::thread::spawn(|| {}),
            },
            pump: None,
            inbox,
            control,
            events,
            shutdown: Arc::new(AtomicBool::new(false)),
            foreground: Arc::new(crate::ForegroundWatch::default()),
            exit_status: Some(Ok(ExitStatus::with_exit_code(0))),
            pump_stopped: true,
            fatal: None,
        }
    }

    #[test]
    fn natural_exit_retries_transient_hangup_before_publishing_outcome() {
        let mut runtime = settled_runtime();
        let (events, receiver) = crate::event_mailbox::bounded(0);
        runtime.events = Arc::clone(&events);
        let mut attempted = Vec::new();
        let processes = close_with_signal(runtime, |_, signal| {
            assert!(
                events.producer_ready(),
                "outcome must wait for the hangup retry"
            );
            attempted.push(*signal);
            attempted.len() == 2
        });
        assert_eq!(
            attempted,
            vec![GroupSignal::Hangup, GroupSignal::Hangup],
            "settled natural close must retry a transient failed hangup before returning"
        );
        assert!(processes.is_some());
        assert!(matches!(
            receiver.next_blocking(),
            Ok(TerminalEvent::Exited(_))
        ));
    }

    #[test]
    fn natural_exit_preserves_failed_hangup_for_explicit_cleanup() {
        let mut attempted = Vec::new();
        let processes = close_with_signal_since(
            settled_runtime(),
            |_, signal| {
                attempted.push(*signal);
                assert_eq!(attempted.len(), 1, "exhausted natural close must return");
                false
            },
            Instant::now() - GIVE_UP_AFTER,
        )
        .expect("natural exit retains cleanup ownership after a failed scan");
        assert_eq!(attempted, vec![GroupSignal::Hangup]);
        let mut alive = [true, false].into_iter();
        finish_shutdown_with(
            processes,
            Instant::now(),
            |_, signal| {
                attempted.push(*signal);
                true
            },
            |_| {
                alive
                    .next()
                    .expect("cleanup must finish after the job exits")
            },
        );
        assert_eq!(
            attempted,
            vec![GroupSignal::Hangup, GroupSignal::Hangup],
            "explicit cleanup must retry the hangup the natural-exit scan could not deliver"
        );
    }

    #[test]
    fn settled_close_without_processes_does_not_wait_for_cleanup_deadline() {
        for requested in [false, true] {
            let mut runtime = settled_runtime();
            runtime.started.processes = None;
            runtime.shutdown.store(requested, Ordering::SeqCst);
            let (sender, inbox) = std::sync::mpsc::sync_channel(1);
            runtime.inbox = inbox;
            let started = Instant::now();
            assert!(close_with_signal(runtime, |_, _| panic!("no process scope")).is_none());
            assert!(
                started.elapsed() < GIVE_UP_AFTER,
                "missing scope must not spend the budget"
            );
            drop(sender);
        }
    }

    #[test]
    fn retained_cleanup_gives_up_when_scans_keep_failing() {
        let processes = close_with_signal_since(
            settled_runtime(),
            |_, _| false,
            Instant::now() - GIVE_UP_AFTER,
        )
        .unwrap();
        let mut attempted = Vec::new();
        let mut checks = 0;
        finish_shutdown_with(
            processes,
            Instant::now() - GIVE_UP_AFTER,
            |_, signal| {
                attempted.push(*signal);
                false
            },
            |_| {
                checks += 1;
                assert_eq!(checks, 1, "an exhausted cleanup must not keep polling");
                true
            },
        );
        assert_eq!(attempted, vec![GroupSignal::Kill]);
    }

    #[test]
    fn escalation_advances_only_past_a_signal_that_was_attempted() {
        let mut escalation = Escalation::Hangup;
        assert_eq!(escalation.due(None), Some(GroupSignal::Hangup));
        escalation = escalation.after(GroupSignal::Hangup, false);
        assert_eq!(
            escalation.due(None),
            Some(GroupSignal::Hangup),
            "a hangup no scan delivered is still owed"
        );
        escalation = escalation.after(GroupSignal::Hangup, true);
        assert_eq!(
            escalation.due(None),
            None,
            "a natural close owes nothing more"
        );
        assert_eq!(
            escalation.due(Some(TERM_AFTER - Duration::from_millis(1))),
            None
        );
        assert_eq!(
            escalation.due(Some(TERM_AFTER)),
            Some(GroupSignal::Terminate)
        );
        escalation = escalation.after(GroupSignal::Terminate, false);
        assert_eq!(
            escalation.due(Some(TERM_AFTER + CLOSING_SLICE)),
            Some(GroupSignal::Terminate),
            "a TERM nobody received is tried again"
        );
        escalation = escalation.after(GroupSignal::Terminate, true);
        assert_eq!(escalation.due(Some(TERM_AFTER + CLOSING_SLICE)), None);
        assert_eq!(escalation.due(Some(KILL_AFTER)), Some(GroupSignal::Kill));
        assert_eq!(
            escalation
                .after(GroupSignal::Kill, true)
                .due(Some(KILL_AFTER)),
            Some(GroupSignal::Kill),
            "KILL repeats until the scope is empty"
        );
    }

    #[test]
    fn kill_is_due_at_its_deadline_even_when_earlier_steps_never_landed() {
        assert_eq!(
            Escalation::Hangup.due(Some(KILL_AFTER)),
            Some(GroupSignal::Kill)
        );
    }
}
