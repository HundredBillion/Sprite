//! Serving a lane of requests on the window's thread.

use std::future::Future;
use std::pin::Pin;
use std::task::Poll;

use gpui::{Context, Task, Window};

/// Serves `lane` with `serve` until the lane closes or the view is gone,
/// yielding to the window's other work after every request.
pub(super) fn drain_lane<V: 'static, R: 'static>(
    lane: async_channel::Receiver<R>,
    window: &mut Window,
    cx: &mut Context<V>,
    serve: impl Fn(&mut V, R, &mut Window, &mut Context<V>) + 'static,
) -> Task<()> {
    cx.spawn_in(window, async move |view, cx| {
        while let Ok(request) = lane.recv().await {
            let served = view.update_in(cx, |view, window, cx| serve(view, request, window, cx));
            if served.is_err() {
                break;
            }
            // `recv` is ready at once while the lane holds anything, and the
            // window's executor never preempts a task. A lane that other
            // threads keep filling would otherwise never park this task, and
            // nothing else on the window thread — the other lane, a frame —
            // would run until it ran dry.
            YieldOnce::default().await;
        }
    })
}

/// Pending once, with its waker already woken, then ready: puts this task at
/// the back of the executor's queue.
#[derive(Default)]
struct YieldOnce {
    yielded: bool,
}

impl Future for YieldOnce {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<()> {
        if self.yielded {
            return Poll::Ready(());
        }
        self.yielded = true;
        cx.waker().wake_by_ref();
        Poll::Pending
    }
}

#[cfg(test)]
mod tests {
    use super::drain_lane;
    use gpui::{Context, IntoElement, Render, Task, Window};

    /// How many stream requests the test serves before it lets its stream
    /// lane run dry, so a drain that never yields ends rather than hangs.
    const STREAM_LIMIT: usize = 1000;
    /// The stream request whose serving sends the one-shot request.
    const SEND_ONE_SHOT_AT: usize = 3;

    struct Host {
        stream_served: usize,
        one_shot_served_at: Option<usize>,
        refill: async_channel::Sender<()>,
        one_shot: async_channel::Sender<()>,
        _drains: [Task<()>; 2],
    }

    impl Render for Host {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            gpui::div()
        }
    }

    /// A lane some other connection keeps full must not starve the other
    /// lane's task, which shares the one window thread with it.
    #[gpui::test]
    fn a_lane_that_never_runs_dry_does_not_starve_the_other_lane(cx: &mut gpui::TestAppContext) {
        let (stream_tx, stream_rx) = async_channel::bounded(4);
        let (one_shot_tx, one_shot_rx) = async_channel::bounded(1);
        for _ in 0..4 {
            stream_tx.try_send(()).expect("prefill the stream lane");
        }
        let (host, cx) = cx.add_window_view(|window, cx| {
            let stream = drain_lane(stream_rx, window, cx, |host: &mut Host, (), _, _| {
                host.stream_served += 1;
                if host.stream_served == SEND_ONE_SHOT_AT {
                    host.one_shot
                        .try_send(())
                        .expect("send the one-shot request");
                }
                // Each request served is replaced, so the lane is never empty
                // until the test's own limit.
                if host.stream_served < STREAM_LIMIT {
                    host.refill.try_send(()).expect("refill the stream lane");
                }
            });
            let one_shot = drain_lane(one_shot_rx, window, cx, |host: &mut Host, (), _, _| {
                host.one_shot_served_at = Some(host.stream_served);
            });
            Host {
                stream_served: 0,
                one_shot_served_at: None,
                refill: stream_tx,
                one_shot: one_shot_tx,
                _drains: [stream, one_shot],
            }
        });
        cx.run_until_parked();
        let served_at = host.read_with(cx, |host, _| host.one_shot_served_at);
        assert!(
            served_at.is_some_and(|at| at <= SEND_ONE_SHOT_AT + 2),
            "the one-shot request waited behind the stream: served after {served_at:?} stream requests"
        );
    }
}
