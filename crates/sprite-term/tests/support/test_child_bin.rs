//! Portable `std`-only test child binary for `sprite-term` and `sprite-app` tests.
//!
//! Compiled lazily once per `target/<profile>` by `test_child_support`. Every
//! CLI flag is executed as an ordered step from left to right so tests can
//! compose PTY mode changes, stdout emissions, stdin reads, and lifecycle
//! behaviors without `/bin/sh`, `/bin/cat`, `stty`, `dd`, `od`, or `python3`.

use std::fs;
use std::io::{self, Read, Write};
#[cfg(unix)]
use std::process::Stdio;
use std::process::{self, Command};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

#[cfg(windows)]
mod win {
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicBool, Ordering};

    const STD_INPUT_HANDLE: u32 = (-10_i32) as u32;
    const STD_OUTPUT_HANDLE: u32 = (-11_i32) as u32;
    const ENABLE_PROCESSED_INPUT: u32 = 0x0001;
    const ENABLE_PROCESSED_OUTPUT: u32 = 0x0001;
    const ENABLE_VIRTUAL_TERMINAL_PROCESSING: u32 = 0x0004;
    const ENABLE_VIRTUAL_TERMINAL_INPUT: u32 = 0x0200;
    const INPUT_CLEAR_MASK: u32 = 0x001F;
    const CP_UTF8: u32 = 65001;
    const PROCESS_TERMINATE: u32 = 0x0001;
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    const SYNCHRONIZE: u32 = 0x0010_0000;
    const STILL_ACTIVE: u32 = 259;
    const WAIT_TIMEOUT: u32 = 258;

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct Coord {
        x: i16,
        y: i16,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct SmallRect {
        left: i16,
        top: i16,
        right: i16,
        bottom: i16,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct ConsoleScreenBufferInfo {
        size: Coord,
        cursor_position: Coord,
        attributes: u16,
        window: SmallRect,
        maximum_window_size: Coord,
    }

    static IGNORE_CTRL: AtomicBool = AtomicBool::new(false);

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetStdHandle(n_std_handle: u32) -> *mut c_void;
        fn GetConsoleMode(h_console_handle: *mut c_void, lp_mode: *mut u32) -> i32;
        fn SetConsoleMode(h_console_handle: *mut c_void, dw_mode: u32) -> i32;
        fn SetConsoleCP(w_code_page_id: u32) -> i32;
        fn SetConsoleOutputCP(w_code_page_id: u32) -> i32;
        fn GetConsoleScreenBufferInfo(
            h_console_output: *mut c_void,
            lp_console_screen_buffer_info: *mut ConsoleScreenBufferInfo,
        ) -> i32;
        fn SetConsoleCtrlHandler(
            handler_routine: Option<unsafe extern "system" fn(u32) -> i32>,
            add: i32,
        ) -> i32;
        fn OpenProcess(
            dw_desired_access: u32,
            b_inherit_handle: i32,
            dw_process_id: u32,
        ) -> *mut c_void;
        fn CloseHandle(h_object: *mut c_void) -> i32;
        fn GetExitCodeProcess(h_process: *mut c_void, lp_exit_code: *mut u32) -> i32;
        fn WaitForSingleObject(h_handle: *mut c_void, dw_milliseconds: u32) -> u32;
        fn TerminateProcess(h_process: *mut c_void, u_exit_code: u32) -> i32;
    }

    unsafe extern "system" fn ctrl_handler(_ctrl_type: u32) -> i32 {
        if IGNORE_CTRL.load(Ordering::Relaxed) {
            1
        } else {
            0
        }
    }

    pub fn init_utf8() {
        unsafe {
            let _ = SetConsoleCP(CP_UTF8);
            let _ = SetConsoleOutputCP(CP_UTF8);
            let handle = GetStdHandle(STD_OUTPUT_HANDLE);
            if handle.is_null() || handle == (-1_isize) as *mut c_void {
                return;
            }
            let mut mode = 0_u32;
            if GetConsoleMode(handle, &mut mode) != 0 {
                let _ = SetConsoleMode(
                    handle,
                    mode | ENABLE_PROCESSED_OUTPUT | ENABLE_VIRTUAL_TERMINAL_PROCESSING,
                );
            }
        }
    }

    pub fn set_input_mode(raw: bool) {
        unsafe {
            let handle = GetStdHandle(STD_INPUT_HANDLE);
            if handle.is_null() || handle == (-1_isize) as *mut c_void {
                return;
            }
            let mut mode = 0_u32;
            if GetConsoleMode(handle, &mut mode) != 0 {
                let mut next = (mode & !INPUT_CLEAR_MASK) | ENABLE_VIRTUAL_TERMINAL_INPUT;
                if !raw {
                    next |= ENABLE_PROCESSED_INPUT;
                }
                let _ = SetConsoleMode(handle, next);
            }
        }
    }

    pub fn pty_size() -> Option<(u16, u16)> {
        unsafe {
            let handle = GetStdHandle(STD_OUTPUT_HANDLE);
            if handle.is_null() || handle == (-1_isize) as *mut c_void {
                return None;
            }
            let mut info = ConsoleScreenBufferInfo::default();
            if GetConsoleScreenBufferInfo(handle, &mut info) == 0 {
                return None;
            }
            let rows = u16::try_from(info.window.bottom - info.window.top + 1).ok()?;
            let cols = u16::try_from(info.window.right - info.window.left + 1).ok()?;
            Some((rows, cols))
        }
    }

    pub fn ignore_signals() {
        IGNORE_CTRL.store(true, Ordering::Relaxed);
        unsafe {
            let _ = SetConsoleCtrlHandler(Some(ctrl_handler), 1);
        }
    }

    pub fn is_pid_alive(pid: u32) -> bool {
        if pid == 0 {
            return false;
        }
        unsafe {
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE, 0, pid);
            if handle.is_null() {
                return false;
            }
            let mut code = 0_u32;
            let ok = GetExitCodeProcess(handle, &mut code);
            let wait = WaitForSingleObject(handle, 0);
            let _ = CloseHandle(handle);
            ok != 0 && code == STILL_ACTIVE && wait == WAIT_TIMEOUT
        }
    }

    pub fn kill_pid(pid: u32) {
        if pid == 0 {
            return;
        }
        unsafe {
            let handle = OpenProcess(PROCESS_TERMINATE, 0, pid);
            if !handle.is_null() {
                let _ = TerminateProcess(handle, 1);
                let _ = CloseHandle(handle);
            }
        }
    }
}

#[cfg(unix)]
mod unix {
    use std::process::{Command, Stdio};

    #[repr(C)]
    struct Winsize {
        ws_row: u16,
        ws_col: u16,
        ws_xpixel: u16,
        ws_ypixel: u16,
    }

    unsafe extern "C" {
        fn signal(sig: i32, handler: usize) -> usize;
        fn ioctl(fd: i32, request: usize, ...) -> i32;
    }

    pub fn set_raw_noecho() {
        let _ = Command::new("/bin/stty")
            .args(["raw", "-echo", "min", "1", "time", "0"])
            .status();
    }

    pub fn set_cbreak_noecho() {
        let _ = Command::new("/bin/stty")
            .args(["-icanon", "-echo", "min", "1", "time", "0"])
            .status();
    }

    pub fn set_noecho() {
        let _ = Command::new("/bin/stty")
            .args(["-icanon", "-echo", "min", "1", "time", "0"])
            .status();
    }

    pub fn pty_size() -> Option<(u16, u16)> {
        #[cfg(any(
            target_os = "macos",
            target_os = "freebsd",
            target_os = "openbsd",
            target_os = "netbsd",
            target_os = "dragonfly"
        ))]
        const TIOCGWINSZ: usize = 0x4008_7468;
        #[cfg(target_os = "linux")]
        const TIOCGWINSZ: usize = 0x5413;
        #[cfg(any(
            target_os = "macos",
            target_os = "freebsd",
            target_os = "openbsd",
            target_os = "netbsd",
            target_os = "dragonfly",
            target_os = "linux"
        ))]
        {
            for fd in [0, 1] {
                let mut ws = Winsize {
                    ws_row: 0,
                    ws_col: 0,
                    ws_xpixel: 0,
                    ws_ypixel: 0,
                };
                if unsafe { ioctl(fd, TIOCGWINSZ, &mut ws) } == 0 && ws.ws_row > 0 && ws.ws_col > 0
                {
                    return Some((ws.ws_row, ws.ws_col));
                }
            }
        }
        let output = Command::new("/bin/stty")
            .arg("size")
            .stdin(std::process::Stdio::inherit())
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&output.stdout);
        let mut parts = text.split_whitespace();
        let rows = parts.next()?.parse().ok()?;
        let cols = parts.next()?.parse().ok()?;
        Some((rows, cols))
    }

    pub fn ignore_hangup() {
        const SIGHUP: i32 = 1;
        const SIG_IGN: usize = 1;
        unsafe {
            let _ = signal(SIGHUP, SIG_IGN);
        }
    }

    pub fn ignore_signals() {
        const SIGHUP: i32 = 1;
        const SIGTERM: i32 = 15;
        const SIG_IGN: usize = 1;
        unsafe {
            let _ = signal(SIGHUP, SIG_IGN);
            let _ = signal(SIGTERM, SIG_IGN);
        }
    }

    #[cfg(target_os = "linux")]
    pub fn is_pid_alive(pid: u32) -> bool {
        if pid == 0 {
            return false;
        }
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
            return false;
        };
        let Some(pos) = stat.rfind(')') else {
            return false;
        };
        let Some(state) = stat[pos + 1..].split_whitespace().next() else {
            return false;
        };
        !matches!(state, "Z" | "X" | "x")
    }

    #[cfg(not(target_os = "linux"))]
    pub fn is_pid_alive(pid: u32) -> bool {
        if pid == 0 {
            return false;
        }
        let Ok(output) = Command::new("ps")
            .args(["-o", "state=", "-p", &pid.to_string()])
            .stderr(Stdio::null())
            .output()
        else {
            return false;
        };
        let state = String::from_utf8_lossy(&output.stdout);
        let state = state.trim();
        !state.is_empty() && !state.starts_with('Z') && !state.contains('E')
    }
}

fn set_raw_noecho() {
    #[cfg(unix)]
    unix::set_raw_noecho();
    #[cfg(windows)]
    win::set_input_mode(true);
}

fn set_cbreak_noecho() {
    #[cfg(unix)]
    unix::set_cbreak_noecho();
    #[cfg(windows)]
    win::set_input_mode(true);
}

fn set_noecho() {
    #[cfg(unix)]
    unix::set_noecho();
    #[cfg(windows)]
    win::set_input_mode(false);
}

fn pty_size() -> (u16, u16) {
    #[cfg(unix)]
    {
        unix::pty_size().unwrap_or((0, 0))
    }
    #[cfg(windows)]
    {
        win::pty_size().unwrap_or((0, 0))
    }
}

fn unescape(input: &str) -> Vec<u8> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'\\' || i + 1 >= bytes.len() {
            out.push(bytes[i]);
            i += 1;
            continue;
        }
        i += 1;
        match bytes[i] {
            b'n' => {
                out.push(b'\n');
                i += 1;
            }
            b'r' => {
                out.push(b'\r');
                i += 1;
            }
            b't' => {
                out.push(b'\t');
                i += 1;
            }
            b'\\' => {
                out.push(b'\\');
                i += 1;
            }
            b'e' => {
                out.push(0x1b);
                i += 1;
            }
            b'x' => {
                if i + 2 < bytes.len()
                    && let Ok(hex) = std::str::from_utf8(&bytes[i + 1..i + 3])
                    && let Ok(value) = u8::from_str_radix(hex, 16)
                {
                    out.push(value);
                    i += 3;
                } else {
                    out.push(b'x');
                    i += 1;
                }
            }
            b'0'..=b'7' => {
                let mut val = 0_u16;
                let mut count = 0;
                while i + count < bytes.len()
                    && count < 3
                    && (b'0'..=b'7').contains(&bytes[i + count])
                {
                    val = val * 8 + u16::from(bytes[i + count] - b'0');
                    count += 1;
                }
                out.push((val & 0xff) as u8);
                i += count;
            }
            other => {
                out.push(b'\\');
                out.push(other);
                i += 1;
            }
        }
    }
    out
}

fn write_stdout(bytes: &[u8]) {
    let mut stdout = io::stdout().lock();
    let _ = stdout.write_all(bytes);
    let _ = stdout.flush();
}

/// Reads one line byte-by-byte without over-reading past `\n` or `\r`.
fn read_line_unbuffered() -> Option<String> {
    let mut stdin = io::stdin().lock();
    let mut buf = Vec::new();
    let mut byte = [0_u8; 1];
    loop {
        match stdin.read(&mut byte) {
            Ok(0) => {
                if buf.is_empty() {
                    return None;
                }
                break;
            }
            Ok(_) => {
                if byte[0] == b'\n' || byte[0] == b'\r' {
                    break;
                }
                buf.push(byte[0]);
            }
            Err(_) => return None,
        }
    }
    Some(String::from_utf8_lossy(&buf).into_owned())
}

fn read_exact_bytes(count: usize) -> Option<Vec<u8>> {
    let mut stdin = io::stdin().lock();
    let mut buf = vec![0_u8; count];
    let mut read = 0;
    while read < count {
        match stdin.read(&mut buf[read..]) {
            Ok(0) => return None,
            Ok(n) => read += n,
            Err(_) => return None,
        }
    }
    Some(buf)
}

fn read_timeout_bytes(max_bytes: usize, timeout_ms: u64) -> Vec<u8> {
    let (tx, rx) = mpsc::channel::<u8>();
    thread::spawn(move || {
        let mut stdin = io::stdin().lock();
        let mut byte = [0_u8; 1];
        for _ in 0..max_bytes {
            match stdin.read(&mut byte) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    if tx.send(byte[0]).is_err() {
                        break;
                    }
                }
            }
        }
    });
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let mut collected = Vec::new();
    while collected.len() < max_bytes {
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            break;
        };
        match rx.recv_timeout(remaining) {
            Ok(byte) => collected.push(byte),
            Err(_) => break,
        }
    }
    collected
}

fn format_hex_spaced(bytes: &[u8]) -> String {
    let mut out = String::new();
    for (index, byte) in bytes.iter().enumerate() {
        if index > 0 {
            out.push(' ');
        }
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

fn format_hex_compact(bytes: &[u8]) -> String {
    let mut out = String::new();
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

fn run_repl() {
    while let Some(line) = read_line_unbuffered() {
        if line.is_empty() {
            continue;
        }
        if line == "EXIT" {
            break;
        }
        if let Some(escaped) = line.strip_prefix("PRINT ") {
            let bytes = unescape(escaped);
            write_stdout(&bytes);
        } else if let Some(escaped) = line.strip_prefix("EMIT ") {
            let bytes = unescape(escaped);
            write_stdout(&bytes);
        } else if let Some(path) = line.strip_prefix("PRINT_FILE ")
            && let Ok(bytes) = fs::read(path)
        {
            write_stdout(&bytes);
        } else if let Some(path) = line.strip_prefix("CAT ")
            && let Ok(bytes) = fs::read(path)
        {
            write_stdout(&bytes);
        } else if let Some(n) = line.strip_prefix("MARK ") {
            write_stdout(format!("MARK{n}\n").as_bytes());
        }
    }
}

fn parse_sgr_mouse_end(slice: &[u8]) -> Option<(usize, u8)> {
    if !slice.starts_with(b"\x1b[<") {
        return None;
    }
    let mut idx = 3;
    for sep_index in 0..3 {
        let start = idx;
        while idx < slice.len() && slice[idx].is_ascii_digit() {
            idx += 1;
        }
        if idx == start || idx >= slice.len() {
            return None;
        }
        if sep_index < 2 {
            if slice[idx] != b';' {
                return None;
            }
            idx += 1;
        } else {
            let term = slice[idx];
            if term == b'M' || term == b'm' {
                return Some((idx + 1, term));
            }
            return None;
        }
    }
    None
}

fn run_mouse_reporter() {
    set_raw_noecho();
    write_stdout(b"\x1b[H\x1b[?1000h\x1b[?1006hhttps://example.com\r\nREADY\r\n");
    let mut stdin = io::stdin().lock();
    let mut pending = Vec::new();
    let mut opened = 0_usize;
    let mut chunk = [0_u8; 1024];
    while let Ok(n) = stdin.read(&mut chunk) {
        if n == 0 {
            break;
        }
        pending.extend_from_slice(&chunk[..n]);
        let mut cursor = 0;
        while cursor < pending.len() {
            let Some(pos) = pending[cursor..].windows(3).position(|w| w == b"\x1b[<") else {
                break;
            };
            let start = cursor + pos;
            if let Some((len, term)) = parse_sgr_mouse_end(&pending[start..]) {
                if term == b'm' {
                    opened += 1;
                    write_stdout(b"\r\nCHILD_OPEN:https://example.com\r\n");
                }
                pending.drain(..start + len);
                cursor = 0;
            } else {
                cursor = start + 1;
            }
        }
        if pending.windows(6).any(|w| w == b"REPORT") {
            write_stdout(format!("\r\nCHILD_REPORT:{opened}\r\n").as_bytes());
            pending.clear();
        }
    }
}

fn run_cat() {
    let mut stdin = io::stdin().lock();
    let mut buf = [0_u8; 4096];
    while let Ok(n) = stdin.read(&mut buf) {
        if n == 0 {
            break;
        }
        write_stdout(&buf[..n]);
    }
}

fn run_cat_echo() {
    while let Some(line) = read_line_unbuffered() {
        let msg = format!("{line}\n{line}\n");
        write_stdout(msg.as_bytes());
    }
}

fn run_drain_stdin() {
    let mut stdin = io::stdin().lock();
    let mut buf = [0_u8; 4096];
    while let Ok(n) = stdin.read(&mut buf) {
        if n == 0 {
            break;
        }
    }
}

fn next_arg(args: &[String], index: &mut usize, flag: &str) -> String {
    *index += 1;
    if *index >= args.len() {
        eprintln!("sprite-child: missing argument for {flag}");
        process::exit(2);
    }
    args[*index].clone()
}

fn main() {
    #[cfg(windows)]
    win::init_utf8();

    let args: Vec<String> = std::env::args().collect();
    let mut start_repl = false;
    let mut i = 1;
    while i < args.len() {
        let flag = args[i].as_str();
        match flag {
            "--raw-noecho" => set_raw_noecho(),
            "--noecho" => set_noecho(),
            "--emit" => {
                let escaped = next_arg(&args, &mut i, flag);
                write_stdout(&unescape(&escaped));
            }
            "--emit-repeat" => {
                let count: usize = next_arg(&args, &mut i, flag).parse().unwrap_or(0);
                let escaped = next_arg(&args, &mut i, flag);
                let chunk = unescape(&escaped);
                let mut stdout = io::stdout().lock();
                for _ in 0..count {
                    let _ = stdout.write_all(&chunk);
                }
                let _ = stdout.flush();
            }
            "--emit-zeros" => {
                let count: usize = next_arg(&args, &mut i, flag).parse().unwrap_or(0);
                let zeros = [0_u8; 8192];
                let mut remaining = count;
                let mut stdout = io::stdout().lock();
                while remaining > 0 {
                    let chunk = remaining.min(zeros.len());
                    let _ = stdout.write_all(&zeros[..chunk]);
                    remaining -= chunk;
                }
                let _ = stdout.flush();
            }
            "--emit-bells" => {
                let count: usize = next_arg(&args, &mut i, flag).parse().unwrap_or(0);
                let bells = [0x07_u8; 4096];
                let mut remaining = count;
                let mut stdout = io::stdout().lock();
                while remaining > 0 {
                    let chunk = remaining.min(bells.len());
                    let _ = stdout.write_all(&bells[..chunk]);
                    remaining -= chunk;
                }
                let _ = stdout.flush();
            }
            "--emit-lines" => {
                let count: usize = next_arg(&args, &mut i, flag).parse().unwrap_or(0);
                let escaped = next_arg(&args, &mut i, flag);
                let line = unescape(&escaped);
                let mut out = Vec::with_capacity(line.len() * count.min(1024));
                let mut stdout = io::stdout().lock();
                for n in 0..count {
                    out.extend_from_slice(&line);
                    if (n + 1) % 1024 == 0 {
                        let _ = stdout.write_all(&out);
                        out.clear();
                    }
                }
                if !out.is_empty() {
                    let _ = stdout.write_all(&out);
                }
                let _ = stdout.flush();
            }
            "--seq" => {
                let start: i64 = next_arg(&args, &mut i, flag).parse().unwrap_or(1);
                let end: i64 = next_arg(&args, &mut i, flag).parse().unwrap_or(0);
                let mut stdout = io::stdout().lock();
                let mut buf = String::new();
                for n in start..=end {
                    buf.push_str(&format!("{n}\n"));
                    if buf.len() >= 8192 {
                        let _ = stdout.write_all(buf.as_bytes());
                        buf.clear();
                    }
                }
                if !buf.is_empty() {
                    let _ = stdout.write_all(buf.as_bytes());
                }
                let _ = stdout.flush();
            }
            "--seq-prefix" => {
                let prefix = next_arg(&args, &mut i, flag);
                let start: i64 = next_arg(&args, &mut i, flag).parse().unwrap_or(1);
                let end: i64 = next_arg(&args, &mut i, flag).parse().unwrap_or(0);
                let mut stdout = io::stdout().lock();
                let mut buf = String::new();
                for n in start..=end {
                    buf.push_str(&format!("{prefix}{n}\n"));
                    if buf.len() >= 8192 {
                        let _ = stdout.write_all(buf.as_bytes());
                        buf.clear();
                    }
                }
                if !buf.is_empty() {
                    let _ = stdout.write_all(buf.as_bytes());
                }
                let _ = stdout.flush();
            }
            "--seq-prefix-ms" => {
                let prefix = next_arg(&args, &mut i, flag);
                let start: i64 = next_arg(&args, &mut i, flag).parse().unwrap_or(1);
                let end: i64 = next_arg(&args, &mut i, flag).parse().unwrap_or(0);
                let delay_ms: u64 = next_arg(&args, &mut i, flag).parse().unwrap_or(0);
                for n in start..=end {
                    write_stdout(format!("{prefix}{n}\n").as_bytes());
                    if delay_ms > 0 {
                        thread::sleep(Duration::from_millis(delay_ms));
                    }
                }
            }
            "--yes" => {
                let line = next_arg(&args, &mut i, flag);
                let payload = format!("{line}\n");
                let mut stdout = io::stdout().lock();
                while stdout.write_all(payload.as_bytes()).is_ok() {
                    if stdout.flush().is_err() {
                        break;
                    }
                }
            }
            "--cat" => run_cat(),
            "--cat-echo" => run_cat_echo(),
            "--cat-file" => {
                let path = next_arg(&args, &mut i, flag);
                if let Ok(bytes) = fs::read(path) {
                    write_stdout(&bytes);
                }
            }
            "--drain-stdin" => run_drain_stdin(),
            "--repl" => {
                set_cbreak_noecho();
                start_repl = true;
            }
            "--mouse-reporter" => run_mouse_reporter(),
            "--wait-line" => {
                let _ = read_line_unbuffered();
            }
            "--wait-line-prefix" => {
                let prefix = unescape(&next_arg(&args, &mut i, flag));
                let suffix = unescape(&next_arg(&args, &mut i, flag));
                if let Some(line) = read_line_unbuffered() {
                    let mut msg = prefix;
                    msg.extend_from_slice(line.as_bytes());
                    msg.extend_from_slice(&suffix);
                    write_stdout(&msg);
                }
            }
            "--read-exact-discard" => {
                let count: usize = next_arg(&args, &mut i, flag).parse().unwrap_or(0);
                let _ = read_exact_bytes(count);
            }
            "--hex-read" => {
                let count: usize = next_arg(&args, &mut i, flag).parse().unwrap_or(0);
                if let Some(bytes) = read_exact_bytes(count) {
                    let hex = format!("{}\n", format_hex_spaced(&bytes));
                    write_stdout(hex.as_bytes());
                }
            }
            "--hex-read-compact" => {
                let count: usize = next_arg(&args, &mut i, flag).parse().unwrap_or(0);
                if let Some(bytes) = read_exact_bytes(count) {
                    let hex = format_hex_compact(&bytes);
                    write_stdout(hex.as_bytes());
                }
            }
            "--hex-read-compact-loop" => {
                let count: usize = next_arg(&args, &mut i, flag).parse().unwrap_or(0);
                while let Some(bytes) = read_exact_bytes(count) {
                    let hex = format!("{}\r\n", format_hex_compact(&bytes));
                    write_stdout(hex.as_bytes());
                }
            }
            "--hex-read-timeout" => {
                let max_bytes: usize = next_arg(&args, &mut i, flag).parse().unwrap_or(0);
                let timeout_ms: u64 = next_arg(&args, &mut i, flag).parse().unwrap_or(0);
                let bytes = read_timeout_bytes(max_bytes, timeout_ms);
                if !bytes.is_empty() {
                    let hex = format!("{}\n", format_hex_spaced(&bytes));
                    write_stdout(hex.as_bytes());
                }
            }
            "--hex-read-compact-timeout-tagged" => {
                let max_bytes: usize = next_arg(&args, &mut i, flag).parse().unwrap_or(0);
                let timeout_ms: u64 = next_arg(&args, &mut i, flag).parse().unwrap_or(0);
                let prefix = unescape(&next_arg(&args, &mut i, flag));
                let suffix = unescape(&next_arg(&args, &mut i, flag));
                let bytes = read_timeout_bytes(max_bytes, timeout_ms);
                let hex = format_hex_compact(&bytes);
                let mut out = Vec::with_capacity(prefix.len() + hex.len() + suffix.len());
                out.extend_from_slice(&prefix);
                out.extend_from_slice(hex.as_bytes());
                out.extend_from_slice(&suffix);
                write_stdout(&out);
            }
            "--pty-size" => {
                let (rows, cols) = pty_size();
                write_stdout(format!("{rows} {cols}\n").as_bytes());
            }
            "--pty-size-loop" => {
                while read_line_unbuffered().is_some() {
                    let (rows, cols) = pty_size();
                    write_stdout(format!("{rows} {cols}\n").as_bytes());
                }
            }
            "--pty-size-loop-prefixed" => {
                let prefix = next_arg(&args, &mut i, flag);
                while read_line_unbuffered().is_some() {
                    let (rows, cols) = pty_size();
                    write_stdout(format!("\n{prefix}{rows} {cols}\n").as_bytes());
                }
            }
            "--echo-line-title" => {
                let prefix = next_arg(&args, &mut i, flag);
                let title = next_arg(&args, &mut i, flag);
                while let Some(line) = read_line_unbuffered() {
                    let msg = format!("{prefix}{line}\n\x1b]2;{title}\x07");
                    write_stdout(msg.as_bytes());
                }
            }
            "--on-line-emit" => {
                let escaped = next_arg(&args, &mut i, flag);
                let bytes = unescape(&escaped);
                while read_line_unbuffered().is_some() {
                    write_stdout(&bytes);
                }
            }
            "--flood-until-line" => {
                let flood_line = next_arg(&args, &mut i, flag);
                let marker_prefix = unescape(&next_arg(&args, &mut i, flag));
                let marker_suffix = unescape(&next_arg(&args, &mut i, flag));
                let stop = Arc::new(AtomicBool::new(false));
                let stop_worker = Arc::clone(&stop);
                let handle = thread::spawn(move || {
                    let payload = format!("{flood_line}\n");
                    while !stop_worker.load(Ordering::Relaxed) {
                        let mut stdout = io::stdout().lock();
                        if stdout.write_all(payload.as_bytes()).is_err() || stdout.flush().is_err()
                        {
                            break;
                        }
                        drop(stdout);
                        thread::yield_now();
                    }
                });
                let line = read_line_unbuffered().unwrap_or_default();
                stop.store(true, Ordering::Relaxed);
                let _ = handle.join();
                let mut msg = marker_prefix;
                msg.extend_from_slice(line.as_bytes());
                msg.extend_from_slice(&marker_suffix);
                write_stdout(&msg);
            }
            "--emit-rows-stepped" => {
                let count: u64 = next_arg(&args, &mut i, flag).parse().unwrap_or(0);
                let delay_ms: u64 = next_arg(&args, &mut i, flag).parse().unwrap_or(50);
                for idx in 0..count {
                    write_stdout(format!("\x1b[3;1Hrow{idx}").as_bytes());
                    thread::sleep(Duration::from_millis(delay_ms));
                }
            }
            "--wait-file" => {
                let path = next_arg(&args, &mut i, flag);
                while !std::path::Path::new(&path).exists() {
                    thread::sleep(Duration::from_millis(5));
                }
            }
            "--touch-file" => {
                let path = next_arg(&args, &mut i, flag);
                let _ = fs::write(path, b"ready");
            }
            "--write-pid-file" => {
                let path = next_arg(&args, &mut i, flag);
                let _ = fs::write(path, format!("{}\n", process::id()));
            }
            "--sleep" => {
                let secs: u64 = next_arg(&args, &mut i, flag).parse().unwrap_or(0);
                thread::sleep(Duration::from_secs(secs));
            }
            "--sleep-ms" => {
                let ms: u64 = next_arg(&args, &mut i, flag).parse().unwrap_or(0);
                thread::sleep(Duration::from_millis(ms));
            }
            "--exit" => {
                let code: i32 = next_arg(&args, &mut i, flag).parse().unwrap_or(0);
                process::exit(code);
            }
            "--emit-pid-marker" => {
                let prefix = next_arg(&args, &mut i, flag);
                write_stdout(format!("{prefix}{}\n", process::id()).as_bytes());
            }
            "--emit-pid-title" => {
                write_stdout(format!("\x1b]0;{}\x07", process::id()).as_bytes());
            }
            "--print-pwd-prompt" => {
                let cwd = std::env::current_dir()
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_default();
                write_stdout(format!("SPRITE_PROMPT:{cwd}>").as_bytes());
            }
            "--check-identity" => {
                let exedir = next_arg(&args, &mut i, flag);
                let term = std::env::var("TERM").unwrap_or_default();
                let program = std::env::var("TERM_PROGRAM").unwrap_or_default();
                let version = std::env::var("TERM_PROGRAM_VERSION").unwrap_or_default();
                let colorterm = std::env::var("COLORTERM").unwrap_or_default();
                write_stdout(format!("IDENT:{term}:{program}:{version}:{colorterm}\n").as_bytes());
                let has_exedir = std::env::var_os("PATH").is_some_and(|path| {
                    std::env::split_paths(&path).any(|entry| entry == std::path::Path::new(&exedir))
                });
                if has_exedir {
                    write_stdout(b"EXEDIR_OK\n");
                } else {
                    write_stdout(b"EXEDIR_MISSING\n");
                }
            }
            "--print-identity-short" => {
                let term = std::env::var("TERM").unwrap_or_default();
                let program = std::env::var("TERM_PROGRAM").unwrap_or_default();
                let colorterm = std::env::var("COLORTERM").unwrap_or_default();
                write_stdout(format!("IDENT:{term}:{program}:{colorterm}\n").as_bytes());
                #[cfg(unix)]
                {
                    if Command::new("infocmp")
                        .arg("xterm-ghostty")
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .status()
                        .is_ok_and(|s| s.success())
                    {
                        write_stdout(b"TERMINFO_OK\n");
                    }
                }
            }
            "--alt-screen-pager" => {
                write_stdout(b"\x1b[?1049h\x1b[H");
                let mut buf = String::new();
                for n in 1..=30 {
                    buf.push_str(&format!("{n}\n"));
                }
                write_stdout(buf.as_bytes());
            }
            "--ignore-hangup" => {
                #[cfg(unix)]
                unix::ignore_hangup();
                #[cfg(windows)]
                win::ignore_signals();
            }
            "--ignore-signals" => {
                #[cfg(unix)]
                unix::ignore_signals();
                #[cfg(windows)]
                win::ignore_signals();
            }
            "--spawn-stubborn-child" => {
                if let Ok(exe) = std::env::current_exe()
                    && let Ok(child) = Command::new(exe)
                        .args(["--ignore-signals", "--sleep", "60"])
                        .spawn()
                {
                    write_stdout(format!("PIDS:{}:{}\n", process::id(), child.id()).as_bytes());
                    thread::sleep(Duration::from_secs(30));
                }
            }
            "--spawn-retained-descendant" => {
                let pid_file = next_arg(&args, &mut i, flag);
                let ready_file = next_arg(&args, &mut i, flag);
                let mode = next_arg(&args, &mut i, flag);
                let tail_args: &[&str] = if mode == "write" {
                    &[
                        "--ignore-hangup",
                        "--touch-file",
                        &ready_file,
                        "--write-loop",
                    ]
                } else {
                    &[
                        "--ignore-hangup",
                        "--touch-file",
                        &ready_file,
                        "--sleep",
                        "30",
                    ]
                };
                if let Ok(exe) = std::env::current_exe()
                    && let Ok(child) = Command::new(exe).args(tail_args).spawn()
                {
                    while !std::path::Path::new(&ready_file).exists() {
                        thread::sleep(Duration::from_millis(5));
                    }
                    let _ = fs::write(&pid_file, format!("{}", child.id()));
                    process::exit(7);
                }
            }
            "--write-loop" => {
                let block = b"descendant output\r\n".repeat(32);
                let mut stdout = io::stdout().lock();
                while stdout.write_all(&block).is_ok() {
                    if stdout.flush().is_err() {
                        break;
                    }
                }
            }
            "--spawn-eof-waiter" => {
                let pid_file = next_arg(&args, &mut i, flag);
                let ready_file = next_arg(&args, &mut i, flag);
                let done_file = next_arg(&args, &mut i, flag);
                if let Ok(exe) = std::env::current_exe()
                    && let Ok(child) = Command::new(exe)
                        .args([
                            "--ignore-hangup",
                            "--touch-file",
                            &ready_file,
                            "--wait-eof-write",
                            &done_file,
                        ])
                        .spawn()
                {
                    while !std::path::Path::new(&ready_file).exists() {
                        thread::sleep(Duration::from_millis(5));
                    }
                    let _ = fs::write(&pid_file, format!("{}", child.id()));
                    process::exit(0);
                }
            }
            "--wait-eof-write" => {
                let done_file = next_arg(&args, &mut i, flag);
                let mut stdin = io::stdin().lock();
                let mut buf = [0_u8; 64];
                while let Ok(n) = stdin.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                }
                let _ = fs::write(done_file, b"hung up");
            }
            "--pid-alive" => {
                let pid: u32 = next_arg(&args, &mut i, flag).parse().unwrap_or(0);
                #[cfg(windows)]
                {
                    process::exit(if win::is_pid_alive(pid) { 0 } else { 1 });
                }
                #[cfg(unix)]
                {
                    process::exit(if unix::is_pid_alive(pid) { 0 } else { 1 });
                }
            }
            "--kill-pid" => {
                let pid: u32 = next_arg(&args, &mut i, flag).parse().unwrap_or(0);
                if pid > 0 {
                    #[cfg(windows)]
                    win::kill_pid(pid);
                    #[cfg(unix)]
                    {
                        let _ = Command::new("kill")
                            .args(["-KILL", &pid.to_string()])
                            .status();
                    }
                }
            }
            other => {
                eprintln!("sprite-child: unknown flag {other}");
                process::exit(2);
            }
        }
        i += 1;
    }
    if start_repl {
        run_repl();
    }
}
