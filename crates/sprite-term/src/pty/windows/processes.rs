//! Win32 Job Object ownership and basename-only process queries for Windows.
//!
//! Each terminal session places its initial child inside a dedicated Job Object
//! configured with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`. Windows 8+ supports
//! nested jobs, so descendants automatically inherit membership and bounded
//! shutdown terminates the whole session tree without PID-reuse races.

use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::sync::Arc;

use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, IsProcessInJob, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_BASIC_PROCESS_ID_LIST, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JobObjectBasicProcessIdList, JobObjectExtendedLimitInformation, QueryInformationJobObject,
    SetInformationJobObject, TerminateJobObject,
};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_FORMAT, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SET_QUOTA,
    PROCESS_TERMINATE, QueryFullProcessImageNameW,
};
use windows::core::PWSTR;

use super::GroupSignal;

/// Console host helpers attached to ConPTY that are not user programs.
fn is_console_infrastructure(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "conhost.exe" | "openconsole.exe"
    )
}

/// Owns the Win32 Job Object tracking every process started inside one session.
pub(crate) struct SessionProcesses {
    job: Arc<OwnedHandle>,
    session: i32,
}

impl SessionProcesses {
    #[cfg(test)]
    #[allow(unsafe_code, reason = "creates an anonymous Job Object for unit tests")]
    pub(crate) fn for_test(session: i32) -> Option<Self> {
        let raw_job = unsafe { CreateJobObjectW(None, None) }.ok()?;
        let job = unsafe { OwnedHandle::from_raw_handle(raw_job.0) };
        Some(Self {
            job: Arc::new(job),
            session,
        })
    }

    pub(crate) fn session_id(&self) -> i32 {
        self.session
    }

    pub(crate) fn job_handle(&self) -> Arc<OwnedHandle> {
        Arc::clone(&self.job)
    }

    #[allow(
        unsafe_code,
        reason = "Win32 Job Object creation and child assignment FFI"
    )]
    pub(crate) fn capture(child: u32) -> Option<Self> {
        let session = i32::try_from(child).ok()?;
        if session <= 0 {
            return None;
        }
        // SAFETY: `CreateJobObjectW(None, None)` creates an anonymous job owned by this handle.
        let raw_job = unsafe { CreateJobObjectW(None, None) }.ok()?;
        // SAFETY: `raw_job` is a freshly created valid handle transferred exclusively here.
        let job = unsafe { OwnedHandle::from_raw_handle(raw_job.0) };

        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        // SAFETY: `info` is valid for `size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>()` bytes.
        unsafe {
            SetInformationJobObject(
                HANDLE(job.as_raw_handle()),
                JobObjectExtendedLimitInformation,
                std::ptr::from_ref(&info).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        }
        .ok()?;

        // SAFETY: opens the spawned child PID with minimum required rights for job assignment.
        let raw_process = unsafe {
            OpenProcess(
                PROCESS_SET_QUOTA | PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION,
                false,
                child,
            )
        }
        .ok()?;
        // SAFETY: `raw_process` is a fresh handle closed when `process` drops.
        let process = unsafe { OwnedHandle::from_raw_handle(raw_process.0) };

        // SAFETY: both handles are valid and live for the duration of the call.
        unsafe {
            AssignProcessToJobObject(HANDLE(job.as_raw_handle()), HANDLE(process.as_raw_handle()))
        }
        .ok()?;

        Some(Self {
            job: Arc::new(job),
            session,
        })
    }

    #[allow(unsafe_code, reason = "Win32 TerminateJobObject FFI")]
    pub(crate) fn signal(&mut self, signal: &GroupSignal) -> bool {
        match signal {
            // Dropping `MasterPty` closes the ConPTY pseudo-console and sends
            // `CTRL_CLOSE_EVENT` to attached console processes, which is the
            // Windows equivalent of SIGHUP.
            GroupSignal::Hangup => true,
            GroupSignal::Terminate | GroupSignal::Kill => {
                // SAFETY: `self.job` holds a live Job Object handle.
                unsafe { TerminateJobObject(HANDLE(self.job.as_raw_handle()), 1) }.ok();
                true
            }
        }
    }

    pub(crate) fn is_alive(&mut self) -> bool {
        let Some(pids) = job_process_ids(&self.job) else {
            return false;
        };
        pids.into_iter().any(|pid| {
            let Some(pid_i32) = i32::try_from(pid).ok() else {
                return true;
            };
            match process_name(pid_i32) {
                Some(name) => !is_console_infrastructure(&name),
                None => true,
            }
        })
    }
}

/// Lists every process ID currently assigned to `job`.
#[allow(unsafe_code, reason = "Win32 QueryInformationJobObject FFI")]
pub(crate) fn job_process_ids(job: &OwnedHandle) -> Option<Vec<u32>> {
    let mut capacity = 32_usize;
    loop {
        let header_size = size_of::<JOBOBJECT_BASIC_PROCESS_ID_LIST>();
        let extra = capacity.saturating_sub(1) * size_of::<usize>();
        let byte_len = header_size + extra;
        let word_len = byte_len.div_ceil(size_of::<usize>());
        let mut storage = vec![0_usize; word_len];
        let mut returned = 0_u32;

        // SAFETY: `storage` is properly aligned for `JOBOBJECT_BASIC_PROCESS_ID_LIST`
        // and holds at least `byte_len` bytes.
        let result = unsafe {
            QueryInformationJobObject(
                Some(HANDLE(job.as_raw_handle())),
                JobObjectBasicProcessIdList,
                storage.as_mut_ptr().cast(),
                (word_len * size_of::<usize>()) as u32,
                Some(&mut returned),
            )
        };
        // SAFETY: `storage` starts with a valid `JOBOBJECT_BASIC_PROCESS_ID_LIST` header.
        let header = unsafe { &*(storage.as_ptr().cast::<JOBOBJECT_BASIC_PROCESS_ID_LIST>()) };
        let assigned = header.NumberOfAssignedProcesses as usize;
        let listed = header.NumberOfProcessIdsInList as usize;
        if assigned > capacity && capacity < 4096 {
            capacity = (assigned + 8).min(4096);
            continue;
        }
        if result.is_err() && listed == 0 {
            return None;
        }
        let ids_ptr = std::ptr::addr_of!(header.ProcessIdList) as *const usize;
        // SAFETY: the kernel wrote `listed` entries into the trailing array within `storage`.
        let slice = unsafe { std::slice::from_raw_parts(ids_ptr, listed) };
        let pids = slice
            .iter()
            .filter_map(|raw| u32::try_from(*raw).ok())
            .collect();
        return Some(pids);
    }
}

/// Whether `pid` currently belongs to `job`.
#[allow(unsafe_code, reason = "Win32 OpenProcess + IsProcessInJob FFI")]
pub(crate) fn is_process_in_job(job: &OwnedHandle, pid: u32) -> bool {
    // SAFETY: opens `pid` read-only with limited query access.
    let Ok(raw_process) = (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) })
    else {
        return false;
    };
    // SAFETY: `raw_process` is a fresh handle closed on drop.
    let process = unsafe { OwnedHandle::from_raw_handle(raw_process.0) };
    let mut in_job = windows::core::BOOL(0);
    // SAFETY: both handles are valid and `in_job` is a valid out-pointer.
    unsafe {
        IsProcessInJob(
            HANDLE(process.as_raw_handle()),
            Some(HANDLE(job.as_raw_handle())),
            &mut in_job,
        )
    }
    .is_ok()
        && in_job.as_bool()
}

/// The basename of the executable `pid` is running, and nothing else.
///
/// Uses `QueryFullProcessImageNameW` and returns only the final path component —
/// never command-line arguments and never environment variables.
#[allow(
    unsafe_code,
    reason = "Win32 OpenProcess + QueryFullProcessImageNameW FFI"
)]
pub(crate) fn process_name(pid: i32) -> Option<String> {
    let pid = u32::try_from(pid).ok()?;
    // SAFETY: opens `pid` with query-limited access only.
    let raw_process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?;
    // SAFETY: `raw_process` is owned exclusively by `process`.
    let process = unsafe { OwnedHandle::from_raw_handle(raw_process.0) };

    let mut buffer = vec![0_u16; 512];
    let mut len = buffer.len() as u32;
    // SAFETY: `buffer` is valid for `len` UTF-16 code units.
    unsafe {
        QueryFullProcessImageNameW(
            HANDLE(process.as_raw_handle()),
            PROCESS_NAME_FORMAT(0),
            PWSTR(buffer.as_mut_ptr()),
            &mut len,
        )
    }
    .ok()?;
    let full = OsString::from_wide(&buffer[..len as usize]);
    let full = full.to_string_lossy();
    let basename = full.rsplit(['\\', '/']).next()?.trim();
    (!basename.is_empty()).then(|| basename.to_owned())
}
