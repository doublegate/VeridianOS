//! Time management system calls
//!
//! Provides kernel-side implementation of time-related operations:
//! monotonic uptime queries and software timer creation/cancellation.
//! All operations delegate to the [`crate::timer`] subsystem.

use super::{SyscallError, SyscallResult};

/// Get monotonic uptime in milliseconds (SYS_TIME_GET_UPTIME = 100)
///
/// # Returns
/// Current uptime in milliseconds since boot.
pub fn sys_time_get_uptime() -> SyscallResult {
    Ok(crate::timer::get_uptime_ms() as usize)
}

/// Create a new timer (SYS_TIME_CREATE_TIMER = 101)
///
/// # Arguments
/// - `mode`: 0 for OneShot, 1 for Periodic
/// - `interval_ms`: Timer interval in milliseconds (must be > 0)
/// - `callback_ptr`: Reserved for future use (user-space signal delivery).
///   Currently ignored; timers fire a kernel-internal no-op callback.
///
/// # Returns
/// The `TimerId` (as `usize`) on success.
pub fn sys_time_create_timer(
    mode: usize,
    interval_ms: usize,
    _callback_ptr: usize,
) -> SyscallResult {
    let timer_mode = match mode {
        0 => crate::timer::TimerMode::OneShot,
        1 => crate::timer::TimerMode::Periodic,
        _ => return Err(SyscallError::InvalidArgument),
    };

    if interval_ms == 0 {
        return Err(SyscallError::InvalidArgument);
    }

    // User-space timers use a no-op kernel callback. In the future this
    // would deliver a signal or event to the calling process.
    fn user_timer_callback(_id: crate::timer::TimerId) {}

    match crate::timer::create_timer(timer_mode, interval_ms as u64, user_timer_callback) {
        Ok(id) => Ok(id.0 as usize),
        Err(_) => Err(SyscallError::ResourceNotFound),
    }
}

/// Cancel an active timer (SYS_TIME_CANCEL_TIMER = 102)
///
/// # Arguments
/// - `timer_id`: The timer ID returned by `SYS_TIME_CREATE_TIMER`.
///
/// # Returns
/// 0 on success.
pub fn sys_time_cancel_timer(timer_id: usize) -> SyscallResult {
    let id = crate::timer::TimerId(timer_id as u64);

    match crate::timer::cancel_timer(id) {
        Ok(()) => Ok(0),
        Err(_) => Err(SyscallError::ResourceNotFound),
    }
}
