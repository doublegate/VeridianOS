//! seccomp(2) and prctl's PR_SET_SECCOMP/PR_GET_SECCOMP (Linux's
//! `kernel/seccomp.c`), and the check every system call passes
//! ([`check`], called at the top of dispatch). The filters themselves are
//! `process::seccomp`'s.

use alloc::{sync::Arc, vec::Vec};

use super::{userspace, SyscallError, SyscallResult};
use crate::process::{
    creds::Caller,
    linux_caps::CAP_SYS_ADMIN,
    seccomp::{self as sc, ret, Filter},
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
            userspace::write_user(uargs, [80u16, 24, 64])?;
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

/// prctl(PR_SET_SECCOMP, mode, filter): strict, or a filter without flags.
pub fn prctl_set(mode: usize, filter: usize) -> SyscallResult {
    match mode {
        1 => {
            if filter != 0 {
                return Err(SyscallError::InvalidArgument);
            }
            set_strict()
        }
        2 => set_filter(0, filter),
        _ => Err(SyscallError::InvalidArgument),
    }
}

fn set_strict() -> SyscallResult {
    let thread = crate::process::current_thread().ok_or(SyscallError::InvalidState)?;
    thread
        .seccomp
        .set_strict()
        .map(|()| 0)
        .map_err(|_| SyscallError::InvalidArgument)
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
    if flags & FLAG_NEW_LISTENER != 0 {
        // Stage 2 (user notification).
        return Err(SyscallError::InvalidArgument);
    }
    let prog = read_prog(uargs)?;
    let thread = crate::process::current_thread().ok_or(SyscallError::InvalidState)?;
    // Without no_new_privs, a filter needs CAP_SYS_ADMIN in the caller's
    // user namespace (seccomp_prepare_filter): EACCES.
    let me = Caller::current();
    if !thread
        .no_new_privs
        .load(core::sync::atomic::Ordering::Acquire)
        && !me.ns_capable(&me.userns, CAP_SYS_ADMIN)
    {
        return Err(SyscallError::PermissionDenied);
    }
    let prog = veridian_cbpf::check_seccomp(&prog).map_err(|_| SyscallError::InvalidArgument)?;
    if thread.seccomp.mode() == sc::MODE_STRICT {
        return Err(SyscallError::InvalidArgument);
    }
    let filter = Filter::new(prog, thread.seccomp.chain(), flags & FLAG_LOG != 0)
        .ok_or(SyscallError::OutOfMemory)?;
    if flags & FLAG_TSYNC == 0 {
        thread
            .seccomp
            .set_filter(filter)
            .map_err(|_| SyscallError::InvalidArgument)?;
        return Ok(0);
    }
    // TSYNC (seccomp_can_sync_threads, seccomp_sync_threads): every other
    // thread must be in filter mode under the caller's chain, or disabled;
    // the first that is not is the return value (or ESRCH with
    // TSYNC_ESRCH). Then all take the new chain, and the caller's
    // no_new_privs.
    let process = crate::process::current_process().ok_or(SyscallError::InvalidState)?;
    let threads: Vec<_> = process.threads.lock().values().cloned().collect();
    let mine = thread.seccomp.chain();
    for t in &threads {
        if Arc::ptr_eq(t, &thread) {
            continue;
        }
        let ok = match t.seccomp.mode() {
            sc::MODE_DISABLED => true,
            // Its chain must be one the caller's grew from (is_ancestor).
            sc::MODE_FILTER => match (t.seccomp.chain(), mine.as_ref()) {
                (Some(theirs), Some(m)) => m.has_ancestor(Some(&theirs)),
                _ => false,
            },
            _ => false,
        };
        if !ok {
            return if flags & FLAG_TSYNC_ESRCH != 0 {
                Err(SyscallError::ProcessNotFound)
            } else {
                Ok(t.tid.0 as usize)
            };
        }
    }
    let nnp = thread
        .no_new_privs
        .load(core::sync::atomic::Ordering::Acquire);
    thread
        .seccomp
        .set_filter(filter)
        .map_err(|_| SyscallError::InvalidArgument)?;
    for t in &threads {
        if Arc::ptr_eq(t, &thread) {
            continue;
        }
        t.seccomp.copy_from(&thread.seccomp);
        if nnp {
            t.no_new_privs
                .store(true, core::sync::atomic::Ordering::Release);
        }
    }
    Ok(0)
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
                // SIGKILL, as __secure_computing_strict.
                let _ = super::process::exit_current(0, 9);
                Outcome::Skip(-1)
            }
        }
        _ => {
            let Some(chain) = thread.seccomp.chain() else {
                return Outcome::Run;
            };
            let data = sc::seccomp_data(nr, ip, args);
            let (r, matched) = sc::run_chain(&chain, &data);
            act(r, &matched, nr, ip, &thread)
        }
    }
}

/// Carry out a filter's action (__seccomp_filter).
fn act(
    r: u32,
    matched: &Arc<Filter>,
    nr: usize,
    ip: u64,
    thread: &Arc<crate::process::thread::Thread>,
) -> Outcome {
    let data = r & ret::DATA;
    match r & ret::ACTION_FULL {
        ret::ALLOW => Outcome::Run,
        ret::LOG => {
            crate::println!("[SECCOMP] log: pid {} call {}", tgid(thread), nr);
            Outcome::Run
        }
        ret::ERRNO => Outcome::Skip(-(data.min(MAX_ERRNO) as isize)),
        ret::TRAP => {
            if matched.log {
                crate::println!("[SECCOMP] trap: pid {} call {}", tgid(thread), nr);
            }
            let mut info = crate::process::signals::SigInfo::fault(SIGSYS, SYS_SECCOMP, ip);
            info.si_errno = data as i32;
            info.fields[1] = (nr as u32 as u64) | (u64::from(sc::audit_arch()) << 32);
            // Forced: a blocked, ignored or default SIGSYS kills (as
            // force_sig_seccomp).
            if !crate::process::signals::queue_fault(info) {
                let _ = super::process::exit_current(0, SIGSYS as u32);
            }
            // The handler sees the registers as they were.
            Outcome::Skip(nr as isize)
        }
        // No tracer can be attached yet (ptrace, Track C item 8): ENOSYS,
        // as Linux without one.
        ret::TRACE => Outcome::Skip(-38),
        // User notification without a listener (stage 2): ENOSYS.
        ret::USER_NOTIF => Outcome::Skip(-38),
        ret::KILL_THREAD => {
            crate::println!("[SECCOMP] kill thread: pid {} call {}", tgid(thread), nr);
            kill(false);
            Outcome::Skip(-1)
        }
        // KILL_PROCESS and any unknown action.
        _ => {
            crate::println!("[SECCOMP] kill process: pid {} call {}", tgid(thread), nr);
            kill(true);
            Outcome::Skip(-1)
        }
    }
}

fn tgid(_thread: &crate::process::thread::Thread) -> u64 {
    crate::process::current_process().map_or(0, |p| p.pid.0)
}

/// End the thread, or the whole process, by SIGSYS, which no handler sees
/// (do_exit/do_group_exit(SIGSYS)). The last thread's end is the process's.
fn kill(process: bool) {
    #[cfg(feature = "alloc")]
    if let Some(p) = crate::process::current_process() {
        if process || p.live_threads() <= 1 {
            p.set_term_signal(SIGSYS as u32);
            if crate::sched::dispatch::current_owner().is_some() {
                crate::process::exit::exit_dispatched(0, true);
            }
        } else if crate::sched::dispatch::current_owner().is_some() {
            crate::process::exit::exit_dispatched(0, false);
        }
    }
    let _ = super::process::exit_current(0, SIGSYS as u32);
}
