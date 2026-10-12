//! The GPUI view for one Pane.
//!
//! It owns one Terminal Session, holds the newest bundle it has been given, and
//! draws that. Everything it knows about the terminal arrives through the
//! public `sprite-term` interface; it never reaches past that seam.

mod geometry;
mod input;
mod list_view;
mod pane_focus;
mod placeholder;
mod render;
mod surfaces;
mod theme;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use gpui::{
    ClipboardItem, Context, FocusHandle, Focusable, Pixels, SharedString, Size, Task, Window,
    point, px,
};
use sprite_term::{
    Admitted, ErrorKind, Rgb, SessionConfig, SessionError, ShutdownHandle, SnapshotBundle,
    TerminalCommand, TerminalSession, TerminalSize,
};

use crate::grid::ScrollAccumulator;
use crate::surface::host::SurfaceHost;
use crate::tokens::{DEFAULT_BACKGROUND as BACKGROUND, DEFAULT_FOREGROUND as FOREGROUND};

use geometry::physical;
use input::Drag;
pub(crate) use input::NativeTextCommitted;
use surfaces::DockDrag;
use surfaces::HostedSurface;
pub(crate) use theme::CellMetrics;
use theme::{chosen_family, unpack};

/// Shown when the worker's control lane is full and a command was refused.
pub(crate) const BUSY_STATUS: &str = "the terminal is busy; the last command was not delivered";
/// Shown when a command is offered to a worker that has gone.
pub(crate) const ENDED_STATUS: &str = "the terminal has ended";

/// What the worker could not take yet. Settings keep the whole reload,
/// because its groups are admitted separately and a partial reload is
/// reverted as one.
#[derive(Default)]
struct Pending {
    settings: Option<crate::config::Settings>,
    resize: Option<sprite_term::ValidTerminalSize>,
}

impl Pending {
    fn clear(&mut self) {
        *self = Self::default();
    }

    fn is_empty(&self) -> bool {
        self.settings.is_none() && self.resize.is_none()
    }
}

enum SessionState {
    NeverStarted,
    Running(TerminalSession),
    Ended(TerminalSession),
}

pub struct TerminalView {
    applied_settings: crate::config::Settings,
    /// What the worker could not take yet, kept so the latest value is
    /// delivered when room returns.
    pending: Pending,
    admission_closed: bool,
    admission_notice: bool,
    /// An ended pane keeps its worker handle until cleanup can join it.
    ///
    /// A pane whose configured program could not be run still has to draw the
    /// reason it could not, and nothing it draws needs a terminal behind it.
    session: SessionState,
    bundle: Option<Arc<SnapshotBundle>>,
    /// Textures for the images this pane is showing.
    ///
    /// Dropped with the view, so closing a pane releases its textures and
    /// closing a tab releases every pane's, without a separate teardown path to
    /// forget to call.
    textures: crate::graphics_cache::GraphicsCache,
    focus: FocusHandle,
    metrics: CellMetrics,
    /// The configured gap around the grid, in logical pixels.
    padding: f32,
    /// Foreground and background to use before the first snapshot arrives.
    ///
    /// Configured colours are held here as well as sent to the terminal, so a
    /// pane that opens on a light background does not spend its first frame
    /// dark.
    fallback_colors: (Rgb, Rgb),
    /// The last size successfully sent, so an unchanged layout sends nothing.
    size: Option<sprite_term::ValidTerminalSize>,
    #[cfg(test)]
    spawn_size: Option<sprite_term::ValidTerminalSize>,
    /// How this pane is reached by observation. Every pane a window opens
    /// has one; it is `None` for a pane whose session never started, once
    /// the pane has begun shutting down, and in tests that build a view
    /// without a window's registry.
    observation: Option<crate::observation::panes::PaneLink>,
    /// What programs have asked this pane to draw beside or over its grid.
    surfaces: SurfaceHost<HostedSurface>,
    surface_window: gpui::AnyWindowHandle,
    dock_drag: Option<DockDrag>,
    /// The pixels this pane has been given.
    ///
    /// A pane is not the window: once a tab holds several, sizing the grid from
    /// the viewport would give every pane the whole window's dimensions and
    /// tell every child the wrong size.
    allocated: Option<Size<Pixels>>,
    status: Option<SharedString>,
    texture_warning: Option<SharedString>,
    /// The title the child set through OSC, if it set one.
    ///
    /// `None` means unknown, never a guess: the engine's own rule, kept here.
    title: Option<SharedString>,
    display_title: Option<SharedString>,
    /// Sub-row scroll remainder, so trackpad gestures are not rounded away.
    scroll: ScrollAccumulator,
    /// The selection gesture in progress, if the pointer is down.
    drag: Option<Drag>,
    plain_link_click: input::PlainLinkClick,
    /// The most recent click awaiting terminal link resolution.
    pending_link_click: Option<u64>,
    hovered_cell: Option<sprite_term::CellPosition>,
    layout_cache: crate::grid::LayoutCache,
    /// Shaped glyphs kept between frames, beside the layout they belong to.
    shape_cache: std::rc::Rc<std::cell::RefCell<crate::grid_paint::ShapeCache>>,
    hovered_link: Option<(u64, sprite_term::HyperlinkSpan)>,
    hover_request: Option<(u64, sprite_term::CellPosition)>,
    /// The cell the current hover answer was asked about, and that cell's row
    /// as it was then. Rows are shared between snapshots while their content
    /// is unchanged, so the same allocation means the answer still holds.
    hover_basis: Option<(sprite_term::CellPosition, Arc<sprite_term::RenderRow>)>,
    next_link_request: u64,
    /// Where the grid's top-left corner sits inside the pane.
    ///
    /// Not the pane's own corner: the padding and the leftover from rounding
    /// the pane down to whole cells sit between the two.
    origin: gpui::Point<Pixels>,
    /// The same corner in window coordinates, learned during paint.
    ///
    /// Mouse positions arrive in window coordinates, and a pane is not
    /// necessarily at the window's origin — it may sit under a tab strip or
    /// beside a sibling. Only the laid-out element knows where it ended up, so
    /// hit testing uses what paint reported rather than a position computed
    /// twice and liable to disagree.
    content_origin: Option<gpui::Point<Pixels>>,
    /// A paste withheld as unsafe, awaiting a second explicit request for the
    /// same text.
    unsafe_paste: crate::confirmation::Confirmation<String>,
    /// Whether the cursor is in the visible half of its blink.
    ///
    /// Always true for a cursor that does not blink, so the phase costs a
    /// non-blinking pane nothing.
    blink_on: bool,
    /// Whether this pane has Pane Focus: it holds the keyboard in its window,
    /// and that window is the active one. The worker is told every change,
    /// because the same fact decides whether the child may write the clipboard
    /// and whether it hears focus reports.
    pane_focus: pane_focus::PaneFocus,
    /// Keeps the focus and window-activation observers alive.
    _focus_observers: [gpui::Subscription; 3],
    /// Text an input method is composing.
    ///
    /// Shown at the cursor and deliberately *not* sent: the terminal learns
    /// nothing about a composition until the person commits it.
    preedit: Option<String>,
    _events: Task<()>,
    _snapshots: Task<()>,
    _retry: Task<()>,
    retry_wake: async_channel::Sender<()>,
    /// Keeps the settings subscription alive for as long as the view is.
    _settings: gpui::Subscription,
}

/// Where an ordinary child exit is reported, with its owning tab and pane.
pub(crate) struct PaneExit {
    pub sender: async_channel::Sender<(crate::tabs::TabId, crate::pane_tree::PaneId)>,
    pub identity: (crate::tabs::TabId, crate::pane_tree::PaneId),
}

#[cfg(test)]
thread_local! {
    pub(crate) static TITLE_STRINGS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(crate) static TITLE_QUERIES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(crate) static FOREGROUND_QUERIES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(crate) static HOVER_LINK_REQUESTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

impl TerminalView {
    fn request_hover_link(&mut self, position: sprite_term::CellPosition) {
        self.hovered_link = None;
        if !matches!(self.session, SessionState::Running(_)) {
            return;
        }
        if self.hover_request.is_none() {
            let request_id = self.next_link_request;
            self.next_link_request = self.next_link_request.wrapping_add(1);
            self.hover_request = Some((request_id, position));
            self.hover_basis = self
                .bundle
                .as_ref()
                .and_then(|bundle| bundle.render.rows.get(usize::from(position.row)))
                .map(|row| (position, Arc::clone(row)));
            #[cfg(test)]
            HOVER_LINK_REQUESTS.with(|count| count.set(count.get() + 1));
            if !self.submit(TerminalCommand::ResolveHyperlink {
                position,
                request_id,
            }) {
                self.hover_request = None;
                self.hover_basis = None;
            }
        }
    }

    /// Whether the newest snapshot still shows `position` exactly as it was
    /// when its link was last asked about.
    fn hover_basis_holds(&self, position: sprite_term::CellPosition) -> bool {
        let Some((asked, row)) = &self.hover_basis else {
            return false;
        };
        *asked == position
            && self
                .bundle
                .as_ref()
                .and_then(|bundle| bundle.render.rows.get(usize::from(position.row)))
                .is_some_and(|current| Arc::ptr_eq(row, current))
    }

    /// Carries the hover across a new snapshot.
    ///
    /// Output elsewhere on screen does not change what is under the pointer,
    /// so the answer already held is kept, restamped with the new generation
    /// (which is what the painter checks), rather than asked for again. Only a
    /// change to the hovered row itself asks again.
    fn follow_hover(&mut self) {
        let Some(cell) = self.hovered_cell else {
            return;
        };
        // An answer still in flight re-checks the row when it arrives.
        if self.hover_request.is_some() {
            return;
        }
        if !self.hover_basis_holds(cell) {
            self.request_hover_link(cell);
            return;
        }
        if let (Some(bundle), Some((_, span))) = (self.bundle.as_ref(), self.hovered_link) {
            self.hovered_link = Some((bundle.generation, span));
        }
    }

    fn request_link_click(&mut self, position: sprite_term::CellPosition) {
        let request_id = self.next_link_request;
        self.next_link_request = self.next_link_request.wrapping_add(1);
        self.pending_link_click = None;
        if !matches!(self.session, SessionState::Running(_)) {
            return;
        }
        self.pending_link_click = Some(request_id);
        if !self.submit(TerminalCommand::ResolveHyperlink {
            position,
            request_id,
        }) {
            self.pending_link_click = None;
        }
    }

    /// `environment` carries this pane's observation variables: the window's
    /// socket and key, and the pane's own identity. It is the only route by
    /// which a child learns the key.
    #[cfg(test)]
    pub fn new(
        command: Option<Vec<std::ffi::OsString>>,
        settings: crate::config::Settings,
        environment: Vec<(std::ffi::OsString, std::ffi::OsString)>,
        observation: Option<crate::observation::panes::PaneLink>,
        exit: PaneExit,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::new_allocated(
            command,
            settings,
            environment,
            observation,
            exit,
            None,
            window,
            cx,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new_allocated(
        command: Option<Vec<std::ffi::OsString>>,
        settings: crate::config::Settings,
        environment: Vec<(std::ffi::OsString, std::ffi::OsString)>,
        observation: Option<crate::observation::panes::PaneLink>,
        exit: PaneExit,
        allocated: Option<Size<Pixels>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        #[cfg(any(test, feature = "startup-benchmark"))]
        crate::startup_benchmark::constructor_started(cx.entity_id(), cx);
        let applied_settings = settings.clone();
        let defaults = theme::session_defaults(&settings);
        let crate::config::Settings {
            font,
            graphics,
            shell,
            scrollback,
            grid,
            ..
        } = settings;

        // The cell is shaped before the session starts, so the child never
        // observes scale-1 metrics for a moment on a HiDPI display.
        let (font_family, mut complaints) = chosen_family(window, font.family.as_deref(), cx);
        let metrics = CellMetrics::measure(
            window,
            font_family.clone(),
            font.size.get(),
            font.line_height.get(),
        );
        let scale_factor = window.scale_factor();

        // A window told what to run gives every one of its panes the same
        // program; otherwise a pane is a login shell, as before.
        let mut config = match command {
            Some(command) => {
                let (program, arguments) = command.split_first().expect("a program to run");
                SessionConfig::terminal_command(program, arguments.to_vec())
            }
            // A preference that cannot be honoured falls back and says so
            // rather than leaving a pane that will not open.
            None => match SessionConfig::shell(&shell.session_preference()) {
                Ok((config, refused)) => {
                    complaints.extend(refused);
                    config
                }
                Err(error) => return Self::failed(error.to_string(), font_family, window, cx),
            },
        };
        // Keep a valid fallback when layout bounds are not available yet.
        config.size = match sprite_term::ValidTerminalSize::new(
            TerminalSize {
                cell_width_px: physical(metrics.width(), scale_factor),
                cell_height_px: physical(metrics.height(), scale_factor),
                ..config.size.dimensions()
            },
            "resize",
        ) {
            Ok(size) => size,
            Err(error) => return Self::failed(error.to_string(), font_family, window, cx),
        };
        if let Some(size) = allocated.and_then(|area| {
            geometry::grid_size(
                crate::grid::content_area(area, grid.padding.get()),
                metrics.width(),
                metrics.height(),
                scale_factor,
            )
        }) {
            config.size = size;
        }
        // The terminal's own limit: how much decoded image it will hold.
        config.graphics = sprite_term::GraphicsPolicy {
            enabled: graphics.enabled,
            storage_bytes: graphics.storage_bytes.get(),
            ..sprite_term::GraphicsPolicy::default()
        };
        let fallback_colors = defaults.fallback_colors;
        config.colors = defaults.colors;
        config.cursor = defaults.cursor;
        config.scrollback_bytes = scrollback.bytes.get();
        config.environment.extend(environment);
        let initial_size = config.size;

        let sprite_term::Spawned {
            session,
            mut events,
            mut snapshots,
        } = match TerminalSession::spawn(config) {
            Ok(session) => session,
            Err(error) => return Self::failed(error.to_string(), font_family, window, cx),
        };

        // Registered before the event task starts, so an answer can never
        // arrive for a pane the registry does not yet know about.
        if let Some(link) = &observation {
            link.panes.register(link.pane, link.tab, session.commands());
        }

        let event_task = cx.spawn(async move |view, cx| {
            loop {
                let decision = crate::terminal_events::decide(events.next().await);
                if decision.stop {
                    let _ = view.update(cx, |view, cx| {
                        view.session = match std::mem::replace(
                            &mut view.session,
                            SessionState::NeverStarted,
                        ) {
                            SessionState::Running(session) | SessionState::Ended(session) => {
                                SessionState::Ended(session)
                            }
                            SessionState::NeverStarted => SessionState::NeverStarted,
                        };
                        // An ended session answers nothing more, so a capture
                        // still waiting on it fails now, with the reason,
                        // rather than at the observation deadline.
                        if let Some(link) = &view.observation {
                            link.panes.fail_all(
                                link.pane,
                                "the pane's session ended before it answered".to_owned(),
                            );
                        }
                        let _ = view.retry_wake.force_send(());
                        view.refresh_display_title(cx);
                    });
                }
                if decision.close_pane {
                    let _ = exit.sender.try_send(exit.identity);
                    return;
                }
                if !decision.effects.is_empty() {
                    let applied = view.update(cx, |view, cx| {
                        let mut repaint = false;
                        for effect in decision.effects {
                            repaint |= view.apply(effect, cx);
                        }
                        // One notify for the batch, and none for a batch that
                        // changed nothing drawn: a hover answer agreeing with
                        // the last one repaints nothing.
                        if repaint {
                            cx.notify();
                        }
                    });
                    if applied.is_err() {
                        return;
                    }
                }
                if decision.stop {
                    return;
                }
            }
        });

        let snapshot_task = cx.spawn(async move |view, cx| {
            while let Ok(bundle) = snapshots.next().await {
                let generation = bundle.generation;
                if view
                    .update(cx, |view, cx| {
                        // Snapshots are latest-only, but delivery across two
                        // independent streams is not ordered against anything
                        // else, so an older generation is simply ignored.
                        let newer = view
                            .bundle
                            .as_ref()
                            .is_none_or(|current| generation > current.generation);
                        if newer {
                            view.refresh_textures(&bundle);
                            view.bundle = Some(bundle);
                            view.follow_hover();
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    return;
                }
            }
        });

        // A reload publishes a new `ActiveSettings`; this is how it reaches a
        // pane. Registered here so a pane created after a reload observes the
        // next one too, having been constructed from the current one.
        let settings_subscription =
            cx.observe_global_in::<crate::config::ActiveSettings>(window, |view, window, cx| {
                let settings = cx.global::<crate::config::ActiveSettings>().0.clone();
                view.apply_settings(&settings, window, cx);
            });

        let (retry_task, retry_wake) = Self::spawn_retry(window, cx);
        let focus = cx.focus_handle();
        let focus_observers = Self::observe_pane_focus(&focus, window, cx);
        let mut view = Self {
            applied_settings,
            pending: Pending::default(),
            admission_closed: false,
            admission_notice: false,
            session: SessionState::Running(session),
            observation,
            surfaces: SurfaceHost::default(),
            surface_window: window.window_handle(),
            dock_drag: None,
            metrics,
            // A setting that did nothing is shown rather than silently
            // ignored: somebody whose file had no effect deserves to know why.
            status: (!complaints.is_empty()).then(|| complaints.join(" · ").into()),
            texture_warning: None,
            bundle: None,
            // The renderer's own limit, separate from the terminal's above.
            textures: crate::graphics_cache::GraphicsCache::with_budget(
                graphics.texture_bytes.get(),
            ),
            focus,
            fallback_colors,
            size: Some(initial_size),
            #[cfg(test)]
            spawn_size: Some(initial_size),
            allocated,
            title: None,
            display_title: None,
            scroll: ScrollAccumulator::default(),
            drag: None,
            plain_link_click: input::PlainLinkClick::default(),
            pending_link_click: None,
            hovered_cell: None,
            hovered_link: None,
            layout_cache: Default::default(),
            shape_cache: Default::default(),
            hover_request: None,
            hover_basis: None,
            next_link_request: 1,
            origin: point(px(grid.padding.get()), px(grid.padding.get())),
            padding: grid.padding.get(),
            content_origin: None,
            unsafe_paste: Default::default(),
            preedit: None,
            blink_on: true,
            pane_focus: pane_focus::PaneFocus::default(),
            _focus_observers: focus_observers,
            _events: event_task,
            _snapshots: snapshot_task,
            _retry: retry_task,
            retry_wake,
            _settings: settings_subscription,
        };
        // The worker starts out denying focus. Saying so explicitly means the
        // two sides agree from the first byte, and every later message is a
        // change the worker hears exactly once.
        view.send(TerminalCommand::Focus(false));
        view
    }

    /// A view that shows why it could not start.
    ///
    /// It owns no session at all, so there is nothing to pump and nothing to
    /// shut down: its event and snapshot tasks are already finished. Spawning
    /// a throwaway shell just to fill the field would fork a process on the
    /// one path where the person's own program has already failed to start.
    fn failed(
        message: String,
        font_family: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        // A failed pane still re-shapes its message when the font changes; a
        // view that ignored reloads would be the one exception to the rule
        // the global relies on.
        // A reload publishes a new `ActiveSettings`; this is how it reaches a
        // pane. Registered here so a pane created after a reload observes the
        // next one too, having been constructed from the current one.
        let settings_subscription =
            cx.observe_global_in::<crate::config::ActiveSettings>(window, |view, window, cx| {
                let settings = cx.global::<crate::config::ActiveSettings>().0.clone();
                view.apply_settings(&settings, window, cx);
            });
        let focus = cx.focus_handle();
        let focus_observers = Self::observe_pane_focus(&focus, window, cx);
        Self {
            applied_settings: crate::config::Settings::default(),
            pending: Pending::default(),
            admission_closed: false,
            admission_notice: false,
            session: SessionState::NeverStarted,
            // A view that never started a session has nothing to observe.
            observation: None,
            surfaces: SurfaceHost::default(),
            surface_window: window.window_handle(),
            dock_drag: None,
            metrics: CellMetrics::measure(
                window,
                font_family,
                crate::config::Font::DEFAULT_SIZE,
                crate::config::Font::DEFAULT_LINE_HEIGHT,
            ),
            bundle: None,
            textures: crate::graphics_cache::GraphicsCache::default(),
            focus,
            fallback_colors: (unpack(FOREGROUND), unpack(BACKGROUND)),
            size: None,
            #[cfg(test)]
            spawn_size: None,
            allocated: None,
            title: None,
            display_title: None,
            status: Some(message.into()),
            texture_warning: None,
            scroll: ScrollAccumulator::default(),
            drag: None,
            plain_link_click: input::PlainLinkClick::default(),
            pending_link_click: None,
            hovered_cell: None,
            hovered_link: None,
            layout_cache: Default::default(),
            shape_cache: Default::default(),
            hover_request: None,
            hover_basis: None,
            next_link_request: 1,
            origin: point(
                px(crate::config::Grid::DEFAULT_PADDING),
                px(crate::config::Grid::DEFAULT_PADDING),
            ),
            padding: crate::config::Grid::DEFAULT_PADDING,
            content_origin: None,
            unsafe_paste: Default::default(),
            preedit: None,
            blink_on: true,
            pane_focus: pane_focus::PaneFocus::default(),
            _focus_observers: focus_observers,
            _events: Task::ready(()),
            _snapshots: Task::ready(()),
            _retry: Task::ready(()),
            retry_wake: async_channel::bounded(1).0,
            _settings: settings_subscription,
        }
    }

    /// Performs one decided effect, returning whether it changed anything the
    /// pane draws. Everything here needs `cx`; nothing here decides anything.
    fn apply(&mut self, effect: crate::terminal_events::Effect, cx: &mut Context<Self>) -> bool {
        use crate::terminal_events::Effect;
        match effect {
            Effect::Status(line) => {
                self.status = Some(line);
                true
            }
            Effect::Title(title) => {
                self.title = title.map(SharedString::from);
                self.refresh_display_title(cx);
                true
            }
            Effect::HoldPaste(text) => {
                self.unsafe_paste.arm(text);
                true
            }
            Effect::HyperlinkResolved {
                position,
                request_id,
                generation,
                uri,
                span,
            } => {
                if self.pending_link_click == Some(request_id) {
                    self.pending_link_click = None;
                    if let Some(uri) = uri {
                        cx.open_url(&uri);
                    }
                }
                if self.hover_request != Some((request_id, position)) {
                    return false;
                }
                self.hover_request = None;
                let before = self.hovered_link.map(|(_, span)| span);
                if self.hovered_cell == Some(position) {
                    // The answer describes the row as it was asked about. While
                    // that row is unchanged it still holds for the newest
                    // snapshot, whatever generation the worker stamped it with.
                    match self.bundle.as_ref().map(|bundle| bundle.generation) {
                        Some(current)
                            if current == generation || self.hover_basis_holds(position) =>
                        {
                            self.hovered_link = span.map(|span| (current, span));
                        }
                        _ => self.request_hover_link(position),
                    }
                }
                if let Some(cell) = self.hovered_cell.filter(|cell| *cell != position) {
                    self.request_hover_link(cell);
                }
                self.hovered_link.map(|(_, span)| span) != before
            }
            // The worker accepted this while the pane had Pane Focus, but the
            // event may have waited in the channel while focus moved. Only
            // this thread knows whether the pane holds focus as it writes.
            Effect::ChildClipboard(_) if !self.pane_focus.held() => false,
            Effect::ChildClipboard(text) | Effect::Clipboard(text) => {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
                true
            }
            // Answers go to the observer that asked; nothing on screen
            // changes, so neither asks for a frame.
            Effect::DeliverHistory { ticket, snapshot } => {
                if let Some(link) = &self.observation {
                    link.panes.answer(link.pane, ticket, Ok(snapshot));
                }
                false
            }
            // A capture that failed answers the one request it belongs to,
            // with the reason, rather than leaving it to wait out the deadline.
            Effect::FailRequest { ticket, reason } => {
                if let Some(link) = &self.observation {
                    link.panes.answer(link.pane, ticket, Err(reason));
                }
                false
            }
        }
    }

    /// Watches both halves of Pane Focus.
    ///
    /// GPUI's focus events already treat an inactive window as holding no
    /// focus, so focus-in and focus-out cover a switch between windows as well
    /// as between panes. Activation is watched too, because focus events wait
    /// for the next frame, and a pane whose window has gone to the background
    /// should stop taking the clipboard now rather than then.
    fn observe_pane_focus(
        focus: &FocusHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> [gpui::Subscription; 3] {
        [
            cx.on_focus_in(focus, window, |view, window, cx| {
                view.refresh_pane_focus(window, cx)
            }),
            cx.on_focus_out(focus, window, |view, _, window, cx| {
                view.refresh_pane_focus(window, cx)
            }),
            cx.observe_window_activation(window, |view, window, cx| {
                view.refresh_pane_focus(window, cx)
            }),
        ]
    }

    /// Recomputes Pane Focus from the window, and reports a change.
    fn refresh_pane_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let held = pane_focus::PaneFocus::compute(window, &self.focus, cx);
        let Some(focused) = self.pane_focus.update(held) else {
            return;
        };
        self.admit_focus();
        // A pane gaining focus starts its blink from visible, and one losing
        // it shows a steady cursor from the next frame on.
        self.blink_on = true;
        if !focused {
            // Leaving the pane is a decision too: a paste held here is not
            // answered by a paste made after coming back.
            self.drop_unsafe_paste(cx);
        }
        cx.notify();
    }

    /// Tells the worker the latest Pane Focus, or keeps it for the retry task
    /// when the lane is full. Losing it there would leave the child allowed
    /// the clipboard, or deaf to focus reports, until the next change.
    fn admit_focus(&mut self) {
        let Some(focused) = self.pane_focus.owed() else {
            return;
        };
        match self.admit(TerminalCommand::Focus(focused)) {
            Ok(Admitted::Accepted) => self.pane_focus.delivered(focused),
            Ok(Admitted::Saturated(_)) => self.pane_focus.refused(focused),
            Ok(Admitted::Closed(_)) | Err(_) => self.pane_focus.forget_pending(),
        }
    }

    /// Whether this pane has Pane Focus.
    pub(crate) fn pane_focused(&self) -> bool {
        self.pane_focus.held()
    }

    /// Hands over the worker so the window can wait for it off the GPUI thread.
    pub fn begin_shutdown(&mut self) -> Option<ShutdownHandle> {
        self.admission_closed = true;
        self.pending.clear();
        self.pane_focus.forget_pending();
        let _ = self.retry_wake.force_send(());
        // Retained view handles must not keep a closed pane reachable by commands.
        if let Some(link) = self.observation.take() {
            link.panes.forget(link.pane);
        }
        // A view with no session has no worker to wait for, so there is
        // nothing to hand over.
        match &mut self.session {
            SessionState::NeverStarted => None,
            SessionState::Running(session) | SessionState::Ended(session) => {
                session.begin_shutdown().ok().flatten()
            }
        }
    }

    fn send(&mut self, command: TerminalCommand) {
        let _accepted = self.submit(command);
    }

    fn submit(&mut self, command: TerminalCommand) -> bool {
        matches!(self.admit(command), Ok(Admitted::Accepted))
    }

    /// Offers a command to the worker. A saturated or closed lane hands the
    /// command back so the caller can keep what is worth keeping; an `Err`
    /// is the command itself being refused.
    fn admit(
        &mut self,
        command: TerminalCommand,
    ) -> Result<Admitted<TerminalCommand>, SessionError> {
        let SessionState::Running(session) = &mut self.session else {
            return Ok(Admitted::Accepted);
        };
        let outcome = session.try_admit(command);
        match &outcome {
            Ok(Admitted::Accepted) => {}
            Ok(Admitted::Saturated(_)) => self.refuse(BUSY_STATUS),
            Ok(Admitted::Closed(_)) => {
                self.admission_closed = true;
                self.refuse(ENDED_STATUS);
            }
            Err(error) => {
                self.admission_closed |= error.kind == ErrorKind::Ended;
                self.refuse(&error.to_string());
            }
        }
        outcome
    }

    /// Shows why a command was not delivered, and wakes the retry task so the
    /// refusal is painted and anything kept is offered again.
    fn refuse(&mut self, status: &str) {
        self.status = Some(status.to_owned().into());
        self.admission_notice = true;
        self.retry_wake.force_send(()).ok();
    }

    // One coalesced wake starts bounded latest-value recovery and paints refusal without another snapshot.
    fn spawn_retry(
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (Task<()>, async_channel::Sender<()>) {
        let (wake, receiver) = async_channel::bounded(1);
        let task = cx.spawn_in(window, async move |view, cx| {
            while receiver.recv().await.is_ok() {
                loop {
                    cx.background_executor()
                        .timer(std::time::Duration::from_millis(50))
                        .await;
                    let pending = view.update_in(cx, |view, window, cx| {
                        if std::mem::take(&mut view.admission_notice) {
                            cx.notify();
                        }
                        if view.admission_closed
                            || !matches!(view.session, SessionState::Running(_))
                        {
                            view.pending.clear();
                            view.pane_focus.forget_pending();
                            return None;
                        }
                        if let Some(settings) = view.pending.settings.take() {
                            view.apply_settings(&settings, window, cx);
                        }
                        if let Some(size) = view.pending.resize {
                            view.admit_resize(size);
                        }
                        if view.pane_focus.pending().is_some() {
                            view.admit_focus();
                        }
                        Some(!view.pending.is_empty() || view.pane_focus.pending().is_some())
                    });
                    match pending {
                        Ok(Some(true)) => {}
                        Ok(Some(false)) => break,
                        _ => return,
                    }
                }
            }
        });
        (task, wake)
    }

    /// What this pane is running, asked of the kernel rather than of the
    /// worker — see [`sprite_term::ForegroundWatch`].
    pub fn foreground(&self) -> sprite_term::ForegroundState {
        #[cfg(test)]
        FOREGROUND_QUERIES.with(|count| count.set(count.get() + 1));
        // Nothing is running in a pane that never started, so closing it must
        // not ask for confirmation.
        match &self.session {
            SessionState::NeverStarted | SessionState::Ended(_) => {
                sprite_term::ForegroundState::Idle
            }
            SessionState::Running(session) => session.foreground(),
        }
    }

    /// Returns the process group when `pid` owns this pane's foreground.
    pub fn foreground_owner_group(&self, pid: u32) -> Option<i32> {
        match &self.session {
            SessionState::NeverStarted | SessionState::Ended(_) => None,
            SessionState::Running(session) => session.foreground_owner_group(pid),
        }
    }

    /// What this pane is called, as the tab and the window title will show it.
    ///
    /// The child's own title first, because a program that set one meant it.
    /// Then the program in the foreground, which is a name the kernel vouches
    /// for. Then nothing — the workspace falls back to the tab's index, and
    /// this view does not invent a word to save it the trouble.
    pub fn title(&self) -> Option<SharedString> {
        #[cfg(test)]
        TITLE_QUERIES.with(|count| count.set(count.get() + 1));
        self.display_title.clone()
    }

    #[cfg(test)]
    pub(crate) fn allocated_for_test(&self) -> Option<Size<Pixels>> {
        self.allocated
    }

    #[cfg(test)]
    pub(crate) fn startup_geometry_for_test(
        &self,
        scale: f32,
    ) -> (
        Option<sprite_term::ValidTerminalSize>,
        Option<sprite_term::ValidTerminalSize>,
    ) {
        (
            self.spawn_size,
            self.allocated.and_then(|area| {
                geometry::grid_size(
                    crate::grid::content_area(area, self.padding),
                    self.metrics.width(),
                    self.metrics.height(),
                    scale,
                )
            }),
        )
    }

    #[cfg(test)]
    pub(crate) fn bundle_for_test(&self) -> Option<Arc<SnapshotBundle>> {
        self.bundle.clone()
    }

    fn refresh_display_title(&mut self, cx: &mut Context<Self>) {
        let wanted = if let Some(title) = &self.title {
            Some(title.clone())
        } else {
            let foreground = self.foreground();
            let program = foreground.program();
            if self.display_title.as_ref().map(|title| title.as_ref()) == program {
                return;
            }
            program.map(|program| {
                #[cfg(test)]
                TITLE_STRINGS.with(|count| count.set(count.get() + 1));
                SharedString::from(program.to_owned())
            })
        };
        if wanted != self.display_title {
            self.display_title = wanted;
            cx.emit(sprite_pane::TitleChanged(self.display_title.clone()));
        }
    }
}

impl gpui::EventEmitter<sprite_pane::TitleChanged> for TerminalView {}

impl Drop for TerminalView {
    fn drop(&mut self) {
        // A pane that is gone must stop being listed, and anyone waiting on it
        // is released rather than left to time out on something already known
        // to have ended.
        if let Some(link) = &self.observation {
            link.panes.forget(link.pane);
        }
    }
}

impl Focusable for TerminalView {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl sprite_pane::Pane for TerminalView {
    type Request = crate::surface::channel::SurfaceRequest;

    fn surface_request(
        &mut self,
        request: Self::Request,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.serve_surface_request(request, window, cx);
    }

    fn cycle_surface_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cycle_focus(window, cx);
    }

    fn cancel_pending_input(&mut self, cx: &mut Context<Self>) {
        self.drop_unsafe_paste(cx);
    }

    fn tick(&mut self, cx: &mut Context<Self>) {
        self.clock_tick(cx);
    }

    fn title(&self) -> Option<SharedString> {
        TerminalView::title(self)
    }

    fn set_allocated(&mut self, size: Size<Pixels>) {
        TerminalView::set_allocated(self, size);
    }

    fn begin_shutdown(&mut self) -> Option<Box<dyn FnOnce() + Send>> {
        let handle = TerminalView::begin_shutdown(self)?;
        // The interface promises blocking work and nothing about children;
        // what this pane's blocking work happens to be stays in here.
        Some(Box::new(move || {
            let _ = handle.wait();
        }))
    }

    fn close_warning(&self) -> Option<sprite_pane::CloseWarning> {
        let state = self.foreground();
        state.should_confirm().then(|| sprite_pane::CloseWarning {
            program: state
                .program()
                .map(|program| SharedString::from(program.to_owned())),
        })
    }
}

#[cfg(test)]
mod submission_regressions;

#[cfg(test)]
mod cosmetic_tests;
