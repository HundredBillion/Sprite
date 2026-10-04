use super::*;

impl Workspace {
    pub(super) fn refresh_titles(&mut self, cx: &mut Context<Self>) {
        let all = self.tabs.all_panes();
        self.pane_titles.retain(|id, _| {
            all.iter()
                .any(|(_, _, pane)| pane.view().entity_id() == *id)
        });
        for (_, _, pane) in all {
            let id = pane.view().entity_id();
            self.pane_titles.entry(id).or_insert_with(|| {
                let workspace = cx.weak_entity();
                let subscription = pane.subscribe_title(
                    cx,
                    Box::new(move |event, cx| {
                        let _ = workspace.update(cx, |workspace, cx| {
                            if let Some(cached) = workspace.pane_titles.get_mut(&id)
                                && cached.title != event.0
                            {
                                cached.title = event.0.clone();
                                workspace.refresh_labels();
                                cx.notify();
                            }
                        });
                    }),
                );
                PaneTitle {
                    title: pane.title(cx),
                    _subscription: subscription,
                }
            });
        }
        self.refresh_labels();
    }

    pub(super) fn refresh_labels(&mut self) {
        self.labels = self
            .tabs
            .order()
            .into_iter()
            .enumerate()
            .map(|(index, tab)| {
                let title = self
                    .tabs
                    .focused_in(tab)
                    .and_then(|pane| self.pane_titles.get(&pane.view().entity_id()))
                    .and_then(|cached| cached.title.as_ref());
                (
                    tab,
                    tab_label(
                        self.tabs.name(tab),
                        title.map(|title| title.as_ref()),
                        index,
                    ),
                )
            })
            .collect();
        self.wanted_title = self
            .tabs
            .active()
            .and_then(|tab| tab.focused())
            .and_then(|pane| self.pane_titles.get(&pane.view().entity_id()))
            .and_then(|cached| cached.title.clone())
            .unwrap_or_else(|| window_title(None).into());
    }
    pub(super) fn begin_rename(&mut self, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.active_tab() else {
            return;
        };
        // Start from the current name, so a rename edits rather than retypes.
        let text = self.tabs.name(tab).unwrap_or_default().to_owned();
        self.mode = Mode::Renaming(TabRename {
            tab,
            label: display_text(format!("{text}\u{258f}")),
            text,
        });
        cx.notify();
    }
    /// One keystroke into a rename in progress. True when the key was for the
    /// rename and must go no further.
    pub(super) fn rename_key(
        &mut self,
        keystroke: &gpui::Keystroke,
        cx: &mut Context<Self>,
    ) -> bool {
        let Mode::Renaming(renaming) = &mut self.mode else {
            return false;
        };
        match rename_step(&renaming.text, keystroke) {
            RenameStep::Editing(text) => {
                renaming.label = display_text(format!("{text}\u{258f}"));
                renaming.text = text;
            }
            RenameStep::Commit(text) => {
                let tab = renaming.tab;
                let name = (!text.is_empty()).then_some(text);
                self.tabs.set_name(tab, name);
                self.refresh_labels();
                self.mode = Mode::Idle;
            }
            RenameStep::Cancel => self.mode = Mode::Idle,
        }
        cx.notify();
        true
    }
}
/// What a tab shows: the Tab Name if a person gave one, else the focused pane's
/// Pane Title, else the tab's position counted from one.
///
/// The focused pane's title rather than any other pane's, because it is the
/// only choice that stays stable as focus moves within a split tab. Pure, so
/// the order can be asserted without a window.
pub(super) fn tab_label(name: Option<&str>, title: Option<&str>, index: usize) -> SharedString {
    match (name, title) {
        (Some(name), _) => display_text(name.to_owned()),
        (None, Some(title)) => display_text(title.to_owned()),
        (None, None) => display_text(format!("{}", index + 1)),
    }
}

/// A tab whose name is being typed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct TabRename {
    pub(super) tab: TabId,
    pub(super) text: String,
    pub(super) label: SharedString,
}

/// Where one keystroke leaves a name being typed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum RenameStep {
    Editing(String),
    Commit(String),
    Cancel,
}

/// The whole of a text field, for a name: append the typed character, delete
/// the last one, keep, or abandon. GPUI has no text field, and a tab name needs
/// none of what one would add — no cursor movement, no selection, no IME
/// composition. Pure, so every key can be asserted without a window.
pub(super) fn rename_step(text: &str, keystroke: &gpui::Keystroke) -> RenameStep {
    match keystroke.key.as_str() {
        "enter" => RenameStep::Commit(text.to_owned()),
        "escape" => RenameStep::Cancel,
        "backspace" => {
            let mut text = text.to_owned();
            text.pop();
            RenameStep::Editing(text)
        }
        _ => match &keystroke.key_char {
            Some(typed) if !keystroke.modifiers.control && !keystroke.modifiers.platform => {
                RenameStep::Editing(format!("{text}{typed}"))
            }
            _ => RenameStep::Editing(text.to_owned()),
        },
    }
}

/// The title bar: the focused pane's Pane Title alone, or the application's name
/// when it has none. The Dock and the switcher already say which application
/// this is, so the title is spent on what is running.
pub(super) fn window_title(title: Option<&str>) -> &str {
    title.unwrap_or("Sprite")
}

impl Workspace {
    pub(super) fn tab_elements(&self, cx: &mut Context<Self>) -> Vec<gpui::Div> {
        let active_tab = self.tabs.active_tab();
        self.labels
            .iter()
            .map(|(tab, cached_label)| {
                let tab = *tab;
                let is_active = Some(tab) == active_tab;
                let editing = self
                    .mode
                    .renaming()
                    .filter(|renaming| renaming.tab == tab)
                    .map(|renaming| renaming.label.clone());
                // A thin bar after the text stands for the caret; there is no
                // cursor to move, so a glyph is all the field needs.
                let label: SharedString = match &editing {
                    Some(label) => label.clone(),
                    None => cached_label.clone(),
                };
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .px(px(14.0))
                    .h_full()
                    .text_size(px(12.0))
                    .bg(rgb(if is_active { TAB_ACTIVE_BG } else { BACKGROUND }))
                    .when(editing.is_some(), |element| element.bg(rgb(TAB_EDIT_BG)))
                    .text_color(rgb(if is_active {
                        TAB_ACTIVE_FG
                    } else {
                        TAB_INACTIVE_FG
                    }))
                    .on_mouse_down(
                        gpui::MouseButton::Left,
                        cx.listener(move |workspace, _event, _window, cx| {
                            workspace.focus_tab(tab, cx);
                        }),
                    )
                    .child(label)
            })
            .collect()
    }
}
#[cfg(test)]
mod tests {
    use super::super::test_support::*;
    use super::*;
    #[gpui::test]
    fn pushed_titles_update_labels_and_closed_panes_release_subscriptions(
        cx: &mut gpui::TestAppContext,
    ) {
        use gpui::AppContext;
        let (workspace, cx) = test_workspace(cx);
        let pane = workspace.update(cx, |workspace, cx| {
            let pane = cx.new(|cx| BusyPane {
                focus: cx.focus_handle(),
            });
            workspace.tabs = crate::tabs::Tabs::new(|_, _| {
                std::rc::Rc::new(pane.clone())
                    as std::rc::Rc<
                        dyn sprite_pane::PaneHandle<
                                Request = crate::surface::channel::SurfaceRequest,
                            >,
                    >
            });
            workspace.refresh_layout(cx);
            pane
        });
        workspace.read_with(cx, |workspace, _| {
            assert_eq!(workspace.labels[0].1.as_ref(), "1");
            assert_eq!(workspace.wanted_title.as_ref(), "Sprite");
        });
        pane.update(cx, |_, cx| {
            cx.emit(sprite_pane::TitleChanged(Some("editor".into())))
        });
        workspace.read_with(cx, |workspace, _| {
            assert_eq!(workspace.labels[0].1.as_ref(), "editor");
            assert_eq!(workspace.wanted_title.as_ref(), "editor");
        });
        workspace.update(cx, |workspace, cx| {
            workspace.begin_rename(cx);
            workspace.rename_key(&plain("a", Some("a")), cx);
            workspace.rename_key(&plain("enter", None), cx);
        });
        pane.update(cx, |_, cx| {
            cx.emit(sprite_pane::TitleChanged(Some("changed".into())))
        });
        workspace.read_with(cx, |workspace, _| {
            assert_eq!(workspace.labels[0].1.as_ref(), "a");
            assert_eq!(workspace.wanted_title.as_ref(), "changed");
        });
        pane.update(cx, |_, cx| cx.emit(sprite_pane::TitleChanged(None)));
        workspace.read_with(cx, |workspace, _| {
            assert_eq!(workspace.wanted_title.as_ref(), "Sprite")
        });
        workspace.update_in(cx, |workspace, window, cx| workspace.open_tab(window, cx));
        workspace.update(cx, |workspace, cx| {
            workspace.close_exited_pane(crate::tabs::TabId(0), PaneId(0), cx)
        });
        let before = super::DISPLAY_STRINGS.with(|count| count.get());
        pane.update(cx, |_, cx| {
            cx.emit(sprite_pane::TitleChanged(Some("stale".into())))
        });
        assert_eq!(super::DISPLAY_STRINGS.with(|count| count.get()), before);
        workspace.read_with(cx, |workspace, _| {
            assert_eq!(workspace.pane_titles.len(), 1)
        });
        let weak = pane.downgrade();
        drop(pane);
        cx.run_until_parked();
        assert!(
            weak.upgrade().is_none(),
            "title subscriptions and cached geometry must release closed panes"
        );
    }
    /// The whole of what a name needs: append, delete, keep, abandon.
    #[test]
    fn typing_edits_the_name_and_enter_keeps_it() {
        assert_eq!(
            rename_step("bui", &plain("l", Some("l"))),
            RenameStep::Editing("buil".to_owned())
        );
        assert_eq!(
            rename_step("build", &plain("backspace", None)),
            RenameStep::Editing("buil".to_owned())
        );
        assert_eq!(
            rename_step("", &plain("backspace", None)),
            RenameStep::Editing(String::new())
        );
        assert_eq!(
            rename_step("build", &plain("enter", None)),
            RenameStep::Commit("build".to_owned())
        );
        assert_eq!(
            rename_step("build", &plain("escape", None)),
            RenameStep::Cancel
        );
    }
    /// A key with no character — an arrow, a function key, a bare modifier — is
    /// not a letter and changes nothing.
    #[test]
    fn a_key_without_a_character_leaves_the_name_alone() {
        assert_eq!(
            rename_step("build", &plain("left", None)),
            RenameStep::Editing("build".to_owned())
        );
    }
    /// Committing nothing removes the custom name rather than storing "".
    #[test]
    fn committing_an_empty_name_is_a_commit_of_nothing() {
        assert_eq!(
            rename_step("", &plain("enter", None)),
            RenameStep::Commit(String::new())
        );
    }
    /// Name, then Pane Title, then index. A name a person typed beats what the
    /// program says; what the program says beats a number.
    #[test]
    fn a_tab_label_prefers_the_name_then_the_title_then_the_index() {
        assert_eq!(tab_label(Some("build"), Some("vim"), 0), "build");
        assert_eq!(tab_label(None, Some("vim"), 0), "vim");
        assert_eq!(tab_label(None, None, 0), "1");
        assert_eq!(tab_label(None, None, 4), "5");
    }
    #[test]
    fn the_window_title_is_the_pane_title_or_sprite() {
        assert_eq!(window_title(Some("vim README.md")), "vim README.md");
        assert_eq!(window_title(None), "Sprite");
    }
}
