//! Change notices: how a connection that asked to `watch` learns that a pane
//! changed, without reading every pane on a timer.
//!
//! The window's panes mark what changed as it happens, on the GPUI thread.
//! With nobody watching a mark is one atomic load. With a watcher, marks for
//! one pane merge until the watcher's connection thread next sends, so a pane
//! that prints a thousand lines in a frame costs one notice, and what a
//! watcher has not yet been sent is bounded by the number of panes, however
//! slowly it reads.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::io::{Read, Write};
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use nix::errno::Errno;
use nix::poll::{PollFd, PollFlags, PollTimeout, poll};
use serde_json::json;

use crate::local_socket::{READ_RECHECK, wait_readable, write_within};
use crate::locked::locked;
use crate::timeouts::drain_deadline;

/// How many connections may stay open at once, watching or waiting for a
/// further request. Half the window's connections, so a program that keeps
/// every one of these can never stop a one-request client such as
/// `sprite panes list` from being served.
pub(crate) const MAX_KEPT: usize = super::MAX_CONNECTIONS / 2;

/// The least time between two notices on one connection: about one frame.
/// Changes that arrive sooner merge into the next notice.
pub(crate) const MIN_EVENT_INTERVAL: Duration = Duration::from_millis(16);

/// One thing that changed about a pane.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Mark {
    /// The terminal drew new content, at this generation.
    Terminal(u64),
    /// This Surface opened, changed or closed.
    Surface(u64),
    Title,
    /// The pane's session ended.
    Lifecycle,
    /// Where the pane sits, how big it is, whether its tab is showing, or
    /// whether it is the Current Pane.
    Layout,
    /// The pane's label or its tab's name.
    Label,
    Created,
    Closed,
}

/// Everything that changed about one pane since the last notice.
#[derive(Debug, Default, Eq, PartialEq)]
struct Change {
    terminal: Option<u64>,
    surfaces: BTreeSet<u64>,
    title: bool,
    lifecycle: bool,
    layout: bool,
    label: bool,
    created: bool,
    closed: bool,
}

impl Change {
    fn note(&mut self, mark: Mark) {
        match mark {
            Mark::Terminal(generation) => {
                self.terminal = Some(
                    self.terminal
                        .map_or(generation, |seen| seen.max(generation)),
                );
            }
            Mark::Surface(id) => {
                self.surfaces.insert(id);
            }
            Mark::Title => self.title = true,
            Mark::Lifecycle => self.lifecycle = true,
            Mark::Layout => self.layout = true,
            Mark::Label => self.label = true,
            Mark::Created => self.created = true,
            Mark::Closed => self.closed = true,
        }
    }

    /// The notice for this change, as one JSON line without its line ending.
    fn line(&self, window: &str, pane: u64) -> String {
        let mut changed: Vec<String> = Vec::new();
        for (set, name) in [
            (self.created, "created"),
            (self.terminal.is_some(), "terminal"),
        ] {
            if set {
                changed.push(name.to_owned());
            }
        }
        changed.extend(self.surfaces.iter().map(|id| format!("surface:{id}")));
        for (set, name) in [
            (self.title, "title"),
            (self.lifecycle, "lifecycle"),
            (self.layout, "layout"),
            (self.label, "label"),
            (self.closed, "closed"),
        ] {
            if set {
                changed.push(name.to_owned());
            }
        }
        let mut notice = json!({
            "protocol": super::PROTOCOL,
            "event": "changed",
            "pane": format!("{window}:{pane}"),
            "changed": changed,
        });
        if let Some(generation) = self.terminal {
            notice["generation"] = json!(generation);
        }
        notice.to_string()
    }
}

#[derive(Default)]
struct Pending {
    changes: BTreeMap<u64, Change>,
    /// Whether the watcher's thread has been woken for what is in `changes`.
    signalled: bool,
}

struct Watcher {
    /// The panes this watcher asked about; `None` for every pane.
    panes: Option<HashSet<u64>>,
    pending: Mutex<Pending>,
    /// Written to wake the watcher's thread. Never blocks.
    wake: UnixStream,
}

impl Watcher {
    fn note(&self, pane: u64, mark: Mark) {
        if self
            .panes
            .as_ref()
            .is_some_and(|panes| !panes.contains(&pane))
        {
            return;
        }
        let wake = {
            let mut pending = locked(&self.pending);
            pending.changes.entry(pane).or_default().note(mark);
            !std::mem::replace(&mut pending.signalled, true)
        };
        if wake {
            // One byte, to a socket that never blocks: it is written whole
            // or refused. A refusal means the pipe is full of wake-ups
            // already, so it loses nothing.
            (&self.wake).write_all(&[1]).ok();
        }
    }
}

#[derive(Default)]
struct Inner {
    watchers: Mutex<Vec<Arc<Watcher>>>,
    /// `watchers.len()`, readable without the lock.
    watching: AtomicUsize,
    kept: AtomicUsize,
    /// Every pane the window has, as it last published them.
    panes: Mutex<HashSet<u64>>,
}

/// One window's watchers. Cloning shares them.
#[derive(Clone, Default)]
pub(crate) struct Hub(Arc<Inner>);

impl Hub {
    /// Records that something about `pane` changed. Free when nobody watches.
    pub(crate) fn mark(&self, pane: u64, mark: Mark) {
        if self.0.watching.load(Ordering::Relaxed) == 0 {
            return;
        }
        for watcher in locked(&self.0.watchers).iter() {
            watcher.note(pane, mark);
        }
    }

    /// Records which panes the window has, so a watch can be refused for a
    /// pane that is not there.
    pub(crate) fn set_panes(&self, panes: impl Iterator<Item = u64>) {
        *locked(&self.0.panes) = panes.collect();
    }

    /// Whether the window has this pane.
    pub(crate) fn has_pane(&self, pane: u64) -> bool {
        locked(&self.0.panes).contains(&pane)
    }

    /// How many connections are watching.
    #[cfg(test)]
    pub(crate) fn watching(&self) -> usize {
        self.0.watching.load(Ordering::Relaxed)
    }

    /// How many connections hold a place among those that stay open.
    #[cfg(test)]
    pub(crate) fn kept(&self) -> usize {
        self.0.kept.load(Ordering::SeqCst)
    }

    /// Takes one of the [`MAX_KEPT`] places for a connection that stays
    /// open, or `None` when they are all taken.
    pub(crate) fn keep(&self) -> Option<Kept> {
        self.0
            .kept
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |kept| {
                (kept < MAX_KEPT).then_some(kept + 1)
            })
            .ok()
            .map(|_| Kept(self.clone()))
    }

    /// Starts watching `panes`, or every pane.
    pub(crate) fn subscribe(&self, panes: Option<HashSet<u64>>) -> std::io::Result<Subscription> {
        let (wake, woken) = UnixStream::pair()?;
        wake.set_nonblocking(true)?;
        woken.set_nonblocking(true)?;
        let watcher = Arc::new(Watcher {
            panes,
            pending: Mutex::default(),
            wake,
        });
        let mut watchers = locked(&self.0.watchers);
        watchers.push(Arc::clone(&watcher));
        self.0.watching.store(watchers.len(), Ordering::Relaxed);
        drop(watchers);
        Ok(Subscription {
            hub: self.clone(),
            watcher,
            woken,
        })
    }
}

/// A connection's place among those that stay open, given back on drop.
pub(crate) struct Kept(Hub);

impl Drop for Kept {
    fn drop(&mut self) {
        self.0.0.kept.fetch_sub(1, Ordering::SeqCst);
    }
}

/// One connection's watch, ended on drop.
pub(crate) struct Subscription {
    hub: Hub,
    watcher: Arc<Watcher>,
    woken: UnixStream,
}

impl Drop for Subscription {
    fn drop(&mut self) {
        let mut watchers = locked(&self.hub.0.watchers);
        watchers.retain(|watcher| !Arc::ptr_eq(watcher, &self.watcher));
        self.hub.0.watching.store(watchers.len(), Ordering::Relaxed);
    }
}

impl Subscription {
    /// Takes every change marked since the last call, as notice lines.
    fn take(&self, window: &str) -> Vec<u8> {
        let mut scratch = [0_u8; 64];
        while matches!((&self.woken).read(&mut scratch), Ok(read) if read > 0) {}
        let changes = {
            let mut pending = locked(&self.watcher.pending);
            pending.signalled = false;
            std::mem::take(&mut pending.changes)
        };
        let mut lines = Vec::new();
        for (pane, change) in changes {
            lines.extend_from_slice(change.line(window, pane).as_bytes());
            lines.push(b'\n');
        }
        lines
    }

    /// Sends notices down `stream` until the peer closes it, the window
    /// closes it, or the peer stops reading.
    ///
    /// Nothing here can wait forever. The thread sleeps in `poll`, looking
    /// again every [`READ_RECHECK`], so a peer that closed is noticed even
    /// when the system fails to wake a sleeping reader (issue #79). Every
    /// write has a deadline that a slow reader cannot stretch (issue #67).
    pub(crate) fn serve(self, window: &str, stream: &UnixStream) {
        let recheck = PollTimeout::from(u16::try_from(READ_RECHECK.as_millis()).unwrap_or(1000));
        let mut not_before = Instant::now();
        loop {
            let mut descriptors = [
                PollFd::new(stream.as_fd(), PollFlags::POLLIN),
                PollFd::new(self.woken.as_fd(), PollFlags::POLLIN),
            ];
            match poll(&mut descriptors, recheck) {
                Ok(_) | Err(Errno::EINTR) => {}
                Err(_) => return,
            }
            let ready = |index: usize| {
                descriptors
                    .get(index)
                    .and_then(|descriptor| descriptor.revents())
                    .is_some_and(|events| !events.is_empty())
            };
            let (peer, woken) = (ready(0), ready(1));
            if peer && !peer_still_open(stream) {
                return;
            }
            if !woken {
                continue;
            }
            // Changes that come sooner than a frame apart wait for the next
            // notice. The wait ends early only if the peer has something to
            // say, which is looked at on the next turn.
            let wait = not_before.saturating_duration_since(Instant::now());
            if !wait.is_zero() && !matches!(wait_readable(stream, wait), Ok(false)) {
                continue;
            }
            let lines = self.take(window);
            if lines.is_empty() {
                continue;
            }
            if write_within(stream, &lines, drain_deadline(lines.len())).is_err() {
                return;
            }
            not_before = Instant::now() + MIN_EVENT_INTERVAL;
        }
    }
}

/// Reads what a watching peer sent, after `poll` reported something. A
/// watching connection takes no further requests, so anything it sends is
/// discarded; the end of its stream, or an error, ends the watch.
fn peer_still_open(mut stream: &UnixStream) -> bool {
    if stream.set_read_timeout(Some(READ_RECHECK)).is_err() {
        return false;
    }
    let mut discarded = [0_u8; 256];
    match stream.read(&mut discarded) {
        Ok(0) => false,
        Ok(_) => true,
        Err(error) => matches!(
            error.kind(),
            std::io::ErrorKind::WouldBlock
                | std::io::ErrorKind::TimedOut
                | std::io::ErrorKind::Interrupted
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};

    fn notices(text: &[u8]) -> Vec<serde_json::Value> {
        std::str::from_utf8(text)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    #[test]
    fn marks_for_one_pane_merge_into_one_notice() {
        let hub = Hub::default();
        let watch = hub.subscribe(None).unwrap();
        for generation in 1..=1000 {
            hub.mark(3, Mark::Terminal(generation));
        }
        hub.mark(3, Mark::Surface(4));
        hub.mark(3, Mark::Title);
        hub.mark(5, Mark::Created);
        let sent = notices(&watch.take("w"));
        assert_eq!(
            sent,
            [
                json!({"protocol": super::super::PROTOCOL, "event": "changed", "pane": "w:3",
                    "changed": ["terminal", "surface:4", "title"], "generation": 1000}),
                json!({"protocol": super::super::PROTOCOL, "event": "changed", "pane": "w:5",
                    "changed": ["created"]}),
            ]
        );
        assert!(watch.take("w").is_empty(), "a change is sent once");
    }

    #[test]
    fn a_watcher_hears_only_the_panes_it_asked_about() {
        let hub = Hub::default();
        let watch = hub.subscribe(Some(HashSet::from([2]))).unwrap();
        hub.mark(1, Mark::Terminal(7));
        assert!(watch.take("w").is_empty());
        hub.mark(2, Mark::Closed);
        assert_eq!(notices(&watch.take("w"))[0]["changed"], json!(["closed"]));
    }

    #[test]
    fn a_label_is_told_apart_from_a_layout_and_known_panes_are_remembered() {
        let hub = Hub::default();
        let watch = hub.subscribe(None).unwrap();
        hub.mark(1, Mark::Label);
        hub.mark(2, Mark::Layout);
        let sent = notices(&watch.take("w"));
        assert_eq!(sent[0]["changed"], json!(["label"]));
        assert_eq!(sent[1]["changed"], json!(["layout"]));
        assert!(!hub.has_pane(1));
        hub.set_panes([1, 4].into_iter());
        assert!(hub.has_pane(4) && !hub.has_pane(2));
    }

    #[test]
    fn nobody_watching_keeps_nothing_and_a_dropped_watch_stops_counting() {
        let hub = Hub::default();
        hub.mark(1, Mark::Terminal(1));
        assert_eq!(hub.watching(), 0);
        let watch = hub.subscribe(None).unwrap();
        assert_eq!(hub.watching(), 1);
        drop(watch);
        assert_eq!(hub.watching(), 0);
        assert!(locked(&hub.0.watchers).is_empty());
    }

    #[test]
    fn only_so_many_connections_may_stay_open() {
        let hub = Hub::default();
        let mut kept: Vec<Kept> = (0..MAX_KEPT)
            .map(|_| hub.keep().expect("a place"))
            .collect();
        assert!(hub.keep().is_none());
        kept.pop();
        assert!(hub.keep().is_some());
    }

    /// The watch's thread ends when its peer closes, with no notice pending
    /// and so nothing to write: it is the wait that notices, not a failed
    /// write.
    #[test]
    fn a_watch_ends_when_its_peer_closes_while_nothing_changes() {
        let hub = Hub::default();
        let watch = hub.subscribe(None).unwrap();
        let (ours, theirs) = UnixStream::pair().unwrap();
        let (done, finished) = std::sync::mpsc::channel();
        let thread = std::thread::spawn(move || {
            watch.serve("w", &ours);
            done.send(()).unwrap();
        });
        theirs.shutdown(std::net::Shutdown::Write).unwrap();
        finished
            .recv_timeout(crate::test_blocking_wait::watchdog())
            .expect("the watch ended");
        thread.join().unwrap();
        assert_eq!(hub.watching(), 0);
    }

    /// A peer that stops reading is dropped once the socket fills, and the
    /// changes it never read stay bounded by the number of panes.
    #[test]
    fn a_watch_ends_when_its_peer_stops_reading() {
        let hub = Hub::default();
        let watch = hub.subscribe(None).unwrap();
        let (ours, _theirs) = UnixStream::pair().unwrap();
        let (done, finished) = std::sync::mpsc::channel();
        let marker = hub.clone();
        let thread = std::thread::spawn(move || {
            watch.serve("w", &ours);
            done.send(()).unwrap();
        });
        let deadline = Instant::now() + crate::test_blocking_wait::watchdog();
        let mut generation = 0;
        while finished.try_recv().is_err() {
            assert!(Instant::now() < deadline, "the watch never ended");
            for pane in 0..super::super::MAX_PANES as u64 {
                generation += 1;
                marker.mark(pane, Mark::Terminal(generation));
            }
            assert!(
                marker
                    .0
                    .watchers
                    .lock()
                    .unwrap()
                    .first()
                    .is_none_or(
                        |watcher| locked(&watcher.pending).changes.len() <= super::super::MAX_PANES
                    )
            );
            crate::test_blocking_wait::pause(Duration::from_millis(1));
        }
        thread.join().unwrap();
        assert_eq!(hub.watching(), 0);
    }

    #[test]
    fn notices_reach_the_peer_as_lines_no_closer_than_a_frame_apart() {
        let hub = Hub::default();
        let watch = hub.subscribe(None).unwrap();
        let (ours, theirs) = UnixStream::pair().unwrap();
        let thread = std::thread::spawn(move || watch.serve("w", &ours));
        let mut lines = BufReader::new(theirs.try_clone().unwrap()).lines();
        hub.mark(1, Mark::Terminal(1));
        let first: serde_json::Value =
            serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
        let started = Instant::now();
        assert_eq!(first["pane"], "w:1");
        // Both changes come within a frame of the first notice, so they
        // are held back and leave together.
        hub.mark(1, Mark::Terminal(2));
        hub.mark(1, Mark::Terminal(3));
        let second: serde_json::Value =
            serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
        assert!(started.elapsed() >= MIN_EVENT_INTERVAL / 2);
        // A thread held up between the two marks could see them apart; the
        // newest generation still arrives, in this notice or the next.
        let last = if second["generation"] == 3 {
            second
        } else {
            serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap()
        };
        assert_eq!(last["generation"], 3);
        drop(lines);
        drop(theirs);
        thread.join().unwrap();
    }
}
