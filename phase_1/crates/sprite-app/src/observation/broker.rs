//! Deciding what an authorised caller may see, and collecting it in bounded
//! time.
//!
//! **Threat model.** A client here is authorised but untrusted. It must not be
//! able to mutate anything, escape its window, or use one slow pane to stall
//! the window. So requests are pull-based and read-only — the parsed request
//! cannot express a write, because there is no variant that means one — scope
//! is resolved against this window's panes only, and the whole request lives
//! under a single deadline rather than one per pane.
//!
//! **What the key does and does not separate.** The key is a boundary between
//! this window and everything else. It is deliberately *not* a boundary between
//! the panes inside it — any pane may read any other pane in its window, so that
//! tools can coordinate across panes. Every session is told the same key, so the
//! requester's identity is a convenience that makes the common request short,
//! never a privilege.
//!
//! That is a decision, not an omission: see ADR 0013. It follows that a program
//! in one pane can read a secret visible in another, and that the window is the
//! unit of trust. Do not add a partial per-pane check here — half a boundary
//! suggests a protection that is not there, which is worse than none.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, TryRecvError};
use std::time::{Duration, Instant};

use sprite_term::{HistoryLines, HistorySnapshot};

use crate::pane_tree::{PaneId, Rect};
use crate::tabs::TabId;

/// One deadline for a whole request, however many panes it covers.
///
/// Per-pane deadlines would let a request grow without limit as panes are
/// added, so a caller could stall the endpoint simply by opening more of them.
pub const DEADLINE: Duration = Duration::from_millis(500);

/// Where a pane lives in this window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PaneAddress {
    pub tab: TabId,
    /// The tab's position in window order, which is the order the schema
    /// promises. Carried rather than derived, because a tab's identity does not
    /// have to follow its position.
    pub tab_order: usize,
    pub pane: PaneId,
    /// The pane's rectangle within its tab, normalised to 0..1.
    pub rect: Rect,
    /// Whether this is the focused pane of its tab.
    pub focused: bool,
}

/// A capture that has been asked for and not yet answered.
pub struct Pending {
    pub address: PaneAddress,
    pub answer: Receiver<Result<Arc<HistorySnapshot>, String>>,
}

/// The window's panes, as the broker is allowed to see them.
///
/// Deliberately narrow: it can list this window's panes and ask one for a
/// snapshot. There is no method that writes, sends input, or hands back a
/// session — a broker cannot do what this trait cannot express.
pub trait PaneSource: Send + Sync {
    /// Every pane in **this** window, in window order.
    ///
    /// A pane in another window is not listed, which is what makes "never
    /// across windows" structural rather than a check that could be forgotten.
    fn panes(&self) -> Vec<PaneAddress>;

    /// Asks one pane for its history and returns immediately.
    ///
    /// Returning without waiting is what lets the broker have every pane
    /// working at once under one deadline.
    fn begin(&self, pane: PaneId, lines: HistoryLines) -> Result<Pending, String>;
}

/// Which panes a caller asked about.
///
/// Every variant reads. There is deliberately no variant that writes, sends
/// input, subscribes, or opens a stream: a request that could mutate cannot be
/// constructed, so no code downstream has to refuse one.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Scope {
    /// The requester's own tab. The default, and by default without the
    /// requester itself — a pane asking "what else is going on" rarely means
    /// its own output.
    Tab { include_self: bool },
    /// One named pane.
    Pane(PaneId),
    /// Every pane in the window. Never beyond it.
    Window,
}

/// A parsed, authorised request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Query {
    /// The pane the caller says it is.
    ///
    /// Self-reported, and only used to shape the default scope. It is not a
    /// privilege: see the note at the top of this module.
    pub from: Option<PaneId>,
    pub scope: Scope,
    pub lines: HistoryLines,
    /// Lay the JSON out for a human. Whitespace only; never a second schema.
    pub pretty: bool,
}

/// The private request protocol between the bundled client and the window.
///
/// Versioned for mismatch diagnostics, not as a third-party contract: tools
/// integrate through the command's JSON output, not by speaking this.
pub const PROTOCOL: &str = "sprite-observation/1";

/// Why a request was not carried out.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Refusal {
    /// Anything about which panes exist or may be seen.
    ///
    /// One answer for "no such pane" and "not your pane", so a caller cannot
    /// map the window by watching the refusal change.
    Denied,
    /// The request text could not be read. Says nothing about any pane, so it
    /// is safe to distinguish from `Denied`: it describes the caller's own
    /// words, not the window's contents.
    Malformed(&'static str),
    /// The caller speaks a version of the private protocol this window does
    /// not. Distinguished so a mismatched client can say so plainly instead of
    /// reporting its own request as nonsense.
    UnsupportedProtocol,
}

/// One pane's answer.
#[derive(Clone, Debug)]
pub struct PaneReport {
    pub address: PaneAddress,
    pub snapshot: Arc<HistorySnapshot>,
}

/// Why one pane is missing from an answer.
///
/// Named rather than free text, because a client has to be able to tell a slow
/// pane from a closed one without reading English.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FailureKind {
    /// Did not answer before the request's deadline.
    Timeout,
    /// Ended before it could answer.
    Closed,
    /// Answered with a failure.
    Errored,
}

impl FailureKind {
    /// The wire name, which is part of the schema.
    pub fn as_str(self) -> &'static str {
        match self {
            // The wire names keep the `pane_` prefix the schema promises,
            // whatever the Rust variants are called.
            Self::Timeout => "pane_timeout",
            Self::Closed => "pane_closed",
            Self::Errored => "pane_error",
        }
    }
}

/// A pane that was asked and did not answer.
#[derive(Clone, Debug, PartialEq)]
pub struct Failure {
    pub address: PaneAddress,
    pub kind: FailureKind,
    pub reason: String,
}

/// What a request produced.
#[derive(Clone, Debug)]
pub struct Report {
    /// False when any pane in scope did not answer. A partial answer is still
    /// an answer: healthy panes are never discarded because one failed.
    pub complete: bool,
    pub panes: Vec<PaneReport>,
    pub failures: Vec<Failure>,
}

/// Reads a request body into a query.
///
/// The grammar is deliberately tiny. Anything unrecognised is refused rather
/// than ignored, so a client cannot smuggle a verb past a lenient parser.
pub fn parse(body: &str) -> Result<Query, Refusal> {
    let mut words = body.split_whitespace().peekable();
    // The protocol token is optional so that a client older than this window
    // is understood rather than refused; a *newer* one names a version this
    // window does not know, and is told so.
    if let Some(word) = words.peek()
        && word.starts_with("sprite-observation/")
    {
        let spoken = *word;
        words.next();
        if spoken != PROTOCOL {
            return Err(Refusal::UnsupportedProtocol);
        }
    }
    match (words.next(), words.next()) {
        (Some("panes"), Some("snapshot")) => {}
        _ => return Err(Refusal::Malformed("the only request is: panes snapshot")),
    }

    let mut from = None;
    let mut scope = None;
    let mut include_self = false;
    let mut pretty = false;
    let mut lines = HistoryLines::default();

    while let Some(word) = words.next() {
        match word {
            "--include-self" => include_self = true,
            "--pretty" => pretty = true,
            "--window" => {
                if scope.is_some() {
                    return Err(Refusal::Malformed("scope given twice"));
                }
                scope = Some(Scope::Window);
            }
            "--pane" => {
                if scope.is_some() {
                    return Err(Refusal::Malformed("scope given twice"));
                }
                let value = words
                    .next()
                    .ok_or(Refusal::Malformed("--pane needs a number"))?;
                let pane = value
                    .parse()
                    .map_err(|_| Refusal::Malformed("--pane needs a number"))?;
                scope = Some(Scope::Pane(PaneId(pane)));
            }
            "--from" => {
                let value = words
                    .next()
                    .ok_or(Refusal::Malformed("--from needs a number"))?;
                let pane = value
                    .parse()
                    .map_err(|_| Refusal::Malformed("--from needs a number"))?;
                from = Some(PaneId(pane));
            }
            "--lines" => {
                let value = words
                    .next()
                    .ok_or(Refusal::Malformed("--lines needs a number"))?;
                let count: usize = value
                    .parse()
                    .map_err(|_| Refusal::Malformed("--lines needs a number"))?;
                // Clamped, not refused, exactly as the extraction path does.
                lines = HistoryLines::new(count);
            }
            _ => return Err(Refusal::Malformed("unknown option")),
        }
    }

    if include_self && !matches!(scope, None | Some(Scope::Tab { .. })) {
        return Err(Refusal::Malformed("--include-self only applies to a tab"));
    }

    Ok(Query {
        from,
        scope: scope.unwrap_or(Scope::Tab { include_self }),
        lines,
        pretty,
    })
}

/// Resolves a query to the panes it may see.
///
/// Every address comes from `source.panes()`, so a pane belonging to another
/// window cannot appear in the result no matter what the caller asked for.
fn resolve(query: &Query, source: &dyn PaneSource) -> Result<Vec<PaneAddress>, Refusal> {
    let panes = source.panes();
    match query.scope {
        Scope::Window => Ok(panes),
        Scope::Pane(wanted) => panes
            .into_iter()
            .find(|address| address.pane == wanted)
            .map(|address| vec![address])
            // A pane in another window and a pane that never existed are the
            // same answer, because telling them apart would confirm the
            // existence of panes outside this window.
            .ok_or(Refusal::Denied),
        Scope::Tab { include_self } => {
            let from = query.from.ok_or(Refusal::Denied)?;
            let tab = panes
                .iter()
                .find(|address| address.pane == from)
                .map(|address| address.tab)
                // A caller claiming to be a pane this window does not have gets
                // the same refusal as any other unseeable pane.
                .ok_or(Refusal::Denied)?;
            Ok(panes
                .into_iter()
                .filter(|address| address.tab == tab)
                .filter(|address| include_self || address.pane != from)
                .collect())
        }
    }
}

/// Carries out a request, with every pane working at once under one deadline.
pub fn collect(
    query: &Query,
    source: &dyn PaneSource,
    deadline: Duration,
) -> Result<Report, Refusal> {
    let addresses = resolve(query, source)?;
    let started = Instant::now();

    // Every pane is asked before any answer is waited for. Asking one and
    // waiting for it before asking the next would make the request take as long
    // as the panes' times added together.
    let mut pending = Vec::with_capacity(addresses.len());
    let mut failures = Vec::new();
    for address in addresses {
        match source.begin(address.pane, query.lines) {
            Ok(request) => pending.push(request),
            // A pane that cannot even be asked — it closed as the request
            // arrived — is named, and costs the request nothing.
            Err(reason) => failures.push(Failure {
                address,
                kind: FailureKind::Closed,
                reason,
            }),
        }
    }

    let mut panes = Vec::with_capacity(pending.len());
    for request in pending {
        let remaining = deadline.saturating_sub(started.elapsed());
        // Once the deadline is spent, a pane is still given the chance to hand
        // over an answer that already arrived while another pane was holding
        // things up. That answer costs nothing and discarding it would punish a
        // healthy pane for a slow neighbour.
        let answer = if remaining.is_zero() {
            request.answer.try_recv().map_err(|error| match error {
                TryRecvError::Empty => RecvTimeoutError::Timeout,
                TryRecvError::Disconnected => RecvTimeoutError::Disconnected,
            })
        } else {
            request.answer.recv_timeout(remaining)
        };

        match answer {
            Ok(Ok(snapshot)) => panes.push(PaneReport {
                address: request.address,
                snapshot,
            }),
            Ok(Err(reason)) => failures.push(Failure {
                address: request.address,
                kind: FailureKind::Errored,
                reason,
            }),
            Err(RecvTimeoutError::Timeout) => failures.push(Failure {
                address: request.address,
                kind: FailureKind::Timeout,
                reason: "the pane did not answer within the deadline".to_owned(),
            }),
            // The pane ended between being asked and answering.
            Err(RecvTimeoutError::Disconnected) => failures.push(Failure {
                address: request.address,
                kind: FailureKind::Closed,
                reason: "the pane closed before it answered".to_owned(),
            }),
        }
    }

    let mut report = Report {
        complete: failures.is_empty(),
        panes,
        failures,
    };
    order_for_schema(&mut report);
    Ok(report)
}

/// Puts a report into the order the schema promises: tabs by window order, then
/// panes by top edge, then left edge, then identity.
///
/// Applied to the finished report rather than left to the order answers
/// happened to arrive in, so which pane was slow today cannot change how a
/// response is serialised.
pub fn order_for_schema(report: &mut Report) {
    fn key(address: &PaneAddress) -> (usize, f32, f32, PaneId) {
        (
            address.tab_order,
            address.rect.y,
            address.rect.x,
            address.pane,
        )
    }
    fn compare(left: &PaneAddress, right: &PaneAddress) -> std::cmp::Ordering {
        let (left_tab, left_y, left_x, left_pane) = key(left);
        let (right_tab, right_y, right_x, right_pane) = key(right);
        left_tab
            .cmp(&right_tab)
            .then(left_y.total_cmp(&right_y))
            .then(left_x.total_cmp(&right_x))
            .then(left_pane.cmp(&right_pane))
    }
    report
        .panes
        .sort_by(|left, right| compare(&left.address, &right.address));
    report
        .failures
        .sort_by(|left, right| compare(&left.address, &right.address));
}

#[cfg(test)]
mod tests {
    use super::*;
    use sprite_term::{PaneRow, ScreenKind, TerminalSize};
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::sync::mpsc::{Sender, channel};

    /// What a pane sends back when it answers.
    type Answer = Result<Arc<HistorySnapshot>, String>;

    /// A deadline short enough to keep the tests quick, standing in for the
    /// 500 ms one the window uses.
    const TEST_DEADLINE: Duration = Duration::from_millis(80);

    fn snapshot(text: &str) -> Arc<HistorySnapshot> {
        Arc::new(HistorySnapshot {
            generation: 1,
            size: TerminalSize::DEFAULT,
            screen: ScreenKind::Primary,
            rows: vec![PaneRow {
                text: text.to_owned(),
                wrapped: false,
                prompt: sprite_term::PromptKind::None,
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
            placements: Vec::new(),
            captured_at_unix_ms: 1_800_000_000_000,
            foreground: None,
        })
    }

    /// How one pane behaves when asked.
    enum Behaviour {
        Answers(&'static str),
        /// Never answers, so the request must give up on it.
        Stalls,
        /// Answers with a failure.
        Fails(&'static str),
        /// Ends without answering.
        Closes,
        /// Cannot even be asked.
        Unaskable,
    }

    struct FakeWindow {
        panes: Vec<PaneAddress>,
        behaviour: HashMap<u64, Behaviour>,
        /// Kept alive so a stalled pane's channel stays open rather than
        /// disconnecting, which would be a different failure.
        held: Mutex<Vec<Sender<Answer>>>,
        asked: Mutex<Vec<PaneId>>,
    }

    impl FakeWindow {
        fn new(panes: &[(u64, u64)]) -> Self {
            Self {
                panes: panes
                    .iter()
                    .enumerate()
                    .map(|(index, (tab, pane))| PaneAddress {
                        tab: TabId(*tab),
                        tab_order: *tab as usize,
                        pane: PaneId(*pane),
                        rect: Rect {
                            x: 0.0,
                            y: index as f32 / 10.0,
                            width: 1.0,
                            height: 0.1,
                        },
                        focused: index == 0,
                    })
                    .collect(),
                behaviour: HashMap::new(),
                held: Mutex::default(),
                asked: Mutex::default(),
            }
        }

        fn with(mut self, pane: u64, behaviour: Behaviour) -> Self {
            self.behaviour.insert(pane, behaviour);
            self
        }
    }

    impl PaneSource for FakeWindow {
        fn panes(&self) -> Vec<PaneAddress> {
            self.panes.clone()
        }

        fn begin(&self, pane: PaneId, _lines: HistoryLines) -> Result<Pending, String> {
            self.asked.lock().expect("lock").push(pane);
            let address = *self
                .panes
                .iter()
                .find(|address| address.pane == pane)
                .expect("the broker only asks for panes this window listed");
            let (sender, answer) = channel();
            match self.behaviour.get(&pane.0) {
                Some(Behaviour::Stalls) => {
                    // Held so the channel stays open: a stall is not a close.
                    self.held.lock().expect("lock").push(sender);
                }
                Some(Behaviour::Fails(reason)) => {
                    let _ = sender.send(Err((*reason).to_owned()));
                }
                Some(Behaviour::Closes) => drop(sender),
                Some(Behaviour::Unaskable) => {
                    return Err("the pane closed as the request arrived".to_owned());
                }
                Some(Behaviour::Answers(text)) => {
                    let _ = sender.send(Ok(snapshot(text)));
                }
                None => {
                    let _ = sender.send(Ok(snapshot("default")));
                }
            }
            Ok(Pending { address, answer })
        }
    }

    fn query(body: &str) -> Query {
        parse(body).expect("a valid request")
    }

    fn seen(report: &Report) -> Vec<u64> {
        report
            .panes
            .iter()
            .map(|report| report.address.pane.0)
            .collect()
    }

    // ---- scope -----------------------------------------------------------

    #[test]
    fn the_default_scope_is_the_requesters_tab_without_the_requester() {
        // Two tabs; the requester is pane 1 in tab 0.
        let window = FakeWindow::new(&[(0, 0), (0, 1), (0, 2), (1, 3)]);
        let report =
            collect(&query("panes snapshot --from 1"), &window, TEST_DEADLINE).expect("allowed");

        assert_eq!(seen(&report), vec![0, 2], "its own tab, minus itself");
        assert!(report.complete);
    }

    #[test]
    fn include_self_adds_the_requester_back() {
        let window = FakeWindow::new(&[(0, 0), (0, 1), (1, 2)]);
        let report = collect(
            &query("panes snapshot --from 1 --include-self"),
            &window,
            TEST_DEADLINE,
        )
        .expect("allowed");

        assert_eq!(seen(&report), vec![0, 1]);
    }

    #[test]
    fn a_named_pane_is_the_only_one_captured() {
        let window = FakeWindow::new(&[(0, 0), (0, 1), (1, 2)]);
        let report = collect(
            &query("panes snapshot --from 0 --pane 2"),
            &window,
            TEST_DEADLINE,
        )
        .expect("allowed");

        assert_eq!(seen(&report), vec![2], "including a pane in another tab");
    }

    #[test]
    fn window_scope_is_every_pane_in_this_window() {
        let window = FakeWindow::new(&[(0, 0), (0, 1), (1, 2), (2, 3)]);
        let report = collect(
            &query("panes snapshot --from 0 --window"),
            &window,
            TEST_DEADLINE,
        )
        .expect("allowed");

        assert_eq!(seen(&report), vec![0, 1, 2, 3]);
    }

    /// The rule that matters most: a request can never reach another window.
    /// A pane this window does not list is refused exactly as a forbidden one
    /// is, so a caller cannot learn that it exists elsewhere.
    #[test]
    fn a_pane_from_another_window_is_refused_like_any_unseeable_pane() {
        let window = FakeWindow::new(&[(0, 0), (0, 1)]);

        let elsewhere = collect(
            &query("panes snapshot --from 0 --pane 77"),
            &window,
            TEST_DEADLINE,
        );
        let never_existed = collect(
            &query("panes snapshot --from 0 --pane 999999"),
            &window,
            TEST_DEADLINE,
        );

        assert_eq!(elsewhere.unwrap_err(), Refusal::Denied);
        assert_eq!(
            never_existed.unwrap_err(),
            Refusal::Denied,
            "indistinguishable from a pane in another window"
        );
        assert!(
            window.asked.lock().expect("lock").is_empty(),
            "a refused request captures nothing at all"
        );
    }

    #[test]
    fn a_caller_claiming_a_pane_this_window_does_not_have_is_refused() {
        let window = FakeWindow::new(&[(0, 0)]);
        let report = collect(&query("panes snapshot --from 42"), &window, TEST_DEADLINE);
        assert_eq!(report.unwrap_err(), Refusal::Denied);
    }

    #[test]
    fn a_default_scoped_request_that_does_not_say_who_it_is_is_refused() {
        let window = FakeWindow::new(&[(0, 0), (0, 1)]);
        let report = collect(&query("panes snapshot"), &window, TEST_DEADLINE);
        assert_eq!(report.unwrap_err(), Refusal::Denied);
    }

    // ---- read-only -------------------------------------------------------

    /// The request grammar admits nothing that mutates. There is no verb for
    /// writing, no subscription, and no stream, so nothing downstream has to
    /// refuse one.
    #[test]
    fn the_request_grammar_admits_nothing_that_mutates() {
        for attempt in [
            "panes write hello",
            "panes send-keys ls",
            "panes input ls\\n",
            "panes paste secret",
            "panes subscribe",
            "panes watch",
            "panes stream",
            "panes snapshot --write",
            "panes snapshot --exec ls",
            "panes kill --pane 1",
            "keys send",
            "",
        ] {
            let outcome = parse(attempt);
            assert!(
                matches!(outcome, Err(Refusal::Malformed(_))),
                "{attempt:?} must not parse, got {outcome:?}"
            );
        }
    }

    #[test]
    fn a_line_count_is_clamped_rather_than_refused() {
        assert_eq!(
            query("panes snapshot --from 0 --lines 999999").lines.get(),
            HistoryLines::MAX
        );
        assert_eq!(query("panes snapshot --from 0").lines.get(), 500);
        assert_eq!(query("panes snapshot --from 0 --lines 0").lines.get(), 0);
    }

    #[test]
    fn scope_may_not_be_given_twice_or_contradicted() {
        assert!(matches!(
            parse("panes snapshot --window --pane 1"),
            Err(Refusal::Malformed(_))
        ));
        assert!(matches!(
            parse("panes snapshot --pane 1 --include-self"),
            Err(Refusal::Malformed(_))
        ));
    }

    // ---- deadline and partial results ------------------------------------

    /// A slow pane must not extend anyone else's request.
    #[test]
    fn one_stalled_pane_does_not_hold_the_whole_request() {
        let window = FakeWindow::new(&[(0, 0), (0, 1), (0, 2)])
            .with(1, Behaviour::Stalls)
            .with(0, Behaviour::Answers("first"))
            .with(2, Behaviour::Answers("third"));

        let started = Instant::now();
        let report = collect(
            &query("panes snapshot --from 9 --window"),
            &window,
            TEST_DEADLINE,
        )
        .expect("allowed");
        let took = started.elapsed();

        assert!(
            took < TEST_DEADLINE * 3,
            "the request is bounded by one deadline, not one per pane: {took:?}"
        );
        assert!(!report.complete, "a pane did not answer");
        assert_eq!(
            seen(&report),
            vec![0, 2],
            "the healthy panes are still returned"
        );
        assert_eq!(report.failures.len(), 1);
        assert_eq!(report.failures[0].address.pane, PaneId(1));
        assert!(report.failures[0].reason.contains("deadline"));
    }

    /// The answer that arrived while a neighbour was stalling is not thrown
    /// away just because the clock ran out before it was read.
    #[test]
    fn an_answer_that_arrived_during_a_stall_is_still_collected() {
        // Pane 0 stalls and is waited on first; pane 1 answered immediately.
        let window = FakeWindow::new(&[(0, 0), (0, 1)])
            .with(0, Behaviour::Stalls)
            .with(1, Behaviour::Answers("ready all along"));

        let report = collect(
            &query("panes snapshot --from 9 --window"),
            &window,
            Duration::from_millis(30),
        )
        .expect("allowed");

        assert_eq!(seen(&report), vec![1]);
        assert!(!report.complete);
    }

    #[test]
    fn a_pane_that_fails_does_not_discard_the_healthy_ones() {
        let window = FakeWindow::new(&[(0, 0), (0, 1), (0, 2)])
            .with(1, Behaviour::Fails("the terminal worker ended"));

        let report = collect(
            &query("panes snapshot --from 9 --window"),
            &window,
            TEST_DEADLINE,
        )
        .expect("allowed");

        assert_eq!(seen(&report), vec![0, 2]);
        assert!(!report.complete);
        assert_eq!(report.failures.len(), 1);
        assert_eq!(report.failures[0].address.pane, PaneId(1));
        assert_eq!(report.failures[0].kind, FailureKind::Errored);
        assert_eq!(
            report.failures[0].reason, "the terminal worker ended",
            "the failure is named rather than silently dropped"
        );
    }

    #[test]
    fn a_pane_that_closes_mid_collection_is_named_and_costs_nothing() {
        let window = FakeWindow::new(&[(0, 0), (0, 1), (0, 2)]).with(1, Behaviour::Closes);

        let started = Instant::now();
        let report = collect(
            &query("panes snapshot --from 9 --window"),
            &window,
            TEST_DEADLINE,
        )
        .expect("allowed");

        assert!(
            started.elapsed() < TEST_DEADLINE,
            "a closed pane is known immediately rather than waited out"
        );
        assert_eq!(seen(&report), vec![0, 2]);
        assert!(report.failures[0].reason.contains("closed"));
    }

    #[test]
    fn a_pane_that_cannot_be_asked_is_named_without_being_waited_for() {
        let window = FakeWindow::new(&[(0, 0), (0, 1)]).with(0, Behaviour::Unaskable);

        let report = collect(
            &query("panes snapshot --from 9 --window"),
            &window,
            TEST_DEADLINE,
        )
        .expect("allowed");

        assert_eq!(seen(&report), vec![1]);
        assert!(!report.complete);
        assert_eq!(report.failures[0].address.pane, PaneId(0));
    }

    #[test]
    fn a_request_every_pane_answers_is_complete() {
        let window = FakeWindow::new(&[(0, 0), (0, 1), (1, 2)]);
        let report = collect(
            &query("panes snapshot --from 9 --window"),
            &window,
            TEST_DEADLINE,
        )
        .expect("allowed");

        assert!(report.complete);
        assert!(report.failures.is_empty());
        assert_eq!(report.panes.len(), 3);
    }
}
