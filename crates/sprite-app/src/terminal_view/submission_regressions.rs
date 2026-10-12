use super::*;

fn submit_probe(commands: sprite_term::CommandSender, command: TerminalCommand) {
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    let sender = std::thread::spawn(move || {
        tx.send(commands.send(command)).ok();
    });
    rx.recv_timeout(std::time::Duration::from_secs(5))
        .expect("probe admission deadline")
        .expect("probe command accepted");
    sender.join().unwrap();
}

#[gpui::test]
fn rejected_link_requests_recover_after_event_pressure(cx: &mut gpui::TestAppContext) {
    let settings = crate::config::Settings::default();
    cx.set_global(crate::config::ActiveSettings(settings));
    let (view, cx) = cx.add_window_view(|window, cx| {
        TerminalView::failed("link recovery".into(), ".SystemUIFont".into(), window, cx)
    });
    // The release line can arrive before `stty -echo` runs and be echoed, so
    // the link is written from the top-left corner where the test looks for it.
    let sprite_term::Spawned {
        session,
        mut events,
        mut snapshots,
    } = TerminalSession::spawn(SessionConfig::command(
        "/bin/sh",
        vec![
            "-c".into(),
            crate::test_event_pressure::pressure_script(
                "printf '\\033[H\\033]8;;https://example.com\\007LINK\\033]8;;\\007'; sleep 30",
            )
            .into(),
        ],
    ))
    .unwrap();
    crate::test_event_pressure::focus_and_release(&session.commands());
    crate::test_event_pressure::saturate_control(&session.commands());
    let position = sprite_term::CellPosition { row: 0, column: 0 };
    view.update(cx, |view, _| {
        view.request_hover_link(position);
        view.request_link_click(position);
        assert!(
            view.hover_request.is_none() && view.pending_link_click.is_none(),
            "a pane without a worker cannot own an outstanding response"
        );
        view.session = SessionState::Running(session);
        view.hovered_cell = Some(position);
        view.request_hover_link(position);
        assert!(
            view.hover_request.is_none(),
            "refused hover must leave no outstanding request"
        );
        view.request_link_click(position);
        assert!(
            view.pending_link_click.is_none(),
            "refused click must leave no outstanding request"
        );
        assert!(
            view.status
                .as_ref()
                .is_some_and(|status| status.contains(BUSY_STATUS))
        );
    });
    let (events_tx, events_rx) = std::sync::mpsc::sync_channel(1);
    let event_drain = std::thread::spawn(move || {
        while let Ok(event) = events.next_blocking() {
            if events_tx.send(event).is_err() {
                break;
            }
        }
    });
    let (snapshots_tx, snapshots_rx) = std::sync::mpsc::sync_channel(1);
    let snapshot_drain = std::thread::spawn(move || {
        while let Ok(bundle) = snapshots.next_blocking() {
            if snapshots_tx.send(bundle).is_err() {
                break;
            }
        }
    });
    // Every write of the burst has been delivered once a hundred have arrived.
    let mut writes = 0;
    while writes < 100 {
        if matches!(
            events_rx
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap(),
            sprite_term::TerminalEvent::ClipboardWrite(_)
        ) {
            writes += 1;
        }
    }
    let bundle = loop {
        let bundle = snapshots_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        if bundle.pane.rows.iter().any(|row| row.text.contains("LINK")) {
            break bundle;
        }
    };
    view.update(cx, |view, _| {
        view.bundle = Some(bundle);
        view.request_hover_link(position);
        assert!(
            view.hover_request.is_some(),
            "accepted hover owns its outstanding request"
        );
    });
    let sprite_term::TerminalEvent::Hyperlink {
        position,
        request_id,
        generation,
        uri,
        span,
    } = events_rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap()
    else {
        panic!("expected real terminal link resolution");
    };
    assert_eq!(uri.as_deref(), Some("https://example.com"));
    let handle = view.update(cx, |view, cx| {
        view.apply(
            crate::terminal_events::Effect::HyperlinkResolved {
                position,
                request_id,
                generation,
                uri,
                span,
            },
            cx,
        );
        assert!(view.hover_request.is_none());
        assert!(
            view.hovered_link.is_some(),
            "hover resumes after refused request"
        );
        assert!(view.pending_link_click.is_none());
        view.begin_shutdown().unwrap()
    });
    handle.wait().unwrap();
    drop(events_rx);
    drop(snapshots_rx);
    event_drain.join().unwrap();
    snapshot_drain.join().unwrap();
}

#[gpui::test]
fn partially_refused_reload_reverts_actual_local_state(cx: &mut gpui::TestAppContext) {
    let settings = crate::config::Settings::default();
    cx.set_global(crate::config::ActiveSettings(settings.clone()));
    cx.set_global(crate::tokens::TokenRegistry::new(&settings.colors));
    let (view, cx) = cx.add_window_view(|window, cx| {
        TerminalView::failed("reload revert".into(), ".SystemUIFont".into(), window, cx)
    });
    let sprite_term::Spawned {
        session,
        events,
        snapshots: _snapshots,
    } = TerminalSession::spawn(SessionConfig::command(
        "/bin/sh",
        vec![
            "-c".into(),
            crate::test_event_pressure::pressure_script("sleep 30").into(),
        ],
    ))
    .unwrap();
    crate::test_event_pressure::focus_and_release(&session.commands());
    crate::test_event_pressure::saturate_control(&session.commands());
    let image = sprite_term::ImagePixels {
        id: 88,
        generation: 1,
        width: 1,
        height: 1,
        transmitted: sprite_term::TransmittedFormat::Rgba,
        pixels: vec![255; 4],
    };
    let mut reloaded = settings.clone();
    reloaded.font.size = crate::config::FontSize::new(21.0);
    reloaded.grid.padding = crate::config::Padding::new(24.0);
    reloaded.graphics.texture_bytes = crate::config::TextureBytes::new(0);
    reloaded.colors.foreground = Some(Rgb {
        r: 0x12,
        g: 0x34,
        b: 0x56,
    });
    reloaded.cursor.blink = Some(false);
    let handle = view.update_in(cx, |view, window, cx| {
        view.session = SessionState::Running(session);
        view.set_allocated(gpui::size(px(800.0), px(480.0)));
        assert!(view.textures.texture(&image).is_some());
        view.apply_settings(&reloaded, window, cx);
        assert_eq!(view.metrics.font_size(), px(21.0));
        assert_eq!(view.padding, 24.0);
        assert_eq!(
            view.fallback_colors,
            theme::session_defaults(&reloaded).fallback_colors
        );
        assert!(view.textures.texture(&image).is_none());
        assert!(
            view.status
                .as_ref()
                .is_some_and(|status| status.contains(BUSY_STATUS))
        );
        view.apply_settings(&settings, window, cx);
        assert_eq!(
            view.metrics.font_size(),
            px(settings.font.size.get()),
            "revert must restore the actually changed renderer font"
        );
        assert_eq!(view.padding, settings.grid.padding.get());
        assert_eq!(
            view.fallback_colors,
            theme::session_defaults(&settings).fallback_colors,
            "fallback reverts even when the terminal never accepted B colors"
        );
        assert!(
            view.textures.texture(&image).is_some(),
            "revert restores the actual texture admission budget"
        );
        assert!(
            view.size.is_none(),
            "refused geometry stays retryable independently of local font state"
        );
        view.begin_shutdown().unwrap()
    });
    handle.wait().unwrap();
    drop(events);
}

/// A reload the worker refuses is kept whole and reverted whole: nothing of
/// it reaches the terminal, and returning to what is applied owes nothing.
#[gpui::test]
fn a_refused_reload_is_kept_whole_and_reverted_whole(cx: &mut gpui::TestAppContext) {
    let mut settings = crate::config::Settings::default();
    settings.cursor.blink = Some(true);
    cx.set_global(crate::config::ActiveSettings(settings.clone()));
    cx.set_global(crate::tokens::TokenRegistry::new(&settings.colors));
    let (view, cx) = cx.add_window_view(|window, cx| {
        TerminalView::failed(
            "refused reload revert".into(),
            chosen_family(window, settings.font.family.as_deref(), cx).0,
            window,
            cx,
        )
    });
    let mut config = SessionConfig::command("/bin/sh", vec!["-c".into(), crate::test_event_pressure::pressure_script("printf 'INPUT_READY\\n'; while read line; do printf 'RESULT:%s\\n\\033]2;RESTORED_READY\\007' \"$line\"; done").into()]);
    let defaults = theme::session_defaults(&settings);
    config.colors = defaults.colors;
    config.cursor = defaults.cursor;
    let sprite_term::Spawned {
        session,
        mut events,
        mut snapshots,
    } = TerminalSession::spawn(config).unwrap();
    crate::test_event_pressure::focus_and_release(&session.commands());
    crate::test_event_pressure::saturate_control(&session.commands());
    let changed_color = Rgb {
        r: 0x12,
        g: 0x34,
        b: 0x56,
    };
    let mut reloaded = settings.clone();
    reloaded.colors.foreground = Some(changed_color);
    reloaded.cursor.blink = Some(false);
    view.update_in(cx, |view, window, cx| {
        view.session = SessionState::Running(session);
        view.applied_settings = settings.clone();
        view.apply_settings(&reloaded, window, cx);
        assert!(
            view.status
                .as_ref()
                .is_some_and(|status| status.contains(BUSY_STATUS))
        );
        assert_eq!(
            view.applied_settings.colors, settings.colors,
            "nothing was admitted"
        );
        assert_eq!(view.applied_settings.cursor.blink, settings.cursor.blink);
        assert_eq!(
            view.pending.settings.as_ref(),
            Some(&reloaded),
            "the reload is kept whole"
        );
        view.apply_settings(&settings, window, cx);
        assert!(
            view.pending.settings.is_none(),
            "reverting to what is applied owes nothing"
        );
    });
    let (history_tx, history_rx) = std::sync::mpsc::sync_channel(1);
    let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
    let event_drain = std::thread::spawn(move || {
        while let Ok(event) = events.next_blocking() {
            if matches!(&event, sprite_term::TerminalEvent::TitleChanged(Some(title)) if title == "RESTORED_READY")
            {
                ready_tx.send(()).ok();
            }
            if let sprite_term::TerminalEvent::History {
                snapshot: history, ..
            } = event
                && history_tx.send(history).is_err()
            {
                break;
            }
        }
    });
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    let snapshot_drain = std::thread::spawn(move || {
        while let Ok(bundle) = snapshots.next_blocking() {
            if tx.send(bundle).is_err() {
                break;
            }
        }
    });
    let wait_for_text = |text: &str| {
        let deadline = std::time::Instant::now() + crate::test_blocking_wait::watchdog();
        loop {
            let bundle = rx
                .recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
                .expect("snapshot deadline");
            if bundle.pane.rows.iter().any(|row| row.text.contains(text)) {
                break bundle;
            }
        }
    };
    let unreached = wait_for_text("INPUT_READY");
    assert_eq!(
        unreached.render.default_foreground, defaults.fallback_colors.0,
        "the refused colors never reached the terminal"
    );
    assert!(
        unreached.render.cursor.blinking,
        "the refused cursor never reached the terminal"
    );
    view.update_in(cx, |view, window, cx| {
        view.apply_settings(&settings, window, cx);
        assert!(view.submit(TerminalCommand::Input(b"restored\n".to_vec())));
    });
    let restored = wait_for_text("RESULT:restored");
    assert_eq!(
        restored.render.default_foreground,
        defaults.fallback_colors.0
    );
    assert!(restored.render.cursor.blinking);
    ready_rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("complete child response");
    let commands = view.update(cx, |view, _| match &view.session {
        SessionState::Running(session) => session.commands(),
        _ => panic!("probe session ended"),
    });
    submit_probe(
        commands.clone(),
        TerminalCommand::CaptureHistory {
            ticket: sprite_term::Ticket::new(0),
            lines: sprite_term::HistoryLines::new(0),
        },
    );
    let settled = history_rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    view.update_in(cx, |view, window, cx| {
        view.apply_settings(&settings, window, cx);
    });
    submit_probe(
        commands,
        TerminalCommand::CaptureHistory {
            ticket: sprite_term::Ticket::new(0),
            lines: sprite_term::HistoryLines::new(0),
        },
    );
    let unchanged = history_rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    assert_eq!(
        unchanged.generation, settled.generation,
        "unchanged reload must not reset terminal defaults"
    );
    let handle = view.update(cx, |view, _| view.begin_shutdown().unwrap());
    handle.wait().unwrap();
    drop(rx);
    drop(history_rx);
    snapshot_drain.join().unwrap();
    event_drain.join().unwrap();
}

#[gpui::test]
fn refused_resize_and_font_reload_retry_the_identical_layout(cx: &mut gpui::TestAppContext) {
    let settings = crate::config::Settings::default();
    cx.set_global(crate::config::ActiveSettings(settings.clone()));
    cx.set_global(crate::tokens::TokenRegistry::new(&settings.colors));
    let (view, cx) = cx.add_window_view(|window, cx| {
        TerminalView::failed(
            "resize regression".into(),
            ".SystemUIFont".into(),
            window,
            cx,
        )
    });
    let burst = crate::test_event_pressure::CLIPBOARD_WRITE.repeat(100);
    let script = format!(
        "stty -echo; read _; printf '%s' '{burst}'; head -c 1048576 /dev/zero; printf 'INPUT_READY\\n'; while read line; do printf '\\nPTY:%s\\n' \"$(stty size)\"; done"
    );
    let sprite_term::Spawned {
        session,
        mut events,
        mut snapshots,
    } = TerminalSession::spawn(SessionConfig::command(
        "/bin/sh",
        vec!["-c".into(), script.into()],
    ))
    .unwrap();
    crate::test_event_pressure::focus_and_release(&session.commands());
    crate::test_event_pressure::saturate_control(&session.commands());
    let initial = snapshots.next_blocking().unwrap();
    let allocated = gpui::size(px(800.0), px(480.0));
    let mut reloaded = settings.clone();
    reloaded.font.size = crate::config::FontSize::new(21.0);
    let wanted = view.update_in(cx, |view, window, cx| {
        view.session = SessionState::Running(session);
        view.size = Some(initial.pane.size);
        view.set_allocated(allocated);
        view.apply_settings(&reloaded, window, cx);
        let wanted = geometry::grid_size(
            crate::grid::content_area(allocated, view.padding),
            view.metrics.width(),
            view.metrics.height(),
            window.scale_factor(),
        )
        .unwrap();
        assert_ne!(wanted, initial.pane.size);
        assert!(
            view.status
                .as_ref()
                .is_some_and(|status| status.contains(BUSY_STATUS))
        );
        assert_ne!(
            view.size,
            Some(wanted),
            "refused resize must remain retryable"
        );
        assert_eq!(
            view.applied_settings.font.size, reloaded.font.size,
            "renderer font is applied independently of refused PTY geometry"
        );
        wanted
    });
    let event_drain = std::thread::spawn(move || while events.next_blocking().is_ok() {});
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    let snapshot_drain = std::thread::spawn(move || {
        while let Ok(bundle) = snapshots.next_blocking() {
            if tx.send(bundle).is_err() {
                break;
            }
        }
    });
    let wait_for_text = |text: &str| {
        let deadline = std::time::Instant::now() + crate::test_blocking_wait::watchdog();
        loop {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            let bundle = rx
                .recv_timeout(remaining)
                .expect("terminal output deadline");
            if bundle.pane.rows.iter().any(|row| row.text.contains(text)) {
                break bundle;
            }
        }
    };
    wait_for_text("INPUT_READY");
    let retry_deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !view.update_in(cx, |view, window, _| {
        view.set_allocated(allocated);
        view.synchronise_size(window);
        view.size == Some(wanted)
    }) {
        assert!(
            std::time::Instant::now() < retry_deadline,
            "identical layout must admit the refused size after pressure clears"
        );
        crate::test_blocking_wait::pause(std::time::Duration::from_millis(5));
        cx.run_until_parked();
    }
    let commands = view.update(cx, |view, _| match &view.session {
        SessionState::Running(session) => session.commands(),
        _ => panic!("probe session ended"),
    });
    submit_probe(commands, TerminalCommand::Input(b"report\n".to_vec()));
    let report = wait_for_text(&format!("PTY:{} {}", wanted.rows(), wanted.cols()));
    assert_eq!(
        report.pane.size, wanted,
        "snapshot and kernel PTY size agree"
    );
    let handle = view.update_in(cx, |view, window, cx| {
        view.apply_settings(&reloaded, window, cx);
        assert_eq!(view.applied_settings.font.size, reloaded.font.size);
        view.begin_shutdown().unwrap()
    });
    handle.wait().unwrap();
    drop(rx);
    snapshot_drain.join().unwrap();
    event_drain.join().unwrap();
}

#[gpui::test]
fn saturated_ui_submission_and_reload_are_visible_refusals(cx: &mut gpui::TestAppContext) {
    let settings = crate::config::Settings::default();
    cx.set_global(crate::config::ActiveSettings(settings.clone()));
    cx.set_global(crate::tokens::TokenRegistry::new(&settings.colors));
    let (sender, _exits) = async_channel::unbounded();
    let (view, cx) = cx.add_window_view(|window, cx| {
        TerminalView::new(
            Some(vec!["/bin/sleep".into(), "30".into()]),
            settings.clone(),
            Vec::new(),
            None,
            PaneExit {
                sender,
                identity: (crate::tabs::TabId(0), crate::pane_tree::PaneId(0)),
            },
            window,
            cx,
        )
    });
    let sprite_term::Spawned {
        session,
        events,
        snapshots: _snapshots,
    } = TerminalSession::spawn(SessionConfig::command(
        "/bin/sh",
        vec![
            "-c".into(),
            crate::test_event_pressure::pressure_script("sleep 30").into(),
        ],
    ))
    .unwrap();
    crate::test_event_pressure::focus_and_release(&session.commands());
    crate::test_event_pressure::saturate_control(&session.commands());
    let guard = std::thread::spawn(move || {
        crate::test_blocking_wait::pause(std::time::Duration::from_secs(1));
        drop(events);
    });
    let mut reloaded = settings.clone();
    reloaded.cursor.blink = Some(false);
    view.update_in(cx, |view, window, cx| {
        view.session = SessionState::Running(session);
        view.status = None;
        let started = std::time::Instant::now();
        view.send(TerminalCommand::Input(b"x".to_vec()));
        assert!(
            started.elapsed() < std::time::Duration::from_millis(200),
            "GPUI submission waited for the worker"
        );
        assert!(
            view.status
                .as_ref()
                .is_some_and(|status| status.contains(BUSY_STATUS))
        );
        view.apply_settings(&reloaded, window, cx);
        assert_eq!(
            view.applied_settings.cursor.blink, settings.cursor.blink,
            "refused reload remains unapplied"
        );
        assert!(
            view.status
                .as_ref()
                .is_some_and(|status| status.contains(BUSY_STATUS))
        );
        view.begin_shutdown();
    });
    guard.join().unwrap();
}
