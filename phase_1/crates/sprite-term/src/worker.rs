//! The terminal-owner worker.
//!
//! One worker thread per Terminal Session owns the PTY master and every
//! libghostty value. Helper threads — the child waiter and the PTY pump —
//! report to it through the same ordered queue and never touch terminal state
//! themselves.

use std::io::Read;
use std::os::fd::RawFd;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender};
use std::thread;

use libghostty_vt::Terminal;
use libghostty_vt::render::{CellIterator, RenderState, RowIterator};
use libghostty_vt::terminal::Options as TerminalOptions;
use portable_pty::{CommandBuilder, ExitStatus, MasterPty, PtySize, native_pty_system};

use crate::pty_unix::Pump;
use crate::snapshot;
use crate::{
    ChildExit, SessionConfig, SessionError, SnapshotBundle, TerminalCommand, TerminalEvent,
    TerminalSize,
};

/// Helper threads get a small explicit stack; they hold no terminal state.
const HELPER_STACK_BYTES: usize = 256 * 1024;

/// How the PTY pump stopped.
pub(crate) enum PumpOutcome {
    Canceled,
    Eof,
    ReadError(String),
}

/// Messages the worker accepts. Application commands and helper-thread reports
/// share one queue so their order is defined.
pub(crate) enum Message {
    /// Task 4 is the first to read the payload; Task 3 only proves it is
    /// carried in order alongside helper-thread reports.
    Command(#[allow(dead_code)] TerminalCommand),
    /// One chunk of PTY output, carrying one output permit.
    PtyOutput(Vec<u8>),
    /// The consumer took a snapshot and is ready for the next one.
    CaptureRequested,
    PumpStopped(PumpOutcome),
    ChildExited(Result<ExitStatus, String>),
    Shutdown,
}

/// The live PTY side of a session, owned solely by the worker.
struct Started {
    master: Box<dyn MasterPty + Send>,
    master_fd: RawFd,
    reader: Box<dyn Read + Send>,
}

pub(crate) fn run(
    config: SessionConfig,
    commands: SyncSender<Message>,
    inbox: Receiver<Message>,
    events: async_channel::Sender<TerminalEvent>,
    snapshots: async_channel::Sender<Arc<SnapshotBundle>>,
    shutdown: Arc<AtomicBool>,
) {
    let started = match start(&config, &commands) {
        Ok(started) => started,
        Err(error) => {
            let _ = events.send_blocking(TerminalEvent::Error(error));
            return;
        }
    };

    // Declared before the pump so it outlives it: the pump borrows this
    // descriptor and is joined when `Pump` drops.
    let Started {
        master,
        master_fd,
        reader,
    } = started;

    let size = config.size;
    let mut terminal = match Terminal::new(TerminalOptions {
        cols: size.cols,
        rows: size.rows,
        max_scrollback: config.max_scrollback,
    }) {
        Ok(terminal) => terminal,
        Err(error) => {
            let _ = events.send_blocking(TerminalEvent::Error(SessionError::new(
                "create_terminal",
                error,
            )));
            return;
        }
    };

    let (mut render_state, mut rows, mut cells) = match render_objects() {
        Ok(objects) => objects,
        Err(error) => {
            let _ = events.send_blocking(TerminalEvent::Error(error));
            return;
        }
    };

    let mut pump = match Pump::start(master_fd, reader, commands.clone()) {
        Ok(pump) => pump,
        Err(error) => {
            let _ = events.send_blocking(TerminalEvent::Error(error));
            return;
        }
    };

    if events.send_blocking(TerminalEvent::Ready).is_err() {
        return;
    }

    // A silent long-running child must still give the application dimensions
    // and cursor state, so generation 0 is published before any output.
    let mut generation = 0_u64;
    let mut dirty = !publish(
        generation,
        size,
        &terminal,
        &mut render_state,
        &mut rows,
        &mut cells,
        &snapshots,
        &events,
    );

    loop {
        if shutdown.load(Ordering::SeqCst) {
            break;
        }

        let Ok(message) = inbox.recv() else {
            break;
        };

        match message {
            Message::PtyOutput(chunk) => {
                // One chunk, one mutation batch, one generation.
                terminal.vt_write(&chunk);
                generation += 1;
                dirty = true;
                pump.return_permit();
            }
            Message::CaptureRequested => {}
            Message::ChildExited(status) => {
                // Reported straight away so a descendant holding the PTY open
                // cannot hide the child's exit, but the loop keeps draining so
                // output already in flight still reaches the application.
                let requested = shutdown.load(Ordering::SeqCst);
                let event = match status {
                    Ok(status) => TerminalEvent::Exited(child_exit(&status, requested)),
                    Err(error) => TerminalEvent::Error(SessionError::new("wait_child", error)),
                };
                if events.send_blocking(event).is_err() {
                    break;
                }
                continue;
            }
            Message::PumpStopped(outcome) => {
                if let PumpOutcome::ReadError(error) = outcome {
                    let _ = events
                        .send_blocking(TerminalEvent::Error(SessionError::new("pty_read", error)));
                }
                break;
            }
            Message::Shutdown => break,
            // Task 4 onward handle application commands here.
            Message::Command(_) => {}
        }

        // Capture only against an empty slot: building a projection that a
        // newer generation would immediately replace wastes the terminal
        // owner's time and delivers nothing.
        if dirty && snapshots.is_empty() {
            dirty = !publish(
                generation,
                size,
                &terminal,
                &mut render_state,
                &mut rows,
                &mut cells,
                &snapshots,
                &events,
            );
        }
    }

    // The final generation still deserves delivery, so the last projection
    // displaces any stale one left in the slot.
    if dirty
        && let Ok(bundle) = snapshot::capture(
            generation,
            size,
            &terminal,
            &mut render_state,
            &mut rows,
            &mut cells,
        )
    {
        let _ = snapshots.force_send(Arc::new(bundle));
    }

    pump.shutdown();
    drop(master);
}

/// Builds one coherent bundle and delivers it. Returns whether it was sent.
#[allow(clippy::too_many_arguments)]
fn publish<'vt>(
    generation: u64,
    size: TerminalSize,
    terminal: &Terminal<'vt, '_>,
    render_state: &mut RenderState<'vt>,
    rows: &mut RowIterator<'vt>,
    cells: &mut CellIterator<'vt>,
    snapshots: &async_channel::Sender<Arc<SnapshotBundle>>,
    events: &async_channel::Sender<TerminalEvent>,
) -> bool {
    match snapshot::capture(generation, size, terminal, render_state, rows, cells) {
        Ok(bundle) => snapshots.try_send(Arc::new(bundle)).is_ok(),
        Err(error) => {
            let _ = events.send_blocking(TerminalEvent::Error(error));
            false
        }
    }
}

type RenderObjects = (
    RenderState<'static>,
    RowIterator<'static>,
    CellIterator<'static>,
);

fn render_objects() -> Result<RenderObjects, SessionError> {
    let render_state =
        RenderState::new().map_err(|error| SessionError::new("create_render_state", error))?;
    let rows =
        RowIterator::new().map_err(|error| SessionError::new("create_row_iterator", error))?;
    let cells =
        CellIterator::new().map_err(|error| SessionError::new("create_cell_iterator", error))?;
    Ok((render_state, rows, cells))
}

/// Opens the PTY, launches the child, and hands the child to its waiter.
fn start(config: &SessionConfig, commands: &SyncSender<Message>) -> Result<Started, SessionError> {
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
    let Some(master_fd) = pair.master.as_raw_fd() else {
        return Err(SessionError::new(
            "open_pty",
            "the PTY master exposed no file descriptor",
        ));
    };

    let reader = pair
        .master
        .try_clone_reader()
        .map_err(|error| SessionError::new("pty_reader", error))?;

    // The parent's slave handle would otherwise hold the PTY open and hide the
    // child's exit.
    drop(pair.slave);

    spawn_child_waiter(child, commands.clone())?;

    Ok(Started {
        master: pair.master,
        master_fd,
        reader,
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
