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

        let outcome = self.settings.diff(&settings);
        if outcome.has(crate::config::LiveChange::Colors) {
            cx.global_mut::<crate::tokens::TokenRegistry>()
                .apply_theme(&settings.colors);
        }
        if outcome.has(crate::config::LiveChange::Observation) {
            self.change_observation(settings.pane_observation.enabled, reply, cx);
        }
        // Published, not pushed: each pane observes the global with its own
        // window in hand, which is what a cell re-measure needs and what this
        // method, reached from an endpoint thread, does not have.
        cx.set_global(crate::config::ActiveSettings(settings.clone()));
        self.settings = settings;
        self.configured_font_size = self.settings.font.size;
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
/// GPUI thread must cost the endpoint one two-second wait, not a thread that
/// never returns.
pub(crate) struct ReloadRequest {
    pub(crate) what: ConfigVerb,
    pub(crate) reply: std::sync::mpsc::SyncSender<String>,
    pub(crate) reply_connection: Option<crate::local_socket::ReplyConnection>,
}

pub(crate) enum RelayError {
    Disconnected,
    Timeout,
}

pub(crate) fn relay<Request, Answer>(
    sender: &async_channel::Sender<Request>,
    timeout: std::time::Duration,
    request: impl FnOnce(std::sync::mpsc::SyncSender<Answer>) -> Request,
) -> Result<Answer, RelayError> {
    let (reply, answer) = std::sync::mpsc::sync_channel(1);
    sender
        .send_blocking(request(reply))
        .map_err(|_| RelayError::Disconnected)?;
    answer
        .recv_timeout(timeout)
        .map_err(|_| RelayError::Timeout)
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
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "workspace::reload::tests::reload_reconciles_observation_endpoint_and_revokes_old_credentials", "--nocapture"])
                .env("SPRITE_OBSERVATION_RELOAD_TEST_CHILD", "1")
                .env("XDG_RUNTIME_DIR", &directory)
                .env("TMPDIR", &directory)
                .output().unwrap();
            std::fs::remove_dir_all(directory).unwrap();
            assert!(
                String::from_utf8_lossy(&output.stdout).contains("1 passed"),
                "reload subprocess must run its exact test"
            );
            assert!(
                output.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
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
}
