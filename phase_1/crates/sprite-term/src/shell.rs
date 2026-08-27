//! Login shell resolution and the identity Sprite gives its children.
//!
//! Sprite resolves and launches a shell; it never reads, writes, or reasons
//! about the user's dotfiles. Whatever the login shell does with its own
//! profile is the user's business.

use std::env;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use crate::{SessionConfig, SessionError, TerminalSize};

/// The terminfo entry generated from the pinned Ghostty source.
const TERM: &str = "xterm-ghostty";

const TERM_PROGRAM: &str = "Sprite";

const TERM_PROGRAM_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Where the bootstrap writes the compiled terminfo database.
const TERMINFO_DIR_VAR: &str = "SPRITE_TERMINFO_DIR";

/// Where Sprite's shell-integration scripts live, if they have been installed.
///
/// Exported to children so a shell can source the matching script. Sprite does
/// not yet inject it automatically — see `integration_directory`.
const INTEGRATION_DIR_VAR: &str = "SPRITE_SHELL_INTEGRATION_DIR";

/// Tried in order when `SHELL` is unusable.
#[cfg(target_os = "macos")]
const FALLBACK_SHELLS: [&str; 2] = ["/bin/zsh", "/bin/sh"];
#[cfg(not(target_os = "macos"))]
const FALLBACK_SHELLS: [&str; 2] = ["/bin/bash", "/bin/sh"];

/// A login shell in the current directory, carrying Sprite's identity.
pub(crate) fn login_shell() -> Result<SessionConfig, SessionError> {
    Ok(configured_shell(&crate::ShellPreference::default())?.0)
}

/// The shell a preference asks for, or the login shell and a reason why not.
///
/// **A configured shell that cannot be run must not cost somebody their
/// terminal.** A typo in a program name would otherwise be a pane that fails to
/// open, which is the one outcome a settings file must never be able to
/// produce. So an unusable preference falls back to the login shell and says
/// what it did, and the fallback is complete: the arguments went with the
/// program they were written for, and applying them to a different shell would
/// be a second guess on top of a first mistake.
pub(crate) fn configured_shell(
    preference: &crate::ShellPreference,
) -> Result<(SessionConfig, Vec<String>), SessionError> {
    let mut complaints = Vec::new();

    let (program, args) = match &preference.program {
        Some(configured) if is_usable_shell(configured) => (
            configured.clone(),
            preference.args.clone().unwrap_or_default(),
        ),
        Some(configured) => {
            let program = resolve_shell(env::var_os("SHELL").as_deref())?;
            complaints.push(format!(
                "shell.program {} is not an absolute executable file; \
                 running {} instead",
                configured.display(),
                program.display()
            ));
            (program, vec![OsString::from("-l")])
        }
        None => (
            resolve_shell(env::var_os("SHELL").as_deref())?,
            vec![OsString::from("-l")],
        ),
    };

    let working_directory = match &preference.startup_directory {
        Some(directory) if directory.is_dir() => Some(directory.clone()),
        Some(directory) => {
            complaints.push(format!(
                "shell.startup_directory {} is not a directory; \
                 starting where Sprite was started",
                directory.display()
            ));
            env::current_dir().ok()
        }
        None => env::current_dir().ok(),
    };

    Ok((
        SessionConfig {
            program,
            args,
            working_directory,
            environment: identity_environment(),
            size: TerminalSize::DEFAULT,
            scrollback_bytes: crate::default_scrollback_bytes(),
            graphics: crate::GraphicsPolicy::default(),
            colors: crate::ColorDefaults::default(),
            cursor: crate::CursorDefaults::default(),
        },
        complaints,
    ))
}

/// Picks the login shell. Pure apart from the filesystem check, so the decision
/// is testable without touching the real environment.
fn resolve_shell(configured: Option<&OsStr>) -> Result<PathBuf, SessionError> {
    // A relative or non-executable SHELL is not honoured: Sprite launches an
    // absolute program or none at all.
    if let Some(value) = configured {
        let candidate = Path::new(value);
        if is_usable_shell(candidate) {
            return Ok(candidate.to_path_buf());
        }
    }

    for fallback in FALLBACK_SHELLS {
        let candidate = Path::new(fallback);
        if is_usable_shell(candidate) {
            return Ok(candidate.to_path_buf());
        }
    }

    Err(SessionError::new(
        "resolve_shell",
        "no absolute executable login shell was found",
    ))
}

fn is_usable_shell(candidate: &Path) -> bool {
    candidate.is_absolute() && is_executable_file(candidate)
}

#[cfg(unix)]
fn is_executable_file(candidate: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    std::fs::metadata(candidate)
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

/// Sprite's identity, applied after the inherited user environment so these
/// values win for the child.
fn identity_environment() -> Vec<(OsString, OsString)> {
    let mut entries = vec![
        (OsString::from("TERM"), OsString::from(TERM)),
        (OsString::from("TERM_PROGRAM"), OsString::from(TERM_PROGRAM)),
        (
            OsString::from("TERM_PROGRAM_VERSION"),
            OsString::from(TERM_PROGRAM_VERSION),
        ),
    ];

    // When the bootstrapped database is present it overrides any user TERMINFO
    // for this child, because that is the only copy guaranteed to match the
    // pinned Ghostty source. That is the development case.
    if let Some(directory) = bootstrapped_terminfo() {
        entries.push((OsString::from("TERMINFO"), directory.into_os_string()));
    } else if let Some(directory) = packaged_terminfo() {
        // The packaged case. `TERMINFO_DIRS` rather than `TERMINFO`, and with a
        // trailing empty element, which ncurses reads as "then the usual
        // places": Sprite's own entry is preferred, and every other terminal's
        // entry still resolves for whatever the child goes on to run. Setting
        // `TERMINFO` here would put a single directory in front of the whole
        // system database and break `ssh` into a machine expecting `xterm`.
        let mut value = directory.into_os_string();
        value.push(":");
        entries.push((OsString::from("TERMINFO_DIRS"), value));
    }

    // Advertised, not injected. Automatic loading differs per shell and each
    // mechanism can break a user's configuration if it is wrong: zsh needs a
    // generated ZDOTDIR that re-sources the real one, bash has no clean
    // interactive hook at all, and getting either wrong leaves someone without
    // their shell. Sprite exports the location and leaves the last step to a
    // deliberate, per-shell implementation.
    if let Some(directory) = integration_directory() {
        entries.push((
            OsString::from(INTEGRATION_DIR_VAR),
            directory.into_os_string(),
        ));
    }

    if let Some(path) = executable_directory()
        .and_then(|directory| prepend_path(&directory, env::var_os("PATH").as_deref()))
    {
        entries.push((OsString::from("PATH"), path));
    }

    entries
}

/// The installed shell-integration directory, if one is configured and present.
fn integration_directory() -> Option<PathBuf> {
    let directory = PathBuf::from(env::var_os(INTEGRATION_DIR_VAR)?);
    directory.is_dir().then_some(directory)
}

/// The database a packaged Sprite installs beside itself.
///
/// **Deliberately not `/usr/share/terminfo`.** A package may not write into the
/// shared database: `ncurses` owns the `ghostty` entry there and Arch's
/// `ghostty-terminfo` owns `xterm-ghostty`, so installing over either is a file
/// conflict that pacman refuses — correctly. Sprite keeps its own copy, which
/// is also the only one guaranteed to match the pinned Ghostty commit, and adds
/// it to the search rather than replacing anything.
///
/// Found relative to the executable rather than hard-coded, so an install under
/// `/usr`, `/usr/local`, or `/opt/sprite` all work without a build-time prefix.
fn packaged_terminfo() -> Option<PathBuf> {
    let directory = executable_directory()?
        .parent()?
        .join("share/sprite/terminfo");
    directory.is_dir().then_some(directory)
}

fn bootstrapped_terminfo() -> Option<PathBuf> {
    let directory = PathBuf::from(env::var_os(TERMINFO_DIR_VAR)?);
    directory.is_dir().then_some(directory)
}

fn executable_directory() -> Option<PathBuf> {
    Some(env::current_exe().ok()?.parent()?.to_path_buf())
}

/// Puts `directory` first in a PATH value.
///
/// Built with `split_paths`/`join_paths` rather than a literal separator, so a
/// path containing the platform separator cannot corrupt the result.
fn prepend_path(directory: &Path, current: Option<&OsStr>) -> Option<OsString> {
    let existing: Vec<PathBuf> = current
        .map(|value| env::split_paths(value).collect())
        .unwrap_or_default();

    let mut entries = Vec::with_capacity(existing.len() + 1);
    entries.push(directory.to_path_buf());
    // Keeping a duplicate would leave the old position shadowing nothing.
    entries.extend(existing.into_iter().filter(|entry| entry != directory));

    env::join_paths(entries).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_absolute_executable_shell_wins() {
        let resolved = resolve_shell(Some(OsStr::new("/bin/sh"))).expect("resolve");
        assert_eq!(resolved, PathBuf::from("/bin/sh"));
    }

    #[test]
    fn unusable_shells_fall_back() {
        for unusable in [
            // Relative, so never honoured.
            "sh",
            // Absolute but absent.
            "/nonexistent/sprite-shell",
            // Absolute and present, but a directory rather than a program.
            "/tmp",
            // Present but not executable.
            "/etc/hostname",
            // Empty.
            "",
        ] {
            let resolved = resolve_shell(Some(OsStr::new(unusable)))
                .unwrap_or_else(|error| panic!("{unusable:?} should fall back: {error}"));
            assert!(
                FALLBACK_SHELLS.contains(&resolved.to_str().expect("utf-8 fallback")),
                "{unusable:?} fell back to {resolved:?}, which is not a fallback shell"
            );
            assert!(resolved.is_absolute());
        }
    }

    #[test]
    fn an_absent_shell_variable_falls_back() {
        let resolved = resolve_shell(None).expect("resolve");
        assert!(FALLBACK_SHELLS.contains(&resolved.to_str().expect("utf-8 fallback")));
    }

    #[test]
    fn the_executable_directory_becomes_the_first_path_entry() {
        let directory = Path::new("/opt/sprite/bin");
        let joined = prepend_path(directory, Some(OsStr::new("/usr/bin:/bin"))).expect("join");

        let entries: Vec<PathBuf> = env::split_paths(&joined).collect();
        assert_eq!(entries[0], directory);
        assert_eq!(entries[1], PathBuf::from("/usr/bin"));
        assert_eq!(entries[2], PathBuf::from("/bin"));
    }

    #[test]
    fn an_existing_entry_moves_to_the_front_rather_than_duplicating() {
        let directory = Path::new("/usr/bin");
        let joined = prepend_path(directory, Some(OsStr::new("/bin:/usr/bin"))).expect("join");

        let entries: Vec<PathBuf> = env::split_paths(&joined).collect();
        assert_eq!(
            entries,
            vec![PathBuf::from("/usr/bin"), PathBuf::from("/bin")]
        );
    }

    #[test]
    fn an_absent_path_still_yields_the_executable_directory() {
        let directory = Path::new("/opt/sprite/bin");
        let joined = prepend_path(directory, None).expect("join");

        let entries: Vec<PathBuf> = env::split_paths(&joined).collect();
        assert_eq!(entries, vec![directory.to_path_buf()]);
    }

    #[test]
    fn a_configured_shell_is_run_with_its_own_arguments() {
        let preference = crate::ShellPreference {
            program: Some(PathBuf::from("/bin/sh")),
            args: Some(vec![OsString::from("-i")]),
            startup_directory: None,
        };
        let (config, complaints) = configured_shell(&preference).expect("resolve");

        assert_eq!(config.program, PathBuf::from("/bin/sh"));
        assert_eq!(config.args, vec![OsString::from("-i")]);
        assert!(complaints.is_empty());
    }

    /// The rule that keeps a typo from costing somebody their terminal.
    #[test]
    fn an_unusable_shell_falls_back_whole_and_says_so() {
        let preference = crate::ShellPreference {
            program: Some(PathBuf::from("/nonexistent/sprite-shell")),
            args: Some(vec![OsString::from("--flag-for-the-other-shell")]),
            startup_directory: None,
        };
        let (config, complaints) = configured_shell(&preference).expect("resolve");

        assert!(config.program.is_absolute());
        assert_ne!(config.program, PathBuf::from("/nonexistent/sprite-shell"));
        assert_eq!(
            config.args,
            vec![OsString::from("-l")],
            "arguments written for one shell are not passed to another"
        );
        assert_eq!(complaints.len(), 1);
        assert!(complaints[0].contains("not an absolute executable file"));
    }

    #[test]
    fn a_startup_directory_is_used_when_it_exists() {
        let preference = crate::ShellPreference {
            startup_directory: Some(PathBuf::from("/tmp")),
            ..crate::ShellPreference::default()
        };
        let (config, complaints) = configured_shell(&preference).expect("resolve");

        assert_eq!(config.working_directory, Some(PathBuf::from("/tmp")));
        assert!(complaints.is_empty());
    }

    #[test]
    fn a_missing_startup_directory_keeps_the_pane_and_says_so() {
        let preference = crate::ShellPreference {
            startup_directory: Some(PathBuf::from("/nonexistent/sprite-directory")),
            ..crate::ShellPreference::default()
        };
        let (config, complaints) = configured_shell(&preference).expect("resolve");

        assert_eq!(config.working_directory, env::current_dir().ok());
        assert_eq!(complaints.len(), 1);
        assert!(complaints[0].contains("is not a directory"));
    }

    #[test]
    fn the_login_shell_is_configured_for_interactive_use() {
        let config = login_shell().expect("resolve a login shell");

        assert!(config.program.is_absolute());
        assert_eq!(config.args, vec![OsString::from("-l")]);
        assert_eq!(config.size, TerminalSize::DEFAULT);
        assert!(config.scrollback_bytes > 0);

        let names: Vec<&OsStr> = config
            .environment
            .iter()
            .map(|(name, _)| name.as_os_str())
            .collect();
        assert!(names.contains(&OsStr::new("TERM")));
        assert!(names.contains(&OsStr::new("TERM_PROGRAM")));
        assert!(names.contains(&OsStr::new("TERM_PROGRAM_VERSION")));
    }
}
