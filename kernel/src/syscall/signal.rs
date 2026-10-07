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

/// Linux x86_64 kernel `struct sigaction` layout (32 bytes).
///
/// Field order MUST match the kernel ABI that musl's rt_sigaction expects:
///   sa_handler (8), sa_flags (8), sa_restorer (8), sa_mask (8).
/// Note: sa_flags is `unsigned long` (8 bytes on x86_64), NOT u32.
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

// SAFETY: repr(C), four 8-byte integer fields at offsets 0/8/16/24, no padding.
// The handler and restorer are plain addresses validated where they are used,
// not Rust function pointers.
unsafe impl crate::syscall::userspace::UserPod for SigAction {}

// Compile-time assertion: Linux x86_64 struct sigaction is 32 bytes.
const _: () = assert!(core::mem::size_of::<SigAction>() == 32);

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
    // mask used to come back as zero (N-95).
    if oldact_ptr != 0 {
        validate_user_ptr_typed::<SigAction>(oldact_ptr)?;
        let old_handler = proc.get_signal_handler(signum).unwrap_or(0);
        let (flags, restorer, mask) = proc.signal_action_extra.lock()[signum];
        super::userspace::write_user(
            oldact_ptr,
            SigAction {
                sa_handler: old_handler as usize,
                sa_flags: flags,
                sa_restorer: restorer as usize,
                sa_mask: mask,
            },
        )?;
    }

    if let Some(act) = new_act {
        proc.set_signal_handler(signum, act.sa_handler as u64)
            .map_err(|_| SyscallError::InvalidArgument)?;
        proc.signal_action_extra.lock()[signum] =
            (act.sa_flags, act.sa_restorer as u64, act.sa_mask);
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

    // Save current mask and apply temporary mask
    let old_mask = proc.get_signal_mask();
    let sanitized = temp_mask & !crate::process::signals::UNBLOCKABLE;
    proc.set_signal_mask(sanitized);

    // Check if there's already a pending unblocked signal
    if proc.get_next_pending_signal().is_some() {
        // Signal already pending -- restore mask and return
        proc.set_signal_mask(old_mask);
        return Err(SyscallError::Interrupted);
    }

    // Block the process until a signal arrives. The signal delivery
    // path (process::exit::deliver_pending_signal) will wake us via
    // sched::wake_up_process when it delivers a signal to this process.
    proc.set_state(crate::process::pcb::ProcessState::Blocked);
    crate::sched::block_process(proc.pid);

    // After waking: restore the original signal mask
    proc.set_signal_mask(old_mask);

    // sigsuspend always returns EINTR per POSIX
    Err(SyscallError::Interrupted)
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

    if frame_ptr == 0 {
        return Err(SyscallError::InvalidArgument);
    }

    let proc = process::current_process().ok_or(SyscallError::InvalidState)?;
    let thread = process::current_thread().ok_or(SyscallError::InvalidState)?;

    // Restore the saved context from the signal frame on the user stack.
    // This reads the SignalFrame at frame_ptr, restores all general-purpose
    // registers, RIP, RFLAGS, RSP, and the signal mask.
    process::signal_delivery::restore_signal_frame(&proc, &thread, frame_ptr)
        .map_err(|_| SyscallError::InvalidArgument)?;

    // Return 0. The normal syscall return path will load the restored context
    // (RIP, RSP, etc.) and resume execution where the signal interrupted.
    Ok(0)
}

/// Check for pending signals and deliver them if a handler is registered.
///
/// This function can be called from the syscall return path to deliver
/// signals at a safe point (between system calls, when the thread is about
/// to return to user mode).
///
/// # Returns
/// - `Ok(true)` if a signal was delivered (thread context modified).
/// - `Ok(false)` if no deliverable signal was pending.
/// - `Err(...)` on failure.
pub fn check_pending_signals() -> SyscallResult {
    match process::signal_delivery::check_pending_signals() {
        Ok(delivered) => Ok(if delivered { 1 } else { 0 }),
        Err(_) => Ok(0), // Silently ignore errors (process may not exist yet)
    }
}
