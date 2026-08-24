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
    let program = resolve_shell(env::var_os("SHELL").as_deref())?;

    Ok(SessionConfig {
        program,
        args: vec![OsString::from("-l")],
        working_directory: env::current_dir().ok(),
        environment: identity_environment(),
        size: TerminalSize::DEFAULT,
        scrollback_bytes: crate::default_scrollback_bytes(),
        graphics: crate::GraphicsPolicy::default(),
        colors: crate::ColorDefaults::default(),
    })
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
    // pinned Ghostty source. Without it, the packaged and system search paths
    // apply; Checkpoint 5 supplies the packaged path.
    if let Some(directory) = bootstrapped_terminfo() {
        entries.push((OsString::from("TERMINFO"), directory.into_os_string()));
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
