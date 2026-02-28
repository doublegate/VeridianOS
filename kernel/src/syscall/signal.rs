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

/// Mirrors the POSIX `struct sigaction` layout expected by user space.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SigAction {
    /// Signal handler function pointer (or SIG_DFL / SIG_IGN).
    pub sa_handler: usize,
    /// Signal mask to apply during handler execution.
    pub sa_mask: u64,
    /// Flags (SA_RESTART, SA_SIGINFO, etc.).
    pub sa_flags: u32,
    /// Padding for alignment.
    pub _pad: u32,
    /// Optional restorer function (used by the kernel to inject sigreturn).
    pub sa_restorer: usize,
}

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
    // Validate signal number (1-31, cannot change SIGKILL=9 or SIGSTOP=19)
    if signum == 0 || signum > 31 {
        return Err(SyscallError::InvalidArgument);
    }
    if signum == 9 || signum == 19 {
        return Err(SyscallError::PermissionDenied);
    }

    // Validate pointers if non-null
    if act_ptr != 0 {
        validate_user_ptr_typed::<SigAction>(act_ptr)?;
    }
    if oldact_ptr != 0 {
        validate_user_ptr_typed::<SigAction>(oldact_ptr)?;
    }

    let proc = process::current_process().ok_or(SyscallError::InvalidState)?;

    // Return the previous handler via oldact_ptr
    if oldact_ptr != 0 {
        let old_handler = proc.get_signal_handler(signum).unwrap_or(0);
        // SAFETY: oldact_ptr was validated as non-null, in user-space, and
        // aligned for SigAction above. We write the previous handler state.
        unsafe {
            let old_act = oldact_ptr as *mut SigAction;
            (*old_act).sa_handler = old_handler as usize;
            (*old_act).sa_mask = 0;
            (*old_act).sa_flags = 0;
            (*old_act)._pad = 0;
            (*old_act).sa_restorer = 0;
        }
    }

    // Install the new handler from act_ptr
    if act_ptr != 0 {
        // SAFETY: act_ptr was validated as non-null, in user-space, and
        // aligned for SigAction above. We read the new handler address.
        let new_act = unsafe { *(act_ptr as *const SigAction) };
        proc.set_signal_handler(signum, new_act.sa_handler as u64)
            .map_err(|_| SyscallError::InvalidArgument)?;
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

    // Save old mask before modifying
    let old_mask = process.get_signal_mask();

    // Write old mask to user space if requested
    if oldset_ptr != 0 {
        validate_user_ptr_typed::<u64>(oldset_ptr)?;
        // SAFETY: oldset_ptr was validated as non-null, in user-space, and
        // aligned for u64 above. We write the previous signal mask.
        unsafe {
            *(oldset_ptr as *mut u64) = old_mask;
        }
    }

    // Apply new mask if a set pointer was provided
    if set_ptr != 0 {
        validate_user_ptr_typed::<u64>(set_ptr)?;
        // SAFETY: set_ptr was validated as non-null, in user-space, and
        // aligned for u64 above. We read the new mask value.
        let new_bits = unsafe { *(set_ptr as *const u64) };

        let updated_mask = match how {
            SIG_BLOCK => old_mask | new_bits,
            SIG_UNBLOCK => old_mask & !new_bits,
            SIG_SETMASK => new_bits,
            _ => return Err(SyscallError::InvalidArgument),
        };

        // SIGKILL (bit 9) and SIGSTOP (bit 19) cannot be blocked
        let sanitized = updated_mask & !((1u64 << 9) | (1u64 << 19));
        process.set_signal_mask(sanitized);
    }

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

    // SAFETY: mask_ptr was validated above.
    let temp_mask = unsafe { *(mask_ptr as *const u64) };

    // Save current mask and apply temporary mask
    let old_mask = proc.get_signal_mask();
    let sanitized = temp_mask & !((1u64 << 9) | (1u64 << 19));
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
    if frame_ptr == 0 {
        return Err(SyscallError::InvalidArgument);
    }

    let proc = process::current_process().ok_or(SyscallError::InvalidState)?;
    let thread = process::current_thread().ok_or(SyscallError::InvalidState)?;

    // Restore the saved context from the signal frame on the user stack.
    // This reads the SignalFrame at frame_ptr, restores all general-purpose
    // registers, RIP, RFLAGS, RSP, and the signal mask.
    process::signal_delivery::restore_signal_frame(proc, thread, frame_ptr)
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
