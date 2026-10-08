//! timerfd -- Timer notification file descriptor
//!
//! Provides file descriptors that deliver timer expiration notifications,
//! integrable with epoll/poll. Used by Qt6 for frame pacing and event
//! loop timeouts, and by KWin for compositor frame scheduling.
//!
//! ## Syscall Interface
//! - `timerfd_create(clockid, flags) -> fd`     (syscall 331)
//! - `timerfd_settime(fd, flags, new, old) -> 0` (syscall 332)
//! - `timerfd_gettime(fd, curr) -> 0`            (syscall 333)
//! - Read via standard `read(2)` on returned fd
//!
//! ## Semantics
//! - **read**: Returns the number of expirations since last read as a u64, or
//!   EAGAIN if there were none. The read itself never blocks: TFD_NONBLOCK is a
//!   property of the open file, and a blocking read waits in the generic read
//!   path, which sleeps until the next expiry (`VfsNode::ready_at_ns`) or a
//!   change from `timerfd_settime` (N-232).
//! - Timer resolution is based on kernel uptime (TSC-derived).

#![allow(dead_code)]

use alloc::collections::BTreeMap;
use core::sync::atomic::{AtomicU64, Ordering};

use spin::Mutex;

use crate::syscall::{SyscallError, SyscallResult};

/// Maximum number of timerfd instances system-wide.
const MAX_TIMERFD_INSTANCES: usize = 4096;

/// Clock IDs (subset of POSIX clocks).
pub const CLOCK_REALTIME: u32 = 0;
pub const CLOCK_MONOTONIC: u32 = 1;
/// CLOCK_BOOTTIME: monotonic time including suspend; with no suspend it is
/// CLOCK_MONOTONIC.
pub const CLOCK_BOOTTIME: u32 = 7;

/// TFD_NONBLOCK: Return EAGAIN instead of blocking.
pub const TFD_NONBLOCK: u32 = 0x800;
/// TFD_CLOEXEC: Set close-on-exec.
pub const TFD_CLOEXEC: u32 = 0x80000;

/// TFD_TIMER_ABSTIME: Interpret new_value.it_value as absolute time.
pub const TFD_TIMER_ABSTIME: u32 = 1;
/// TFD_TIMER_CANCEL_ON_SET: with a CLOCK_REALTIME absolute timer, end early
/// when the clock is set. Accepted; the clock is never set.
pub const TFD_TIMER_CANCEL_ON_SET: u32 = 2;

/// Time specification matching `struct timespec` layout.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Timespec {
    pub tv_sec: i64,
    pub tv_nsec: i64,
}

// SAFETY: repr(C), two i64 fields, no padding: every bit pattern is a value.
unsafe impl crate::syscall::userspace::UserPod for Timespec {}

impl Timespec {
    pub fn to_ns(&self) -> u64 {
        (self.tv_sec as u64)
            .saturating_mul(1_000_000_000)
            .saturating_add(self.tv_nsec as u64)
    }

    pub fn is_zero(&self) -> bool {
        self.tv_sec == 0 && self.tv_nsec == 0
    }

    /// A time Linux accepts: not negative, nanoseconds below a second.
    pub fn is_valid(&self) -> bool {
        self.tv_sec >= 0 && (0..1_000_000_000).contains(&self.tv_nsec)
    }
}

/// Timer interval specification matching `struct itimerspec`.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct Itimerspec {
    /// Interval for periodic timer (0 = one-shot).
    pub it_interval: Timespec,
    /// Initial expiration time.
    pub it_value: Timespec,
}

// SAFETY: repr(C), two UserPod Timespec fields, no padding.
unsafe impl crate::syscall::userspace::UserPod for Itimerspec {}

/// Internal timerfd state.
struct TimerFdInstance {
    /// Clock type (CLOCK_REALTIME or CLOCK_MONOTONIC).
    clock_id: u32,
    /// Current timer specification.
    spec: Itimerspec,
    /// Absolute expiration time in nanoseconds (monotonic).
    next_expiry_ns: u64,
    /// Number of expirations accumulated since last read.
    expirations: u64,
    /// Whether the timer is armed.
    armed: bool,
    /// Armed with an absolute time on CLOCK_REALTIME: the expiry follows
    /// the wall clock when it is set ([`clock_was_set`]).
    wall_absolute: bool,
    /// With TFD_TIMER_CANCEL_ON_SET: the wall clock's set count when armed;
    /// a read after the clock was set fails once with ECANCELED.
    cancel_on_set: Option<u64>,
}

/// Global registry of timerfd instances.
static TIMERFD_REGISTRY: Mutex<BTreeMap<u32, TimerFdInstance>> = Mutex::new(BTreeMap::new());

/// Next ID for timerfd allocation.
static NEXT_TIMERFD_ID: AtomicU64 = AtomicU64::new(1);

/// Get current monotonic time in nanoseconds from kernel uptime.
fn monotonic_now_ns() -> u64 {
    crate::timer::monotonic_ns()
}

/// Create a new timerfd.
///
/// # Arguments
/// - `clockid`: `CLOCK_REALTIME` or `CLOCK_MONOTONIC`.
/// - `flags`: Combination of `TFD_NONBLOCK`, `TFD_CLOEXEC`.
///
/// # Returns
/// The timerfd ID on success.
pub fn timerfd_create(clockid: u32, flags: u32) -> SyscallResult {
    if !matches!(clockid, CLOCK_REALTIME | CLOCK_MONOTONIC | CLOCK_BOOTTIME)
        || flags & !(TFD_NONBLOCK | TFD_CLOEXEC) != 0
    {
        return Err(SyscallError::InvalidArgument);
    }

    let instance = TimerFdInstance {
        clock_id: clockid,
        spec: Itimerspec::default(),
        next_expiry_ns: 0,
        expirations: 0,
        armed: false,
        wall_absolute: false,
        cancel_on_set: None,
    };

    let id = NEXT_TIMERFD_ID.fetch_add(1, Ordering::Relaxed) as u32;

    let mut registry = TIMERFD_REGISTRY.lock();
    if registry.len() >= MAX_TIMERFD_INSTANCES {
        return Err(SyscallError::OutOfMemory);
    }
    registry.insert(id, instance);
    Ok(id as usize)
}

/// Count the expirations up to `now` (the next expiry of a periodic timer
/// moves on; a one-shot timer disarms).
fn update(instance: &mut TimerFdInstance, now: u64) {
    if !instance.armed || now < instance.next_expiry_ns {
        return;
    }
    let interval_ns = instance.spec.it_interval.to_ns();
    if interval_ns > 0 {
        let periods = 1 + (now - instance.next_expiry_ns) / interval_ns;
        instance.expirations = instance.expirations.saturating_add(periods);
        instance.next_expiry_ns = instance
            .next_expiry_ns
            .saturating_add(periods.saturating_mul(interval_ns));
    } else {
        instance.expirations = instance.expirations.saturating_add(1);
        instance.armed = false;
    }
}

/// The setting as `timerfd_gettime` reports it: the time left until the
/// next expiry, and the interval.
fn current(instance: &TimerFdInstance, now: u64) -> Itimerspec {
    if !instance.armed {
        return Itimerspec {
            it_interval: instance.spec.it_interval,
            ..Default::default()
        };
    }
    let remaining_ns = instance.next_expiry_ns.saturating_sub(now);
    Itimerspec {
        it_interval: instance.spec.it_interval,
        it_value: Timespec {
            tv_sec: (remaining_ns / 1_000_000_000) as i64,
            tv_nsec: (remaining_ns % 1_000_000_000) as i64,
        },
    }
}

/// Arm (or, with a zero `it_value`, disarm) a timerfd: `it_value` relative,
/// or absolute with TFD_TIMER_ABSTIME. The previous setting (as
/// `timerfd_gettime` would have reported it) goes to `old_spec`. EINVAL for
/// unknown flags or an invalid time.
pub fn timerfd_settime(
    tfd_id: u32,
    flags: u32,
    new_spec: &Itimerspec,
    old_spec: Option<&mut Itimerspec>,
) -> SyscallResult {
    if flags & !(TFD_TIMER_ABSTIME | TFD_TIMER_CANCEL_ON_SET) != 0
        || !new_spec.it_value.is_valid()
        || !new_spec.it_interval.is_valid()
    {
        return Err(SyscallError::InvalidArgument);
    }
    let now = monotonic_now_ns();
    let mut registry = TIMERFD_REGISTRY.lock();
    let instance = registry
        .get_mut(&tfd_id)
        .ok_or(SyscallError::BadFileDescriptor)?;
    update(instance, now);
    if let Some(old) = old_spec {
        *old = current(instance, now);
    }

    instance.spec = *new_spec;
    instance.expirations = 0;
    instance.armed = !new_spec.it_value.is_zero();
    let absolute = flags & TFD_TIMER_ABSTIME != 0;
    let on_wall_clock = absolute && instance.clock_id == CLOCK_REALTIME;
    instance.wall_absolute = instance.armed && on_wall_clock;
    instance.cancel_on_set =
        (instance.armed && on_wall_clock && flags & TFD_TIMER_CANCEL_ON_SET != 0)
            .then(crate::timer::realtime::set_count);
    instance.next_expiry_ns = match (instance.armed, absolute) {
        (false, _) => 0,
        // A wall time: the monotonic time the wall clock will read it.
        (true, true) if on_wall_clock => crate::timer::realtime::monotonic_at(
            new_spec.it_value.to_ns().min(i64::MAX as u64) as i64,
        ),
        (true, true) => new_spec.it_value.to_ns(),
        (true, false) => now.saturating_add(new_spec.it_value.to_ns()),
    };
    drop(registry);
    // Waiters sleep until the old expiry: they recompute it.
    io_changed();
    Ok(0)
}

/// The current setting: the time left until the next expiry, and the
/// interval.
pub fn timerfd_gettime(tfd_id: u32) -> Result<Itimerspec, SyscallError> {
    let now = monotonic_now_ns();
    let mut registry = TIMERFD_REGISTRY.lock();
    let instance = registry
        .get_mut(&tfd_id)
        .ok_or(SyscallError::BadFileDescriptor)?;
    update(instance, now);
    Ok(current(instance, now))
}

/// Read a timerfd: the number of expirations since the last read (then 0),
/// or WouldBlock (EAGAIN) if there were none. Never blocks (see the module
/// documentation).
pub fn timerfd_read(tfd_id: u32) -> Result<u64, SyscallError> {
    let now = monotonic_now_ns();
    let mut registry = TIMERFD_REGISTRY.lock();
    let instance = registry
        .get_mut(&tfd_id)
        .ok_or(SyscallError::BadFileDescriptor)?;
    // The wall clock was set under a TFD_TIMER_CANCEL_ON_SET timer: once
    // per change, as Linux's timerfd_canceled.
    let set = crate::timer::realtime::set_count();
    if let Some(armed_at) = instance.cancel_on_set {
        if armed_at != set {
            instance.cancel_on_set = Some(set);
            instance.expirations = 0;
            return Err(SyscallError::Canceled);
        }
    }
    update(instance, now);
    match core::mem::take(&mut instance.expirations) {
        0 => Err(SyscallError::WouldBlock),
        count => Ok(count),
    }
}

/// The wall clock moved by `delta_ns`: timers armed at an absolute wall
/// time keep it (their monotonic expiry moves the other way), and waiters
/// are woken (a TFD_TIMER_CANCEL_ON_SET timer's read now fails).
pub fn clock_was_set(delta_ns: i64) {
    {
        let mut registry = TIMERFD_REGISTRY.lock();
        for instance in registry.values_mut().filter(|i| i.armed && i.wall_absolute) {
            instance.next_expiry_ns = if delta_ns >= 0 {
                instance.next_expiry_ns.saturating_sub(delta_ns as u64)
            } else {
                instance
                    .next_expiry_ns
                    .saturating_add(delta_ns.unsigned_abs())
            };
        }
    }
    io_changed();
}

/// Whether a read would return expirations now.
pub fn is_readable(tfd_id: u32) -> bool {
    let registry = TIMERFD_REGISTRY.lock();
    registry.get(&tfd_id).is_some_and(|i| {
        i.expirations > 0
            || (i.armed && monotonic_now_ns() >= i.next_expiry_ns)
            || i.cancel_on_set
                .is_some_and(|at| at != crate::timer::realtime::set_count())
    })
}

/// When the timer next expires, if it is armed and has no expirations
/// waiting to be read (`VfsNode::ready_at_ns`).
pub fn next_expiry(tfd_id: u32) -> Option<u64> {
    let registry = TIMERFD_REGISTRY.lock();
    registry
        .get(&tfd_id)
        .filter(|i| i.armed && i.expirations == 0)
        .map(|i| i.next_expiry_ns)
}

fn io_changed() {
    #[cfg(feature = "alloc")]
    crate::sched::dispatch::io_event();
}

/// Close (destroy) a timerfd instance.
pub fn timerfd_close(tfd_id: u32) -> SyscallResult {
    let mut registry = TIMERFD_REGISTRY.lock();
    registry
        .remove(&tfd_id)
        .ok_or(SyscallError::BadFileDescriptor)?;
    Ok(0)
}

// ── VfsNode adapter ────────────────────────────────────────────────────

use alloc::{sync::Arc, vec::Vec};

use super::{DirEntry, Metadata, NodeType, Permissions, VfsNode};
use crate::error::KernelError;

/// VfsNode wrapper around a timerfd instance.
///
/// This allows timerfd to be inserted into a process's file table so that
/// standard read()/close()/epoll work on it. musl's timerfd_create()
/// syscall expects a real file descriptor.
pub struct TimerFdNode {
    tfd_id: u32,
}

impl TimerFdNode {
    pub fn new(tfd_id: u32) -> Self {
        Self { tfd_id }
    }

    /// Get the internal timerfd ID (needed for timerfd_settime/gettime).
    pub fn tfd_id(&self) -> u32 {
        self.tfd_id
    }
}

impl VfsNode for TimerFdNode {
    fn node_type(&self) -> NodeType {
        NodeType::CharDevice
    }

    fn is_stream(&self) -> bool {
        true
    }

    fn read(&self, _offset: usize, buffer: &mut [u8]) -> Result<usize, KernelError> {
        if buffer.len() < 8 {
            return Err(KernelError::InvalidArgument {
                name: "buflen",
                value: "must be at least 8 bytes for timerfd",
            });
        }
        let val = timerfd_read(self.tfd_id).map_err(|e| match e {
            SyscallError::WouldBlock => KernelError::WouldBlock,
            // The wall clock was set under a TFD_TIMER_CANCEL_ON_SET timer.
            SyscallError::Canceled => KernelError::FsError(crate::error::FsError::Canceled),
            _ => KernelError::FsError(crate::error::FsError::BadFileDescriptor),
        })?;
        buffer[..8].copy_from_slice(&val.to_le_bytes());
        Ok(8)
    }

    fn write(&self, _offset: usize, _data: &[u8]) -> Result<usize, KernelError> {
        // timerfd is not writable via write(2)
        Err(KernelError::PermissionDenied {
            operation: "write timerfd",
        })
    }

    fn poll_readiness(&self) -> u16 {
        let mut events = 0u16;
        if is_readable(self.tfd_id) {
            events |= 0x0001; // POLLIN
        }
        events
    }

    fn wakes_io_waiters(&self) -> bool {
        true // timerfd_settime calls io_event; expiries are ready_at_ns
    }

    fn ready_at_ns(&self) -> Option<u64> {
        next_expiry(self.tfd_id)
    }

    fn metadata(&self) -> Result<Metadata, KernelError> {
        Ok(Metadata {
            size: 0,
            node_type: NodeType::CharDevice,
            permissions: Permissions::from_mode(0o666),
            uid: 0,
            gid: 0,
            created: 0,
            modified: 0,
            accessed: 0,
            inode: 0,
        })
    }

    fn readdir(&self) -> Result<Vec<DirEntry>, KernelError> {
        Err(KernelError::FsError(crate::error::FsError::NotADirectory))
    }

    fn lookup(&self, _name: &str) -> Result<Arc<dyn VfsNode>, KernelError> {
        Err(KernelError::FsError(crate::error::FsError::NotADirectory))
    }

    fn create(
        &self,
        _name: &str,
        _permissions: Permissions,
    ) -> Result<Arc<dyn VfsNode>, KernelError> {
        Err(KernelError::FsError(crate::error::FsError::NotADirectory))
    }

    fn mkdir(
        &self,
        _name: &str,
        _permissions: Permissions,
    ) -> Result<Arc<dyn VfsNode>, KernelError> {
        Err(KernelError::FsError(crate::error::FsError::NotADirectory))
    }

    fn unlink(&self, _name: &str) -> Result<(), KernelError> {
        Err(KernelError::FsError(crate::error::FsError::NotADirectory))
    }

    fn as_any(&self) -> Option<&dyn core::any::Any> {
        Some(self)
    }

    fn truncate(&self, _size: usize) -> Result<(), KernelError> {
        Err(KernelError::PermissionDenied {
            operation: "truncate timerfd",
        })
    }
}

impl Drop for TimerFdNode {
    fn drop(&mut self) {
        let _ = timerfd_close(self.tfd_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An absolute CLOCK_REALTIME timer keeps its wall time when the clock
    /// is set, and with TFD_TIMER_CANCEL_ON_SET its next read fails with
    /// ECANCELED, once.
    #[test]
    fn wall_clock_timers_follow_clock_changes() {
        let _clock = crate::timer::realtime::TEST_CLOCK_LOCK.lock();
        let id = timerfd_create(CLOCK_REALTIME, 0).unwrap() as u32;
        let wall_now = crate::timer::realtime::now_ns();
        let at = wall_now + 10_000_000_000;
        let spec = Itimerspec {
            it_interval: Timespec::default(),
            it_value: Timespec {
                tv_sec: at / 1_000_000_000,
                tv_nsec: at % 1_000_000_000,
            },
        };
        timerfd_settime(id, TFD_TIMER_ABSTIME | TFD_TIMER_CANCEL_ON_SET, &spec, None).unwrap();
        let before = next_expiry(id).unwrap();
        // The clock jumps 4 s forward: the timer fires 4 s sooner.
        let delta = crate::timer::realtime::set_now_ns(wall_now + 4_000_000_000);
        clock_was_set(delta);
        assert_eq!(next_expiry(id).unwrap(), before - 4_000_000_000);
        assert!(is_readable(id));
        assert_eq!(timerfd_read(id), Err(SyscallError::Canceled));
        assert_eq!(timerfd_read(id), Err(SyscallError::WouldBlock));
        crate::timer::realtime::set_now_ns(wall_now);
        TIMERFD_REGISTRY.lock().remove(&id);
    }

    /// The registry is a process-wide static: tests that reset it must not
    /// run concurrently, or one test's reset removes another's instance.
    static TEST_SERIAL: spin::Mutex<()> = spin::Mutex::new(());

    fn spec(value_s: i64, value_ns: i64, interval_s: i64) -> Itimerspec {
        Itimerspec {
            it_interval: Timespec {
                tv_sec: interval_s,
                tv_nsec: 0,
            },
            it_value: Timespec {
                tv_sec: value_s,
                tv_nsec: value_ns,
            },
        }
    }

    /// N-232 and Linux's rules: invalid times and flags are EINVAL; the
    /// old value is the time that was left; a read before the expiry is
    /// EAGAIN (it never blocks); waiters are told when it expires.
    #[test]
    fn timerfd_settime_follows_linux() {
        let _serial = TEST_SERIAL.lock();
        assert_eq!(
            timerfd_create(CLOCK_MONOTONIC, 0x4),
            Err(SyscallError::InvalidArgument)
        );
        assert!(timerfd_create(CLOCK_BOOTTIME, TFD_CLOEXEC).is_ok());
        let id = timerfd_create(CLOCK_MONOTONIC, 0).unwrap() as u32;

        let bad = |sp: Itimerspec, flags| timerfd_settime(id, flags, &sp, None);
        assert_eq!(
            bad(spec(1, 1_000_000_000, 0), 0),
            Err(SyscallError::InvalidArgument)
        );
        assert_eq!(bad(spec(-1, 0, 0), 0), Err(SyscallError::InvalidArgument));
        assert_eq!(bad(spec(0, -5, 0), 0), Err(SyscallError::InvalidArgument));
        assert_eq!(bad(spec(1, 0, -1), 0), Err(SyscallError::InvalidArgument));
        assert_eq!(bad(spec(1, 0, 0), 0x8), Err(SyscallError::InvalidArgument));

        timerfd_settime(id, 0, &spec(100, 0, 0), None).unwrap();
        assert_eq!(timerfd_read(id), Err(SyscallError::WouldBlock));
        assert!(next_expiry(id).is_some_and(|t| t > monotonic_now_ns()));
        let mut old = Itimerspec::default();
        timerfd_settime(id, 0, &spec(0, 0, 0), Some(&mut old)).unwrap();
        // The time left (host tests: a clock that does not advance).
        assert!(old.it_value.tv_sec >= 98 && old.it_value.tv_sec <= 100);
        assert_eq!(next_expiry(id), None);
    }

    /// Expirations are counted against the clock: a periodic timer counts
    /// every period that passed and moves on; a one-shot timer counts once
    /// and disarms.
    #[test]
    fn expirations_follow_the_clock() {
        let mut t = TimerFdInstance {
            clock_id: CLOCK_MONOTONIC,
            spec: Itimerspec {
                it_interval: Timespec {
                    tv_sec: 0,
                    tv_nsec: 100,
                },
                it_value: Timespec::default(),
            },
            next_expiry_ns: 1000,
            expirations: 0,
            armed: true,
            wall_absolute: false,
            cancel_on_set: None,
        };
        update(&mut t, 999);
        assert_eq!((t.expirations, t.next_expiry_ns), (0, 1000));
        update(&mut t, 1350);
        assert_eq!((t.expirations, t.next_expiry_ns, t.armed), (4, 1400, true));
        assert_eq!(current(&t, 1350).it_value.tv_nsec, 50);
        t.spec.it_interval = Timespec::default();
        update(&mut t, 1400);
        assert_eq!((t.expirations, t.armed), (5, false));
        assert_eq!(current(&t, 2000).it_value, Timespec::default());
    }

    #[test]
    fn test_timerfd_create_monotonic() {
        let _serial = TEST_SERIAL.lock();

        let id = timerfd_create(CLOCK_MONOTONIC, 0).unwrap();
        assert!(id > 0);
    }

    #[test]
    fn test_timerfd_create_invalid_clock() {
        let _serial = TEST_SERIAL.lock();

        assert!(timerfd_create(99, 0).is_err());
    }

    #[test]
    fn test_timerfd_disarm() {
        let _serial = TEST_SERIAL.lock();

        let id = timerfd_create(CLOCK_MONOTONIC, TFD_NONBLOCK).unwrap() as u32;

        // Arm with 1 second
        let spec = Itimerspec {
            it_value: Timespec {
                tv_sec: 1,
                tv_nsec: 0,
            },
            it_interval: Timespec::default(),
        };
        timerfd_settime(id, 0, &spec, None).unwrap();

        // Disarm
        let zero = Itimerspec::default();
        timerfd_settime(id, 0, &zero, None).unwrap();

        // Read should fail (disarmed)
        assert!(timerfd_read(id).is_err());
    }

    #[test]
    fn test_timerfd_gettime_disarmed() {
        let _serial = TEST_SERIAL.lock();

        let id = timerfd_create(CLOCK_MONOTONIC, 0).unwrap() as u32;
        let current = timerfd_gettime(id).unwrap();
        assert!(current.it_value.is_zero());
    }

    #[test]
    fn test_timerfd_close() {
        let _serial = TEST_SERIAL.lock();

        let id = timerfd_create(CLOCK_MONOTONIC, 0).unwrap() as u32;
        timerfd_close(id).unwrap();
        assert!(timerfd_gettime(id).is_err());
    }

    #[test]
    fn test_timespec_to_ns() {
        let ts = Timespec {
            tv_sec: 1,
            tv_nsec: 500_000_000,
        };
        assert_eq!(ts.to_ns(), 1_500_000_000);
    }

    #[test]
    fn test_timespec_zero() {
        let ts = Timespec::default();
        assert!(ts.is_zero());
        assert_eq!(ts.to_ns(), 0);
    }
}
