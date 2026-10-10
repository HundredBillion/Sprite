use super::*;

impl Workspace {
    /// Turns pane observation on or off while the window is running.
    ///
    /// Turning it **off** destroys the endpoint outright — the socket leaves the
    /// filesystem and the key stops being accepted — rather than leaving a
    /// socket that refuses politely, and stops injecting credentials into
    /// sessions started afterwards. Sessions already running keep running; they
    /// simply hold credentials that no longer open anything.
    ///
    /// Turning it **on** opens a *new* endpoint with a new key and a new socket.
    /// Reviving the old one would mean a key someone captured while observation
    /// was enabled started working again the moment it was re-enabled.
    pub fn set_observation_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.change_observation(enabled, None, cx);
    }
    pub(super) fn change_observation(
        &mut self,
        enabled: bool,
        reply: Option<&crate::local_socket::ReplyConnection>,
        cx: &mut Context<Self>,
    ) {
        if enabled == self.settings.pane_observation.enabled {
            return;
        }
        self.settings.pane_observation.enabled = enabled;
        if enabled {
            self.endpoint = open_endpoint(&self.panes, &self.reload_sender);
        } else if let Some(mut endpoint) = self.endpoint.take() {
            endpoint.close_after_reply(reply);
        }
        self.refresh_layout(cx);
        cx.notify();
    }
    /// Re-reads the configuration file and reports what became of it.
    ///
    /// Three outcomes, kept apart on purpose. A file that will not parse leaves
    /// the running configuration entirely alone and reports the error with the
    /// line it is on — replacing a working setup with defaults because of a
    /// missing bracket would be a worse answer than doing nothing. A file that
    /// parses is applied, and anything inside it that could not be used is
    /// reported field by field while the rest takes effect. And a change that
    /// cannot honestly be applied to a session that is already running is said
    /// to be waiting for the next one, rather than silently dropped.
    pub(super) fn reload(
        &mut self,
        reply: Option<&crate::local_socket::ReplyConnection>,
        cx: &mut Context<Self>,
    ) -> String {
        if self.stopping {
            return "this window is closing".to_owned();
        }
        let Some(path) = self.config_path.clone().or_else(crate::config::path) else {
            return "there is nowhere to read a configuration file from \
                    (neither XDG_CONFIG_HOME nor HOME is set)"
                .to_owned();
        };
        let candidate = match crate::config::Settings::load_candidate(&path) {
            Ok(candidate) => candidate,
            Err(error) => {
                return format!("not reloaded; the running configuration is unchanged\n{error}");
            }
        };
        let (settings, complaints) = candidate;

        // File against file: the zoom lives beside `self.settings`, so a
        // zoomed window is not mistaken for a font change and the zoom
        // outlasts the reload.
        let outcome = self.settings.diff(&settings);
        if outcome.has(crate::config::LiveChange::Colors) {
            cx.global_mut::<crate::tokens::TokenRegistry>()
                .apply_theme(&settings.colors);
        }
        if outcome.has(crate::config::LiveChange::Observation) {
            self.change_observation(settings.pane_observation.enabled, reply, cx);
        }
        self.settings = settings;
        // A zoom that now matches the file's own size is no zoom at all, so
        // the window follows the file from here on.
        if self.font_zoom == Some(self.settings.font.size) {
            self.font_zoom = None;
        }
        // Published, not pushed: each pane observes the global with its own
        // window in hand, which is what a cell re-measure needs and what this
        // method, reached from an endpoint thread, does not have.
        cx.set_global(crate::config::ActiveSettings(self.active_settings()));
        cx.notify();

        outcome.describe(&path, &complaints.0)
    }
}
/// Opens an endpoint that answers from this window's panes.
pub(super) fn open_endpoint(
    panes: &Arc<WindowPanes>,
    reload: &async_channel::Sender<ReloadRequest>,
) -> Option<Endpoint> {
    let panes = Arc::clone(panes);
    let reload = reload.clone();
    Endpoint::open(move |request| {
        crate::observation::request::respond(
            panes.as_ref(),
            &reload,
            &request.body,
            Some(request.reply_connection),
        )
    })
    .ok()
}

/// A reload asked for from an endpoint thread, and where to put the answer.
///
/// The reply travels on a `std::sync::mpsc` channel rather than an async one
/// because the waiting side is a plain thread that needs a *timeout*: a wedged
/// GPUI thread must cost the endpoint a bounded wait, not a thread that never
/// returns. It carries the request's claim, which the window must win before
/// it reloads anything.
pub(crate) struct ReloadRequest {
    pub(crate) what: ConfigVerb,
    pub(crate) reply: Relayed<String>,
    pub(crate) reply_connection: Option<crate::local_socket::ReplyConnection>,
}

/// The channel that reload and print requests cross to the GPUI thread on.
///
/// It has room for a request from every connection the observation endpoint
/// serves at once, so handing a request over never blocks: `relay` starts its
/// bounded wait only after the hand-over, and an endpoint thread waiting for
/// room in front of a wedged GPUI thread would wait, and hold its slot,
/// forever.
pub(crate) fn reload_channel() -> (
    async_channel::Sender<ReloadRequest>,
    async_channel::Receiver<ReloadRequest>,
) {
    async_channel::bounded(crate::observation::endpoint::MAX_CONNECTIONS)
}

/// How much longer an asker waits once the window has claimed its request.
///
/// Bounded, because a wedged GPUI thread must not pin an endpoint thread and
/// its connection slot forever. Generous, because by then the work is under
/// way and the only honest early answer is that it still is.
pub(crate) const AFTER_CLAIM: std::time::Duration = std::time::Duration::from_secs(10);

/// Where one relayed request stands, shared by the thread that asked and the
/// window that answers.
///
/// It starts waiting and leaves that state exactly once: the window claims it
/// in order to apply it, or the asker abandons it in order to report that
/// nothing changed. Both moves are one atomic step out of waiting, so
/// whichever comes second learns that it lost before it acts.
#[derive(Clone, Debug, Default)]
pub(crate) struct Claim(Arc<std::sync::atomic::AtomicU8>);

impl Claim {
    const WAITING: u8 = 0;
    const CLAIMED: u8 = 1;
    const ABANDONED: u8 = 2;

    fn leave_waiting(&self, to: u8) -> Result<u8, u8> {
        self.0.compare_exchange(
            Self::WAITING,
            to,
            std::sync::atomic::Ordering::AcqRel,
            std::sync::atomic::Ordering::Acquire,
        )
    }

    /// Moves a waiting request to claimed. True when the window may apply it.
    fn claim(&self) -> bool {
        matches!(
            self.leave_waiting(Self::CLAIMED),
            Ok(_) | Err(Self::CLAIMED)
        )
    }

    /// Moves a waiting request to abandoned. True when the window can no
    /// longer apply it, so the asker may say that nothing changed.
    pub(crate) fn abandon(&self) -> bool {
        matches!(
            self.leave_waiting(Self::ABANDONED),
            Ok(_) | Err(Self::ABANDONED)
        )
    }
}

/// The answering half of a relayed request: where the answer goes, and the
/// claim that decides whether there may be one.
///
/// Public because Surface requests carry it and are public; only `send` is
/// usable outside this crate.
pub struct Relayed<Answer> {
    claim: Claim,
    reply: std::sync::mpsc::SyncSender<Answer>,
}

impl<Answer> Relayed<Answer> {
    /// A waiting request's reply, and the asker's hold on its claim.
    pub(crate) fn waiting(reply: std::sync::mpsc::SyncSender<Answer>) -> (Self, Claim) {
        let claim = Claim::default();
        (
            Self {
                claim: claim.clone(),
                reply,
            },
            claim,
        )
    }

    /// Takes the request for applying.
    ///
    /// False when the asker has already given up and told its caller that
    /// nothing changed: the request must then be dropped without being
    /// applied.
    pub(crate) fn claim(&self) -> bool {
        self.claim.claim()
    }

    /// Sends the answer to an asker that may have stopped listening.
    pub fn send(&self, answer: Answer) -> Result<(), std::sync::mpsc::SendError<Answer>> {
        self.reply.send(answer)
    }
}

/// A reply nobody has claimed or abandoned yet, for a request a test builds
/// by hand. Only tests may: a reply made this way has no asker that could
/// abandon it, so production requests always come from `relay`.
#[cfg(test)]
impl<Answer> From<std::sync::mpsc::SyncSender<Answer>> for Relayed<Answer> {
    fn from(reply: std::sync::mpsc::SyncSender<Answer>) -> Self {
        Self::waiting(reply).0
    }
}

impl<Answer> std::fmt::Debug for Relayed<Answer> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Relayed")
            .field("claim", &self.claim)
            .finish_non_exhaustive()
    }
}

/// How long an asker waits on the window.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Patience {
    /// For the window to answer. A request it has not claimed by then is
    /// abandoned, and will never be applied.
    pub(crate) answer: std::time::Duration,
    /// Further, when the window claimed the request before the asker could
    /// abandon it.
    pub(crate) after_claim: std::time::Duration,
}

impl Patience {
    /// Waits `answer` for the window, and [`AFTER_CLAIM`] more once it has
    /// claimed the request.
    pub(crate) const fn new(answer: std::time::Duration) -> Self {
        Self {
            answer,
            after_claim: AFTER_CLAIM,
        }
    }
}

#[derive(Debug)]
pub(crate) enum RelayError {
    /// The window is gone; the request never reached it.
    Disconnected,
    /// The window did not claim the request in time and now never will, so
    /// nothing was applied.
    Timeout,
    /// The window claimed the request and has not answered within the extra
    /// wait. It may still be applying it, so nobody may say nothing changed.
    Applying,
}

/// Hands a request to the window and waits, within `patience`, for its answer.
///
/// The request carries a claim the window must win before acting on it. When
/// the first wait runs out, the asker tries to abandon the claim instead; only
/// if that succeeds is the request known never to be applied. If the window
/// won first, the asker keeps waiting for the real answer, up to
/// `patience.after_claim`.
pub(crate) fn relay<Request, Answer>(
    sender: &async_channel::Sender<Request>,
    patience: Patience,
    request: impl FnOnce(Relayed<Answer>) -> Request,
) -> Result<Answer, RelayError> {
    let (reply, answer) = std::sync::mpsc::sync_channel(1);
    let (reply, claim) = Relayed::waiting(reply);
    sender
        .send_blocking(request(reply))
        .map_err(|_| RelayError::Disconnected)?;
    // A reply dropped unanswered ends this wait early too; the claim, not the
    // channel, then decides what may be said.
    if let Ok(answer) = answer.recv_timeout(patience.answer) {
        return Ok(answer);
    }
    if claim.abandon() {
        return Err(RelayError::Timeout);
    }
    answer
        .recv_timeout(patience.after_claim)
        .map_err(|_| RelayError::Applying)
}

#[cfg(test)]
mod tests {
    use super::super::test_support::*;
    use super::*;
    #[gpui::test]
    fn reload_reconciles_observation_endpoint_and_revokes_old_credentials(
        cx: &mut gpui::TestAppContext,
    ) {
        // A child owns runtime-directory variables without racing parallel tests.
        // Its private directory also works without a logged-in desktop.
        if std::env::var_os("SPRITE_OBSERVATION_RELOAD_TEST_CHILD").is_none() {
            let directory = std::env::temp_dir().join(format!("sp-r-{:x}", std::process::id()));
            std::fs::create_dir_all(&directory).unwrap();
            crate::test_child::run_child_test(
                "workspace::reload::tests::reload_reconciles_observation_endpoint_and_revokes_old_credentials",
                &[
                    ("SPRITE_OBSERVATION_RELOAD_TEST_CHILD", "1".as_ref()),
                    ("XDG_RUNTIME_DIR", directory.as_os_str()),
                    ("TMPDIR", directory.as_os_str()),
                ],
                "the reload subprocess",
            );
            std::fs::remove_dir_all(directory).unwrap();
            return;
        }
        use std::io::{Read, Write};
        use std::os::unix::net::UnixStream;
        let (workspace, cx) = test_workspace(cx);
        let path = std::env::temp_dir().join(format!(
            "sprite-observation-reload-{}.toml",
            std::process::id()
        ));
        workspace.update(cx, |workspace, cx| {
            workspace.config_path = Some(path.clone());
            workspace.set_observation_enabled(true, cx);
        });
        let (old_socket, old_key, layout_count, title_count) =
            workspace.read_with(cx, |workspace, _| {
                let endpoint = workspace
                    .endpoint
                    .as_ref()
                    .expect("enabled observation endpoint");
                (
                    endpoint.socket_path().to_owned(),
                    endpoint.key_hex(),
                    workspace.panes.layout_publications(),
                    workspace.pane_titles.len(),
                )
            });
        assert!(old_socket.exists());
        std::fs::write(&path, "[pane_observation]\nenabled = false\n").unwrap();
        let (answer, receive) = async_channel::bounded(1);
        let socket = old_socket.clone();
        let key = old_key.clone();
        let client = std::thread::spawn(move || {
            let result = (|| -> std::io::Result<String> {
                let mut stream = UnixStream::connect(socket)?;
                stream.set_read_timeout(Some(std::time::Duration::from_secs(3)))?;
                writeln!(stream, "{key} sprite-observation/1 config reload")?;
                let mut answer = String::new();
                stream.read_to_string(&mut answer)?;
                Ok(answer)
            })();
            answer.send_blocking(result).unwrap();
        });
        let executor = cx.executor();
        executor.allow_parking();
        let report = executor
            .block_test(async { receive.recv().await.unwrap() })
            .unwrap();
        client.join().unwrap();
        assert!(report.starts_with(&format!("reloaded {}", path.display())));
        assert!(report.contains("applied now: pane_observation"));
        workspace.read_with(cx, |workspace, cx| {
            assert!(!workspace.settings.pane_observation.enabled);
            assert!(
                !cx.global::<crate::config::ActiveSettings>()
                    .0
                    .pane_observation
                    .enabled
            );
            assert!(
                workspace.endpoint.is_none(),
                "reload must close observation before installing the new preference"
            );
            let environment = super::session_environment(
                workspace.endpoint.as_ref(),
                workspace.surfaces.as_ref(),
                crate::tabs::TabId(0),
                PaneId(0),
            );
            assert!(
                !environment
                    .iter()
                    .any(|(key, _)| key == "SPRITE_OBSERVATION_KEY"
                        || key == "SPRITE_OBSERVATION_SOCKET")
            );
        });
        assert!(!old_socket.exists());
        assert!(UnixStream::connect(&old_socket).is_err());

        std::fs::write(&path, "[pane_observation]\nenabled = true\n").unwrap();
        let report = workspace.update(cx, |workspace, cx| workspace.reload(None, cx));
        assert!(report.contains("applied now: pane_observation"));
        let new_socket = workspace.read_with(cx, |workspace, cx| {
            assert!(workspace.settings.pane_observation.enabled);
            assert!(
                cx.global::<crate::config::ActiveSettings>()
                    .0
                    .pane_observation
                    .enabled
            );
            let endpoint = workspace
                .endpoint
                .as_ref()
                .expect("reload must reopen observation");
            assert!(
                endpoint.key_hex() != old_key,
                "reenabling creates a fresh key"
            );
            assert_ne!(endpoint.socket_path(), old_socket);
            assert_eq!(workspace.panes.layout_publications(), layout_count);
            assert_eq!(workspace.pane_titles.len(), title_count);
            endpoint.socket_path().to_owned()
        });
        let mut rejected = UnixStream::connect(&new_socket).unwrap();
        rejected
            .set_read_timeout(Some(std::time::Duration::from_secs(2)))
            .unwrap();
        writeln!(rejected, "{old_key} unused").unwrap();
        let mut response = String::new();
        rejected.read_to_string(&mut response).unwrap();
        assert_eq!(response.trim(), crate::observation::endpoint::DENIED);
        let unchanged = workspace.update(cx, |workspace, cx| workspace.reload(None, cx));
        assert!(unchanged.contains("nothing changed"));
        workspace.update(cx, |workspace, cx| {
            assert_eq!(
                workspace.endpoint.as_ref().unwrap().socket_path(),
                new_socket
            );
            workspace.begin_shutdown(cx);
        });
        assert!(!new_socket.exists());
        std::fs::remove_file(path).unwrap();
    }
    /// The distinction the whole command rests on: what may change under a
    /// running shell, and what may not.
    #[test]
    fn changes_are_sorted_by_when_they_can_apply() {
        use crate::config::Settings;

        let current = Settings::default();

        let mut fonts = current.clone();
        fonts.font.size = crate::config::FontSize::new(20.0);
        let outcome = current.diff(&fonts);
        assert_eq!(outcome.live, vec![crate::config::LiveChange::Font]);
        assert!(outcome.next_session.is_empty());

        let mut grid = current.clone();
        grid.grid.padding = crate::config::Padding::new(24.0);
        let outcome = current.diff(&grid);
        assert_eq!(outcome.live, vec![crate::config::LiveChange::Grid]);
        assert!(outcome.next_session.is_empty());

        let mut highlights = current.clone();
        highlights.highlights = crate::config::Highlights::from_groups(vec![(
            "Comment".to_owned(),
            crate::config::HighlightStyle {
                italic: Some(true),
                ..Default::default()
            },
        )]);
        let outcome = current.diff(&highlights);
        assert_eq!(outcome.live, vec![crate::config::LiveChange::Highlights]);
        assert!(outcome.next_session.is_empty());

        let mut shell = current.clone();
        shell.scrollback.bytes = crate::config::ScrollbackBytes::new(4096);
        shell.shell.program = crate::config::NonBlank::new("/bin/zsh".into());
        let outcome = current.diff(&shell);
        assert!(outcome.live.is_empty());
        assert_eq!(
            outcome.next_session,
            vec![
                crate::config::NextSessionChange::Shell,
                crate::config::NextSessionChange::Scrollback
            ]
        );

        // The two graphics limits part company here: one belongs to the
        // renderer and can change now, the other to a terminal already running.
        let mut graphics = current.clone();
        graphics.graphics.texture_bytes = crate::config::TextureBytes::new(1024);
        graphics.graphics.storage_bytes = crate::config::StorageBytes::new(1024);
        let outcome = current.diff(&graphics);
        assert_eq!(outcome.live, vec![crate::config::LiveChange::TextureBudget]);
        assert_eq!(
            outcome.next_session,
            vec![crate::config::NextSessionChange::GraphicsStorage]
        );

        assert_eq!(current.diff(&current), Default::default());
    }
    #[test]
    fn a_reload_report_says_what_happened_to_each_part() {
        use crate::config::Settings;

        let mut next = Settings::default();
        next.font.size = crate::config::FontSize::new(20.0);
        next.scrollback.bytes = crate::config::ScrollbackBytes::new(4096);
        let report = Settings::default().diff(&next).describe(
            std::path::Path::new("/home/someone/.config/sprite/config.toml"),
            &["cursor.style \"wobbly\" is not one of them".to_owned()],
        );

        assert!(report.starts_with("reloaded /home/someone/.config/sprite/config.toml"));
        assert!(report.contains("applied now: font"));
        assert!(report.contains("waiting for a new pane: scrollback"));
        assert!(report.contains("ignored: cursor.style"));

        let unchanged = Settings::default()
            .diff(&Settings::default())
            .describe(std::path::Path::new("/tmp/config.toml"), &[]);
        assert!(unchanged.contains("nothing changed"));
    }

    /// A reload the endpoint has given up on was reported as "nothing was
    /// changed", so the window must never apply it afterwards.
    ///
    /// The GPUI thread is not run while the endpoint waits, which is exactly
    /// what a busy window looks like from an endpoint thread; the window then
    /// reaches the queued request only after the endpoint has answered.
    #[gpui::test]
    fn a_reload_the_endpoint_gave_up_on_is_never_applied(cx: &mut gpui::TestAppContext) {
        let (workspace, cx) = test_workspace(cx);
        let path = std::env::temp_dir().join(format!(
            "sprite-abandoned-reload-{}.toml",
            std::process::id()
        ));
        std::fs::write(&path, "[font]\nsize = 21.0\n").unwrap();
        let (sender, size_before) = workspace.update(cx, |workspace, _| {
            workspace.config_path = Some(path.clone());
            (workspace.reload_sender.clone(), workspace.font_size())
        });
        assert_ne!(size_before, 21.0, "the file must ask for a change");

        let endpoint = std::thread::spawn(move || {
            relay(
                &sender,
                Patience {
                    answer: std::time::Duration::from_millis(50),
                    after_claim: std::time::Duration::from_secs(5),
                },
                |reply| ReloadRequest {
                    what: ConfigVerb::Reload,
                    reply,
                    reply_connection: None,
                },
            )
        });
        let outcome = endpoint.join().unwrap();
        assert!(
            matches!(outcome, Err(RelayError::Timeout)),
            "the endpoint gave up and may say nothing changed: {outcome:?}"
        );

        cx.run_until_parked();
        workspace.read_with(cx, |workspace, _| {
            assert!(
                workspace.reload_sender.is_empty(),
                "the window took the request off its queue"
            );
            assert_eq!(
                workspace.font_size(),
                size_before,
                "an abandoned reload must never be applied"
            );
        });
        std::fs::remove_file(&path).unwrap();
    }

    /// Once the window has claimed a request, its real answer is what the
    /// asker reports, even when it arrives after the first wait.
    ///
    /// Claimed by the asker's own closure, before the request is sent, so the
    /// claim is certain to precede the asker's attempt to abandon it.
    #[test]
    fn a_claimed_request_returns_the_real_answer_after_the_first_wait() {
        let (sender, requests) = async_channel::bounded::<Relayed<String>>(1);
        let window = std::thread::spawn(move || {
            let reply = requests.recv_blocking().expect("a request");
            crate::test_blocking_wait::pause(std::time::Duration::from_millis(200));
            reply
                .send("applied".to_owned())
                .expect("the asker is still listening");
        });
        let started = std::time::Instant::now();
        let answer = relay(
            &sender,
            Patience {
                answer: std::time::Duration::from_millis(40),
                after_claim: std::time::Duration::from_secs(5),
            },
            |reply: Relayed<String>| {
                assert!(reply.claim(), "nobody has given up yet");
                reply
            },
        );
        window.join().unwrap();
        assert_eq!(answer.expect("the window's real answer"), "applied");
        assert!(started.elapsed() >= std::time::Duration::from_millis(200));
    }

    /// The wait after a claim is bounded, and what it ends with never says
    /// nothing was changed.
    #[test]
    fn a_claimed_request_that_is_not_answered_in_time_is_still_applying() {
        let (sender, requests) = async_channel::bounded::<Relayed<String>>(1);
        let (finished, done) = std::sync::mpsc::channel::<()>();
        let window = std::thread::spawn(move || {
            // Held open without an answer, as a window still applying would.
            let reply = requests.recv_blocking().expect("a request");
            let _ = done.recv();
            drop(reply);
        });
        let started = std::time::Instant::now();
        let answer = relay(
            &sender,
            Patience {
                answer: std::time::Duration::from_millis(30),
                after_claim: std::time::Duration::from_millis(120),
            },
            |reply: Relayed<String>| {
                assert!(reply.claim());
                reply
            },
        );
        let waited = started.elapsed();
        finished.send(()).unwrap();
        window.join().unwrap();
        assert!(
            matches!(answer, Err(RelayError::Applying)),
            "a claimed request is never reported as a timeout: {answer:?}"
        );
        assert!(
            waited >= std::time::Duration::from_millis(150),
            "{waited:?}"
        );
        assert!(waited < std::time::Duration::from_secs(2), "{waited:?}");
    }

    /// A wedged window holds up no endpoint thread before that thread's own
    /// wait has started: every connection the endpoint serves at once can
    /// hand its request over, and each then gives up within its patience.
    #[test]
    fn every_endpoint_connection_hands_its_request_over_while_the_window_is_wedged() {
        use crate::observation::endpoint::MAX_CONNECTIONS;
        let (sender, _wedged) = reload_channel();
        let (done, finished) = std::sync::mpsc::channel();
        for _ in 0..MAX_CONNECTIONS {
            let (sender, done) = (sender.clone(), done.clone());
            std::thread::spawn(move || {
                let answer = relay(
                    &sender,
                    Patience {
                        answer: std::time::Duration::from_millis(20),
                        after_claim: std::time::Duration::from_secs(5),
                    },
                    |reply| ReloadRequest {
                        what: ConfigVerb::Print,
                        reply,
                        reply_connection: None,
                    },
                );
                let _ = done.send(matches!(answer, Err(RelayError::Timeout)));
            });
        }
        for _ in 0..MAX_CONNECTIONS {
            let timed_out = finished
                .recv_timeout(std::time::Duration::from_secs(5))
                .expect("an asker is still waiting to hand its request over");
            assert!(timed_out, "nothing answered, so nothing was applied");
        }
    }

    /// Abandoning and claiming exclude each other: a request the asker gave
    /// up on can no longer be claimed by the window.
    #[test]
    fn an_abandoned_request_can_no_longer_be_claimed() {
        let (sender, requests) = async_channel::bounded::<Relayed<String>>(1);
        let answer = relay(
            &sender,
            Patience {
                answer: std::time::Duration::from_millis(20),
                after_claim: std::time::Duration::from_secs(5),
            },
            |reply| reply,
        );
        assert!(matches!(answer, Err(RelayError::Timeout)), "{answer:?}");
        let late = requests.try_recv().expect("the request is still queued");
        assert!(!late.claim(), "the window must drop it unapplied");
    }

    /// Observation turned on by a reload must find the panes that were opened
    /// while it was off, not only the ones opened afterwards.
    #[gpui::test]
    fn panes_opened_while_observation_was_off_are_observable_once_reload_turns_it_on(
        cx: &mut gpui::TestAppContext,
    ) {
        // A child owns runtime-directory variables without racing parallel
        // tests. Its private directory also works without a logged-in desktop.
        if std::env::var_os("SPRITE_OBSERVATION_EXISTING_PANES_TEST_CHILD").is_none() {
            let directory = std::env::temp_dir().join(format!("sp-e-{:x}", std::process::id()));
            std::fs::create_dir_all(&directory).unwrap();
            crate::test_child::run_child_test(
                "workspace::reload::tests::panes_opened_while_observation_was_off_are_observable_once_reload_turns_it_on",
                &[
                    ("SPRITE_OBSERVATION_EXISTING_PANES_TEST_CHILD", "1".as_ref()),
                    ("XDG_RUNTIME_DIR", directory.as_os_str()),
                    ("TMPDIR", directory.as_os_str()),
                ],
                "the existing-panes subprocess",
            );
            std::fs::remove_dir_all(directory).unwrap();
            return;
        }
        use std::io::{Read, Write};
        use std::os::unix::net::UnixStream;
        let (workspace, cx) = test_workspace(cx);
        let path =
            std::env::temp_dir().join(format!("sprite-existing-panes-{}.toml", std::process::id()));
        // Real sessions, so the panes can answer. The test workspace's own
        // first pane runs a program that does not exist, so it cannot answer
        // and is left out of what is expected.
        let opened = workspace.update_in(cx, |workspace, window, cx| {
            workspace.config_path = Some(path.clone());
            workspace.command = Some(vec![
                "/bin/sh".into(),
                "-c".into(),
                "printf 'existing-pane\\n'; exec sleep 30".into(),
            ]);
            workspace.split(Orientation::Vertical, window, cx);
            workspace.open_tab(window, cx);
            let mut opened: Vec<u64> = workspace
                .tabs
                .all_panes()
                .into_iter()
                .map(|(_, pane, _)| pane.0)
                .filter(|pane| *pane != 0)
                .collect();
            opened.sort_unstable();
            opened
        });
        assert_eq!(opened.len(), 2, "a split and a new tab: {opened:?}");
        workspace.read_with(cx, |workspace, _| {
            assert!(workspace.endpoint.is_none(), "observation starts off");
        });

        std::fs::write(&path, "[pane_observation]\nenabled = true\n").unwrap();
        let report = workspace.update(cx, |workspace, cx| workspace.reload(None, cx));
        assert!(report.contains("applied now: pane_observation"), "{report}");
        let (socket, key) = workspace.read_with(cx, |workspace, _| {
            let endpoint = workspace
                .endpoint
                .as_ref()
                .expect("the reload turned observation on");
            (endpoint.socket_path().to_owned(), endpoint.key_hex())
        });

        let (answer, receive) = async_channel::bounded(1);
        let client = std::thread::spawn(move || {
            let result = (|| -> std::io::Result<String> {
                let mut stream = UnixStream::connect(socket)?;
                stream.set_read_timeout(Some(std::time::Duration::from_secs(5)))?;
                writeln!(stream, "{key} sprite-observation/1 panes snapshot --window")?;
                let mut answer = String::new();
                stream.read_to_string(&mut answer)?;
                Ok(answer)
            })();
            answer.send_blocking(result).unwrap();
        });
        let executor = cx.executor();
        executor.allow_parking();
        let response = executor
            .block_test(async { receive.recv().await.unwrap() })
            .unwrap();
        client.join().unwrap();

        let value: serde_json::Value =
            serde_json::from_str(&response).unwrap_or_else(|error| panic!("{error}: {response}"));
        let mut answered: Vec<u64> = value["panes"]
            .as_array()
            .unwrap_or_else(|| panic!("a pane list: {response}"))
            .iter()
            .map(|pane| pane["pane"].as_u64().expect("a pane id"))
            .collect();
        answered.sort_unstable();
        assert_eq!(
            answered, opened,
            "every pane opened while observation was off is listed and answers: {response}"
        );
        // The first pane never got a session, so it is listed as closed
        // rather than silently missing; no other pane may fail.
        let failed: Vec<u64> = value["errors"]
            .as_array()
            .unwrap_or_else(|| panic!("an error list: {response}"))
            .iter()
            .map(|error| error["pane"].as_u64().expect("a pane id"))
            .collect();
        assert_eq!(failed, [0], "{response}");

        let cleanups = workspace.update(cx, |workspace, cx| workspace.begin_shutdown(cx));
        executor.block_test(async move {
            for cleanup in cleanups {
                cleanup.await;
            }
        });
        std::fs::remove_file(path).unwrap();
    }
    /// Zoom is the person's, not the file's. A reload that only recolours the
    /// window must neither undo three steps of zoom nor claim the font changed,
    /// and reset still returns to the size the file asks for.
    #[gpui::test]
    fn a_colour_only_reload_keeps_the_zoom_and_reports_only_colours(cx: &mut gpui::TestAppContext) {
        let (workspace, cx) = test_workspace(cx);
        let path =
            std::env::temp_dir().join(format!("sprite-zoom-reload-{}.toml", std::process::id()));
        workspace.update(cx, |workspace, _| {
            workspace.config_path = Some(path.clone());
        });
        draw_workspace(cx);
        cx.simulate_keystrokes("ctrl-shift-= ctrl-shift-= ctrl-shift-=");
        let zoomed = crate::config::Font::DEFAULT_SIZE + 3.0;
        cx.update(|_, cx| {
            assert_eq!(
                cx.global::<crate::config::ActiveSettings>().0.font.size,
                zoomed,
                "three zoom steps reached the panes"
            );
        });

        // Observation stays off, as `test_workspace` set it, so the only
        // difference from the running file is the background colour.
        std::fs::write(
            &path,
            "[pane_observation]\nenabled = false\n\n[colors]\nbackground = \"#203040\"\n",
        )
        .unwrap();
        let report = workspace.update(cx, |workspace, cx| workspace.reload(None, cx));
        std::fs::remove_file(&path).unwrap();

        assert_eq!(
            report,
            format!("reloaded {}\napplied now: colors", path.display())
        );
        cx.update(|_, cx| {
            let active = &cx.global::<crate::config::ActiveSettings>().0;
            assert_eq!(active.font.size, zoomed, "the zoom survives the reload");
            assert_eq!(
                active.colors.background,
                crate::config::Colors::parse_hex("#203040")
            );
        });
        cx.simulate_keystrokes("ctrl-shift-0");
        cx.update(|_, cx| {
            assert_eq!(
                cx.global::<crate::config::ActiveSettings>().0.font.size,
                crate::config::Font::DEFAULT_SIZE,
                "reset returns to the file's size"
            );
        });
    }

    /// The file's size still matters while zoomed: it is where reset goes. An
    /// unzoomed window simply follows it.
    #[gpui::test]
    fn a_reloaded_font_size_is_where_reset_goes_and_zoom_stays_on_top(
        cx: &mut gpui::TestAppContext,
    ) {
        let (workspace, cx) = test_workspace(cx);
        let path = std::env::temp_dir().join(format!(
            "sprite-zoom-size-reload-{}.toml",
            std::process::id()
        ));
        workspace.update(cx, |workspace, _| {
            workspace.config_path = Some(path.clone());
        });
        draw_workspace(cx);
        let active_size = |cx: &mut gpui::VisualTestContext| {
            cx.update(|_, cx| {
                cx.global::<crate::config::ActiveSettings>()
                    .0
                    .font
                    .size
                    .get()
            })
        };
        cx.simulate_keystrokes("ctrl-shift-=");
        assert_eq!(active_size(cx), crate::config::Font::DEFAULT_SIZE + 1.0);

        std::fs::write(
            &path,
            "[pane_observation]\nenabled = false\n\n[font]\nsize = 20\n",
        )
        .unwrap();
        let report = workspace.update(cx, |workspace, cx| workspace.reload(None, cx));
        assert_eq!(
            report,
            format!("reloaded {}\napplied now: font", path.display())
        );
        assert_eq!(
            active_size(cx),
            crate::config::Font::DEFAULT_SIZE + 1.0,
            "a zoomed window keeps its zoom"
        );
        cx.simulate_keystrokes("ctrl-shift-0");
        assert_eq!(active_size(cx), 20.0, "reset goes to the new file size");

        std::fs::write(
            &path,
            "[pane_observation]\nenabled = false\n\n[font]\nsize = 22\n",
        )
        .unwrap();
        workspace.update(cx, |workspace, cx| workspace.reload(None, cx));
        std::fs::remove_file(&path).unwrap();
        assert_eq!(active_size(cx), 22.0, "an unzoomed window follows the file");
    }
}
