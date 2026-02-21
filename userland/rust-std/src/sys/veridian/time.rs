//! Time operations for VeridianOS.
//!
//! Maps Rust time operations to VeridianOS syscalls:
//! - `clock_gettime` -> SYS_CLOCK_GETTIME (160)
//! - `clock_getres` -> SYS_CLOCK_GETRES (161)
//! - `nanosleep` -> SYS_NANOSLEEP (162)
//! - `gettimeofday` -> SYS_GETTIMEOFDAY (163)
//! - `get_uptime` -> SYS_TIME_GET_UPTIME (100)

use super::{
    syscall0, syscall2, syscall_result, SyscallError, SYS_CLOCK_GETRES, SYS_CLOCK_GETTIME,
    SYS_GETTIMEOFDAY, SYS_NANOSLEEP, SYS_TIME_GET_UPTIME,
};

// ============================================================================
// Clock IDs (POSIX)
// ============================================================================

/// System-wide real-time clock.
pub const CLOCK_REALTIME: usize = 0;
/// Monotonic clock (cannot be set, not affected by NTP).
pub const CLOCK_MONOTONIC: usize = 1;
/// Per-process CPU-time clock.
pub const CLOCK_PROCESS_CPUTIME_ID: usize = 2;
/// Per-thread CPU-time clock.
pub const CLOCK_THREAD_CPUTIME_ID: usize = 3;

// ============================================================================
// Time Structures
// ============================================================================

/// POSIX timespec structure.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct Timespec {
    /// Seconds.
    pub tv_sec: i64,
    /// Nanoseconds (0..999_999_999).
    pub tv_nsec: i64,
}

/// POSIX timeval structure.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct Timeval {
    /// Seconds.
    pub tv_sec: i64,
    /// Microseconds (0..999_999).
    pub tv_usec: i64,
}

// ============================================================================
// Time Operations
// ============================================================================

/// Get the current time from a clock source.
///
/// # Arguments
/// - `clock_id`: Clock to query (CLOCK_REALTIME, CLOCK_MONOTONIC, etc.)
/// - `tp`: Pointer to Timespec to fill
pub fn clock_gettime(clock_id: usize, tp: *mut Timespec) -> Result<usize, SyscallError> {
    // SAFETY: Caller must provide a valid Timespec pointer.
    let ret = unsafe { syscall2(SYS_CLOCK_GETTIME, clock_id, tp as usize) };
    syscall_result(ret)
}

/// Get the resolution of a clock source.
pub fn clock_getres(clock_id: usize, res: *mut Timespec) -> Result<usize, SyscallError> {
    // SAFETY: Caller must provide a valid Timespec pointer.
    let ret = unsafe { syscall2(SYS_CLOCK_GETRES, clock_id, res as usize) };
    syscall_result(ret)
}

/// Sleep for the specified duration.
///
/// # Arguments
/// - `req`: Requested sleep duration
/// - `rem`: If non-null, remaining time if interrupted
///
/// # Returns
/// 0 on success (full sleep), or error if interrupted.
pub fn nanosleep(req: *const Timespec, rem: *mut Timespec) -> Result<usize, SyscallError> {
    // SAFETY: Caller must provide valid Timespec pointers (rem may be null).
    let ret = unsafe { syscall2(SYS_NANOSLEEP, req as usize, rem as usize) };
    syscall_result(ret)
}

/// Get the current time of day.
pub fn gettimeofday(tv: *mut Timeval, _tz: usize) -> Result<usize, SyscallError> {
    // SAFETY: Caller must provide a valid Timeval pointer.
    let ret = unsafe { syscall2(SYS_GETTIMEOFDAY, tv as usize, 0) };
    syscall_result(ret)
}

/// Get kernel uptime in ticks.
pub fn get_uptime() -> Result<usize, SyscallError> {
    // SAFETY: This syscall takes no pointer arguments.
    let ret = unsafe { syscall0(SYS_TIME_GET_UPTIME) };
    syscall_result(ret)
}

/// Convenience: sleep for the given number of milliseconds.
pub fn sleep_ms(ms: u64) -> Result<usize, SyscallError> {
    let req = Timespec {
        tv_sec: (ms / 1000) as i64,
        tv_nsec: ((ms % 1000) * 1_000_000) as i64,
    };
    nanosleep(&req as *const Timespec, core::ptr::null_mut())
}

/// Convenience: sleep for the given number of seconds.
pub fn sleep(seconds: u64) -> Result<usize, SyscallError> {
    let req = Timespec {
        tv_sec: seconds as i64,
        tv_nsec: 0,
    };
    nanosleep(&req as *const Timespec, core::ptr::null_mut())
}
