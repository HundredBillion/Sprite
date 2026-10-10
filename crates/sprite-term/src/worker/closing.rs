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
) -> Escalation {
    let Some(signal) = escalation.due(requested) else {
        return escalation;
    };
    let attempted = processes.signal(&signal);
    escalation.after(signal, attempted)
}

pub(super) fn close(runtime: Runtime) -> Option<pty_unix::SessionProcesses> {
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

    let closing_started = Instant::now();

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
            );
        }

        let settled = exit_status.is_some() && pump_stopped;
        // A requested shutdown is not finished while anything the pane started
        // is still running; a natural exit only owes the single hangup above.
        let descendants_gone = !requested
            || processes
                .as_mut()
                .is_some_and(|processes| !processes.is_alive());
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

pub(crate) fn finish_shutdown(mut processes: pty_unix::SessionProcesses, requested_at: Instant) {
    // The natural close already sent the session its hangup; what remains is
    // TERM and KILL, each counted only once a scan has reached the members.
    let mut escalation = Escalation::Terminate;
    while processes.is_alive() {
        let waited = requested_at.elapsed();
        escalation = escalate(&mut processes, escalation, Some(waited));
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
