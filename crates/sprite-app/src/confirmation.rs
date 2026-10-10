//! A question answered by doing the same thing again.
//!
//! Closing a busy pane and pasting text that would run as commands both ask
//! first, and both are answered by repeating the gesture. What counts as
//! "again" is the whole of the safety: the auto-repeat of a key that is still
//! held down is the first press continuing, not a second decision, and a
//! repeat aimed at something other than what was asked about answers nothing.
//! This type holds that rule so that each question does not re-derive it.

/// One pending question about a `T`, or none.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Confirmation<T> {
    armed: Option<T>,
}

// Written out rather than derived: a derived `Default` would demand
// `T: Default`, and an unarmed question needs no subject at all.
impl<T> Default for Confirmation<T> {
    fn default() -> Self {
        Self { armed: None }
    }
}

impl<T: PartialEq> Confirmation<T> {
    /// Asks about `subject`, replacing any earlier question.
    pub(crate) fn arm(&mut self, subject: T) {
        self.armed = Some(subject);
    }

    /// Whether this gesture answers the question.
    ///
    /// Only a fresh press aimed at the same subject does, and answering uses
    /// the question up, so a third press asks again instead of acting twice.
    /// Anything else leaves the question exactly as it was.
    pub(crate) fn answer(&mut self, subject: &T, is_held: bool) -> bool {
        if is_held || self.armed.as_ref() != Some(subject) {
            return false;
        }
        self.armed = None;
        true
    }

    /// Withdraws the question; nothing can answer it now.
    pub(crate) fn disarm(&mut self) {
        self.armed = None;
    }

    /// Whether a question is waiting for its answer.
    pub(crate) fn is_armed(&self) -> bool {
        self.armed.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_fresh_press_on_the_same_subject_confirms() {
        let mut confirmation = Confirmation::default();
        assert!(!confirmation.is_armed());
        assert!(
            !confirmation.answer(&"close pane", false),
            "nothing was asked"
        );

        confirmation.arm("close pane");
        assert!(confirmation.is_armed());
        assert!(
            !confirmation.answer(&"close pane", true),
            "an auto-repeat is the first press still going"
        );
        assert!(
            !confirmation.answer(&"close tab", false),
            "a different subject is a different question"
        );
        assert!(confirmation.is_armed(), "neither withdrew the question");

        assert!(confirmation.answer(&"close pane", false));
        assert!(!confirmation.is_armed(), "an answer is used up");
        assert!(
            !confirmation.answer(&"close pane", false),
            "so a third press asks again rather than acting twice"
        );
    }

    #[test]
    fn a_withdrawn_question_cannot_be_answered() {
        let mut confirmation = Confirmation::default();
        confirmation.arm(String::from("one\ntwo"));
        confirmation.disarm();
        assert!(!confirmation.is_armed());
        assert!(!confirmation.answer(&String::from("one\ntwo"), false));
    }

    #[test]
    fn arming_again_replaces_the_subject() {
        let mut confirmation = Confirmation::default();
        confirmation.arm(1);
        confirmation.arm(2);
        assert!(!confirmation.answer(&1, false));
        assert!(confirmation.answer(&2, false));
    }
}
