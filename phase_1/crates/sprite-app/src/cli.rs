//! What `sprite` does with its command line.
//!
//! Sprite has never taken arguments, so the first rule is that it still opens a
//! window when given none. Parsing is kept pure and separate from acting on it,
//! because the alternative — deciding and doing in one pass inside `main` — is
//! the part that cannot be tested without a display.

use std::ffi::{OsStr, OsString};

/// What the command line asked for.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Invocation {
    /// Open a window. What Sprite does with no arguments at all.
    Window(WindowArgs),
    /// Ask the containing window about its panes and print the answer.
    Snapshot(SnapshotArgs),
    /// Ask the containing window to re-read its configuration file.
    ConfigReload,
    Help,
    Version,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct WindowArgs {
    /// Run this instead of a login shell.
    ///
    /// Exists so Sprite can be handed a workload — the comparison against
    /// Ghostty could not be run at all while the only way to start a program
    /// was to type it.
    pub command: Option<Vec<OsString>>,
}

/// Which panes to ask about, and how to print them.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SnapshotArgs {
    pub scope: Scope,
    pub lines: Option<usize>,
    pub pretty: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Scope {
    /// This pane's own tab, without this pane. The default.
    #[default]
    Tab,
    /// This pane's own tab, including this pane.
    TabWithSelf,
    /// One named pane.
    Pane(u64),
    /// Every pane in this window.
    Window,
}

/// Why a command line could not be acted on.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UsageError(pub String);

impl std::fmt::Display for UsageError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

pub const USAGE: &str = "\
sprite — a terminal

    sprite                       open a window
    sprite -e <program> [args]   open a window running <program>
    sprite panes snapshot        print what other panes in this window show
    sprite config reload         re-read the configuration file in this window

Options for `panes snapshot`:
    --include-self               include the pane making the request
    --pane <id>                  one pane, named by the id the schema reports
    --window                     every pane in this window
    --lines <n>                  history lines per pane (0-5000, default 500)
    --pretty                     lay the JSON out for a human

The JSON goes to standard output and diagnostics to standard error. A response
that parses exits zero even when `complete` is false, because the panes that did
answer are still usable.";

/// Reads a command line, without acting on any of it.
pub fn parse_arguments<I, S>(arguments: I) -> Result<Invocation, UsageError>
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    let mut arguments = arguments.into_iter().map(Into::into).peekable();

    let Some(first) = arguments.next() else {
        // No arguments: unchanged behaviour, a window.
        return Ok(Invocation::Window(WindowArgs::default()));
    };

    match text(&first).as_deref() {
        Some("--help" | "-h") => Ok(Invocation::Help),
        Some("--version" | "-V") => Ok(Invocation::Version),
        Some("-e") => {
            let command: Vec<OsString> = arguments.collect();
            if command.is_empty() {
                return Err(UsageError("-e needs a program to run".to_owned()));
            }
            // Everything after `-e` belongs to the child, including anything
            // that looks like one of Sprite's own options: a program's `--help`
            // is its own business.
            Ok(Invocation::Window(WindowArgs {
                command: Some(command),
            }))
        }
        Some("config") => {
            let Some(sub) = arguments.next() else {
                return Err(UsageError(
                    "config needs a command, such as: reload".to_owned(),
                ));
            };
            match text(&sub).as_deref() {
                Some("reload") => match arguments.next() {
                    None => Ok(Invocation::ConfigReload),
                    Some(extra) => Err(UsageError(format!(
                        "config reload takes no arguments, but was given {}",
                        extra.to_string_lossy()
                    ))),
                },
                Some(other) => Err(UsageError(format!("unknown config command: {other}"))),
                None => Err(UsageError("arguments must be valid text".to_owned())),
            }
        }
        Some("panes") => {
            // `unwrap_or_default` here would turn "no subcommand at all" into an
            // empty one and report it as unknown, which tells a person nothing.
            let Some(sub) = arguments.next() else {
                return Err(UsageError(
                    "panes needs a command, such as: snapshot".to_owned(),
                ));
            };
            match text(&sub).as_deref() {
                Some("snapshot") => snapshot(arguments).map(Invocation::Snapshot),
                Some(other) => Err(UsageError(format!("unknown panes command: {other}"))),
                None => Err(UsageError("arguments must be valid text".to_owned())),
            }
        }
        Some(other) => Err(UsageError(format!("unknown argument: {other}"))),
        None => Err(UsageError("arguments must be valid text".to_owned())),
    }
}

fn snapshot(arguments: impl Iterator<Item = OsString>) -> Result<SnapshotArgs, UsageError> {
    let mut arguments = arguments.peekable();
    let mut parsed = SnapshotArgs::default();
    let mut scope_given = false;

    while let Some(argument) = arguments.next() {
        let Some(word) = text(&argument) else {
            return Err(UsageError("arguments must be valid text".to_owned()));
        };
        match word.as_str() {
            "--pretty" => parsed.pretty = true,
            "--include-self" => {
                // Refused rather than ignored where it would mean nothing: with
                // `--window` or `--pane` the request already covers this pane,
                // and silently accepting a flag that changes nothing teaches a
                // caller that it did something.
                if scope_given {
                    return Err(UsageError(
                        "--include-self applies to the default tab scope, not --window or --pane"
                            .to_owned(),
                    ));
                }
                parsed.scope = Scope::TabWithSelf;
            }
            "--window" => {
                if scope_given || parsed.scope == Scope::TabWithSelf {
                    return Err(UsageError("choose one of --window or --pane".to_owned()));
                }
                scope_given = true;
                parsed.scope = Scope::Window;
            }
            "--pane" => {
                if scope_given || parsed.scope == Scope::TabWithSelf {
                    return Err(UsageError("choose one of --window or --pane".to_owned()));
                }
                scope_given = true;
                parsed.scope = Scope::Pane(number(&mut arguments, "--pane")?);
            }
            "--lines" => {
                let lines = number(&mut arguments, "--lines")?;
                parsed.lines = Some(usize::try_from(lines).unwrap_or(usize::MAX));
            }
            other => return Err(UsageError(format!("unknown option: {other}"))),
        }
    }
    Ok(parsed)
}

fn number(arguments: &mut impl Iterator<Item = OsString>, option: &str) -> Result<u64, UsageError> {
    let value = arguments
        .next()
        .ok_or_else(|| UsageError(format!("{option} needs a number")))?;
    text(&value)
        .and_then(|text| text.parse().ok())
        .ok_or_else(|| UsageError(format!("{option} needs a number")))
}

fn text(value: &OsStr) -> Option<String> {
    value.to_str().map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_reload_is_its_own_invocation() {
        assert_eq!(
            parse_arguments(["config", "reload"]).expect("parse"),
            Invocation::ConfigReload
        );
    }

    #[test]
    fn config_says_what_it_needs_rather_than_guessing() {
        let missing = parse_arguments(["config"]).expect_err("no subcommand");
        assert!(missing.0.contains("reload"), "it names a way forward");

        assert!(parse_arguments(["config", "reboot"]).is_err());
        // A flag that does nothing must not look as though it did.
        assert!(parse_arguments(["config", "reload", "--now"]).is_err());
    }

    fn parsed(arguments: &[&str]) -> Invocation {
        parse_arguments(arguments.iter().map(OsString::from)).expect("a valid command line")
    }

    fn rejected(arguments: &[&str]) -> String {
        parse_arguments(arguments.iter().map(OsString::from))
            .expect_err("this command line is not valid")
            .0
    }

    /// The rule that outranks the rest: Sprite has never taken arguments, and
    /// running it with none must still open a window.
    #[test]
    fn no_arguments_still_opens_a_window() {
        let empty: [&str; 0] = [];
        assert_eq!(parsed(&empty), Invocation::Window(WindowArgs::default()));
    }

    #[test]
    fn a_program_can_be_handed_to_a_new_window() {
        assert_eq!(
            parsed(&["-e", "cat", "big-file"]),
            Invocation::Window(WindowArgs {
                command: Some(vec![OsString::from("cat"), OsString::from("big-file")]),
            })
        );
        assert_eq!(rejected(&["-e"]), "-e needs a program to run");
    }

    /// A program's own options belong to the program.
    #[test]
    fn everything_after_dash_e_belongs_to_the_child() {
        assert_eq!(
            parsed(&["-e", "sh", "-c", "--pretty --window"]),
            Invocation::Window(WindowArgs {
                command: Some(
                    ["sh", "-c", "--pretty --window"]
                        .iter()
                        .map(OsString::from)
                        .collect()
                ),
            })
        );
    }

    #[test]
    fn the_default_snapshot_scope_is_the_tab_without_this_pane() {
        assert_eq!(
            parsed(&["panes", "snapshot"]),
            Invocation::Snapshot(SnapshotArgs {
                scope: Scope::Tab,
                lines: None,
                pretty: false,
            })
        );
    }

    #[test]
    fn scope_options_are_read() {
        assert_eq!(
            parsed(&["panes", "snapshot", "--window"]),
            Invocation::Snapshot(SnapshotArgs {
                scope: Scope::Window,
                ..SnapshotArgs::default()
            })
        );
        assert_eq!(
            parsed(&["panes", "snapshot", "--pane", "7"]),
            Invocation::Snapshot(SnapshotArgs {
                scope: Scope::Pane(7),
                ..SnapshotArgs::default()
            })
        );
        assert_eq!(
            parsed(&["panes", "snapshot", "--include-self"]),
            Invocation::Snapshot(SnapshotArgs {
                scope: Scope::TabWithSelf,
                ..SnapshotArgs::default()
            })
        );
        assert_eq!(
            parsed(&["panes", "snapshot", "--lines", "12", "--pretty"]),
            Invocation::Snapshot(SnapshotArgs {
                scope: Scope::Tab,
                lines: Some(12),
                pretty: true,
            })
        );
    }

    /// The window refuses `--include-self` alongside a wider scope, so the
    /// client must not offer the combination and then report the window's
    /// refusal as though the request had been reasonable.
    #[test]
    fn contradictory_scopes_are_refused_here_rather_than_by_the_window() {
        assert!(
            rejected(&["panes", "snapshot", "--window", "--include-self"])
                .contains("--include-self")
        );
        assert!(rejected(&["panes", "snapshot", "--include-self", "--window"]).contains("one of"));
        assert!(rejected(&["panes", "snapshot", "--window", "--pane", "1"]).contains("one of"));
        assert!(rejected(&["panes", "snapshot", "--pane", "1", "--window"]).contains("one of"));
    }

    #[test]
    fn nonsense_is_refused_with_something_a_person_can_act_on() {
        assert_eq!(rejected(&["--nonsense"]), "unknown argument: --nonsense");
        assert_eq!(
            rejected(&["panes"]),
            "panes needs a command, such as: snapshot"
        );
        assert_eq!(
            rejected(&["panes", "write"]),
            "unknown panes command: write"
        );
        assert_eq!(
            rejected(&["panes", "snapshot", "--exec", "ls"]),
            "unknown option: --exec"
        );
        assert_eq!(
            rejected(&["panes", "snapshot", "--lines"]),
            "--lines needs a number"
        );
        assert_eq!(
            rejected(&["panes", "snapshot", "--pane", "everything"]),
            "--pane needs a number"
        );
    }

    #[test]
    fn help_and_version_are_recognised() {
        assert_eq!(parsed(&["--help"]), Invocation::Help);
        assert_eq!(parsed(&["-h"]), Invocation::Help);
        assert_eq!(parsed(&["--version"]), Invocation::Version);
        assert_eq!(parsed(&["-V"]), Invocation::Version);
    }
}
