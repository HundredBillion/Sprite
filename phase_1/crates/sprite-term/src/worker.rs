//! The terminal-owner worker.
//!
//! One worker thread per Terminal Session owns the PTY master and, from Task 3
//! onward, every libghostty value. Helper threads report to it through the same
//! ordered queue and never touch terminal state themselves.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender};
use std::thread;

use portable_pty::{CommandBuilder, ExitStatus, MasterPty, PtySize, native_pty_system};

use crate::{ChildExit, SessionConfig, SessionError, TerminalCommand, TerminalEvent};

/// Helper threads get a small explicit stack; they hold no terminal state.
const HELPER_STACK_BYTES: usize = 256 * 1024;

/// Messages the worker accepts. Application commands and helper-thread reports
/// share one queue so their order is defined.
pub(crate) enum Message {
    /// Task 4 is the first to read the payload; Task 2 only proves it is
    /// carried in order alongside helper-thread reports.
    Command(#[allow(dead_code)] TerminalCommand),
    ChildExited(Result<ExitStatus, String>),
    Shutdown,
}

/// The live PTY side of a session, owned solely by the worker.
struct Session {
    #[allow(dead_code)]
    master: Box<dyn MasterPty + Send>,
}

pub(crate) fn run(
    config: SessionConfig,
    commands: SyncSender<Message>,
    inbox: Receiver<Message>,
    events: async_channel::Sender<TerminalEvent>,
    _snapshots: async_channel::Sender<Arc<crate::SnapshotBundle>>,
    shutdown: Arc<AtomicBool>,
) {
    let _session = match start(&config, &commands) {
        Ok(session) => session,
        Err(error) => {
            let _ = events.send_blocking(TerminalEvent::Error(error));
            return;
        }
    };

    if events.send_blocking(TerminalEvent::Ready).is_err() {
        return;
    }

    serve(inbox, &events, &shutdown);
}

/// Opens the PTY, launches the child, and hands the child to its waiter.
fn start(config: &SessionConfig, commands: &SyncSender<Message>) -> Result<Session, SessionError> {
    let size = config.size;
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: size.rows,
            cols: size.cols,
            pixel_width: size.pixel_width(),
            pixel_height: size.pixel_height(),
        })
        .map_err(|error| SessionError::new("open_pty", error))?;

    let mut command = CommandBuilder::new(&config.program);
    for argument in &config.args {
        command.arg(argument);
    }
    if let Some(directory) = &config.working_directory {
        command.cwd(directory);
    }
    for (key, value) in &config.environment {
        command.env(key, value);
    }

    let child = pair
        .slave
        .spawn_command(command)
        .map_err(|error| SessionError::new("spawn_child", error))?;

    // Both identifiers must exist before the child is handed off: process-group
    // shutdown and pump cancellation depend on them, and discovering a missing
    // one later would mean weakening either guarantee.
    if child.process_id().is_none() {
        return Err(SessionError::new(
            "spawn_child",
            "the child reported no process id",
        ));
    }
    if pair.master.as_raw_fd().is_none() {
        return Err(SessionError::new(
            "open_pty",
            "the PTY master exposed no file descriptor",
        ));
    }

    // The parent's slave handle would otherwise hold the PTY open and hide the
    // child's exit.
    drop(pair.slave);

    spawn_child_waiter(child, commands.clone())?;

    Ok(Session {
        master: pair.master,
    })
}

/// Blocks in `Child::wait` off the worker so a quiet exit is reaped without a
/// timer, and descendants holding the PTY open cannot mask it.
fn spawn_child_waiter(
    mut child: Box<dyn portable_pty::Child + Send + Sync>,
    commands: SyncSender<Message>,
) -> Result<(), SessionError> {
    thread::Builder::new()
        .name("sprite-term-child-waiter".to_owned())
        .stack_size(HELPER_STACK_BYTES)
        .spawn(move || {
            let status = child.wait().map_err(|error| error.to_string());
            let _ = commands.send(Message::ChildExited(status));
        })
        .map(|_| ())
        .map_err(|error| SessionError::new("spawn_child_waiter", error))
}

/// The worker's steady state: block in `recv`, never poll.
fn serve(
    inbox: Receiver<Message>,
    events: &async_channel::Sender<TerminalEvent>,
    shutdown: &Arc<AtomicBool>,
) {
    loop {
        if shutdown.load(Ordering::SeqCst) {
            return;
        }

        let Ok(message) = inbox.recv() else {
            return;
        };

        match message {
            Message::ChildExited(status) => {
                let requested = shutdown.load(Ordering::SeqCst);
                let event = match status {
                    Ok(status) => TerminalEvent::Exited(child_exit(&status, requested)),
                    Err(error) => TerminalEvent::Error(SessionError::new("wait_child", error)),
                };
                let _ = events.send_blocking(event);
                return;
            }
            Message::Shutdown => return,
            // Task 3 onward handle application commands here.
            Message::Command(_) => {}
        }
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
