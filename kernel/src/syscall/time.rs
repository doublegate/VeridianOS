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

// SAFETY: two i64 fields, no padding; every bit pattern is a valid value.
unsafe impl super::userspace::UserPod for Timespec {}

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

    let now_ns = crate::timer::monotonic_ns();
    let mono = Timespec {
        tv_sec: (now_ns / 1_000_000_000) as i64,
        tv_nsec: (now_ns % 1_000_000_000) as i64,
    };

    let ts = match clock_id {
        CLOCK_MONOTONIC => mono,
        // Realtime = monotonic (no RTC-based epoch yet; starts at boot)
        CLOCK_REALTIME => mono,
        _ => return Err(SyscallError::InvalidArgument),
    };

    super::userspace::write_user(tp_ptr, ts)?;
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
        super::userspace::write_user(res_ptr, res)?;
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

    let req: Timespec = super::userspace::read_user(req_ptr)?;

    if req.tv_sec < 0 || req.tv_nsec < 0 || req.tv_nsec >= 1_000_000_000 {
        return Err(SyscallError::InvalidArgument);
    }

    let sleep_ms = (req.tv_sec as u64) * 1000 + (req.tv_nsec as u64) / 1_000_000;

    // Boot-path cooperative dispatch: skip the spin loop so the child
    // thread yields back promptly and the cooperative scheduler can make
    // progress. The sleep is effectively a no-op in this context.
    #[cfg(target_arch = "x86_64")]
    let in_boot_coop = crate::arch::x86_64::usermode::BOOT_CLONE_YIELD_PENDING
        .load(core::sync::atomic::Ordering::Acquire);
    #[cfg(not(target_arch = "x86_64"))]
    let in_boot_coop = false;

    if !in_boot_coop {
        let start = crate::timer::get_uptime_ms();
        // Busy-wait with interrupt-enabled halts so APIC timer ISR can
        // advance UPTIME_MS (SFMASK clears IF on syscall entry).
        while crate::timer::get_uptime_ms() - start < sleep_ms {
            if crate::sched::wait_for_interrupt_in_syscall() {
                return Err(SyscallError::Interrupted);
            }
        }
    }

    // Write zero remaining time
    if rem_ptr != 0 {
        validate_user_ptr_typed::<Timespec>(rem_ptr)?;
        let zero = Timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        super::userspace::write_user(rem_ptr, zero)?;
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

    super::userspace::write_user(tv_ptr, tv)?;
    Ok(0)
}
