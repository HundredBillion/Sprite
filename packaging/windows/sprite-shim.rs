//! Console shim (`sprite.com`) for `sprite.exe`.
//!
//! Release builds of `sprite.exe` use `windows_subsystem = "windows"` so Start
//! Menu and Explorer launches do not open a console window. Because Windows
//! `%PATHEXT%` resolves `.COM` before `.EXE`, placing this small console
//! binary beside `sprite.exe` keeps `sprite panes ...`, `sprite surface ...`,
//! and `sprite --version` attached to the caller's terminal with inherited
//! stdin/stdout/stderr and exit codes.

use std::process::{Command, ExitCode};

fn main() -> ExitCode {
    let exe = match std::env::current_exe() {
        Ok(path) => path,
        Err(error) => {
            eprintln!("sprite: failed to locate current executable: {error}");
            return ExitCode::FAILURE;
        }
    };
    let target = exe.with_file_name("sprite.exe");
    let status = match Command::new(&target)
        .args(std::env::args_os().skip(1))
        .status()
    {
        Ok(status) => status,
        Err(error) => {
            eprintln!("sprite: failed to launch {}: {error}", target.display());
            return ExitCode::FAILURE;
        }
    };
    if status.success() {
        return ExitCode::SUCCESS;
    }
    match status.code().and_then(|code| u8::try_from(code).ok()) {
        Some(code) if code != 0 => ExitCode::from(code),
        Some(_) | None => ExitCode::FAILURE,
    }
}
