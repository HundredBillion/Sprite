//! Blocking coordination keeps real worker timing independent of the paused GPUI test scheduler.

#![cfg(test)]

use std::time::Duration;

#[expect(
    clippy::disallowed_methods,
    reason = "the one blocking pause tests may use"
)]
pub(crate) fn pause(duration: Duration) {
    std::thread::sleep(duration);
}

/// How long a test waits for a condition before failing: five seconds, or
/// `SPRITE_TEST_WATCHDOG_SECS` where a loaded runner needs longer.
pub(crate) fn watchdog() -> Duration {
    std::env::var("SPRITE_TEST_WATCHDOG_SECS")
        .ok()
        .and_then(|seconds| seconds.parse().ok())
        .map_or(Duration::from_secs(5), Duration::from_secs)
}

/// Resolves once `predicate` holds for `entity`, re-checking whenever the
/// entity notifies or emits, as GPUI's `Entity::condition` does.
///
/// GPUI's own version gives up after one second of wall-clock time, which a
/// debug build of the terminal on a loaded runner can outlast with nothing
/// wrong. This one waits for [`watchdog`], so a slow runner gets the same
/// allowance as every other wait in the suite.
pub(crate) fn condition<V: gpui::EventEmitter<()> + 'static>(
    entity: &gpui::Entity<V>,
    cx: &gpui::TestAppContext,
    mut predicate: impl FnMut(&V, &gpui::App) -> bool,
) -> impl std::future::Future<Output = ()> {
    let limit = watchdog();
    let (wake, woken) = async_channel::bounded::<()>(1024);
    let subscriptions = {
        let mut app = cx.app.borrow_mut();
        (
            app.observe(entity, {
                let wake = wake.clone();
                move |_, _| {
                    wake.try_send(()).ok();
                }
            }),
            app.subscribe(entity, {
                let wake = wake.clone();
                move |_, _: &(), _| {
                    wake.try_send(()).ok();
                }
            }),
        )
    };
    // Nothing else wakes a wait whose condition never comes true, so a thread
    // does it at the deadline, and the elapsed-time check fails instead of parking.
    std::thread::spawn(move || {
        pause(limit);
        wake.try_send(()).ok();
    });
    let app = cx.app.clone();
    let handle = entity.downgrade();
    let started = std::time::Instant::now();
    async move {
        loop {
            {
                let app = app.borrow();
                let entity = handle
                    .upgrade()
                    .expect("entity dropped with a pending condition");
                if predicate(entity.read(&app), &app) {
                    break;
                }
            }
            assert!(
                started.elapsed() < limit,
                "condition not met within {limit:?}"
            );
            app.borrow().background_executor().start_waiting();
            woken.recv().await.ok();
            app.borrow().background_executor().finish_waiting();
        }
        drop(subscriptions);
    }
}
