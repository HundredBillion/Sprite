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

    /// Signals every owned group a fresh scan finds, after rereading each
    /// member's identity.
    ///
    /// Returns whether the attempt was complete. `false` means the scan, or a
    /// member's reread, could not finish, so someone who should have received
    /// this signal may not have; escalation must not count it as delivered.
    pub(crate) fn signal(&mut self, signal: &GroupSignal) -> bool {
        let members = self.members();
        self.signal_members(members, read_process, |group| signal_group(group, signal))
    }

    /// The decision half of `signal`, free of the platform's process table.
    fn signal_members(
        &self,
        members: Result<Vec<Process>, ()>,
        mut reread: impl FnMut(i32) -> Result<Option<Process>, ()>,
        mut send: impl FnMut(i32),
    ) -> bool {
        let Ok(members) = members else {
            return false;
        };
        let mut signalled = HashSet::new();
        let mut unreadable = HashSet::new();
        for member in members {
            if signalled.contains(&member.group) {
                continue;
            }
            match reread(member.pid) {
                Ok(Some(now)) if same_member(member, now) && self.owned(&now) => {
                    send(member.group);
                    signalled.insert(member.group);
                }
                // Gone, or a different process now: nothing of this session is
                // left there to signal.
                Ok(_) => {}
                Err(()) => {
                    unreadable.insert(member.group);
                }
            }
        }
        unreadable.iter().all(|group| signalled.contains(group))
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

/// The fields of one BSD-info reading that identify a process.
#[cfg(any(target_os = "macos", test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Reading {
    pid: i32,
    group: i32,
    birth: u64,
    live: bool,
}

/// One process's record from a reading, its session, and a second reading.
///
/// Each step answers `Ok(None)` once the kernel says the process no longer
/// exists, and that ends the record as "gone": a member that exits mid-scan
/// has nothing left to signal, and treating it as a failure would abort the
/// whole scan and signal nobody. Any other failure, or an identity that
/// changed between the readings, leaves the scan incomplete — only
/// disappearance is proof.
#[cfg(any(target_os = "macos", test))]
fn sandwiched_record(
    pid: i32,
    before: Result<Option<Reading>, ()>,
    session: impl FnOnce() -> Result<Option<i32>, ()>,
    after: impl FnOnce() -> Result<Option<Reading>, ()>,
) -> Result<Option<Process>, ()> {
    let Some(before) = before? else {
        return Ok(None);
    };
    let Some(session) = session()? else {
        return Ok(None);
    };
    let Some(after) = after()? else {
        return Ok(None);
    };
    if (before.pid, before.group, before.birth) != (after.pid, after.group, after.birth) {
        return Err(());
    }
    Ok(Some(Process {
        pid,
        group: after.group,
        session,
        birth: after.birth,
        live: after.live,
    }))
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
fn read_process(pid: i32) -> Result<Option<Process>, ()> {
    use nix::libc;
    fn bsd(pid: i32) -> Result<Option<Reading>, ()> {
        // SAFETY: BSD info is a plain C output record and its exact byte size is supplied.
        let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
        let size = std::mem::size_of_val(&info) as i32;
        // A short reading that sets no errno must not inherit an earlier
        // ESRCH and pass for a vanished process.
        nix::errno::Errno::clear();
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
        if count == size {
            return Ok(Some(Reading {
                pid: info.pbi_pid as i32,
                group: info.pbi_pgid as i32,
                birth: info.pbi_start_tvsec * 1_000_000 + info.pbi_start_tvusec,
                // Darwin sys/proc.h defines SZOMB as 5; libc exports no named binding.
                live: info.pbi_status != 5,
            }));
        }
        if nix::errno::Errno::last() == nix::errno::Errno::ESRCH {
            Ok(None)
        } else {
            Err(())
        }
    }
    sandwiched_record(pid, bsd(pid), || process_session(pid), || bsd(pid))
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
    fn a_process_that_vanishes_mid_read_is_gone_not_an_incomplete_scan() {
        let reading = Reading {
            pid: 43,
            group: 43,
            birth: 2,
            live: true,
        };
        let complete = Process {
            pid: 43,
            group: 43,
            session: 42,
            birth: 2,
            live: true,
        };
        assert_eq!(
            sandwiched_record(43, Ok(None), || panic!("not asked"), || panic!("not read")),
            Ok(None)
        );
        assert_eq!(
            sandwiched_record(43, Ok(Some(reading)), || Ok(None), || panic!("not read")),
            Ok(None),
            "gone before its session was read"
        );
        assert_eq!(
            sandwiched_record(43, Ok(Some(reading)), || Ok(Some(42)), || Ok(None)),
            Ok(None),
            "gone before the second reading"
        );
        assert_eq!(
            sandwiched_record(43, Ok(Some(reading)), || Ok(Some(42)), || Ok(Some(reading))),
            Ok(Some(complete))
        );
        // Only disappearance is proof; anything else leaves the scan incomplete.
        assert_eq!(
            sandwiched_record(
                43,
                Ok(Some(reading)),
                || Ok(Some(42)),
                || {
                    Ok(Some(Reading {
                        birth: 3,
                        ..reading
                    }))
                }
            ),
            Err(())
        );
        assert_eq!(
            sandwiched_record(43, Ok(Some(reading)), || Err(()), || Ok(Some(reading))),
            Err(())
        );
        assert_eq!(
            sandwiched_record(43, Err(()), || Ok(Some(42)), || Ok(Some(reading))),
            Err(())
        );
    }

    #[test]
    fn a_signal_attempt_skips_vanished_members_and_reports_unreached_ones() {
        let scope = SessionProcesses {
            session: 42,
            leader_birth: Some(1),
            own_group: 7,
            retired: false,
        };
        let gone = Process {
            pid: 43,
            group: 43,
            session: 42,
            birth: 2,
            live: true,
        };
        let present = Process {
            pid: 44,
            group: 44,
            session: 42,
            birth: 3,
            live: true,
        };
        let sibling = Process {
            pid: 45,
            group: 44,
            session: 42,
            birth: 4,
            live: true,
        };

        let mut sent = Vec::new();
        assert!(
            scope.signal_members(
                Ok(vec![gone, present]),
                |pid| Ok((pid == present.pid).then_some(present)),
                |group| sent.push(group),
            ),
            "a member that vanished has nothing left to signal"
        );
        assert_eq!(sent, vec![44]);

        let mut sent = Vec::new();
        assert!(!scope.signal_members(Err(()), |_| unreachable!(), |group| sent.push(group)));
        assert!(sent.is_empty(), "an incomplete scan signals nobody");

        let mut sent = Vec::new();
        assert!(
            !scope.signal_members(
                Ok(vec![gone, present]),
                |pid| if pid == gone.pid {
                    Err(())
                } else {
                    Ok(Some(present))
                },
                |group| sent.push(group),
            ),
            "a member whose identity could not be read was not reached"
        );
        assert_eq!(
            sent,
            vec![44],
            "the members that could be reached still are"
        );

        let mut sent = Vec::new();
        assert!(
            scope.signal_members(
                Ok(vec![sibling, present]),
                |pid| if pid == sibling.pid {
                    Err(())
                } else {
                    Ok(Some(present))
                },
                |group| sent.push(group),
            ),
            "an unreadable member whose group was signalled through another was reached"
        );
        assert_eq!(sent, vec![44]);
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
