# Own ordinary terminal-session jobs across process groups

A terminal owns ordinary jobs by the process session established by pinned
portable-pty 0.9.0, rather than by ancestry or its shell and foreground groups.
Bash job control creates additional groups, and reparented jobs retain their
session after their shell exits. Deliberate setsid detachment crosses this
ownership boundary and is excluded. Foreground observation retains its shell
group separately; no public session API or Pane cleanup ownership changes.

Capture ownership before the child waiter can reap the leader. The successful
portable-pty spawn contract establishes SID equal to child PID, and an unreaped
child reserves that PID even if it exited before metadata capture. Record leader
birth when available, including Linux zombie metadata. If leader metadata is
unavailable, retain the contract-established session scope rather than dropping
ordinary jobs; an unverified present leader makes scans incomplete until it
vanishes, and a known changed birth retires ownership. This fallback relies on
the pinned spawn contract and does not guess ownership from a later recycled PID.

Every escalation and explicit completion discovers fresh live session members.
Immediately before signaling, reread a member's PID, birth, SID and group;
exclude caller groups, nonpositive groups, foreign sessions and zombies. A
complete empty scope retires permanently. Failed or incomplete scans neither
signal guessed groups nor declare completion, and remain bounded by the existing
shutdown deadline. KILL rescans repeat until completion or deadline to reach
newly created groups. Natural exit still sends one HUP; explicit shutdown keeps
TERM at two seconds, KILL at three seconds and its six-second budget. Existing
natural output drain and off-UI-thread cleanup stay intact. After natural
completion, the worker returns retained process ownership to the existing
shutdown handle through its join result. Blocking ShutdownHandle::wait performs
any remaining explicit escalation on its caller’s background executor. The
worker returns no remaining scope once its closing loop observed an explicit
request, so that policy never spends a second six-second budget. This decision
uses the observed request time, not the final flag: an auto-close request during
natural Exited publication must still receive the retained scope. The handoff
clock starts at begin_shutdown and includes joining the naturally ending worker;
there is no new thread, supervisor or Pane lifecycle owner.

Linux reads numeric /proc entries, excludes proven foreign SIDs with getsid
before reading protected stat metadata, and parses stat after the last closing
parenthesis. This prevents hidepid=1 foreign-process metadata denial from
blocking ordinary-job cleanup. Stat still supplies actual SID, group and birth
for ownership selection and the immediate pre-signal identity check. A failed
getsid, including a security-policy denial, leaves discovery incomplete; only
ESRCH proves disappearance. Darwin uses the existing nix/libc bindings:
proc_listallpids returns PID counts and accepts buffer bytes, with bounded growth for full buffers;
BSD info, getsid and BSD info again establish stable birth and group identity.
Darwin sys/proc.h defines SZOMB as 5. Darwin probes SID with getsid before
requesting protected full BSD metadata,
skipping proven foreign sessions even when their BSD metadata would be denied.
Apple’s current getsid implementation performs no same-user privilege check;
unknown SID or unavailable possibly owned metadata still makes discovery
incomplete. Owned records retain the BSDinfo/getsid/BSDinfo identity sandwich;
permission errors are never discarded without proving foreign ownership.
This environment validates Linux behavior;
Darwin adapter source was checked against pinned bindings and Apple sources but
has not been compiled or run on macOS.

Portable killpg has a residual identity-check-to-signal race, and enumerating
processes is not an atomic session snapshot. Session IDs have no portable birth
token after all original members disappear. Retirement and member checks reduce
ordinary PID/group reuse exposure but do not provide adversarial containment.
A process-tree walk misses reparented jobs; cgroups or a separate supervisor would
change deployment and architecture beyond this repair.

Sources: portable-pty 0.9.0 src/unix.rs pre_exec setsid; libc 0.2.186 Apple
proc_bsdinfo/proc_listallpids/proc_pidinfo bindings;
[Linux procfs mount options](https://docs.kernel.org/filesystems/proc.html#mount-options),
[Linux getsid security hook](https://github.com/torvalds/linux/blob/master/kernel/sys.c),
[Apple libproc implementation](https://github.com/apple-oss-distributions/xnu/blob/main/libsyscall/wrappers/libproc/libproc.c),
[BSD process info](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/sys/proc_info.h),
[process status constants](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/sys/proc.h),
[getsid implementation](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/kern/kern_prot.c),
[BSD-info privilege checks](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/kern/proc_info.c).
