//! The window's panes, as the broker reaches them.
//!
//! A pane's session lives on its own worker thread, its view lives on the GPUI
//! thread, and the endpoint answers requests on another thread again. This is
//! the one place those meet, and it is deliberately narrow: a request can be
//! submitted and an answer collected, and nothing here hands out a session, a
//! PTY, or a way to write to a child.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use sprite_term::{CommandSender, HistoryLines, HistorySnapshot, TerminalCommand};

use crate::observation::broker::{PaneAddress, PaneSource, Pending};

/// What a pane sends back when it answers.
type Answer = Result<Arc<HistorySnapshot>, String>;
use crate::pane_tree::{PaneId, Rect};
use crate::tabs::TabId;

/// What a pane needs in order to be observable.
///
/// Held by the pane's view, which is the single consumer of its session's
/// events and therefore the only thing able to forward an answer.
#[derive(Clone)]
pub struct PaneLink {
    pub pane: PaneId,
    pub tab: TabId,
    pub panes: Arc<WindowPanes>,
}

/// One registered pane.
struct Entry {
    tab: TabId,
    /// Where the pane sits, refreshed from the window as the layout changes.
    ///
    /// Held here rather than asked for at request time because the layout lives
    /// on the GPUI thread, and a request must never have to wait for a frame.
    placement: Placement,
    commands: CommandSender,
    /// Requests submitted and not yet answered, oldest first.
    ///
    /// A queue rather than a single slot because two clients may ask the same
    /// pane at once. The worker handles commands in order and answers each
    /// exactly once, and the view forwards answers in the order they arrive, so
    /// matching the oldest waiter to the next answer pairs them correctly.
    waiting: VecDeque<std::sync::mpsc::Sender<Answer>>,
}

/// Where a pane sits in the window, as the schema reports it.
#[derive(Clone, Copy, Debug)]
pub struct Placement {
    pub tab_order: usize,
    pub rect: Rect,
    pub focused: bool,
}

impl Default for Placement {
    fn default() -> Self {
        Self {
            tab_order: 0,
            rect: Rect::FULL,
            focused: false,
        }
    }
}

/// Every pane in one window that observation may reach.
#[derive(Default)]
pub struct WindowPanes {
    entries: Mutex<HashMap<PaneId, Entry>>,
}

impl WindowPanes {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Records a pane, so requests can reach it.
    pub fn register(&self, pane: PaneId, tab: TabId, commands: CommandSender) {
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        entries.insert(
            pane,
            Entry {
                tab,
                // Corrected by the next layout the window publishes; a pane
                // that has not been laid out yet still has to be answerable.
                placement: Placement::default(),
                commands,
                waiting: VecDeque::new(),
            },
        );
    }

    /// Records where the window's panes currently sit.
    ///
    /// Published by the window as the layout changes. Panes the window no
    /// longer has are ignored rather than added back: this reports placement,
    /// not membership.
    pub fn set_layout(&self, placements: &[(PaneId, Placement)]) {
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        for (pane, placement) in placements {
            if let Some(entry) = entries.get_mut(pane) {
                entry.placement = *placement;
            }
        }
    }

    /// Forgets a pane that has closed.
    ///
    /// Anyone still waiting on it is released rather than left to time out: the
    /// pane is known to be gone, so making a caller wait out the deadline for
    /// it would be a lie about what is happening.
    pub fn forget(&self, pane: PaneId) {
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(mut entry) = entries.remove(&pane) {
            for waiter in entry.waiting.drain(..) {
                let _ = waiter.send(Err("the pane closed before it answered".to_owned()));
            }
        }
    }

    /// Hands one pane's answer to whoever asked for it first.
    ///
    /// Called from the view, which is the single consumer of a session's
    /// events. An answer nobody is waiting for is dropped: it belongs to a
    /// request that has already given up.
    pub fn deliver(&self, pane: PaneId, snapshot: Arc<HistorySnapshot>) {
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(entry) = entries.get_mut(&pane)
            && let Some(waiter) = entry.waiting.pop_front()
        {
            let _ = waiter.send(Ok(snapshot));
        }
    }

    /// Reports that a pane failed, to whoever asked for it first.
    pub fn deliver_failure(&self, pane: PaneId, reason: String) {
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(entry) = entries.get_mut(&pane)
            && let Some(waiter) = entry.waiting.pop_front()
        {
            let _ = waiter.send(Err(reason));
        }
    }
}

impl PaneSource for WindowPanes {
    fn panes(&self) -> Vec<PaneAddress> {
        let entries = self
            .entries
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let mut addresses: Vec<PaneAddress> = entries
            .iter()
            .map(|(pane, entry)| PaneAddress {
                tab: entry.tab,
                tab_order: entry.placement.tab_order,
                pane: *pane,
                rect: entry.placement.rect,
                focused: entry.placement.focused,
            })
            .collect();
        // A map has no order, and a caller must not see panes shuffle between
        // requests. This is the schema's order: tabs by window order, then
        // panes by top edge, then left edge, then identity.
        addresses.sort_by(|left, right| {
            left.tab_order
                .cmp(&right.tab_order)
                .then(left.rect.y.total_cmp(&right.rect.y))
                .then(left.rect.x.total_cmp(&right.rect.x))
                .then(left.pane.cmp(&right.pane))
        });
        addresses
    }

    fn begin(&self, pane: PaneId, lines: HistoryLines) -> Result<Pending, String> {
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let entry = entries
            .get_mut(&pane)
            .ok_or_else(|| "the pane closed before it could be asked".to_owned())?;

        let (sender, answer) = std::sync::mpsc::channel();
        // Queued before the command is sent, so an answer cannot arrive before
        // there is anyone recorded to receive it.
        entry.waiting.push_back(sender);
        let address = PaneAddress {
            tab: entry.tab,
            tab_order: entry.placement.tab_order,
            pane,
            rect: entry.placement.rect,
            focused: entry.placement.focused,
        };
        if let Err(error) = entry.commands.send(TerminalCommand::CaptureHistory(lines)) {
            entry.waiting.pop_back();
            return Err(error.to_string());
        }
        Ok(Pending { address, answer })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sprite_term::{PaneRow, PromptKind, ScreenKind, SessionConfig, TerminalSession};
    use std::time::Duration;

    fn snapshot() -> Arc<HistorySnapshot> {
        Arc::new(HistorySnapshot {
            generation: 1,
            size: sprite_term::TerminalSize::DEFAULT,
            screen: ScreenKind::Primary,
            rows: vec![PaneRow {
                text: "answer".to_owned(),
                wrapped: false,
                prompt: PromptKind::None,
            }],
            history_rows: 0,
            requested: 0,
            available: 0,
            cursor: sprite_term::CursorSnapshot {
                row: 0,
                column: 0,
                visible: true,
                blinking: false,
            },
            viewport: sprite_term::Viewport {
                total_rows: 24,
                offset: 0,
                visible_rows: 24,
            },
            title: None,
            working_directory: None,
            captured_at_unix_ms: 1_800_000_000_000,
            foreground: None,
        })
    }

    /// A real session, because a `CommandSender` cannot be fabricated — it is
    /// only obtainable from one. The child does nothing; the test never waits
    /// for it to answer, only for the plumbing around it.
    fn session() -> TerminalSession {
        TerminalSession::spawn(SessionConfig::command(
            "/bin/sh",
            vec!["-c".into(), "sleep 30".into()],
        ))
        .expect("spawn a session")
    }

    #[test]
    fn a_registered_pane_is_listed_in_a_stable_order() {
        let panes = WindowPanes::new();
        let first = session();
        let second = session();
        panes.register(PaneId(5), TabId(1), first.commands());
        panes.register(PaneId(2), TabId(0), second.commands());

        let listed = panes.panes();
        let order: Vec<(u64, u64)> = listed
            .iter()
            .map(|address| (address.tab.0, address.pane.0))
            .collect();
        assert_eq!(
            order,
            vec![(0, 2), (1, 5)],
            "ordered by the window's layout, never by hash order"
        );
    }

    #[test]
    fn an_unregistered_pane_cannot_be_asked() {
        let panes = WindowPanes::new();
        let outcome = panes.begin(PaneId(1), HistoryLines::default());
        assert!(outcome.is_err(), "a pane the window does not have");
    }

    #[test]
    fn an_answer_reaches_the_caller_that_asked_for_it() {
        let panes = WindowPanes::new();
        let session = session();
        panes.register(PaneId(0), TabId(0), session.commands());

        let pending = panes
            .begin(PaneId(0), HistoryLines::default())
            .expect("asked");
        panes.deliver(PaneId(0), snapshot());

        let answer = pending
            .answer
            .recv_timeout(Duration::from_secs(1))
            .expect("an answer arrived");
        assert_eq!(answer.expect("a snapshot").rows[0].text, "answer");
    }

    /// Two callers asking one pane at once must each get an answer, and must
    /// not both be handed the same one while the other waits forever.
    #[test]
    fn concurrent_requests_for_one_pane_are_answered_in_order() {
        let panes = WindowPanes::new();
        let session = session();
        panes.register(PaneId(0), TabId(0), session.commands());

        let first = panes
            .begin(PaneId(0), HistoryLines::default())
            .expect("asked");
        let second = panes
            .begin(PaneId(0), HistoryLines::default())
            .expect("asked");

        panes.deliver(PaneId(0), snapshot());
        panes.deliver(PaneId(0), snapshot());

        assert!(
            first
                .answer
                .recv_timeout(Duration::from_secs(1))
                .expect("first answered")
                .is_ok()
        );
        assert!(
            second
                .answer
                .recv_timeout(Duration::from_secs(1))
                .expect("second answered")
                .is_ok()
        );
    }

    /// A pane that closes releases its waiters immediately. Making them wait
    /// out the deadline would report "did not answer in time" for something
    /// already known to be gone.
    #[test]
    fn forgetting_a_pane_releases_whoever_was_waiting() {
        let panes = WindowPanes::new();
        let session = session();
        panes.register(PaneId(0), TabId(0), session.commands());
        let pending = panes
            .begin(PaneId(0), HistoryLines::default())
            .expect("asked");

        panes.forget(PaneId(0));

        let answer = pending
            .answer
            .recv_timeout(Duration::from_secs(1))
            .expect("released rather than left waiting");
        assert!(answer.unwrap_err().contains("closed"));
        assert!(panes.panes().is_empty());
    }

    #[test]
    fn an_answer_nobody_is_waiting_for_is_discarded() {
        let panes = WindowPanes::new();
        let session = session();
        panes.register(PaneId(0), TabId(0), session.commands());

        // No request outstanding: this must not panic, grow a queue, or be
        // handed to the next caller as a stale answer.
        panes.deliver(PaneId(0), snapshot());

        let pending = panes
            .begin(PaneId(0), HistoryLines::default())
            .expect("asked");
        assert!(
            pending.answer.try_recv().is_err(),
            "a later request does not receive an earlier abandoned answer"
        );
    }
}
