use super::{GroupSignal, signal_group};
use nix::unistd::{getpgrp, getsid};
use std::collections::HashSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Process {
    pid: i32,
    group: i32,
    session: i32,
    birth: u64,
    live: bool,
}

pub(crate) struct SessionProcesses {
    session: i32,
    leader_birth: Option<u64>,
    own_group: i32,
    retired: bool,
}

impl SessionProcesses {
    pub(crate) fn capture(child: u32) -> Option<Self> {
        let session = i32::try_from(child).ok()?;
        let own_session = getsid(None).ok()?.as_raw();
        if session <= 0 || session == own_session {
            return None;
        }
        let leader = read_process(session).ok().flatten();
        if leader.is_some_and(|p| p.session != session) {
            return None;
        }
        // The unreaped child reserves this PID even if it already exited;
        // portable-pty established its session before spawn returned.
        Some(Self {
            session,
            leader_birth: leader.map(|p| p.birth),
            own_group: getpgrp().as_raw(),
            retired: false,
        })
    }

    fn owned(&self, process: &Process) -> bool {
        process.live
            && process.session == self.session
            && process.group > 0
            && process.group != self.own_group
    }

    fn members(&mut self) -> Result<Vec<Process>, ()> {
        if self.retired {
            return Ok(vec![]);
        }
        self.select(scan(self.session))
    }

    fn select(&mut self, snapshot: Result<Vec<Process>, ()>) -> Result<Vec<Process>, ()> {
        if self.retired {
            return Ok(vec![]);
        }
        let all = snapshot?;
        if let Some(leader) = all.iter().find(|p| p.pid == self.session) {
            let Some(birth) = self.leader_birth else {
                return Err(());
            };
            if birth != leader.birth {
                self.retired = true;
                return Ok(vec![]);
            }
        }
        let members: Vec<_> = all.into_iter().filter(|p| self.owned(p)).collect();
        if members.is_empty() {
            self.retired = true;
        }
        Ok(members)
    }

    pub(crate) fn signal(&mut self, signal: &GroupSignal) {
        let Ok(members) = self.members() else {
            return;
        };
        let mut signalled = HashSet::new();
        for member in members {
            if signalled.contains(&member.group) {
                continue;
            }
            if read_process(member.pid)
                .ok()
                .flatten()
                .is_some_and(|now| same_member(member, now) && self.owned(&now))
            {
                signal_group(member.group, signal);
                signalled.insert(member.group);
            }
        }
    }

    pub(crate) fn is_alive(&mut self) -> bool {
        self.members().map_or(true, |members| !members.is_empty())
    }
}

fn same_member(before: Process, after: Process) -> bool {
    before.pid == after.pid
        && before.birth == after.birth
        && before.session == after.session
        && before.group == after.group
        && after.live
}

#[cfg(target_os = "linux")]
fn parse_stat(pid: i32, stat: &str) -> Result<Process, ()> {
    let tail = stat.get(stat.rfind(')').ok_or(())? + 1..).ok_or(())?;
    let fields: Vec<_> = tail.split_whitespace().collect();
    let field = |index| fields.get(index).copied().ok_or(());
    Ok(Process {
        pid,
        group: field(2)?.parse().map_err(|_| ())?,
        session: field(3)?.parse().map_err(|_| ())?,
        birth: field(19)?.parse().map_err(|_| ())?,
        live: !matches!(field(0)?, "Z" | "X" | "x"),
    })
}

#[cfg(target_os = "linux")]
fn read_process(pid: i32) -> Result<Option<Process>, ()> {
    match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(stat) => parse_stat(pid, &stat).map(Some),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(()),
    }
}

#[cfg(target_os = "linux")]
fn scan(session: i32) -> Result<Vec<Process>, ()> {
    let mut processes = Vec::new();
    for entry in std::fs::read_dir("/proc").map_err(|_| ())? {
        let entry = entry.map_err(|_| ())?;
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|s| s.parse::<i32>().ok())
        else {
            continue;
        };
        if let Some(process) = scoped_record(pid, session, process_session, read_process)? {
            processes.push(process);
        }
    }
    Ok(processes)
}

#[cfg(any(target_os = "linux", target_os = "macos", test))]
fn scoped_record(
    pid: i32,
    session: i32,
    mut sid: impl FnMut(i32) -> Result<Option<i32>, ()>,
    mut metadata: impl FnMut(i32) -> Result<Option<Process>, ()>,
) -> Result<Option<Process>, ()> {
    match sid(pid)? {
        Some(found) if found == session => metadata(pid),
        _ => Ok(None),
    }
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
fn read_process(pid: i32) -> Result<Option<Process>, ()> {
    use nix::{libc, unistd::Pid};
    fn bsd(pid: i32) -> Result<libc::proc_bsdinfo, ()> {
        // SAFETY: BSD info is a plain C output record and its exact byte size is supplied.
        let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
        let size = std::mem::size_of_val(&info) as i32;
        // SAFETY: libproc writes only within this BSD info output buffer.
        let count = unsafe {
            libc::proc_pidinfo(
                pid,
                libc::PROC_PIDTBSDINFO,
                0,
                (&mut info as *mut libc::proc_bsdinfo).cast(),
                size,
            )
        };
        if count != size {
            return Err(());
        }
        Ok(info)
    }
    let before = match bsd(pid) {
        Ok(info) => info,
        Err(()) if nix::errno::Errno::last() == nix::errno::Errno::ESRCH => return Ok(None),
        Err(()) => return Err(()),
    };
    let session = getsid(Some(Pid::from_raw(pid))).map_err(|_| ())?.as_raw();
    let after = bsd(pid)?;
    if (
        before.pbi_pid,
        before.pbi_pgid,
        before.pbi_start_tvsec,
        before.pbi_start_tvusec,
    ) != (
        after.pbi_pid,
        after.pbi_pgid,
        after.pbi_start_tvsec,
        after.pbi_start_tvusec,
    ) {
        return Err(());
    }
    Ok(Some(Process {
        pid,
        group: after.pbi_pgid as i32,
        session,
        birth: after.pbi_start_tvsec * 1_000_000 + after.pbi_start_tvusec,
        // Darwin sys/proc.h defines SZOMB as 5; libc exports no named binding.
        live: after.pbi_status != 5,
    }))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn process_session(pid: i32) -> Result<Option<i32>, ()> {
    match getsid(Some(nix::unistd::Pid::from_raw(pid))) {
        Ok(session) => Ok(Some(session.as_raw())),
        Err(nix::errno::Errno::ESRCH) => Ok(None),
        Err(_) => Err(()),
    }
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
fn scan(session: i32) -> Result<Vec<Process>, ()> {
    use nix::libc;
    // SAFETY: a null output asks libproc for a PID count without writing.
    let count = unsafe { libc::proc_listallpids(std::ptr::null_mut(), 0) };
    if count <= 0 {
        return Err(());
    }
    let mut capacity = count as usize + 64;
    for _ in 0..4 {
        let mut pids = vec![0_i32; capacity];
        let bytes = i32::try_from(capacity * std::mem::size_of::<i32>()).map_err(|_| ())?;
        // SAFETY: the output buffer holds exactly the supplied byte capacity.
        let count = unsafe { libc::proc_listallpids(pids.as_mut_ptr().cast(), bytes) };
        if count <= 0 {
            return Err(());
        }
        if count as usize >= capacity {
            capacity *= 2;
            continue;
        }
        let mut processes = Vec::new();
        for pid in pids.into_iter().take(count as usize).filter(|pid| *pid > 0) {
            if let Some(process) = scoped_record(pid, session, process_session, read_process)? {
                processes.push(process);
            }
        }
        return Ok(processes);
    }
    Err(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn adapter_skips_proven_foreign_denied_metadata_without_losing_owned_jobs() {
        let owned = Process {
            pid: 43,
            group: 43,
            session: 42,
            birth: 2,
            live: true,
        };
        let mut records = Vec::new();
        for pid in [1, 43] {
            let record = scoped_record(
                pid,
                42,
                |pid| Ok(Some(if pid == 1 { 1 } else { 42 })),
                |pid| {
                    if pid == 1 { Err(()) } else { Ok(Some(owned)) }
                },
            )
            .expect("foreign protected metadata must not block owned jobs");
            records.extend(record);
        }
        assert_eq!(records, vec![owned]);
        assert!(scoped_record(43, 42, |_| Err(()), |_| Ok(Some(owned))).is_err());
        assert!(scoped_record(43, 42, |_| Ok(Some(42)), |_| Err(())).is_err());
    }
    #[test]
    fn capture_rejects_nonpositive_and_callers_session() {
        assert!(SessionProcesses::capture(0).is_none());
        assert!(SessionProcesses::capture(getsid(None).unwrap().as_raw() as u32).is_none());
    }
    #[test]
    fn selection_excludes_foreign_detached_caller_and_dead_processes() {
        let scope = SessionProcesses {
            session: 42,
            leader_birth: Some(1),
            own_group: 7,
            retired: false,
        };
        let member = Process {
            pid: 43,
            group: 43,
            session: 42,
            birth: 2,
            live: true,
        };
        assert!(scope.owned(&member));
        for other in [
            Process {
                session: 99,
                ..member
            },
            Process { group: 7, ..member },
            Process { group: 0, ..member },
            Process {
                group: -1,
                ..member
            },
            Process {
                live: false,
                ..member
            },
        ] {
            assert!(!scope.owned(&other));
        }
        for other in [
            Process { birth: 3, ..member },
            Process {
                session: 99,
                ..member
            },
            Process {
                group: 99,
                ..member
            },
            Process {
                live: false,
                ..member
            },
        ] {
            assert!(!same_member(member, other));
        }
    }
    #[test]
    fn incomplete_scans_remain_pending_and_empty_scopes_never_revive() {
        let mut scope = SessionProcesses {
            session: 42,
            leader_birth: Some(1),
            own_group: 7,
            retired: false,
        };
        let member = Process {
            pid: 43,
            group: 43,
            session: 42,
            birth: 2,
            live: true,
        };
        assert!(scope.select(Err(())).is_err());
        assert!(!scope.retired);
        assert_eq!(scope.select(Ok(vec![member])).unwrap(), vec![member]);
        assert!(scope.select(Ok(vec![])).unwrap().is_empty());
        assert!(scope.select(Ok(vec![member])).unwrap().is_empty());
    }
    #[test]
    fn changed_leader_identity_retires_scope_and_unknown_identity_stays_pending() {
        let leader = Process {
            pid: 42,
            group: 42,
            session: 42,
            birth: 3,
            live: true,
        };
        let mut scope = SessionProcesses {
            session: 42,
            leader_birth: None,
            own_group: 7,
            retired: false,
        };
        assert!(scope.select(Ok(vec![leader])).is_err());
        assert!(!scope.retired);
        scope.leader_birth = Some(1);
        assert!(scope.select(Ok(vec![leader])).unwrap().is_empty());
        assert!(scope.retired);
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn capture_after_leader_exit_retains_live_ordinary_jobs() {
        use portable_pty::{CommandBuilder, PtySize, native_pty_system};
        use std::time::{Duration, Instant};
        let file = std::env::temp_dir().join(format!("sprite-fast-capture-{}", std::process::id()));
        let pair = native_pty_system().openpty(PtySize::default()).unwrap();
        let mut command = CommandBuilder::new("python3");
        command.args(["-c", "import os,signal,sys,time; p=os.fork();\nif p==0:\n os.setpgid(0,0); signal.signal(signal.SIGHUP,signal.SIG_IGN); open(sys.argv[1],'w').write(str(os.getpid())); time.sleep(30); os._exit(0)\nwhile not os.path.exists(sys.argv[1]): time.sleep(.001)\nos._exit(0)"]);
        command.arg(&file);
        let mut child = pair.slave.spawn_command(command).unwrap();
        let leader = child.process_id().unwrap() as i32;
        let deadline = Instant::now() + Duration::from_secs(3);
        while read_process(leader).unwrap().is_some_and(|p| p.live) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        let exited = read_process(leader).unwrap().unwrap();
        assert!(!exited.live, "leader must exit before capture");
        let member_pid: i32 = std::fs::read_to_string(&file).unwrap().parse().unwrap();
        let member = read_process(member_pid).unwrap().unwrap();
        struct Cleanup(Process, std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                if read_process(self.0.pid)
                    .ok()
                    .flatten()
                    .is_some_and(|now| same_member(self.0, now))
                {
                    let _ = nix::sys::signal::kill(
                        nix::unistd::Pid::from_raw(self.0.pid),
                        nix::sys::signal::Signal::SIGKILL,
                    );
                }
                let _ = std::fs::remove_file(&self.1);
            }
        }
        let _cleanup = Cleanup(member, file);
        let mut scope = SessionProcesses::capture(leader as u32).unwrap();
        assert_eq!(scope.leader_birth, Some(exited.birth));
        child.wait().unwrap();
        assert!(scope.is_alive());
        scope.signal(&GroupSignal::Kill);
        while read_process(member_pid).unwrap().is_some_and(|p| p.live) && Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(!scope.is_alive());
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn stat_parser_handles_parentheses_and_rejects_incomplete_records() {
        let stat = "42 (odd ) name)) S 1 43 42 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 123";
        let parsed = parse_stat(42, stat).unwrap();
        assert_eq!((parsed.group, parsed.session, parsed.birth), (43, 42, 123));
        assert!(parse_stat(42, "42 (broken) Z 1").is_err());
    }
}
