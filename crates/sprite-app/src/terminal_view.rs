//! The GPUI view for one Pane.
//!
//! It owns one Terminal Session, holds the newest bundle it has been given, and
//! draws that. Everything it knows about the terminal arrives through the
//! public `sprite-term` interface; it never reaches past that seam.

mod geometry;
mod input;
mod render;
mod surfaces;
mod theme;

use std::sync::Arc;

use gpui::{
    ClipboardItem, Context, FocusHandle, Focusable, Pixels, SharedString, Size, Task, Window,
    point, px,
};
use sprite_term::{
    Rgb, SessionConfig, ShutdownHandle, SnapshotBundle, TerminalCommand, TerminalSession,
    TerminalSize,
};

use crate::grid::ScrollAccumulator;
use crate::surface::host::SurfaceHost;
use crate::tokens::{DEFAULT_BACKGROUND as BACKGROUND, DEFAULT_FOREGROUND as FOREGROUND};

use geometry::physical;
use input::Drag;
use render::BLINK_INTERVAL;
use surfaces::HostedSurface;
use theme::{chosen_family, measure_cell_width, unpack};

pub struct TerminalView {
    /// The pane's terminal, or `None` for a view that never started one.
    ///
    /// A pane whose configured program could not be run still has to draw the
    /// reason it could not, and nothing it draws needs a terminal behind it.
    session: Option<TerminalSession>,
    bundle: Option<Arc<SnapshotBundle>>,
    /// Textures for the images this pane is showing.
    ///
    /// Dropped with the view, so closing a pane releases its textures and
    /// closing a tab releases every pane's, without a separate teardown path to
    /// forget to call.
    textures: crate::graphics_cache::GraphicsCache,
    focus: FocusHandle,
    /// The configured text size, which the cell metrics follow.
    font_size: Pixels,
    /// Measured from the font actually rendered, in logical pixels.
    cell_width: Pixels,
    cell_height: Pixels,
    /// The configured line-height ratio, kept so a size change re-derives the
    /// cell height from the same ratio the theme asked for.
    line_height: f32,
    /// The configured gap around the grid, in logical pixels.
    padding: f32,
    /// Resolved once, then used for both measuring and drawing.
    font_family: SharedString,
    /// Foreground and background to use before the first snapshot arrives.
    ///
    /// Configured colours are held here as well as sent to the terminal, so a
    /// pane that opens on a light background does not spend its first frame
    /// dark.
    fallback_colors: (Rgb, Rgb),
    /// The last size successfully sent, so an unchanged layout sends nothing.
    size: Option<TerminalSize>,
    /// How this pane is reached by observation, if the window has an endpoint.
    observation: Option<crate::observation::panes::PaneLink>,
    /// What programs have asked this pane to draw beside or over its grid.
    surfaces: SurfaceHost<HostedSurface>,
    /// The pixels this pane has been given.
    ///
    /// A pane is not the window: once a tab holds several, sizing the grid from
    /// the viewport would give every pane the whole window's dimensions and
    /// tell every child the wrong size.
    allocated: Option<Size<Pixels>>,
    status: Option<SharedString>,
    /// The title the child set through OSC, if it set one.
    ///
    /// `None` means unknown, never a guess: the engine's own rule, kept here.
    title: Option<SharedString>,
    /// Sub-row scroll remainder, so trackpad gestures are not rounded away.
    scroll: ScrollAccumulator,
    /// The selection gesture in progress, if the pointer is down.
    drag: Option<Drag>,
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
    /// A paste withheld as unsafe, awaiting a second explicit request.
    pending_unsafe_paste: Option<String>,
    /// Whether the cursor is in the visible half of its blink.
    ///
    /// Always true for a cursor that does not blink, so the phase costs a
    /// non-blinking pane nothing.
    blink_on: bool,
    /// Text an input method is composing.
    ///
    /// Shown at the cursor and deliberately *not* sent: the terminal learns
    /// nothing about a composition until the person commits it.
    preedit: Option<String>,
    _events: Task<()>,
    _snapshots: Task<()>,
    _blink: Task<()>,
    /// Keeps the settings subscription alive for as long as the view is.
    _settings: gpui::Subscription,
}

impl TerminalView {
    /// `environment` carries this pane's observation variables: the window's
    /// socket and key, and the pane's own identity. It is the only route by
    /// which a child learns the key.
    pub fn new(
        command: Option<Vec<std::ffi::OsString>>,
        settings: crate::config::Settings,
        environment: Vec<(std::ffi::OsString, std::ffi::OsString)>,
        observation: Option<crate::observation::panes::PaneLink>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let crate::config::Settings {
            font,
            graphics,
            colors,
            cursor,
            shell,
            scrollback,
            grid,
            ..
        } = settings;

        // The cell is shaped before the session starts, so the child never
        // observes scale-1 metrics for a moment on a HiDPI display.
        let font_size = px(font.size);
        let (font_family, mut complaints) = chosen_family(window, font.family.as_deref());
        let cell_width = measure_cell_width(window, &font_family, font_size);
        let scale_factor = window.scale_factor();

        // A window told what to run gives every one of its panes the same
        // program; otherwise a pane is a login shell, as before.
        let mut config = match command {
            Some(command) => {
                let (program, arguments) = command.split_first().expect("a program to run");
                SessionConfig::command(program, arguments.to_vec())
            }
            // A preference that cannot be honoured falls back and says so
            // rather than leaving a pane that will not open.
            None => match SessionConfig::shell(&shell) {
                Ok((config, refused)) => {
                    complaints.extend(refused);
                    config
                }
                Err(error) => return Self::failed(error.to_string(), font_family, window, cx),
            },
        };
        // The initial 24x80 grid is kept; only the physical cell metrics are
        // corrected for the display this window opened on.
        config.size = TerminalSize {
            cell_width_px: physical(cell_width, scale_factor),
            cell_height_px: physical(
                px(crate::config::Font::cell_height(
                    font.size,
                    font.line_height,
                )),
                scale_factor,
            ),
            ..config.size
        };
        // The terminal's own limit: how much decoded image it will hold.
        config.graphics = sprite_term::GraphicsPolicy {
            enabled: graphics.enabled,
            storage_bytes: graphics.storage_bytes,
            ..sprite_term::GraphicsPolicy::default()
        };
        // Kept for the frames before the first snapshot, when there is no
        // terminal state to ask.
        let fallback_colors = (
            colors.foreground.unwrap_or_else(|| unpack(FOREGROUND)),
            colors.background.unwrap_or_else(|| unpack(BACKGROUND)),
        );
        // Written into the pane's *default* colours, so a program that sets its
        // own still wins.
        //
        // Foreground and background are always supplied, configured or not:
        // libghostty reports the pair only when it knows both, and a pane that
        // supplied neither would draw its cells in the placeholder black a
        // render state starts with while its window drew Sprite's own colour
        // behind them.
        config.colors = sprite_term::ColorDefaults {
            foreground: Some(fallback_colors.0),
            background: Some(fallback_colors.1),
            cursor: colors.cursor,
            palette: colors.palette,
        };
        config.cursor = sprite_term::CursorDefaults {
            style: cursor.style,
            blink: cursor.blink,
        };
        config.scrollback_bytes = scrollback.bytes;
        config.environment.extend(environment);
        let initial_size = config.size;

        let mut session = match TerminalSession::spawn(config) {
            Ok(session) => session,
            Err(error) => return Self::failed(error.to_string(), font_family, window, cx),
        };

        // Registered before the event task starts, so an answer can never
        // arrive for a pane the registry does not yet know about.
        if let Some(link) = &observation {
            link.panes.register(link.pane, link.tab, session.commands());
        }

        let events = session.take_event_stream();
        let snapshots = session.take_snapshot_stream();

        let event_task = cx.spawn(async move |view, cx| {
            let Ok(mut events) = events else { return };
            loop {
                let decision = crate::terminal_events::decide(events.next().await);
                if !decision.effects.is_empty() {
                    let applied = view.update(cx, |view, cx| {
                        for effect in decision.effects {
                            view.apply(effect, cx);
                        }
                        // One notify for the batch: an event that asked for
                        // nothing does not repaint.
                        cx.notify();
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
            let Ok(mut snapshots) = snapshots else { return };
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

        Self {
            session: Some(session),
            observation,
            surfaces: SurfaceHost::default(),
            font_size,
            // A setting that did nothing is shown rather than silently
            // ignored: somebody whose file had no effect deserves to know why.
            status: (!complaints.is_empty()).then(|| complaints.join(" · ").into()),
            bundle: None,
            // The renderer's own limit, separate from the terminal's above.
            textures: crate::graphics_cache::GraphicsCache::with_budget(graphics.texture_bytes),
            focus: cx.focus_handle(),
            cell_width,
            cell_height: px(crate::config::Font::cell_height(
                font.size,
                font.line_height,
            )),
            line_height: font.line_height,
            font_family,
            fallback_colors,
            size: Some(initial_size),
            allocated: None,
            title: None,
            scroll: ScrollAccumulator::default(),
            drag: None,
            origin: point(px(grid.padding), px(grid.padding)),
            padding: grid.padding,
            content_origin: None,
            pending_unsafe_paste: None,
            preedit: None,
            blink_on: true,
            _events: event_task,
            _snapshots: snapshot_task,
            _blink: Self::spawn_blink(cx),
            _settings: settings_subscription,
        }
    }

    /// One timer per pane, running whether or not anything blinks: it wakes
    /// twice a second, notices a steady cursor, and does nothing. A failed
    /// pane has one too, because a grid Surface hosted in it may blink.
    ///
    /// Starting and stopping it as programs change the cursor would be more
    /// moving parts for less than a millisecond of work.
    fn spawn_blink(cx: &mut Context<Self>) -> Task<()> {
        cx.spawn(async move |view, cx| {
            loop {
                cx.background_executor().timer(BLINK_INTERVAL).await;
                if view.update(cx, |view, cx| view.tick_blink(cx)).is_err() {
                    return;
                }
            }
        })
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
        window: &Window,
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
        Self {
            session: None,
            // A view that never started a session has nothing to observe.
            observation: None,
            surfaces: SurfaceHost::default(),
            font_size: px(crate::config::Font::DEFAULT_SIZE),
            bundle: None,
            textures: crate::graphics_cache::GraphicsCache::default(),
            focus: cx.focus_handle(),
            cell_width: px(8.0),
            cell_height: px(crate::config::Font::cell_height(
                crate::config::Font::DEFAULT_SIZE,
                crate::config::Font::DEFAULT_LINE_HEIGHT,
            )),
            line_height: crate::config::Font::DEFAULT_LINE_HEIGHT,
            font_family,
            fallback_colors: (unpack(FOREGROUND), unpack(BACKGROUND)),
            size: None,
            allocated: None,
            title: None,
            status: Some(message.into()),
            scroll: ScrollAccumulator::default(),
            drag: None,
            origin: point(
                px(crate::config::Grid::DEFAULT_PADDING),
                px(crate::config::Grid::DEFAULT_PADDING),
            ),
            padding: crate::config::Grid::DEFAULT_PADDING,
            content_origin: None,
            pending_unsafe_paste: None,
            preedit: None,
            blink_on: true,
            _events: Task::ready(()),
            _snapshots: Task::ready(()),
            _blink: Self::spawn_blink(cx),
            _settings: settings_subscription,
        }
    }

    /// Performs one decided effect. Everything here needs `cx`; nothing here
    /// decides anything.
    fn apply(&mut self, effect: crate::terminal_events::Effect, cx: &mut Context<Self>) {
        use crate::terminal_events::Effect;
        match effect {
            Effect::Status(line) => self.status = Some(line),
            Effect::Title(title) => self.title = title.map(SharedString::from),
            Effect::HoldPaste(text) => self.pending_unsafe_paste = Some(text),
            Effect::OpenUrl(uri) => cx.open_url(&uri),
            Effect::Clipboard(text) => cx.write_to_clipboard(ClipboardItem::new_string(text)),
            Effect::DeliverHistory(history) => {
                if let Some(link) = &self.observation {
                    link.panes.deliver(link.pane, history);
                }
            }
            // A pane in a bad state must not leave an observation request
            // waiting out the deadline: the pane cannot answer, and this is why.
            Effect::FailRequest(reason) => {
                if let Some(link) = &self.observation {
                    link.panes.deliver_failure(link.pane, reason);
                }
            }
        }
    }

    /// Hands over the worker so the window can wait for it off the GPUI thread.
    pub fn begin_shutdown(&mut self) -> Option<ShutdownHandle> {
        // A view with no session has no worker to wait for, so there is
        // nothing to hand over.
        let session = self.session.as_mut()?;
        session.begin_shutdown().ok().flatten()
    }

    fn send(&mut self, command: TerminalCommand) {
        // Sending to a view with no session is a no-op, not an error: a failed
        // pane has nothing to send to, and reporting a send failure over its
        // status line would replace the reason it failed with a symptom.
        let Some(session) = self.session.as_mut() else {
            return;
        };
        if let Err(error) = session.send(command) {
            self.status = Some(error.to_string().into());
        }
    }

    /// What this pane is running, asked of the kernel rather than of the
    /// worker — see [`sprite_term::ForegroundWatch`].
    pub fn foreground(&self) -> sprite_term::ForegroundState {
        // Nothing is running in a pane that never started, so closing it must
        // not ask for confirmation.
        let Some(session) = self.session.as_ref() else {
            return sprite_term::ForegroundState::Idle;
        };
        session.foreground()
    }

    /// What this pane is called, as the tab and the window title will show it.
    ///
    /// The child's own title first, because a program that set one meant it.
    /// Then the program in the foreground, which is a name the kernel vouches
    /// for. Then nothing — the workspace falls back to the tab's index, and
    /// this view does not invent a word to save it the trouble.
    pub fn title(&self) -> Option<SharedString> {
        if let Some(title) = &self.title {
            return Some(title.clone());
        }
        self.foreground()
            .program()
            .map(|program| SharedString::from(program.to_owned()))
    }
}

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
