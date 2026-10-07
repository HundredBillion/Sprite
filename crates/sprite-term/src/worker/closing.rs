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

    // A hangup is the polite request every well-behaved program honours.
    if let Some(processes) = &mut processes {
        processes.signal(&GroupSignal::Hangup);
    }
    let mut escalation = 1_u8;

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
        if let Some(since) = requested_at {
            let waited = since.elapsed();
            if waited >= KILL_AFTER {
                if let Some(processes) = &mut processes {
                    processes.signal(&GroupSignal::Kill);
                }
                escalation = 3;
            } else if waited >= TERM_AFTER && escalation < 2 {
                if let Some(processes) = &mut processes {
                    processes.signal(&GroupSignal::Terminate);
                }
                escalation = 2;
            }
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
    events.seal(outcomes);

    drop(master);
    // Natural completion leaves ordinary jobs for a later explicit owner;
    // an attempted explicit cleanup must never spend its budget twice.
    if requested_at.is_none() {
        processes
    } else {
        None
    }
}

pub(crate) fn finish_shutdown(mut processes: pty_unix::SessionProcesses, requested_at: Instant) {
    let mut terminated = false;
    while processes.is_alive() {
        let waited = requested_at.elapsed();
        if waited >= KILL_AFTER {
            processes.signal(&GroupSignal::Kill);
        } else if waited >= TERM_AFTER && !terminated {
            processes.signal(&GroupSignal::Terminate);
            terminated = true;
        }
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
