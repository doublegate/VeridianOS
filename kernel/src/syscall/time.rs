//! Time management system calls
//!
//! Provides kernel-side implementation of time-related operations:
//! monotonic uptime queries, POSIX clock/time functions, nanosleep,
//! and software timer creation/cancellation.
//! All operations delegate to the [`crate::timer`] subsystem.

use super::{validate_user_ptr_typed, SyscallError, SyscallResult};

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

// ============================================================================
// POSIX-style time syscalls (160-163)
// ============================================================================

/// Clock identifiers matching POSIX clock_gettime.
const CLOCK_REALTIME: usize = 0;
const CLOCK_MONOTONIC: usize = 1;

/// POSIX timespec structure layout (matches C struct timespec).
#[repr(C)]
#[derive(Clone, Copy)]
struct Timespec {
    tv_sec: i64,
    tv_nsec: i64,
}

/// POSIX timeval structure layout (matches C struct timeval).
#[repr(C)]
#[derive(Clone, Copy)]
struct Timeval {
    tv_sec: i64,
    tv_usec: i64,
}

/// Get the current time for a given clock (SYS_CLOCK_GETTIME = 160).
///
/// # Arguments
/// - `clock_id`: CLOCK_REALTIME (0) or CLOCK_MONOTONIC (1).
/// - `tp_ptr`: User-space pointer to a `struct timespec`.
///
/// # Returns
/// 0 on success.
pub fn sys_clock_gettime(clock_id: usize, tp_ptr: usize) -> SyscallResult {
    validate_user_ptr_typed::<Timespec>(tp_ptr)?;

    let uptime_ms = crate::timer::get_uptime_ms();

    let ts = match clock_id {
        CLOCK_MONOTONIC => Timespec {
            tv_sec: (uptime_ms / 1000) as i64,
            tv_nsec: ((uptime_ms % 1000) * 1_000_000) as i64,
        },
        CLOCK_REALTIME => {
            // Realtime = monotonic (no RTC yet, epoch starts at boot)
            Timespec {
                tv_sec: (uptime_ms / 1000) as i64,
                tv_nsec: ((uptime_ms % 1000) * 1_000_000) as i64,
            }
        }
        _ => return Err(SyscallError::InvalidArgument),
    };

    // SAFETY: tp_ptr was validated as aligned, non-null, and in user space.
    unsafe {
        core::ptr::write(tp_ptr as *mut Timespec, ts);
    }
    Ok(0)
}

/// Get clock resolution (SYS_CLOCK_GETRES = 161).
///
/// # Arguments
/// - `clock_id`: CLOCK_REALTIME (0) or CLOCK_MONOTONIC (1).
/// - `res_ptr`: User-space pointer to a `struct timespec` (may be NULL).
///
/// # Returns
/// 0 on success.
pub fn sys_clock_getres(clock_id: usize, res_ptr: usize) -> SyscallResult {
    match clock_id {
        CLOCK_REALTIME | CLOCK_MONOTONIC => {}
        _ => return Err(SyscallError::InvalidArgument),
    }

    if res_ptr != 0 {
        validate_user_ptr_typed::<Timespec>(res_ptr)?;
        // Timer resolution is 1ms (hardware timer tick granularity)
        let res = Timespec {
            tv_sec: 0,
            tv_nsec: 1_000_000, // 1ms in nanoseconds
        };
        // SAFETY: res_ptr was validated above.
        unsafe {
            core::ptr::write(res_ptr as *mut Timespec, res);
        }
    }
    Ok(0)
}

/// Sleep for a specified duration (SYS_NANOSLEEP = 162).
///
/// # Arguments
/// - `req_ptr`: User-space pointer to a `struct timespec` with the requested
///   sleep duration.
/// - `rem_ptr`: User-space pointer to a `struct timespec` for remaining time
///   (may be NULL). Set to zero on normal completion.
///
/// # Returns
/// 0 on success.
pub fn sys_nanosleep(req_ptr: usize, rem_ptr: usize) -> SyscallResult {
    validate_user_ptr_typed::<Timespec>(req_ptr)?;

    // SAFETY: req_ptr was validated as aligned, non-null, and in user space.
    let req = unsafe { core::ptr::read(req_ptr as *const Timespec) };

    if req.tv_sec < 0 || req.tv_nsec < 0 || req.tv_nsec >= 1_000_000_000 {
        return Err(SyscallError::InvalidArgument);
    }

    let sleep_ms = (req.tv_sec as u64) * 1000 + (req.tv_nsec as u64) / 1_000_000;
    let start = crate::timer::get_uptime_ms();

    // Busy-wait with yields (no blocking timer infrastructure yet)
    while crate::timer::get_uptime_ms() - start < sleep_ms {
        crate::sched::yield_cpu();
    }

    // Write zero remaining time
    if rem_ptr != 0 {
        validate_user_ptr_typed::<Timespec>(rem_ptr)?;
        let zero = Timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        // SAFETY: rem_ptr was validated above.
        unsafe {
            core::ptr::write(rem_ptr as *mut Timespec, zero);
        }
    }

    Ok(0)
}

/// Get time of day (SYS_GETTIMEOFDAY = 163).
///
/// # Arguments
/// - `tv_ptr`: User-space pointer to a `struct timeval`.
/// - `tz_ptr`: Timezone pointer (ignored, always NULL behavior).
///
/// # Returns
/// 0 on success.
pub fn sys_gettimeofday(tv_ptr: usize, _tz_ptr: usize) -> SyscallResult {
    if tv_ptr == 0 {
        return Err(SyscallError::InvalidPointer);
    }
    validate_user_ptr_typed::<Timeval>(tv_ptr)?;

    let uptime_ms = crate::timer::get_uptime_ms();
    let tv = Timeval {
        tv_sec: (uptime_ms / 1000) as i64,
        tv_usec: ((uptime_ms % 1000) * 1000) as i64,
    };

    // SAFETY: tv_ptr was validated above.
    unsafe {
        core::ptr::write(tv_ptr as *mut Timeval, tv);
    }
    Ok(0)
}
