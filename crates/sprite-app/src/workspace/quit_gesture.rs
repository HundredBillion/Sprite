use std::time::{Duration, Instant};

const HOLD_DURATION: Duration = Duration::from_millis(1200);
const DOUBLE_PRESS: Duration = Duration::from_millis(500);
const RELEASE_GRACE: Duration = Duration::from_millis(600);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum QuitSource {
    Shortcut,
    Window,
    Platform,
}

#[derive(Default)]
pub(super) struct QuitGesture {
    last_press: Option<(QuitSource, Instant)>,
    holding: bool,
    committed: bool,
    last_repeat: Option<Instant>,
    repeat_cadence: Duration,
    deadline: Option<Instant>,
}

impl QuitGesture {
    pub(super) fn cancel(&mut self) {
        *self = Self::default();
    }

    pub(super) fn visible(&self) -> bool {
        self.last_press.is_some()
    }

    pub(super) fn source(&self) -> Option<QuitSource> {
        self.last_press.map(|(source, _)| source)
    }

    pub(super) fn deadline(&self) -> Option<Instant> {
        self.deadline
    }

    pub(super) fn modifier_released(&mut self) {
        if !self.committed {
            self.holding = false;
        }
    }

    pub(super) fn poll(&mut self, now: Instant) -> bool {
        if self.deadline.is_some_and(|deadline| now >= deadline) {
            let quit = self.committed;
            *self = Self::default();
            return quit;
        }
        false
    }

    pub(super) fn release(&mut self) -> bool {
        self.holding = false;
        if self.committed {
            *self = Self::default();
            return true;
        }
        false
    }

    pub(super) fn press(&mut self, source: QuitSource, is_repeat: bool, now: Instant) -> bool {
        if is_repeat {
            if source == QuitSource::Shortcut
                && self.holding
                && let Some((QuitSource::Shortcut, started)) = self.last_press
            {
                // macOS can omit Command-letter key-up, so only repeats prove a hold.
                self.repeat_cadence = now.duration_since(self.last_repeat.unwrap_or(started));
                self.last_repeat = Some(now);
                self.committed |= now.duration_since(started) >= HOLD_DURATION;
                if self.committed {
                    // Leave slow repeats in this window until the key has gone quiet.
                    self.deadline = Some(now + RELEASE_GRACE.max(self.repeat_cadence * 2));
                }
            }
            return false;
        }
        if self.committed {
            if source == QuitSource::Shortcut {
                // A fresh press restarts the keyboard's initial repeat delay.
                self.deadline =
                    Some(now + (HOLD_DURATION + RELEASE_GRACE).max(self.repeat_cadence * 2));
                self.last_repeat = Some(now);
            }
            return false;
        }
        if self.last_press.is_some_and(|(previous, at)| {
            previous == source && now.duration_since(at) <= DOUBLE_PRESS
        }) {
            *self = Self::default();
            return true;
        }
        self.last_press = Some((source, now));
        self.holding = source == QuitSource::Shortcut;
        self.last_repeat = None;
        self.repeat_cadence = Duration::ZERO;
        self.deadline = Some(now + HOLD_DURATION + RELEASE_GRACE);
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_quit_source_requires_a_second_intentional_request() {
        for source in [
            QuitSource::Shortcut,
            QuitSource::Window,
            QuitSource::Platform,
        ] {
            let mut gesture = QuitGesture::default();
            let now = Instant::now();
            assert!(
                !gesture.press(source, false, now),
                "{source:?} quit on its first request"
            );
            assert!(gesture.press(source, false, now + Duration::from_millis(500)));
        }
    }

    #[test]
    fn repeats_do_not_confirm_and_slow_or_different_requests_start_again() {
        let now = Instant::now();
        let mut gesture = QuitGesture::default();
        assert!(!gesture.press(QuitSource::Shortcut, false, now));
        assert!(!gesture.press(QuitSource::Shortcut, true, now + Duration::from_millis(100)));
        assert!(!gesture.press(
            QuitSource::Shortcut,
            false,
            now + Duration::from_millis(501)
        ));
        assert!(!gesture.press(QuitSource::Window, false, now + Duration::from_millis(600)));
        assert!(gesture.press(QuitSource::Window, false, now + Duration::from_millis(700)));
    }

    #[test]
    fn a_hold_needs_a_repeat_at_the_threshold_then_waits_for_release() {
        let now = Instant::now();
        let mut gesture = QuitGesture::default();
        assert!(!gesture.press(QuitSource::Shortcut, false, now));
        for milliseconds in [400, 800, 1199, 1200] {
            assert!(!gesture.press(
                QuitSource::Shortcut,
                true,
                now + Duration::from_millis(milliseconds)
            ));
        }
        assert!(gesture.release());
        assert!(!gesture.release(), "a release must not quit twice");
    }

    #[test]
    fn a_completed_hold_waits_for_repeats_to_go_quiet() {
        let now = Instant::now();
        let mut gesture = QuitGesture::default();
        assert!(!gesture.press(QuitSource::Shortcut, false, now));
        for milliseconds in [400, 800, 1200, 1600] {
            assert!(!gesture.press(
                QuitSource::Shortcut,
                true,
                now + Duration::from_millis(milliseconds)
            ));
        }
        assert!(!gesture.poll(now + Duration::from_millis(2399)));
        assert!(gesture.poll(now + Duration::from_millis(2400)));
        assert!(!gesture.poll(now + Duration::from_secs(10)));
    }

    #[test]
    fn a_cancelled_hold_or_tap_cannot_be_answered_later() {
        let now = Instant::now();
        let mut gesture = QuitGesture::default();
        assert!(!gesture.press(QuitSource::Shortcut, false, now));
        assert!(gesture.visible());
        gesture.cancel();
        assert!(!gesture.visible());
        assert!(!gesture.press(
            QuitSource::Shortcut,
            false,
            now + Duration::from_millis(100)
        ));
        assert!(!gesture.press(
            QuitSource::Shortcut,
            true,
            now + Duration::from_millis(1300)
        ));
        gesture.cancel();
        assert!(!gesture.poll(now + Duration::from_secs(10)));
        assert!(!gesture.release());
    }

    #[test]
    fn missing_or_early_releases_never_turn_a_tap_into_a_hold() {
        let now = Instant::now();
        for release in [false, true] {
            let mut gesture = QuitGesture::default();
            assert!(!gesture.press(QuitSource::Shortcut, false, now));
            if release {
                assert!(!gesture.press(
                    QuitSource::Shortcut,
                    true,
                    now + Duration::from_millis(1199)
                ));
                assert!(!gesture.release());
            }
            assert!(!gesture.poll(now + Duration::from_secs(5)));
            assert!(!gesture.visible());
            assert!(!gesture.release());
        }
    }

    #[test]
    fn a_slow_initial_repeat_preserves_a_safe_release_grace() {
        let now = Instant::now();
        let mut gesture = QuitGesture::default();
        assert!(!gesture.press(QuitSource::Shortcut, false, now));
        assert!(!gesture.press(
            QuitSource::Shortcut,
            true,
            now + Duration::from_millis(1300)
        ));
        assert!(!gesture.poll(now + Duration::from_millis(3899)));
        assert!(gesture.poll(now + Duration::from_millis(3900)));
    }

    #[test]
    fn a_double_press_does_not_need_the_first_release() {
        let now = Instant::now();
        for release in [false, true] {
            let mut gesture = QuitGesture::default();
            assert!(!gesture.press(QuitSource::Shortcut, false, now));
            if release {
                assert!(!gesture.release());
            }
            assert!(gesture.press(
                QuitSource::Shortcut,
                false,
                now + Duration::from_millis(400)
            ));
        }
    }

    #[test]
    fn releasing_the_modifier_cancels_an_early_hold_but_keeps_double_press_consent() {
        let now = Instant::now();
        let mut gesture = QuitGesture::default();
        assert!(!gesture.press(QuitSource::Shortcut, false, now));
        gesture.modifier_released();
        assert!(!gesture.press(
            QuitSource::Shortcut,
            true,
            now + Duration::from_millis(1200)
        ));
        assert!(!gesture.release());
        assert!(!gesture.poll(now + Duration::from_secs(5)));
        assert!(!gesture.press(QuitSource::Shortcut, false, now + Duration::from_secs(6)));
        gesture.modifier_released();
        assert!(gesture.press(
            QuitSource::Shortcut,
            false,
            now + Duration::from_millis(6400)
        ));
    }

    #[test]
    fn a_committed_hold_waits_when_the_modifier_is_released_first() {
        let now = Instant::now();
        let mut gesture = QuitGesture::default();
        assert!(!gesture.press(QuitSource::Shortcut, false, now));
        assert!(!gesture.press(
            QuitSource::Shortcut,
            true,
            now + Duration::from_millis(1200)
        ));
        gesture.modifier_released();
        assert!(!gesture.poll(now + Duration::from_millis(1300)));
        assert!(gesture.release());
    }

    #[test]
    fn a_fresh_quit_press_keeps_a_committed_hold_in_this_window() {
        let now = Instant::now();
        let mut gesture = QuitGesture::default();
        assert!(!gesture.press(QuitSource::Shortcut, false, now));
        for milliseconds in [400, 800, 1200] {
            assert!(!gesture.press(
                QuitSource::Shortcut,
                true,
                now + Duration::from_millis(milliseconds)
            ));
        }
        assert!(!gesture.press(
            QuitSource::Shortcut,
            false,
            now + Duration::from_millis(1900)
        ));
        assert!(!gesture.poll(now + Duration::from_millis(2000)));
        assert!(!gesture.poll(now + Duration::from_millis(3699)));
        assert!(gesture.poll(now + Duration::from_millis(3700)));
    }

    #[test]
    fn a_fresh_committed_press_waits_for_its_initial_repeat_delay() {
        let now = Instant::now();
        let mut gesture = QuitGesture::default();
        assert!(!gesture.press(QuitSource::Shortcut, false, now));
        for milliseconds in [1000, 1100, 1200] {
            assert!(!gesture.press(
                QuitSource::Shortcut,
                true,
                now + Duration::from_millis(milliseconds)
            ));
        }
        assert!(!gesture.press(
            QuitSource::Shortcut,
            false,
            now + Duration::from_millis(1700)
        ));
        assert!(!gesture.poll(now + Duration::from_millis(2300)));
        assert!(!gesture.press(
            QuitSource::Shortcut,
            true,
            now + Duration::from_millis(2700)
        ));
        assert!(!gesture.poll(now + Duration::from_millis(4699)));
        assert!(gesture.poll(now + Duration::from_millis(4700)));
    }
}
