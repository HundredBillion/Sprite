//! `sprite panes snapshot`: the shell-facing observation client.
//!
//! It reads the socket, key, and its own pane identity from the environment its
//! window injected, sends one bounded request, and writes the answer to standard
//! output. It is the only supported consumer of the private protocol; other
//! tools integrate through this command's JSON rather than by connecting to the
//! socket themselves.
//!
//! Socket reads and writes have inactivity timeouts so a silent peer cannot
//! stall the shell. These are per-operation limits, not a total exchange deadline.

use std::io::{BufReader, Read, Write};
use std::os::unix::net::UnixStream;

use sprite_term::HistoryLines;

use crate::cli::SnapshotArgs;
use crate::observation::broker::PROTOCOL;
use crate::observation::request::{self, Query, Scope};
use crate::pane_tree::PaneId;

/// The environment a window gives each of its sessions.
pub const SOCKET_VARIABLE: &str = "SPRITE_OBSERVATION_SOCKET";
pub const KEY_VARIABLE: &str = "SPRITE_OBSERVATION_KEY";
pub const PANE_VARIABLE: &str = "SPRITE_PANE";

use crate::timeouts::bound_client;

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
        writeln!(
            errors,
            "sprite: not running inside a Sprite window, so there are no panes to observe\n\
             (this command reads {SOCKET_VARIABLE} and {KEY_VARIABLE}, which a Sprite window sets \
             for the sessions it starts)"
        )
        .ok();
        return Exit::NoWindow;
    };

    let request = match request_line(args, &key, environment(PANE_VARIABLE).as_deref()) {
        Ok(request) => request,
        Err(message) => {
            writeln!(errors, "sprite: {message}").ok();
            return Exit::Usage;
        }
    };

    let answer = match exchange(&socket, &request) {
        Ok(answer) => answer,
        Err(error) => {
            writeln!(
                errors,
                "sprite: could not ask this window about its panes: {error}"
            )
            .ok();
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
        writeln!(errors, "sprite: {detail}").ok();
        return Exit::Refused;
    }

    writeln!(out, "{trimmed}").ok();
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
        writeln!(
            errors,
            "sprite: not running inside a Sprite window, so there is no window to reload\n\
             (a window reloads its own configuration; this command asks the one it is running in)"
        )
        .ok();
        return Exit::NoWindow;
    };

    match exchange(&socket, &format!("{key} {PROTOCOL} config reload")) {
        Ok(answer) => {
            let trimmed = answer.trim();
            if trimmed.is_empty() {
                writeln!(
                    errors,
                    "sprite: the window closed the connection without answering"
                )
                .ok();
                return Exit::Refused;
            }
            // A refusal is prose too, so it is told apart by what it says
            // rather than by its shape: only a reload that happened begins by
            // saying so.
            if !trimmed.starts_with("reloaded ") {
                writeln!(errors, "sprite: {trimmed}").ok();
                return Exit::Refused;
            }
            writeln!(out, "{trimmed}").ok();
            Exit::Ok
        }
        Err(error) => {
            writeln!(
                errors,
                "sprite: could not ask this window to reload: {error}"
            )
            .ok();
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
        let (settings, complaints) = crate::config::Settings::load_explicit(path);
        for complaint in complaints.0 {
            writeln!(errors, "sprite: {complaint}").ok();
        }
        writeln!(out, "# {}", path.display()).ok();
        write!(out, "{}", settings.to_toml()).ok();
        return Exit::Ok;
    }

    match (environment(SOCKET_VARIABLE), environment(KEY_VARIABLE)) {
        (Some(socket), Some(key)) => {
            match exchange(&socket, &format!("{key} {PROTOCOL} config print")) {
                Ok(answer) if !answer.trim().is_empty() => {
                    let invalid = match crate::config::Settings::parse_candidate(&answer) {
                        Ok((_, complaints)) if complaints.0.is_empty() => None,
                        Ok((_, complaints)) => Some(complaints.0.join("; ")),
                        Err(error) => Some(error),
                    };
                    if let Some(error) = invalid {
                        writeln!(
                            errors,
                            "sprite: invalid configuration response: {} ({error})",
                            answer.trim()
                        )
                        .ok();
                        return Exit::Refused;
                    }
                    writeln!(out, "# in effect in this window").ok();
                    write!(out, "{}", answer.trim_end()).ok();
                    writeln!(out).ok();
                    Exit::Ok
                }
                Ok(_) => {
                    writeln!(
                        errors,
                        "sprite: the window closed the connection without answering"
                    )
                    .ok();
                    Exit::Refused
                }
                Err(error) => {
                    writeln!(errors, "sprite: could not ask this window: {error}").ok();
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
                writeln!(errors, "sprite: {complaint}").ok();
            }
            match path {
                Some(path) if path.exists() => {
                    writeln!(out, "# {} (not running inside a window)", path.display()).ok();
                }
                Some(path) => {
                    writeln!(
                        out,
                        "# {} does not exist, so these are Sprite's defaults",
                        path.display()
                    )
                    .ok();
                }
                None => {
                    writeln!(out, "# no configuration file location; Sprite's defaults").ok();
                }
            }
            write!(out, "{}", settings.to_toml()).ok();
            Exit::Ok
        }
    }
}

/// Builds the request, which carries the key and nothing the window did not ask
/// for.
///
/// Only the key is written here. Everything after it is the request grammar,
/// and `request::render` is the one place that knows how to write it — the same
/// module the window reads it back with.
fn request_line(args: &SnapshotArgs, key: &str, pane: Option<&str>) -> Result<String, String> {
    // The pane says who it is so the default scope can mean "my tab". It is
    // read from the environment rather than accepted as an argument: a caller
    // naming someone else's pane would learn nothing it could not already ask
    // for, but it would make the common request easy to get wrong.
    let from = match pane {
        Some(text) => {
            let id = text.parse().map_err(|_| {
                // The window sets this variable, so a value that is not a pane
                // id means something outside Sprite replaced it. Saying so
                // beats asking as though no pane had been named, which would
                // quietly widen a `--window` request into a success.
                format!("{PANE_VARIABLE} is set to {text:?}, which is not a pane id")
            })?;
            Some(PaneId(id))
        }
        None => {
            if matches!(args.scope, Scope::Tab { .. }) {
                return Err(format!(
                    "this session was not told which pane it is, so \"my tab\" has no meaning here \
                     (expected {PANE_VARIABLE}); try --window or --pane"
                ));
            }
            None
        }
    };

    let query = Query {
        from,
        scope: args.scope,
        // The command line can leave history unmentioned; the wire type cannot.
        // They still describe the same request, because what `HistoryLines`
        // defaults to is exactly what the window gives a request that says
        // nothing — so `render` leaves it unmentioned again.
        lines: args
            .lines
            .map_or_else(HistoryLines::default, HistoryLines::new),
        pretty: args.pretty,
    };
    Ok(format!("{key} {}", request::render(&query)))
}

/// Connects for one exchange. The only way this command reaches the window,
/// so every exchange carries the client timeout.
pub(crate) fn open_exchange(socket: &std::path::Path) -> std::io::Result<UnixStream> {
    let stream = UnixStream::connect(socket)?;
    bound_client(&stream)?;
    Ok(stream)
}

/// One request, one answer, with a bound on every step.
fn exchange(socket: &str, request: &str) -> std::io::Result<String> {
    let stream = open_exchange(std::path::Path::new(socket))?;

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
    fn an_observation_exchange_is_bounded_by_the_client_timeout() -> std::io::Result<()> {
        let timeouts = crate::timeouts::exchange_timeouts(open_exchange)?;
        let bound = Some(crate::timeouts::CLIENT_TIMEOUT);
        assert_eq!(timeouts, (bound, bound));
        Ok(())
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
                    scope: Scope::Pane(PaneId(9)),
                    ..SnapshotArgs::default()
                },
                Some("0")
            )
            .ends_with("--pane 9")
        );
        assert!(
            line(
                &SnapshotArgs {
                    scope: Scope::Tab { include_self: true },
                    ..SnapshotArgs::default()
                },
                Some("0")
            )
            .ends_with("--include-self")
        );
    }

    /// The clamp used to happen at the window, which saw whatever number was
    /// typed; it now happens here, because a `Query` holds a `HistoryLines` and
    /// that type cannot carry more than the maximum. Both lines still mean the
    /// same to the window, and this is the only place the client-side clamp can
    /// be seen going wrong.
    #[test]
    fn a_history_length_above_the_maximum_is_clamped_before_it_is_sent() {
        let request = line(
            &SnapshotArgs {
                lines: Some(HistoryLines::MAX + 1),
                ..SnapshotArgs::default()
            },
            Some("0"),
        );
        assert!(
            request.ends_with(&format!("--lines {}", HistoryLines::MAX)),
            "{request}"
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
