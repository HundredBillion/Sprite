//! The window's one clock.
//!
//! A cursor blinks, and a silent program's name is discovered, on the same
//! beat for every pane in the window. One timer for the window rather than one
//! per pane: each pane decides what a beat means for it, and only the pane
//! with Pane Focus repaints.

use super::*;

/// Half a blink. The rate every terminal has used since the VT100.
pub(super) const BLINK_INTERVAL: std::time::Duration = std::time::Duration::from_millis(530);

/// How often resources are sampled while the pane count stays the same.
const RESOURCE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60);

impl Workspace {
    /// Starts the clock. It stops by itself once the window is gone.
    pub(super) fn spawn_clock(cx: &mut Context<Self>) -> gpui::Task<()> {
        cx.spawn(async move |workspace, cx| {
            loop {
                cx.background_executor().timer(BLINK_INTERVAL).await;
                if workspace
                    .update(cx, |workspace, cx| workspace.tick_panes(cx))
                    .is_err()
                {
                    return;
                }
            }
        })
    }

    /// Every pane in every tab, background tabs included: a tab shows its
    /// pane's title whether or not it is the tab in front.
    fn tick_panes(&mut self, cx: &mut Context<Self>) {
        self.expire_control(cx);
        self.sample_resources(cx);
        let panes: Vec<_> = self
            .tabs
            .all_panes()
            .into_iter()
            .map(|(_, _, pane)| Rc::clone(pane))
            .collect();
        for pane in panes {
            pane.tick(cx);
        }
    }

    /// A sample when the pane count changes, so an opened or closed pane can
    /// be read against memory, and otherwise once a minute.
    fn sample_resources(&mut self, cx: &mut Context<Self>) {
        if !crate::diagnostics::perf_enabled() {
            return;
        }
        let now = cx.background_executor().now();
        let panes = self.tabs.all_panes().len();
        let reason = match self.resources_sampled {
            Some((at, sampled)) if sampled == panes => {
                if now.saturating_duration_since(at) < RESOURCE_INTERVAL {
                    return;
                }
                "interval"
            }
            _ => "panes",
        };
        self.resources_sampled = Some((now, panes));
        crate::diagnostics::log_resources(reason, panes);
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::*;
    use super::*;

    struct TickingPane {
        focus: FocusHandle,
        ticks: Rc<std::cell::Cell<usize>>,
    }

    impl Focusable for TickingPane {
        fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
            self.focus.clone()
        }
    }

    impl Render for TickingPane {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().track_focus(&self.focus)
        }
    }

    impl gpui::EventEmitter<sprite_pane::TitleChanged> for TickingPane {}

    impl sprite_pane::Pane for TickingPane {
        type Request = SurfaceRequest;
        fn title(&self) -> Option<SharedString> {
            None
        }
        fn set_allocated(&mut self, _: Size<Pixels>) {}
        fn begin_shutdown(&mut self) -> Option<Box<dyn FnOnce() + Send>> {
            None
        }
        fn close_warning(&self) -> Option<sprite_pane::CloseWarning> {
            None
        }
        fn tick(&mut self, _: &mut Context<Self>) {
            self.ticks.set(self.ticks.get() + 1);
        }
    }

    /// One timer for the window, and every pane hears each beat once —
    /// including a pane in a background tab, whose tab label still needs its
    /// title.
    #[gpui::test]
    fn one_window_clock_beats_every_pane_once_per_interval(cx: &mut gpui::TestAppContext) {
        let (workspace, cx) = test_workspace(cx);
        let ticks: Vec<Rc<std::cell::Cell<usize>>> = (0..3).map(|_| Rc::default()).collect();
        workspace.update(cx, |workspace, cx| {
            let mut next = ticks.iter();
            let mut make = |_, _| {
                let ticks = Rc::clone(next.next().unwrap());
                Rc::new(cx.new(|cx| TickingPane {
                    focus: cx.focus_handle(),
                    ticks,
                })) as Rc<dyn PaneHandle<Request = SurfaceRequest>>
            };
            workspace.tabs = Tabs::new(&mut make);
            workspace.tabs.split(Orientation::Horizontal, &mut make);
            workspace.tabs.open(&mut make);
            workspace.refresh_layout(cx);
        });
        cx.run_until_parked();
        let counts = || ticks.iter().map(|count| count.get()).collect::<Vec<_>>();
        assert_eq!(counts(), [0, 0, 0]);
        cx.executor().advance_clock(BLINK_INTERVAL);
        cx.run_until_parked();
        assert_eq!(counts(), [1, 1, 1]);
        cx.executor().advance_clock(BLINK_INTERVAL);
        cx.run_until_parked();
        assert_eq!(counts(), [2, 2, 2]);
    }
}
