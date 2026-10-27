use std::time::{Duration, Instant};

#[test]
fn diagnostics_are_persisted_to_the_selected_directory() {
    let root = std::env::temp_dir().join(format!("sprite-logging-test-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let guard =
        sprite_app::diagnostics::init_at(root.clone(), "sprite_app=info,sprite_app::timing=debug")
            .unwrap();
    tracing::debug!(target: "sprite_app::timing", "visible-debug-marker");
    tracing::debug!(target: "sprite_app::test", "hidden-debug-marker");
    tracing::info!(target: "other_target", "hidden-other-marker");
    let _ = sprite_app::Settings::parse("private-config-secret = [broken");
    tracing::info!(target: "sprite_app::test", "logging-test-marker");
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if std::fs::read_dir(&root).unwrap().flatten().any(|entry| {
            std::fs::read_dir(entry.path())
                .ok()
                .into_iter()
                .flatten()
                .flatten()
                .any(|file| {
                    std::fs::read_to_string(file.path())
                        .is_ok_and(|contents| contents.contains("logging-test-marker"))
                })
        }) {
            drop(guard);
            let contents = std::fs::read_dir(&root)
                .unwrap()
                .flatten()
                .flat_map(|entry| {
                    std::fs::read_dir(entry.path())
                        .ok()
                        .into_iter()
                        .flatten()
                        .flatten()
                })
                .filter_map(|entry| std::fs::read_to_string(entry.path()).ok())
                .collect::<String>();
            assert!(contents.contains("visible-debug-marker"));
            assert!(!contents.contains("hidden-debug-marker"));
            assert!(!contents.contains("hidden-other-marker"));
            assert!(!contents.contains("private-config-secret"));
            assert!(contents.contains("invalid configuration syntax"));
            std::fs::remove_dir_all(root).unwrap();
            return;
        }
        std::thread::park_timeout(Duration::from_millis(10));
    }
    panic!("the diagnostic event was not persisted");
}

#[test]
fn cli_stdout_is_not_contaminated_by_logging() {
    let binary = env!("CARGO_BIN_EXE_sprite");
    for arguments in [
        vec!["--version"],
        vec!["panes", "snapshot"],
        vec!["surface", "open", "--fill"],
        vec![
            "config",
            "print",
            "--config",
            "/nonexistent/sprite-config.toml",
        ],
    ] {
        let run = |level: &str| {
            std::process::Command::new(binary)
                .args(&arguments)
                .env("SPRITE_LOG", level)
                .env_remove("SPRITE_OBSERVATION_SOCKET")
                .env_remove("SPRITE_OBSERVATION_KEY")
                .env_remove("SPRITE_SURFACE_SOCKET")
                .env_remove("SPRITE_SURFACE_KEY")
                .stdin(std::process::Stdio::null())
                .output()
                .unwrap()
        };
        let normal = run("sprite_app=warn");
        let verbose = run("trace");
        assert_eq!(normal.status.code(), verbose.status.code(), "{arguments:?}");
        assert_eq!(normal.stdout, verbose.stdout, "{arguments:?}");
        assert_eq!(normal.stderr, verbose.stderr, "{arguments:?}");
    }
}

#[test]
fn authenticated_snapshot_reply_is_not_mixed_with_diagnostics() {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixListener;

    let root = std::env::temp_dir().join(format!("sprite-log-protocol-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::remove_file(root.join("test.sock")).ok();
    let socket = root.join("test.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let peer = std::thread::spawn(move || {
        listener.set_nonblocking(true).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(Instant::now() < deadline, "client did not connect");
                    std::thread::park_timeout(Duration::from_millis(10));
                }
                Err(error) => panic!("accept failed: {error}"),
            }
        };
        stream.set_nonblocking(false).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut line = String::new();
        BufReader::new(stream.try_clone().unwrap())
            .read_line(&mut line)
            .unwrap();
        assert!(line.starts_with("test-private-key "));
        stream.write_all(b"{\"protocol_marker\":true}\n").unwrap();
    });
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_sprite"))
        .args(["panes", "snapshot", "--window"])
        .env("SPRITE_OBSERVATION_SOCKET", &socket)
        .env("SPRITE_OBSERVATION_KEY", "test-private-key")
        .env_remove("SPRITE_PANE")
        .env("SPRITE_LOG", "trace")
        .output()
        .unwrap();
    peer.join().unwrap();
    assert!(output.status.success());
    assert_eq!(output.stdout, b"{\"protocol_marker\":true}\n");
    assert!(output.stderr.is_empty());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn authenticated_surface_stream_is_not_mixed_with_diagnostics() {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixListener;
    use std::process::Stdio;

    let root = std::env::temp_dir().join(format!("sprite-log-surface-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let socket = root.join("test.sock");
    std::fs::remove_file(&socket).ok();
    let listener = UnixListener::bind(&socket).unwrap();
    let peer = std::thread::spawn(move || {
        listener.set_nonblocking(true).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(Instant::now() < deadline, "client did not connect");
                    std::thread::park_timeout(Duration::from_millis(10));
                }
                Err(error) => panic!("accept failed: {error}"),
            }
        };
        stream.set_nonblocking(false).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut line = String::new();
        BufReader::new(stream.try_clone().unwrap())
            .read_line(&mut line)
            .unwrap();
        assert!(line.starts_with("surface-private-key "));
        stream
            .write_all(b"{\"type\":\"opened\"}\n{\"type\":\"closed\"}\n")
            .unwrap();
    });
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_sprite"))
        .args(["surface", "open", "--fill"])
        .env("SPRITE_SURFACE_SOCKET", &socket)
        .env("SPRITE_SURFACE_KEY", "surface-private-key")
        .env("SPRITE_PANE", "1")
        .env("SPRITE_LOG", "trace")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"{}\n").unwrap();
    let output = child.wait_with_output().unwrap();
    peer.join().unwrap();
    assert!(output.status.success());
    assert_eq!(
        output.stdout,
        b"{\"type\":\"opened\"}\n{\"type\":\"closed\"}\n"
    );
    assert!(output.stderr.is_empty());
    std::fs::remove_dir_all(root).unwrap();
}
