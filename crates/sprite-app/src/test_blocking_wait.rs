//! Blocking coordination keeps real worker timing independent of the paused GPUI test scheduler.

#![cfg(test)]

pub(crate) fn pause(duration: std::time::Duration) {
    std::thread::sleep(duration);
}
