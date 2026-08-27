//! What a pane is running, answerable without asking its worker.
//!
//! A pane must not close a program out from under somebody. The question
//! "is anything running here?" therefore has to be answerable *at the moment a
//! key is pressed*, not a round-trip later: a worker busy applying a flood of
//! output would answer late, and a confirmation that appears after the pane has
//! closed is no confirmation at all.
//!
//! So it is answered from the kernel instead. Every terminal has a foreground
//! process group — the one that receives a Ctrl+C — and comparing it against
//! the group the pane's own shell was put in says whether the shell is sitting
//! at a prompt or waiting on something it started. That is the same question a
//! shell asks itself, and it needs nothing from the worker thread.

use std::os::fd::{OwnedFd, RawFd};
use std::sync::OnceLock;

use crate::pty_unix;

/// What is in the foreground of a pane's terminal.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ForegroundState {
    /// Nothing but the pane's own shell, sitting at a prompt.
    Idle,
    /// A program the shell started, with its name when it can be read safely.
    ///
    /// The name is a basename read from the kernel — never arguments, never
    /// environment — and `None` rather than a guess when it cannot be had.
    Busy(Option<String>),
    /// The platform could not be asked: too early, or a PTY that answers no
    /// such question.
    ///
    /// Deliberately distinct from `Idle`. "I do not know" must not be allowed
    /// to read as "nothing is running", or an unanswerable case becomes a
    /// silently closed pane.
    Unknown,
}

impl ForegroundState {
    /// Whether closing this pane should ask first.
    ///
    /// `Unknown` does not: a terminal that could never answer would otherwise
    /// prompt on every close, and a prompt nobody can ever dismiss by fixing
    /// the situation is a prompt people learn to dismiss without reading.
    pub fn should_confirm(&self) -> bool {
        matches!(self, Self::Busy(_))
    }

    /// The program's name, when one is known.
    pub fn program(&self) -> Option<&str> {
        match self {
            Self::Busy(name) => name.as_deref(),
            _ => None,
        }
    }
}

struct Attached {
    /// A private duplicate, so the question survives the worker's own copy.
    master: OwnedFd,
    /// The group the pane's shell was put in at spawn.
    shell_group: i32,
}

/// A handle on the "what is running" question for one pane.
///
/// Shared between the session and its worker: the worker attaches the PTY once
/// it exists, and the application asks whenever it needs to know.
#[derive(Default)]
pub struct ForegroundWatch {
    attached: OnceLock<Attached>,
}

impl ForegroundWatch {
    /// Called once by the worker, as soon as the PTY and child exist.
    ///
    /// A session whose shell has no process group is left unattached, and
    /// answers `Unknown` forever — which is honest: without the group there is
    /// nothing to compare against.
    pub(crate) fn attach(&self, master_fd: RawFd, shell_group: Option<i32>) {
        let Some(shell_group) = shell_group else {
            return;
        };
        let Some(master) = pty_unix::duplicate(master_fd) else {
            return;
        };
        let _ = self.attached.set(Attached {
            master,
            shell_group,
        });
    }

    /// Asks the kernel what is in the foreground of this pane, right now.
    pub fn state(&self) -> ForegroundState {
        let Some(attached) = self.attached.get() else {
            return ForegroundState::Unknown;
        };
        let Some(group) = pty_unix::foreground_group(&attached.master) else {
            return ForegroundState::Unknown;
        };
        if group == attached.shell_group {
            return ForegroundState::Idle;
        }
        // A group the terminal still names but nothing is left in belongs to a
        // program that has already finished; there is nothing to lose by
        // closing.
        if !pty_unix::group_is_alive(group) {
            return ForegroundState::Idle;
        }
        ForegroundState::Busy(executable_name(group))
    }
}

impl std::fmt::Debug for ForegroundWatch {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ForegroundWatch")
            .field("attached", &self.attached.get().is_some())
            .finish()
    }
}

/// The basename of a process, and nothing else about it.
///
/// `/proc/<pid>/comm` holds a name and nothing more. The arguments and the
/// environment sit beside it and are deliberately not read: a pane needs to say
/// *what* is running, never with what secrets on its command line.
fn executable_name(pid: i32) -> Option<String> {
    let comm = std::fs::read_to_string(format!("/proc/{pid}/comm")).ok()?;
    let name = comm.trim();
    (!name.is_empty()).then(|| name.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unattached_watch_knows_that_it_does_not_know() {
        let watch = ForegroundWatch::default();
        assert_eq!(watch.state(), ForegroundState::Unknown);
        assert!(
            !watch.state().should_confirm(),
            "not knowing must not mean prompting forever"
        );
    }

    #[test]
    fn a_shell_without_a_group_stays_unattached() {
        let watch = ForegroundWatch::default();
        watch.attach(0, None);
        assert_eq!(watch.state(), ForegroundState::Unknown);
    }

    #[test]
    fn only_a_running_program_asks_for_confirmation() {
        assert!(ForegroundState::Busy(Some("vim".to_owned())).should_confirm());
        assert!(ForegroundState::Busy(None).should_confirm());
        assert!(!ForegroundState::Idle.should_confirm());
        assert!(!ForegroundState::Unknown.should_confirm());
        assert_eq!(
            ForegroundState::Busy(Some("vim".to_owned())).program(),
            Some("vim")
        );
        assert_eq!(ForegroundState::Idle.program(), None);
    }
}
