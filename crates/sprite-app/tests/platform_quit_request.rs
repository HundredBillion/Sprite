//! GPUI's should-terminate hook, driven through its test platform: the
//! question macOS asks before a Quit from the Dock or the menu bar, or the
//! end of the session, ends the application.

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{TerminateAnswer, TerminateReply};

/// The replies the callback was handed, kept so a test can answer them later.
type Held = Rc<RefCell<Vec<TerminateReply>>>;

fn hold_replies(cx: &gpui::TestAppContext) -> Held {
    let held: Held = Rc::default();
    let keep = Rc::clone(&held);
    cx.update(|cx| cx.on_should_terminate(move |reply, _| keep.borrow_mut().push(reply)));
    held
}

fn take(held: &Held) -> TerminateReply {
    let reply = held.borrow_mut().pop();
    let Some(reply) = reply else {
        panic!("the callback was not asked");
    };
    reply
}

#[gpui::test]
fn without_a_callback_the_platform_terminates_at_once(cx: &mut gpui::TestAppContext) {
    assert_eq!(
        cx.simulate_should_terminate().answer(),
        TerminateAnswer::Allowed
    );
}

#[gpui::test]
fn the_callback_answers_now_or_later(cx: &mut gpui::TestAppContext) {
    let held = hold_replies(cx);
    let cancelled = cx.simulate_should_terminate();
    assert_eq!(cancelled.answer(), TerminateAnswer::Pending);
    take(&held).cancel();
    assert_eq!(cancelled.answer(), TerminateAnswer::Cancelled);

    let allowed = cx.simulate_should_terminate();
    assert_eq!(allowed.answer(), TerminateAnswer::Pending);
    take(&held).allow();
    assert_eq!(allowed.answer(), TerminateAnswer::Allowed);
}

/// One reply is outstanding at a time: a request made while the application
/// still holds one is refused without asking, and the held reply decides.
#[gpui::test]
fn a_request_while_one_is_unanswered_is_refused_without_asking(cx: &mut gpui::TestAppContext) {
    let held = hold_replies(cx);
    let first = cx.simulate_should_terminate();
    let second = cx.simulate_should_terminate();
    assert_eq!(second.answer(), TerminateAnswer::Cancelled);
    assert_eq!(held.borrow().len(), 1, "the callback was asked twice");
    take(&held).allow();
    assert_eq!(first.answer(), TerminateAnswer::Allowed);
}

/// A reply nobody answers lets the application end, as it did before the
/// hook existed, rather than leaving it impossible to quit.
#[gpui::test]
fn a_dropped_reply_allows(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| cx.on_should_terminate(|reply, _| drop(reply)));
    assert_eq!(
        cx.simulate_should_terminate().answer(),
        TerminateAnswer::Allowed
    );
}

/// A quit the application requested itself is already decided, and is not
/// put to the callback a second time.
#[gpui::test]
fn a_quit_the_app_requested_is_not_asked_about(cx: &mut gpui::TestAppContext) {
    let held = hold_replies(cx);
    cx.update(|cx| cx.quit());
    assert_eq!(
        cx.simulate_should_terminate().answer(),
        TerminateAnswer::Allowed
    );
    assert!(held.borrow().is_empty(), "the callback was asked");
}
