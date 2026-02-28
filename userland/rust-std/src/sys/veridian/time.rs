//! Time operations for VeridianOS.
//!
//! Provides both low-level syscall wrappers and higher-level types:
//!
//! - Low-level: `clock_gettime`, `clock_getres`, `nanosleep`, `gettimeofday`,
//!   `get_uptime`
//! - High-level: `Duration`, `Instant`, `SystemTime`
//!
//! Syscall mappings:
//! - `clock_gettime`  -> SYS_CLOCK_GETTIME (160)
//! - `clock_getres`   -> SYS_CLOCK_GETRES (161)
//! - `nanosleep`      -> SYS_NANOSLEEP (162)
//! - `gettimeofday`   -> SYS_GETTIMEOFDAY (163)
//! - `get_uptime`     -> SYS_TIME_GET_UPTIME (100)

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
// Low-level syscall wrappers (preserved from original API)
// ============================================================================

/// Get the current time from a clock source.
pub fn clock_gettime(clock_id: usize, tp: *mut Timespec) -> Result<usize, SyscallError> {
    let ret = unsafe { syscall2(SYS_CLOCK_GETTIME, clock_id, tp as usize) };
    syscall_result(ret)
}

/// Get the resolution of a clock source.
pub fn clock_getres(clock_id: usize, res: *mut Timespec) -> Result<usize, SyscallError> {
    let ret = unsafe { syscall2(SYS_CLOCK_GETRES, clock_id, res as usize) };
    syscall_result(ret)
}

/// Sleep for the specified duration.
pub fn nanosleep(req: *const Timespec, rem: *mut Timespec) -> Result<usize, SyscallError> {
    let ret = unsafe { syscall2(SYS_NANOSLEEP, req as usize, rem as usize) };
    syscall_result(ret)
}

/// Get the current time of day.
pub fn gettimeofday(tv: *mut Timeval, _tz: usize) -> Result<usize, SyscallError> {
    let ret = unsafe { syscall2(SYS_GETTIMEOFDAY, tv as usize, 0) };
    syscall_result(ret)
}

/// Get kernel uptime in ticks.
pub fn get_uptime() -> Result<usize, SyscallError> {
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

// ============================================================================
// Duration
// ============================================================================

/// A span of time with nanosecond precision.
///
/// This is a simplified version of `std::time::Duration`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Duration {
    secs: u64,
    nanos: u32, // 0..999_999_999
}

impl Duration {
    /// Zero duration.
    pub const ZERO: Duration = Duration { secs: 0, nanos: 0 };

    /// One second.
    pub const SECOND: Duration = Duration { secs: 1, nanos: 0 };

    /// One millisecond.
    pub const MILLISECOND: Duration = Duration {
        secs: 0,
        nanos: 1_000_000,
    };

    /// One microsecond.
    pub const MICROSECOND: Duration = Duration {
        secs: 0,
        nanos: 1_000,
    };

    /// One nanosecond.
    pub const NANOSECOND: Duration = Duration { secs: 0, nanos: 1 };

    /// Create from seconds and nanoseconds.
    pub const fn new(secs: u64, nanos: u32) -> Self {
        let extra_secs = (nanos / 1_000_000_000) as u64;
        Duration {
            secs: secs + extra_secs,
            nanos: nanos % 1_000_000_000,
        }
    }

    /// Create from seconds.
    pub const fn from_secs(secs: u64) -> Self {
        Duration { secs, nanos: 0 }
    }

    /// Create from milliseconds.
    pub const fn from_millis(millis: u64) -> Self {
        Duration {
            secs: millis / 1_000,
            nanos: ((millis % 1_000) * 1_000_000) as u32,
        }
    }

    /// Create from microseconds.
    pub const fn from_micros(micros: u64) -> Self {
        Duration {
            secs: micros / 1_000_000,
            nanos: ((micros % 1_000_000) * 1_000) as u32,
        }
    }

    /// Create from nanoseconds.
    pub const fn from_nanos(nanos: u64) -> Self {
        Duration {
            secs: nanos / 1_000_000_000,
            nanos: (nanos % 1_000_000_000) as u32,
        }
    }

    /// Return the whole seconds.
    pub const fn as_secs(&self) -> u64 {
        self.secs
    }

    /// Return the nanosecond component.
    pub const fn subsec_nanos(&self) -> u32 {
        self.nanos
    }

    /// Return the millisecond component.
    pub const fn subsec_millis(&self) -> u32 {
        self.nanos / 1_000_000
    }

    /// Return the microsecond component.
    pub const fn subsec_micros(&self) -> u32 {
        self.nanos / 1_000
    }

    /// Total duration in milliseconds.
    pub const fn as_millis(&self) -> u128 {
        (self.secs as u128) * 1_000 + (self.nanos as u128) / 1_000_000
    }

    /// Total duration in microseconds.
    pub const fn as_micros(&self) -> u128 {
        (self.secs as u128) * 1_000_000 + (self.nanos as u128) / 1_000
    }

    /// Total duration in nanoseconds.
    pub const fn as_nanos(&self) -> u128 {
        (self.secs as u128) * 1_000_000_000 + self.nanos as u128
    }

    /// Is this a zero-length duration?
    pub const fn is_zero(&self) -> bool {
        self.secs == 0 && self.nanos == 0
    }

    /// Checked addition.
    pub fn checked_add(self, rhs: Duration) -> Option<Duration> {
        let mut secs = self.secs.checked_add(rhs.secs)?;
        let mut nanos = self.nanos + rhs.nanos;
        if nanos >= 1_000_000_000 {
            nanos -= 1_000_000_000;
            secs = secs.checked_add(1)?;
        }
        Some(Duration { secs, nanos })
    }

    /// Checked subtraction.
    pub fn checked_sub(self, rhs: Duration) -> Option<Duration> {
        let mut secs = self.secs.checked_sub(rhs.secs)?;
        let nanos;
        if self.nanos >= rhs.nanos {
            nanos = self.nanos - rhs.nanos;
        } else {
            secs = secs.checked_sub(1)?;
            nanos = 1_000_000_000 + self.nanos - rhs.nanos;
        }
        Some(Duration { secs, nanos })
    }

    /// Saturating subtraction.
    pub fn saturating_sub(self, rhs: Duration) -> Duration {
        self.checked_sub(rhs).unwrap_or(Duration::ZERO)
    }

    /// Convert to a `Timespec`.
    pub fn to_timespec(&self) -> Timespec {
        Timespec {
            tv_sec: self.secs as i64,
            tv_nsec: self.nanos as i64,
        }
    }

    /// Create from a `Timespec`.
    pub fn from_timespec(ts: &Timespec) -> Self {
        Duration {
            secs: ts.tv_sec as u64,
            nanos: ts.tv_nsec as u32,
        }
    }
}

impl core::ops::Add for Duration {
    type Output = Duration;
    fn add(self, rhs: Duration) -> Duration {
        self.checked_add(rhs)
            .expect("overflow when adding durations")
    }
}

impl core::ops::Sub for Duration {
    type Output = Duration;
    fn sub(self, rhs: Duration) -> Duration {
        self.checked_sub(rhs)
            .expect("overflow when subtracting durations")
    }
}

// ============================================================================
// Instant
// ============================================================================

/// A measurement of a monotonically non-decreasing clock.
///
/// Uses `CLOCK_MONOTONIC` via `clock_gettime(160)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Instant {
    secs: u64,
    nanos: u32,
}

impl Instant {
    /// Capture the current instant.
    pub fn now() -> Self {
        let mut ts = Timespec::default();
        let _ = clock_gettime(CLOCK_MONOTONIC, &mut ts);
        Instant {
            secs: ts.tv_sec as u64,
            nanos: ts.tv_nsec as u32,
        }
    }

    /// Time elapsed since this instant was captured.
    pub fn elapsed(&self) -> Duration {
        let now = Instant::now();
        now.duration_since(*self)
    }

    /// Duration between `self` and an earlier instant.
    ///
    /// Panics if `earlier` is later than `self`.
    pub fn duration_since(&self, earlier: Instant) -> Duration {
        self.checked_duration_since(earlier)
            .expect("supplied instant is later than self")
    }

    /// Checked duration between two instants.
    pub fn checked_duration_since(&self, earlier: Instant) -> Option<Duration> {
        let secs;
        let nanos;
        if self.nanos >= earlier.nanos {
            secs = self.secs.checked_sub(earlier.secs)?;
            nanos = self.nanos - earlier.nanos;
        } else {
            secs = self.secs.checked_sub(earlier.secs)?.checked_sub(1)?;
            nanos = 1_000_000_000 + self.nanos - earlier.nanos;
        }
        Some(Duration { secs, nanos })
    }

    /// Saturating duration since an earlier instant.
    pub fn saturating_duration_since(&self, earlier: Instant) -> Duration {
        self.checked_duration_since(earlier)
            .unwrap_or(Duration::ZERO)
    }

    /// Add a duration to this instant.
    pub fn checked_add(&self, dur: Duration) -> Option<Instant> {
        let mut secs = self.secs.checked_add(dur.secs)?;
        let mut nanos = self.nanos + dur.nanos;
        if nanos >= 1_000_000_000 {
            nanos -= 1_000_000_000;
            secs = secs.checked_add(1)?;
        }
        Some(Instant { secs, nanos })
    }
}

impl core::ops::Add<Duration> for Instant {
    type Output = Instant;
    fn add(self, dur: Duration) -> Instant {
        self.checked_add(dur)
            .expect("overflow when adding duration to instant")
    }
}

// ============================================================================
// SystemTime
// ============================================================================

/// A measurement of the system clock (wall-clock time).
///
/// Uses `CLOCK_REALTIME` via `clock_gettime(160)`.  This clock can be
/// adjusted (e.g. by NTP) and is not guaranteed to be monotonic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct SystemTime {
    secs: u64,
    nanos: u32,
}

impl SystemTime {
    /// The Unix epoch: 1970-01-01T00:00:00Z.
    pub const UNIX_EPOCH: SystemTime = SystemTime { secs: 0, nanos: 0 };

    /// Get the current system time.
    pub fn now() -> Self {
        let mut ts = Timespec::default();
        let _ = clock_gettime(CLOCK_REALTIME, &mut ts);
        SystemTime {
            secs: ts.tv_sec as u64,
            nanos: ts.tv_nsec as u32,
        }
    }

    /// Duration since the UNIX epoch.
    pub fn duration_since_epoch(&self) -> Duration {
        Duration {
            secs: self.secs,
            nanos: self.nanos,
        }
    }

    /// Duration since an earlier `SystemTime`.
    pub fn duration_since(&self, earlier: SystemTime) -> Result<Duration, SyscallError> {
        let secs;
        let nanos;
        if self.nanos >= earlier.nanos {
            secs = self
                .secs
                .checked_sub(earlier.secs)
                .ok_or(SyscallError::InvalidArgument)?;
            nanos = self.nanos - earlier.nanos;
        } else {
            secs = self
                .secs
                .checked_sub(earlier.secs)
                .and_then(|s| s.checked_sub(1))
                .ok_or(SyscallError::InvalidArgument)?;
            nanos = 1_000_000_000 + self.nanos - earlier.nanos;
        }
        Ok(Duration { secs, nanos })
    }

    /// Time elapsed since this measurement.
    pub fn elapsed(&self) -> Result<Duration, SyscallError> {
        SystemTime::now().duration_since(*self)
    }

    /// Add a duration.
    pub fn checked_add(&self, dur: Duration) -> Option<SystemTime> {
        let mut secs = self.secs.checked_add(dur.secs)?;
        let mut nanos = self.nanos + dur.nanos;
        if nanos >= 1_000_000_000 {
            nanos -= 1_000_000_000;
            secs = secs.checked_add(1)?;
        }
        Some(SystemTime { secs, nanos })
    }

    /// Subtract a duration.
    pub fn checked_sub(&self, dur: Duration) -> Option<SystemTime> {
        let secs;
        let nanos;
        if self.nanos >= dur.nanos {
            secs = self.secs.checked_sub(dur.secs)?;
            nanos = self.nanos - dur.nanos;
        } else {
            secs = self.secs.checked_sub(dur.secs)?.checked_sub(1)?;
            nanos = 1_000_000_000 + self.nanos - dur.nanos;
        }
        Some(SystemTime { secs, nanos })
    }

    /// Seconds since the Unix epoch.
    pub fn as_secs(&self) -> u64 {
        self.secs
    }

    /// Nanosecond component.
    pub fn subsec_nanos(&self) -> u32 {
        self.nanos
    }
}

impl core::ops::Add<Duration> for SystemTime {
    type Output = SystemTime;
    fn add(self, dur: Duration) -> SystemTime {
        self.checked_add(dur)
            .expect("overflow when adding duration to system time")
    }
}

impl core::ops::Sub<Duration> for SystemTime {
    type Output = SystemTime;
    fn sub(self, dur: Duration) -> SystemTime {
        self.checked_sub(dur)
            .expect("overflow when subtracting duration from system time")
    }
}
