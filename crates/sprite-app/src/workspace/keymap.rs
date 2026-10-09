use super::*;

impl Workspace {
    pub(super) fn focus_direction(&mut self, direction: Direction, cx: &mut Context<Self>) {
        if self.tabs.focus_direction(direction).is_some() {
            self.refresh_layout(cx);
            cx.notify();
        }
    }
    pub(super) fn switch_tab(&mut self, forwards: bool, cx: &mut Context<Self>) {
        if forwards {
            self.tabs.next_tab();
        } else {
            self.tabs.previous_tab();
        }
        self.refresh_layout(cx);
        cx.notify();
    }
    pub(super) fn focus_tab(&mut self, tab: TabId, cx: &mut Context<Self>) {
        self.mode = Mode::Idle;
        if self.tabs.focus_tab(tab) {
            self.refresh_layout(cx);
            cx.notify();
        }
    }
    /// Changes the text size of every pane in this window.
    ///
    /// Every pane rather than the focused one: a window with one pane in a
    /// different size from its neighbours looks broken. Each pane re-measures
    /// its cell and tells its child the new grid, which is why this resizes
    /// rather than merely redraws.
    pub(super) fn adjust_font(&mut self, delta: f32, cx: &mut Context<Self>) {
        // A keystroke has no complaints channel, so the size is simply held
        // inside the readable range; a file setting goes through the same
        // rule and says so when it had to.
        let wanted = crate::config::FontSize::new(self.settings.font.size.get() + delta);
        self.apply_font_size(wanted, cx);
    }
    /// Back to the configured size, which is what a person means by "reset" —
    /// not back to Sprite's built-in default.
    pub(super) fn reset_font(&mut self, cx: &mut Context<Self>) {
        let configured = self.configured_font_size;
        self.apply_font_size(configured, cx);
    }
    pub(super) fn apply_font_size(
        &mut self,
        size: crate::config::FontSize,
        cx: &mut Context<Self>,
    ) {
        if size == self.settings.font.size {
            return;
        }
        self.settings.font.size = size;
        // The size is a setting like any other, so it travels the way a reload
        // does: published once, applied by every pane with its own window.
        cx.set_global(crate::config::ActiveSettings(self.settings.clone()));
        cx.notify();
    }
    pub(super) fn focus_active_pane(&self, window: &mut Window, cx: &Context<Self>) {
        let Some(pane) = self.tabs.active().and_then(|tab| tab.focused()) else {
            return;
        };
        let handle = pane.focus_handle(cx);
        // Hosted Surfaces share the pane's focus subtree and keep their keyboard focus.
        if !handle.contains_focused(window, cx) {
            window.focus(&handle);
        }
    }
    pub(super) fn focus_pane(&mut self, pane: PaneId, cx: &mut Context<Self>) {
        if self.tabs.focus_pane(pane) {
            self.mode = Mode::Idle;
            self.refresh_layout(cx);
            cx.notify();
        }
    }
}
/// The workspace's own bindings, resolved before anything reaches a terminal.
///
/// Deliberately few, and all requiring Ctrl+Shift so they cannot collide with
/// what a child program expects to receive.
///
/// Shift is not always a *flag*. GPUI clears `modifiers.shift` for a key whose
/// character has no case to carry it — `-`, `=`, `0` — and reports the shifted
/// glyph instead, so Ctrl+Shift+Minus arrives as Ctrl with the key `_`. The
/// shift is in the glyph rather than the flag, and a binding that insists on
/// the flag never fires. That cost this checkpoint a live test to find.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum WorkspaceAction {
    SplitRight,
    SplitDown,
    ClosePane,
    FontLarger,
    FontSmaller,
    FontReset,
    NewTab,
    CloseTab,
    Quit,
    RenameTab,
    NextTab,
    PreviousTab,
    CycleFocus,
    Focus(Direction),
    Resize(Direction),
}

impl WorkspaceAction {
    /// Whether holding the binding down acts again on every auto-repeat.
    ///
    /// Moving focus, nudging a boundary and stepping the font size are
    /// movements, and a person holds the key to keep them going. Everything
    /// else is one act per press: holding the split key makes one split, and
    /// holding a close key cannot answer the question its own first press
    /// raised. Resetting the size is one act too; repeating it would change
    /// nothing anyway.
    pub(super) fn repeats(self) -> bool {
        matches!(
            self,
            Self::Focus(_) | Self::Resize(_) | Self::FontLarger | Self::FontSmaller
        )
    }
}

pub(super) fn workspace_action(keystroke: &gpui::Keystroke) -> Option<WorkspaceAction> {
    let modifiers = &keystroke.modifiers;
    // Command on macOS and Super on Linux are GPUI's platform modifier. These
    // are direct alternatives to the arrow bindings below, matching the
    // platform-native left/right pane navigation used by Ghostty.
    if modifiers.platform
        && !modifiers.control
        && !modifiers.shift
        && !modifiers.alt
        && !modifiers.function
    {
        return match keystroke.key.as_str() {
            "[" => Some(WorkspaceAction::Focus(Direction::Left)),
            "]" => Some(WorkspaceAction::Focus(Direction::Right)),
            "q" if cfg!(target_os = "macos") => Some(WorkspaceAction::Quit),
            "w" if cfg!(target_os = "linux") => Some(WorkspaceAction::Quit),
            _ => None,
        };
    }
    if !modifiers.control || modifiers.platform {
        return None;
    }
    let key = keystroke.key.as_str();
    // Either spelling of shift counts: the flag, or a glyph that only a shifted
    // key produces. Requiring one means Ctrl+Minus still reaches the child,
    // which is what a program that binds it expects.
    if !(modifiers.shift || matches!(key, "_" | "+" | ")")) {
        return None;
    }
    // Alt belongs to the child, with one exception: the arrows move a boundary.
    // Carving out four keystrokes costs the child nothing a program is likely
    // to want, and resizing without a mouse has to be spelled somehow.
    if modifiers.alt {
        return match key {
            "left" => Some(WorkspaceAction::Resize(Direction::Left)),
            "right" => Some(WorkspaceAction::Resize(Direction::Right)),
            "up" => Some(WorkspaceAction::Resize(Direction::Up)),
            "down" => Some(WorkspaceAction::Resize(Direction::Down)),
            _ => None,
        };
    }
    match key {
        "d" => Some(WorkspaceAction::SplitRight),
        "e" => Some(WorkspaceAction::SplitDown),
        "w" => Some(WorkspaceAction::ClosePane),
        // Both spellings, because a keyboard reports the unshifted key on some
        // layouts and the shifted one on others, and a size binding that works
        // on only one machine is not a binding.
        "=" | "+" => Some(WorkspaceAction::FontLarger),
        "-" | "_" => Some(WorkspaceAction::FontSmaller),
        "0" | ")" => Some(WorkspaceAction::FontReset),
        "t" => Some(WorkspaceAction::NewTab),
        "q" => Some(WorkspaceAction::CloseTab),
        "r" => Some(WorkspaceAction::RenameTab),
        "pagedown" => Some(WorkspaceAction::NextTab),
        "pageup" => Some(WorkspaceAction::PreviousTab),
        "space" => Some(WorkspaceAction::CycleFocus),
        "left" => Some(WorkspaceAction::Focus(Direction::Left)),
        "right" => Some(WorkspaceAction::Focus(Direction::Right)),
        "up" => Some(WorkspaceAction::Focus(Direction::Up)),
        "down" => Some(WorkspaceAction::Focus(Direction::Down)),
        _ => None,
    }
}

impl Workspace {
    pub(super) fn key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let action = workspace_action(&event.keystroke);
        match &self.mode {
            Mode::Renaming(_) => {
                self.rename_key(&event.keystroke, cx);
                cx.stop_propagation();
                return;
            }
            Mode::ConfirmingClose(_) => {
                if event.keystroke.key == "escape" {
                    self.dismiss_pending_close(cx);
                    cx.stop_propagation();
                    return;
                }
                if !matches!(
                    action,
                    Some(
                        WorkspaceAction::ClosePane
                            | WorkspaceAction::CloseTab
                            | WorkspaceAction::Quit
                    )
                ) {
                    self.dismiss_pending_close(cx);
                }
            }
            Mode::DraggingDivider(_) => {
                if action.is_some() {
                    self.end_divider_drag(cx);
                }
            }
            Mode::Idle => {}
        }
        let Some(action) = action else {
            return;
        };
        // Claimed even when ignored below, so a held binding is never typed
        // into the child either.
        cx.stop_propagation();
        if event.is_held && !action.repeats() {
            return;
        }
        match action {
            WorkspaceAction::SplitRight => {
                self.split(Orientation::Horizontal, window, cx);
            }
            WorkspaceAction::SplitDown => {
                self.split(Orientation::Vertical, window, cx);
            }
            WorkspaceAction::ClosePane => self.close_focused_pane(cx),
            WorkspaceAction::FontLarger => self.adjust_font(1.0, cx),
            WorkspaceAction::FontSmaller => self.adjust_font(-1.0, cx),
            WorkspaceAction::FontReset => self.reset_font(cx),
            WorkspaceAction::NewTab => self.open_tab(window, cx),
            WorkspaceAction::CloseTab => self.close_active_tab(cx),
            WorkspaceAction::Quit => self.quit(window, cx),
            WorkspaceAction::RenameTab => self.begin_rename(cx),
            WorkspaceAction::NextTab => self.switch_tab(true, cx),
            WorkspaceAction::PreviousTab => self.switch_tab(false, cx),
            WorkspaceAction::CycleFocus => self.cycle_surface_focus(window, cx),
            WorkspaceAction::Focus(direction) => {
                self.focus_direction(direction, cx);
            }
            WorkspaceAction::Resize(direction) => {
                self.nudge_divider(direction, window, cx);
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::super::test_support::*;
    use super::*;
    use gpui::Modifiers;

    struct KeyboardPane {
        focus: gpui::FocusHandle,
        keys: Rc<std::cell::RefCell<Vec<String>>>,
    }

    impl gpui::Focusable for KeyboardPane {
        fn focus_handle(&self, _: &gpui::App) -> gpui::FocusHandle {
            self.focus.clone()
        }
    }
    impl gpui::Render for KeyboardPane {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div().track_focus(&self.focus).on_key_down(cx.listener(
                |pane, event: &KeyDownEvent, _, _| {
                    pane.keys.borrow_mut().push(event.keystroke.key.clone());
                },
            ))
        }
    }
    impl gpui::EventEmitter<sprite_pane::TitleChanged> for KeyboardPane {}
    impl sprite_pane::Pane for KeyboardPane {
        type Request = SurfaceRequest;
        fn title(&self) -> Option<SharedString> {
            None
        }
        fn close_warning(&self) -> Option<sprite_pane::CloseWarning> {
            Some(sprite_pane::CloseWarning {
                program: Some("editor".into()),
            })
        }
        fn set_allocated(&mut self, _: Size<Pixels>) {}
        fn begin_shutdown(&mut self) -> Option<Box<dyn FnOnce() + Send>> {
            None
        }
    }

    /// One auto-repeat of a key that is still held down.
    fn hold(cx: &mut gpui::VisualTestContext, keystroke: gpui::Keystroke) {
        cx.simulate_event(KeyDownEvent {
            keystroke,
            is_held: true,
        });
    }

    /// Two panes that both report a running program, so every close asks.
    fn busy_panes(workspace: &gpui::Entity<Workspace>, cx: &mut gpui::VisualTestContext) {
        workspace.update(cx, |workspace, cx| {
            let mut make = |_, _| {
                Rc::new(cx.new(|cx| BusyPane {
                    focus: cx.focus_handle(),
                })) as Rc<dyn PaneHandle<Request = SurfaceRequest>>
            };
            workspace.tabs = Tabs::new(&mut make);
            workspace.tabs.split(Orientation::Horizontal, &mut make);
            workspace.tabs.focus_pane(PaneId(0));
            workspace.refresh_layout(cx);
        });
        draw_workspace(cx);
    }

    fn asking(workspace: &gpui::Entity<Workspace>, cx: &mut gpui::VisualTestContext) -> bool {
        workspace.read_with(cx, |workspace, _| {
            matches!(workspace.mode, Mode::ConfirmingClose(_))
        })
    }

    /// Holding a close key auto-repeats it. The first press asks; the repeats
    /// that follow while the key is still down are that same press, so they
    /// neither answer the question nor dismiss it. Letting go and pressing
    /// again is what closes.
    #[gpui::test]
    fn a_held_close_key_cannot_answer_its_own_question(cx: &mut gpui::TestAppContext) {
        let (workspace, cx) = test_workspace(cx);
        busy_panes(&workspace, cx);

        cx.simulate_keystrokes("ctrl-shift-w");
        assert!(asking(&workspace, cx));
        for _ in 0..3 {
            hold(cx, press("w", ctrl_shift()));
        }
        workspace.read_with(cx, |workspace, _| {
            assert_eq!(
                workspace.tabs.active().unwrap().len(),
                2,
                "a held repeat closed the pane"
            );
        });
        assert!(
            asking(&workspace, cx),
            "a held repeat dismissed the question"
        );
        cx.simulate_keystrokes("ctrl-shift-w");
        workspace.read_with(cx, |workspace, _| {
            assert_eq!(
                workspace.tabs.active().unwrap().len(),
                1,
                "a fresh second press still confirms"
            );
            assert!(matches!(workspace.mode, Mode::Idle));
        });

        // The same for a tab. A second tab keeps the window open when one closes.
        workspace.update(cx, |workspace, cx| {
            workspace.tabs.open(|_, _| {
                Rc::new(cx.new(|cx| BusyPane {
                    focus: cx.focus_handle(),
                })) as Rc<dyn PaneHandle<Request = SurfaceRequest>>
            });
            workspace.refresh_layout(cx);
        });
        draw_workspace(cx);
        cx.simulate_keystrokes("ctrl-shift-q");
        assert!(asking(&workspace, cx));
        for _ in 0..3 {
            hold(cx, press("q", ctrl_shift()));
        }
        workspace.read_with(cx, |workspace, _| {
            assert_eq!(workspace.tabs.len(), 2, "a held repeat closed the tab");
        });
        assert!(asking(&workspace, cx));
        cx.simulate_keystrokes("ctrl-shift-q");
        workspace.read_with(cx, |workspace, _| assert_eq!(workspace.tabs.len(), 1));
    }

    /// Holding the split key makes one split and holding the new-tab key one
    /// tab. Movement is what holding a key is for, so the font keeps growing.
    #[gpui::test]
    fn a_held_binding_acts_once_unless_it_is_movement(cx: &mut gpui::TestAppContext) {
        let (workspace, cx) = test_workspace(cx);
        draw_workspace(cx);
        cx.simulate_keystrokes("ctrl-shift-d");
        for _ in 0..3 {
            hold(cx, press("d", ctrl_shift()));
        }
        workspace.read_with(cx, |workspace, _| {
            assert_eq!(
                workspace.tabs.active().unwrap().len(),
                2,
                "holding the split key made more than one split"
            );
        });
        draw_workspace(cx);
        cx.simulate_keystrokes("ctrl-shift-t");
        for _ in 0..3 {
            hold(cx, press("t", ctrl_shift()));
        }
        workspace.read_with(cx, |workspace, _| {
            assert_eq!(
                workspace.tabs.len(),
                2,
                "holding the new-tab key opened more than one tab"
            );
        });
        draw_workspace(cx);
        let before = workspace.read_with(cx, |workspace, _| workspace.settings.font.size.get());
        cx.simulate_keystrokes("ctrl-shift-=");
        for _ in 0..2 {
            hold(cx, press("=", ctrl_shift()));
        }
        let after = workspace.read_with(cx, |workspace, _| workspace.settings.font.size.get());
        assert!(
            (after - before - 3.0).abs() < 1e-3,
            "a held zoom key keeps stepping: {before} -> {after}"
        );
    }

    /// A deliberate act withdraws a close question: a button press anywhere,
    /// or the window going to the background. Pointer motion, a wheel turn, a
    /// modifier on its own and a resize are not decisions and leave it standing.
    #[gpui::test]
    fn a_close_question_is_withdrawn_by_deliberate_input_only(cx: &mut gpui::TestAppContext) {
        use gpui::{MouseButton, ScrollDelta, ScrollWheelEvent};
        let (workspace, cx) = test_workspace(cx);
        busy_panes(&workspace, cx);
        cx.update(|window, _| window.activate_window());
        cx.run_until_parked();

        cx.simulate_keystrokes("ctrl-shift-w");
        assert!(asking(&workspace, cx));
        cx.simulate_resize(gpui::size(px(900.0), px(700.0)));
        draw_workspace(cx);
        let pane = cx.debug_bounds("workspace-pane-0").unwrap().center();
        cx.simulate_mouse_move(pane, None, Modifiers::default());
        cx.simulate_event(ScrollWheelEvent {
            position: pane,
            delta: ScrollDelta::Lines(gpui::point(0.0, 3.0)),
            ..Default::default()
        });
        cx.simulate_modifiers_change(ctrl());
        assert!(
            asking(&workspace, cx),
            "a resize, motion, a wheel turn or a modifier withdrew the question"
        );

        cx.simulate_mouse_down(pane, MouseButton::Right, Modifiers::default());
        cx.simulate_mouse_up(pane, MouseButton::Right, Modifiers::default());
        assert!(
            !asking(&workspace, cx),
            "a button press withdraws the question"
        );
        cx.simulate_keystrokes("ctrl-shift-w");
        assert!(
            asking(&workspace, cx),
            "so the next close asks again rather than closing"
        );

        cx.deactivate_window();
        assert!(
            !asking(&workspace, cx),
            "switching away from the window withdraws the question"
        );
        workspace.read_with(cx, |workspace, _| {
            assert_eq!(workspace.tabs.active().unwrap().len(), 2)
        });
    }

    #[gpui::test]
    fn modal_keys_reach_only_the_intended_consumer(cx: &mut gpui::TestAppContext) {
        let (workspace, cx) = test_workspace(cx);
        let keys = Rc::new(std::cell::RefCell::new(Vec::new()));
        workspace.update(cx, |workspace, cx| {
            let mut make = |_, _| {
                Rc::new(cx.new(|cx| KeyboardPane {
                    focus: cx.focus_handle(),
                    keys: keys.clone(),
                })) as Rc<dyn PaneHandle<Request = SurfaceRequest>>
            };
            workspace.tabs = Tabs::new(&mut make);
            workspace.tabs.split(Orientation::Horizontal, make);
            workspace.tabs.focus_pane(PaneId(0));
            workspace.refresh_layout(cx);
        });
        draw_workspace(cx);
        let first_focus = focused_handle(&workspace, cx);
        cx.simulate_keystrokes("a");
        assert_eq!(&*keys.borrow(), &["a"]);
        cx.simulate_keystrokes("ctrl-shift-r b backspace c enter");
        workspace.read_with(cx, |workspace, _| {
            assert!(matches!(workspace.mode, Mode::Idle));
            assert_eq!(
                workspace.tabs.name(workspace.tabs.active_tab().unwrap()),
                Some("c")
            );
        });
        assert_eq!(&*keys.borrow(), &["a"]);
        cx.simulate_keystrokes("ctrl-shift-r d escape");
        workspace.read_with(cx, |workspace, _| {
            assert_eq!(
                workspace.tabs.name(workspace.tabs.active_tab().unwrap()),
                Some("c")
            )
        });
        cx.simulate_keystrokes("ctrl-shift-w escape");
        assert_eq!(&*keys.borrow(), &["a"]);
        cx.simulate_keystrokes("ctrl-shift-w z");
        workspace.read_with(cx, |workspace, _| {
            assert!(matches!(workspace.mode, Mode::Idle))
        });
        assert_eq!(&*keys.borrow(), &["a", "z"]);
        workspace.update(cx, |workspace, cx| {
            let placed = workspace.dividers[0].0;
            workspace.begin_divider_drag(placed, placed.boundary, cx);
        });
        draw_workspace(cx);
        cx.simulate_keystrokes("y");
        workspace.read_with(cx, |workspace, _| {
            assert!(matches!(workspace.mode, Mode::DraggingDivider(_)))
        });
        assert_eq!(&*keys.borrow(), &["a", "z", "y"]);
        cx.simulate_keystrokes("ctrl-shift-right");
        draw_workspace(cx);
        workspace.read_with(cx, |workspace, _| {
            assert!(matches!(workspace.mode, Mode::Idle))
        });
        let second_focus = focused_handle(&workspace, cx);
        assert_ne!(first_focus, second_focus);
        cx.update(|window, _| assert!(second_focus.is_focused(window)));
        assert_eq!(&*keys.borrow(), &["a", "z", "y"]);
    }

    #[gpui::test]
    fn window_focus_follows_split_tab_switch_and_close(cx: &mut gpui::TestAppContext) {
        let (workspace, cx) = test_workspace(cx);
        draw_workspace(cx);
        let first = focused_handle(&workspace, cx);
        cx.update(|window, _| assert!(first.is_focused(window)));
        cx.simulate_keystrokes("ctrl-shift-d");
        draw_workspace(cx);
        let split = focused_handle(&workspace, cx);
        assert_ne!(first, split);
        cx.update(|window, _| assert!(split.is_focused(window)));
        cx.simulate_keystrokes("ctrl-shift-t");
        draw_workspace(cx);
        let second_tab = focused_handle(&workspace, cx);
        assert_ne!(split, second_tab);
        cx.update(|window, _| assert!(second_tab.is_focused(window)));
        cx.simulate_keystrokes("ctrl-shift-pageup");
        draw_workspace(cx);
        cx.update(|window, _| assert!(split.is_focused(window)));
        cx.simulate_keystrokes("ctrl-shift-w");
        draw_workspace(cx);
        cx.update(|window, _| assert!(first.is_focused(window)));
        cx.simulate_keystrokes("ctrl-shift-q");
        draw_workspace(cx);
        cx.update(|window, _| assert!(second_tab.is_focused(window)));
        workspace.update(cx, |workspace, cx| workspace.close_active_tab(cx));
        draw_workspace(cx);
        workspace.read_with(cx, |workspace, _| {
            assert!(workspace.tabs.active().is_none());
            assert_eq!(workspace.tabs.active_tab(), None);
        });
        workspace.update_in(cx, |workspace, window, cx| workspace.open_tab(window, cx));
        draw_workspace(cx);
        workspace.read_with(cx, |workspace, _| {
            assert!(workspace.tabs.active().is_none());
        });
    }
    #[gpui::test]
    fn modes_cancel_and_close_confirmation_remains_scope_specific(cx: &mut gpui::TestAppContext) {
        use super::Mode;
        use gpui::AppContext;
        let (workspace, cx) = test_workspace(cx);
        workspace.update(cx, |workspace, cx| {
            workspace.tabs = crate::tabs::Tabs::new(|_, _| {
                std::rc::Rc::new(cx.new(|cx| BusyPane {
                    focus: cx.focus_handle(),
                }))
                    as std::rc::Rc<
                        dyn sprite_pane::PaneHandle<
                                Request = crate::surface::channel::SurfaceRequest,
                            >,
                    >
            });
            workspace.tabs.split(Orientation::Horizontal, |_, _| {
                std::rc::Rc::new(cx.new(|cx| BusyPane {
                    focus: cx.focus_handle(),
                }))
                    as std::rc::Rc<
                        dyn sprite_pane::PaneHandle<
                                Request = crate::surface::channel::SurfaceRequest,
                            >,
                    >
            });
            workspace.refresh_layout(cx);
            workspace.begin_rename(cx);
            assert!(matches!(workspace.mode, Mode::Renaming(_)));
            let placed = divider_placements(&workspace.tabs.dividers(), 800.0, 600.0, 0.0)[0];
            workspace.begin_divider_drag(placed, 400.0, cx);
            assert!(matches!(workspace.mode, Mode::DraggingDivider(_)));
            assert!(workspace.mode.renaming().is_none());
            assert!(!workspace.may_close(CloseScope::Pane, cx));
            assert!(matches!(workspace.mode, Mode::ConfirmingClose(_)));
            assert!(workspace.mode.divider_drag().is_none());
            assert!(!workspace.may_close(CloseScope::Tab, cx));
            assert!(workspace.may_close(CloseScope::Tab, cx));
            assert!(matches!(workspace.mode, Mode::Idle));
            assert!(!workspace.may_close(CloseScope::Pane, cx));
        });
        draw_workspace(cx);
        cx.simulate_keystrokes("escape");
        workspace.read_with(cx, |workspace, _| {
            assert!(matches!(workspace.mode, Mode::Idle));
            assert_eq!(workspace.tabs.active().unwrap().len(), 2);
        });
        cx.simulate_keystrokes("ctrl-shift-w");
        workspace.read_with(cx, |workspace, _| {
            assert!(matches!(workspace.mode, Mode::ConfirmingClose(_)))
        });
        cx.simulate_keystrokes("a");
        workspace.read_with(cx, |workspace, _| {
            assert!(matches!(workspace.mode, Mode::Idle))
        });
        cx.simulate_keystrokes("ctrl-shift-w");
        workspace.update(cx, |workspace, cx| {
            workspace.focus_pane(PaneId(0), cx);
            assert!(matches!(workspace.mode, Mode::Idle));
            assert!(!workspace.may_close(CloseScope::Pane, cx));
            workspace.dismiss_pending_close(cx);
        });
        cx.simulate_keystrokes("ctrl-shift-w ctrl-shift-w");
        workspace.read_with(cx, |workspace, _| {
            assert_eq!(workspace.tabs.active().unwrap().len(), 1)
        });
    }
    #[test]
    fn only_movement_and_zoom_repeat_while_held() {
        use WorkspaceAction::*;
        for action in [
            Focus(Direction::Left),
            Focus(Direction::Right),
            Focus(Direction::Up),
            Focus(Direction::Down),
            Resize(Direction::Left),
            Resize(Direction::Right),
            Resize(Direction::Up),
            Resize(Direction::Down),
            FontLarger,
            FontSmaller,
        ] {
            assert!(action.repeats(), "{action:?}");
        }
        for action in [
            SplitRight,
            SplitDown,
            ClosePane,
            FontReset,
            NewTab,
            CloseTab,
            Quit,
            RenameTab,
            NextTab,
            PreviousTab,
            CycleFocus,
        ] {
            assert!(!action.repeats(), "{action:?}");
        }
    }

    #[test]
    fn rename_is_bound_to_ctrl_shift_r() {
        assert_eq!(
            workspace_action(&press("r", ctrl_shift())),
            Some(WorkspaceAction::RenameTab)
        );
        assert_eq!(workspace_action(&press("r", ctrl())), None);
    }
    #[test]
    fn a_letter_binding_needs_both_modifiers() {
        assert_eq!(
            workspace_action(&press("d", ctrl_shift())),
            Some(WorkspaceAction::SplitRight)
        );
        assert_eq!(workspace_action(&press("d", ctrl())), None);
        assert_eq!(workspace_action(&press("d", Modifiers::default())), None);
    }
    #[test]
    fn ctrl_shift_space_cycles_focus_between_the_terminal_and_its_surfaces() {
        assert_eq!(
            workspace_action(&press("space", ctrl_shift())),
            Some(WorkspaceAction::CycleFocus)
        );
        assert_eq!(workspace_action(&press("space", ctrl())), None);
        assert_eq!(
            workspace_action(&press("space", Modifiers::default())),
            None
        );
    }
    /// The defect this checkpoint's live test found: GPUI folds shift into the
    /// glyph for keys with no case, so the size bindings arrive with the flag
    /// already cleared. Insisting on the flag made all three dead keys.
    #[test]
    fn size_bindings_accept_shift_folded_into_the_glyph() {
        assert_eq!(
            workspace_action(&press("_", ctrl())),
            Some(WorkspaceAction::FontSmaller)
        );
        assert_eq!(
            workspace_action(&press("+", ctrl())),
            Some(WorkspaceAction::FontLarger)
        );
        assert_eq!(
            workspace_action(&press(")", ctrl())),
            Some(WorkspaceAction::FontReset)
        );
    }
    #[test]
    fn size_bindings_also_accept_the_flag() {
        assert_eq!(
            workspace_action(&press("-", ctrl_shift())),
            Some(WorkspaceAction::FontSmaller)
        );
        assert_eq!(
            workspace_action(&press("=", ctrl_shift())),
            Some(WorkspaceAction::FontLarger)
        );
        assert_eq!(
            workspace_action(&press("0", ctrl_shift())),
            Some(WorkspaceAction::FontReset)
        );
    }
    /// Ctrl alone belongs to the child. A program that binds Ctrl+Minus keeps
    /// it; only the shifted spelling is the workspace's.
    #[test]
    fn unshifted_ctrl_symbols_reach_the_child() {
        for key in ["-", "=", "0"] {
            assert_eq!(workspace_action(&press(key, ctrl())), None);
        }
    }
    #[test]
    fn other_modifiers_disqualify_a_binding() {
        let with_alt = Modifiers {
            control: true,
            shift: true,
            alt: true,
            ..Modifiers::default()
        };
        assert_eq!(workspace_action(&press("d", with_alt)), None);

        let with_platform = Modifiers {
            control: true,
            shift: true,
            platform: true,
            ..Modifiers::default()
        };
        assert_eq!(workspace_action(&press("_", with_platform)), None);
    }
    #[test]
    fn navigation_keys_keep_their_names() {
        assert_eq!(
            workspace_action(&press("left", ctrl_shift())),
            Some(WorkspaceAction::Focus(Direction::Left))
        );
        assert_eq!(
            workspace_action(&press("right", ctrl_shift())),
            Some(WorkspaceAction::Focus(Direction::Right))
        );
        assert_eq!(
            workspace_action(&press("up", ctrl_shift())),
            Some(WorkspaceAction::Focus(Direction::Up))
        );
        assert_eq!(
            workspace_action(&press("down", ctrl_shift())),
            Some(WorkspaceAction::Focus(Direction::Down))
        );
        assert_eq!(
            workspace_action(&press("pagedown", ctrl_shift())),
            Some(WorkspaceAction::NextTab)
        );
        assert_eq!(workspace_action(&press("f5", ctrl_shift())), None);
    }
    #[test]
    fn platform_brackets_move_focus_left_and_right() {
        assert_eq!(
            workspace_action(&press("[", platform())),
            Some(WorkspaceAction::Focus(Direction::Left))
        );
        assert_eq!(
            workspace_action(&press("]", platform())),
            Some(WorkspaceAction::Focus(Direction::Right))
        );
    }
    #[test]
    fn platform_brackets_require_only_the_platform_modifier() {
        for modifiers in [
            Modifiers {
                platform: true,
                control: true,
                ..Modifiers::default()
            },
            Modifiers {
                platform: true,
                shift: true,
                ..Modifiers::default()
            },
            Modifiers {
                platform: true,
                alt: true,
                ..Modifiers::default()
            },
            Modifiers {
                platform: true,
                function: true,
                ..Modifiers::default()
            },
        ] {
            assert_eq!(workspace_action(&press("[", modifiers)), None);
            assert_eq!(workspace_action(&press("]", modifiers)), None);
        }
    }
    #[test]
    fn platform_quit_shortcut_follows_the_host_convention() {
        #[cfg(target_os = "macos")]
        assert_eq!(
            workspace_action(&press("q", platform())),
            Some(WorkspaceAction::Quit)
        );

        #[cfg(target_os = "linux")]
        assert_eq!(
            workspace_action(&press("w", platform())),
            Some(WorkspaceAction::Quit)
        );
    }
    #[test]
    fn ctrl_shift_alt_arrows_resize() {
        let modifiers = Modifiers {
            control: true,
            shift: true,
            alt: true,
            ..Modifiers::default()
        };
        assert_eq!(
            workspace_action(&press("left", modifiers)),
            Some(WorkspaceAction::Resize(Direction::Left))
        );
        assert_eq!(
            workspace_action(&press("right", modifiers)),
            Some(WorkspaceAction::Resize(Direction::Right))
        );
        assert_eq!(
            workspace_action(&press("up", modifiers)),
            Some(WorkspaceAction::Resize(Direction::Up))
        );
        assert_eq!(
            workspace_action(&press("down", modifiers)),
            Some(WorkspaceAction::Resize(Direction::Down))
        );
    }
    /// Alt still belongs to the child everywhere else, so a program that binds
    /// an alt key keeps it.
    #[test]
    fn alt_disqualifies_every_binding_but_the_arrows() {
        let modifiers = Modifiers {
            control: true,
            shift: true,
            alt: true,
            ..Modifiers::default()
        };
        for key in ["d", "e", "w", "t", "q", "=", "-", "0", "pageup", "pagedown"] {
            assert_eq!(workspace_action(&press(key, modifiers)), None, "{key}");
        }
    }
    /// Without alt the arrows still move focus rather than a boundary.
    #[test]
    fn arrows_without_alt_still_move_focus() {
        assert_eq!(
            workspace_action(&press("left", ctrl_shift())),
            Some(WorkspaceAction::Focus(Direction::Left))
        );
    }
}
