//! Time management system calls
//!
//! Provides kernel-side implementation of time-related operations:
//! monotonic uptime queries, POSIX clock/time functions, nanosleep,
//! and software timer creation/cancellation.
//! All operations delegate to the [`crate::timer`] subsystem.

use super::{validate_user_ptr_typed, SyscallError, SyscallResult};

/// Get monotonic uptime in milliseconds (SYS_TIME_GET_UPTIME = 1124)
///
/// # Returns
/// Current uptime in milliseconds since boot.
pub fn sys_time_get_uptime() -> SyscallResult {
    Ok(crate::timer::get_uptime_ms() as usize)
}

/// Create a new timer (SYS_TIME_CREATE_TIMER = 1125)
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

/// Cancel an active timer (SYS_TIME_CANCEL_TIMER = 1126)
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

/// Linux clock IDs (N-218).
const CLOCK_REALTIME: i32 = 0;
const CLOCK_MONOTONIC: i32 = 1;
const CLOCK_PROCESS_CPUTIME_ID: i32 = 2;
const CLOCK_THREAD_CPUTIME_ID: i32 = 3;
const CLOCK_MONOTONIC_RAW: i32 = 4;
const CLOCK_REALTIME_COARSE: i32 = 5;
const CLOCK_MONOTONIC_COARSE: i32 = 6;
const CLOCK_BOOTTIME: i32 = 7;
const CLOCK_REALTIME_ALARM: i32 = 8;
const CLOCK_BOOTTIME_ALARM: i32 = 9;
const CLOCK_TAI: i32 = 11;

/// What a CPU clock counts (the low two bits of a CPU clock ID).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CpuClockKind {
    /// User and system time (CPUCLOCK_PROF).
    Prof,
    /// User time (CPUCLOCK_VIRT).
    Virt,
    /// Run time (CPUCLOCK_SCHED).
    Sched,
}

/// A clock a clock ID names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Clock {
    /// Wall time (REALTIME, its coarse and alarm forms, TAI: no leap
    /// second offset is kept, so TAI reads as UTC, as on Linux until one
    /// is set).
    Realtime,
    /// Time since boot (MONOTONIC, RAW, COARSE, BOOTTIME and its alarm
    /// form: there is no suspend to leave out).
    Monotonic,
    /// The CPU time of a process (`None`: the caller's).
    ProcessCpu(Option<u64>, CpuClockKind),
    /// The CPU time of a thread (`None`: the calling thread).
    ThreadCpu(Option<u64>, CpuClockKind),
}

impl Clock {
    /// Decode a clock ID: the fixed ones, or a CPU clock made by
    /// clock_getcpuclockid / pthread_getcpuclockid (`~id << 3 | thread << 2
    /// | kind`, Linux's encoding; id 0 is the caller). EINVAL otherwise.
    pub(crate) fn decode(id: i32) -> Result<Self, SyscallError> {
        if id < 0 {
            let kind = match id & 3 {
                0 => CpuClockKind::Prof,
                1 => CpuClockKind::Virt,
                2 => CpuClockKind::Sched,
                _ => return Err(SyscallError::InvalidArgument),
            };
            let target = (!(id >> 3)) as u32 as u64;
            let target = (target != 0).then_some(target);
            return Ok(if id & 4 != 0 {
                Clock::ThreadCpu(target, kind)
            } else {
                Clock::ProcessCpu(target, kind)
            });
        }
        match id {
            CLOCK_REALTIME | CLOCK_REALTIME_COARSE | CLOCK_REALTIME_ALARM | CLOCK_TAI => {
                Ok(Clock::Realtime)
            }
            CLOCK_MONOTONIC
            | CLOCK_MONOTONIC_RAW
            | CLOCK_MONOTONIC_COARSE
            | CLOCK_BOOTTIME
            | CLOCK_BOOTTIME_ALARM => Ok(Clock::Monotonic),
            CLOCK_PROCESS_CPUTIME_ID => Ok(Clock::ProcessCpu(None, CpuClockKind::Sched)),
            CLOCK_THREAD_CPUTIME_ID => Ok(Clock::ThreadCpu(None, CpuClockKind::Sched)),
            _ => Err(SyscallError::InvalidArgument),
        }
    }
}

/// The nanoseconds a CPU clock of `kind` reads from `times`.
fn cpu_clock_ns(times: crate::sched::cputime::CpuTimes, kind: CpuClockKind) -> u64 {
    match kind {
        CpuClockKind::Prof | CpuClockKind::Sched => times.runtime_ns,
        CpuClockKind::Virt => times.user_system_ns().0,
    }
}

/// A CPU clock's reading: any process's (as Linux lets clock_gettime read
/// it), only the caller's own threads'; EINVAL for none such.
#[cfg(feature = "alloc")]
fn read_cpu_clock(clock: Clock) -> Result<u64, SyscallError> {
    let caller = crate::process::current_process().ok_or(SyscallError::InvalidState)?;
    match clock {
        Clock::ProcessCpu(pid, kind) => {
            let process = match pid {
                None => caller,
                Some(pid) => crate::process::find_process(crate::process::ProcessId(pid))
                    .ok_or(SyscallError::InvalidArgument)?,
            };
            Ok(cpu_clock_ns(process.cpu_times(), kind))
        }
        Clock::ThreadCpu(tid, kind) => {
            let tid = match tid {
                None => {
                    crate::process::current_thread()
                        .ok_or(SyscallError::InvalidState)?
                        .tid
                        .0
                }
                Some(tid) => tid,
            };
            if caller.get_thread(crate::process::ThreadId(tid)).is_none() {
                return Err(SyscallError::InvalidArgument);
            }
            let times = crate::sched::dispatch::thread_cpu((caller.pid.0, tid)).unwrap_or_default();
            Ok(cpu_clock_ns(times, kind))
        }
        _ => Err(SyscallError::InvalidArgument),
    }
}

/// The time on `clock` in nanoseconds (wall time may be negative: before
/// 1970).
fn clock_now_ns(clock: Clock) -> Result<i64, SyscallError> {
    match clock {
        Clock::Realtime => Ok(crate::timer::realtime::now_ns()),
        Clock::Monotonic => Ok(crate::timer::monotonic_ns().min(i64::MAX as u64) as i64),
        #[cfg(feature = "alloc")]
        _ => Ok(read_cpu_clock(clock)?.min(i64::MAX as u64) as i64),
        #[cfg(not(feature = "alloc"))]
        _ => Err(SyscallError::InvalidArgument),
    }
}

/// POSIX timespec structure layout (matches C struct timespec).
#[repr(C)]
#[derive(Clone, Copy)]
struct Timespec {
    tv_sec: i64,
    tv_nsec: i64,
}

// SAFETY: two i64 fields, no padding; every bit pattern is a valid value.
unsafe impl super::userspace::UserPod for Timespec {}

impl Timespec {
    /// `ns` as seconds and nanoseconds, the nanoseconds in 0..1e9 also for
    /// negative times.
    fn from_ns(ns: i64) -> Self {
        Self {
            tv_sec: ns.div_euclid(1_000_000_000),
            tv_nsec: ns.rem_euclid(1_000_000_000),
        }
    }

    /// The nanoseconds a valid timespec gives (EINVAL for a negative or
    /// out-of-range field).
    fn to_ns(self) -> Result<i64, SyscallError> {
        if self.tv_sec < 0 || !(0..1_000_000_000).contains(&self.tv_nsec) {
            return Err(SyscallError::InvalidArgument);
        }
        Ok(self
            .tv_sec
            .saturating_mul(1_000_000_000)
            .saturating_add(self.tv_nsec))
    }
}

/// POSIX timeval structure layout (matches C struct timeval).
#[repr(C)]
#[derive(Clone, Copy)]
struct Timeval {
    tv_sec: i64,
    tv_usec: i64,
}

// SAFETY: two i64 fields, no padding; every bit pattern is a valid value.
unsafe impl super::userspace::UserPod for Timeval {}

/// `struct timezone`, which settimeofday stores and gettimeofday reports
/// (obsolete; nothing in the kernel uses it).
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Timezone {
    tz_minuteswest: i32,
    tz_dsttime: i32,
}

// SAFETY: two i32 fields, no padding; every bit pattern is a valid value.
unsafe impl super::userspace::UserPod for Timezone {}

/// The timezone settimeofday last set (minutes west, DST type).
static SYS_TZ: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

/// Setting the wall clock needs privilege (CAP_SYS_TIME; root here). A
/// caller that is not a process is refused, not trusted.
fn require_time_privilege() -> Result<(), SyscallError> {
    match crate::process::current_process() {
        Some(p) if p.euid() == 0 => Ok(()),
        _ => Err(SyscallError::OperationNotPermitted),
    }
}

/// The wall clock moved by `delta_ns`: timers on it follow.
fn clock_was_set(delta_ns: i64) {
    #[cfg(feature = "alloc")]
    crate::fs::timerfd::clock_was_set(delta_ns);
    #[cfg(not(feature = "alloc"))]
    let _ = delta_ns;
}

/// clock_gettime (Linux 228): every Linux clock (N-218), REALTIME counting
/// from 1970 (N-219).
pub fn sys_clock_gettime(clock_id: usize, tp_ptr: usize) -> SyscallResult {
    let clock = Clock::decode(clock_id as i32)?;
    let ns = clock_now_ns(clock)?;
    super::userspace::write_user(tp_ptr, Timespec::from_ns(ns))?;
    Ok(0)
}

/// clock_getres (Linux 229): one nanosecond for every clock (the clock
/// source is the TSC, and the coarse clocks read it too); EINVAL for an
/// unknown clock, a NULL `res` allowed.
pub fn sys_clock_getres(clock_id: usize, res_ptr: usize) -> SyscallResult {
    let clock = Clock::decode(clock_id as i32)?;
    // A CPU clock of an unknown process or thread is EINVAL here too.
    clock_now_ns(clock)?;
    if res_ptr != 0 {
        super::userspace::write_user(res_ptr, Timespec::from_ns(1))?;
    }
    Ok(0)
}

/// clock_settime (Linux 227): only CLOCK_REALTIME is settable (EINVAL for
/// the others; EPERM for CPU clocks, as Linux), by root.
pub fn sys_clock_settime(clock_id: usize, tp_ptr: usize) -> SyscallResult {
    match clock_id as i32 {
        CLOCK_REALTIME => {}
        id if matches!(
            Clock::decode(id)?,
            Clock::ProcessCpu(..) | Clock::ThreadCpu(..)
        ) =>
        {
            return Err(SyscallError::OperationNotPermitted)
        }
        _ => return Err(SyscallError::InvalidArgument),
    }
    let ts: Timespec = super::userspace::read_user(tp_ptr)?;
    let ns = ts.to_ns()?;
    require_time_privilege()?;
    clock_was_set(crate::timer::realtime::set_now_ns(ns));
    Ok(0)
}

/// gettimeofday (Linux 96): the wall time, and the stored timezone; either
/// pointer may be NULL (N-220).
pub fn sys_gettimeofday(tv_ptr: usize, tz_ptr: usize) -> SyscallResult {
    if tv_ptr != 0 {
        let ts = Timespec::from_ns(crate::timer::realtime::now_ns());
        let tv = Timeval {
            tv_sec: ts.tv_sec,
            tv_usec: ts.tv_nsec / 1000,
        };
        super::userspace::write_user(tv_ptr, tv)?;
    }
    if tz_ptr != 0 {
        let tz = SYS_TZ.load(core::sync::atomic::Ordering::Relaxed);
        super::userspace::write_user(
            tz_ptr,
            Timezone {
                tz_minuteswest: tz as u32 as i32,
                tz_dsttime: (tz >> 32) as u32 as i32,
            },
        )?;
    }
    Ok(0)
}

/// settimeofday (Linux 164): set the wall time and/or the timezone, by
/// root; EINVAL for a bad timeval or a timezone offset beyond 15 hours.
pub fn sys_settimeofday(tv_ptr: usize, tz_ptr: usize) -> SyscallResult {
    let tv: Option<Timeval> = (tv_ptr != 0)
        .then(|| super::userspace::read_user(tv_ptr))
        .transpose()?;
    let tz: Option<Timezone> = (tz_ptr != 0)
        .then(|| super::userspace::read_user(tz_ptr))
        .transpose()?;
    let wall_ns = match tv {
        Some(tv) => {
            if tv.tv_sec < 0 || !(0..1_000_000).contains(&tv.tv_usec) {
                return Err(SyscallError::InvalidArgument);
            }
            Some(
                tv.tv_sec
                    .saturating_mul(1_000_000_000)
                    .saturating_add(tv.tv_usec * 1000),
            )
        }
        None => None,
    };
    if let Some(tz) = tz {
        if !(-15 * 60..=15 * 60).contains(&tz.tz_minuteswest) {
            return Err(SyscallError::InvalidArgument);
        }
    }
    require_time_privilege()?;
    if let Some(tz) = tz {
        SYS_TZ.store(
            (tz.tz_minuteswest as u32 as u64) | ((tz.tz_dsttime as u32 as u64) << 32),
            core::sync::atomic::Ordering::Relaxed,
        );
    }
    if let Some(ns) = wall_ns {
        clock_was_set(crate::timer::realtime::set_now_ns(ns));
    }
    Ok(0)
}

/// time (Linux 201): the wall time in seconds, also stored at `tloc` if it
/// is not NULL.
pub fn sys_time(tloc: usize) -> SyscallResult {
    let secs = Timespec::from_ns(crate::timer::realtime::now_ns()).tv_sec;
    if tloc != 0 {
        super::userspace::write_user(tloc, secs)?;
    }
    Ok(secs as usize)
}

/// Sleep for a specified duration (SYS_nanosleep = 35).
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

    // A dispatched thread really sleeps (see `sleep_until`).
    #[cfg(feature = "alloc")]
    if crate::sched::dispatch::current_owner().is_some() {
        let total = (req.tv_sec as u64)
            .saturating_mul(1_000_000_000)
            .saturating_add(req.tv_nsec as u64);
        let deadline = crate::sched::dispatch::clock_ns().saturating_add(total);
        return sleep_until(deadline, rem_ptr);
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

/// Sleep the calling dispatched thread until `deadline` (`monotonic_ns`):
/// off the run queue until the timer wakes it, or a signal does -- then
/// EINTR, with the time left written to `rem_ptr` (if non-zero) as Linux
/// does. On completion `rem_ptr` gets zero.
#[cfg(feature = "alloc")]
pub(crate) fn sleep_until(deadline: u64, rem_ptr: usize) -> SyscallResult {
    use crate::sched::dispatch;
    static SLEEP: dispatch::WaitQueue = dispatch::WaitQueue::new();
    let interrupted = matches!(
        dispatch::wait_event(&SLEEP, Some(deadline), || false),
        Err(dispatch::WaitError::Interrupted)
    );
    if rem_ptr != 0 {
        let left = if interrupted {
            deadline.saturating_sub(dispatch::clock_ns())
        } else {
            0
        };
        let rem = Timespec {
            tv_sec: (left / 1_000_000_000) as i64,
            tv_nsec: (left % 1_000_000_000) as i64,
        };
        super::userspace::write_user(rem_ptr, rem)?;
    }
    if interrupted {
        Err(SyscallError::Interrupted)
    } else {
        Ok(0)
    }
}

/// `clock_nanosleep` (Linux 230): a relative sleep, or with TIMER_ABSTIME
/// (flags bit 0) until an absolute time on the clock: on CLOCK_REALTIME
/// the monotonic time the wall clock will then read (a later
/// clock_settime does not move the sleep, which Linux would). A thread CPU
/// clock is EINVAL as on Linux; sleeping on a process CPU clock is not
/// supported (ENOTSUP). An absolute sleep never writes `rem`.
pub fn sys_clock_nanosleep(
    clock_id: usize,
    flags: usize,
    req_ptr: usize,
    rem_ptr: usize,
) -> SyscallResult {
    const TIMER_ABSTIME: usize = 1;
    let clock = Clock::decode(clock_id as i32)?;
    match clock {
        Clock::Realtime | Clock::Monotonic => {}
        Clock::ThreadCpu(..) => return Err(SyscallError::InvalidArgument),
        Clock::ProcessCpu(..) => return Err(SyscallError::NotSupported),
    }
    if flags & TIMER_ABSTIME == 0 {
        return sys_nanosleep(req_ptr, rem_ptr);
    }
    let req: Timespec = super::userspace::read_user(req_ptr)?;
    let at = req.to_ns()?;
    let deadline = match clock {
        Clock::Realtime => crate::timer::realtime::monotonic_at(at),
        _ => at as u64,
    };
    #[cfg(feature = "alloc")]
    if crate::sched::dispatch::current_owner().is_some() {
        return sleep_until(deadline, 0);
    }
    while crate::timer::monotonic_ns() < deadline {
        if crate::sched::wait_for_interrupt_in_syscall() {
            return Err(SyscallError::Interrupted);
        }
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Linux's CPU clock IDs: `~id << 3 | thread << 2 | kind`.
    fn cpu_clock(id: u32, thread: bool, kind: i32) -> i32 {
        ((!id as i32) << 3) | ((thread as i32) << 2) | kind
    }

    #[test]
    fn clock_ids_decode_as_linux() {
        assert_eq!(Clock::decode(CLOCK_REALTIME), Ok(Clock::Realtime));
        assert_eq!(Clock::decode(CLOCK_TAI), Ok(Clock::Realtime));
        assert_eq!(Clock::decode(CLOCK_BOOTTIME_ALARM), Ok(Clock::Monotonic));
        assert_eq!(Clock::decode(CLOCK_MONOTONIC_COARSE), Ok(Clock::Monotonic));
        assert_eq!(
            Clock::decode(CLOCK_PROCESS_CPUTIME_ID),
            Ok(Clock::ProcessCpu(None, CpuClockKind::Sched))
        );
        assert_eq!(
            Clock::decode(CLOCK_THREAD_CPUTIME_ID),
            Ok(Clock::ThreadCpu(None, CpuClockKind::Sched))
        );
        for bad in [10, 12, 16, i32::MAX] {
            assert_eq!(Clock::decode(bad), Err(SyscallError::InvalidArgument));
        }
        // clock_getcpuclockid(42) and pthread_getcpuclockid(tid 7).
        assert_eq!(
            Clock::decode(cpu_clock(42, false, 2)),
            Ok(Clock::ProcessCpu(Some(42), CpuClockKind::Sched))
        );
        assert_eq!(
            Clock::decode(cpu_clock(7, true, 1)),
            Ok(Clock::ThreadCpu(Some(7), CpuClockKind::Virt))
        );
        assert_eq!(
            Clock::decode(cpu_clock(0, false, 0)),
            Ok(Clock::ProcessCpu(None, CpuClockKind::Prof))
        );
        // Kind 3 does not exist.
        assert_eq!(
            Clock::decode(cpu_clock(42, false, 3)),
            Err(SyscallError::InvalidArgument)
        );
    }

    #[test]
    fn timespecs_convert_both_ways() {
        let t = Timespec::from_ns(-1);
        assert_eq!((t.tv_sec, t.tv_nsec), (-1, 999_999_999));
        let t = Timespec::from_ns(1_500_000_000);
        assert_eq!((t.tv_sec, t.tv_nsec), (1, 500_000_000));
        assert_eq!(t.to_ns(), Ok(1_500_000_000));
        let bad = Timespec {
            tv_sec: 1,
            tv_nsec: 1_000_000_000,
        };
        assert_eq!(bad.to_ns(), Err(SyscallError::InvalidArgument));
        let negative = Timespec {
            tv_sec: -1,
            tv_nsec: 0,
        };
        assert_eq!(negative.to_ns(), Err(SyscallError::InvalidArgument));
        // The CPU clock kinds read run time or its user part.
        let times = crate::sched::cputime::CpuTimes {
            runtime_ns: 100,
            user_ticks: 1,
            system_ticks: 1,
            ..crate::sched::cputime::CpuTimes::ZERO
        };
        assert_eq!(cpu_clock_ns(times, CpuClockKind::Sched), 100);
        assert_eq!(cpu_clock_ns(times, CpuClockKind::Prof), 100);
        assert_eq!(cpu_clock_ns(times, CpuClockKind::Virt), 50);
    }
}
