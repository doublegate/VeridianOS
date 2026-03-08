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
//! - **read**: Returns the number of expirations since last read as a u64.
//!   Returns EAGAIN if no expirations and non-blocking, otherwise blocks.
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

/// TFD_NONBLOCK: Return EAGAIN instead of blocking.
pub const TFD_NONBLOCK: u32 = 0x800;
/// TFD_CLOEXEC: Set close-on-exec.
pub const TFD_CLOEXEC: u32 = 0x80000;

/// TFD_TIMER_ABSTIME: Interpret new_value.it_value as absolute time.
pub const TFD_TIMER_ABSTIME: u32 = 1;

/// Time specification matching `struct timespec` layout.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Timespec {
    pub tv_sec: i64,
    pub tv_nsec: i64,
}

impl Timespec {
    pub fn to_ns(&self) -> u64 {
        (self.tv_sec as u64)
            .saturating_mul(1_000_000_000)
            .saturating_add(self.tv_nsec as u64)
    }

    pub fn is_zero(&self) -> bool {
        self.tv_sec == 0 && self.tv_nsec == 0
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

/// Internal timerfd state.
struct TimerFdInstance {
    /// Clock type (CLOCK_REALTIME or CLOCK_MONOTONIC).
    clock_id: u32,
    /// Whether non-blocking mode is active.
    nonblock: bool,
    /// Current timer specification.
    spec: Itimerspec,
    /// Absolute expiration time in nanoseconds (monotonic).
    next_expiry_ns: u64,
    /// Number of expirations accumulated since last read.
    expirations: u64,
    /// Whether the timer is armed.
    armed: bool,
    /// Owner process ID.
    owner_pid: u64,
}

/// Global registry of timerfd instances.
static TIMERFD_REGISTRY: Mutex<BTreeMap<u32, TimerFdInstance>> = Mutex::new(BTreeMap::new());

/// Next ID for timerfd allocation.
static NEXT_TIMERFD_ID: AtomicU64 = AtomicU64::new(1);

/// Get current monotonic time in nanoseconds from kernel uptime.
fn monotonic_now_ns() -> u64 {
    // Use the kernel's uptime counter (TSC-based on x86_64)
    let uptime_ms = crate::timer::get_uptime_ms();
    uptime_ms.saturating_mul(1_000_000)
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
    if clockid != CLOCK_REALTIME && clockid != CLOCK_MONOTONIC {
        return Err(SyscallError::InvalidArgument);
    }

    let pid = crate::process::current_process()
        .map(|p| p.pid.0)
        .unwrap_or(0);

    let nonblock = (flags & TFD_NONBLOCK) != 0;

    let instance = TimerFdInstance {
        clock_id: clockid,
        nonblock,
        spec: Itimerspec::default(),
        next_expiry_ns: 0,
        expirations: 0,
        armed: false,
        owner_pid: pid,
    };

    let id = NEXT_TIMERFD_ID.fetch_add(1, Ordering::Relaxed) as u32;

    let mut registry = TIMERFD_REGISTRY.lock();
    if registry.len() >= MAX_TIMERFD_INSTANCES {
        return Err(SyscallError::OutOfMemory);
    }
    registry.insert(id, instance);
    Ok(id as usize)
}

/// Arm or disarm a timerfd.
///
/// # Arguments
/// - `tfd_id`: Timer fd ID.
/// - `flags`: `TFD_TIMER_ABSTIME` for absolute time.
/// - `new_spec`: New timer specification.
///
/// # Returns
/// The previous timer specification via `old_spec` (if non-null).
pub fn timerfd_settime(
    tfd_id: u32,
    flags: u32,
    new_spec: &Itimerspec,
    old_spec: Option<&mut Itimerspec>,
) -> SyscallResult {
    let mut registry = TIMERFD_REGISTRY.lock();
    let instance = registry
        .get_mut(&tfd_id)
        .ok_or(SyscallError::BadFileDescriptor)?;

    // Return old value if requested
    if let Some(old) = old_spec {
        *old = instance.spec;
    }

    instance.spec = *new_spec;
    instance.expirations = 0;

    if new_spec.it_value.is_zero() {
        // Disarm the timer
        instance.armed = false;
        instance.next_expiry_ns = 0;
    } else {
        instance.armed = true;
        let now = monotonic_now_ns();

        if (flags & TFD_TIMER_ABSTIME) != 0 {
            // Absolute time
            instance.next_expiry_ns = new_spec.it_value.to_ns();
        } else {
            // Relative time
            instance.next_expiry_ns = now.saturating_add(new_spec.it_value.to_ns());
        }
    }

    Ok(0)
}

/// Get the current timer specification.
pub fn timerfd_gettime(tfd_id: u32) -> Result<Itimerspec, SyscallError> {
    let registry = TIMERFD_REGISTRY.lock();
    let instance = registry
        .get(&tfd_id)
        .ok_or(SyscallError::BadFileDescriptor)?;

    if !instance.armed {
        return Ok(Itimerspec::default());
    }

    let now = monotonic_now_ns();
    let remaining_ns = instance.next_expiry_ns.saturating_sub(now);

    Ok(Itimerspec {
        it_interval: instance.spec.it_interval,
        it_value: Timespec {
            tv_sec: (remaining_ns / 1_000_000_000) as i64,
            tv_nsec: (remaining_ns % 1_000_000_000) as i64,
        },
    })
}

/// Read from a timerfd -- returns number of expirations since last read.
///
/// Checks the timer against current time and accumulates expirations.
/// If nonblock is set, returns EAGAIN immediately when no expirations.
/// In blocking mode, busy-waits with scheduler yield until the timer
/// fires (capped at 30s to prevent permanent hangs).
pub fn timerfd_read(tfd_id: u32) -> Result<u64, SyscallError> {
    let start = crate::timer::get_uptime_ms();
    const MAX_BLOCK_MS: u64 = 30_000;

    loop {
        let mut registry = TIMERFD_REGISTRY.lock();
        let instance = registry
            .get_mut(&tfd_id)
            .ok_or(SyscallError::BadFileDescriptor)?;

        if !instance.armed {
            if instance.nonblock {
                return Err(SyscallError::WouldBlock);
            }
            // Timer not armed and blocking -- wait for it to be armed
            drop(registry);
            if crate::timer::get_uptime_ms() - start >= MAX_BLOCK_MS {
                return Err(SyscallError::WouldBlock);
            }
            crate::sched::yield_cpu();
            continue;
        }

        // Check for expirations against current TSC-based time
        let now = monotonic_now_ns();
        if now >= instance.next_expiry_ns {
            let interval_ns = instance.spec.it_interval.to_ns();
            if interval_ns > 0 {
                let elapsed = now - instance.next_expiry_ns;
                let extra_expirations = elapsed / interval_ns;
                instance.expirations = instance.expirations.saturating_add(1 + extra_expirations);
                instance.next_expiry_ns = instance
                    .next_expiry_ns
                    .saturating_add((1 + extra_expirations) * interval_ns);
            } else {
                instance.expirations = instance.expirations.saturating_add(1);
                instance.armed = false;
            }
        }

        if instance.expirations > 0 {
            let count = instance.expirations;
            instance.expirations = 0;
            return Ok(count);
        }

        if instance.nonblock {
            return Err(SyscallError::WouldBlock);
        }

        // Release lock, yield, and retry
        drop(registry);
        if crate::timer::get_uptime_ms() - start >= MAX_BLOCK_MS {
            return Err(SyscallError::WouldBlock);
        }
        crate::sched::yield_cpu();
    }
}

/// Query whether a timerfd is readable (timer has expired).
/// Used by epoll to check readiness without consuming data.
pub fn is_readable(tfd_id: u32) -> bool {
    let registry = TIMERFD_REGISTRY.lock();
    let instance = match registry.get(&tfd_id) {
        Some(i) => i,
        None => return false,
    };
    if !instance.armed {
        return false;
    }
    let now = monotonic_now_ns();
    now >= instance.next_expiry_ns || instance.expirations > 0
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

    fn read(&self, _offset: usize, buffer: &mut [u8]) -> Result<usize, KernelError> {
        if buffer.len() < 8 {
            return Err(KernelError::InvalidArgument {
                name: "buflen",
                value: "must be at least 8 bytes for timerfd",
            });
        }
        let val = timerfd_read(self.tfd_id).map_err(|e| match e {
            SyscallError::WouldBlock => KernelError::WouldBlock,
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

    #[test]
    fn test_timerfd_create_monotonic() {
        TIMERFD_REGISTRY.lock().clear();

        let id = timerfd_create(CLOCK_MONOTONIC, 0).unwrap();
        assert!(id > 0);
    }

    #[test]
    fn test_timerfd_create_invalid_clock() {
        TIMERFD_REGISTRY.lock().clear();

        assert!(timerfd_create(99, 0).is_err());
    }

    #[test]
    fn test_timerfd_disarm() {
        TIMERFD_REGISTRY.lock().clear();

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
        TIMERFD_REGISTRY.lock().clear();

        let id = timerfd_create(CLOCK_MONOTONIC, 0).unwrap() as u32;
        let current = timerfd_gettime(id).unwrap();
        assert!(current.it_value.is_zero());
    }

    #[test]
    fn test_timerfd_close() {
        TIMERFD_REGISTRY.lock().clear();

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
