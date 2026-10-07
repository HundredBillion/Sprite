use super::*;

fn submit_probe(commands: sprite_term::CommandSender, command: TerminalCommand) {
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    let sender = std::thread::spawn(move || {
        let _ = tx.send(commands.send(command));
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
    let sprite_term::Spawned { session, mut events, mut snapshots } = TerminalSession::spawn(SessionConfig::command("/bin/sh", vec!["-c".into(), "i=0; while [ $i -lt 100 ]; do printf '\\033]2;TITLE%s\\007' $i; i=$((i+1)); done; head -c 1048576 /dev/zero; printf '\\033]8;;https://example.com\\007LINK\\033]8;;\\007'; sleep 30".into()])).unwrap();
    crate::test_blocking_wait::pause(std::time::Duration::from_millis(300));
    snapshots.next_blocking().unwrap();
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
                .is_some_and(|status| status.contains("queue is full"))
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
    loop {
        if matches!(events_rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap(), sprite_term::TerminalEvent::TitleChanged(Some(title)) if title == "TITLE99")
        {
            break;
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
    let sprite_term::Spawned { session, events, mut snapshots } = TerminalSession::spawn(SessionConfig::command("/bin/sh", vec!["-c".into(), "i=0; while [ $i -lt 100 ]; do printf '\\033]2;TITLE%s\\007' $i; i=$((i+1)); done; head -c 1048576 /dev/zero; sleep 30".into()])).unwrap();
    crate::test_blocking_wait::pause(std::time::Duration::from_millis(300));
    snapshots.next_blocking().unwrap();
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
                .is_some_and(|status| status.contains("queue is full"))
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

#[gpui::test]
fn accepted_colors_revert_after_the_other_reload_groups_refuse(cx: &mut gpui::TestAppContext) {
    for accept_colors in [true, false] {
        let mut settings = crate::config::Settings::default();
        settings.cursor.blink = Some(true);
        cx.set_global(crate::config::ActiveSettings(settings.clone()));
        cx.set_global(crate::tokens::TokenRegistry::new(&settings.colors));
        let (view, cx) = cx.add_window_view(|window, cx| {
            TerminalView::failed(
                "accepted colors revert".into(),
                ".SystemUIFont".into(),
                window,
                cx,
            )
        });
        let mut config = SessionConfig::command("/bin/sh", vec!["-c".into(), "stty -echo; i=0; while [ $i -lt 100 ]; do printf '\\033]2;TITLE%s\\007' $i; i=$((i+1)); done; head -c 1048576 /dev/zero; printf 'INPUT_READY\\n'; while read line; do printf 'RESULT:%s\\n\\033]2;RESTORED_READY\\007' \"$line\"; done".into()]);
        let defaults = theme::session_defaults(&settings);
        config.colors = defaults.colors;
        config.cursor = defaults.cursor;
        let sprite_term::Spawned {
            session,
            mut events,
            mut snapshots,
        } = TerminalSession::spawn(config).unwrap();
        crate::test_blocking_wait::pause(std::time::Duration::from_millis(300));
        let mut reloaded = settings.clone();
        let changed_color = Rgb {
            r: 0x12,
            g: 0x34,
            b: 0x56,
        };
        reloaded.colors.foreground = Some(changed_color);
        reloaded.cursor.blink = Some(false);
        view.update_in(cx, |view, window, cx| {
            view.session = SessionState::Running(session);
            view.applied_settings = settings.clone();
            if !accept_colors {
                let mut cursor_only = settings.clone();
                cursor_only.cursor.blink = Some(false);
                view.apply_settings(&cursor_only, window, cx);
                assert_eq!(
                    view.applied_settings.cursor.blink,
                    Some(false),
                    "the reserved slot accepts cursor B"
                );
            }
            view.apply_settings(&reloaded, window, cx);
            assert!(
                view.status
                    .as_ref()
                    .is_some_and(|status| status.contains("queue is full")),
                "the other terminal group refuses after the reserved slot is occupied"
            );
            assert_eq!(view.applied_settings.cursor.blink, Some(accept_colors));
            view.apply_settings(&settings, window, cx);
        });
        let (history_tx, history_rx) = std::sync::mpsc::sync_channel(1);
        let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
        let event_drain = std::thread::spawn(move || {
            while let Ok(event) = events.next_blocking() {
                if matches!(&event, sprite_term::TerminalEvent::TitleChanged(Some(title)) if title == "RESTORED_READY")
                {
                    let _ = ready_tx.send(());
                }
                if let sprite_term::TerminalEvent::History(history) = event
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
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            loop {
                let bundle = rx
                    .recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
                    .expect("snapshot deadline");
                if bundle.pane.rows.iter().any(|row| row.text.contains(text)) {
                    break bundle;
                }
            }
        };
        let changed = wait_for_text("INPUT_READY");
        assert_eq!(
            changed.render.default_foreground,
            if accept_colors {
                changed_color
            } else {
                theme::session_defaults(&settings).fallback_colors.0
            },
            "accepted color state survives refusal of the other group"
        );
        assert_eq!(
            changed.render.cursor.blinking, accept_colors,
            "accepted cursor state survives refusal of the other group"
        );
        view.update_in(cx, |view, window, cx| {
            view.apply_settings(&settings, window, cx);
            assert!(view.submit(TerminalCommand::Input(b"restored\n".to_vec())));
        });
        let restored = wait_for_text("RESULT:restored");
        assert_eq!(
            restored.render.default_foreground,
            theme::session_defaults(&settings).fallback_colors.0,
            "return to A reconciles the accepted B defaults after the prior A submission refused"
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
            TerminalCommand::CaptureHistory(sprite_term::HistoryLines::new(0)),
        );
        let settled = history_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        view.update_in(cx, |view, window, cx| {
            view.apply_settings(&settings, window, cx);
        });
        submit_probe(
            commands,
            TerminalCommand::CaptureHistory(sprite_term::HistoryLines::new(0)),
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
    let titles: String = (0..100)
        .map(|index| format!("\x1b]2;TITLE{index}\x07"))
        .collect();
    let script = format!(
        "stty -echo; printf '%s' '{titles}'; head -c 1048576 /dev/zero; printf 'INPUT_READY\\n'; while read line; do printf '\\nPTY:%s\\n' \"$(stty size)\"; done"
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
    crate::test_blocking_wait::pause(std::time::Duration::from_millis(300));
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
                .is_some_and(|status| status.contains("queue is full"))
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
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
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
    view.update_in(cx, |view, window, _| {
        view.set_allocated(allocated);
        view.synchronise_size(window);
        assert_eq!(
            view.size,
            Some(wanted),
            "identical layout retries the refused size"
        );
    });
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
    let sprite_term::Spawned { session, events, mut snapshots } = TerminalSession::spawn(SessionConfig::command("/bin/sh", vec!["-c".into(), "i=0; while [ $i -lt 100 ]; do printf '\\033]2;TITLE%s\\007' $i; i=$((i+1)); done; head -c 1048576 /dev/zero; sleep 30".into()])).unwrap();
    crate::test_blocking_wait::pause(std::time::Duration::from_millis(300));
    snapshots.next_blocking().unwrap();
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
                .is_some_and(|status| status.contains("queue is full"))
        );
        view.apply_settings(&reloaded, window, cx);
        assert_eq!(
            view.applied_settings.cursor.blink, settings.cursor.blink,
            "refused reload remains unapplied"
        );
        assert!(
            view.status
                .as_ref()
                .is_some_and(|status| status.contains("queue is full"))
        );
        view.begin_shutdown();
    });
    guard.join().unwrap();
}
