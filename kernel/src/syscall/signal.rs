//! Signal-related system calls
//!
//! Provides syscall implementations for POSIX-style signal management:
//! - `sys_sigaction` (120): Install or query a signal handler
//! - `sys_sigprocmask` (121): Block/unblock signals
//! - `sys_sigsuspend` (122): Atomically set mask and suspend
//! - `sys_sigreturn` (123): Return from signal trampoline

use super::{validate_user_ptr_typed, SyscallError, SyscallResult};
use crate::process;

// ============================================================================
// Signal action flags (matching POSIX sa_flags)
// ============================================================================

/// Restart interrupted syscalls automatically.
pub const SA_RESTART: u32 = 0x1000_0000;
/// Do not generate SIGCHLD when children stop.
pub const SA_NOCLDSTOP: u32 = 0x0000_0001;
/// Use sa_sigaction instead of sa_handler.
pub const SA_SIGINFO: u32 = 0x0000_0004;
/// Use alternate signal stack (sigaltstack).
pub const SA_ONSTACK: u32 = 0x0800_0000;
/// Reset handler to SIG_DFL on entry.
pub const SA_RESETHAND: u32 = 0x8000_0000;
/// Do not add signal to mask during handler.
pub const SA_NODEFER: u32 = 0x4000_0000;
/// Do not create zombie children.
pub const SA_NOCLDWAIT: u32 = 0x0000_0002;

// ============================================================================
// Signal mask operations
// ============================================================================

/// How to modify the signal mask in sigprocmask.
pub const SIG_BLOCK: usize = 0;
/// Unblock signals in the provided set.
pub const SIG_UNBLOCK: usize = 1;
/// Replace the mask entirely.
pub const SIG_SETMASK: usize = 2;

/// Default signal handler (terminate process).
pub const SIG_DFL: usize = 0;
/// Ignore the signal.
pub const SIG_IGN: usize = 1;

// ============================================================================
// User-space signal action structure (repr(C) for ABI stability)
// ============================================================================

/// Linux kernel `struct sigaction` layout on x86_64 and arm64 (32 bytes).
///
/// Field order MUST match the kernel ABI that musl's rt_sigaction expects:
///   sa_handler (8), sa_flags (8), sa_restorer (8), sa_mask (8).
/// Note: sa_flags is `unsigned long` (8 bytes on x86_64), NOT u32.
#[cfg(not(target_arch = "riscv64"))]
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SigAction {
    /// Signal handler function pointer (or SIG_DFL / SIG_IGN).
    pub sa_handler: usize, // offset 0
    /// Flags (SA_RESTART, SA_SIGINFO, SA_NOCLDSTOP, etc.).
    pub sa_flags: u64, // offset 8 (unsigned long on x86_64)
    /// Optional restorer function (used by the kernel to inject sigreturn).
    pub sa_restorer: usize, // offset 16
    /// Signal mask to apply during handler execution.
    pub sa_mask: u64, // offset 24
}

/// RISC-V `struct sigaction`: no SA_RESTORER and no restorer field (Linux
/// returns from handlers through its vDSO, this kernel through its own
/// trampoline page); the mask follows the flags, at offset 16. musl's
/// riscv64 `k_sigaction` matches it (plus an unused pointer the kernel does
/// not read).
#[cfg(target_arch = "riscv64")]
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SigAction {
    pub sa_handler: usize,
    pub sa_flags: u64,
    pub sa_mask: u64,
}

impl SigAction {
    /// The action as user space sees it.
    fn new(handler: u64, flags: u64, restorer: u64, mask: u64) -> Self {
        #[cfg(not(target_arch = "riscv64"))]
        return Self {
            sa_handler: handler as usize,
            sa_flags: flags,
            sa_restorer: restorer as usize,
            sa_mask: mask,
        };
        // No restorer on RISC-V (see above).
        #[cfg(target_arch = "riscv64")]
        {
            let _ = restorer;
            Self {
                sa_handler: handler as usize,
                sa_flags: flags,
                sa_mask: mask,
            }
        }
    }

    /// The restorer the C library named (0 on RISC-V, which has none).
    fn restorer(&self) -> u64 {
        #[cfg(not(target_arch = "riscv64"))]
        return self.sa_restorer as u64;
        #[cfg(target_arch = "riscv64")]
        0
    }
}

// SAFETY: repr(C), 8-byte integer fields with no padding. The handler and
// restorer are plain addresses validated where they are used, not Rust
// function pointers.
unsafe impl crate::syscall::userspace::UserPod for SigAction {}

#[cfg(not(target_arch = "riscv64"))]
const _: () = assert!(core::mem::size_of::<SigAction>() == 32);
#[cfg(target_arch = "riscv64")]
const _: () = assert!(core::mem::size_of::<SigAction>() == 24);

// ============================================================================
// Syscall implementations
// ============================================================================

/// Install or query a signal handler (syscall 120).
///
/// Reads/writes the signal handler from/to the PCB's signal_handlers table.
/// The handler address is stored as a u64 in the PCB's `[u64; 32]` array.
///
/// # Arguments
/// - `signum`: Signal number (1-31).
/// - `act_ptr`: Pointer to new `SigAction` (0 to query only).
/// - `oldact_ptr`: Pointer to receive previous `SigAction` (0 to skip).
///
/// # Returns
/// 0 on success.
pub fn sys_sigaction(signum: usize, act_ptr: usize, oldact_ptr: usize) -> SyscallResult {
    // Signals 1-64 (N-209). SIGKILL and SIGSTOP can be queried but not
    // changed (EINVAL, as Linux; it was EACCES even for a query, N-105).
    if signum == 0 || signum > crate::process::signals::NSIG {
        return Err(SyscallError::InvalidArgument);
    }
    if act_ptr != 0 && (signum == 9 || signum == 19) {
        return Err(SyscallError::InvalidArgument);
    }

    // Read the new action first: act and oldact may be the same buffer.
    let new_act: Option<SigAction> = if act_ptr != 0 {
        validate_user_ptr_typed::<SigAction>(act_ptr)?;
        Some(super::userspace::read_user(act_ptr)?)
    } else {
        None
    };

    let proc = process::current_process().ok_or(SyscallError::InvalidState)?;

    // The whole previous action, not just the handler: flags, restorer and
    // mask used to come back as zero (N-95). The old action is taken and
    // the new one installed in one step, then the old one copied out: a
    // fault on `oldact` leaves the new action installed, as on Linux,
    // where it used to leave it uninstalled (PR #23 review).
    if oldact_ptr != 0 {
        validate_user_ptr_typed::<SigAction>(oldact_ptr)?;
    }
    let new = new_act.map(|act| {
        (
            act.sa_handler as u64,
            (act.sa_flags, act.restorer(), act.sa_mask),
        )
    });
    let (old_handler, (flags, restorer, mask)) = proc
        .swap_signal_action(signum, new)
        .map_err(|_| SyscallError::InvalidArgument)?;
    if oldact_ptr != 0 {
        super::userspace::write_user(
            oldact_ptr,
            SigAction::new(old_handler, flags, restorer, mask),
        )?;
    }

    Ok(0)
}

/// Block, unblock, or set the process signal mask (syscall 121).
///
/// This syscall works fully using the PCB's existing signal mask API.
///
/// # Arguments
/// - `how`: SIG_BLOCK, SIG_UNBLOCK, or SIG_SETMASK.
/// - `set_ptr`: Pointer to the new mask bits (u64). 0 to query only.
/// - `oldset_ptr`: Pointer to receive the previous mask (u64). 0 to skip.
///
/// # Returns
/// 0 on success.
pub fn sys_sigprocmask(how: usize, set_ptr: usize, oldset_ptr: usize) -> SyscallResult {
    let process = process::current_process().ok_or(SyscallError::InvalidState)?;

    // Read the new set before writing the old one: callers commonly pass
    // the same buffer for both (`sigprocmask(SIG_BLOCK, &s, &s)`), and
    // writing first made the call apply the old mask (N-97).
    let new_bits: Option<u64> = if set_ptr != 0 {
        validate_user_ptr_typed::<u64>(set_ptr)?;
        Some(super::userspace::read_user(set_ptr)?)
    } else {
        None
    };
    let updated = |old: u64| -> Result<u64, SyscallError> {
        let Some(bits) = new_bits else {
            return Ok(old);
        };
        let mask = match how {
            SIG_BLOCK => old | bits,
            SIG_UNBLOCK => old & !bits,
            SIG_SETMASK => bits,
            _ => return Err(SyscallError::InvalidArgument),
        };
        // SIGKILL and SIGSTOP cannot be blocked (Linux layout, N-96).
        Ok(mask & !crate::process::signals::UNBLOCKABLE)
    };

    // The mask is per thread (N-109).
    let thread = process::current_thread();
    let old_mask = match &thread {
        Some(t) => t.sigmask.load(core::sync::atomic::Ordering::Acquire),
        None => process.get_signal_mask(),
    };
    let new_mask = updated(old_mask)?;

    if oldset_ptr != 0 {
        validate_user_ptr_typed::<u64>(oldset_ptr)?;
        super::userspace::write_user::<u64>(oldset_ptr, old_mask)?;
    }
    if new_bits.is_some() {
        match &thread {
            Some(t) => {
                crate::process::signals::set_mask(&process, t, new_mask);
            }
            None => {
                process.set_signal_mask(new_mask);
            }
        }
    }

    Ok(0)
}

/// rt_sigpending (native 360): the signals pending for the calling thread
/// that it blocks (thread and process sets), written as a 64-bit Linux set.
pub fn sys_sigpending(set_ptr: usize, size: usize) -> SyscallResult {
    use core::sync::atomic::Ordering;
    if size != 0 && size != 8 {
        return Err(SyscallError::InvalidArgument);
    }
    validate_user_ptr_typed::<u64>(set_ptr)?;
    let proc = process::current_process().ok_or(SyscallError::InvalidState)?;
    let set = match process::current_thread() {
        Some(t) => {
            (t.sigpending.load(Ordering::Acquire) | proc.pending_signals.load(Ordering::Acquire))
                & t.sigmask.load(Ordering::Acquire)
        }
        None => proc.pending_signals.load(Ordering::Acquire) & proc.get_signal_mask(),
    };
    super::userspace::write_user::<u64>(set_ptr, set)?;
    Ok(0)
}

/// Read the siginfo a sigqueue caller passes for signal `sig`, applying
/// Linux's rule: a process may not pose as the kernel, or as kill or tkill
/// from another process -- a non-negative si_code, or SI_TKILL, is EPERM
/// unless the caller signals itself (`own`).
#[cfg(feature = "alloc")]
fn queued_siginfo(
    uinfo: usize,
    sig: usize,
    own: bool,
) -> Result<crate::process::signals::SigInfo, SyscallError> {
    use crate::process::signals::{SigInfo, SI_TKILL};
    validate_user_ptr_typed::<SigInfo>(uinfo)?;
    let mut info: SigInfo = super::userspace::read_user(uinfo)?;
    if (info.si_code >= 0 || info.si_code == SI_TKILL) && !own {
        return Err(SyscallError::OperationNotPermitted);
    }
    info.si_signo = sig as i32;
    Ok(info)
}

/// A signal number argument: 0 (existence and permission only) to NSIG.
fn signal_number(sig: usize) -> Result<usize, SyscallError> {
    let sig = sig as u32 as i32;
    if (0..=crate::process::signals::NSIG as i32).contains(&sig) {
        Ok(sig as usize)
    } else {
        Err(SyscallError::InvalidArgument)
    }
}

/// rt_sigqueueinfo(tgid, sig, info): send `sig` to process `tgid` with the
/// caller's siginfo (sigqueue passes SI_QUEUE, its pid and uid, and the
/// value). Permission as kill; EAGAIN when the real-time queue is full.
#[cfg(feature = "alloc")]
pub fn sys_rt_sigqueueinfo(tgid: usize, sig: usize, uinfo: usize) -> SyscallResult {
    let tgid = super::process::positive_id(tgid).map_err(|_| SyscallError::ProcessNotFound)?;
    let sig = signal_number(sig)?;
    let caller = process::current_process().ok_or(SyscallError::InvalidState)?;
    let pid = process::ProcessId(tgid as u64);
    let info = queued_siginfo(uinfo, sig, caller.pid == pid)?;
    // Existence and permission first (sig 0 stops there).
    super::filesystem::kill_one(&caller, pid, 0)?;
    if sig == 0 {
        return Ok(0);
    }
    if let Some(target) = process::get_process(pid) {
        if target
            .dispatched
            .load(core::sync::atomic::Ordering::Acquire)
        {
            return match crate::process::signals::send_info_to_process(&target, info) {
                Ok(true) => Ok(0),
                Ok(false) => super::filesystem::send_info(pid, sig as i32, info),
                Err(crate::error::KernelError::WouldBlock) => Err(SyscallError::WouldBlock),
                Err(_) => Err(SyscallError::ProcessNotFound),
            };
        }
    }
    super::filesystem::send_info(pid, sig as i32, info)
}

/// rt_tgsigqueueinfo(tgid, tid, sig, info): as rt_sigqueueinfo, to thread
/// `tid` of process `tgid` (pthread_sigqueue).
#[cfg(feature = "alloc")]
pub fn sys_rt_tgsigqueueinfo(tgid: usize, tid: usize, sig: usize, uinfo: usize) -> SyscallResult {
    let tgid = super::process::positive_id(tgid)?;
    let tid = super::process::positive_id(tid)?;
    let sig = signal_number(sig)?;
    let caller = process::current_process().ok_or(SyscallError::InvalidState)?;
    let pid = process::ProcessId(tgid as u64);
    let info = queued_siginfo(uinfo, sig, caller.pid == pid)?;
    if super::process::process_of_thread(tid) != Some(pid) {
        return Err(SyscallError::ProcessNotFound);
    }
    super::filesystem::kill_one(&caller, pid, 0)?;
    if sig == 0 {
        return Ok(0);
    }
    let target = process::get_process(pid).ok_or(SyscallError::ProcessNotFound)?;
    if target
        .dispatched
        .load(core::sync::atomic::Ordering::Acquire)
    {
        let thread = target
            .get_thread(process::ThreadId(tid as u64))
            .ok_or(SyscallError::ProcessNotFound)?;
        return crate::process::signals::send_to_thread(&target, &thread, info)
            .map(|_| 0)
            .map_err(|e| match e {
                crate::error::KernelError::WouldBlock => SyscallError::WouldBlock,
                _ => SyscallError::ProcessNotFound,
            });
    }
    super::filesystem::send_info(pid, sig as i32, info)
}

/// rt_sigtimedwait(set, info, timeout, sigsetsize): take one of the
/// signals in `set` (SIGKILL and SIGSTOP excepted) that is pending for the
/// calling thread, or wait for one -- at most `timeout` (a relative
/// timespec; NULL waits without limit, zero polls) -- and return its
/// number, its siginfo written to `info` if non-NULL. While waiting the
/// signals in `set` are unblocked, so their arrival wakes the thread;
/// another signal it does not block ends the wait with EINTR (and is then
/// delivered). EAGAIN when the time runs out.
#[cfg(feature = "alloc")]
pub fn sys_rt_sigtimedwait(
    set_ptr: usize,
    info_ptr: usize,
    timeout_ptr: usize,
    size: usize,
) -> SyscallResult {
    use core::sync::atomic::Ordering;

    use crate::{
        process::signals::{self, SigInfo},
        sched::dispatch,
    };

    if size != 8 {
        return Err(SyscallError::InvalidArgument);
    }
    validate_user_ptr_typed::<u64>(set_ptr)?;
    let set: u64 = super::userspace::read_user(set_ptr)?;
    let set = set & !signals::UNBLOCKABLE;
    let timeout_ns = if timeout_ptr == 0 {
        None
    } else {
        validate_user_ptr_typed::<[i64; 2]>(timeout_ptr)?;
        let [sec, nsec]: [i64; 2] = super::userspace::read_user(timeout_ptr)?;
        if sec < 0 || !(0..1_000_000_000).contains(&nsec) {
            return Err(SyscallError::InvalidArgument);
        }
        Some(
            (sec as u64)
                .saturating_mul(1_000_000_000)
                .saturating_add(nsec as u64),
        )
    };
    let taken = |info: SigInfo| -> SyscallResult {
        if info_ptr != 0 {
            validate_user_ptr_typed::<SigInfo>(info_ptr)?;
            super::userspace::write_user(info_ptr, info)?;
        }
        Ok(info.signo())
    };

    if let Some(info) = signals::take_pending_in(set) {
        return taken(info);
    }
    if timeout_ns == Some(0) || dispatch::current_owner().is_none() {
        return Err(SyscallError::WouldBlock);
    }

    // Wait with the waited-for signals unblocked (Linux's real_blocked),
    // then put the mask back before taking one, so none is delivered to a
    // handler instead.
    let proc = process::current_process().ok_or(SyscallError::InvalidState)?;
    let thread = process::current_thread().ok_or(SyscallError::InvalidState)?;
    let old = thread.sigmask.load(Ordering::Acquire);
    signals::set_mask(&proc, &thread, old & !set);
    let deadline = timeout_ns.map(|ns| dispatch::clock_ns().saturating_add(ns));
    static WAITERS: dispatch::WaitQueue = dispatch::WaitQueue::new();
    let waited = dispatch::wait_event(&WAITERS, deadline, || signals::pending_in(set));
    signals::set_mask(&proc, &thread, old);
    drop(thread);
    drop(proc);

    match signals::take_pending_in(set) {
        Some(info) => taken(info),
        None => match waited {
            Err(dispatch::WaitError::TimedOut) => Err(SyscallError::WouldBlock),
            _ => Err(SyscallError::Interrupted),
        },
    }
}

/// Atomically set signal mask and suspend until a signal arrives (syscall 122).
///
/// Saves the current signal mask, replaces it with the provided mask, then
/// suspends the thread. When a non-blocked signal arrives, the original mask
/// is restored and the syscall returns EINTR.
///
/// # Arguments
/// - `mask_ptr`: Pointer to the temporary signal mask (u64).
///
/// # Returns
/// Always returns `Err(Interrupted)` when a signal wakes the process.
pub fn sys_sigsuspend(mask_ptr: usize) -> SyscallResult {
    if mask_ptr == 0 {
        return Err(SyscallError::InvalidArgument);
    }
    validate_user_ptr_typed::<u64>(mask_ptr)?;

    let proc = process::current_process().ok_or(SyscallError::InvalidState)?;

    let temp_mask: u64 = super::userspace::read_user(mask_ptr)?;

    // A dispatched thread waits with the temporary mask until a signal it
    // does not block arrives; the mask in force before is put back when
    // the handler returns (it goes into the handler's frame), as Linux.
    #[cfg(feature = "alloc")]
    if crate::sched::dispatch::current_owner().is_some() {
        use core::sync::atomic::Ordering;
        let thread = process::current_thread().ok_or(SyscallError::InvalidState)?;
        let old = crate::process::signals::set_mask(&proc, &thread, temp_mask);
        thread.saved_sigmask.store(old, Ordering::Release);
        thread.has_saved_sigmask.store(true, Ordering::Release);
        drop(thread);
        drop(proc);
        // Sleeps until a signal to act on wakes it; nothing else does.
        static SUSPENDED: crate::sched::dispatch::WaitQueue =
            crate::sched::dispatch::WaitQueue::new();
        let _ = crate::sched::dispatch::wait_event(&SUSPENDED, None, || false);
        return Err(SyscallError::Interrupted);
    }

    // Only a dispatched thread has a current process.
    let _ = (proc, temp_mask);
    Err(SyscallError::InvalidState)
}

/// The temporary signal mask of a wait (ppoll, epoll_pwait, epoll_pwait2),
/// installed by [`begin_wait_sigmask`] and ended by [`WaitSigmask::end`].
#[must_use]
pub(crate) struct WaitSigmask {
    old: Option<u64>,
}

/// Install `*mask_ptr`, a sigset of `size` bytes (which must be 8: EINVAL
/// otherwise), as the calling thread's mask for one wait; NULL installs
/// nothing. The mask in force before is saved as sigsuspend saves it.
pub(crate) fn begin_wait_sigmask(
    mask_ptr: usize,
    size: usize,
) -> Result<WaitSigmask, SyscallError> {
    if mask_ptr == 0 {
        return Ok(WaitSigmask { old: None });
    }
    if size != 8 {
        return Err(SyscallError::InvalidArgument);
    }
    let mask: u64 = super::userspace::read_user(mask_ptr)?;
    let proc = process::current_process().ok_or(SyscallError::InvalidState)?;
    let thread = process::current_thread().ok_or(SyscallError::InvalidState)?;
    let old = crate::process::signals::set_mask(&proc, &thread, mask);
    thread
        .saved_sigmask
        .store(old, core::sync::atomic::Ordering::Release);
    thread
        .has_saved_sigmask
        .store(true, core::sync::atomic::Ordering::Release);
    Ok(WaitSigmask { old: Some(old) })
}

impl WaitSigmask {
    /// The wait returned `result`. Interrupted by a signal, the saved mask
    /// comes back on the way to user mode -- after the handler, whose frame
    /// records it -- as Linux does; otherwise it comes back now, so a
    /// signal only the temporary mask let through stays pending.
    pub(crate) fn end(self, result: &SyscallResult) {
        let Some(old) = self.old else {
            return;
        };
        // Every architecture's delivery restores it from the handler frame
        // (x86_64's `frame` and the generic one).
        #[cfg(feature = "alloc")]
        if *result == Err(SyscallError::Interrupted)
            && crate::sched::dispatch::current_owner().is_some()
        {
            return;
        }
        #[cfg(not(feature = "alloc"))]
        let _ = result;
        if let (Some(proc), Some(thread)) = (process::current_process(), process::current_thread())
        {
            if thread
                .has_saved_sigmask
                .swap(false, core::sync::atomic::Ordering::AcqRel)
            {
                crate::process::signals::set_mask(&proc, &thread, old);
            }
        }
    }
}

/// Return from a signal handler trampoline (syscall 123).
///
/// Called by the signal trampoline code after a signal handler returns.
/// Restores the interrupted context (registers, signal mask) from the
/// signal frame on the user stack.
///
/// # Arguments
/// - `frame_ptr`: Pointer to the saved signal frame on the user stack.
///
/// # Returns
/// 0 on success (the thread context has been restored to the pre-signal
/// state; the normal syscall return path will resume at the interrupted
/// instruction).
/// sigaltstack(ss, old_ss) (N-222): set and/or report the calling thread's
/// alternate signal stack, as Linux does: the new settings are read first
/// (EFAULT), then applied (EPERM while running on the alternate stack,
/// EINVAL for unknown modes, ENOMEM below MINSIGSTKSZ), then the previous
/// ones written out. `stack_t` is { ss_sp: u64, ss_flags: i32, pad, ss_size:
/// u64 }, 24 bytes.
pub fn sys_sigaltstack(ss: usize, old_ss: usize) -> SyscallResult {
    use crate::{
        process::signals::{AltStackError, AltStackRequest},
        syscall::userspace::{read_user_bytes, write_user_bytes},
    };

    let thread = process::current_thread().ok_or(SyscallError::InvalidState)?;
    // The caller's stack pointer decides SS_ONSTACK and EPERM.
    #[cfg(all(target_arch = "x86_64", target_os = "none"))]
    let sp = crate::arch::x86_64::syscall::get_syscall_frame().map_or(0, |f| f.rsp);
    #[cfg(all(target_arch = "aarch64", target_os = "none"))]
    let sp = crate::arch::aarch64::switch::live_user_frame().map_or(0, |f| f.sp);
    #[cfg(all(target_arch = "riscv64", target_os = "none"))]
    let sp = crate::arch::riscv64::switch::live_user_frame().map_or(0, |f| f.x[2]);
    #[cfg(not(all(
        any(
            target_arch = "x86_64",
            target_arch = "aarch64",
            target_arch = "riscv64"
        ),
        target_os = "none"
    )))]
    let sp = 0u64;

    let request = if ss != 0 {
        let mut raw = [0u8; 24];
        read_user_bytes(ss, &mut raw)?;
        let word = |at: usize| u64::from_ne_bytes(raw[at..at + 8].try_into().unwrap_or([0; 8]));
        Some(AltStackRequest {
            sp: word(0),
            flags: i32::from_ne_bytes(raw[8..12].try_into().unwrap_or([0; 4])),
            size: word(16),
        })
    } else {
        None
    };

    let previous = {
        let mut alt = thread.altstack.lock();
        let previous = alt.report(sp);
        if let Some(request) = request {
            alt.set(request, sp).map_err(|e| match e {
                AltStackError::OnStack => SyscallError::OperationNotPermitted,
                AltStackError::Invalid => SyscallError::InvalidArgument,
                AltStackError::TooSmall => SyscallError::OutOfMemory,
            })?;
        }
        previous
    };

    if old_ss != 0 {
        let mut raw = [0u8; 24];
        raw[0..8].copy_from_slice(&previous.sp.to_ne_bytes());
        raw[8..12].copy_from_slice(&previous.flags.to_ne_bytes());
        raw[16..24].copy_from_slice(&previous.size.to_ne_bytes());
        write_user_bytes(old_ss, &raw)?;
    }
    Ok(0)
}

pub fn sys_sigreturn(frame_ptr: usize) -> SyscallResult {
    // A dispatched thread: Linux rt_sigreturn, restoring into the live
    // system call frame (the frame address comes from the user RSP, not
    // from an argument). An unusable frame kills the thread with SIGSEGV.
    #[cfg(all(feature = "alloc", target_arch = "x86_64"))]
    if crate::sched::dispatch::current_owner().is_some() {
        return match crate::arch::x86_64::syscall::with_syscall_frame(
            crate::process::signals::rt_sigreturn,
        ) {
            Some(Some(rax)) => Ok(rax as usize),
            _ => super::process::exit_current(0, 11),
        };
    }
    // AArch64 and RISC-V: the same, into the exception frame the per-CPU
    // block records for the call in progress (`syscall_frame`).
    #[cfg(all(
        feature = "alloc",
        target_os = "none",
        any(target_arch = "aarch64", target_arch = "riscv64")
    ))]
    if crate::sched::dispatch::current_owner().is_some() {
        #[cfg(target_arch = "aarch64")]
        type Frame = crate::arch::aarch64::switch::UserFrame;
        #[cfg(target_arch = "riscv64")]
        type Frame = crate::arch::riscv64::switch::UserFrame;
        // SAFETY: this CPU's own block; `syscall_frame` is the current
        // task's exception frame on its kernel stack, live for this call.
        let frame = unsafe { (*crate::arch::percpu::this_arch_cpu()).syscall_frame } as *mut Frame;
        if !frame.is_null() {
            // SAFETY: as above; nothing else touches the frame now.
            if let Some(x0) = crate::process::signals::rt_sigreturn(unsafe { &mut *frame }) {
                return Ok(x0 as usize);
            }
        }
        return super::process::exit_current(0, 11);
    }

    // Every user thread is a dispatched task (since D4), so nothing gets
    // here. The old path restored a frame read from `frame_ptr` without a
    // user-range check, and its delivery half had no callers: both were
    // removed (PR #22 review).
    let _ = frame_ptr;
    Err(SyscallError::InvalidState)
}
