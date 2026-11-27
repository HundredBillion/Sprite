use super::*;

#[test]
fn a_pass_of_title_changes_yields_one_event_with_the_latest_title() {
    let mut terminal = crate::new_terminal(80, 24, 0).expect("terminal");
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

/// ConPTY titles a new console with its program's path, and cmd.exe puts
/// that path back after each command. Neither is a title the program chose:
/// the first is not reported at all, and the second takes the program's own
/// title back.
#[cfg(windows)]
#[test]
fn a_consoles_default_title_is_not_the_programs_title() {
    let mut terminal = crate::new_terminal(80, 24, 0).expect("terminal");
    let notices = Rc::new(RefCell::new(Notices::default()));
    notices.borrow_mut().console_program = Some("cmd.exe".into());
    register_title(&mut terminal, Rc::clone(&notices)).expect("title callback");

    terminal.vt_write(b"\x1b]0;C:\\WINDOWS\\system32\\cmd.exe\x07");
    assert!(notices.borrow_mut().take().is_empty());

    terminal.vt_write(b"\x1b]0;C:\\WINDOWS\\system32\\cmd.exe - vim\x07");
    assert!(matches!(
        notices.borrow_mut().take().as_slice(),
        [TerminalEvent::TitleChanged(Some(title))] if title == "C:\\WINDOWS\\system32\\cmd.exe - vim"
    ));

    terminal.vt_write(b"\x1b]0;C:\\WINDOWS\\system32\\cmd.exe\x07");
    assert!(matches!(
        notices.borrow_mut().take().as_slice(),
        [TerminalEvent::TitleChanged(None)]
    ));
    terminal.vt_write(b"\x1b]0;C:\\WINDOWS\\system32\\cmd.exe\x07");
    assert!(notices.borrow_mut().take().is_empty());
}
