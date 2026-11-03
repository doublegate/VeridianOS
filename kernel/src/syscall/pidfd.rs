//! pidfd_open, pidfd_send_signal and pidfd_getfd, as Linux 7.3's
//! `kernel/pid.c` and `kernel/signal.c`; the pidfd file is
//! [`crate::fs::pidfd`]'s. waitid(P_PIDFD) and clone(CLONE_PIDFD) use
//! [`install`] and [`target`].

use alloc::sync::Arc;

use super::{SyscallError, SyscallResult};
use crate::{
    fs::{
        file::{File, OpenFlags},
        pidfd::{PidFdNode, PIDFD_NONBLOCK, PIDFD_THREAD},
    },
    process::{self, Process, ProcessId, ThreadId},
};

/// `PIDFD_SELF_THREAD`: pidfd_send_signal's name for the calling thread.
const PIDFD_SELF_THREAD: i32 = -10000;
/// `PIDFD_SELF_THREAD_GROUP`: its name for the calling process.
const PIDFD_SELF_THREAD_GROUP: i32 = -10001;
/// pidfd_send_signal's scope flags.
const PIDFD_SIGNAL_THREAD: u32 = 1 << 0;
const PIDFD_SIGNAL_THREAD_GROUP: u32 = 1 << 1;
const PIDFD_SIGNAL_PROCESS_GROUP: u32 = 1 << 2;

/// Put a pidfd for `process` (for its thread `tid` with PIDFD_THREAD) in
/// the caller's file table: read-write and close-on-exec, as every pidfd
/// is, and O_NONBLOCK if asked.
pub(super) fn install(
    process: &Arc<Process>,
    tid: Option<ThreadId>,
    nonblock: bool,
) -> SyscallResult {
    install_node(Arc::new(PidFdNode::new(process, tid)), nonblock)
}

/// Put the pidfd `node` in the caller's file table, as [`install`] says.
fn install_node(node: Arc<PidFdNode>, nonblock: bool) -> SyscallResult {
    let mut flags = OpenFlags::read_write();
    flags.nonblock = nonblock;
    let file = File::new(node, flags);
    let caller = process::current_process().ok_or(SyscallError::InvalidState)?;
    let fd = caller
        .file_table
        .lock()
        .open_with_flags(Arc::new(file), true)
        .map_err(super::map_kernel_error)?;
    Ok(fd)
}

/// clone(CLONE_PIDFD): [`install`] a pidfd and store its number at the
/// caller's `ptr` (an int); a store that faults closes it again (EFAULT).
/// With `autokill` (CLONE_PIDFD_AUTOKILL) its last close SIGKILLs the
/// process, armed only once the store is made.
pub(super) fn install_at(
    process: &Arc<Process>,
    tid: Option<ThreadId>,
    ptr: usize,
    autokill: bool,
) -> SyscallResult {
    let node = Arc::new(PidFdNode::new(process, tid));
    let fd = install_node(node.clone(), false)?;
    if super::userspace::write_user::<i32>(ptr, fd as i32).is_err() {
        uninstall(fd);
        return Err(SyscallError::InvalidPointer);
    }
    if autokill {
        node.arm_autokill();
    }
    Ok(fd)
}

/// Close a pidfd [`install`] made, when the call that made it fails.
pub(super) fn uninstall(fd: usize) {
    if let Some(caller) = process::current_process() {
        let _ = caller.file_table.lock().close(fd);
    }
}

/// The pidfd `fd` is, with its open file (whose O_NONBLOCK waitid uses):
/// EBADF for no open file or one that is not a pidfd, as Linux's
/// pidfd_pid.
pub(super) fn target(fd: usize) -> Result<(Arc<File>, ProcessId, Option<ThreadId>), SyscallError> {
    let file = super::file_calls::open_file(fd)?;
    let node = file
        .node
        .as_any()
        .and_then(|any| any.downcast_ref::<PidFdNode>())
        .ok_or(SyscallError::BadFileDescriptor)?;
    let (pid, tid) = (node.pid(), node.thread());
    Ok((file, pid, tid))
}

/// pidfd_open(pid, flags): a pidfd for process `pid`, or with PIDFD_THREAD
/// for thread `pid`. Other flags than PIDFD_NONBLOCK and PIDFD_THREAD, or
/// a pid not above 0, are EINVAL; no such process (or one already reaped)
/// ESRCH; a thread that does not lead its process, without PIDFD_THREAD,
/// ENOENT. An exited process not yet reaped can be opened.
pub fn sys_pidfd_open(pid: usize, flags: usize) -> SyscallResult {
    let flags = flags as u32;
    if flags & !(PIDFD_NONBLOCK | PIDFD_THREAD) != 0 {
        return Err(SyscallError::InvalidArgument);
    }
    let id = pid as u32 as i32;
    if id <= 0 {
        return Err(SyscallError::InvalidArgument);
    }
    let id = id as u64;
    let nonblock = flags & PIDFD_NONBLOCK != 0;
    if let Some(process) = process::get_process(ProcessId(id)) {
        let tid = (flags & PIDFD_THREAD != 0).then_some(ThreadId(id));
        return install(&process, tid, nonblock);
    }
    // Not a process: a thread of one, which only PIDFD_THREAD names.
    let owner = super::process::process_of_thread(id as usize)
        .and_then(process::get_process)
        .ok_or(SyscallError::ProcessNotFound)?;
    if flags & PIDFD_THREAD == 0 {
        return Err(SyscallError::ResourceNotFound);
    }
    install(&owner, Some(ThreadId(id)), nonblock)
}

/// Where pidfd_send_signal sends.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Scope {
    Thread,
    Process,
    Group,
}

/// pidfd_send_signal(pidfd, sig, info, flags): `sig` to the process the
/// pidfd names (to its thread for a PIDFD_THREAD pidfd), or as the one
/// flag given asks, to the thread, the process or its process group; the
/// calling thread or process for PIDFD_SELF_THREAD and
/// PIDFD_SELF_THREAD_GROUP; an open `/proc/<pid>` directory names its
/// process as a pidfd does. Unknown flags or more than one are EINVAL, any
/// other file EBADF. With `info`, its si_signo must be `sig`
/// (EINVAL) and a si_code the kernel keeps (SI_USER, SI_KERNEL, SI_TKILL)
/// is EPERM except to the caller itself; otherwise the target sees
/// SI_USER (SI_TKILL for a thread) from the caller. Permission is kill's
/// (EPERM); an exited process is ESRCH.
pub fn sys_pidfd_send_signal(pidfd: usize, sig: usize, info: usize, flags: usize) -> SyscallResult {
    use crate::process::signals::{SigInfo, NSIG, SI_TKILL, SI_USER};
    let flags = flags as u32;
    let all = PIDFD_SIGNAL_THREAD | PIDFD_SIGNAL_THREAD_GROUP | PIDFD_SIGNAL_PROCESS_GROUP;
    if flags & !all != 0 || flags.count_ones() > 1 {
        return Err(SyscallError::InvalidArgument);
    }
    let caller = process::current_process().ok_or(SyscallError::InvalidState)?;
    let (pid, mut scope, tid) = match pidfd as u32 as i32 {
        PIDFD_SELF_THREAD => {
            let me = process::current_thread().ok_or(SyscallError::InvalidState)?;
            (caller.pid, Scope::Thread, Some(me.tid))
        }
        PIDFD_SELF_THREAD_GROUP => (caller.pid, Scope::Process, None),
        _ => match target(pidfd) {
            Ok((_, pid, tid)) => {
                let scope = if tid.is_some() {
                    Scope::Thread
                } else {
                    Scope::Process
                };
                (pid, scope, tid)
            }
            // A /proc/<pid> directory names its process too.
            Err(SyscallError::BadFileDescriptor) => {
                let file = super::file_calls::open_file(pidfd)?;
                let pid = crate::fs::procfs::process_dir_pid(&*file.node)
                    .ok_or(SyscallError::BadFileDescriptor)?;
                (ProcessId(pid), Scope::Process, None)
            }
            Err(e) => return Err(e),
        },
    };
    scope = match flags {
        PIDFD_SIGNAL_THREAD => Scope::Thread,
        PIDFD_SIGNAL_THREAD_GROUP => Scope::Process,
        PIDFD_SIGNAL_PROCESS_GROUP => Scope::Group,
        _ => scope,
    };
    let sig = sig as u32 as i32;
    if !(0..=NSIG as i32).contains(&sig) {
        return Err(SyscallError::InvalidArgument);
    }
    let info = if info != 0 {
        let mut given: SigInfo = super::userspace::read_user(info)?;
        if given.si_signo != sig {
            return Err(SyscallError::InvalidArgument);
        }
        let own = pid == caller.pid && scope != Scope::Group;
        if !own && (given.si_code >= 0 || given.si_code == SI_TKILL) {
            return Err(SyscallError::OperationNotPermitted);
        }
        given.si_signo = sig;
        given
    } else {
        let code = if scope == Scope::Thread {
            SI_TKILL
        } else {
            SI_USER
        };
        SigInfo::from_process(sig as usize, code, caller.pid.0, caller.credentials().ruid)
    };
    let target = process::get_process(pid).ok_or(SyscallError::ProcessNotFound)?;
    match scope {
        Scope::Group => {
            let pgid = target.pgid.load(core::sync::atomic::Ordering::Acquire);
            super::filesystem::signal_matching(&caller, sig, Some(info), |p| {
                p.pgid.load(core::sync::atomic::Ordering::Acquire) == pgid
            })
        }
        Scope::Process | Scope::Thread => {
            // Existence and permission as kill (sig 0 stops there).
            super::filesystem::kill_one(&caller, pid, 0)?;
            if sig == 0 {
                return Ok(0);
            }
            let thread = match (scope, tid) {
                (Scope::Thread, Some(tid)) => target.get_thread(tid),
                // PIDFD_SIGNAL_THREAD on a process pidfd: its leader.
                (Scope::Thread, None) => target.get_thread(ThreadId(pid.0)),
                _ => None,
            };
            if scope == Scope::Thread && thread.is_none() {
                return Err(SyscallError::ProcessNotFound);
            }
            super::signal::deliver(&target, thread.as_ref(), info)
        }
    }
}

/// pidfd_getfd(pidfd, fd, flags): a copy, close-on-exec, of descriptor
/// `fd` of the process the pidfd names, for a caller that may trace it
/// (Linux's PTRACE_MODE_ATTACH_REALCREDS: EPERM otherwise). Flags other
/// than 0 are EINVAL, a bad pidfd EBADF, an exited process ESRCH, and a
/// descriptor it does not have EBADF.
pub fn sys_pidfd_getfd(pidfd: usize, fd: usize, flags: usize) -> SyscallResult {
    if flags as u32 != 0 {
        return Err(SyscallError::InvalidArgument);
    }
    let (_, pid, _) = target(pidfd)?;
    let target = process::get_process(pid).ok_or(SyscallError::ProcessNotFound)?;
    let caller = process::creds::Caller::current();
    let permitted = super::debug::current_permitted();
    // Checked and taken under the target's credential guard, as Linux
    // holds exec_update_lock: a set-ID exec cannot install its credentials
    // between the check and the lookup, so no descriptor the privileged
    // image opens is handed out on the old credentials' check.
    let file = {
        let _guard = target.cred_guard.lock();
        if !super::debug::may_access_process(&caller, permitted, &target) {
            return Err(SyscallError::OperationNotPermitted);
        }
        if !target.is_alive() {
            return Err(SyscallError::ProcessNotFound);
        }
        target
            .file_table
            .lock()
            .get(fd as u32 as usize)
            .ok_or(SyscallError::BadFileDescriptor)?
    };
    let me = process::current_process().ok_or(SyscallError::InvalidState)?;
    let fd = me
        .file_table
        .lock()
        .open_with_flags(file, true)
        .map_err(super::map_kernel_error)?;
    Ok(fd)
}
