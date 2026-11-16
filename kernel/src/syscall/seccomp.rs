//! seccomp(2) and prctl's PR_SET_SECCOMP/PR_GET_SECCOMP (Linux's
//! `kernel/seccomp.c`), the check every system call passes ([`check`],
//! called at the top of dispatch), and the user-notification listener
//! ([`NotifyNode`] and its ioctls). The filters themselves are
//! `process::seccomp`'s.

use alloc::{sync::Arc, vec::Vec};
use core::sync::atomic::Ordering;

use spin::Once;

use super::{userspace, SyscallError, SyscallResult};
use crate::{
    error::{FsError, KernelError},
    fs::{
        file::{File, OpenFlags},
        DirEntry, Metadata, NodeType, Permissions, VfsNode,
    },
    process::{
        creds::Caller,
        linux_caps::CAP_SYS_ADMIN,
        seccomp::{self as sc, errno, ret, AddFd, Filter, NotifState, TargetStep},
        thread::ThreadState,
    },
};

/// seccomp(2)'s operations.
const SET_MODE_STRICT: usize = 0;
const SET_MODE_FILTER: usize = 1;
const GET_ACTION_AVAIL: usize = 2;
const GET_NOTIF_SIZES: usize = 3;

/// SECCOMP_FILTER_FLAG_*.
const FLAG_TSYNC: usize = 1 << 0;
const FLAG_LOG: usize = 1 << 1;
const FLAG_SPEC_ALLOW: usize = 1 << 2;
const FLAG_NEW_LISTENER: usize = 1 << 3;
const FLAG_TSYNC_ESRCH: usize = 1 << 4;
const FLAG_WAIT_KILLABLE_RECV: usize = 1 << 5;
const FLAGS_KNOWN: usize = FLAG_TSYNC
    | FLAG_LOG
    | FLAG_SPEC_ALLOW
    | FLAG_NEW_LISTENER
    | FLAG_TSYNC_ESRCH
    | FLAG_WAIT_KILLABLE_RECV;

/// SIGSYS and its si_code for a seccomp TRAP (SYS_SECCOMP).
const SIGSYS: usize = 31;
const SYS_SECCOMP: i32 = 1;
/// The largest errno ERRNO returns (MAX_ERRNO).
const MAX_ERRNO: u32 = 4095;
/// EINTR as a return value.
const EINTR_RET: isize = -(errno::EINTR as isize);

/// The listener's ioctls (`include/uapi/linux/seccomp.h`, type '!').
const IOCTL_NOTIF_RECV: u32 = 0xc050_2100;
const IOCTL_NOTIF_SEND: u32 = 0xc018_2101;
const IOCTL_NOTIF_ID_VALID: u32 = 0x4008_2102;
/// The same, with the direction bits it had before Linux 5.17.
const IOCTL_NOTIF_ID_VALID_WRONG_DIR: u32 = 0x8008_2102;
const IOCTL_NOTIF_SET_FLAGS: u32 = 0x4008_2104;
/// NOTIF_ADDFD, by type and number only: it is extensible, its size in the
/// command (EA_IOCTL).
const IOCTL_NOTIF_ADDFD_NR: u32 = 0x2103;
const IOC_SIZE_SHIFT: u32 = 16;
const IOC_SIZE_MASK: u32 = 0x3fff;
const IOC_INOUT: u32 = 0xc000_0000;

/// The structures' sizes: seccomp_notif, seccomp_notif_resp,
/// seccomp_notif_addfd (SECCOMP_NOTIFY_ADDFD_SIZE_VER0 and LATEST).
const NOTIF_SIZE: usize = 80;
const RESP_SIZE: usize = 24;
const ADDFD_SIZE: usize = 24;

/// SECCOMP_ADDFD_FLAG_*.
const ADDFD_FLAG_SETFD: u32 = 1 << 0;
const ADDFD_FLAG_SEND: u32 = 1 << 1;
/// O_CLOEXEC, the only flag a new descriptor takes.
const O_CLOEXEC: u32 = 0o2000000;

/// seccomp(op, flags, uargs).
pub fn sys_seccomp(op: usize, flags: usize, uargs: usize) -> SyscallResult {
    let flags = flags as u32 as usize;
    match op as u32 as usize {
        SET_MODE_STRICT => {
            if flags != 0 || uargs != 0 {
                return Err(SyscallError::InvalidArgument);
            }
            set_strict()
        }
        SET_MODE_FILTER => set_filter(flags, uargs),
        GET_ACTION_AVAIL => {
            if flags != 0 {
                return Err(SyscallError::InvalidArgument);
            }
            let action: u32 = userspace::read_user(uargs)?;
            if sc::action_known(action) {
                Ok(0)
            } else {
                Err(SyscallError::NotSupported)
            }
        }
        GET_NOTIF_SIZES => {
            if flags != 0 {
                return Err(SyscallError::InvalidArgument);
            }
            // struct seccomp_notif_sizes: seccomp_notif, seccomp_notif_resp,
            // seccomp_data.
            userspace::write_user(uargs, [NOTIF_SIZE as u16, RESP_SIZE as u16, 64])?;
            Ok(0)
        }
        _ => Err(SyscallError::InvalidArgument),
    }
}

/// prctl(PR_GET_SECCOMP): the calling thread's mode.
pub fn prctl_get() -> SyscallResult {
    let thread = crate::process::current_thread().ok_or(SyscallError::InvalidState)?;
    Ok(thread.seccomp.mode() as usize)
}

/// The return register after a TRAP, as Linux's syscall_rollback leaves
/// it: the call number on x86_64, the first argument (seccomp_data's
/// args[0], bytes 16..24) elsewhere.
fn rolled_back(nr: usize, sd: &[u8; 64]) -> isize {
    if cfg!(target_arch = "x86_64") {
        nr as isize
    } else {
        let mut a0 = [0u8; 8];
        a0.copy_from_slice(&sd[16..24]);
        u64::from_ne_bytes(a0) as isize
    }
}

/// prctl(PR_SET_SECCOMP, mode, filter): strict, or a filter without flags.
/// Strict mode ignores `filter`, as prctl_set_seccomp sets it to NULL: a
/// variadic `prctl(PR_SET_SECCOMP, SECCOMP_MODE_STRICT)` passes whatever
/// the register held (PR #49 review).
pub fn prctl_set(mode: usize, filter: usize) -> SyscallResult {
    match mode {
        1 => set_strict(),
        2 => set_filter(0, filter),
        _ => Err(SyscallError::InvalidArgument),
    }
}

fn set_strict() -> SyscallResult {
    let process = crate::process::current_process().ok_or(SyscallError::InvalidState)?;
    let thread = crate::process::current_thread().ok_or(SyscallError::InvalidState)?;
    // Under the thread map, as every change of a thread's mode
    // (Linux: siglock).
    let _threads = process.threads.lock();
    thread
        .seccomp
        .set_strict()
        .map(|()| 0)
        .map_err(|_| SyscallError::InvalidArgument)
}

/// Whether thread `t` is past the point where seccomp applies to it
/// (PF_EXITING): TSYNC neither checks nor changes it.
fn exiting(t: &crate::process::thread::Thread) -> bool {
    matches!(t.get_state(), ThreadState::Zombie | ThreadState::Dead)
}

/// SECCOMP_SET_MODE_FILTER (seccomp_set_mode_filter).
fn set_filter(flags: usize, uargs: usize) -> SyscallResult {
    if flags & !FLAGS_KNOWN != 0 {
        return Err(SyscallError::InvalidArgument);
    }
    // TSYNC's thread id on failure and NEW_LISTENER's fd share the return
    // value, so together they need TSYNC_ESRCH.
    if flags & FLAG_TSYNC != 0 && flags & FLAG_NEW_LISTENER != 0 && flags & FLAG_TSYNC_ESRCH == 0 {
        return Err(SyscallError::InvalidArgument);
    }
    if flags & FLAG_WAIT_KILLABLE_RECV != 0 && flags & FLAG_NEW_LISTENER == 0 {
        return Err(SyscallError::InvalidArgument);
    }
    let prog = read_prog(uargs)?;
    let process = crate::process::current_process().ok_or(SyscallError::InvalidState)?;
    let thread = crate::process::current_thread().ok_or(SyscallError::InvalidState)?;
    // Without no_new_privs, a filter needs CAP_SYS_ADMIN in the caller's
    // user namespace (seccomp_prepare_filter): EACCES.
    let me = Caller::current();
    if !thread.no_new_privs.load(Ordering::Acquire) && !me.ns_capable(&me.userns, CAP_SYS_ADMIN) {
        return Err(SyscallError::PermissionDenied);
    }
    let prog = veridian_cbpf::check_seccomp(&prog).map_err(|_| SyscallError::InvalidArgument)?;

    // The listener's descriptor is reserved first, so running out of
    // descriptors installs nothing, and filled only once the filter is in
    // place (get_unused_fd_flags, fd_install): until then no other thread
    // can see, close or reuse it, and a failure just gives it back.
    let listener = if flags & FLAG_NEW_LISTENER != 0 {
        let fd = process
            .file_table
            .lock()
            .reserve()
            .map_err(super::map_kernel_error)?;
        Some((Arc::new(NotifyNode::new()), fd))
    } else {
        None
    };
    let result = install(
        flags,
        prog,
        &process,
        &thread,
        listener.as_ref().map(|(n, _)| n),
    );
    match (result, listener) {
        (Ok(v), None) => Ok(v),
        (Ok(_), Some((node, fd))) => {
            let file = Arc::new(File::new(node, OpenFlags::read_write()));
            process.file_table.lock().fill(fd, file, true);
            Ok(fd)
        }
        (Err(e), Some((_, fd))) => {
            process.file_table.lock().unreserve(fd);
            Err(e)
        }
        (Err(e), None) => Err(e),
    }
}

/// Install `prog` on the calling thread, or with TSYNC on all of its
/// process's threads, checked and done under the thread map (and for
/// TSYNC the credential guard, so no exec decides privileges halfway).
fn install(
    flags: usize,
    prog: veridian_cbpf::SeccompProgram,
    process: &Arc<crate::process::Process>,
    thread: &Arc<crate::process::thread::Thread>,
    listener: Option<&Arc<NotifyNode>>,
) -> SyscallResult {
    let _cred = (flags & FLAG_TSYNC != 0).then(|| process.cred_guard.lock());
    let threads = process.threads.lock();
    if thread.seccomp.mode() == sc::MODE_STRICT {
        return Err(SyscallError::InvalidArgument);
    }
    let head = thread.seccomp.head();
    // At most one listener on a chain (has_duplicate_listener): EBUSY.
    if listener.is_some() && head.as_ref().is_some_and(|h| h.has_listener()) {
        return Err(SyscallError::Busy);
    }
    let first_id = listener.map(|_| first_notification_id());
    let chain = Filter::make(
        prog,
        head.clone(),
        flags & FLAG_LOG != 0,
        first_id,
        flags & FLAG_WAIT_KILLABLE_RECV != 0,
    )
    .ok_or(SyscallError::OutOfMemory)?;
    let filter = chain.head().clone();
    if flags & FLAG_TSYNC != 0 {
        // seccomp_can_sync_threads: every other thread must be in filter
        // mode under the caller's chain, or disabled; the first that is not
        // is the return value (or ESRCH with TSYNC_ESRCH).
        for t in threads.values() {
            if Arc::ptr_eq(t, thread) || exiting(t) {
                continue;
            }
            let ok = match t.seccomp.mode() {
                sc::MODE_DISABLED => true,
                sc::MODE_FILTER => match (t.seccomp.head(), head.as_ref()) {
                    (Some(theirs), Some(mine)) => mine.has_ancestor(Some(&theirs)),
                    _ => false,
                },
                _ => false,
            };
            if !ok {
                return if flags & FLAG_TSYNC_ESRCH != 0 {
                    Err(SyscallError::ProcessNotFound)
                } else {
                    Ok(crate::process::pidns::vnr(t.tid.0) as usize)
                };
            }
        }
    }
    thread
        .seccomp
        .set_filter(chain)
        .map_err(|_| SyscallError::InvalidArgument)?;
    if flags & FLAG_TSYNC != 0 {
        // seccomp_sync_threads: the new chain, and the caller's
        // no_new_privs.
        let nnp = thread.no_new_privs.load(Ordering::Acquire);
        for t in threads.values() {
            if Arc::ptr_eq(t, thread) || exiting(t) {
                continue;
            }
            t.seccomp.copy_from(&thread.seccomp);
            if nnp {
                t.no_new_privs.store(true, Ordering::Release);
            }
        }
    }
    if let Some(node) = listener {
        node.filter.call_once(|| filter);
    }
    Ok(0)
}

/// The first notification id of a new listener: random, as Linux's, so
/// ids of different listeners do not repeat one another.
fn first_notification_id() -> u64 {
    let b = crate::crypto::random::random_bytes(8);
    let mut id = [0u8; 8];
    id.copy_from_slice(&b[..8]);
    u64::from_ne_bytes(id)
}

/// `struct sock_fprog` at `uargs` and its instructions.
fn read_prog(uargs: usize) -> Result<Vec<veridian_cbpf::Insn>, SyscallError> {
    // { unsigned short len; struct sock_filter *filter; }
    let [len_word, ptr]: [u64; 2] = userspace::read_user(uargs)?;
    let len = (len_word & 0xffff) as usize;
    if len == 0 || len > veridian_cbpf::MAX_INSNS {
        return Err(SyscallError::InvalidArgument);
    }
    let mut bytes = alloc::vec![0u8; len * 8];
    userspace::read_user_bytes(ptr as usize, &mut bytes)?;
    Ok(bytes
        .chunks_exact(8)
        .map(|c| veridian_cbpf::Insn::from_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]))
        .collect())
}

/// What [`check`] decides for a call.
pub enum Outcome {
    /// Run it.
    Run,
    /// Do not run it: return this value (an errno as a negative number, or
    /// the registers as they were for TRAP).
    Skip(isize),
}

/// The seccomp check for call `nr` with `args`, made at `ip`
/// (__secure_computing). Kills the thread or process itself for the kill
/// actions and for strict mode.
pub fn check(nr: usize, args: [u64; 6], ip: u64) -> Outcome {
    let Some(thread) = crate::process::current_thread() else {
        return Outcome::Run;
    };
    match thread.seccomp.mode() {
        sc::MODE_DISABLED => Outcome::Run,
        sc::MODE_STRICT => {
            if sc::strict_allows(nr) {
                Outcome::Run
            } else {
                // The thread alone, by SIGKILL, as __secure_computing_strict's
                // do_exit(SIGKILL); the process with its last thread (PR #49
                // review).
                kill_by(false, 9);
                Outcome::Skip(-1)
            }
        }
        _ => {
            let Some(chain) = thread.seccomp.head() else {
                return Outcome::Run;
            };
            let data = sc::seccomp_data(nr, ip, args);
            let (r, matched) = sc::run_chain(&chain, &data);
            act(r, &matched, nr, ip, &data, &thread)
        }
    }
}

/// An errno a filter or supervisor chose: EINTR from either is final, not
/// an interrupted wait a handler restarts (only Linux's ERESTARTSYS is).
fn chosen_errno(v: isize, thread: &crate::process::thread::Thread) -> Outcome {
    if v == EINTR_RET {
        thread.no_restart.store(true, Ordering::Release);
    }
    Outcome::Skip(v)
}

/// Carry out a filter's action (__seccomp_filter).
fn act(
    r: u32,
    matched: &Arc<Filter>,
    nr: usize,
    ip: u64,
    sd: &[u8; 64],
    thread: &Arc<crate::process::thread::Thread>,
) -> Outcome {
    let data = r & ret::DATA;
    match r & ret::ACTION_FULL {
        ret::ALLOW => Outcome::Run,
        ret::LOG => {
            crate::println!("[SECCOMP] log: pid {} call {}", tgid(), nr);
            Outcome::Run
        }
        ret::ERRNO => chosen_errno(-(data.min(MAX_ERRNO) as isize), thread),
        ret::TRAP => {
            if matched.log {
                crate::println!("[SECCOMP] trap: pid {} call {}", tgid(), nr);
            }
            let mut info = crate::process::signals::SigInfo::fault(SIGSYS, SYS_SECCOMP, ip);
            info.si_errno = data as i32;
            info.fields[1] = (nr as u32 as u64) | (u64::from(sc::audit_arch()) << 32);
            // Forced: a blocked, ignored or default SIGSYS kills (as
            // force_sig_seccomp).
            if !crate::process::signals::queue_fault(info) {
                let _ = super::process::exit_current(0, SIGSYS as u32);
            }
            // The handler sees the registers as they were (syscall_rollback):
            // on x86_64 rax is orig_ax, the number; AArch64 and RISC-V put
            // back x0/a0, the first argument (orig_x0, orig_a0), which the
            // return register shares (PR #49 review).
            Outcome::Skip(rolled_back(nr, sd))
        }
        // No tracer can be attached yet (ptrace, Track C item 8): ENOSYS,
        // as Linux without one.
        ret::TRACE => Outcome::Skip(-(errno::ENOSYS as isize)),
        ret::USER_NOTIF => user_notification(matched, *sd, thread),
        ret::KILL_THREAD => {
            crate::println!("[SECCOMP] kill thread: pid {} call {}", tgid(), nr);
            kill(false);
            Outcome::Skip(-1)
        }
        // KILL_PROCESS and any unknown action.
        _ => {
            crate::println!("[SECCOMP] kill process: pid {} call {}", tgid(), nr);
            kill(true);
            Outcome::Skip(-1)
        }
    }
}

/// SECCOMP_RET_USER_NOTIF (seccomp_do_user_notification): hand the call to
/// the listener's supervisor and wait for its answer, carrying out the
/// descriptor installs it asks for meanwhile. ENOSYS without a listener.
/// A signal ends the wait (EINTR, restarted as Linux's ERESTARTSYS), but
/// with WAIT_KILLABLE_RECV only a fatal one once the supervisor has it.
fn user_notification(
    f: &Arc<Filter>,
    sd: [u8; 64],
    thread: &Arc<crate::process::thread::Thread>,
) -> Outcome {
    let enosys = Outcome::Skip(-(errno::ENOSYS as isize));
    let Some(notify) = f.notify() else {
        return enosys;
    };
    let Some(id) = notify.lock().begin(thread.tid.0, sd) else {
        return enosys;
    };
    f.changed();
    let received = || {
        notify
            .lock()
            .list
            .iter()
            .find(|n| n.id == id)
            .is_some_and(|n| n.state != NotifState::Init)
    };
    let ready = || {
        notify
            .lock()
            .list
            .iter()
            .find(|n| n.id == id)
            .is_none_or(|n| n.state == NotifState::Replied || !n.addfd.is_empty())
    };
    loop {
        let killable = f.wait_killable_recv && received();
        let waited = if killable {
            crate::sched::dispatch::wait_event_killable(&f.wq, None, ready)
        } else {
            crate::sched::dispatch::wait_event(&f.wq, None, ready)
        };
        if waited.is_err() {
            // Received meanwhile: from now on only a fatal signal counts.
            if !killable && f.wait_killable_recv && received() {
                continue;
            }
            notify.lock().finish(id);
            f.changed();
            return Outcome::Skip(EINTR_RET);
        }
        let step = notify.lock().step(id);
        match step {
            TargetStep::Wait => {}
            TargetStep::AddFd(req) => {
                let result = receive_fd(&req);
                notify.lock().addfd_done(id, &req, result);
                f.changed();
            }
            TargetStep::Replied(error, val, flags) => {
                notify.lock().finish(id);
                f.changed();
                if flags & sc::USER_NOTIF_FLAG_CONTINUE != 0 {
                    return Outcome::Run;
                }
                // syscall_set_return_value: the error if any, else the value.
                let v = if error != 0 {
                    error as isize
                } else {
                    val as isize
                };
                return chosen_errno(v, thread);
            }
        }
    }
}

/// Install a supervisor's descriptor in the calling (notifying) process
/// (receive_fd, receive_fd_replace): the descriptor, or a negative errno.
fn receive_fd(req: &AddFd<Arc<File>>) -> i64 {
    let Some(process) = crate::process::current_process() else {
        return -(errno::ESRCH as i64);
    };
    let table = process.file_table.lock();
    req.file.inc_ref();
    let r = match req.newfd {
        None => table.open_with_flags(req.file.clone(), req.cloexec),
        Some(fd) => table
            .replace_at(fd as usize, req.file.clone(), req.cloexec)
            .map(|()| fd as usize),
    };
    match r {
        Ok(fd) => fd as i64,
        Err(e) => {
            req.file.dec_ref();
            super::linux_compat::to_linux_errno(super::map_kernel_error(e)) as i64
        }
    }
}

fn tgid() -> u64 {
    crate::process::current_process().map_or(0, |p| p.pid.0)
}

/// End the thread, or the whole process, by SIGSYS, which no handler sees
/// (do_exit/do_group_exit(SIGSYS)). The last thread's end is the process's.
fn kill(process: bool) {
    kill_by(process, SIGSYS as u32);
}

/// [`kill`] by signal `sig`: the thread alone (do_exit(sig)) unless
/// `process` or it is the last one.
fn kill_by(process: bool, sig: u32) {
    #[cfg(feature = "alloc")]
    if let Some(p) = crate::process::current_process() {
        if process || p.live_threads() <= 1 {
            p.set_term_signal(sig);
            if crate::sched::dispatch::current_owner().is_some() {
                crate::process::exit::exit_dispatched(0, true);
            }
        } else if crate::sched::dispatch::current_owner().is_some() {
            crate::process::exit::exit_dispatched(0, false);
        }
    }
    let _ = super::process::exit_current(0, sig);
}

// ── The listener ───────────────────────────────────────────────────────

/// The listener descriptor's file ("seccomp notify", an anonymous inode on
/// Linux). Closing its last reference closes the listener: everything
/// outstanding is answered ENOSYS (seccomp_notify_release).
pub struct NotifyNode {
    /// The filter, set once it is installed.
    filter: Once<Arc<Filter>>,
}

impl NotifyNode {
    fn new() -> Self {
        Self {
            filter: Once::new(),
        }
    }
}

impl Drop for NotifyNode {
    fn drop(&mut self) {
        if let Some(f) = self.filter.get() {
            if let Some(n) = f.notify() {
                n.lock().detach();
            }
            f.changed();
        }
    }
}

/// A linux errno (positive) as the matching error.
fn from_errno(e: i32) -> SyscallError {
    match e {
        errno::ENOENT => SyscallError::ResourceNotFound,
        errno::EINPROGRESS => SyscallError::InProgress,
        errno::EBUSY => SyscallError::Busy,
        errno::ESRCH => SyscallError::ProcessNotFound,
        errno::EINTR => SyscallError::Interrupted,
        9 => SyscallError::BadFileDescriptor,
        24 => SyscallError::ResourceLimitExceeded,
        12 => SyscallError::OutOfMemory,
        _ => SyscallError::InvalidArgument,
    }
}

/// The listener's ioctls (seccomp_notify_ioctl), or None if `file` is not
/// a listener. Unknown commands are EINVAL, as on Linux.
pub fn notify_ioctl(file: &File, cmd: usize, arg: usize) -> Option<SyscallResult> {
    let node = file
        .node
        .as_any()
        .and_then(|any| any.downcast_ref::<NotifyNode>())?;
    let Some(f) = node.filter.get() else {
        return Some(Err(SyscallError::InvalidArgument));
    };
    let notify = f.notify()?;
    let cmd = cmd as u32;
    Some(match cmd {
        IOCTL_NOTIF_RECV => notif_recv(f, notify, arg),
        IOCTL_NOTIF_SEND => notif_send(f, notify, arg),
        IOCTL_NOTIF_ID_VALID | IOCTL_NOTIF_ID_VALID_WRONG_DIR => {
            let id: u64 = match userspace::read_user(arg) {
                Ok(id) => id,
                Err(e) => return Some(Err(e)),
            };
            if notify.lock().id_valid(id) {
                Ok(0)
            } else {
                Err(SyscallError::ResourceNotFound)
            }
        }
        IOCTL_NOTIF_SET_FLAGS => {
            // The flags are the argument itself.
            if arg as u64 & !sc::USER_NOTIF_FD_SYNC_WAKE_UP != 0 {
                Err(SyscallError::InvalidArgument)
            } else {
                notify.lock().flags = arg as u64;
                Ok(0)
            }
        }
        _ if cmd & !(IOC_INOUT | (IOC_SIZE_MASK << IOC_SIZE_SHIFT)) == IOCTL_NOTIF_ADDFD_NR => {
            notif_addfd(
                f,
                notify,
                arg,
                ((cmd >> IOC_SIZE_SHIFT) & IOC_SIZE_MASK) as usize,
            )
        }
        _ => Err(SyscallError::InvalidArgument),
    })
}

type NotifyLock = spin::Mutex<sc::Notify>;

/// NOTIF_RECV (seccomp_notify_recv): wait for a notification nobody has
/// received yet and copy it out. The buffer must be zeroed (EINVAL), so
/// the structure can grow; ENOENT once nothing can notify any more.
fn notif_recv(f: &Arc<Filter>, notify: &NotifyLock, arg: usize) -> SyscallResult {
    let mut buf = [0u8; NOTIF_SIZE];
    userspace::read_user_bytes(arg, &mut buf)?;
    if buf.iter().any(|&b| b != 0) {
        return Err(SyscallError::InvalidArgument);
    }
    crate::sched::dispatch::wait_event(&f.wq, None, || {
        !f.has_users() || notify.lock().readiness().0
    })
    .map_err(|_| SyscallError::Interrupted)?;
    let (id, tid, data) = notify.lock().recv().ok_or(SyscallError::ResourceNotFound)?;
    f.changed();
    buf[0..8].copy_from_slice(&id.to_ne_bytes());
    // The notifying thread as the supervisor sees it (0 if outside its
    // PID namespace).
    let pid = crate::process::pidns::vnr(tid) as u32;
    buf[8..12].copy_from_slice(&pid.to_ne_bytes());
    buf[16..80].copy_from_slice(&data);
    if let Err(e) = userspace::write_user_bytes(arg, &buf) {
        // Not lost: another receive gets it.
        notify.lock().unrecv(id);
        f.changed();
        return Err(e);
    }
    Ok(0)
}

/// NOTIF_SEND (seccomp_notify_send): answer a received notification,
/// exactly once.
fn notif_send(f: &Arc<Filter>, notify: &NotifyLock, arg: usize) -> SyscallResult {
    let mut buf = [0u8; RESP_SIZE];
    userspace::read_user_bytes(arg, &mut buf)?;
    let word = |o: usize| u64::from_ne_bytes(buf[o..o + 8].try_into().unwrap_or([0; 8]));
    let half = |o: usize| u32::from_ne_bytes(buf[o..o + 4].try_into().unwrap_or([0; 4]));
    let (id, val, error, flags) = (word(0), word(8) as i64, half(16) as i32, half(20));
    if flags & !sc::USER_NOTIF_FLAG_CONTINUE != 0 {
        return Err(SyscallError::InvalidArgument);
    }
    if flags & sc::USER_NOTIF_FLAG_CONTINUE != 0 && (error != 0 || val != 0) {
        return Err(SyscallError::InvalidArgument);
    }
    notify
        .lock()
        .send(id, error, val, flags)
        .map_err(from_errno)?;
    f.changed();
    Ok(0)
}

/// NOTIF_ADDFD (seccomp_notify_addfd): have the notifying thread install
/// one of the supervisor's descriptors, and wait until it has: the new
/// descriptor's number. With SEND that is also the call's result.
fn notif_addfd(f: &Arc<Filter>, notify: &NotifyLock, arg: usize, size: usize) -> SyscallResult {
    if !(ADDFD_SIZE..4096).contains(&size) {
        return Err(SyscallError::InvalidArgument);
    }
    // copy_struct_from_user: bytes past the known structure must be zero.
    let mut raw = alloc::vec![0u8; size];
    userspace::read_user_bytes(arg, &mut raw)?;
    if raw[ADDFD_SIZE..].iter().any(|&b| b != 0) {
        return Err(SyscallError::ArgumentListTooLong);
    }
    let word = u64::from_ne_bytes(raw[0..8].try_into().unwrap_or([0; 8]));
    let half = |o: usize| u32::from_ne_bytes(raw[o..o + 4].try_into().unwrap_or([0; 4]));
    let (id, flags, srcfd, newfd, newfd_flags) = (word, half(8), half(12), half(16), half(20));
    if newfd_flags & !O_CLOEXEC != 0 {
        return Err(SyscallError::InvalidArgument);
    }
    if flags & !(ADDFD_FLAG_SETFD | ADDFD_FLAG_SEND) != 0 {
        return Err(SyscallError::InvalidArgument);
    }
    if newfd != 0 && flags & ADDFD_FLAG_SETFD == 0 {
        return Err(SyscallError::InvalidArgument);
    }
    let process = crate::process::current_process().ok_or(SyscallError::InvalidState)?;
    // An O_PATH descriptor too, as Linux's fget_raw (PR #49 review).
    let file = process
        .file_table
        .lock()
        .get_path(srcfd as usize)
        .ok_or(SyscallError::BadFileDescriptor)?;
    let req = Arc::new(AddFd {
        file,
        newfd: (flags & ADDFD_FLAG_SETFD != 0).then_some(newfd),
        cloexec: newfd_flags & O_CLOEXEC != 0,
        send: flags & ADDFD_FLAG_SEND != 0,
        result: spin::Mutex::new(None),
    });
    notify.lock().addfd(id, req.clone()).map_err(from_errno)?;
    f.changed();
    let done = crate::sched::dispatch::wait_event(&f.wq, None, || req.result.lock().is_some());
    let result = match done {
        Ok(()) => *req.result.lock(),
        // Interrupted: taken back unless it was carried out meanwhile, or
        // waited for while the target installs it, whatever signals come
        // (the lock is not held while waiting: the target needs it to
        // report the result).
        Err(_) => {
            let cancel = notify.lock().addfd_cancel(id, &req);
            match cancel {
                sc::AddFdCancel::Removed => None,
                sc::AddFdCancel::Done(r) => Some(r),
                sc::AddFdCancel::InFlight => loop {
                    if let Some(r) = *req.result.lock() {
                        break Some(r);
                    }
                    crate::sched::dispatch::yield_now();
                },
            }
        }
    };
    match result {
        Some(r) if r >= 0 => Ok(r as usize),
        Some(r) => Err(from_errno((-r) as i32)),
        None => Err(SyscallError::Interrupted),
    }
}

impl VfsNode for NotifyNode {
    /// An anonymous inode, as on Linux.
    fn statfs(&self) -> Result<crate::fs::statfs::FsStats, KernelError> {
        Ok(crate::fs::statfs::FsStats::simple(
            crate::fs::statfs::ANON_INODE_FS_MAGIC,
        ))
    }

    fn node_type(&self) -> NodeType {
        NodeType::CharDevice
    }

    fn is_stream(&self) -> bool {
        true
    }

    fn read(&self, _offset: usize, _buffer: &mut [u8]) -> Result<usize, KernelError> {
        Err(KernelError::InvalidArgument {
            name: "fd",
            value: "a seccomp listener cannot be read",
        })
    }

    fn write(&self, _offset: usize, _data: &[u8]) -> Result<usize, KernelError> {
        Err(KernelError::InvalidArgument {
            name: "fd",
            value: "a seccomp listener cannot be written",
        })
    }

    /// seccomp_notify_poll: POLLIN while a notification waits to be
    /// received, POLLOUT while one waits for its answer, POLLHUP once no
    /// thread can notify any more.
    fn poll_readiness(&self) -> u16 {
        const POLLIN: u16 = 0x1;
        const POLLOUT: u16 = 0x4;
        const POLLHUP: u16 = 0x10;
        const POLLRDNORM: u16 = 0x40;
        const POLLWRNORM: u16 = 0x100;
        let Some(f) = self.filter.get() else {
            return 0;
        };
        let (pending, sent) = f.notify().map_or((false, false), |n| n.lock().readiness());
        let mut events = 0;
        if pending {
            events |= POLLIN | POLLRDNORM;
        }
        if sent {
            events |= POLLOUT | POLLWRNORM;
        }
        if !f.has_users() {
            events |= POLLHUP;
        }
        events
    }

    fn wakes_io_waiters(&self) -> bool {
        true // every change goes through Filter::changed
    }

    fn as_any(&self) -> Option<&dyn core::any::Any> {
        Some(self)
    }

    fn metadata(&self) -> Result<Metadata, KernelError> {
        Ok(Metadata {
            size: 0,
            node_type: NodeType::CharDevice,
            permissions: Permissions::from_mode(0o600),
            uid: 0,
            gid: 0,
            created: 0,
            modified: 0,
            accessed: 0,
            inode: 0,
        })
    }

    fn readdir(&self) -> Result<Vec<DirEntry>, KernelError> {
        Err(KernelError::FsError(FsError::NotADirectory))
    }

    fn lookup(&self, _name: &str) -> Result<Arc<dyn VfsNode>, KernelError> {
        Err(KernelError::FsError(FsError::NotADirectory))
    }

    fn create(
        &self,
        _name: &str,
        _permissions: Permissions,
    ) -> Result<Arc<dyn VfsNode>, KernelError> {
        Err(KernelError::FsError(FsError::NotADirectory))
    }

    fn mkdir(
        &self,
        _name: &str,
        _permissions: Permissions,
    ) -> Result<Arc<dyn VfsNode>, KernelError> {
        Err(KernelError::FsError(FsError::NotADirectory))
    }

    fn unlink(&self, _name: &str) -> Result<(), KernelError> {
        Err(KernelError::FsError(FsError::NotADirectory))
    }

    fn truncate(&self, _size: usize) -> Result<(), KernelError> {
        Err(KernelError::InvalidArgument {
            name: "fd",
            value: "a seccomp listener cannot be truncated",
        })
    }
}
