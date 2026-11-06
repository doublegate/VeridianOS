//! restart_syscall and the restart block (Linux kernel/signal.c, and the
//! restart_block users in kernel/time/hrtimer.c and fs/select.c).
//!
//! A relative sleep, or a poll with a timeout, that a signal interrupts
//! records what is left of it in its thread's block and fails with
//! [`SyscallError::RestartBlock`] (Linux -ERESTART_RESTARTBLOCK, which user
//! space never sees). On the way back to user mode a caught signal turns
//! that into EINTR; a stop, or a return with no signal left to act on,
//! turns the call into restart_syscall (`process::signals::on_return`),
//! which carries on to the same deadline instead of starting the whole
//! duration again. rt_sigreturn forgets the block, so a restart_syscall
//! with none is EINTR, as Linux's do_no_restart_syscall.

use super::{SyscallError, SyscallResult};

/// What restart_syscall carries on with.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Restart {
    /// Nothing: restart_syscall is EINTR.
    #[default]
    None,
    /// nanosleep or a relative clock_nanosleep, until `deadline`
    /// (`dispatch::clock_ns`), the time left written to `rem_ptr` (if
    /// non-zero) when it is interrupted again.
    Sleep { deadline: u64, rem_ptr: usize },
    /// poll with a timeout: the `nfds` pollfds at `fds_ptr`, until
    /// `deadline` (`multiplex::now_ns`).
    Poll {
        fds_ptr: usize,
        nfds: usize,
        deadline: u64,
    },
}

/// A call that can be restarted was interrupted: record `restart` for the
/// calling thread and return the error that asks for it. A thread that is
/// not dispatched has no signal return path to act on the block, so it
/// gets plain EINTR.
#[cfg(feature = "alloc")]
pub(crate) fn interrupted(restart: Restart) -> SyscallError {
    if crate::sched::dispatch::current_owner().is_none() {
        return SyscallError::Interrupted;
    }
    match crate::process::current_thread() {
        Some(thread) => {
            *thread.restart.lock() = restart;
            SyscallError::RestartBlock
        }
        None => SyscallError::Interrupted,
    }
}

/// restart_syscall(): carry on with the call the calling thread's block
/// describes, taking the block (a later interruption records it again).
pub fn sys_restart_syscall() -> SyscallResult {
    let restart = match crate::process::current_thread() {
        Some(thread) => core::mem::take(&mut *thread.restart.lock()),
        None => Restart::None,
    };
    match restart {
        Restart::None => Err(SyscallError::Interrupted),
        #[cfg(feature = "alloc")]
        Restart::Sleep { deadline, rem_ptr } => super::time::sleep_until(deadline, rem_ptr, true),
        #[cfg(feature = "alloc")]
        Restart::Poll {
            fds_ptr,
            nfds,
            deadline,
        } => super::multiplex::poll_until(fds_ptr, nfds, deadline),
        #[cfg(not(feature = "alloc"))]
        _ => Err(SyscallError::Interrupted),
    }
}
