//! Lazy, cross-process cached compiler for the standalone `sprite-child`
//! test helper binary.
//!
//! Compiling once per `target/<profile>` with a file lock and source hash stamp
//! lets every `sprite-term` and `sprite-app` test spawn a tiny, deterministic
//! `std`-only executable instead of `/bin/sh`, `/bin/cat`, `stty`, or `python3`.

use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

const TEST_CHILD_SOURCE: &str = include_str!("../tests/support/test_child_bin.rs");

/// Executable filename of the compiled test child helper.
///
/// Kept within 15 bytes: Linux reports a process's name from `comm`, which it
/// truncates to `TASK_COMM_LEN - 1`, and foreground tests compare that name
/// against this one.
pub fn test_child_program_name() -> &'static str {
    if cfg!(windows) {
        "sprite-child.exe"
    } else {
        "sprite-child"
    }
}

fn source_stamp() -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for chunk in [
        TEST_CHILD_SOURCE.as_bytes(),
        b"\0",
        std::env::consts::OS.as_bytes(),
        b"\0",
        std::env::consts::ARCH.as_bytes(),
    ] {
        for &byte in chunk {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
    }
    format!("{hash:016x}")
}

fn target_profile_dir() -> io::Result<PathBuf> {
    let exe = std::env::current_exe()?;
    let parent = exe.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "current executable has no parent directory",
        )
    })?;
    if parent.ends_with("deps") {
        parent.parent().map(Path::to_path_buf).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "deps directory has no parent directory",
            )
        })
    } else {
        Ok(parent.to_path_buf())
    }
}

/// Ensures `sprite-child` is compiled for the current target profile and
/// returns its path.
pub fn ensure_test_child_exe() -> io::Result<PathBuf> {
    let dir = target_profile_dir()?;
    let exe_path = dir.join(test_child_program_name());
    let stamp_path = dir.join("sprite-child.stamp");
    let lock_path = dir.join("sprite-child.lock");

    let lock_file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)?;
    lock_file.lock()?;

    let want_stamp = source_stamp();
    let up_to_date = exe_path.exists()
        && fs::read_to_string(&stamp_path)
            .ok()
            .is_some_and(|existing| existing.trim() == want_stamp);
    if up_to_date {
        drop(lock_file);
        return Ok(exe_path);
    }

    let pid = std::process::id();
    let src_path = dir.join(format!("sprite-child-{pid}.rs"));
    let tmp_exe = dir.join(format!(
        "sprite-child-{pid}.tmp{}",
        std::env::consts::EXE_SUFFIX
    ));

    fs::write(&src_path, TEST_CHILD_SOURCE)?;
    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| OsString::from("rustc"));
    let output = Command::new(rustc)
        .args(["--edition=2024", "-O", "-C", "debuginfo=0"])
        .arg(&src_path)
        .arg("-o")
        .arg(&tmp_exe)
        .output();
    fs::remove_file(&src_path).ok();

    let output = match output {
        Ok(output) => output,
        Err(error) => {
            fs::remove_file(&tmp_exe).ok();
            drop(lock_file);
            return Err(error);
        }
    };
    if !output.status.success() {
        fs::remove_file(&tmp_exe).ok();
        drop(lock_file);
        return Err(io::Error::other(format!(
            "failed to compile sprite-child: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }

    if let Err(error) = fs::rename(&tmp_exe, &exe_path) {
        fs::remove_file(&exe_path).ok();
        if let Err(retry_error) = fs::rename(&tmp_exe, &exe_path) {
            fs::remove_file(&tmp_exe).ok();
            drop(lock_file);
            return Err(retry_error.kind().into());
        }
        let _ = error;
    }

    fs::write(&stamp_path, format!("{want_stamp}\n"))?;
    drop(lock_file);
    Ok(exe_path)
}
