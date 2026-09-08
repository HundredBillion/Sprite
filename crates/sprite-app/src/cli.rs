//! What `sprite` does with its command line.
//!
//! Sprite has never taken arguments, so the first rule is that it still opens a
//! window when given none. Parsing is kept pure and separate from acting on it,
//! because the alternative — deciding and doing in one pass inside `main` — is
//! the part that cannot be tested without a display.

use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

use crate::observation::request::Scope;
use crate::pane_tree::PaneId;
use crate::surface::channel::{
    DEFAULT_DOCK_SIZE, MAX_DOCK_SIZE, MIN_DOCK_SIZE, Position as SurfacePosition,
    Side as SurfaceSide,
};

/// What the command line asked for.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Invocation {
    /// Open a window. What Sprite does with no arguments at all.
    Window(WindowArgs),
    /// Ask the containing window about its panes and print the answer.
    Snapshot(SnapshotArgs),
    /// Ask the containing window to re-read its configuration file.
    ConfigReload,
    /// Print the configuration that is actually in effect.
    ConfigPrint(ConfigPrintArgs),
    SurfaceOpen(SurfaceOpenArgs),
    /// Hand the keyboard to this pane's terminal, or to the Surface named.
    SurfaceFocus(Option<u64>),
    TokenRegister(TokenRegisterArgs),
    Help,
    Version,
}

/// Which configuration `config print` should describe.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ConfigPrintArgs {
    /// Read this file instead of asking the window, or of discovery.
    pub path: Option<PathBuf>,
}

/// Where to open a Surface, and whether it takes the keyboard.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SurfaceOpenArgs {
    pub position: SurfacePosition,
    pub side: SurfaceSide,
    /// A dock's width in logical pixels.
    pub size: u32,
    pub focus: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TokenRegisterArgs {
    pub name: String,
    pub default: sprite_term::Rgb,
    pub description: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct WindowArgs {
    /// Run this instead of a login shell.
    ///
    /// Exists so Sprite can be handed a workload — the comparison against
    /// Ghostty could not be run at all while the only way to start a program
    /// was to type it.
    pub command: Option<Vec<OsString>>,
    /// Read settings from here instead of the usual place.
    ///
    /// Explicit, so it wins over discovery — and the window keeps it, so a
    /// later `sprite config reload` re-reads *this* file rather than quietly
    /// switching to the one discovery would have found.
    pub config: Option<PathBuf>,
}

/// Which panes to ask about, and how to print them.
///
/// The scope is the same type the wire parser produces, so the command line and
/// the window cannot disagree about what a scope is or which combinations of
/// flags exist.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SnapshotArgs {
    pub scope: Scope,
    pub lines: Option<usize>,
    pub pretty: bool,
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
    sprite --config <path>       open a window reading settings from <path>
    sprite panes snapshot        print what other panes in this window show
    sprite config reload         re-read the configuration file in this window
    sprite config print          print the settings that are actually in effect
    sprite surface open …        draw native UI in this pane from a description on stdin
    sprite surface focus [ID]    hand the keyboard to this pane's terminal, or to Surface ID
    sprite token register <name> <#rrggbb> [description]
                                 add a colour token the theme can override

Options for `config print`:
    --config <path>              describe this file instead of asking the window

Options for `panes snapshot`:
    --include-self               include the pane making the request
    --pane <id>                  one pane, named by the id the schema reports
    --window                     every pane in this window
    --lines <n>                  history lines per pane (0-5000, default 500)
    --pretty                     lay the JSON out for a human

Options for `surface open` (one position is required):
    --fill                       in place of the grid
    --dock left|right            in a strip beside the grid
    --overlay                    floating over the grid
    --size <px>                  a dock's width (64-4096, default 240)
    --no-focus                   open without taking the keyboard

For `surface open`, standard input carries the description as JSON, then any
number of further JSON documents: a description replaces the Surface;
{\"type\":\"focus\"} hands the keyboard back; {\"type\":\"close\"} closes it, as
does closing standard input. Events arrive on standard output, one JSON line
each.

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
        Some("-e" | "--config") => {
            // Both open a window, and `--config` may precede `-e`, so window
            // options are read by one loop rather than by matching on the first
            // word alone.
            let mut rest = vec![first];
            rest.extend(arguments);
            window(rest).map(Invocation::Window)
        }
        Some("config") => {
            let Some(sub) = arguments.next() else {
                return Err(UsageError(
                    "config needs a command, such as: reload".to_owned(),
                ));
            };
            match text(&sub).as_deref() {
                Some("print") => config_print(arguments).map(Invocation::ConfigPrint),
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
        Some("surface") => match arguments.next().as_deref().and_then(text).as_deref() {
            Some("open") => Ok(Invocation::SurfaceOpen(surface_open(arguments)?)),
            Some("focus") => match arguments.next() {
                None => Ok(Invocation::SurfaceFocus(None)),
                Some(id) => match id.to_string_lossy().parse::<u64>() {
                    Ok(id) => Ok(Invocation::SurfaceFocus(Some(id))),
                    Err(_) => Err(UsageError(format!(
                        "surface focus takes a Surface id, not {}",
                        id.to_string_lossy()
                    ))),
                },
            },
            Some(other) => Err(UsageError(format!("unknown surface command: {other}"))),
            None => Err(UsageError(
                "surface needs a command, such as: open".to_owned(),
            )),
        },
        Some("token") => match arguments.next().as_deref().and_then(text).as_deref() {
            Some("register") => Ok(Invocation::TokenRegister(token_register(arguments)?)),
            Some(other) => Err(UsageError(format!("unknown token command: {other}"))),
            None => Err(UsageError(
                "token needs a command, such as: register".to_owned(),
            )),
        },
        Some(other) => Err(UsageError(format!("unknown argument: {other}"))),
        None => Err(UsageError("arguments must be valid text".to_owned())),
    }
}

/// Reads the options that open a window.
fn window(arguments: Vec<OsString>) -> Result<WindowArgs, UsageError> {
    let mut arguments = arguments.into_iter();
    let mut parsed = WindowArgs::default();

    while let Some(argument) = arguments.next() {
        let Some(word) = text(&argument) else {
            return Err(UsageError("arguments must be valid text".to_owned()));
        };
        match word.as_str() {
            "--config" => {
                let path = arguments
                    .next()
                    .ok_or_else(|| UsageError("--config needs a path".to_owned()))?;
                if path.is_empty() {
                    return Err(UsageError("--config needs a path".to_owned()));
                }
                parsed.config = Some(PathBuf::from(path));
            }
            "-e" => {
                let command: Vec<OsString> = arguments.collect();
                if command.is_empty() {
                    return Err(UsageError("-e needs a program to run".to_owned()));
                }
                // Everything after `-e` belongs to the child, including
                // anything that looks like one of Sprite's own options: a
                // program's `--help` is its own business.
                parsed.command = Some(command);
                return Ok(parsed);
            }
            other => return Err(UsageError(format!("unknown argument: {other}"))),
        }
    }
    Ok(parsed)
}

fn config_print(arguments: impl Iterator<Item = OsString>) -> Result<ConfigPrintArgs, UsageError> {
    let mut arguments = arguments;
    let mut parsed = ConfigPrintArgs::default();

    while let Some(argument) = arguments.next() {
        match text(&argument).as_deref() {
            Some("--config") => {
                let path = arguments
                    .next()
                    .ok_or_else(|| UsageError("--config needs a path".to_owned()))?;
                if path.is_empty() {
                    return Err(UsageError("--config needs a path".to_owned()));
                }
                parsed.path = Some(PathBuf::from(path));
            }
            Some(other) => return Err(UsageError(format!("unknown option: {other}"))),
            None => return Err(UsageError("arguments must be valid text".to_owned())),
        }
    }
    Ok(parsed)
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
                parsed.scope = Scope::Tab { include_self: true };
            }
            "--window" => {
                if scope_given || matches!(parsed.scope, Scope::Tab { include_self: true }) {
                    return Err(UsageError("choose one of --window or --pane".to_owned()));
                }
                scope_given = true;
                parsed.scope = Scope::Window;
            }
            "--pane" => {
                if scope_given || matches!(parsed.scope, Scope::Tab { include_self: true }) {
                    return Err(UsageError("choose one of --window or --pane".to_owned()));
                }
                scope_given = true;
                parsed.scope = Scope::Pane(PaneId(number(&mut arguments, "--pane")?));
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

fn surface_open(
    mut arguments: impl Iterator<Item = OsString>,
) -> Result<SurfaceOpenArgs, UsageError> {
    let mut position = None;
    let mut side = SurfaceSide::Left;
    let mut size = DEFAULT_DOCK_SIZE as u32;
    let mut focus = true;
    while let Some(argument) = arguments.next() {
        match text(&argument).as_deref() {
            Some("--fill") => set_position(&mut position, SurfacePosition::Fill)?,
            Some("--overlay") => set_position(&mut position, SurfacePosition::Overlay)?,
            Some("--dock") => {
                set_position(&mut position, SurfacePosition::Dock)?;
                let name = arguments
                    .next()
                    .and_then(|value| text(&value))
                    .ok_or_else(|| UsageError("--dock needs a side: left or right".to_owned()))?;
                side = SurfaceSide::parse(&name)
                    .ok_or_else(|| UsageError(format!("--dock takes left or right, not {name}")))?;
            }
            Some("--size") => {
                let pixels = number(&mut arguments, "--size")?;
                let allowed = (MIN_DOCK_SIZE as u64)..=(MAX_DOCK_SIZE as u64);
                if !allowed.contains(&pixels) {
                    return Err(UsageError(format!(
                        "--size is between {} and {} pixels",
                        MIN_DOCK_SIZE as u64, MAX_DOCK_SIZE as u64
                    )));
                }
                size = pixels as u32;
            }
            Some("--no-focus") => focus = false,
            _ => {
                return Err(UsageError(format!(
                    "unknown option: {}",
                    argument.to_string_lossy()
                )));
            }
        }
    }
    let position = position.ok_or_else(|| {
        UsageError(
            "surface open needs a position: --fill, --dock left|right, or --overlay".to_owned(),
        )
    })?;
    Ok(SurfaceOpenArgs {
        position,
        side,
        size,
        focus,
    })
}

fn set_position(
    slot: &mut Option<SurfacePosition>,
    position: SurfacePosition,
) -> Result<(), UsageError> {
    if slot.is_some() {
        return Err(UsageError("surface open takes one position".to_owned()));
    }
    *slot = Some(position);
    Ok(())
}

fn token_register(
    mut arguments: impl Iterator<Item = OsString>,
) -> Result<TokenRegisterArgs, UsageError> {
    let name = arguments
        .next()
        .and_then(|value| text(&value))
        .ok_or_else(|| {
            UsageError(
                "token register needs a name, a #rrggbb default, and a description".to_owned(),
            )
        })?;
    let default = arguments
        .next()
        .and_then(|value| text(&value))
        .and_then(|value| crate::config::Colors::parse_hex(&value))
        .ok_or_else(|| UsageError(format!("token register {name} needs a #rrggbb default")))?;
    let description = arguments
        .filter_map(|value| text(&value))
        .collect::<Vec<String>>()
        .join(" ");
    Ok(TokenRegisterArgs {
        name,
        default,
        description,
    })
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
    fn an_explicit_configuration_file_reaches_the_window() {
        assert_eq!(
            parsed(&["--config", "/tmp/other.toml"]),
            Invocation::Window(WindowArgs {
                command: None,
                config: Some(PathBuf::from("/tmp/other.toml")),
            })
        );
        // Both, and in this order: `-e` swallows the rest of the line, so
        // anything of Sprite's own must come before it.
        assert_eq!(
            parsed(&["--config", "/tmp/other.toml", "-e", "cat"]),
            Invocation::Window(WindowArgs {
                command: Some(vec![OsString::from("cat")]),
                config: Some(PathBuf::from("/tmp/other.toml")),
            })
        );
        assert_eq!(rejected(&["--config"]), "--config needs a path");
    }

    #[test]
    fn config_print_can_be_pointed_at_a_file() {
        assert_eq!(
            parsed(&["config", "print"]),
            Invocation::ConfigPrint(ConfigPrintArgs { path: None })
        );
        assert_eq!(
            parsed(&["config", "print", "--config", "/tmp/other.toml"]),
            Invocation::ConfigPrint(ConfigPrintArgs {
                path: Some(PathBuf::from("/tmp/other.toml")),
            })
        );
        assert_eq!(
            rejected(&["config", "print", "--pretty"]),
            "unknown option: --pretty"
        );
    }

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
                config: None,
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
                config: None,
            })
        );
    }

    #[test]
    fn the_default_snapshot_scope_is_the_tab_without_this_pane() {
        assert_eq!(
            parsed(&["panes", "snapshot"]),
            Invocation::Snapshot(SnapshotArgs {
                scope: Scope::Tab {
                    include_self: false,
                },
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
                scope: Scope::Pane(PaneId(7)),
                ..SnapshotArgs::default()
            })
        );
        assert_eq!(
            parsed(&["panes", "snapshot", "--include-self"]),
            Invocation::Snapshot(SnapshotArgs {
                scope: Scope::Tab { include_self: true },
                ..SnapshotArgs::default()
            })
        );
        assert_eq!(
            parsed(&["panes", "snapshot", "--lines", "12", "--pretty"]),
            Invocation::Snapshot(SnapshotArgs {
                scope: Scope::Tab {
                    include_self: false,
                },
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

    #[test]
    fn a_surface_open_names_its_position_and_options() {
        assert_eq!(
            parsed(&[
                "surface",
                "open",
                "--dock",
                "right",
                "--size",
                "300",
                "--no-focus"
            ]),
            Invocation::SurfaceOpen(SurfaceOpenArgs {
                position: SurfacePosition::Dock,
                side: SurfaceSide::Right,
                size: 300,
                focus: false,
            })
        );
        assert_eq!(
            parsed(&["surface", "open", "--fill"]),
            Invocation::SurfaceOpen(SurfaceOpenArgs {
                position: SurfacePosition::Fill,
                side: SurfaceSide::Left,
                size: 240,
                focus: true,
            })
        );
        assert_eq!(
            parsed(&["surface", "focus"]),
            Invocation::SurfaceFocus(None)
        );
        assert_eq!(
            parsed(&["surface", "focus", "7"]),
            Invocation::SurfaceFocus(Some(7))
        );
        assert!(parse_arguments(["surface", "focus", "blob"].iter().map(OsString::from)).is_err());
    }

    #[test]
    fn a_surface_open_needs_exactly_one_position_and_a_sane_size() {
        assert!(rejected(&["surface", "open"]).contains("position"));
        assert!(rejected(&["surface", "open", "--fill", "--overlay"]).contains("one position"));
        assert!(rejected(&["surface", "open", "--dock"]).contains("left or right"));
        assert!(rejected(&["surface", "open", "--dock", "top"]).contains("left or right"));
        assert!(rejected(&["surface", "open", "--fill", "--size", "10"]).contains("64"));
        assert!(rejected(&["surface", "open", "--fill", "--sparkle"]).contains("unknown option"));
        assert!(rejected(&["surface", "focus", "now"]).contains("Surface id"));
        assert!(rejected(&["surface"]).contains("needs a command"));
        assert!(rejected(&["surface", "close"]).contains("unknown surface command"));
    }

    #[test]
    fn a_token_registration_takes_a_name_a_colour_and_the_rest_as_its_description() {
        assert_eq!(
            parsed(&[
                "token",
                "register",
                "scm.added",
                "#40a02b",
                "Added",
                "lines"
            ]),
            Invocation::TokenRegister(TokenRegisterArgs {
                name: "scm.added".to_owned(),
                default: sprite_term::Rgb {
                    r: 0x40,
                    g: 0xa0,
                    b: 0x2b
                },
                description: "Added lines".to_owned(),
            })
        );
        assert!(rejected(&["token", "register"]).contains("needs a name"));
        assert!(rejected(&["token", "register", "scm.added", "green"]).contains("#rrggbb"));
        assert!(rejected(&["token", "list"]).contains("unknown token command"));
    }
}
