//! The window's panes, as the broker reaches them.
//!
//! A pane's session lives on its own worker thread, its view lives on the GPUI
//! thread, and the endpoint answers requests on another thread again. This is
//! the one place those meet, and it is deliberately narrow: a request can be
//! submitted and an answer collected, and nothing here hands out a session, a
//! PTY, or a way to write to a child.

use std::collections::HashMap;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};

use sprite_term::{CommandSender, HistoryLines, HistorySnapshot, TerminalCommand, Ticket};

use crate::observation::broker::{PaneAddress, PaneSource, Pending, Withdraw};

/// What a pane sends back when it answers.
type Answer = Result<Arc<HistorySnapshot>, String>;
use crate::pane_tree::{PaneId, Rect};
use crate::tabs::TabId;

/// Every registered pane, shared with each outstanding request so that the
/// request can withdraw itself.
type Entries = Arc<Mutex<HashMap<PaneId, Entry>>>;

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
    /// Requests submitted and not yet answered, by the ticket each was sent
    /// with.
    ///
    /// Keyed rather than queued: an answer carries the ticket of the request
    /// it answers, so it can reach only that request. Arrival order, which
    /// any unrelated event emitted in between would shift, plays no part.
    waiting: HashMap<Ticket, std::sync::mpsc::Sender<Answer>>,
}

/// Where a pane sits in the window, as the schema reports it.
#[derive(Clone, Copy, Debug, PartialEq)]
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
    /// Shared with each outstanding request's [`Withdraw`], so a caller that
    /// stops waiting can take its own ticket back.
    entries: Entries,
    /// The next ticket to hand out. One counter for the whole window, so a
    /// ticket names one request whichever pane it went to.
    next_ticket: AtomicU64,
    #[cfg(test)]
    layout_publications: std::sync::atomic::AtomicUsize,
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
                waiting: HashMap::new(),
            },
        );
    }

    #[cfg(test)]
    pub(crate) fn layout_publications(&self) -> usize {
        self.layout_publications
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Records where the window's panes currently sit.
    ///
    /// Published by the window as the layout changes. Panes the window no
    /// longer has are ignored rather than added back: this reports placement,
    /// not membership.
    pub fn set_layout(&self, placements: &[(PaneId, Placement)]) {
        #[cfg(test)]
        self.layout_publications
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
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
        if let Some(entry) = entries.remove(&pane) {
            for waiter in entry.waiting.into_values() {
                let _ = waiter.send(Err("the pane closed before it answered".to_owned()));
            }
        }
    }

    /// Hands one pane's answer to the request holding `ticket`.
    ///
    /// Called from the view, which is the single consumer of a session's
    /// events. An answer whose ticket nobody holds is dropped: its request has
    /// already given up, and handing it to anyone else would answer a question
    /// they did not ask.
    pub fn deliver(&self, pane: PaneId, ticket: Ticket, snapshot: Arc<HistorySnapshot>) {
        self.answer(pane, ticket, Ok(snapshot));
    }

    /// Reports that one request's capture failed, to that request only.
    pub fn deliver_failure(&self, pane: PaneId, ticket: Ticket, reason: String) {
        self.answer(pane, ticket, Err(reason));
    }

    fn answer(&self, pane: PaneId, ticket: Ticket, answer: Answer) {
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(entry) = entries.get_mut(&pane)
            && let Some(waiter) = entry.waiting.remove(&ticket)
        {
            let _ = waiter.send(answer);
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

        let ticket = Ticket::new(
            self.next_ticket
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        );
        let (sender, answer) = std::sync::mpsc::channel();
        // Recorded before the command is sent, so an answer cannot arrive
        // before there is anyone recorded to receive it.
        entry.waiting.insert(ticket, sender);
        let address = PaneAddress {
            tab: entry.tab,
            tab_order: entry.placement.tab_order,
            pane,
            rect: entry.placement.rect,
            focused: entry.placement.focused,
        };
        if let Err(error) = entry
            .commands
            .try_send(TerminalCommand::CaptureHistory { ticket, lines })
        {
            entry.waiting.remove(&ticket);
            return Err(error.to_string());
        }
        // A caller that stops waiting takes its ticket back, so the registry
        // holds nothing for a request nobody will read. Weak, so an
        // outstanding request does not keep a closed window's registry alive.
        let registry = Arc::downgrade(&self.entries);
        Ok(Pending {
            address,
            answer,
            withdraw: Withdraw::new(move || {
                if let Some(entries) = registry.upgrade() {
                    let mut entries = entries.lock().unwrap_or_else(|error| error.into_inner());
                    if let Some(entry) = entries.get_mut(&pane) {
                        entry.waiting.remove(&ticket);
                    }
                }
            }),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observation::endpoint::Endpoint;
    use sprite_term::{
        PaneRow, PromptKind, ScreenKind, SessionConfig, Spawned, TerminalEvent, TerminalSession,
    };
    use std::time::Duration;

    fn snapshot(text: &str) -> Arc<HistorySnapshot> {
        Arc::new(HistorySnapshot {
            generation: 1,
            size: sprite_term::ValidTerminalSize::DEFAULT,
            screen: ScreenKind::Primary,
            rows: vec![PaneRow {
                text: text.into(),
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
                style: Default::default(),
            },
            viewport: sprite_term::Viewport {
                total_rows: 24,
                offset: 0,
                visible_rows: 24,
            },
            title: None,
            working_directory: None,
            placements: Vec::new(),
            captured_at_unix_ms: 1_800_000_000_000,
            foreground: None,
        })
    }

    /// Keep both receivers alive so the worker can serve registry commands.
    fn session() -> Spawned {
        let mut spawned = TerminalSession::spawn(SessionConfig::command(
            "/bin/sh",
            vec!["-c".into(), "sleep 30".into()],
        ))
        .expect("spawn a session");
        assert!(matches!(
            spawned.events.next_blocking().expect("worker started"),
            TerminalEvent::Ready
        ));
        spawned.snapshots.next_blocking().expect("initial snapshot");
        spawned
    }

    /// The tickets a pane is still waiting on, oldest first.
    fn waiting(panes: &WindowPanes, pane: PaneId) -> Vec<Ticket> {
        let entries = panes.entries.lock().unwrap();
        let mut tickets: Vec<Ticket> = entries[&pane].waiting.keys().copied().collect();
        tickets.sort_unstable();
        tickets
    }

    /// The one ticket a pane is waiting on.
    fn only_ticket(panes: &WindowPanes, pane: PaneId) -> Ticket {
        let tickets = waiting(panes, pane);
        assert_eq!(tickets.len(), 1, "exactly one request is outstanding");
        tickets[0]
    }

    fn text(answer: Answer) -> String {
        answer.expect("a snapshot").rows[0].text.to_string()
    }

    #[test]
    fn saturated_observation_refuses_without_holding_the_ui_registry() {
        let mut spawned = TerminalSession::spawn(SessionConfig::command("/bin/sh", vec!["-c".into(), "i=0; while [ $i -lt 100 ]; do printf '\\033]2;TITLE%s\\007' $i; i=$((i+1)); done; head -c 1048576 /dev/zero; sleep 30".into()])).unwrap();
        crate::test_blocking_wait::pause(Duration::from_millis(300));
        spawned.snapshots.next_blocking().unwrap();
        let panes = WindowPanes::new();
        panes.register(PaneId(0), TabId(0), spawned.session.commands());
        let (tx, rx) = std::sync::mpsc::channel();
        let registry = Arc::clone(&panes);
        std::thread::spawn(move || {
            let result = registry
                .begin(PaneId(0), HistoryLines::default())
                .map(|_| ());
            let _ = tx.send(result);
        });
        let result = rx.recv_timeout(Duration::from_secs(1));
        spawned
            .session
            .begin_shutdown()
            .unwrap()
            .unwrap()
            .wait()
            .unwrap();
        let refusal = result
            .expect("observation submission must release the UI registry promptly")
            .expect_err("saturated queue refuses observation");
        assert!(refusal.contains("queue is full"));
        let entries = panes.entries.lock().unwrap();
        assert!(
            entries[&PaneId(0)].waiting.is_empty(),
            "a refused request leaves no waiter"
        );
    }

    #[test]
    fn a_registered_pane_is_listed_in_a_stable_order() {
        let panes = WindowPanes::new();
        let first = session();
        let second = session();
        panes.register(PaneId(5), TabId(1), first.session.commands());
        panes.register(PaneId(2), TabId(0), second.session.commands());

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
        panes.register(PaneId(0), TabId(0), session.session.commands());

        let pending = panes
            .begin(PaneId(0), HistoryLines::default())
            .expect("asked");
        panes.deliver(
            PaneId(0),
            only_ticket(&panes, PaneId(0)),
            snapshot("answer"),
        );

        let answer = pending
            .answer
            .recv_timeout(Duration::from_secs(1))
            .expect("an answer arrived");
        assert_eq!(text(answer), "answer");
    }

    /// A capture that fails must tell its waiter why, not leave it to time
    /// out and be reported for the wrong reason.
    #[test]
    fn a_failure_reaches_the_caller_that_asked_for_it() {
        let panes = WindowPanes::new();
        let session = session();
        panes.register(PaneId(0), TabId(0), session.session.commands());

        let pending = panes
            .begin(PaneId(0), HistoryLines::default())
            .expect("asked");
        panes.deliver_failure(
            PaneId(0),
            only_ticket(&panes, PaneId(0)),
            "the child exited".to_owned(),
        );

        let answer = pending
            .answer
            .recv_timeout(Duration::from_secs(1))
            .expect("a failure arrived");
        assert_eq!(
            answer.expect_err("the pane failed"),
            "the child exited",
            "the waiter learns the actual reason, not a timeout"
        );
    }

    /// Two callers asking one pane at once each get the answer to their own
    /// request, whatever order the answers arrive in.
    #[test]
    fn concurrent_requests_for_one_pane_each_get_their_own_answer() {
        let panes = WindowPanes::new();
        let session = session();
        panes.register(PaneId(0), TabId(0), session.session.commands());

        let first = panes
            .begin(PaneId(0), HistoryLines::default())
            .expect("asked");
        let second = panes
            .begin(PaneId(0), HistoryLines::default())
            .expect("asked");
        let tickets = waiting(&panes, PaneId(0));
        assert_eq!(tickets.len(), 2, "two requests, two tickets");

        // Newest first: arrival order must not decide who receives what.
        panes.deliver(PaneId(0), tickets[1], snapshot("second"));
        panes.deliver(PaneId(0), tickets[0], snapshot("first"));

        let answer = |pending: &Pending| {
            pending
                .answer
                .recv_timeout(Duration::from_secs(1))
                .expect("answered")
        };
        assert_eq!(text(answer(&first)), "first");
        assert_eq!(text(answer(&second)), "second");
    }

    /// A failure names the request it belongs to and reaches no other.
    #[test]
    fn a_failure_reaches_only_the_request_it_belongs_to() {
        let panes = WindowPanes::new();
        let session = session();
        panes.register(PaneId(0), TabId(0), session.session.commands());

        let first = panes
            .begin(PaneId(0), HistoryLines::default())
            .expect("asked");
        let second = panes
            .begin(PaneId(0), HistoryLines::default())
            .expect("asked");
        let tickets = waiting(&panes, PaneId(0));

        panes.deliver_failure(PaneId(0), tickets[1], "capture failed".to_owned());

        assert!(
            first.answer.try_recv().is_err(),
            "the other request is still waiting for its own answer"
        );
        assert_eq!(
            second
                .answer
                .recv_timeout(Duration::from_secs(1))
                .expect("answered")
                .expect_err("failed"),
            "capture failed"
        );
        assert_eq!(waiting(&panes, PaneId(0)), vec![tickets[0]]);
    }

    /// A caller that stops waiting, as one does when its deadline passes,
    /// takes its ticket back: nothing is kept for a request nobody will read.
    #[test]
    fn a_request_that_stops_waiting_takes_its_ticket_back() {
        let panes = WindowPanes::new();
        let session = session();
        panes.register(PaneId(0), TabId(0), session.session.commands());

        let pending = panes
            .begin(PaneId(0), HistoryLines::default())
            .expect("asked");
        assert_eq!(waiting(&panes, PaneId(0)).len(), 1);

        drop(pending);

        assert!(
            waiting(&panes, PaneId(0)).is_empty(),
            "the request withdrew its own ticket"
        );
    }

    /// The answer to a request that gave up must not be handed to the request
    /// that came after it.
    #[test]
    fn an_answer_for_an_expired_ticket_reaches_nobody() {
        let panes = WindowPanes::new();
        let session = session();
        panes.register(PaneId(0), TabId(0), session.session.commands());

        let expired = panes
            .begin(PaneId(0), HistoryLines::default())
            .expect("asked");
        let late = only_ticket(&panes, PaneId(0));
        drop(expired);
        let current = panes
            .begin(PaneId(0), HistoryLines::default())
            .expect("asked again");

        // The worker's answer to the request that gave up arrives now.
        panes.deliver(PaneId(0), late, snapshot("late"));
        assert!(
            current.answer.try_recv().is_err(),
            "a late answer is dropped, not given to the next request"
        );

        panes.deliver(
            PaneId(0),
            only_ticket(&panes, PaneId(0)),
            snapshot("current"),
        );
        assert_eq!(
            text(
                current
                    .answer
                    .recv_timeout(Duration::from_secs(1))
                    .expect("its own answer")
            ),
            "current"
        );
    }

    /// A pane that closes releases its waiters immediately. Making them wait
    /// out the deadline would report "did not answer in time" for something
    /// already known to be gone.
    #[test]
    fn forgetting_a_pane_releases_whoever_was_waiting() {
        let panes = WindowPanes::new();
        let session = session();
        panes.register(PaneId(0), TabId(0), session.session.commands());
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

    /// Turning observation off must not disturb a session that is running. The
    /// pane keeps its child and its output; it simply becomes unreachable.
    #[test]
    fn a_session_keeps_running_when_observation_is_switched_off() {
        let panes = WindowPanes::new();
        let mut session = session();
        panes.register(PaneId(0), TabId(0), session.session.commands());

        // What switching observation off does to a window: the endpoint is
        // destroyed. Nothing here touches the session.
        let directory = std::env::temp_dir().join(format!("sprite-panes-{}", std::process::id()));
        drop(Endpoint::open_in(directory.clone(), |_| String::new()).expect("an endpoint"));
        let _ = std::fs::remove_dir_all(&directory);

        // The child is still there, and the session still takes commands.
        assert!(
            session
                .session
                .send(TerminalCommand::Resize(
                    sprite_term::ValidTerminalSize::DEFAULT
                ))
                .is_ok(),
            "the session is alive and accepting commands"
        );
        session
            .session
            .send(TerminalCommand::CaptureHistory {
                ticket: Ticket::new(0),
                lines: HistoryLines::default(),
            })
            .expect("request a fresh answer after disabling observation");
        assert!(matches!(
            session
                .events
                .next_blocking()
                .expect("worker still answers"),
            TerminalEvent::History { .. }
        ));
        assert_eq!(panes.panes().len(), 1, "and the pane is still a pane");
    }

    #[test]
    fn an_answer_nobody_is_waiting_for_is_discarded() {
        let panes = WindowPanes::new();
        let session = session();
        panes.register(PaneId(0), TabId(0), session.session.commands());

        // No request outstanding: this must not panic, grow a map, or be
        // handed to the next caller as a stale answer.
        panes.deliver(PaneId(0), Ticket::new(u64::MAX), snapshot("stale"));

        let pending = panes
            .begin(PaneId(0), HistoryLines::default())
            .expect("asked");
        assert!(
            pending.answer.try_recv().is_err(),
            "a later request does not receive an earlier abandoned answer"
        );
    }
}
