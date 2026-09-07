//! What a terminal event asks the view to do.

use std::sync::Arc;

use gpui::SharedString;
use sprite_term::{HistorySnapshot, SessionError, TerminalEvent};

/// One thing an event asks the view to do.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Effect {
    Status(SharedString),
    HoldPaste(String),
    OpenUrl(String),
    Clipboard(String),
    DeliverHistory(Arc<HistorySnapshot>),
    FailRequest(String),
}

/// What one event implies, and whether the stream is finished.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Decision {
    pub effects: Vec<Effect>,
    pub stop: bool,
}

/// The line a pane shows once its child is gone.
///
/// A signal is reported ahead of a status because a killed child usually has
/// no meaningful one, and an ordinary success says only that it ended: a
/// person closing a shell should not be shown "status 0".
fn describe_exit(exit: &sprite_term::ChildExit) -> String {
    match (&exit.signal, exit.code) {
        (Some(signal), _) => format!("[session ended on {signal}]"),
        (None, Some(0)) => "[session ended]".to_owned(),
        (None, Some(code)) => format!("[session ended with status {code}]"),
        (None, None) => "[session ended]".to_owned(),
    }
}

/// One event in, the effects it implies out.
///
/// Pure on purpose. Every arm of the view's old event loop only *wrote* view
/// state, never read it, so there is nothing to own here — which is what lets
/// all thirteen arms be tested without a GPUI `Window`.
pub(crate) fn decide(event: Result<TerminalEvent, SessionError>) -> Decision {
    let mut effects = Vec::new();
    let mut stop = false;

    match event {
        // Nothing to present. Working directory and bell are carried for
        // observation and for a future bell policy; a title change has no
        // presentation because nothing sets a window title. A graphics probe
        // belongs to whoever asked for it, and a pane draws only text.
        Ok(TerminalEvent::Ready)
        | Ok(TerminalEvent::Bell)
        | Ok(TerminalEvent::WorkingDirectoryChanged(_))
        | Ok(TerminalEvent::TitleChanged(_))
        | Ok(TerminalEvent::Graphics(_))
        // No link, or a refused scheme. Indistinguishable on purpose.
        | Ok(TerminalEvent::Hyperlink { uri: None, .. }) => {}

        Ok(TerminalEvent::UnsafePaste(text)) => {
            // Held, not performed. The person sees why and repeats the paste.
            let lines = text.lines().count();
            effects.push(Effect::HoldPaste(text));
            effects.push(Effect::Status(
                format!(
                    "[paste held: {lines} lines would run as commands — \
                     press Ctrl+Shift+V again to paste anyway]"
                )
                .into(),
            ));
        }

        // Terminal Core already applied the scheme policy, so reaching here
        // means the target is allowed. Sprite never builds a command line from
        // terminal-provided text.
        Ok(TerminalEvent::Hyperlink { uri: Some(uri), .. }) => {
            effects.push(Effect::OpenUrl(uri));
        }

        // Belongs to whoever asked for it. The view forwards because it is the
        // single consumer of this session's events, and arrival order is what
        // lets the registry pair answers with waiters.
        Ok(TerminalEvent::History(history)) => {
            effects.push(Effect::DeliverHistory(history));
        }

        // Terminal Core already applied the OSC 52 policy for one and the
        // person asked for the other; neither needs a policy here.
        Ok(TerminalEvent::ClipboardWrite(text)) | Ok(TerminalEvent::SelectionCopied(text)) => {
            if !text.is_empty() {
                effects.push(Effect::Clipboard(text));
            }
        }

        Ok(TerminalEvent::Error(error)) => {
            // The waiter first: a pane in a bad state must not leave an
            // observation request waiting out the deadline.
            effects.push(Effect::FailRequest(error.to_string()));
            effects.push(Effect::Status(error.to_string().into()));
        }

        Ok(TerminalEvent::Exited(exit)) => {
            effects.push(Effect::Status(describe_exit(&exit).into()));
            stop = true;
        }

        // After the session ends the stream simply closes. That is completion,
        // not a new failure to report.
        Err(_) => stop = true,
    }

    Decision { effects, stop }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sprite_term::CellPosition;

    /// The effects of an event that must leave the pump running.
    ///
    /// Asserting `stop` here rather than in each test means no arm can quietly
    /// gain a `stop = true`: an event that ends the pump early costs the pane
    /// every later snapshot, status line and exit report.
    fn effects(event: TerminalEvent) -> Vec<Effect> {
        let decision = decide(Ok(event));
        assert!(
            !decision.stop,
            "only an exit or a closed stream ends the pump"
        );
        decision.effects
    }

    /// `SessionError::new` is crate-private to `sprite-term`, so a test builds
    /// one from its public fields.
    fn error(operation: &'static str, message: &str) -> SessionError {
        SessionError {
            operation,
            message: message.to_owned(),
        }
    }

    /// `CellPosition` has no `Default`, and no assertion here depends on which
    /// cell a link was resolved at.
    fn origin() -> CellPosition {
        CellPosition { row: 0, column: 0 }
    }

    #[test]
    fn events_with_no_presentation_ask_for_nothing() {
        for event in [
            TerminalEvent::Ready,
            TerminalEvent::Bell,
            TerminalEvent::WorkingDirectoryChanged(None),
            TerminalEvent::Hyperlink {
                position: origin(),
                uri: None,
            },
        ] {
            assert!(effects(event).is_empty());
        }
    }

    #[test]
    fn a_held_paste_explains_itself_and_is_kept() {
        let held = effects(TerminalEvent::UnsafePaste("one\ntwo\n".to_owned()));
        assert_eq!(held.len(), 2, "a held paste both holds and explains");
        assert!(matches!(held[0], Effect::HoldPaste(ref text) if text == "one\ntwo\n"));
        assert!(matches!(held[1], Effect::Status(ref line) if line.contains("2 lines")));
    }

    #[test]
    fn an_allowed_link_is_opened() {
        let opened = effects(TerminalEvent::Hyperlink {
            position: origin(),
            uri: Some("https://example.invalid/".to_owned()),
        });
        assert!(
            matches!(opened.as_slice(), [Effect::OpenUrl(uri)] if uri == "https://example.invalid/")
        );
    }

    #[test]
    fn empty_clipboard_writes_are_not_performed() {
        assert!(effects(TerminalEvent::ClipboardWrite(String::new())).is_empty());
        assert!(effects(TerminalEvent::SelectionCopied(String::new())).is_empty());
    }

    #[test]
    fn clipboard_writes_carry_their_text() {
        assert!(matches!(
            effects(TerminalEvent::ClipboardWrite("copied".to_owned())).as_slice(),
            [Effect::Clipboard(text)] if text == "copied"
        ));
        assert!(matches!(
            effects(TerminalEvent::SelectionCopied("selected".to_owned())).as_slice(),
            [Effect::Clipboard(text)] if text == "selected"
        ));
    }

    /// A pane that fails must say so, not leave a requester waiting out the
    /// observation deadline.
    #[test]
    fn an_error_both_reports_and_fails_the_waiter() {
        let raised = effects(TerminalEvent::Error(error("read", "broke")));
        assert_eq!(raised.len(), 2);
        // The reason, not just the fact: this string is what the waiter gets
        // back in place of an observation timeout, so an empty one would leave
        // the requester no better off than the deadline it replaced.
        assert!(
            matches!(&raised[0], Effect::FailRequest(reason) if reason.contains("broke")),
            "the waiter is told why, not just that it failed"
        );
        assert!(matches!(raised[1], Effect::Status(_)));
    }

    /// A session error is a report, not an ending. Stopping the pump here would
    /// cost the pane every later snapshot, status line and exit report over one
    /// recoverable failure.
    #[test]
    fn a_session_error_does_not_end_the_pump() {
        let decision = decide(Ok(TerminalEvent::Error(error("read", "broke"))));
        assert!(
            !decision.stop,
            "a session error is reported, not fatal to the pump"
        );
    }

    #[test]
    fn a_finished_stream_stops_the_loop() {
        assert!(decide(Err(error("ended", "closed"))).stop);
        assert!(decide(Err(error("ended", "closed"))).effects.is_empty());
        assert!(decide(Ok(TerminalEvent::Ready)).effects.is_empty());
        assert!(!decide(Ok(TerminalEvent::Ready)).stop);
    }

    /// Nothing sets a window title, so the event has no presentation. The old
    /// arm called `cx.notify()` for it, repainting once per shell prompt.
    #[test]
    fn a_title_change_asks_for_nothing() {
        assert!(effects(TerminalEvent::TitleChanged(Some("x".to_owned()))).is_empty());
    }

    /// A graphics probe belongs to whoever asked for it; the view draws only
    /// text, so there is nothing to present.
    #[test]
    fn a_graphics_answer_asks_for_nothing() {
        assert!(effects(TerminalEvent::Graphics(Arc::default())).is_empty());
    }

    #[test]
    fn a_history_answer_is_forwarded_whole() {
        let captured = snapshot();
        assert!(matches!(
            effects(TerminalEvent::History(Arc::clone(&captured))).as_slice(),
            [Effect::DeliverHistory(delivered)] if delivered == &captured
        ));
    }

    /// An exit is the one event that both says something and ends the stream:
    /// there is nothing more to read once the child is gone.
    #[test]
    fn an_exit_is_reported_and_ends_the_loop() {
        let ended = decide(Ok(TerminalEvent::Exited(sprite_term::ChildExit {
            code: Some(3),
            signal: None,
            requested: false,
        })));
        assert!(ended.stop);
        assert!(
            matches!(ended.effects.as_slice(), [Effect::Status(line)] if line.contains("status 3"))
        );
    }

    fn snapshot() -> Arc<HistorySnapshot> {
        Arc::new(HistorySnapshot {
            generation: 1,
            size: sprite_term::TerminalSize::DEFAULT,
            screen: sprite_term::ScreenKind::Primary,
            rows: vec![sprite_term::PaneRow {
                text: "answer".to_owned(),
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
                style: Default::default(),
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
}
