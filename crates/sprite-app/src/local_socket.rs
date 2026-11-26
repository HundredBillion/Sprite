//! Private local transport shared by independent protocol adapters.
//!
//! Uses Unix domain sockets on Linux and macOS, and Win32 Named Pipes
//! (`\\.\pipe\sprite-...`) on Windows so native tools and `libuv` clients
//! (`sprite.nvim`) speak the same local stream protocol without TCP ports.

#[cfg(unix)]
use nix::errno::Errno;
#[cfg(unix)]
use nix::poll::{PollFd, PollFlags, PollTimeout, poll};
use std::collections::HashMap;
use std::fs;
#[cfg(unix)]
use std::fs::File;
#[cfg(any(unix, test))]
use std::io::Read;
use std::io::{self, BufRead, BufReader, Write};
use std::net::Shutdown;
#[cfg(unix)]
use std::os::fd::AsFd;
#[cfg(unix)]
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
#[cfg(unix)]
use std::os::unix::net::{UnixDatagram, UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::timeouts::WRITE_TIMEOUT;

/// Platform-native private local byte stream.
#[cfg(unix)]
pub(crate) type LocalStream = UnixStream;

#[cfg(windows)]
pub(crate) use windows_ipc::{
    LocalListener, LocalStream, WakeReceiver, WakeSender, create_private_dir, create_private_file,
    wake_pair,
};

#[cfg(unix)]
pub(crate) use unix_wake::{WakeReceiver, WakeSender, create_private_dir, wake_pair};

#[cfg(test)]
pub(crate) trait LocalStreamExt {
    fn drain_available(&mut self) -> io::Result<String>;
}

#[cfg(all(test, unix))]
impl LocalStreamExt for LocalStream {
    fn drain_available(&mut self) -> io::Result<String> {
        self.set_nonblocking(true)?;
        let mut bytes = Vec::new();
        let mut scratch = [0_u8; 4096];
        loop {
            match self.read(&mut scratch) {
                Ok(0) => break,
                Ok(read) => bytes.extend_from_slice(&scratch[..read]),
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            }
        }
        String::from_utf8(bytes)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "non-utf8 stream data"))
    }
}

#[cfg(unix)]
mod unix_wake {
    use std::fs;
    use std::io::{self, Read, Write};
    use std::os::fd::{AsFd, BorrowedFd};
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    use std::os::unix::net::UnixStream;
    use std::path::Path;

    pub(crate) struct WakeSender(UnixStream);
    pub(crate) struct WakeReceiver(UnixStream);

    pub(crate) fn wake_pair() -> io::Result<(WakeSender, WakeReceiver)> {
        let (send, recv) = UnixStream::pair()?;
        send.set_nonblocking(true)?;
        recv.set_nonblocking(true)?;
        Ok((WakeSender(send), WakeReceiver(recv)))
    }

    impl WakeSender {
        pub(crate) fn wake(&self) {
            (&self.0).write_all(&[1]).ok();
        }
    }

    impl WakeReceiver {
        pub(crate) fn drain(&self) {
            let mut scratch = [0_u8; 64];
            while matches!((&self.0).read(&mut scratch), Ok(read) if read > 0) {}
        }

        pub(crate) fn as_fd(&self) -> BorrowedFd<'_> {
            self.0.as_fd()
        }
    }

    pub(crate) fn create_private_dir(directory: &Path) -> io::Result<()> {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(directory)?;
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700))
    }
}

#[cfg(windows)]
mod windows_ipc {
    use std::ffi::OsStr;
    use std::fs::{self, File};
    use std::io::{self, Read, Write};
    use std::net::Shutdown;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use std::path::{Path, PathBuf};
    #[cfg(test)]
    use std::sync::atomic::AtomicUsize;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use windows::Win32::Foundation::{
        ERROR_BROKEN_PIPE, ERROR_IO_PENDING, ERROR_NO_DATA, ERROR_OPERATION_ABORTED,
        ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED, GENERIC_READ, GENERIC_WRITE, HANDLE, HLOCAL,
        INVALID_HANDLE_VALUE, LocalFree, WAIT_EVENT, WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT,
    };
    use windows::Win32::Security::Authorization::{
        ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
        SDDL_REVISION_1,
    };
    use windows::Win32::Security::{
        GetTokenInformation, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER,
        TokenUser,
    };
    use windows::Win32::Storage::FileSystem::{
        CreateDirectoryW, CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_FLAG_FIRST_PIPE_INSTANCE,
        FILE_FLAG_OVERLAPPED, FILE_SHARE_DELETE, FILE_SHARE_NONE, FILE_SHARE_READ,
        FILE_SHARE_WRITE, LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY, LockFileEx,
        OPEN_ALWAYS, OPEN_EXISTING, PIPE_ACCESS_DUPLEX, ReadFile, WriteFile,
    };
    use windows::Win32::System::IO::{
        CancelIoEx, GetOverlappedResult, OVERLAPPED, OVERLAPPED_0, OVERLAPPED_0_0,
    };
    use windows::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE,
        PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
        PeekNamedPipe, WaitNamedPipeW,
    };
    use windows::Win32::System::Threading::{
        CreateEventW, GetCurrentProcess, INFINITE, OpenProcessToken, ResetEvent, SetEvent,
        WaitForMultipleObjects, WaitForSingleObject,
    };
    use windows::core::{PCWSTR, PWSTR};

    const PIPE_PREFIX: &str = r"\\.\pipe\";
    const PIPE_BUFFER_BYTES: u32 = 256 * 1024;
    const CONNECT_WAIT_MS: u32 = 5_000;

    fn wide_null(value: &OsStr) -> Vec<u16> {
        value.encode_wide().chain(std::iter::once(0)).collect()
    }

    pub(crate) fn is_pipe_path(path: &Path) -> bool {
        path.to_string_lossy().starts_with(PIPE_PREFIX)
    }

    struct OwnerSecurityDescriptor {
        raw: PSECURITY_DESCRIPTOR,
    }

    impl Drop for OwnerSecurityDescriptor {
        fn drop(&mut self) {
            if !self.raw.0.is_null() {
                // SAFETY: `self.raw` was allocated by `ConvertStringSecurityDescriptorToSecurityDescriptorW`.
                unsafe {
                    let _ = LocalFree(Some(HLOCAL(self.raw.0)));
                }
            }
        }
    }

    fn current_user_sid() -> io::Result<String> {
        let mut token = HANDLE::default();
        // SAFETY: queries the current process token with read-only `TOKEN_QUERY`.
        unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) }
            .map_err(|_| io::Error::last_os_error())?;
        // SAFETY: `token` is a fresh handle owned here.
        let token = unsafe { OwnedHandle::from_raw_handle(token.0) };

        let mut needed = 0_u32;
        // SAFETY: queries the required buffer size; `ERROR_INSUFFICIENT_BUFFER` is expected.
        unsafe {
            GetTokenInformation(
                HANDLE(token.as_raw_handle()),
                TokenUser,
                None,
                0,
                &mut needed,
            )
            .ok();
        }
        if needed == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut buffer = vec![0_u8; needed as usize];
        // SAFETY: `buffer` has `needed` bytes as reported by `GetTokenInformation`.
        unsafe {
            GetTokenInformation(
                HANDLE(token.as_raw_handle()),
                TokenUser,
                Some(buffer.as_mut_ptr().cast()),
                needed,
                &mut needed,
            )
        }
        .map_err(|_| io::Error::last_os_error())?;
        // SAFETY: `TokenUser` populates a `TOKEN_USER` struct at the start of `buffer`.
        let user = unsafe { &*(buffer.as_ptr().cast::<TOKEN_USER>()) };
        let mut sid_wide = PWSTR::null();
        // SAFETY: `user.User.Sid` points into `buffer` which remains live.
        unsafe { ConvertSidToStringSidW(user.User.Sid, &mut sid_wide) }
            .map_err(|_| io::Error::last_os_error())?;
        let sid = unsafe { sid_wide.to_string() }.map_err(io::Error::other);
        // SAFETY: `sid_wide` was allocated by `ConvertSidToStringSidW`.
        unsafe {
            let _ = LocalFree(Some(HLOCAL(sid_wide.0.cast())));
        }
        sid
    }

    fn owner_only_descriptor(inherit: bool) -> io::Result<OwnerSecurityDescriptor> {
        let sid = current_user_sid()?;
        let sddl = if inherit {
            format!("D:P(A;OICI;GA;;;{sid})")
        } else {
            format!("D:P(A;;GA;;;{sid})")
        };
        let wide = wide_null(OsStr::new(&sddl));
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        // SAFETY: `wide` is NUL-terminated UTF-16 and `descriptor` receives a `LocalAlloc` pointer.
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                PCWSTR(wide.as_ptr()),
                SDDL_REVISION_1,
                &mut descriptor,
                None,
            )
        }
        .map_err(|_| io::Error::last_os_error())?;
        Ok(OwnerSecurityDescriptor { raw: descriptor })
    }

    pub(crate) fn create_private_dir(directory: &Path) -> io::Result<()> {
        if directory.is_dir() {
            return Ok(());
        }
        if let Some(parent) = directory.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent)?;
        }
        let sd = owner_only_descriptor(true)?;
        let sa = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: sd.raw.0,
            bInheritHandle: false.into(),
        };
        let wide = wide_null(directory.as_os_str());
        // SAFETY: `wide` is NUL-terminated and `sa` points to `sd` which outlives the call.
        match unsafe { CreateDirectoryW(PCWSTR(wide.as_ptr()), Some(&sa)) } {
            Ok(()) => Ok(()),
            Err(_) if directory.is_dir() => Ok(()),
            Err(_) => Err(io::Error::last_os_error()),
        }
    }

    pub(crate) fn create_private_file(path: &Path, truncate: bool) -> io::Result<File> {
        let sd = owner_only_descriptor(false)?;
        let sa = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: sd.raw.0,
            bInheritHandle: false.into(),
        };
        let wide = wide_null(path.as_os_str());
        // SAFETY: `wide` is NUL-terminated UTF-16 and `sa` points to `sd` which outlives the call.
        let raw = unsafe {
            CreateFileW(
                PCWSTR(wide.as_ptr()),
                GENERIC_READ.0 | GENERIC_WRITE.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                Some(&sa),
                OPEN_ALWAYS,
                FILE_ATTRIBUTE_NORMAL,
                None,
            )
        }
        .map_err(|_| io::Error::last_os_error())?;
        // SAFETY: `raw` is a fresh file handle owned exclusively here.
        let file = File::from(unsafe { OwnedHandle::from_raw_handle(raw.0) });
        if truncate {
            file.set_len(0)?;
        }
        Ok(file)
    }

    fn create_event(manual_reset: bool) -> io::Result<OwnedHandle> {
        // SAFETY: anonymous Win32 event with default security attributes.
        let raw = unsafe { CreateEventW(None, manual_reset, false, PCWSTR::null()) }
            .map_err(|_| io::Error::last_os_error())?;
        // SAFETY: `raw` is a fresh event handle owned exclusively here.
        Ok(unsafe { OwnedHandle::from_raw_handle(raw.0) })
    }

    pub(crate) struct WakeSender(Arc<OwnedHandle>);
    pub(crate) struct WakeReceiver(Arc<OwnedHandle>);

    pub(crate) fn wake_pair() -> io::Result<(WakeSender, WakeReceiver)> {
        let event = Arc::new(create_event(true)?);
        Ok((WakeSender(Arc::clone(&event)), WakeReceiver(event)))
    }

    impl WakeSender {
        pub(crate) fn wake(&self) {
            // SAFETY: `self.0` holds a live event handle.
            unsafe {
                SetEvent(HANDLE(self.0.as_raw_handle())).ok();
            }
        }
    }

    impl WakeReceiver {
        pub(crate) fn drain(&self) {
            // SAFETY: `self.0` holds a live event handle.
            unsafe {
                ResetEvent(HANDLE(self.0.as_raw_handle())).ok();
            }
        }

        pub(crate) fn wait_timeout(&self, limit: Duration) -> bool {
            let millis = limit.as_millis().min(u128::from(u32::MAX - 1)) as u32;
            // SAFETY: `self.0` holds a live event handle.
            unsafe { WaitForSingleObject(HANDLE(self.0.as_raw_handle()), millis) == WAIT_OBJECT_0 }
        }

        pub(crate) fn raw_handle(&self) -> HANDLE {
            HANDLE(self.0.as_raw_handle())
        }
    }

    #[derive(Debug)]
    struct WindowsPipe {
        handle: OwnedHandle,
        read_timeout: Mutex<Option<Duration>>,
        write_timeout: Mutex<Option<Duration>>,
        read_shutdown: AtomicBool,
        write_shutdown: AtomicBool,
    }

    #[derive(Clone, Debug)]
    pub(crate) struct LocalStream {
        inner: Arc<WindowsPipe>,
    }

    fn resolve_pipe_target(path: &Path) -> io::Result<String> {
        if is_pipe_path(path) {
            return Ok(path.to_string_lossy().into_owned());
        }
        let target = fs::read_to_string(path)?;
        let target = target.trim();
        if target.starts_with(PIPE_PREFIX) {
            Ok(target.to_owned())
        } else {
            Err(io::Error::new(
                io::ErrorKind::ConnectionRefused,
                "invalid local pipe marker",
            ))
        }
    }

    impl LocalStream {
        fn from_handle(handle: OwnedHandle) -> Self {
            Self {
                inner: Arc::new(WindowsPipe {
                    handle,
                    read_timeout: Mutex::new(None),
                    write_timeout: Mutex::new(None),
                    read_shutdown: AtomicBool::new(false),
                    write_shutdown: AtomicBool::new(false),
                }),
            }
        }

        pub(crate) fn connect(path: impl AsRef<Path>) -> io::Result<Self> {
            let pipe_name = resolve_pipe_target(path.as_ref())?;
            let wide = wide_null(OsStr::new(&pipe_name));
            loop {
                // SAFETY: `wide` is NUL-terminated UTF-16 naming a Win32 Named Pipe.
                match unsafe {
                    CreateFileW(
                        PCWSTR(wide.as_ptr()),
                        GENERIC_READ.0 | GENERIC_WRITE.0,
                        FILE_SHARE_NONE,
                        None,
                        OPEN_EXISTING,
                        FILE_FLAG_OVERLAPPED,
                        None,
                    )
                } {
                    Ok(raw) => {
                        // SAFETY: `raw` is a fresh pipe handle transferred here.
                        let handle = unsafe { OwnedHandle::from_raw_handle(raw.0) };
                        return Ok(Self::from_handle(handle));
                    }
                    Err(_) => {
                        let error = io::Error::last_os_error();
                        if error.raw_os_error() == Some(ERROR_PIPE_BUSY.0 as i32) {
                            // SAFETY: waits briefly for a free pipe instance.
                            if unsafe { WaitNamedPipeW(PCWSTR(wide.as_ptr()), CONNECT_WAIT_MS) }
                                .as_bool()
                            {
                                continue;
                            }
                            return Err(io::Error::new(io::ErrorKind::TimedOut, "named pipe busy"));
                        }
                        if error.kind() == io::ErrorKind::NotFound {
                            return Err(io::Error::new(io::ErrorKind::ConnectionRefused, error));
                        }
                        return Err(error);
                    }
                }
            }
        }

        pub(crate) fn try_clone(&self) -> io::Result<Self> {
            Ok(self.clone())
        }

        pub(crate) fn set_read_timeout(&self, duration: Option<Duration>) -> io::Result<()> {
            if let Some(d) = duration
                && d.is_zero()
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "cannot set a zero read timeout",
                ));
            }
            *crate::locked::locked(&self.inner.read_timeout) = duration;
            Ok(())
        }

        pub(crate) fn set_write_timeout(&self, duration: Option<Duration>) -> io::Result<()> {
            if let Some(d) = duration
                && d.is_zero()
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "cannot set a zero write timeout",
                ));
            }
            *crate::locked::locked(&self.inner.write_timeout) = duration;
            Ok(())
        }

        #[cfg(test)]
        pub(crate) fn read_timeout(&self) -> io::Result<Option<Duration>> {
            Ok(*crate::locked::locked(&self.inner.read_timeout))
        }

        #[cfg(test)]
        pub(crate) fn write_timeout(&self) -> io::Result<Option<Duration>> {
            Ok(*crate::locked::locked(&self.inner.write_timeout))
        }

        pub(crate) fn shutdown(&self, how: Shutdown) -> io::Result<()> {
            match how {
                Shutdown::Read => {
                    self.inner.read_shutdown.store(true, Ordering::SeqCst);
                }
                Shutdown::Write => {
                    self.inner.write_shutdown.store(true, Ordering::SeqCst);
                }
                Shutdown::Both => {
                    self.inner.read_shutdown.store(true, Ordering::SeqCst);
                    self.inner.write_shutdown.store(true, Ordering::SeqCst);
                    let handle = HANDLE(self.inner.handle.as_raw_handle());
                    // SAFETY: cancels any pending overlapped read/write on this live handle.
                    unsafe {
                        CancelIoEx(handle, None).ok();
                    }
                }
            }
            Ok(())
        }

        pub(crate) fn wait_readable(&self, limit: Duration) -> io::Result<bool> {
            if self.peek_readable_or_closed()? {
                return Ok(true);
            }
            if limit.is_zero() {
                return Ok(false);
            }
            let deadline = Instant::now() + limit;
            let idle = create_event(true)?;
            let raw = HANDLE(idle.as_raw_handle());
            loop {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Ok(false);
                }
                let slice = remaining.min(Duration::from_millis(4));
                let millis = slice.as_millis().clamp(1, u128::from(u32::MAX - 1)) as u32;
                // SAFETY: `idle` is a private manual-reset event that stays unsignaled for this slice.
                let waited = unsafe { WaitForSingleObject(raw, millis) };
                if waited == WAIT_FAILED {
                    return Err(io::Error::last_os_error());
                }
                if self.peek_readable_or_closed()? {
                    return Ok(true);
                }
            }
        }

        /// Non-blocking readiness or disconnect probe using `PeekNamedPipe`.
        pub(crate) fn peek_readable_or_closed(&self) -> io::Result<bool> {
            if self.inner.read_shutdown.load(Ordering::SeqCst) {
                return Ok(true);
            }
            let mut available = 0_u32;
            let handle = HANDLE(self.inner.handle.as_raw_handle());
            // SAFETY: `PeekNamedPipe` queries buffered byte count without consuming data.
            match unsafe { PeekNamedPipe(handle, None, 0, None, Some(&mut available), None) } {
                Ok(()) => Ok(available > 0),
                Err(_) => {
                    let error = io::Error::last_os_error();
                    if matches!(
                        error.raw_os_error(),
                        Some(code)
                            if code == ERROR_BROKEN_PIPE.0 as i32
                                || code == ERROR_NO_DATA.0 as i32
                                || code == ERROR_OPERATION_ABORTED.0 as i32
                    ) {
                        Ok(true)
                    } else {
                        Err(error)
                    }
                }
            }
        }

        #[cfg(test)]
        pub(crate) fn pair() -> io::Result<(Self, Self)> {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let name = format!(
                "{PIPE_PREFIX}sprite-pair-{:x}-{:x}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::SeqCst)
            );
            let server_handle = create_pipe_instance(&name, true)?;
            let event = create_event(true)?;
            let mut overlapped = OVERLAPPED {
                hEvent: HANDLE(event.as_raw_handle()),
                ..Default::default()
            };
            let raw = HANDLE(server_handle.as_raw_handle());
            // SAFETY: initiates overlapped accept before the client connects.
            let pending = match unsafe { ConnectNamedPipe(raw, Some(&mut overlapped)) } {
                Ok(()) => false,
                Err(_) => {
                    let code = io::Error::last_os_error().raw_os_error();
                    if code == Some(ERROR_PIPE_CONNECTED.0 as i32) {
                        false
                    } else if code == Some(ERROR_IO_PENDING.0 as i32) {
                        true
                    } else {
                        return Err(io::Error::last_os_error());
                    }
                }
            };
            let client = Self::connect(Path::new(&name))?;
            if pending {
                let mut transferred = 0_u32;
                // SAFETY: `overlapped` and `event` remain live until completion returns.
                unsafe { GetOverlappedResult(raw, &overlapped, &mut transferred, true) }
                    .map_err(|_| io::Error::last_os_error())?;
            }
            Ok((Self::from_handle(server_handle), client))
        }

        fn read_impl(&self, buf: &mut [u8]) -> io::Result<usize> {
            if buf.is_empty() || self.inner.read_shutdown.load(Ordering::SeqCst) {
                return Ok(0);
            }
            let event = create_event(true)?;
            let mut overlapped = OVERLAPPED {
                hEvent: HANDLE(event.as_raw_handle()),
                ..Default::default()
            };
            let handle = HANDLE(self.inner.handle.as_raw_handle());
            let timeout = *crate::locked::locked(&self.inner.read_timeout);
            // SAFETY: `buf` and `overlapped` stay live and unmoved until `GetOverlappedResult` completes.
            let started = unsafe { ReadFile(handle, Some(buf), None, Some(&mut overlapped)) };
            let pending = match started {
                Ok(()) => false,
                Err(_) => {
                    let error = io::Error::last_os_error();
                    match error.raw_os_error() {
                        Some(code) if code == ERROR_IO_PENDING.0 as i32 => true,
                        Some(code)
                            if code == ERROR_BROKEN_PIPE.0 as i32
                                || code == ERROR_NO_DATA.0 as i32 =>
                        {
                            return Ok(0);
                        }
                        Some(code) if code == ERROR_OPERATION_ABORTED.0 as i32 => {
                            return Ok(0);
                        }
                        _ => return Err(error),
                    }
                }
            };
            if pending {
                if self.inner.read_shutdown.load(Ordering::SeqCst) {
                    // SAFETY: `overlapped` and `buf` stay live; cancel this exact IRP if still
                    // pending and wait for kernel completion before dropping `buf`.
                    let completed = unsafe {
                        CancelIoEx(handle, Some(&overlapped)).ok();
                        let mut transferred = 0_u32;
                        GetOverlappedResult(handle, &overlapped, &mut transferred, true)
                            .map(|()| transferred)
                    };
                    return match completed {
                        Ok(transferred) => Ok(transferred as usize),
                        Err(_) => {
                            let error = io::Error::last_os_error();
                            match error.raw_os_error() {
                                Some(code)
                                    if code == ERROR_OPERATION_ABORTED.0 as i32
                                        || code == ERROR_BROKEN_PIPE.0 as i32
                                        || code == ERROR_NO_DATA.0 as i32 =>
                                {
                                    Ok(0)
                                }
                                _ => Err(error),
                            }
                        }
                    };
                }
                let wait_ms = timeout.map_or(INFINITE, |d| {
                    d.as_millis().clamp(1, u128::from(u32::MAX - 1)) as u32
                });
                // SAFETY: waits on the event associated with `overlapped`.
                let waited = unsafe { WaitForSingleObject(HANDLE(event.as_raw_handle()), wait_ms) };
                if waited == WAIT_TIMEOUT {
                    // SAFETY: cancel this exact overlapped request and wait for kernel completion before dropping `buf`.
                    unsafe {
                        CancelIoEx(handle, Some(&overlapped)).ok();
                    }
                    let mut transferred = 0_u32;
                    let completed =
                        unsafe { GetOverlappedResult(handle, &overlapped, &mut transferred, true) };
                    return match completed {
                        Ok(()) => Ok(transferred as usize),
                        Err(_) => {
                            let error = io::Error::last_os_error();
                            match error.raw_os_error() {
                                Some(code) if code == ERROR_OPERATION_ABORTED.0 as i32 => Err(
                                    io::Error::new(io::ErrorKind::TimedOut, "pipe read timed out"),
                                ),
                                Some(code)
                                    if code == ERROR_BROKEN_PIPE.0 as i32
                                        || code == ERROR_NO_DATA.0 as i32 =>
                                {
                                    Ok(0)
                                }
                                _ => Err(error),
                            }
                        }
                    };
                }
                if waited != WAIT_OBJECT_0 {
                    let error = if waited == WAIT_FAILED {
                        io::Error::last_os_error()
                    } else {
                        io::Error::other("unexpected pipe read wait result")
                    };
                    // SAFETY: cancel pending overlapped read and drain completion before dropping `buf`.
                    unsafe {
                        CancelIoEx(handle, Some(&overlapped)).ok();
                        let mut transferred = 0_u32;
                        GetOverlappedResult(handle, &overlapped, &mut transferred, true).ok();
                    }
                    return Err(error);
                }
            }
            let mut transferred = 0_u32;
            // SAFETY: `overlapped` is signaled or already complete.
            match unsafe { GetOverlappedResult(handle, &overlapped, &mut transferred, true) } {
                Ok(()) => Ok(transferred as usize),
                Err(_) => {
                    let error = io::Error::last_os_error();
                    match error.raw_os_error() {
                        Some(code)
                            if code == ERROR_BROKEN_PIPE.0 as i32
                                || code == ERROR_NO_DATA.0 as i32 =>
                        {
                            Ok(0)
                        }
                        Some(code) if code == ERROR_OPERATION_ABORTED.0 as i32 => {
                            if self.inner.read_shutdown.load(Ordering::SeqCst) {
                                Ok(0)
                            } else {
                                Err(io::Error::new(io::ErrorKind::Interrupted, error))
                            }
                        }
                        _ => Err(error),
                    }
                }
            }
        }

        fn write_impl(&self, buf: &[u8]) -> io::Result<usize> {
            if buf.is_empty() {
                return Ok(0);
            }
            if self.inner.write_shutdown.load(Ordering::SeqCst) {
                return Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "write direction shut down",
                ));
            }
            let event = create_event(true)?;
            let mut overlapped = OVERLAPPED {
                hEvent: HANDLE(event.as_raw_handle()),
                ..Default::default()
            };
            let handle = HANDLE(self.inner.handle.as_raw_handle());
            let timeout = *crate::locked::locked(&self.inner.write_timeout);
            // SAFETY: `buf` and `overlapped` stay live until `GetOverlappedResult` completes.
            let started = unsafe { WriteFile(handle, Some(buf), None, Some(&mut overlapped)) };
            let pending = match started {
                Ok(()) => false,
                Err(_) => {
                    let error = io::Error::last_os_error();
                    match error.raw_os_error() {
                        Some(code) if code == ERROR_IO_PENDING.0 as i32 => true,
                        Some(code)
                            if code == ERROR_BROKEN_PIPE.0 as i32
                                || code == ERROR_NO_DATA.0 as i32
                                || (code == ERROR_OPERATION_ABORTED.0 as i32
                                    && self.inner.write_shutdown.load(Ordering::SeqCst)) =>
                        {
                            return Err(io::Error::new(io::ErrorKind::BrokenPipe, error));
                        }
                        _ => return Err(error),
                    }
                }
            };
            if pending {
                if self.inner.write_shutdown.load(Ordering::SeqCst) {
                    // SAFETY: `overlapped` and `buf` stay live; cancel this exact IRP if still
                    // pending and wait for kernel completion before returning.
                    let completed = unsafe {
                        CancelIoEx(handle, Some(&overlapped)).ok();
                        let mut transferred = 0_u32;
                        GetOverlappedResult(handle, &overlapped, &mut transferred, true)
                            .map(|()| transferred)
                    };
                    return match completed {
                        Ok(transferred) => Ok(transferred as usize),
                        Err(_) => {
                            let error = io::Error::last_os_error();
                            if matches!(
                                error.raw_os_error(),
                                Some(code)
                                    if code == ERROR_OPERATION_ABORTED.0 as i32
                                        || code == ERROR_BROKEN_PIPE.0 as i32
                                        || code == ERROR_NO_DATA.0 as i32
                            ) {
                                Err(io::Error::new(io::ErrorKind::BrokenPipe, error))
                            } else {
                                Err(error)
                            }
                        }
                    };
                }
                let wait_ms = timeout.map_or(INFINITE, |d| {
                    d.as_millis().clamp(1, u128::from(u32::MAX - 1)) as u32
                });
                // SAFETY: waits on the event associated with `overlapped`.
                let waited = unsafe { WaitForSingleObject(HANDLE(event.as_raw_handle()), wait_ms) };
                if waited == WAIT_TIMEOUT {
                    // SAFETY: cancel and await completion before dropping `buf`.
                    unsafe {
                        CancelIoEx(handle, Some(&overlapped)).ok();
                    }
                    let mut transferred = 0_u32;
                    let completed =
                        unsafe { GetOverlappedResult(handle, &overlapped, &mut transferred, true) };
                    return match completed {
                        Ok(()) => Ok(transferred as usize),
                        Err(_) => {
                            let error = io::Error::last_os_error();
                            match error.raw_os_error() {
                                Some(code) if code == ERROR_OPERATION_ABORTED.0 as i32 => Err(
                                    io::Error::new(io::ErrorKind::TimedOut, "pipe write timed out"),
                                ),
                                Some(code)
                                    if code == ERROR_BROKEN_PIPE.0 as i32
                                        || code == ERROR_NO_DATA.0 as i32 =>
                                {
                                    Err(io::Error::new(io::ErrorKind::BrokenPipe, error))
                                }
                                _ => Err(error),
                            }
                        }
                    };
                }
                if waited != WAIT_OBJECT_0 {
                    let error = if waited == WAIT_FAILED {
                        io::Error::last_os_error()
                    } else {
                        io::Error::other("unexpected pipe write wait result")
                    };
                    // SAFETY: cancel pending overlapped write and drain completion before dropping `buf`.
                    unsafe {
                        CancelIoEx(handle, Some(&overlapped)).ok();
                        let mut transferred = 0_u32;
                        GetOverlappedResult(handle, &overlapped, &mut transferred, true).ok();
                    }
                    return Err(error);
                }
            }
            let mut transferred = 0_u32;
            // SAFETY: `overlapped` is signaled or already complete.
            match unsafe { GetOverlappedResult(handle, &overlapped, &mut transferred, true) } {
                Ok(()) => Ok(transferred as usize),
                Err(_) => {
                    let error = io::Error::last_os_error();
                    if matches!(
                        error.raw_os_error(),
                        Some(code)
                            if code == ERROR_BROKEN_PIPE.0 as i32
                                || code == ERROR_NO_DATA.0 as i32
                                || (code == ERROR_OPERATION_ABORTED.0 as i32
                                    && self.inner.write_shutdown.load(Ordering::SeqCst))
                    ) {
                        Err(io::Error::new(io::ErrorKind::BrokenPipe, error))
                    } else {
                        Err(error)
                    }
                }
            }
        }
    }

    #[cfg(test)]
    impl super::LocalStreamExt for LocalStream {
        /// Non-blocking drain of all currently buffered bytes on this Named Pipe.
        fn drain_available(&mut self) -> io::Result<String> {
            let mut bytes = Vec::new();
            let handle = HANDLE(self.inner.handle.as_raw_handle());
            let mut scratch = [0_u8; 4096];
            loop {
                let mut available = 0_u32;
                // SAFETY: `PeekNamedPipe` queries buffered byte count without consuming data.
                match unsafe { PeekNamedPipe(handle, None, 0, None, Some(&mut available), None) } {
                    Ok(()) => {
                        if available == 0 {
                            break;
                        }
                        let want = (available as usize).min(scratch.len());
                        let read = self.read(&mut scratch[..want])?;
                        if read == 0 {
                            break;
                        }
                        bytes.extend_from_slice(&scratch[..read]);
                    }
                    Err(_) => {
                        let error = io::Error::last_os_error();
                        if matches!(
                            error.raw_os_error(),
                            Some(code)
                                if code == ERROR_BROKEN_PIPE.0 as i32
                                    || code == ERROR_NO_DATA.0 as i32
                                    || code == ERROR_OPERATION_ABORTED.0 as i32
                        ) {
                            break;
                        }
                        return Err(error);
                    }
                }
            }
            String::from_utf8(bytes)
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "non-utf8 stream data"))
        }
    }

    impl Read for LocalStream {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.read_impl(buf)
        }
    }

    impl Read for &LocalStream {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.read_impl(buf)
        }
    }

    impl Write for LocalStream {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.write_impl(buf)
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl Write for &LocalStream {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.write_impl(buf)
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    pub(crate) fn create_pipe_instance(pipe_name: &str, first: bool) -> io::Result<OwnedHandle> {
        let sd = owner_only_descriptor(false)?;
        let sa = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: sd.raw.0,
            bInheritHandle: false.into(),
        };
        let wide = wide_null(OsStr::new(pipe_name));
        let mut open_mode = PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED;
        if first {
            open_mode |= FILE_FLAG_FIRST_PIPE_INSTANCE;
        }
        let pipe_mode =
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS;
        // SAFETY: `wide` is NUL-terminated and `sa` points to `sd` which outlives the call.
        let raw = unsafe {
            CreateNamedPipeW(
                PCWSTR(wide.as_ptr()),
                open_mode,
                pipe_mode,
                PIPE_UNLIMITED_INSTANCES,
                PIPE_BUFFER_BYTES,
                PIPE_BUFFER_BYTES,
                0,
                Some(&sa),
            )
        };
        if raw == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `raw` is a fresh pipe server handle owned exclusively here.
        Ok(unsafe { OwnedHandle::from_raw_handle(raw.0) })
    }

    fn try_lock_sentinel(file: &File) -> io::Result<()> {
        let mut overlapped = OVERLAPPED {
            Anonymous: OVERLAPPED_0 {
                Anonymous: OVERLAPPED_0_0 {
                    Offset: 0,
                    OffsetHigh: 1,
                },
            },
            ..Default::default()
        };
        // SAFETY: `file` owns a live handle and `overlapped` specifies byte offset 0x1_0000_0000.
        unsafe {
            LockFileEx(
                HANDLE(file.as_raw_handle()),
                LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY,
                None,
                1,
                0,
                &mut overlapped,
            )
        }
        .map_err(|_| io::Error::last_os_error())
    }

    /// A Windows Named Pipe listener, optionally backed by a locked discovery marker file.
    pub(crate) struct LocalListener {
        pipe_name: String,
        next_instance: Mutex<Option<OwnedHandle>>,
        _marker_lock: Option<File>,
        marker_path: Option<PathBuf>,
    }

    impl LocalListener {
        pub(crate) fn bind(socket: &Path) -> io::Result<Self> {
            if is_pipe_path(socket) {
                let pipe_name = socket.to_string_lossy().into_owned();
                let instance = create_pipe_instance(&pipe_name, true)?;
                return Ok(Self {
                    pipe_name,
                    next_instance: Mutex::new(Some(instance)),
                    _marker_lock: None,
                    marker_path: None,
                });
            }
            if let Some(parent) = socket.parent() {
                create_private_dir(parent)?;
            }
            let pipe_name = format!(
                "{PIPE_PREFIX}sprite-{}",
                super::SocketKey::generate()?.to_hex()
            );
            let instance = create_pipe_instance(&pipe_name, true)?;
            let mut marker = create_private_file(socket, false)?;
            try_lock_sentinel(&marker)?;
            marker.set_len(0)?;
            marker.write_all(pipe_name.as_bytes())?;
            marker.flush()?;
            Ok(Self {
                pipe_name,
                next_instance: Mutex::new(Some(instance)),
                _marker_lock: Some(marker),
                marker_path: Some(socket.to_path_buf()),
            })
        }

        #[cfg(test)]
        pub(crate) fn pipe_name(&self) -> &str {
            &self.pipe_name
        }

        pub(crate) fn accept_cancellable(
            &self,
            cancel: &WakeReceiver,
        ) -> io::Result<Option<LocalStream>> {
            self.accept_inner(cancel, INFINITE)
        }

        pub(crate) fn accept_timeout(
            &self,
            cancel: &WakeReceiver,
            limit: Duration,
        ) -> io::Result<Option<LocalStream>> {
            let millis = limit.as_millis().min(u128::from(u32::MAX - 1)) as u32;
            self.accept_inner(cancel, millis)
        }

        fn accept_inner(
            &self,
            cancel: &WakeReceiver,
            timeout_ms: u32,
        ) -> io::Result<Option<LocalStream>> {
            let existing = crate::locked::locked(&self.next_instance).take();
            let instance = match existing {
                Some(existing) => existing,
                None => create_pipe_instance(&self.pipe_name, false)?,
            };
            let connect_event = create_event(true)?;
            let mut overlapped = OVERLAPPED {
                hEvent: HANDLE(connect_event.as_raw_handle()),
                ..Default::default()
            };
            let raw = HANDLE(instance.as_raw_handle());
            // SAFETY: initiates overlapped `ConnectNamedPipe` on `raw`.
            let pending = match unsafe { ConnectNamedPipe(raw, Some(&mut overlapped)) } {
                Ok(()) => false,
                Err(_) => {
                    let error = io::Error::last_os_error();
                    match error.raw_os_error() {
                        Some(code) if code == ERROR_PIPE_CONNECTED.0 as i32 => false,
                        Some(code) if code == ERROR_IO_PENDING.0 as i32 => true,
                        _ => return Err(error),
                    }
                }
            };
            if pending {
                let handles = [cancel.raw_handle(), HANDLE(connect_event.as_raw_handle())];
                // SAFETY: waits until `cancel` is signaled, `connect_event` fires, or `timeout_ms` elapses.
                let waited = unsafe { WaitForMultipleObjects(&handles, false, timeout_ms) };
                if waited == WAIT_OBJECT_0 || waited == WAIT_TIMEOUT {
                    // SAFETY: cancel pending connect and wait for completion before dropping `overlapped`.
                    unsafe {
                        CancelIoEx(raw, Some(&overlapped)).ok();
                    }
                    let mut transferred = 0_u32;
                    let completed =
                        unsafe { GetOverlappedResult(raw, &overlapped, &mut transferred, true) };
                    if waited == WAIT_TIMEOUT && completed.is_ok() {
                        if let Ok(replacement) = create_pipe_instance(&self.pipe_name, false) {
                            *crate::locked::locked(&self.next_instance) = Some(replacement);
                        }
                        return Ok(Some(LocalStream::from_handle(instance)));
                    }
                    if waited == WAIT_TIMEOUT {
                        let mut guard = crate::locked::locked(&self.next_instance);
                        if guard.is_none() {
                            *guard = Some(instance);
                        }
                    } else {
                        unsafe {
                            DisconnectNamedPipe(raw).ok();
                        }
                    }
                    return Ok(None);
                }
                if waited != WAIT_EVENT(WAIT_OBJECT_0.0 + 1) {
                    let error = if waited == WAIT_FAILED {
                        io::Error::last_os_error()
                    } else {
                        io::Error::other("unexpected pipe connect wait result")
                    };
                    // SAFETY: cancel pending connect and drain completion before dropping `overlapped`.
                    unsafe {
                        CancelIoEx(raw, Some(&overlapped)).ok();
                        let mut transferred = 0_u32;
                        GetOverlappedResult(raw, &overlapped, &mut transferred, true).ok();
                    }
                    return Err(error);
                }
                let mut transferred = 0_u32;
                // SAFETY: `connect_event` is signaled.
                unsafe { GetOverlappedResult(raw, &overlapped, &mut transferred, true) }
                    .map_err(|_| io::Error::last_os_error())?;
            }
            // Pre-create the next server instance immediately so back-to-back clients never hit FileNotFound.
            if let Ok(replacement) = create_pipe_instance(&self.pipe_name, false) {
                *crate::locked::locked(&self.next_instance) = Some(replacement);
            }
            Ok(Some(LocalStream::from_handle(instance)))
        }
    }

    impl Drop for LocalListener {
        fn drop(&mut self) {
            drop(self._marker_lock.take());
            if let Some(path) = self.marker_path.take() {
                fs::remove_file(path).ok();
            }
        }
    }

    pub(crate) fn sweep_marker_entry(entry: fs::DirEntry) {
        let path = entry.path();
        if path.extension().is_none_or(|ext| ext != "sock")
            || !entry.file_type().is_ok_and(|kind| kind.is_file())
        {
            return;
        }
        let Ok(file) = fs::OpenOptions::new().read(true).write(true).open(&path) else {
            return;
        };
        if try_lock_sentinel(&file).is_ok() {
            drop(file);
            fs::remove_file(&path).ok();
        }
    }

    pub(crate) fn generate_random_bytes(bytes: &mut [u8]) -> io::Result<()> {
        use windows::Win32::Security::Cryptography::{
            BCRYPT_USE_SYSTEM_PREFERRED_RNG, BCryptGenRandom,
        };
        // SAFETY: fills `bytes` using Windows' system CSPRNG.
        let status = unsafe { BCryptGenRandom(None, bytes, BCRYPT_USE_SYSTEM_PREFERRED_RNG) };
        if status.is_ok() {
            Ok(())
        } else {
            Err(io::Error::other(format!(
                "BCryptGenRandom failed: 0x{:08x}",
                status.0 as u32
            )))
        }
    }
}

const KEY_BYTES: usize = 32;

/// How long the listener rests after `accept` fails for any reason other than
/// "nothing is waiting".
///
/// Long enough that a listener the system has run out of descriptors for costs
/// nothing while it waits, short enough that service resumes almost as soon as
/// they are freed.
const ACCEPT_REST_MS: u16 = 100;

/// The longest socket path this platform can actually carry.
///
/// A Unix socket address keeps its path in `sockaddr_un.sun_path`, which is 108
/// bytes on Linux and 104 on macOS, NUL terminator included — so the usable
/// length is one less than the array.
///
/// This was a flat 100 on every platform, which is shorter than either, and the
/// difference was not academic. macOS `$TMPDIR` is itself a ~49-byte path under
/// `/var/folders`, so a socket nested one directory deeper than the product
/// nests it exceeded 100 — which is exactly what the endpoint tests do, and why
/// every one of them failed on macOS while passing on Linux. A limit that
/// refuses paths the platform would accept is not a safety margin; it is a bug
/// that only shows up where the base path is long.
///
/// Deliberately not read from `libc`: that would be a new direct dependency,
/// and a third-party notice, for two integers each platform's headers fix.
#[cfg(target_os = "macos")]
pub(crate) const MAX_SOCKET_PATH: usize = 103;
#[cfg(all(unix, not(target_os = "macos")))]
pub(crate) const MAX_SOCKET_PATH: usize = 107;

/// A per-window secret, compared in constant time and wiped when dropped.
pub struct SocketKey {
    bytes: [u8; KEY_BYTES],
}

impl SocketKey {
    /// Generates a key from the operating system's cryptographic source.
    ///
    /// Read straight from `/dev/urandom` rather than through a random-number
    /// crate: this is the only randomness Sprite needs, and on Linux the device
    /// is the same CSPRNG such a crate would reach for, so the dependency would
    /// buy nothing and still have to be audited. It must never come from a
    /// seeded or reproducible generator — an observer who can predict the key
    /// can read every pane.
    pub fn generate() -> std::io::Result<Self> {
        let mut bytes = [0_u8; KEY_BYTES];
        #[cfg(unix)]
        {
            let mut source = File::open("/dev/urandom")?;
            source.read_exact(&mut bytes)?;
        }
        #[cfg(windows)]
        {
            windows_ipc::generate_random_bytes(&mut bytes)?;
        }
        Ok(Self { bytes })
    }

    /// The key as lowercase hex, which is the only form that leaves this type.
    pub fn to_hex(&self) -> String {
        let mut hex = String::with_capacity(KEY_BYTES * 2);
        for byte in self.bytes {
            hex.push(nibble(byte >> 4));
            hex.push(nibble(byte & 0x0f));
        }
        hex
    }

    /// Whether `candidate` is this key, compared without leaking where it first
    /// differs.
    ///
    /// A comparison that stops at the first wrong byte tells a caller how much
    /// of a guess was right, which is enough to recover a key one byte at a
    /// time. Every path through this function looks at all 32 bytes.
    pub fn matches(&self, candidate: &str) -> bool {
        let mut guess = [0_u8; KEY_BYTES];
        // A malformed candidate is compared against zeroes rather than
        // returning early, so a wrong length costs the same as a wrong key.
        let well_formed = decode_hex(candidate, &mut guess);
        let mut difference = 0_u8;
        // Every byte, every time: `zip` over the full arrays visits all 32 just
        // as unconditionally as an indexed loop, so no path returns early.
        for (mine, theirs) in self.bytes.iter().zip(guess.iter()) {
            difference |= mine ^ theirs;
        }
        difference == 0 && well_formed
    }
}

impl Drop for SocketKey {
    fn drop(&mut self) {
        // Closing the window destroys the key. Overwriting it means a later
        // read of freed memory finds zeroes rather than a working secret.
        self.bytes.fill(0);
        // Keep the key wipe observable to the optimizer without unsafe memory writes.
        std::hint::black_box(&self.bytes);
    }
}

fn nibble(value: u8) -> char {
    char::from_digit(u32::from(value), 16).unwrap_or('0')
}

/// Decodes exactly `KEY_BYTES` of hex, reporting whether the input was valid.
///
/// Always fills `out` and always inspects the whole buffer.
fn decode_hex(text: &str, out: &mut [u8; KEY_BYTES]) -> bool {
    let bytes = text.as_bytes();
    let mut valid = bytes.len() == KEY_BYTES * 2;
    for (index, slot) in out.iter_mut().enumerate() {
        let high = bytes.get(index * 2).copied().unwrap_or(b'!');
        let low = bytes.get(index * 2 + 1).copied().unwrap_or(b'!');
        match (hex_value(high), hex_value(low)) {
            (Some(high), Some(low)) => *slot = (high << 4) | low,
            _ => {
                valid = false;
                *slot = 0;
            }
        }
    }
    valid
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[derive(Clone, Copy)]
pub(crate) struct TransportPolicy {
    pub name: &'static str,
    pub filename_hex: usize,
    pub suffix: &'static str,
    pub max_connections: usize,
    pub max_first_line: usize,
    pub handshake_timeout: Duration,
    pub write_timeout: Duration,
}

pub(crate) struct Authenticated {
    pub stream: LocalStream,
    // A pipelined Surface message may already be buffered after the key line.
    pub reader: BufReader<LocalStream>,
    pub body: String,
    pub reply_connection: ReplyConnection,
    /// This connection's place under the cap. A handler that leaves work
    /// running after it returns keeps a share, so the connection still
    /// counts until that work ends too.
    pub slot: Arc<ConnectionSlot>,
}

type Connections = Arc<Mutex<HashMap<usize, LocalStream>>>;

/// Only the authenticated origin may finish a bounded one-shot reply during closure.
#[derive(Clone, Debug)]
pub(crate) struct ReplyConnection {
    connections: Connections,
    id: usize,
}

/// One connection's place under the cap, given back when the last share is
/// dropped.
pub(crate) struct ConnectionSlot {
    connections: Connections,
    id: usize,
}

impl Drop for ConnectionSlot {
    fn drop(&mut self) {
        crate::locked::locked(&self.connections).remove(&self.id);
    }
}

pub(crate) struct LocalSocket {
    socket: PathBuf,
    /// `None` for a socket whose file permissions are its only check.
    key: Option<Arc<SocketKey>>,
    running: Arc<AtomicBool>,
    connections: Connections,
    listener: Option<JoinHandle<()>>,
    #[cfg(unix)]
    cancellation: UnixStream,
    #[cfg(windows)]
    cancellation: WakeSender,
}

impl LocalSocket {
    pub fn open_in<H>(
        directory: PathBuf,
        key: Arc<SocketKey>,
        policy: TransportPolicy,
        refused: fn(&mut LocalStream),
        handler: H,
    ) -> io::Result<Self>
    where
        H: Fn(Authenticated) + Send + Sync + 'static,
    {
        let mut name = SocketKey::generate()?.to_hex();
        name.truncate(policy.filename_hex);
        #[cfg(unix)]
        let socket = directory.join(format!("{name}{}", policy.suffix));
        #[cfg(windows)]
        let socket = {
            let _ = directory;
            PathBuf::from(format!(r"\\.\pipe\sprite-{name}{}", policy.suffix))
        };
        Self::open_at(socket, Some(key), policy, refused, handler)
    }

    /// Opens a socket at a path the caller chose, in a directory only its
    /// owner may enter. With no key, every connection the file permissions let
    /// through is served, and its whole first line is the body.
    #[cfg(unix)]
    pub fn open_at<H>(
        socket: PathBuf,
        key: Option<Arc<SocketKey>>,
        policy: TransportPolicy,
        refused: fn(&mut LocalStream),
        handler: H,
    ) -> io::Result<Self>
    where
        H: Fn(Authenticated) + Send + Sync + 'static,
    {
        let directory = socket
            .parent()
            .ok_or_else(|| io::Error::other("a local socket needs a directory"))?
            .to_path_buf();
        create_private_dir(&directory).inspect_err(|error| {
            tracing::warn!(target: "sprite_app::ipc", subsystem = policy.name, kind = ?error.kind(), "socket directory creation failed");
        })?;
        let (cancellation, cancelled) = UnixStream::pair()?;
        let listener = bind_private(&socket).inspect_err(|error| { tracing::warn!(target: "sprite_app::ipc", subsystem = policy.name, kind = ?error.kind(), "socket bind failed"); })?;
        if let Err(error) = listener.set_nonblocking(true) {
            fs::remove_file(&socket).ok();
            return Err(error);
        }
        let running = Arc::new(AtomicBool::new(true));
        let connections: Connections = Arc::default();
        let thread = std::thread::Builder::new()
            .name(policy.name.to_owned())
            .spawn({
                let running = Arc::clone(&running);
                let key = key.clone();
                let connections = Arc::clone(&connections);
                let handler = Arc::new(handler);
                move || {
                    let mut next_id = 0;
                    let mut housekeeping = fs::read_dir(directory).ok();
                    loop {
                        let mut fds = [
                            PollFd::new(cancelled.as_fd(), PollFlags::POLLIN),
                            PollFd::new(listener.as_fd(), PollFlags::POLLIN),
                        ];
                        let timeout = if housekeeping.is_some() {
                            PollTimeout::ZERO
                        } else {
                            PollTimeout::NONE
                        };
                        match poll(&mut fds, timeout) {
                            Err(Errno::EINTR) => continue,
                            // A failing `poll` is waited out like a failing
                            // `accept`: only closing the socket ends the loop.
                            Err(error) => {
                                tracing::warn!(target: "sprite_app::ipc", subsystem = policy.name, kind = ?error, "listener poll failed");
                                if rest(&cancelled, &running) {
                                    break;
                                }
                                continue;
                            }
                            Ok(_) => {}
                        }
                        if !running.load(Ordering::SeqCst)
                            || !fds[0].revents().unwrap_or_else(PollFlags::empty).is_empty()
                        {
                            break;
                        }
                        // One entry per turn keeps accepts and cancellation ahead of cleanup.
                        if let Some(entries) = housekeeping.as_mut() {
                            match entries.next() {
                                Some(Ok(entry)) => sweep_socket_entry(entry),
                                Some(Err(_)) => {}
                                None => housekeeping = None,
                            }
                        }
                        if fds[1].revents().unwrap_or_else(PollFlags::empty).is_empty() {
                            continue;
                        }
                        let stream = match listener.accept() {
                            Ok((stream, _)) => stream,
                            Err(error) if !rests_after(&error) => continue,
                            Err(error) => {
                                tracing::warn!(target: "sprite_app::ipc", subsystem = policy.name, kind = ?error.kind(), "listener accept failed");
                                if rest(&cancelled, &running) {
                                    break;
                                }
                                continue;
                            }
                        };
                        // BSD-derived systems hand the listener's O_NONBLOCK to accepted
                        // sockets; the handshake and handlers rely on blocking reads
                        // bounded by socket timeouts.
                        if stream.set_nonblocking(false).is_err() {
                            continue;
                        }
                        let id = next_id;
                        next_id += 1;
                        let deadline = Instant::now() + policy.handshake_timeout;
                        let mut live = crate::locked::locked(&connections);
                        if live.len() >= policy.max_connections {
                            continue;
                        }
                        let Ok(cancel) = stream.try_clone() else {
                            continue;
                        };
                        live.insert(id, cancel);
                        drop(live);
                        let slot = Arc::new(ConnectionSlot {
                            connections: Arc::clone(&connections),
                            id,
                        });
                        let running = Arc::clone(&running);
                        let key = key.clone();
                        let handler = Arc::clone(&handler);
                        // The slot also drops if spawning fails or an adapter panics.
                        std::thread::Builder::new()
                            .name(format!("{}-connection", policy.name))
                            .spawn(move || {
                                let _slot = slot;
                                let mut stream = stream;
                                if stream
                                    .set_write_timeout(Some(policy.write_timeout))
                                    .is_err()
                                {
                                    return;
                                }
                                match authenticate(&stream, key.as_deref(), &running, policy, deadline) {
                                    Ok((reader, body)) => {
                                        let started = Instant::now();
                                        handler(Authenticated {
                                            stream,
                                            reader,
                                            body,
                                            reply_connection: ReplyConnection {
                                                connections: Arc::clone(&_slot.connections),
                                                id,
                                            },
                                            slot: Arc::clone(&_slot),
                                        });
                                        tracing::debug!(target: "sprite_app::ipc", subsystem = policy.name, connection = id, elapsed_ms = started.elapsed().as_millis() as u64, "connection handler ended");
                                    }
                                    Err(_) => {
                                        tracing::debug!(target: "sprite_app::ipc", subsystem = policy.name, connection = id, "connection refused");
                                        refused(&mut stream);
                                    },
                                }
                            })
                            .inspect_err(|error| { tracing::warn!(target: "sprite_app::ipc", subsystem = policy.name, kind = ?error.kind(), "connection thread failed to start"); })
                            .ok();
                    }
                }
            });
        let thread = match thread {
            Ok(thread) => thread,
            Err(error) => {
                tracing::warn!(target: "sprite_app::ipc", subsystem = policy.name, kind = ?error.kind(), "listener thread failed to start");
                fs::remove_file(&socket).ok();
                return Err(error);
            }
        };
        tracing::info!(target: "sprite_app::ipc", subsystem = policy.name, "listener opened");
        Ok(Self {
            socket,
            key,
            running,
            connections,
            listener: Some(thread),
            cancellation,
        })
    }

    #[cfg(windows)]
    pub fn open_at<H>(
        socket: PathBuf,
        key: Option<Arc<SocketKey>>,
        policy: TransportPolicy,
        refused: fn(&mut LocalStream),
        handler: H,
    ) -> io::Result<Self>
    where
        H: Fn(Authenticated) + Send + Sync + 'static,
    {
        let directory = (!windows_ipc::is_pipe_path(&socket))
            .then(|| socket.parent().map(Path::to_path_buf))
            .flatten();
        let listener = bind_private(&socket).inspect_err(|error| {
            tracing::warn!(target: "sprite_app::ipc", subsystem = policy.name, kind = ?error.kind(), "pipe bind failed");
        })?;
        let (cancellation, cancelled) = wake_pair()?;
        let running = Arc::new(AtomicBool::new(true));
        let connections: Connections = Arc::default();
        let thread = std::thread::Builder::new()
            .name(policy.name.to_owned())
            .spawn({
                let running = Arc::clone(&running);
                let key = key.clone();
                let connections = Arc::clone(&connections);
                let handler = Arc::new(handler);
                move || {
                    let mut next_id = 0_usize;
                    if let Some(dir) = directory
                        && let Ok(entries) = fs::read_dir(dir)
                    {
                        for entry in entries.flatten() {
                            sweep_socket_entry(entry);
                        }
                    }
                    loop {
                        if !running.load(Ordering::SeqCst) {
                            break;
                        }
                        let stream = match listener.accept_cancellable(&cancelled) {
                            Ok(Some(stream)) => stream,
                            Ok(None) => break,
                            Err(error) => {
                                tracing::warn!(target: "sprite_app::ipc", subsystem = policy.name, kind = ?error.kind(), "listener accept failed");
                                if cancelled.wait_timeout(Duration::from_millis(u64::from(ACCEPT_REST_MS)))
                                    || !running.load(Ordering::SeqCst)
                                {
                                    break;
                                }
                                continue;
                            }
                        };
                        let id = next_id;
                        next_id += 1;
                        let deadline = Instant::now() + policy.handshake_timeout;
                        let mut live = crate::locked::locked(&connections);
                        if live.len() >= policy.max_connections {
                            continue;
                        }
                        let Ok(cancel) = stream.try_clone() else {
                            continue;
                        };
                        live.insert(id, cancel);
                        drop(live);
                        let slot = Arc::new(ConnectionSlot {
                            connections: Arc::clone(&connections),
                            id,
                        });
                        let running = Arc::clone(&running);
                        let key = key.clone();
                        let handler = Arc::clone(&handler);
                        std::thread::Builder::new()
                            .name(format!("{}-connection", policy.name))
                            .spawn(move || {
                                let _slot = slot;
                                let mut stream = stream;
                                if stream
                                    .set_write_timeout(Some(policy.write_timeout))
                                    .is_err()
                                {
                                    return;
                                }
                                match authenticate(&stream, key.as_deref(), &running, policy, deadline) {
                                    Ok((reader, body)) => {
                                        let started = Instant::now();
                                        handler(Authenticated {
                                            stream,
                                            reader,
                                            body,
                                            reply_connection: ReplyConnection {
                                                connections: Arc::clone(&_slot.connections),
                                                id,
                                            },
                                            slot: Arc::clone(&_slot),
                                        });
                                        tracing::debug!(target: "sprite_app::ipc", subsystem = policy.name, connection = id, elapsed_ms = started.elapsed().as_millis() as u64, "connection handler ended");
                                    }
                                    Err(_) => {
                                        tracing::debug!(target: "sprite_app::ipc", subsystem = policy.name, connection = id, "connection refused");
                                        refused(&mut stream);
                                    }
                                }
                            })
                            .inspect_err(|error| {
                                tracing::warn!(target: "sprite_app::ipc", subsystem = policy.name, kind = ?error.kind(), "connection thread failed to start");
                            })
                            .ok();
                    }
                    drop(listener);
                }
            });
        let thread = match thread {
            Ok(thread) => thread,
            Err(error) => {
                tracing::warn!(target: "sprite_app::ipc", subsystem = policy.name, kind = ?error.kind(), "listener thread failed to start");
                return Err(error);
            }
        };
        tracing::info!(target: "sprite_app::ipc", subsystem = policy.name, "listener opened");
        Ok(Self {
            socket,
            key,
            running,
            connections,
            listener: Some(thread),
            cancellation,
        })
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket
    }
    /// The key, or an empty string for a socket that has none.
    pub fn key_hex(&self) -> String {
        self.key
            .as_ref()
            .map_or_else(String::new, |key| key.to_hex())
    }

    /// How many connections hold a slot against the cap right now.
    #[cfg(test)]
    pub(crate) fn live_connections(&self) -> usize {
        self.connections.lock().unwrap().len()
    }

    pub fn close(&mut self) {
        self.close_after_reply(None);
    }

    /// Stop authentication and cancel peers while the initiating one-shot writes its reply.
    /// A registry-scoped identity cannot exempt a connection on a later socket.
    pub fn close_after_reply(&mut self, reply: Option<&ReplyConnection>) {
        if self.listener.is_none() {
            return;
        }
        self.running.store(false, Ordering::SeqCst);
        // Cancellation stays reachable even after the public socket pathname disappears.
        #[cfg(unix)]
        self.cancellation.shutdown(Shutdown::Both).ok();
        #[cfg(windows)]
        self.cancellation.wake();
        if let Some(thread) = self.listener.take() {
            thread.join().ok();
        }
        for (id, stream) in crate::locked::locked(&self.connections).iter() {
            let finishing_reply = reply.is_some_and(|reply| {
                Arc::ptr_eq(&reply.connections, &self.connections) && reply.id == *id
            });
            stream
                .shutdown(if finishing_reply {
                    Shutdown::Read
                } else {
                    Shutdown::Both
                })
                .ok();
        }
        #[cfg(unix)]
        fs::remove_file(&self.socket).ok();
        #[cfg(windows)]
        if !windows_ipc::is_pipe_path(&self.socket) {
            fs::remove_file(&self.socket).ok();
        }
        // Other windows may be between directory creation and binding their socket.
        // Leaving the shared directory avoids racing their startup.
    }
}

impl Drop for LocalSocket {
    fn drop(&mut self) {
        self.close();
    }
}

/// Whether the listener rests before trying again after `accept` fails. No
/// failure ends the listener; only closing the socket does.
///
/// Nothing waiting after all, or a signal interrupting the call, goes straight
/// back to `poll`. Anything else rests first, because `poll` would report the
/// same waiting connection at once and going straight back would spin.
/// Running out of descriptors (`EMFILE`, `ENFILE`), buffers (`ENOBUFS`) or
/// memory (`ENOMEM`), and a peer that hung up mid-accept (`ECONNABORTED`), all
/// pass, so the listener waits them out. An unexpected error is treated the
/// same way rather than ending the loop: a listener that stops silently leaves
/// a socket that accepts connections nobody will ever serve.
#[cfg(unix)]
fn rests_after(error: &io::Error) -> bool {
    !matches!(
        error.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
    )
}

/// Rests for [`ACCEPT_REST_MS`], waking early if the socket is closed.
///
/// Returns whether the socket is being closed.
#[cfg(unix)]
#[expect(
    clippy::disallowed_methods,
    reason = "a failed poll must not turn this rest into a spin; this is the listener thread"
)]
fn rest(cancelled: &UnixStream, running: &AtomicBool) -> bool {
    let mut fds = [PollFd::new(cancelled.as_fd(), PollFlags::POLLIN)];
    if poll(&mut fds, PollTimeout::from(ACCEPT_REST_MS)).is_err() {
        // A failing `poll` must not turn the rest into a spin.
        std::thread::sleep(Duration::from_millis(u64::from(ACCEPT_REST_MS)));
    }
    !running.load(Ordering::SeqCst) || !fds[0].revents().unwrap_or_else(PollFlags::empty).is_empty()
}

#[cfg(unix)]
pub(crate) fn bind_private(socket: &Path) -> io::Result<UnixListener> {
    let length = socket.as_os_str().len();
    if length > MAX_SOCKET_PATH {
        return Err(io::Error::other(format!(
            "the local socket path is {length} bytes and this platform's sockaddr_un holds {MAX_SOCKET_PATH}: {}",
            socket.display()
        )));
    }
    let listener = UnixListener::bind(socket)?;
    if let Err(error) = fs::set_permissions(socket, fs::Permissions::from_mode(0o600)) {
        drop(listener);
        fs::remove_file(socket).ok();
        return Err(error);
    }
    Ok(listener)
}

#[cfg(windows)]
pub(crate) fn bind_private(socket: &Path) -> io::Result<LocalListener> {
    LocalListener::bind(socket)
}

/// Moves what `bytes` holds of the line being read into `line`: up to and
/// including its newline, if one is there. Returns how many bytes it took
/// and whether the line is complete. A line longer than `max` bytes, its
/// ending included, is refused.
fn gather(bytes: &[u8], line: &mut Vec<u8>, max: usize) -> io::Result<(usize, bool)> {
    let newline = bytes.iter().position(|byte| *byte == b'\n');
    let count = newline.map_or(bytes.len(), |index| index + 1);
    if count > max - line.len() {
        return Err(io::ErrorKind::InvalidData.into());
    }
    line.extend_from_slice(bytes.get(..count).unwrap_or_default());
    if newline.is_none() && line.len() == max {
        return Err(io::ErrorKind::InvalidData.into());
    }
    Ok((count, newline.is_some()))
}

fn authenticate(
    stream: &LocalStream,
    key: Option<&SocketKey>,
    running: &AtomicBool,
    policy: TransportPolicy,
    deadline: Instant,
) -> io::Result<(BufReader<LocalStream>, String)> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = Vec::new();
    loop {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .filter(|duration| !duration.is_zero())
            .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "handshake deadline"))?;
        if !running.load(Ordering::SeqCst) {
            return Err(io::ErrorKind::Interrupted.into());
        }
        // Socket timeouts apply per read, so every read uses the remaining total budget.
        stream.set_read_timeout(Some(remaining))?;
        let bytes = reader.fill_buf()?;
        if bytes.is_empty() {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        let (count, ended) = gather(bytes, &mut line, policy.max_first_line)?;
        reader.consume(count);
        if ended {
            break;
        }
    }
    let line = std::str::from_utf8(&line).map_err(|_| io::ErrorKind::InvalidData)?;
    let line = line.trim_end_matches(['\r', '\n']);
    let body = match key {
        Some(key) => {
            let (presented, body) = line.split_once(' ').unwrap_or((line, ""));
            if !key.matches(presented) {
                return Err(io::ErrorKind::PermissionDenied.into());
            }
            body
        }
        None => line,
    };
    if Instant::now() >= deadline || !running.load(Ordering::SeqCst) {
        return Err(io::ErrorKind::PermissionDenied.into());
    }
    stream.set_read_timeout(None)?;
    Ok((reader, body.to_owned()))
}

/// The most a writer hands the socket in one write, so it checks its drain
/// deadline at least this often.
const WRITE_CHUNK_BYTES: usize = 64 * 1024;

/// Writes `bytes` within `budget`, one chunk at a time. Before each chunk
/// the socket's write timeout is set to the smaller of `WRITE_TIMEOUT` and
/// the time left. A write that makes no progress still fails after
/// `WRITE_TIMEOUT`, and a reader that makes a little progress on every
/// write still runs out of time. Running out is a timeout like any other.
///
/// The timeout is a property of the socket, shared by every clone of it.
/// Once a connection is opened only its writer writes to it, so narrowing
/// the timeout here does not affect any other writer.
pub(crate) fn write_within(
    mut stream: &LocalStream,
    bytes: &[u8],
    budget: Duration,
) -> io::Result<()> {
    let started = Instant::now();
    let mut rest = bytes;
    while !rest.is_empty() {
        let left = budget.saturating_sub(started.elapsed());
        // Checked before the socket is told: it refuses a zero timeout.
        if left.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "the reader fell below the drain floor",
            ));
        }
        // macOS refuses to set a timeout on a socket whose program has
        // closed its end, with EINVAL rather than the broken pipe a write
        // would report. The timeout here is never zero and never more than
        // `WRITE_TIMEOUT`, so that refusal can only mean the program is gone.
        stream
            .set_write_timeout(Some(left.min(WRITE_TIMEOUT)))
            .map_err(|error| {
                if error.kind() == io::ErrorKind::InvalidInput {
                    io::Error::new(io::ErrorKind::NotConnected, error)
                } else {
                    error
                }
            })?;
        let (chunk, _) = rest.split_at(rest.len().min(WRITE_CHUNK_BYTES));
        match stream.write(chunk) {
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(written) => rest = rest.get(written..).unwrap_or_default(),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    stream.flush()
}

/// Waits up to `limit` for `stream` to have something to read: data, the
/// peer's end-of-stream, or an error. Returns whether it has.
///
/// On macOS a thread asleep in `recv` can miss the peer closing its end and
/// wait forever (issue #79). A reader that waits here first, for a bounded
/// time, and reads only when told there is something, cannot.
pub(crate) fn wait_readable(stream: &LocalStream, limit: Duration) -> io::Result<bool> {
    #[cfg(unix)]
    {
        let millis = u16::try_from(limit.as_millis()).unwrap_or(u16::MAX);
        let mut descriptors = [PollFd::new(stream.as_fd(), PollFlags::POLLIN)];
        match poll(&mut descriptors, PollTimeout::from(millis)) {
            Ok(0) | Err(Errno::EINTR) => Ok(false),
            Ok(_) => Ok(true),
            Err(error) => Err(error.into()),
        }
    }
    #[cfg(windows)]
    {
        stream.wait_readable(limit)
    }
}

/// How often a reader waiting for a quiet peer looks again.
pub(crate) const READ_RECHECK: Duration = Duration::from_secs(1);

/// Reads the next line a connection that stays open sends, without its line
/// ending. `None` when the peer closed instead.
///
/// The peer may stay quiet for as long as it likes: a closed connection is
/// noticed through [`wait_readable`] on Unix and overlapped `ReadFile` timeouts
/// on Windows, never by a read that could sleep through it. Once a line has
/// started, the rest must arrive within `patience`, and the line may be at most
/// `max` bytes.
pub(crate) fn next_line(
    reader: &mut BufReader<LocalStream>,
    max: usize,
    patience: Duration,
) -> io::Result<Option<String>> {
    let mut line = Vec::new();
    let mut deadline: Option<Instant> = None;
    loop {
        if reader.buffer().is_empty() {
            let limit = match deadline {
                None => READ_RECHECK,
                Some(deadline) => deadline
                    .checked_duration_since(Instant::now())
                    .filter(|left| !left.is_zero())
                    .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "line deadline"))?
                    .min(READ_RECHECK),
            };
            #[cfg(unix)]
            {
                if !wait_readable(reader.get_ref(), limit)? {
                    continue;
                }
                // Readiness was reported, so this read returns at once; the
                // timeout only covers a report that turns out to be wrong.
                // macOS refuses to set a timeout on a socket whose peer has
                // closed (EINVAL). The read below then reports the closing, so
                // a refusal here is not an error of its own.
                reader.get_ref().set_read_timeout(Some(READ_RECHECK)).ok();
            }
            #[cfg(windows)]
            {
                reader.get_ref().set_read_timeout(Some(limit)).ok();
            }
        }
        let bytes = match reader.fill_buf() {
            Ok(bytes) => bytes,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock
                        | io::ErrorKind::TimedOut
                        | io::ErrorKind::Interrupted
                ) =>
            {
                continue;
            }
            Err(error) => return Err(error),
        };
        if bytes.is_empty() {
            return if line.is_empty() {
                Ok(None)
            } else {
                Err(io::ErrorKind::UnexpectedEof.into())
            };
        }
        deadline.get_or_insert_with(|| Instant::now() + patience);
        let (count, ended) = gather(bytes, &mut line, max)?;
        reader.consume(count);
        if ended {
            break;
        }
    }
    let line = String::from_utf8(line).map_err(|_| io::ErrorKind::InvalidData)?;
    Ok(Some(line.trim_end_matches(['\r', '\n']).to_owned()))
}

/// Removes abandoned socket files without entering a live listener's accept queue.
#[cfg(test)]
pub(crate) fn sweep_dead_sockets(directory: &Path) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        sweep_socket_entry(entry);
    }
}

#[cfg(test)]
thread_local! {
    static SWEPT_ENTRIES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(unix)]
fn sweep_socket_entry(entry: fs::DirEntry) {
    #[cfg(test)]
    SWEPT_ENTRIES.with(|count| count.set(count.get() + 1));
    let path = entry.path();
    if path.extension().is_none_or(|extension| extension != "sock")
        || !entry.file_type().is_ok_and(|kind| kind.is_socket())
    {
        return;
    }
    let Ok(probe) = UnixDatagram::unbound() else {
        return;
    };
    if probe.set_nonblocking(true).is_err() {
        return;
    }
    // A live stream socket rejects a datagram with EPROTOTYPE before checking its backlog.
    // Stream probes can instead report ECONNREFUSED for a full live backlog on macOS.
    // Any error other than a definite refusal leaves the path alone.
    if let Err(error) = probe.connect(&path)
        && error.kind() == io::ErrorKind::ConnectionRefused
    {
        fs::remove_file(&path).ok();
    }
}

#[cfg(windows)]
fn sweep_socket_entry(entry: fs::DirEntry) {
    #[cfg(test)]
    SWEPT_ENTRIES.with(|count| count.set(count.get() + 1));
    windows_ipc::sweep_marker_entry(entry);
}

/// The per-user runtime directory this window's socket lives in.
///
/// On Linux, `XDG_RUNTIME_DIR`: a directory the system already guarantees is
/// private to one user and cleaned up on logout. There is deliberately no fall
/// back to a world-writable temporary directory: an endpoint nobody else can
/// reach is the whole point, so it is better to have no observation surface
/// than one in a place another user can reach.
#[cfg(all(unix, not(target_os = "macos")))]
pub(crate) fn runtime_directory() -> std::io::Result<PathBuf> {
    let base = std::env::var_os("XDG_RUNTIME_DIR").ok_or_else(|| {
        std::io::Error::other(
            "XDG_RUNTIME_DIR is not set, so there is no private directory for the \
             observation socket; observation is unavailable rather than placed \
             somewhere other users could reach",
        )
    })?;
    Ok(PathBuf::from(base).join("sprite"))
}

/// The per-user runtime directory this window's socket lives in.
///
/// macOS has no `XDG_RUNTIME_DIR`, and the equivalent guarantee lives
/// elsewhere: `TMPDIR` there is a per-user directory under `/var/folders`,
/// created by the system and readable only by its owner — the same property
/// `XDG_RUNTIME_DIR` is chosen for on Linux. Without this, observation would
/// simply not exist on macOS, which is not parity.
///
/// `TMPDIR` is honoured because that is where the system publishes the path;
/// the mode is not taken on trust either way, since [`LocalSocket::open_in`]
/// applies 0700 to the directory it is given before the socket exists.
#[cfg(target_os = "macos")]
pub(crate) fn runtime_directory() -> std::io::Result<PathBuf> {
    let base = std::env::var_os("TMPDIR").ok_or_else(|| {
        std::io::Error::other(
            "TMPDIR is not set, so there is no private directory for the \
             observation socket; observation is unavailable rather than placed \
             somewhere other users could reach",
        )
    })?;
    Ok(PathBuf::from(base).join("sprite"))
}

/// The per-user runtime directory for Windows window discovery markers.
///
/// Actual IPC streams use owner-only Win32 Named Pipes (`\\.\pipe\sprite-...`);
/// `%LOCALAPPDATA%\sprite\r` holds locked marker files so `sprite panes list`
/// discovers running windows without opening any network port.
#[cfg(target_os = "windows")]
pub(crate) fn runtime_directory() -> std::io::Result<PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA").ok_or_else(|| {
        std::io::Error::other(
            "LOCALAPPDATA is not set, so there is no private directory for \
             window discovery markers",
        )
    })?;
    Ok(PathBuf::from(base).join("sprite").join("r"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_blocking_wait::watchdog;
    use std::io::Write;
    use std::sync::mpsc;

    /// A reader over one end of a pair, and the other end.
    fn line_pair() -> (BufReader<LocalStream>, LocalStream) {
        let (ours, theirs) = LocalStream::pair().unwrap();
        (BufReader::new(ours), theirs)
    }

    const LINE_PATIENCE: Duration = Duration::from_millis(200);

    #[test]
    fn next_line_returns_each_line_and_then_the_peers_leaving() {
        let (mut reader, mut peer) = line_pair();
        peer.write_all(b"first\r\nsec").unwrap();
        assert_eq!(
            next_line(&mut reader, 64, watchdog()).unwrap().as_deref(),
            Some("first")
        );
        peer.write_all(b"ond\n").unwrap();
        assert_eq!(
            next_line(&mut reader, 64, watchdog()).unwrap().as_deref(),
            Some("second")
        );
        drop(peer);
        assert_eq!(next_line(&mut reader, 64, watchdog()).unwrap(), None);
    }

    /// A peer may say nothing for longer than a line is given to arrive in:
    /// the limit runs from a line's first byte, not from the last line.
    #[test]
    fn next_line_waits_out_a_quiet_peer_and_reads_what_it_then_sends() {
        let (mut reader, mut peer) = line_pair();
        let thread = std::thread::spawn(move || next_line(&mut reader, 64, LINE_PATIENCE));
        crate::test_blocking_wait::pause(LINE_PATIENCE * 3);
        assert!(!thread.is_finished(), "a quiet peer was given up on");
        peer.write_all(b"late\n").unwrap();
        assert_eq!(thread.join().unwrap().unwrap().as_deref(), Some("late"));
    }

    /// The peer leaving while the reader waits is noticed by the wait, with
    /// no read asleep to miss it (issue #79).
    #[test]
    fn next_line_notices_the_peer_leaving_while_it_waits() {
        let (mut reader, peer) = line_pair();
        let (done, finished) = mpsc::channel();
        let thread = std::thread::spawn(move || {
            done.send(next_line(&mut reader, 64, LINE_PATIENCE).unwrap())
                .unwrap();
        });
        crate::test_blocking_wait::pause(Duration::from_millis(50));
        drop(peer);
        assert_eq!(finished.recv_timeout(watchdog()).unwrap(), None);
        thread.join().unwrap();
    }

    /// A peer that is gone altogether is the end of the stream, not an
    /// error: macOS refuses socket options on such a socket, and that
    /// refusal must not be reported in place of the closing.
    #[test]
    fn next_line_reads_a_closed_peer_as_the_end_of_the_stream() {
        for _ in 0..200 {
            let (mut reader, mut peer) = line_pair();
            peer.write_all(b"last\n").unwrap();
            drop(peer);
            assert_eq!(
                next_line(&mut reader, 64, watchdog()).unwrap().as_deref(),
                Some("last")
            );
            assert_eq!(next_line(&mut reader, 64, watchdog()).unwrap(), None);
        }
    }

    #[test]
    fn next_line_gives_up_on_a_line_that_stalls_or_grows_too_long() {
        let (mut reader, mut peer) = line_pair();
        peer.write_all(b"never finished").unwrap();
        let started = Instant::now();
        let stalled = next_line(&mut reader, 64, LINE_PATIENCE).unwrap_err();
        assert_eq!(stalled.kind(), io::ErrorKind::TimedOut);
        assert!(started.elapsed() >= LINE_PATIENCE);

        let (mut reader, mut peer) = line_pair();
        peer.write_all(&[b'x'; 65]).unwrap();
        let long = next_line(&mut reader, 64, watchdog()).unwrap_err();
        assert_eq!(long.kind(), io::ErrorKind::InvalidData);

        let (mut reader, mut peer) = line_pair();
        peer.write_all(b"cut off").unwrap();
        drop(peer);
        let cut = next_line(&mut reader, 64, watchdog()).unwrap_err();
        assert_eq!(cut.kind(), io::ErrorKind::UnexpectedEof);
    }

    const POLICY: TransportPolicy = TransportPolicy {
        name: "sprite-transport-test",
        filename_hex: 24,
        suffix: ".sock",
        max_connections: 2,
        max_first_line: 128,
        handshake_timeout: Duration::from_millis(200),
        write_timeout: Duration::from_millis(200),
    };

    struct Scratch(PathBuf);
    impl Scratch {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            Self(std::env::temp_dir().join(format!(
                "ls-{:x}-{:x}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::SeqCst)
            )))
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).ok();
        }
    }

    fn rejected(stream: &mut LocalStream) {
        writeln!(stream, "denied").ok();
        stream.shutdown(Shutdown::Write).ok();
    }

    #[cfg(unix)]
    fn assert_eof(stream: &mut LocalStream) {
        // Darwin rejects socket options if the peer has already closed.
        let deadline = Instant::now() + watchdog();
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(
                !remaining.is_zero(),
                "peer did not close before the watchdog"
            );
            let mut descriptors = [PollFd::new(stream.as_fd(), PollFlags::POLLIN)];
            match poll(&mut descriptors, PollTimeout::try_from(remaining).unwrap()) {
                Ok(0) | Err(Errno::EINTR) => continue,
                Ok(_) => break,
                Err(error) => panic!("waiting for EOF: {error}"),
            }
        }
        assert_eq!(stream.read(&mut [0]).unwrap(), 0);
    }

    #[cfg(windows)]
    fn assert_eof(stream: &mut LocalStream) {
        stream.set_read_timeout(Some(watchdog())).unwrap();
        assert_eq!(stream.read(&mut [0]).unwrap(), 0);
    }

    #[test]
    fn eof_is_observed_even_when_the_peer_already_closed() {
        let (mut stream, peer) = LocalStream::pair().unwrap();
        drop(peer);
        assert_eof(&mut stream);
    }

    fn wait_for_count(socket: &LocalSocket, count: usize) {
        let deadline = Instant::now() + watchdog();
        while socket.connections.lock().unwrap().len() != count {
            assert!(
                Instant::now() < deadline,
                "connection count did not reach {count}"
            );
            std::thread::yield_now();
        }
    }

    #[cfg(unix)]
    fn run_in_child(test: &str, run: impl FnOnce(&Path)) {
        const CHILD: &str = "SPRITE_SOCKET_HOUSEKEEPING_CHILD";
        if let Some(directory) = std::env::var_os(CHILD) {
            run(Path::new(&directory));
            return;
        }
        let scratch = Scratch::new();
        create_private_dir(&scratch.0).unwrap();
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", test, "--nocapture", "--test-threads=1"])
            .env(CHILD, &scratch.0)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        // The child can wait for a reply and then for housekeeping, each for one watchdog.
        let deadline = Instant::now() + watchdog() * 3;
        let mut expired = false;
        while child.try_wait().unwrap().is_none() {
            if Instant::now() >= deadline {
                expired = true;
                child.kill().unwrap();
                break;
            }
            crate::test_blocking_wait::pause(Duration::from_millis(10));
        }
        let output = child.wait_with_output().unwrap();
        println!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            !expired,
            "socket housekeeping exceeded the independent watchdog"
        );
        assert!(
            output.status.success(),
            "socket housekeeping subprocess failed"
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
    }

    #[cfg(unix)]
    fn saturated_listener(path: &Path) -> (UnixListener, Vec<UnixStream>) {
        use std::os::fd::{AsRawFd, FromRawFd};
        use std::os::unix::ffi::OsStrExt;
        let listener = bind_private(path).unwrap();
        // SAFETY: the listener owns a valid socket descriptor.
        assert_eq!(unsafe { nix::libc::listen(listener.as_raw_fd(), 1) }, 0);
        let mut held = Vec::new();
        for _ in 0..16 {
            // SAFETY: socket returns a fresh descriptor, transferred once to UnixStream.
            let fd = unsafe { nix::libc::socket(nix::libc::AF_UNIX, nix::libc::SOCK_STREAM, 0) };
            assert!(fd >= 0);
            let stream = unsafe { UnixStream::from_raw_fd(fd) };
            stream.set_nonblocking(true).unwrap();
            // SAFETY: zero is a valid initial sockaddr_un representation.
            let mut address: nix::libc::sockaddr_un = unsafe { std::mem::zeroed() };
            address.sun_family = nix::libc::AF_UNIX as _;
            let bytes = path.as_os_str().as_bytes();
            assert!(bytes.len() < address.sun_path.len());
            for (slot, byte) in address.sun_path.iter_mut().zip(bytes) {
                *slot = *byte as _;
            }
            let length = std::mem::size_of_val(&address) as nix::libc::socklen_t;
            #[cfg(target_os = "macos")]
            {
                address.sun_len = length as _;
            }
            // SAFETY: address is initialized and its supplied length matches its allocation.
            let result = unsafe {
                nix::libc::connect(
                    fd,
                    (&address as *const nix::libc::sockaddr_un).cast(),
                    length,
                )
            };
            if result != 0 {
                let error = io::Error::last_os_error();
                eprintln!(
                    "saturated backlog on {} after {} connections: {error}",
                    std::env::consts::OS,
                    held.len()
                );
                assert!(
                    matches!(error.raw_os_error(), Some(code) if code == nix::libc::ECONNREFUSED || code == nix::libc::EAGAIN || code == nix::libc::EINPROGRESS)
                );
                assert!(!held.is_empty());
                return (listener, held);
            }
            held.push(stream);
        }
        panic!("private backlog did not saturate");
    }

    #[cfg(unix)]
    #[test]
    fn housekeeping_preserves_a_saturated_live_listener() {
        run_in_child(
            "local_socket::tests::housekeeping_preserves_a_saturated_live_listener",
            |directory| {
                let full = directory.join("full.sock");
                let (_listener, _held) = saturated_listener(&full);
                let live = directory.join("live.sock");
                let _live = bind_private(&live).unwrap();
                let stale = directory.join("stale.sock");
                drop(bind_private(&stale).unwrap());
                sweep_dead_sockets(directory);
                assert!(full.exists(), "a full live backlog must never be unlinked");
                assert!(live.exists(), "ordinary live listener must survive");
                let deadline = Instant::now() + watchdog();
                while stale.exists() && Instant::now() < deadline {
                    sweep_dead_sockets(directory);
                    std::thread::yield_now();
                }
                assert!(!stale.exists(), "genuinely stale socket must be removed");
            },
        );
    }

    #[cfg(unix)]
    #[test]
    fn opening_serves_requests_without_sweeping_on_the_calling_thread() {
        run_in_child(
            "local_socket::tests::opening_serves_requests_without_sweeping_on_the_calling_thread",
            |directory| {
                let full = directory.join("full.sock");
                let (_listener, _held) = saturated_listener(&full);
                let stale = directory.join("stale.sock");
                drop(bind_private(&stale).unwrap());
                for index in 0..256 {
                    fs::write(directory.join(format!("ordinary-{index}")), b"").unwrap();
                }
                SWEPT_ENTRIES.with(|count| count.set(0));
                let (sent, received) = mpsc::channel();
                let mut socket = LocalSocket::open_in(
                    directory.to_owned(),
                    Arc::new(SocketKey::generate().unwrap()),
                    POLICY,
                    rejected,
                    move |connection| {
                        sent.send(connection.body).unwrap();
                    },
                )
                .unwrap();
                assert_eq!(
                    SWEPT_ENTRIES.with(|count| count.get()),
                    0,
                    "opening must not traverse the directory on the calling/UI thread"
                );
                let _client = authenticated_client(&socket);
                assert_eq!(received.recv_timeout(watchdog()).unwrap(), "reply");
                let deadline = Instant::now() + watchdog();
                while stale.exists() && Instant::now() < deadline {
                    std::thread::yield_now();
                }
                assert!(
                    !stale.exists(),
                    "listener owner must eventually clean stale files"
                );
                assert!(
                    full.exists(),
                    "opening must preserve the saturated listener"
                );
                socket.close();
                assert!(!socket.socket_path().exists());
            },
        );
    }

    #[test]
    fn authentication_requires_a_complete_bounded_utf8_line_and_the_exact_key() {
        let key = SocketKey::generate().unwrap();
        let running = AtomicBool::new(true);
        for line in [
            "\n".to_owned(),
            "wrong hello\n".to_owned(),
            format!("{} hello", key.to_hex()),
            format!("{} {}\n", key.to_hex(), "x".repeat(POLICY.max_first_line)),
            format!("{} hello\n", "a".repeat(65)),
        ] {
            let (mut client, server) = LocalStream::pair().unwrap();
            client.write_all(line.as_bytes()).unwrap();
            drop(client);
            assert!(
                authenticate(
                    &server,
                    Some(&key),
                    &running,
                    POLICY,
                    Instant::now() + POLICY.handshake_timeout
                )
                .is_err()
            );
        }
        let (mut client, server) = LocalStream::pair().unwrap();
        client.write_all(&[0xff, b'\n']).unwrap();
        assert!(
            authenticate(
                &server,
                Some(&key),
                &running,
                POLICY,
                Instant::now() + POLICY.handshake_timeout
            )
            .is_err()
        );
    }

    #[test]
    fn the_exact_line_limit_is_accepted_and_buffered_followup_bytes_survive() {
        let key = SocketKey::generate().unwrap();
        let (mut client, server) = LocalStream::pair().unwrap();
        let body = "x".repeat(POLICY.max_first_line - 66);
        write!(client, "{} {body}\nnext\n", key.to_hex()).unwrap();
        let (mut reader, parsed) = authenticate(
            &server,
            Some(&key),
            &AtomicBool::new(true),
            POLICY,
            Instant::now() + POLICY.handshake_timeout,
        )
        .unwrap();
        assert_eq!(parsed, body);
        assert_eq!(reader.get_ref().read_timeout().unwrap(), None);
        let mut next = String::new();
        reader.read_line(&mut next).unwrap();
        assert_eq!(next, "next\n");
    }

    #[test]
    fn a_trickling_client_cannot_restart_the_handshake_deadline() {
        let key = SocketKey::generate().unwrap();
        let (mut client, server) = LocalStream::pair().unwrap();
        let (stop, stopped) = mpsc::channel();
        let writer = std::thread::spawn(move || {
            while stopped.recv_timeout(Duration::from_millis(20)).is_err() {
                if client.write_all(b"a").is_err() {
                    break;
                }
            }
        });
        let start = Instant::now();
        let result = authenticate(
            &server,
            Some(&key),
            &AtomicBool::new(true),
            POLICY,
            start + POLICY.handshake_timeout,
        );
        let elapsed = start.elapsed();
        stop.send(()).unwrap();
        writer.join().unwrap();
        assert!(result.is_err());
        assert!(elapsed >= POLICY.handshake_timeout);
        assert!(
            elapsed < Duration::from_secs(1),
            "deadline was renewed: {elapsed:?}"
        );
    }

    #[test]
    fn a_silent_client_and_a_cancelled_key_never_authenticate() {
        let key = SocketKey::generate().unwrap();
        let (_client, server) = LocalStream::pair().unwrap();
        assert!(
            authenticate(
                &server,
                Some(&key),
                &AtomicBool::new(true),
                POLICY,
                Instant::now() + POLICY.handshake_timeout
            )
            .is_err()
        );
        let (mut client, server) = LocalStream::pair().unwrap();
        writeln!(client, "{} hello", key.to_hex()).unwrap();
        assert!(
            authenticate(
                &server,
                Some(&key),
                &AtomicBool::new(false),
                POLICY,
                Instant::now() + POLICY.handshake_timeout
            )
            .is_err()
        );
    }

    #[test]
    fn the_cap_covers_authenticated_connections_and_releases_after_completion() {
        let scratch = Scratch::new();
        let (entered, entry) = mpsc::channel();
        let mut socket = LocalSocket::open_in(
            scratch.0.clone(),
            Arc::new(SocketKey::generate().unwrap()),
            POLICY,
            rejected,
            move |mut connection| {
                entered.send(()).unwrap();
                let mut done = String::new();
                connection.reader.read_line(&mut done).ok();
            },
        )
        .unwrap();
        let mut clients = Vec::new();
        for _ in 0..POLICY.max_connections {
            let mut stream = LocalStream::connect(socket.socket_path()).unwrap();
            writeln!(stream, "{} hello", socket.key_hex()).unwrap();
            entry.recv_timeout(watchdog()).unwrap();
            clients.push(stream);
        }
        let mut excess = LocalStream::connect(socket.socket_path()).unwrap();
        assert_eof(&mut excess);
        drop(clients.pop());
        wait_for_count(&socket, 1);
        let mut next = LocalStream::connect(socket.socket_path()).unwrap();
        writeln!(next, "{} hello", socket.key_hex()).unwrap();
        entry.recv_timeout(watchdog()).unwrap();
        socket.close();
        wait_for_count(&socket, 0);
    }

    #[test]
    fn silent_handshakes_are_counted_against_the_connection_cap() {
        let scratch = Scratch::new();
        let policy = TransportPolicy {
            handshake_timeout: Duration::from_secs(30),
            ..POLICY
        };
        let mut socket = LocalSocket::open_in(
            scratch.0.clone(),
            Arc::new(SocketKey::generate().unwrap()),
            policy,
            rejected,
            |_| panic!("unauthenticated"),
        )
        .unwrap();
        let _clients: Vec<_> = (0..policy.max_connections)
            .map(|_| LocalStream::connect(socket.socket_path()).unwrap())
            .collect();
        wait_for_count(&socket, policy.max_connections);
        let mut excess = LocalStream::connect(socket.socket_path()).unwrap();
        assert_eof(&mut excess);
        socket.close();
        wait_for_count(&socket, 0);
    }

    #[test]
    fn closing_interrupts_silent_handshakes_and_removes_only_the_socket() {
        let scratch = Scratch::new();
        create_private_dir(&scratch.0).unwrap();
        let policy = TransportPolicy {
            handshake_timeout: Duration::from_secs(30),
            ..POLICY
        };
        let mut socket = LocalSocket::open_in(
            scratch.0.clone(),
            Arc::new(SocketKey::generate().unwrap()),
            policy,
            rejected,
            |_| panic!("unauthenticated"),
        )
        .unwrap();
        let path = socket.socket_path().to_owned();
        let mut client = LocalStream::connect(&path).unwrap();
        // macOS can reject changing the read timeout after the peer has shut down.
        client.set_read_timeout(Some(watchdog())).unwrap();
        wait_for_count(&socket, 1);
        socket.close();
        socket.close();
        let mut result = String::new();
        client.read_to_string(&mut result).unwrap();
        assert!(result.is_empty() || result == "denied\n");
        wait_for_count(&socket, 0);
        #[cfg(unix)]
        assert!(!path.exists());
        assert!(LocalStream::connect(&path).is_err());
        assert!(scratch.0.exists());
    }

    /// A socket with no key serves the whole first line, at the path it was
    /// given, in a private directory.
    #[test]
    fn a_keyless_socket_serves_the_whole_line_at_its_own_path() {
        let scratch = Scratch::new();
        let path = scratch.0.join("window-test.sock");
        let (seen, bodies) = mpsc::channel();
        let socket =
            LocalSocket::open_at(path.clone(), None, POLICY, rejected, move |connection| {
                seen.send(connection.body).unwrap();
            })
            .unwrap();
        assert_eq!(socket.socket_path(), path);
        #[cfg(unix)]
        {
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
            assert_eq!(
                fs::metadata(&scratch.0).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
        let mut client = LocalStream::connect(&path).unwrap();
        writeln!(client, "{{\"a\": 1}} with spaces").unwrap();
        assert_eq!(
            bodies.recv_timeout(watchdog()).unwrap(),
            "{\"a\": 1} with spaces"
        );
    }

    fn gated_reply_socket(
        directory: &Path,
    ) -> (
        LocalSocket,
        mpsc::Receiver<ReplyConnection>,
        mpsc::Sender<()>,
    ) {
        let (entered, identities) = mpsc::channel();
        let (finish, finished) = mpsc::channel();
        let finished = Mutex::new(finished);
        let socket = LocalSocket::open_in(
            directory.to_owned(),
            Arc::new(SocketKey::generate().unwrap()),
            POLICY,
            rejected,
            move |mut connection| {
                assert_eq!(
                    connection.stream.write_timeout().unwrap(),
                    Some(POLICY.write_timeout)
                );
                entered.send(connection.reply_connection).unwrap();
                finished.lock().unwrap().recv_timeout(watchdog()).unwrap();
                writeln!(connection.stream, "finished").ok();
            },
        )
        .unwrap();
        (socket, identities, finish)
    }

    fn authenticated_client(socket: &LocalSocket) -> LocalStream {
        let mut client = LocalStream::connect(socket.socket_path()).unwrap();
        client.set_read_timeout(Some(watchdog())).unwrap();
        writeln!(client, "{} reply", socket.key_hex()).unwrap();
        client
    }

    #[test]
    fn closing_for_one_reply_cancels_other_clients_and_preserves_write_timeout() {
        let scratch = Scratch::new();
        let (entered, identities) = mpsc::channel();
        let (finish, finished) = mpsc::channel();
        let finished = Mutex::new(finished);
        let policy = TransportPolicy {
            max_connections: 3,
            handshake_timeout: Duration::from_secs(30),
            ..POLICY
        };
        let mut socket = LocalSocket::open_in(
            scratch.0.clone(),
            Arc::new(SocketKey::generate().unwrap()),
            policy,
            rejected,
            move |mut connection| {
                assert_eq!(
                    connection.stream.write_timeout().unwrap(),
                    Some(POLICY.write_timeout)
                );
                entered.send(connection.reply_connection).unwrap();
                if connection.body == "reply" {
                    finished.lock().unwrap().recv_timeout(watchdog()).unwrap();
                    writeln!(connection.stream, "finished").unwrap();
                } else {
                    connection.reader.read_line(&mut String::new()).ok();
                }
            },
        )
        .unwrap();
        let mut client = authenticated_client(&socket);
        let origin = identities.recv_timeout(watchdog()).unwrap();
        let mut unrelated = LocalStream::connect(socket.socket_path()).unwrap();
        unrelated.set_read_timeout(Some(watchdog())).unwrap();
        writeln!(unrelated, "{} unrelated", socket.key_hex()).unwrap();
        identities.recv_timeout(watchdog()).unwrap();
        let mut partial = LocalStream::connect(socket.socket_path()).unwrap();
        partial.set_read_timeout(Some(watchdog())).unwrap();
        write!(partial, "{} unfinished", socket.key_hex()).unwrap();
        wait_for_count(&socket, 3);
        socket.close_after_reply(Some(&origin));
        #[cfg(unix)]
        {
            assert!(!socket.socket_path().exists());
            assert!(LocalStream::connect(socket.socket_path()).is_err());
        }
        let mut answer = String::new();
        unrelated.read_to_string(&mut answer).unwrap();
        assert!(answer.is_empty());
        if let Err(error) = partial.read_to_string(&mut answer) {
            assert_eq!(error.kind(), io::ErrorKind::ConnectionReset);
        }
        assert!(answer.is_empty() || answer == "denied\n");
        wait_for_count(&socket, 1);
        finish.send(()).unwrap();
        let mut answer = String::new();
        client.read_to_string(&mut answer).unwrap();
        assert_eq!(answer, "finished\n");
        wait_for_count(&socket, 0);
        assert!(LocalStream::connect(socket.socket_path()).is_err());
    }

    #[test]
    fn foreign_and_reopened_reply_identities_cannot_spare_another_connection() {
        let source_dir = Scratch::new();
        let target_dir = Scratch::new();
        let (mut source, identities, finish_source) = gated_reply_socket(&source_dir.0);
        let _source_client = authenticated_client(&source);
        let foreign = identities.recv_timeout(watchdog()).unwrap();
        let (mut target, identities, finish_target) = gated_reply_socket(&target_dir.0);
        let mut target_client = authenticated_client(&target);
        let old_target = identities.recv_timeout(watchdog()).unwrap();
        assert_eq!(foreign.id, old_target.id);
        target.close_after_reply(Some(&foreign));
        #[cfg(windows)]
        finish_target.send(()).unwrap();
        assert_eq!(target_client.read(&mut [0]).unwrap(), 0);
        #[cfg(unix)]
        finish_target.send(()).unwrap();
        wait_for_count(&target, 0);
        drop(target);
        let (mut reopened, identities, finish_reopened) = gated_reply_socket(&target_dir.0);
        let mut reopened_client = authenticated_client(&reopened);
        let new_target = identities.recv_timeout(watchdog()).unwrap();
        assert_eq!(old_target.id, new_target.id);
        reopened.close_after_reply(Some(&old_target));
        #[cfg(windows)]
        finish_reopened.send(()).unwrap();
        assert_eq!(reopened_client.read(&mut [0]).unwrap(), 0);
        #[cfg(unix)]
        finish_reopened.send(()).unwrap();
        wait_for_count(&reopened, 0);
        source.close();
        finish_source.send(()).unwrap();
        wait_for_count(&source, 0);
    }

    #[test]
    fn completed_reply_identity_and_ordinary_close_never_spare_active_clients() {
        let scratch = Scratch::new();
        let (mut socket, identities, finish) = gated_reply_socket(&scratch.0);
        let mut first = authenticated_client(&socket);
        let stale = identities.recv_timeout(watchdog()).unwrap();
        finish.send(()).unwrap();
        first.read_to_string(&mut String::new()).unwrap();
        wait_for_count(&socket, 0);
        let mut second = authenticated_client(&socket);
        let active = identities.recv_timeout(watchdog()).unwrap();
        assert_ne!(stale.id, active.id);
        socket.close_after_reply(Some(&stale));
        #[cfg(windows)]
        finish.send(()).unwrap();
        assert_eq!(second.read(&mut [0]).unwrap(), 0);
        #[cfg(unix)]
        finish.send(()).unwrap();
        wait_for_count(&socket, 0);

        let (mut socket, identities, finish) = gated_reply_socket(&scratch.0);
        let mut client = authenticated_client(&socket);
        let _active = identities.recv_timeout(watchdog()).unwrap();
        socket.close();
        #[cfg(windows)]
        finish.send(()).unwrap();
        assert_eq!(client.read(&mut [0]).unwrap(), 0);
        #[cfg(unix)]
        finish.send(()).unwrap();
        wait_for_count(&socket, 0);
    }

    #[test]
    fn panicking_adapters_return_their_connection_slot() {
        let scratch = Scratch::new();
        let (entered, entry) = mpsc::channel();
        let socket = LocalSocket::open_in(
            scratch.0.clone(),
            Arc::new(SocketKey::generate().unwrap()),
            POLICY,
            rejected,
            move |_| {
                entered.send(()).unwrap();
                panic!("adapter failed");
            },
        )
        .unwrap();
        let mut client = LocalStream::connect(socket.socket_path()).unwrap();
        writeln!(client, "{} hello", socket.key_hex()).unwrap();
        entry.recv_timeout(watchdog()).unwrap();
        wait_for_count(&socket, 0);
    }
    #[cfg(unix)]
    #[test]
    fn closing_after_unlink_cancels_the_listener_and_authenticated_client() {
        let scratch = Scratch::new();
        let (entered, entry) = mpsc::channel();
        let mut socket = LocalSocket::open_in(
            scratch.0.clone(),
            Arc::new(SocketKey::generate().unwrap()),
            POLICY,
            rejected,
            move |mut connection| {
                entered.send(()).unwrap();
                let mut line = String::new();
                connection.reader.read_line(&mut line).ok();
            },
        )
        .unwrap();
        let mut client = LocalStream::connect(socket.socket_path()).unwrap();
        writeln!(client, "{} hello", socket.key_hex()).unwrap();
        entry.recv_timeout(watchdog()).unwrap();
        // macOS refuses socket options once both directions are shut down.
        client.set_read_timeout(Some(watchdog())).unwrap();
        fs::remove_file(socket.socket_path()).unwrap();
        let (done, completed) = mpsc::channel();
        std::thread::spawn(move || {
            socket.close();
            done.send(()).unwrap();
        });
        completed
            .recv_timeout(watchdog())
            .expect("unlinked listener must close");
        assert_eq!(client.read(&mut [0]).unwrap(), 0);
    }

    /// Lowers this process's soft descriptor limit.
    #[cfg(unix)]
    fn lower_descriptor_limit(limit: nix::libc::rlim_t) {
        let mut current = nix::libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        // SAFETY: getrlimit and setrlimit only read and write the struct they
        // are given.
        unsafe {
            assert_eq!(
                nix::libc::getrlimit(nix::libc::RLIMIT_NOFILE, &mut current),
                0
            );
            let lowered = nix::libc::rlimit {
                rlim_cur: current.rlim_cur.min(limit),
                rlim_max: current.rlim_max,
            };
            assert_eq!(nix::libc::setrlimit(nix::libc::RLIMIT_NOFILE, &lowered), 0);
        }
    }

    /// Opens descriptors until the process has none left, then gives one back
    /// for the caller's own end of a connection.
    #[cfg(unix)]
    fn exhaust_descriptors() -> Vec<File> {
        let mut held = Vec::new();
        loop {
            match File::open("/dev/null") {
                Ok(file) => held.push(file),
                Err(error) if error.raw_os_error() == Some(Errno::EMFILE as i32) => break,
                Err(error) => panic!("exhausting descriptors failed another way: {error}"),
            }
        }
        held.pop()
            .expect("at least one descriptor opens under the lowered limit");
        held
    }

    /// CPU time this process has used, user and system together.
    #[cfg(unix)]
    fn cpu_time() -> Duration {
        let mut usage = std::mem::MaybeUninit::<nix::libc::rusage>::zeroed();
        // SAFETY: getrusage fills the struct it is given and reads nothing
        // else; it is fully written when the call returns 0.
        let usage = unsafe {
            assert_eq!(
                nix::libc::getrusage(nix::libc::RUSAGE_SELF, usage.as_mut_ptr()),
                0
            );
            usage.assume_init()
        };
        let time = |value: nix::libc::timeval| {
            Duration::from_secs(value.tv_sec as u64) + Duration::from_micros(value.tv_usec as u64)
        };
        time(usage.ru_utime) + time(usage.ru_stime)
    }

    /// Running out of descriptors is a pause, not the end of the listener.
    ///
    /// `accept` fails with `EMFILE` while a connection waits, and keeps failing
    /// until descriptors are freed. The listener used to stop at the first
    /// failure, leaving a socket nobody served; skipping the failure instead
    /// would spin, because `poll` keeps reporting the waiting connection.
    #[cfg(unix)]
    #[test]
    fn running_out_of_descriptors_rests_the_listener_instead_of_ending_it() {
        const CHILD: &str = "SPRITE_ACCEPT_EXHAUSTION_TEST_CHILD";
        // The descriptor limit is process-wide, so it is lowered in a child
        // running only this test rather than under parallel neighbours.
        if std::env::var_os(CHILD).is_none() {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "local_socket::tests::running_out_of_descriptors_rests_the_listener_instead_of_ending_it",
                    "--nocapture",
                    "--test-threads=1",
                ])
                .env(CHILD, "1")
                .output()
                .unwrap();
            assert!(
                String::from_utf8_lossy(&output.stdout).contains("1 passed"),
                "the subprocess must run and pass its exact test\n{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(
                output.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        use std::os::fd::AsRawFd;

        let scratch = Scratch::new();
        let (bodies, received) = mpsc::channel();
        let mut socket = LocalSocket::open_in(
            scratch.0.clone(),
            Arc::new(SocketKey::generate().unwrap()),
            POLICY,
            rejected,
            move |connection| {
                bodies.send(connection.body).ok();
            },
        )
        .unwrap();
        let key = socket.key_hex();
        let path = socket.socket_path().to_owned();
        // A limit just above what is open now, so exhausting it is quick and
        // freeing it leaves room for an accept and its clones.
        let lowest_free = File::open("/dev/null").unwrap().as_raw_fd();
        lower_descriptor_limit(lowest_free as nix::libc::rlim_t + 32);

        // A connection arrives while no descriptor is free to accept it with.
        let held = exhaust_descriptors();
        let mut first = connect_with_retries(&path);
        writeln!(first, "{key} first").ok();

        // The listener waits that out without spinning.
        let before = cpu_time();
        crate::test_blocking_wait::pause(Duration::from_millis(600));
        // Only meaningful where a failed accept leaves the connection queued
        // (Linux); macOS drops it, so poll has nothing to report there.
        let spent = cpu_time().saturating_sub(before);
        assert!(
            spent < Duration::from_millis(200),
            "the listener spun for {spent:?} of CPU while descriptors were exhausted"
        );

        // Once descriptors return, a connection is served. Linux keeps the one
        // that waited; macOS discards it when `accept` fails, so a second one
        // arrives as well and either proves the listener survived.
        drop(held);
        let mut retry = connect_with_retries(&path);
        writeln!(retry, "{key} retry").ok();
        let body = received
            .recv_timeout(watchdog())
            .expect("the listener must survive running out of descriptors");
        assert!(["first", "retry"].contains(&body.as_str()), "{body}");

        // Closing still ends a listener that is resting.
        let held = exhaust_descriptors();
        // macOS can refuse a connect with ENOTCONN just after descriptors
        // were freed or taken, and discards a connection a failed accept
        // touched, so neither the connect nor the write is required to land.
        let mut second = connect_with_retries(&path);
        writeln!(second, "{key} second").ok();
        crate::test_blocking_wait::pause(Duration::from_millis(250));
        let (done, completed) = mpsc::channel();
        std::thread::spawn(move || {
            socket.close();
            done.send(()).unwrap();
        });
        completed
            .recv_timeout(watchdog())
            .expect("closing must end a resting listener");
        drop(held);
        drop((first, second, retry));
    }

    /// Connects, retrying briefly: macOS can refuse a connect with ENOTCONN
    /// just after descriptors were freed or taken.
    #[cfg(unix)]
    fn connect_with_retries(path: &Path) -> UnixStream {
        (0..20)
            .find_map(|_| {
                UnixStream::connect(path).ok().or_else(|| {
                    crate::test_blocking_wait::pause(Duration::from_millis(10));
                    None
                })
            })
            .expect("all 20 attempts to connect to the listener failed")
    }

    /// An open cancellation peer lets the rest expire; a closed peer reports
    /// cancellation even while the running flag is still set.
    #[cfg(unix)]
    #[test]
    fn a_rest_ends_when_the_socket_closes_and_lasts_a_period_otherwise() {
        let running = AtomicBool::new(true);
        let (cancellation, cancelled) = UnixStream::pair().unwrap();

        let started = Instant::now();
        assert!(!rest(&cancelled, &running));
        let undisturbed = started.elapsed();
        assert!(
            undisturbed >= Duration::from_millis(u64::from(ACCEPT_REST_MS) - 10),
            "{undisturbed:?}"
        );

        drop(cancellation);
        assert!(rest(&cancelled, &running));
        assert!(running.load(Ordering::SeqCst));
    }

    /// Only "nothing was waiting" and an interrupted call go straight back to
    /// `poll`; every other failed `accept` rests first. Checked here as well as
    /// by the exhaustion test, because on macOS a failed accept drops the
    /// waiting connection, so that test cannot see a listener that skips the
    /// rest and spins, and no test can make the other failures happen.
    #[cfg(unix)]
    #[test]
    fn only_a_spurious_or_interrupted_accept_skips_the_rest() {
        let rests = |errno: Errno| rests_after(&io::Error::from_raw_os_error(errno as i32));
        for errno in [
            Errno::EMFILE,
            Errno::ENFILE,
            Errno::ECONNABORTED,
            Errno::ENOBUFS,
            Errno::ENOMEM,
            Errno::EBADF,
            Errno::EINVAL,
        ] {
            assert!(rests(errno), "{errno}");
        }
        assert!(!rests(Errno::EAGAIN) && !rests(Errno::EINTR));
    }

    #[test]
    fn drain_available_reads_buffered_bytes_without_blocking() {
        let (mut reader, mut writer) = LocalStream::pair().unwrap();
        assert_eq!(reader.drain_available().unwrap(), "");
        writer.write_all(b"first\nsecond\n").unwrap();
        // Wait briefly for kernel pipe/socket buffer visibility.
        let deadline = Instant::now() + watchdog();
        let mut drained = String::new();
        while drained.len() < "first\nsecond\n".len() {
            assert!(Instant::now() < deadline, "drained={drained:?}");
            drained.push_str(&reader.drain_available().unwrap());
            if drained.len() < "first\nsecond\n".len() {
                std::thread::yield_now();
            }
        }
        assert_eq!(drained, "first\nsecond\n");
        assert_eq!(reader.drain_available().unwrap(), "");
    }

    #[cfg(windows)]
    #[test]
    fn windows_named_pipe_marker_and_first_instance_security() {
        let scratch = Scratch::new();
        create_private_dir(&scratch.0).unwrap();
        let marker = scratch.0.join("window-win-test.sock");
        let listener = LocalListener::bind(&marker).unwrap();
        let contents = fs::read_to_string(&marker).unwrap();
        assert!(contents.starts_with(r"\\.\pipe\sprite-"), "{contents}");
        assert_eq!(listener.pipe_name(), contents);
        assert!(
            windows_ipc::create_pipe_instance(&contents, true).is_err(),
            "FILE_FLAG_FIRST_PIPE_INSTANCE must reject a second first instance"
        );
        let stale = scratch.0.join("stale.sock");
        fs::write(&stale, r"\\.\pipe\sprite-stale").unwrap();
        sweep_dead_sockets(&scratch.0);
        assert!(marker.exists(), "live locked marker must survive sweep");
        assert!(!stale.exists(), "unlocked stale marker must be swept");
        drop(listener);
        assert!(!marker.exists(), "marker file must be removed on drop");

        let bad_marker = scratch.0.join("bad.sock");
        fs::write(&bad_marker, "127.0.0.1:9999").unwrap();
        let err = LocalStream::connect(&bad_marker).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::ConnectionRefused);
    }
}
