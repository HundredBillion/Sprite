//! Login shell resolution and the identity Sprite gives its children.
//!
//! Sprite resolves and launches a shell; it never reads, writes, or reasons
//! about the user's dotfiles. Whatever the login shell does with its own
//! profile is the user's business.

use std::env;
use std::ffi::{OsStr, OsString};
#[cfg(unix)]
use std::fs;
use std::path::{Path, PathBuf};

use crate::{SessionConfig, SessionError, ValidTerminalSize};

/// The terminfo entry generated from the pinned Ghostty source.
const TERM: &str = "xterm-ghostty";

const TERM_PROGRAM: &str = "Sprite";

const TERM_PROGRAM_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Where the bootstrap writes the compiled terminfo database.
#[cfg(unix)]
const TERMINFO_DIR_VAR: &str = "SPRITE_TERMINFO_DIR";

/// Where Sprite's shell-integration scripts live, if they have been installed.
///
/// Exported to children so a shell can source the matching script. Sprite does
/// not yet inject it automatically — see `integration_directory`.
const INTEGRATION_DIR_VAR: &str = "SPRITE_SHELL_INTEGRATION_DIR";

/// Tried in order when `SHELL` is unusable on Unix.
#[cfg(target_os = "macos")]
const FALLBACK_SHELLS: [&str; 2] = ["/bin/zsh", "/bin/sh"];
#[cfg(all(unix, not(target_os = "macos")))]
const FALLBACK_SHELLS: [&str; 2] = ["/bin/bash", "/bin/sh"];

fn default_shell_args() -> Vec<OsString> {
    #[cfg(unix)]
    {
        vec![OsString::from("-l")]
    }
    #[cfg(windows)]
    {
        Vec::new()
    }
}

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
            (program, default_shell_args())
        }
        None => (
            resolve_shell(env::var_os("SHELL").as_deref())?,
            default_shell_args(),
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
            size: ValidTerminalSize::DEFAULT,
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

    #[cfg(unix)]
    for fallback in FALLBACK_SHELLS {
        let candidate = Path::new(fallback);
        if is_usable_shell(candidate) {
            return Ok(candidate.to_path_buf());
        }
    }

    #[cfg(windows)]
    for candidate in windows_fallback_shells() {
        if is_usable_shell(&candidate) {
            return Ok(candidate);
        }
    }

    Err(SessionError::new(
        "resolve_shell",
        "no absolute executable login shell was found",
    ))
}

#[cfg(windows)]
fn windows_fallback_shells() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    // PowerShell 7 (`pwsh.exe`) first: on PATH, then its standard install directory.
    if let Some(path) = env::var_os("PATH") {
        for directory in env::split_paths(&path) {
            let candidate = directory.join("pwsh.exe");
            if candidate.is_absolute() {
                candidates.push(candidate);
            }
        }
    }
    if let Some(program_files) = env::var_os("ProgramFiles") {
        candidates.push(PathBuf::from(program_files).join("PowerShell\\7\\pwsh.exe"));
    }
    let system_root = env::var_os("SystemRoot")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    candidates.push(system_root.join(r"System32\WindowsPowerShell\v1.0\powershell.exe"));
    if let Some(comspec) = env::var_os("COMSPEC") {
        candidates.push(PathBuf::from(comspec));
    }
    candidates.push(system_root.join(r"System32\cmd.exe"));
    candidates
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

#[cfg(windows)]
fn is_executable_file(candidate: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(candidate) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    let Some(extension) = candidate.extension().and_then(OsStr::to_str) else {
        return false;
    };
    let lower = extension.to_ascii_lowercase();
    if matches!(lower.as_str(), "exe" | "com" | "cmd" | "bat") {
        return true;
    }
    env::var("PATHEXT").is_ok_and(|exts| {
        exts.split(';')
            .any(|ext| ext.trim_start_matches('.').eq_ignore_ascii_case(&lower))
    })
}

/// Sprite's identity, applied after the inherited user environment so these
/// values win for the child.
pub(crate) fn identity_environment() -> Vec<(OsString, OsString)> {
    let mut entries = vec![
        (OsString::from("TERM"), OsString::from(TERM)),
        (OsString::from("TERM_PROGRAM"), OsString::from(TERM_PROGRAM)),
        (OsString::from("COLORTERM"), OsString::from("truecolor")),
        (
            OsString::from("TERM_PROGRAM_VERSION"),
            OsString::from(TERM_PROGRAM_VERSION),
        ),
    ];

    // When the bootstrapped database is present it overrides any user TERMINFO
    // for this child, because that is the only copy guaranteed to match the
    // pinned Ghostty source. That is the development case.
    #[cfg(unix)]
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

    // Advertised here, and put at the front of PATH below when it names a
    // present directory: a distribution drops its launcher there and the
    // shell inside Sprite finds it first, on every platform, without touching
    // a system bin.
    if let Some(directory) = integration_directory() {
        entries.push((
            OsString::from(INTEGRATION_DIR_VAR),
            directory.into_os_string(),
        ));
    }

    if let Some(path) = prepend_path(&path_front(), env::var_os("PATH").as_deref()) {
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
/// `/usr`, `/usr/local`, `/opt/sprite`, or inside `Sprite.app` all work without
/// a build-time prefix. The executable's path is canonicalised first: on macOS
/// `current_exe` returns the path the process was started by, and a `sprite`
/// that is a symlink into a bundle must find the database in the bundle, not
/// beside the link.
#[cfg(unix)]
fn packaged_terminfo() -> Option<PathBuf> {
    let executable = env::current_exe().ok()?;
    let executable = fs::canonicalize(&executable).unwrap_or(executable);
    terminfo_candidates(&executable)
        .into_iter()
        .find(|directory| directory.is_dir())
}

/// Where a database may sit relative to an executable, in preference order.
///
/// `<prefix>/share/sprite/terminfo` for a prefix install, where the binary is
/// `<prefix>/bin/sprite`. `<Contents>/Resources/terminfo` for an application
/// bundle, where the binary is `<Contents>/MacOS/sprite`. Both are tried on
/// every platform: a directory that is absent costs one `stat`, and a `cfg`
/// would cost a code path nobody on the other platform exercises.
#[cfg(any(unix, test))]
fn terminfo_candidates(executable: &Path) -> Vec<PathBuf> {
    let Some(prefix) = executable.parent().and_then(Path::parent) else {
        return Vec::new();
    };
    vec![
        prefix.join("share/sprite/terminfo"),
        prefix.join("Resources/terminfo"),
    ]
}

#[cfg(unix)]
fn bootstrapped_terminfo() -> Option<PathBuf> {
    let directory = PathBuf::from(env::var_os(TERMINFO_DIR_VAR)?);
    directory.is_dir().then_some(directory)
}

fn executable_directory() -> Option<PathBuf> {
    Some(env::current_exe().ok()?.parent()?.to_path_buf())
}

fn same_path_entry(left: &Path, right: &Path) -> bool {
    #[cfg(windows)]
    {
        left.to_string_lossy()
            .eq_ignore_ascii_case(&right.to_string_lossy())
    }
    #[cfg(not(windows))]
    {
        left == right
    }
}

/// Puts `directories` at the front of PATH, in order, and drops them from
/// wherever else they appeared: a duplicate left behind would shadow nothing
/// and mislead anyone reading the variable.
fn prepend_path(directories: &[PathBuf], current: Option<&OsStr>) -> Option<OsString> {
    if directories.is_empty() {
        return None;
    }
    let existing: Vec<PathBuf> = current
        .map(|value| env::split_paths(value).collect())
        .unwrap_or_default();

    let mut entries: Vec<PathBuf> = directories.to_vec();
    entries.extend(
        existing
            .into_iter()
            .filter(|entry| !directories.iter().any(|dir| same_path_entry(dir, entry))),
    );

    env::join_paths(entries).ok()
}

/// What goes ahead of the inherited PATH: the shell-integration directory, so
/// a distribution's launcher is found before the program it wraps, then
/// Sprite's own, so `sprite` itself resolves inside a pane. Either may be
/// absent; an absent integration directory is simply not there.
fn path_front() -> Vec<PathBuf> {
    integration_directory()
        .into_iter()
        .chain(executable_directory())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A prefix install keeps the database beside the binary's prefix, so
    /// `/usr`, `/usr/local` and `/opt/sprite` all work without a build-time
    /// path.
    #[test]
    fn a_prefix_install_looks_under_share() {
        let candidates = terminfo_candidates(Path::new("/usr/local/bin/sprite"));
        assert_eq!(
            candidates,
            vec![
                PathBuf::from("/usr/local/share/sprite/terminfo"),
                PathBuf::from("/usr/local/Resources/terminfo"),
            ]
        );
    }

    /// Inside an application bundle the binary is at Contents/MacOS and the
    /// database, being a resource, is at Contents/Resources.
    #[test]
    fn a_bundle_looks_under_resources() {
        let candidates =
            terminfo_candidates(Path::new("/Applications/Sprite.app/Contents/MacOS/sprite"));
        assert!(candidates.contains(&PathBuf::from(
            "/Applications/Sprite.app/Contents/Resources/terminfo"
        )));
    }

    #[test]
    fn an_executable_with_no_parent_prefix_has_no_candidates() {
        assert!(terminfo_candidates(Path::new("sprite")).is_empty());
    }

    fn test_shell_path() -> PathBuf {
        #[cfg(unix)]
        {
            PathBuf::from("/bin/sh")
        }
        #[cfg(windows)]
        {
            resolve_shell(None).expect("resolve a Windows fallback shell")
        }
    }

    #[test]
    fn an_absolute_executable_shell_wins() {
        let shell = test_shell_path();
        let resolved = resolve_shell(Some(shell.as_os_str())).expect("resolve");
        assert_eq!(resolved, shell);
    }

    #[test]
    fn unusable_shells_fall_back() {
        let temp = env::temp_dir();
        let cases: Vec<OsString> = [
            // Relative, so never honoured.
            OsString::from("sh"),
            // Absolute but absent.
            temp.join("nonexistent-sprite-shell").into_os_string(),
            // Absolute and present, but a directory rather than a program.
            temp.into_os_string(),
            // Empty.
            OsString::new(),
        ]
        .into_iter()
        .chain(cfg!(unix).then(|| OsString::from("/etc/hostname")))
        .collect();

        for unusable in cases {
            let resolved = resolve_shell(Some(unusable.as_os_str()))
                .unwrap_or_else(|error| panic!("{unusable:?} should fall back: {error}"));
            #[cfg(unix)]
            assert!(
                FALLBACK_SHELLS.contains(&resolved.to_str().expect("utf-8 fallback")),
                "{unusable:?} fell back to {resolved:?}, which is not a fallback shell"
            );
            assert!(resolved.is_absolute());
            assert!(is_usable_shell(&resolved));
        }
    }

    #[test]
    fn an_absent_shell_variable_falls_back() {
        let resolved = resolve_shell(None).expect("resolve");
        #[cfg(unix)]
        assert!(FALLBACK_SHELLS.contains(&resolved.to_str().expect("utf-8 fallback")));
        assert!(resolved.is_absolute());
        assert!(is_usable_shell(&resolved));
    }

    #[test]
    fn the_executable_directory_becomes_the_first_path_entry() {
        let directory = PathBuf::from("/opt/sprite/bin");
        let existing = env::join_paths([Path::new("/usr/bin"), Path::new("/bin")]).expect("join");
        let joined = prepend_path(std::slice::from_ref(&directory), Some(existing.as_os_str()))
            .expect("join");

        let entries: Vec<PathBuf> = env::split_paths(&joined).collect();
        assert_eq!(entries[0], directory);
        assert_eq!(entries[1], PathBuf::from("/usr/bin"));
        assert_eq!(entries[2], PathBuf::from("/bin"));
    }

    #[test]
    fn an_existing_entry_moves_to_the_front_rather_than_duplicating() {
        let directory = PathBuf::from("/usr/bin");
        let existing = env::join_paths([Path::new("/bin"), Path::new("/usr/bin")]).expect("join");
        let joined = prepend_path(&[directory], Some(existing.as_os_str())).expect("join");

        let entries: Vec<PathBuf> = env::split_paths(&joined).collect();
        assert_eq!(
            entries,
            vec![PathBuf::from("/usr/bin"), PathBuf::from("/bin")]
        );
    }

    #[test]
    fn an_absent_path_still_yields_the_executable_directory() {
        let directory = PathBuf::from("/opt/sprite/bin");
        let joined = prepend_path(std::slice::from_ref(&directory), None).expect("join");

        let entries: Vec<PathBuf> = env::split_paths(&joined).collect();
        assert_eq!(entries, vec![directory]);
    }

    #[test]
    fn an_integration_directory_goes_ahead_of_sprite_s_own_and_nothing_goes_ahead_of_nothing() {
        let integration = PathBuf::from("/opt/sprite-nvim/bin");
        let own = PathBuf::from("/opt/sprite/bin");
        let existing =
            env::join_paths([Path::new("/opt/sprite/bin"), Path::new("/usr/bin")]).expect("join");
        let joined = prepend_path(
            &[integration.clone(), own.clone()],
            Some(existing.as_os_str()),
        )
        .expect("join");

        let entries: Vec<PathBuf> = env::split_paths(&joined).collect();
        assert_eq!(entries, vec![integration, own, PathBuf::from("/usr/bin")]);

        assert_eq!(prepend_path(&[], Some(OsStr::new("/usr/bin"))), None);
    }

    #[test]
    fn a_configured_shell_is_run_with_its_own_arguments() {
        let shell = test_shell_path();
        let preference = crate::ShellPreference {
            program: Some(shell.clone()),
            args: Some(vec![OsString::from("-i")]),
            startup_directory: None,
        };
        let (config, complaints) = configured_shell(&preference).expect("resolve");

        assert_eq!(config.program, shell);
        assert_eq!(config.args, vec![OsString::from("-i")]);
        assert!(complaints.is_empty());
    }

    /// The rule that keeps a typo from costing somebody their terminal.
    #[test]
    fn an_unusable_shell_falls_back_whole_and_says_so() {
        let missing = env::temp_dir().join("nonexistent-sprite-shell");
        let preference = crate::ShellPreference {
            program: Some(missing.clone()),
            args: Some(vec![OsString::from("--flag-for-the-other-shell")]),
            startup_directory: None,
        };
        let (config, complaints) = configured_shell(&preference).expect("resolve");

        assert!(config.program.is_absolute());
        assert_ne!(config.program, missing);
        assert_eq!(
            config.args,
            default_shell_args(),
            "arguments written for one shell are not passed to another"
        );
        assert_eq!(complaints.len(), 1);
        assert!(complaints[0].contains("not an absolute executable file"));
    }

    #[test]
    fn a_startup_directory_is_used_when_it_exists() {
        let directory = env::temp_dir();
        let preference = crate::ShellPreference {
            startup_directory: Some(directory.clone()),
            ..crate::ShellPreference::default()
        };
        let (config, complaints) = configured_shell(&preference).expect("resolve");

        assert_eq!(config.working_directory, Some(directory));
        assert!(complaints.is_empty());
    }

    #[test]
    fn a_missing_startup_directory_keeps_the_pane_and_says_so() {
        let preference = crate::ShellPreference {
            startup_directory: Some(env::temp_dir().join("nonexistent-sprite-directory")),
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
        assert_eq!(config.args, default_shell_args());
        assert_eq!(config.size, ValidTerminalSize::DEFAULT);
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
