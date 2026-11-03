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
    /// Wall time (REALTIME, its coarse and alarm forms).
    Realtime,
    /// TAI: wall time plus the TAI offset adjtimex sets (ADJ_TAI; zero, so
    /// TAI reads as UTC, until one is set, as on Linux).
    Tai,
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
            CLOCK_REALTIME | CLOCK_REALTIME_COARSE | CLOCK_REALTIME_ALARM => Ok(Clock::Realtime),
            CLOCK_TAI => Ok(Clock::Tai),
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

/// The nanoseconds a CPU clock of `kind` reads from `times`, whose user
/// part is `user_ns` (from its `PrevSplit`, so the clock never goes back).
fn cpu_clock_ns(times: crate::sched::cputime::CpuTimes, user_ns: u64, kind: CpuClockKind) -> u64 {
    match kind {
        CpuClockKind::Prof | CpuClockKind::Sched => times.runtime_ns,
        CpuClockKind::Virt => user_ns,
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
            let (times, (user, _)) = process.cpu_split();
            Ok(cpu_clock_ns(times, user, kind))
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
            let (times, (user, _)) =
                crate::sched::dispatch::thread_cpu_split((caller.pid.0, tid)).unwrap_or_default();
            Ok(cpu_clock_ns(times, user, kind))
        }
        _ => Err(SyscallError::InvalidArgument),
    }
}

/// The time on `clock` (named by `id`) in nanoseconds (wall time may be
/// negative: before 1970); the monotonic and boot-time clocks read the
/// caller's time namespace's offsets ahead (ADR 0022 H2).
fn clock_now_ns(clock: Clock, id: i32) -> Result<i64, SyscallError> {
    match clock {
        Clock::Realtime => Ok(crate::timer::realtime::now_ns()),
        Clock::Tai => Ok(crate::timer::realtime::now_ns().saturating_add(tai_offset_ns())),
        Clock::Monotonic => Ok(
            crate::process::ns::clock_from_host(id, crate::timer::monotonic_ns())
                .min(i64::MAX as u64) as i64,
        ),
        #[cfg(feature = "alloc")]
        _ => Ok(read_cpu_clock(clock)?.min(i64::MAX as u64) as i64),
        #[cfg(not(feature = "alloc"))]
        _ => Err(SyscallError::InvalidArgument),
    }
}

/// CLOCK_TAI minus CLOCK_REALTIME, in nanoseconds.
fn tai_offset_ns() -> i64 {
    crate::timer::ntp::tai_offset_s() as i64 * 1_000_000_000
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

/// Setting the wall clock needs CAP_SYS_TIME (ADR 0020). A caller that is
/// not a process is refused, not trusted.
fn require_time_privilege() -> Result<(), SyscallError> {
    let caller = crate::process::creds::Caller::current();
    if crate::process::current_process().is_some()
        && caller.capable(crate::process::linux_caps::CAP_SYS_TIME)
    {
        Ok(())
    } else {
        Err(SyscallError::OperationNotPermitted)
    }
}

/// The wall clock moved by `delta_ns`: timers on it follow.
fn clock_was_set(delta_ns: i64) {
    #[cfg(feature = "alloc")]
    crate::fs::timerfd::clock_was_set(delta_ns);
    crate::timer::ktimerd::clock_was_set(delta_ns);
}

/// clock_gettime (Linux 228): every Linux clock (N-218), REALTIME counting
/// from 1970 (N-219).
pub fn sys_clock_gettime(clock_id: usize, tp_ptr: usize) -> SyscallResult {
    let clock = Clock::decode(clock_id as i32)?;
    let ns = clock_now_ns(clock, clock_id as i32)?;
    super::userspace::write_user(tp_ptr, Timespec::from_ns(ns))?;
    Ok(0)
}

/// clock_getres (Linux 229): one nanosecond for every clock (the clock
/// source is the TSC, and the coarse clocks read it too); EINVAL for an
/// unknown clock, a NULL `res` allowed.
pub fn sys_clock_getres(clock_id: usize, res_ptr: usize) -> SyscallResult {
    let clock = Clock::decode(clock_id as i32)?;
    // A CPU clock of an unknown process or thread is EINVAL here too.
    clock_now_ns(clock, clock_id as i32)?;
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

// SAFETY: integers only, the padding named as fields (no implicit
// padding); every bit pattern is a valid value.
unsafe impl super::userspace::UserPod for crate::timer::ntp::Timex {}

/// adjtimex on the system clock (Linux's do_adjtimex): validate `tx` as
/// Linux does (EPERM without CAP_SYS_TIME for anything but reading, EINVAL
/// for a bad mode), step CLOCK_REALTIME for ADJ_SETOFFSET (EINVAL if it
/// would read before boot), then apply the modes and fill `tx`. Returns
/// the clock state.
fn do_adjtimex(tx: &mut crate::timer::ntp::Timex) -> Result<i32, SyscallError> {
    use crate::timer::{ntp, realtime};
    ntp::validate(tx, require_time_privilege().is_ok()).map_err(|refused| match refused {
        ntp::Refused::Invalid => SyscallError::InvalidArgument,
        ntp::Refused::NotPermitted => SyscallError::OperationNotPermitted,
    })?;
    let (state, moved, tai_moved) = ntp::with(|ntp| {
        // Linux reports the time read before the step.
        let now = realtime::now_ns();
        let moved = if tx.modes & ntp::ADJ_SETOFFSET != 0 {
            Some(
                realtime::inject_offset(ntp::setoffset_ns(tx))
                    .ok_or(SyscallError::InvalidArgument)?,
            )
        } else {
            None
        };
        let tai = ntp.tai();
        let secs = realtime::now_ns().div_euclid(1_000_000_000);
        let state = ntp.adjtimex(tx, now, secs);
        Ok::<_, SyscallError>((state, moved, ntp.tai() != tai))
    })?;
    if let Some(delta) = moved {
        clock_was_set(delta);
    }
    if tai_moved {
        crate::timer::realtime::mark_set();
        clock_was_set(0);
    }
    Ok(state)
}

/// adjtimex (Linux 159): read the kernel's NTP state, or change it with
/// CAP_SYS_TIME (`timer::ntp`: ADJ_SETOFFSET steps the clock and ADJ_TAI
/// sets CLOCK_TAI; the rest is stored and read back). The struct is
/// written back only on success; returns the clock state (TIME_ERROR, 5,
/// while unsynchronized).
pub fn sys_adjtimex(tx_ptr: usize) -> SyscallResult {
    let mut tx: crate::timer::ntp::Timex = super::userspace::read_user(tx_ptr)?;
    let state = do_adjtimex(&mut tx)?;
    super::userspace::write_user(tx_ptr, tx)?;
    Ok(state as usize)
}

/// What clock_adjtime does with a clock ID (Linux's do_clock_adjtime):
/// CLOCK_REALTIME is adjtimex; every other clock Linux has cannot be
/// adjusted (EOPNOTSUPP), CPU clocks included whatever they name; IDs that
/// name no clock are EINVAL (a posix clock fd, `id & 7 == 3`: there are
/// none; the auxiliary clocks 16..=23: not configured).
fn clock_adjtime_target(id: i32) -> Result<(), SyscallError> {
    match id {
        CLOCK_REALTIME => Ok(()),
        id if id < 0 && id & 7 == 3 => Err(SyscallError::InvalidArgument),
        id if id < 0 => Err(SyscallError::NotSupported),
        1..=9 | CLOCK_TAI => Err(SyscallError::NotSupported),
        _ => Err(SyscallError::InvalidArgument),
    }
}

/// clock_adjtime (Linux 305): adjtimex on CLOCK_REALTIME
/// ([`clock_adjtime_target`] for the others).
pub fn sys_clock_adjtime(clock_id: usize, tx_ptr: usize) -> SyscallResult {
    let mut tx: crate::timer::ntp::Timex = super::userspace::read_user(tx_ptr)?;
    clock_adjtime_target(clock_id as i32)?;
    let state = do_adjtimex(&mut tx)?;
    super::userspace::write_user(tx_ptr, tx)?;
    Ok(state as usize)
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
///   (may be NULL), written only when the sleep is interrupted.
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
        return sleep_until(deadline, rem_ptr, true);
    }

    let sleep_ms = (req.tv_sec as u64) * 1000 + (req.tv_nsec as u64) / 1_000_000;

    // Not dispatched (no user thread is, without the dispatcher): wait with
    // interrupt-enabled halts so the timer can advance the uptime.
    let start = crate::timer::get_uptime_ms();
    while crate::timer::get_uptime_ms() - start < sleep_ms {
        if crate::sched::wait_for_interrupt_in_syscall() {
            return Err(SyscallError::Interrupted);
        }
    }

    // A completed sleep leaves `rem` alone (see `sleep_until`).
    let _ = rem_ptr;
    Ok(0)
}

/// Sleep the calling dispatched thread until `deadline` (`monotonic_ns`):
/// off the run queue until the timer wakes it, or a signal does -- then
/// EINTR, with the time left written to `rem_ptr` (if non-zero) as Linux
/// does. A completed sleep does not touch `rem_ptr`, as on Linux: writing
/// zero there made a sleep with a bad `rem` pointer fail with EFAULT after
/// it had slept (PR #23 review). A `restartable` (relative) sleep that is
/// interrupted leaves a restart block instead of plain EINTR, so after a
/// stop it sleeps to the same deadline (`restart`), as Linux's
/// hrtimer_nanosleep.
#[cfg(feature = "alloc")]
pub(crate) fn sleep_until(deadline: u64, rem_ptr: usize, restartable: bool) -> SyscallResult {
    use crate::sched::dispatch;
    static SLEEP: dispatch::WaitQueue = dispatch::WaitQueue::new();
    let interrupted = matches!(
        dispatch::wait_event(&SLEEP, Some(deadline), || false),
        Err(dispatch::WaitError::Interrupted)
    );
    if interrupted && rem_ptr != 0 {
        let left = deadline.saturating_sub(dispatch::clock_ns());
        let rem = Timespec {
            tv_sec: (left / 1_000_000_000) as i64,
            tv_nsec: (left % 1_000_000_000) as i64,
        };
        super::userspace::write_user(rem_ptr, rem)?;
    }
    if !interrupted {
        Ok(0)
    } else if restartable {
        Err(super::restart::interrupted(
            super::restart::Restart::Sleep { deadline, rem_ptr },
        ))
    } else {
        Err(SyscallError::Interrupted)
    }
}

/// `clock_nanosleep` (Linux 230): a relative sleep, or with TIMER_ABSTIME
/// (flags bit 0) until an absolute time on the clock: on CLOCK_REALTIME
/// the monotonic time the wall clock will then read (a later
/// clock_settime does not move the sleep, which Linux would); on the
/// monotonic and boot-time clocks the time is the caller's time
/// namespace's, taken back to the host's. A thread CPU
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
        Clock::Realtime | Clock::Tai | Clock::Monotonic => {}
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
        Clock::Tai => crate::timer::realtime::monotonic_at(at.saturating_sub(tai_offset_ns())),
        _ => crate::process::ns::clock_to_host(clock_id as i32, at as u64),
    };
    #[cfg(feature = "alloc")]
    if crate::sched::dispatch::current_owner().is_some() {
        return sleep_until(deadline, 0, false);
    }
    while crate::timer::monotonic_ns() < deadline {
        if crate::sched::wait_for_interrupt_in_syscall() {
            return Err(SyscallError::Interrupted);
        }
    }
    Ok(0)
}

/// `struct itimerval` ({it_interval, it_value}, two timevals) as a
/// setting, as Linux's get_itimerval checks it: EINVAL for a negative
/// tv_sec or a tv_usec outside 0..1000000.
fn itimerval_setting(v: [Timeval; 2]) -> Result<crate::process::itimer::Setting, SyscallError> {
    let ns = |t: Timeval| {
        if t.tv_sec < 0 || !(0..1_000_000).contains(&t.tv_usec) {
            return Err(SyscallError::InvalidArgument);
        }
        Ok((t.tv_sec as u64)
            .saturating_mul(1_000_000_000)
            .saturating_add(t.tv_usec as u64 * 1_000))
    };
    Ok(crate::process::itimer::Setting {
        interval: ns(v[0])?,
        value: ns(v[1])?,
    })
}

/// A setting as `struct itimerval`, nanoseconds truncated to microseconds.
fn setting_itimerval(s: crate::process::itimer::Setting) -> [Timeval; 2] {
    let tv = |ns: u64| Timeval {
        tv_sec: (ns / 1_000_000_000) as i64,
        tv_usec: (ns % 1_000_000_000 / 1_000) as i64,
    };
    [tv(s.interval), tv(s.value)]
}

/// getitimer(which, value): ITIMER_REAL, ITIMER_VIRTUAL or ITIMER_PROF
/// (`process::itimer`); EINVAL for another.
#[cfg(feature = "alloc")]
pub fn sys_getitimer(which: usize, value_ptr: usize) -> SyscallResult {
    let process = crate::process::current_process().ok_or(SyscallError::InvalidState)?;
    let setting =
        crate::process::itimer::get(&process, which).ok_or(SyscallError::InvalidArgument)?;
    super::userspace::write_user(value_ptr, setting_itimerval(setting))?;
    Ok(0)
}

/// setitimer(which, new, old): a NULL `new` disarms the timer, as Linux
/// still allows.
#[cfg(feature = "alloc")]
pub fn sys_setitimer(which: usize, new_ptr: usize, old_ptr: usize) -> SyscallResult {
    let new = if new_ptr == 0 {
        crate::process::itimer::Setting::default()
    } else {
        itimerval_setting(super::userspace::read_user::<[Timeval; 2]>(new_ptr)?)?
    };
    let old = set_itimer(which, new)?;
    if old_ptr != 0 {
        super::userspace::write_user(old_ptr, setting_itimerval(old))?;
    }
    Ok(0)
}

#[cfg(feature = "alloc")]
fn set_itimer(
    which: usize,
    new: crate::process::itimer::Setting,
) -> Result<crate::process::itimer::Setting, SyscallError> {
    use crate::process::itimer::SetError;
    let process = crate::process::current_process().ok_or(SyscallError::InvalidState)?;
    crate::process::itimer::set(&process, which, new).map_err(|e| match e {
        SetError::Invalid => SyscallError::InvalidArgument,
        SetError::NoThread => SyscallError::WouldBlock,
    })
}

/// alarm(seconds): ITIMER_REAL to `seconds` (0 disarms it), returning the
/// seconds the replaced setting had left (`itimer::alarm_seconds`).
#[cfg(feature = "alloc")]
pub fn sys_alarm(seconds: usize) -> SyscallResult {
    let new = crate::process::itimer::Setting {
        value: (seconds as u32 as u64) * 1_000_000_000,
        interval: 0,
    };
    let old = set_itimer(crate::process::itimer::ITIMER_REAL, new)?;
    Ok(crate::process::itimer::alarm_seconds(old) as usize)
}

/// `struct sigevent` (64 bytes): the value, signal and notification, then
/// a union of which only the thread id (SIGEV_THREAD_ID) reaches the
/// kernel.
#[repr(C)]
#[derive(Clone, Copy)]
struct Sigevent {
    value: u64,
    signo: i32,
    notify: i32,
    tid: i32,
    pad: [i32; 11],
}

// SAFETY: integers only, no padding (8 + 4 + 4 + 4 + 44 = 64 bytes, 8-byte
// aligned); every bit pattern is a valid value.
unsafe impl super::userspace::UserPod for Sigevent {}

/// The clock timer_create puts a timer on for clock ID `id`, called by
/// thread `tid` of process `pid` (Linux's clockid_to_kclock): EINVAL for an
/// unknown clock; EOPNOTSUPP for one without timers (the raw and coarse
/// clocks) or for another process's CPU clock, which only Linux supports.
/// A relative CLOCK_REALTIME setting runs on the monotonic clock
/// (`posix_timer::settime`).
fn timer_clock(
    id: i32,
    pid: u64,
    tid: u64,
) -> Result<crate::process::posix_timer::TimerClock, SyscallError> {
    use crate::process::posix_timer::TimerClock;
    if matches!(
        id,
        CLOCK_MONOTONIC_RAW | CLOCK_REALTIME_COARSE | CLOCK_MONOTONIC_COARSE
    ) {
        return Err(SyscallError::NotSupported);
    }
    let user = |kind| kind == CpuClockKind::Virt;
    Ok(match Clock::decode(id)? {
        // A timer on CLOCK_TAI runs as one on CLOCK_REALTIME: it does not
        // see the TAI offset (docs/KNOWN-LIMITATIONS.md).
        Clock::Realtime | Clock::Tai => TimerClock::Wall,
        Clock::Monotonic if matches!(id, CLOCK_BOOTTIME | CLOCK_BOOTTIME_ALARM) => TimerClock::Boot,
        Clock::Monotonic => TimerClock::Mono,
        Clock::ProcessCpu(target, kind) => {
            if target.is_some_and(|target| target != pid) {
                return Err(SyscallError::NotSupported);
            }
            TimerClock::ProcessCpu { user: user(kind) }
        }
        Clock::ThreadCpu(target, kind) => TimerClock::ThreadCpu {
            tid: target.unwrap_or(tid),
            user: user(kind),
        },
    })
}

/// A `struct sigevent` as a timer's notification (Linux's good_sigevent):
/// EINVAL for an unknown kind, a signal outside 1..=64, or (with
/// SIGEV_THREAD_ID) a thread `is_thread` does not know.
fn timer_notify(
    event: Sigevent,
    is_thread: impl Fn(u64) -> bool,
) -> Result<crate::process::posix_timer::Notify, SyscallError> {
    use crate::process::posix_timer::{
        Notify, SIGEV_NONE, SIGEV_SIGNAL, SIGEV_THREAD, SIGEV_THREAD_ID,
    };
    let thread = match event.notify {
        SIGEV_NONE => return Ok(Notify::None),
        SIGEV_SIGNAL | SIGEV_THREAD => None,
        SIGEV_THREAD_ID if event.tid > 0 && is_thread(event.tid as u64) => Some(event.tid as u64),
        _ => return Err(SyscallError::InvalidArgument),
    };
    if !(1..=crate::process::signals::NSIG as i32).contains(&event.signo) {
        return Err(SyscallError::InvalidArgument);
    }
    Ok(Notify::Signal {
        signo: event.signo as usize,
        value: event.value,
        thread,
    })
}

#[cfg(feature = "alloc")]
fn timer_error(e: crate::process::posix_timer::TimerError) -> SyscallError {
    match e {
        crate::process::posix_timer::TimerError::Invalid => SyscallError::InvalidArgument,
        crate::process::posix_timer::TimerError::Again => SyscallError::WouldBlock,
    }
}

/// A timer id from a syscall argument (`timer_t` is an int): EINVAL for
/// one outside 0..=INT_MAX, as Linux's lock_timer.
fn timer_id(arg: usize) -> Result<i32, SyscallError> {
    i32::try_from(arg as u32 as i64).map_err(|_| SyscallError::InvalidArgument)
}

/// timer_create(clock, sevp, timerid): a POSIX timer
/// (`process::posix_timer`). The alarm clocks need CAP_WAKE_ALARM (EPERM).
#[cfg(feature = "alloc")]
pub fn sys_timer_create(clock_id: usize, sevp: usize, id_ptr: usize) -> SyscallResult {
    let event = match sevp {
        0 => None,
        ptr => Some(super::userspace::read_user::<Sigevent>(ptr)?),
    };
    let process = crate::process::current_process().ok_or(SyscallError::InvalidState)?;
    let tid = crate::process::current_thread().map_or(process.pid.0, |t| t.tid.0);
    let clock_id = clock_id as i32;
    let clock = timer_clock(clock_id, process.pid.0, tid)?;
    if let crate::process::posix_timer::TimerClock::ThreadCpu { tid, .. } = clock {
        process
            .get_thread(crate::process::ThreadId(tid))
            .ok_or(SyscallError::InvalidArgument)?;
    }
    if matches!(clock_id, CLOCK_REALTIME_ALARM | CLOCK_BOOTTIME_ALARM)
        && !crate::process::creds::Caller::current()
            .capable(crate::process::linux_caps::CAP_WAKE_ALARM)
    {
        return Err(SyscallError::OperationNotPermitted);
    }
    let notify = event
        .map(|event| {
            timer_notify(event, |tid| {
                process.get_thread(crate::process::ThreadId(tid)).is_some()
            })
        })
        .transpose()?;
    let id = crate::process::posix_timer::create(&process, clock, notify).map_err(timer_error)?;
    if let Err(e) = super::userspace::write_user(id_ptr, id) {
        let _ = crate::process::posix_timer::delete(&process, id);
        return Err(e);
    }
    Ok(0)
}

/// `struct itimerspec` ({it_interval, it_value}) as a timer setting
/// (EINVAL for an invalid timespec, as Linux's timespec64_valid).
fn itimerspec_spec(v: [Timespec; 2]) -> Result<crate::process::posix_timer::Spec, SyscallError> {
    Ok(crate::process::posix_timer::Spec {
        interval: v[0].to_ns()?,
        value: v[1].to_ns()?,
    })
}

fn spec_itimerspec(s: crate::process::posix_timer::Spec) -> [Timespec; 2] {
    [
        Timespec::from_ns(s.interval.max(0)),
        Timespec::from_ns(s.value.max(0)),
    ]
}

/// timer_settime(timerid, flags, new, old): TIMER_ABSTIME sets an expiry
/// on the clock, which in a time namespace is first taken back to the
/// host's (Linux's common_timer_set); a NULL `new` is EINVAL.
#[cfg(feature = "alloc")]
pub fn sys_timer_settime(id: usize, flags: usize, new_ptr: usize, old_ptr: usize) -> SyscallResult {
    if new_ptr == 0 {
        return Err(SyscallError::InvalidArgument);
    }
    let new = super::userspace::read_user::<[Timespec; 2]>(new_ptr)?;
    let mut new = itimerspec_spec(new)?;
    let process = crate::process::current_process().ok_or(SyscallError::InvalidState)?;
    let absolute = flags as i32 & crate::process::posix_timer::TIMER_ABSTIME as i32 != 0;
    if absolute && new.value != 0 {
        use crate::process::{ns, posix_timer::TimerClock};
        let clock =
            crate::process::posix_timer::clock_of(&process, timer_id(id)?).map_err(timer_error)?;
        let clock_id = match clock {
            TimerClock::Mono => Some(ns::CLOCK_MONOTONIC),
            TimerClock::Boot => Some(ns::CLOCK_BOOTTIME),
            _ => None,
        };
        if let Some(clock_id) = clock_id {
            // A time already past stays a setting (1 ns, long gone), not 0,
            // which would disarm the timer.
            new.value = (ns::clock_to_host(clock_id, new.value as u64) as i64).max(1);
        }
    }
    let old = crate::process::posix_timer::settime(&process, timer_id(id)?, absolute, new)
        .map_err(timer_error)?;
    if old_ptr != 0 {
        super::userspace::write_user(old_ptr, spec_itimerspec(old))?;
    }
    Ok(0)
}

/// timer_gettime(timerid, curr).
#[cfg(feature = "alloc")]
pub fn sys_timer_gettime(id: usize, cur_ptr: usize) -> SyscallResult {
    let process = crate::process::current_process().ok_or(SyscallError::InvalidState)?;
    let spec =
        crate::process::posix_timer::gettime(&process, timer_id(id)?).map_err(timer_error)?;
    super::userspace::write_user(cur_ptr, spec_itimerspec(spec))?;
    Ok(0)
}

/// timer_getoverrun(timerid): the overrun of its last delivered signal.
#[cfg(feature = "alloc")]
pub fn sys_timer_getoverrun(id: usize) -> SyscallResult {
    let process = crate::process::current_process().ok_or(SyscallError::InvalidState)?;
    crate::process::posix_timer::getoverrun(&process, timer_id(id)?)
        .map(|n| n as usize)
        .map_err(timer_error)
}

/// timer_delete(timerid).
#[cfg(feature = "alloc")]
pub fn sys_timer_delete(id: usize) -> SyscallResult {
    let process = crate::process::current_process().ok_or(SyscallError::InvalidState)?;
    crate::process::posix_timer::delete(&process, timer_id(id)?).map_err(timer_error)?;
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// timer_create's clocks: the raw and coarse clocks have no timers,
    /// another process's CPU clock is not supported, a thread's is named
    /// by its id.
    #[test]
    fn timer_clocks_follow_clockid_to_kclock() {
        use crate::process::posix_timer::TimerClock;
        assert_eq!(timer_clock(CLOCK_REALTIME, 5, 6), Ok(TimerClock::Wall));
        assert_eq!(timer_clock(CLOCK_TAI, 5, 6), Ok(TimerClock::Wall));
        assert_eq!(timer_clock(CLOCK_BOOTTIME, 5, 6), Ok(TimerClock::Boot));
        assert_eq!(timer_clock(CLOCK_MONOTONIC, 5, 6), Ok(TimerClock::Mono));
        for id in [
            CLOCK_MONOTONIC_RAW,
            CLOCK_REALTIME_COARSE,
            CLOCK_MONOTONIC_COARSE,
        ] {
            assert_eq!(timer_clock(id, 5, 6), Err(SyscallError::NotSupported));
        }
        assert_eq!(timer_clock(10, 5, 6), Err(SyscallError::InvalidArgument));
        assert_eq!(
            timer_clock(CLOCK_PROCESS_CPUTIME_ID, 5, 6),
            Ok(TimerClock::ProcessCpu { user: false })
        );
        assert_eq!(
            timer_clock(CLOCK_THREAD_CPUTIME_ID, 5, 6),
            Ok(TimerClock::ThreadCpu {
                tid: 6,
                user: false
            })
        );
        // clock_getcpuclockid(pid): ~pid << 3 | kind (1: CPUCLOCK_VIRT).
        let cpu = |pid: i32, kind| (!pid) << 3 | kind;
        assert_eq!(
            timer_clock(cpu(5, 1), 5, 6),
            Ok(TimerClock::ProcessCpu { user: true })
        );
        assert_eq!(
            timer_clock(cpu(7, 2), 5, 6),
            Err(SyscallError::NotSupported)
        );
        assert_eq!(
            timer_clock(cpu(9, 2) | 4, 5, 6),
            Ok(TimerClock::ThreadCpu {
                tid: 9,
                user: false
            })
        );
    }

    /// good_sigevent: the kinds, the signal range, and SIGEV_THREAD_ID's
    /// thread.
    #[test]
    fn timer_notify_follows_good_sigevent() {
        use crate::process::posix_timer::Notify;
        let event = |notify, signo, tid| Sigevent {
            value: 77,
            signo,
            notify,
            tid,
            pad: [0; 11],
        };
        let known = |tid| tid == 6;
        assert_eq!(timer_notify(event(1, 0, 0), known), Ok(Notify::None));
        let signal = |thread| Notify::Signal {
            signo: 34,
            value: 77,
            thread,
        };
        assert_eq!(timer_notify(event(0, 34, 0), known), Ok(signal(None)));
        assert_eq!(timer_notify(event(2, 34, 0), known), Ok(signal(None)));
        assert_eq!(timer_notify(event(4, 34, 6), known), Ok(signal(Some(6))));
        for bad in [
            event(4, 34, 7),
            event(0, 0, 0),
            event(0, 65, 0),
            event(3, 34, 0),
            event(6, 34, 6),
        ] {
            assert_eq!(timer_notify(bad, known), Err(SyscallError::InvalidArgument));
        }
    }

    /// Timer ids are ints: one past INT_MAX (or negative) names no timer.
    #[test]
    fn timer_ids_are_non_negative_ints() {
        assert_eq!(timer_id(0), Ok(0));
        assert_eq!(timer_id(i32::MAX as usize), Ok(i32::MAX));
        assert_eq!(
            timer_id(-1i32 as u32 as usize),
            Err(SyscallError::InvalidArgument)
        );
        assert_eq!(timer_id(usize::MAX), Err(SyscallError::InvalidArgument));
    }

    /// Linux's CPU clock IDs: `~id << 3 | thread << 2 | kind`.
    fn cpu_clock(id: u32, thread: bool, kind: i32) -> i32 {
        ((!id as i32) << 3) | ((thread as i32) << 2) | kind
    }

    #[test]
    fn clock_ids_decode_as_linux() {
        assert_eq!(Clock::decode(CLOCK_REALTIME), Ok(Clock::Realtime));
        assert_eq!(Clock::decode(CLOCK_TAI), Ok(Clock::Tai));
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
        let user = times.user_system_ns().0;
        assert_eq!(cpu_clock_ns(times, user, CpuClockKind::Sched), 100);
        assert_eq!(cpu_clock_ns(times, user, CpuClockKind::Prof), 100);
        assert_eq!(cpu_clock_ns(times, user, CpuClockKind::Virt), 50);
    }

    /// itimerval: interval first; EINVAL for a negative second or a
    /// microsecond count outside 0..1000000; back out truncated to
    /// microseconds.
    #[test]
    fn itimerval_conversions_follow_get_itimerval() {
        let tv = |tv_sec, tv_usec| Timeval { tv_sec, tv_usec };
        let s = itimerval_setting([tv(1, 500), tv(2, 999_999)]).unwrap();
        assert_eq!(s.interval, 1_000_500_000);
        assert_eq!(s.value, 2_999_999_000);
        for bad in [tv(-1, 0), tv(0, -1), tv(0, 1_000_000)] {
            assert!(matches!(
                itimerval_setting([tv(0, 0), bad]),
                Err(SyscallError::InvalidArgument)
            ));
            assert!(matches!(
                itimerval_setting([bad, tv(0, 0)]),
                Err(SyscallError::InvalidArgument)
            ));
        }
        let back = setting_itimerval(crate::process::itimer::Setting {
            value: 3_000_001_999,
            interval: 999,
        });
        assert_eq!((back[0].tv_sec, back[0].tv_usec), (0, 0));
        assert_eq!((back[1].tv_sec, back[1].tv_usec), (3, 1));
    }

    /// clock_adjtime adjusts only CLOCK_REALTIME, as Linux: the other
    /// clocks are EOPNOTSUPP, IDs naming no clock EINVAL.
    #[test]
    fn clock_adjtime_targets() {
        assert_eq!(clock_adjtime_target(CLOCK_REALTIME), Ok(()));
        for id in [
            CLOCK_MONOTONIC,
            CLOCK_BOOTTIME,
            CLOCK_REALTIME_ALARM,
            CLOCK_TAI,
            2,
            3,
        ] {
            assert_eq!(
                clock_adjtime_target(id),
                Err(SyscallError::NotSupported),
                "{id}"
            );
        }
        // A CPU clock, of any process.
        assert_eq!(
            clock_adjtime_target(!(12345 << 3) | 2),
            Err(SyscallError::NotSupported)
        );
        // -5 is a posix clock fd (fd 0: `!0 << 3 | 3`).
        for id in [10, 12, 16, 23, 1000, -5] {
            assert_eq!(
                clock_adjtime_target(id),
                Err(SyscallError::InvalidArgument),
                "{id}"
            );
        }
    }
}
