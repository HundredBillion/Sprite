//! Blocking coordination keeps real worker timing independent of the paused GPUI test scheduler.

#![cfg(test)]

use std::time::Duration;

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
