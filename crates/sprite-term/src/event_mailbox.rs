//! Ordered event ownership with one retained mutation batch and reserved outcomes.
use crate::{SessionError, TerminalEvent};
use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

const NATURAL_DRAIN: Duration = Duration::from_secs(2);

struct State {
    normal: VecDeque<TerminalEvent>,
    pending: VecDeque<TerminalEvent>,
    outcomes: VecDeque<TerminalEvent>,
    canceled: bool,
    consumer_gone: bool,
    sealed: bool,
    drain_deadline: Option<Instant>,
}

pub(crate) struct Mailbox {
    state: Mutex<State>,
    space: Condvar,
    wake: async_channel::Sender<()>,
    capacity: usize,
}

pub(crate) struct Receiver {
    mailbox: Arc<Mailbox>,
    wake: async_channel::Receiver<()>,
}

pub(crate) fn bounded(capacity: usize) -> (Arc<Mailbox>, Receiver) {
    let (wake, receiver) = async_channel::bounded(1);
    let mailbox = Arc::new(Mailbox {
        state: Mutex::new(State {
            normal: VecDeque::new(),
            pending: VecDeque::new(),
            outcomes: VecDeque::new(),
            canceled: false,
            consumer_gone: false,
            sealed: false,
            drain_deadline: None,
        }),
        space: Condvar::new(),
        wake,
        capacity,
    });
    (
        Arc::clone(&mailbox),
        Receiver {
            mailbox,
            wake: receiver,
        },
    )
}

pub(crate) struct CompletionGuard(Arc<Mailbox>);

impl Drop for CompletionGuard {
    fn drop(&mut self) {
        let mut state = self
            .0
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if !state.sealed {
            state
                .outcomes
                .push_back(TerminalEvent::Error(SessionError::new(
                    "worker",
                    "the terminal worker ended before cleanup completed",
                )));
            state.sealed = true;
            self.0.wake.close();
            self.0.space.notify_all();
        }
    }
}

impl Mailbox {
    pub(crate) fn completion_guard(self: &Arc<Self>) -> CompletionGuard {
        CompletionGuard(Arc::clone(self))
    }

    /// Transfers the whole batch before applying pressure.
    /// The next mutation waits until the retained batch drains or cancellation arrives.
    pub(crate) fn publish(&self, batch: Vec<TerminalEvent>) -> bool {
        let mut state = self.state.lock().unwrap();
        if state.consumer_gone || state.sealed {
            return false;
        }
        if batch.is_empty() {
            return !state.canceled;
        }
        assert!(state.pending.is_empty(), "only one produced batch may wait");
        let mut batch = VecDeque::from(batch);
        while state.normal.len() < self.capacity {
            let Some(event) = batch.pop_front() else {
                break;
            };
            state.normal.push_back(event);
        }
        state.pending = batch;
        let _ = self.wake.force_send(());
        while !state.pending.is_empty() && !state.canceled && !state.consumer_gone {
            if let Some(deadline) = state.drain_deadline {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return false;
                }
                state = self.space.wait_timeout(state, remaining).unwrap().0;
            } else {
                state = self.space.wait(state).unwrap();
            }
        }
        !state.canceled && !state.consumer_gone
    }

    /// The waiter starts the deadline independently of the worker inbox.
    /// Event pressure must not hide direct-child exit indefinitely.
    pub(crate) fn begin_natural_drain(&self) {
        self.state.lock().unwrap().drain_deadline = Some(Instant::now() + NATURAL_DRAIN);
        self.space.notify_all();
    }

    pub(crate) fn drain_remaining(&self) -> Option<Duration> {
        self.state
            .lock()
            .unwrap()
            .drain_deadline
            .map(|deadline| deadline.saturating_duration_since(Instant::now()))
    }

    pub(crate) fn producer_ready(&self) -> bool {
        let state = self.state.lock().unwrap();
        state.pending.is_empty() && !state.canceled && !state.consumer_gone && !state.sealed
    }

    pub(crate) fn finish_remaining(&self) -> Option<Duration> {
        self.state.lock().unwrap().drain_deadline.map(|deadline| {
            (deadline + crate::worker::CLEANUP_BUDGET - NATURAL_DRAIN)
                .saturating_duration_since(Instant::now())
        })
    }

    pub(crate) fn cancel(&self) {
        self.state.lock().unwrap().canceled = true;
        self.space.notify_all();
    }

    /// Cleanup reserves a fatal error and a child outcome.
    /// Final outcomes follow retained live events without waiting for the consumer.
    pub(crate) fn seal(&self, outcomes: Vec<TerminalEvent>) {
        assert!(outcomes.len() <= 2);
        let mut state = self.state.lock().unwrap();
        if state.sealed {
            return;
        }
        state.outcomes = outcomes.into();
        state.sealed = true;
        self.wake.close();
        self.space.notify_all();
    }
}

impl Receiver {
    fn pop(&self) -> Result<Option<TerminalEvent>, SessionError> {
        let mut state = self
            .mailbox
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let event = state
            .normal
            .pop_front()
            .or_else(|| state.pending.pop_front())
            .or_else(|| state.outcomes.pop_front());
        if event.is_some() {
            self.mailbox.space.notify_all();
            return Ok(event);
        }
        if state.sealed {
            return Err(SessionError::new(
                "event_stream",
                "the terminal session ended",
            ));
        }
        Ok(None)
    }

    pub(crate) async fn next(&self) -> Result<TerminalEvent, SessionError> {
        loop {
            if let Some(event) = self.pop()? {
                return Ok(event);
            }
            let _ = self.wake.recv().await;
        }
    }

    pub(crate) fn next_blocking(&self) -> Result<TerminalEvent, SessionError> {
        loop {
            if let Some(event) = self.pop()? {
                return Ok(event);
            }
            let _ = self.wake.recv_blocking();
        }
    }
}

impl Drop for Receiver {
    fn drop(&mut self) {
        let mut state = self
            .mailbox
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        state.consumer_gone = true;
        state.normal.clear();
        state.pending.clear();
        state.outcomes.clear();
        self.mailbox.space.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancellation_retains_a_single_batch_larger_than_normal_capacity() {
        let (mailbox, receiver) = bounded(2);
        mailbox.cancel();
        let batch = (0..100)
            .map(|index| TerminalEvent::TitleChanged(Some(index.to_string())))
            .collect();
        assert!(!mailbox.publish(batch));
        mailbox.seal(vec![TerminalEvent::Ready]);
        for index in 0..100 {
            assert!(
                matches!(receiver.next_blocking().unwrap(), TerminalEvent::TitleChanged(Some(title)) if title == index.to_string())
            );
        }
        assert!(matches!(
            receiver.next_blocking().unwrap(),
            TerminalEvent::Ready
        ));
        assert!(receiver.next_blocking().is_err());
    }
}

#[cfg(test)]
mod producer_lifetime_tests {
    use super::*;

    #[test]
    fn a_panicking_worker_closes_events_with_the_session_owner_retained() {
        let (mailbox, receiver) = bounded(2);
        let worker_mailbox = Arc::clone(&mailbox);
        let worker = std::thread::spawn(move || {
            let _completion = worker_mailbox.completion_guard();
            worker_mailbox.publish(vec![TerminalEvent::Ready]);
            panic!("simulated worker failure");
        });
        assert!(worker.join().is_err());
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            assert!(matches!(
                receiver.next_blocking().unwrap(),
                TerminalEvent::Ready
            ));
            let mut error = false;
            while let Ok(event) = receiver.next_blocking() {
                error |= matches!(event, TerminalEvent::Error(_));
            }
            let _ = tx.send(error);
        });
        let result = rx.recv_timeout(Duration::from_millis(200));
        mailbox.seal(Vec::new());
        assert!(
            result.expect("event stream outlived its failed producer"),
            "unexpected termination has a final error"
        );
    }
}
