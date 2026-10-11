use super::*;
use libghostty_vt::terminal::Options as TerminalOptions;

#[test]
fn a_pass_of_title_changes_yields_one_event_with_the_latest_title() {
    let mut terminal = Terminal::new(TerminalOptions {
        cols: 80,
        rows: 24,
        max_scrollback: 0,
    })
    .expect("terminal");
    let notices = Rc::new(RefCell::new(Notices::default()));
    register_title(&mut terminal, Rc::clone(&notices)).expect("title callback");
    register_pwd(&mut terminal, Rc::clone(&notices)).expect("pwd callback");

    let retitles: String = (0..100)
        .map(|index| format!("\x1b]2;title-{index}\x07"))
        .collect();
    terminal.vt_write(retitles.as_bytes());
    // A second chunk in the same pass is still the same pass.
    terminal.vt_write(b"\x1b]7;file:///tmp/one\x07\x1b]7;file:///tmp/two\x07");

    let events = notices.borrow_mut().take();
    assert!(
        matches!(
            events.as_slice(),
            [
                TerminalEvent::TitleChanged(Some(title)),
                TerminalEvent::WorkingDirectoryChanged(Some(directory)),
            ] if title == "title-99" && directory.contains("/tmp/two")
        ),
        "{events:?}"
    );
    assert!(
        notices.borrow_mut().take().is_empty(),
        "a taken notice is not reported twice"
    );
}
