//! `sprite panes snapshot`: the shell-facing observation client.
//!
//! It reads the socket, key, and its own pane identity from the environment its
//! window injected, sends one bounded request, and writes the answer to standard
//! output. It is the only supported consumer of the private protocol; other
//! tools integrate through this command's JSON rather than by connecting to the
//! socket themselves.
//!
//! **It never hangs.** Every step has a timeout, because this runs inside a
//! shell where a command that never returns is worse than one that fails.

use std::io::{BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

use crate::cli::{Scope, SnapshotArgs};
use crate::observation::broker::PROTOCOL;

/// The environment a window gives each of its sessions.
pub const SOCKET_VARIABLE: &str = "SPRITE_OBSERVATION_SOCKET";
pub const KEY_VARIABLE: &str = "SPRITE_OBSERVATION_KEY";
pub const PANE_VARIABLE: &str = "SPRITE_PANE";

/// Generous next to the window's own 500 ms deadline: this bounds the whole
/// exchange including encoding a response that may be megabytes, and exists to
/// prevent a hang rather than to enforce a schedule.
const TIMEOUT: Duration = Duration::from_secs(15);

/// What the command should exit with.
///
/// A response that parses is a success even when it reports missing panes: the
/// panes that did answer are usable, and a shell pipeline should not have to
/// treat a partial answer as a failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Exit {
    Ok = 0,
    Usage = 2,
    /// Not running inside a Sprite window, so there is nothing to ask.
    NoWindow = 3,
    /// The window's socket could not be reached or did not answer.
    Unreachable = 4,
    /// The window answered, but not with a snapshot.
    Refused = 5,
}

/// Runs one request, writing JSON to `out` and diagnostics to `errors`.
pub fn run_snapshot(args: &SnapshotArgs, out: &mut dyn Write, errors: &mut dyn Write) -> Exit {
    let environment = |name: &str| std::env::var(name).ok().filter(|value| !value.is_empty());

    let (Some(socket), Some(key)) = (environment(SOCKET_VARIABLE), environment(KEY_VARIABLE))
    else {
        // The common case for this message is a person running the command in
        // an ordinary terminal, so it says what is wrong rather than naming a
        // variable and leaving them to guess.
        let _ = writeln!(
            errors,
            "sprite: not running inside a Sprite window, so there are no panes to observe\n\
             (this command reads {SOCKET_VARIABLE} and {KEY_VARIABLE}, which a Sprite window sets \
             for the sessions it starts)"
        );
        return Exit::NoWindow;
    };

    let request = match request_line(args, &key, environment(PANE_VARIABLE).as_deref()) {
        Ok(request) => request,
        Err(message) => {
            let _ = writeln!(errors, "sprite: {message}");
            return Exit::Usage;
        }
    };

    let answer = match exchange(&socket, &request) {
        Ok(answer) => answer,
        Err(error) => {
            let _ = writeln!(
                errors,
                "sprite: could not ask this window about its panes: {error}"
            );
            return Exit::Unreachable;
        }
    };

    // The window answers refusals in plain words, so anything that is not a
    // JSON object is a refusal — and must reach standard error, never standard
    // output, where a caller would parse it as data.
    let trimmed = answer.trim();
    if serde_json::from_str::<serde_json::Value>(trimmed).is_err() {
        let detail = if trimmed.is_empty() {
            "the window closed the connection without answering"
        } else {
            trimmed
        };
        let _ = writeln!(errors, "sprite: {detail}");
        return Exit::Refused;
    }

    let _ = writeln!(out, "{trimmed}");
    Exit::Ok
}

/// Asks the containing window to re-read its configuration file.
///
/// The answer is prose rather than JSON — it is written for the person who
/// typed the command, and there is nothing here for a tool to parse — so it
/// goes to standard output as it arrives.
pub fn run_config_reload(out: &mut dyn Write, errors: &mut dyn Write) -> Exit {
    let environment = |name: &str| std::env::var(name).ok().filter(|value| !value.is_empty());

    let (Some(socket), Some(key)) = (environment(SOCKET_VARIABLE), environment(KEY_VARIABLE))
    else {
        let _ = writeln!(
            errors,
            "sprite: not running inside a Sprite window, so there is no window to reload\n\
             (a window reloads its own configuration; this command asks the one it is running in)"
        );
        return Exit::NoWindow;
    };

    match exchange(&socket, &format!("{key} {PROTOCOL} config reload")) {
        Ok(answer) => {
            let trimmed = answer.trim();
            if trimmed.is_empty() {
                let _ = writeln!(
                    errors,
                    "sprite: the window closed the connection without answering"
                );
                return Exit::Refused;
            }
            // A refusal is prose too, so it is told apart by what it says
            // rather than by its shape: only a reload that happened begins by
            // saying so.
            if !trimmed.starts_with("reloaded ") {
                let _ = writeln!(errors, "sprite: {trimmed}");
                return Exit::Refused;
            }
            let _ = writeln!(out, "{trimmed}");
            Exit::Ok
        }
        Err(error) => {
            let _ = writeln!(
                errors,
                "sprite: could not ask this window to reload: {error}"
            );
            Exit::Unreachable
        }
    }
}

/// Prints the configuration that is actually in effect.
///
/// Inside a window it asks that window, because after a reload the file and
/// what the window is using can differ, and the useful answer is the second.
/// Outside one — or when a file is named — it reads the file itself and says
/// which, so the output is never ambiguous about what it describes.
pub fn run_config_print(
    args: &crate::cli::ConfigPrintArgs,
    out: &mut dyn Write,
    errors: &mut dyn Write,
) -> Exit {
    let environment = |name: &str| std::env::var(name).ok().filter(|value| !value.is_empty());

    if let Some(path) = &args.path {
        let (settings, complaints) = crate::config::Settings::load_from(path);
        for complaint in complaints.0 {
            let _ = writeln!(errors, "sprite: {complaint}");
        }
        let _ = writeln!(out, "# {}", path.display());
        let _ = write!(out, "{}", settings.to_toml());
        return Exit::Ok;
    }

    match (environment(SOCKET_VARIABLE), environment(KEY_VARIABLE)) {
        (Some(socket), Some(key)) => {
            match exchange(&socket, &format!("{key} {PROTOCOL} config print")) {
                Ok(answer) if !answer.trim().is_empty() => {
                    let _ = writeln!(out, "# in effect in this window");
                    let _ = write!(out, "{}", answer.trim_end());
                    let _ = writeln!(out);
                    Exit::Ok
                }
                Ok(_) => {
                    let _ = writeln!(
                        errors,
                        "sprite: the window closed the connection without answering"
                    );
                    Exit::Refused
                }
                Err(error) => {
                    let _ = writeln!(errors, "sprite: could not ask this window: {error}");
                    Exit::Unreachable
                }
            }
        }
        _ => {
            // Not inside a window is not a failure here: what Sprite *would*
            // load is a real answer, and it is what somebody checking a file
            // before opening a window is asking for.
            let path = crate::config::path();
            let (settings, complaints) = match &path {
                Some(path) => crate::config::Settings::load_from(path),
                None => (crate::config::Settings::default(), Default::default()),
            };
            for complaint in complaints.0 {
                let _ = writeln!(errors, "sprite: {complaint}");
            }
            match path {
                Some(path) if path.exists() => {
                    let _ = writeln!(out, "# {} (not running inside a window)", path.display());
                }
                Some(path) => {
                    let _ = writeln!(
                        out,
                        "# {} does not exist, so these are Sprite's defaults",
                        path.display()
                    );
                }
                None => {
                    let _ = writeln!(out, "# no configuration file location; Sprite's defaults");
                }
            }
            let _ = write!(out, "{}", settings.to_toml());
            Exit::Ok
        }
    }
}

/// Builds the request, which carries the key and nothing the window did not ask
/// for.
fn request_line(args: &SnapshotArgs, key: &str, pane: Option<&str>) -> Result<String, String> {
    let mut request = format!("{key} {PROTOCOL} panes snapshot");

    // The pane says who it is so the default scope can mean "my tab". It is
    // read from the environment rather than accepted as an argument: a caller
    // naming someone else's pane would learn nothing it could not already ask
    // for, but it would make the common request easy to get wrong.
    if let Some(pane) = pane {
        request.push_str(&format!(" --from {pane}"));
    } else if matches!(args.scope, Scope::Tab | Scope::TabWithSelf) {
        return Err(format!(
            "this session was not told which pane it is, so \"my tab\" has no meaning here \
             (expected {PANE_VARIABLE}); try --window or --pane"
        ));
    }

    match args.scope {
        Scope::Tab => {}
        Scope::TabWithSelf => request.push_str(" --include-self"),
        Scope::Pane(pane) => request.push_str(&format!(" --pane {pane}")),
        Scope::Window => request.push_str(" --window"),
    }
    if let Some(lines) = args.lines {
        request.push_str(&format!(" --lines {lines}"));
    }
    if args.pretty {
        request.push_str(" --pretty");
    }
    Ok(request)
}

/// One request, one answer, with a bound on every step.
fn exchange(socket: &str, request: &str) -> std::io::Result<String> {
    let stream = UnixStream::connect(socket)?;
    stream.set_read_timeout(Some(TIMEOUT))?;
    stream.set_write_timeout(Some(TIMEOUT))?;

    {
        let mut writer = &stream;
        writeln!(writer, "{request}")?;
        writer.flush()?;
    }
    // The window closes its write half when an answer is complete, so reading
    // to the end is the frame. A response may be laid out over many lines.
    let mut answer = String::new();
    BufReader::new(&stream).read_to_string(&mut answer)?;
    Ok(answer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::SnapshotArgs;

    fn line(args: &SnapshotArgs, pane: Option<&str>) -> String {
        request_line(args, "the-key", pane).expect("a request")
    }

    #[test]
    fn a_request_names_the_protocol_and_the_calling_pane() {
        let request = line(&SnapshotArgs::default(), Some("4"));
        assert_eq!(
            request,
            format!("the-key {PROTOCOL} panes snapshot --from 4")
        );
    }

    #[test]
    fn scope_and_options_reach_the_window() {
        assert!(
            line(
                &SnapshotArgs {
                    scope: Scope::Window,
                    lines: Some(12),
                    pretty: true,
                },
                Some("0")
            )
            .ends_with("panes snapshot --from 0 --window --lines 12 --pretty")
        );
        assert!(
            line(
                &SnapshotArgs {
                    scope: Scope::Pane(9),
                    ..SnapshotArgs::default()
                },
                Some("0")
            )
            .ends_with("--pane 9")
        );
        assert!(
            line(
                &SnapshotArgs {
                    scope: Scope::TabWithSelf,
                    ..SnapshotArgs::default()
                },
                Some("0")
            )
            .ends_with("--include-self")
        );
    }

    /// A session with no pane identity can still ask about named panes, but
    /// "my tab" has no meaning, and saying so beats sending a request the
    /// window will refuse.
    #[test]
    fn a_session_without_a_pane_identity_is_told_what_it_can_still_ask() {
        let refused = request_line(&SnapshotArgs::default(), "k", None).expect_err("no identity");
        assert!(refused.contains("--window"), "it names a way forward");

        let allowed = request_line(
            &SnapshotArgs {
                scope: Scope::Window,
                ..SnapshotArgs::default()
            },
            "k",
            None,
        )
        .expect("a window request needs no identity");
        assert!(allowed.ends_with("--window"));
        assert!(!allowed.contains("--from"));
    }
}
