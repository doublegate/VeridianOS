//! eventfd -- Event notification file descriptor
//!
//! Provides a file descriptor for event wait/notify, commonly used by
//! epoll-based event loops (Qt6, glib). Supports both counter and
//! semaphore semantics.
//!
//! ## Syscall Interface
//! - `eventfd_create(initval, flags) -> fd`  (syscall 330)
//! - Read/write via standard `read(2)` / `write(2)` on returned fd
//!
//! ## Semantics
//! - **write**: Adds the 8-byte unsigned integer to the internal counter.
//!   Blocks (or returns EAGAIN) if the counter would overflow `u64::MAX - 1`.
//! - **read**: Returns the current counter as an 8-byte unsigned integer and
//!   resets it to zero. In semaphore mode, returns 1 and decrements by 1.
//!   Blocks (or returns EAGAIN) if the counter is zero.

#![allow(dead_code)]

use alloc::collections::BTreeMap;
use core::sync::atomic::{AtomicU64, Ordering};

use spin::Mutex;

use crate::syscall::{SyscallError, SyscallResult};

/// Maximum number of eventfd instances system-wide.
const MAX_EVENTFD_INSTANCES: usize = 4096;

/// EFD_SEMAPHORE flag: read returns 1 and decrements (instead of draining).
pub const EFD_SEMAPHORE: u32 = 1;
/// EFD_NONBLOCK flag: reads/writes return EAGAIN instead of blocking.
pub const EFD_NONBLOCK: u32 = 0x800;
/// EFD_CLOEXEC flag: set close-on-exec (tracked but not enforced in kernel).
pub const EFD_CLOEXEC: u32 = 0x80000;

/// Internal eventfd instance.
struct EventFdInstance {
    /// Current counter value.
    counter: u64,
    /// Whether semaphore mode is active.
    semaphore: bool,
    /// Whether non-blocking mode is active.
    nonblock: bool,
    /// Owner process ID.
    owner_pid: u64,
}

/// Global registry of eventfd instances, keyed by a monotonic ID.
static EVENTFD_REGISTRY: Mutex<BTreeMap<u32, EventFdInstance>> = Mutex::new(BTreeMap::new());

/// Next ID for eventfd allocation.
static NEXT_EVENTFD_ID: AtomicU64 = AtomicU64::new(1);

/// Create a new eventfd.
///
/// # Arguments
/// - `initval`: Initial counter value.
/// - `flags`: Combination of `EFD_SEMAPHORE`, `EFD_NONBLOCK`, `EFD_CLOEXEC`.
///
/// # Returns
/// The eventfd ID (used as a pseudo-fd) on success.
pub fn eventfd_create(initval: u32, flags: u32) -> SyscallResult {
    let pid = crate::process::current_process()
        .map(|p| p.pid.0)
        .unwrap_or(0);

    let semaphore = (flags & EFD_SEMAPHORE) != 0;
    let nonblock = (flags & EFD_NONBLOCK) != 0;

    let instance = EventFdInstance {
        counter: initval as u64,
        semaphore,
        nonblock,
        owner_pid: pid,
    };

    let id = NEXT_EVENTFD_ID.fetch_add(1, Ordering::Relaxed) as u32;

    let mut registry = EVENTFD_REGISTRY.lock();
    if registry.len() >= MAX_EVENTFD_INSTANCES {
        return Err(SyscallError::OutOfMemory);
    }
    registry.insert(id, instance);
    Ok(id as usize)
}

/// Read from an eventfd. Returns the counter value as a u64.
///
/// In normal mode: returns the full counter and resets to 0.
/// In semaphore mode: returns 1 and decrements by 1.
/// If counter is 0 and nonblock is set, returns EAGAIN (WouldBlock).
pub fn eventfd_read(efd_id: u32) -> Result<u64, SyscallError> {
    let mut registry = EVENTFD_REGISTRY.lock();
    let instance = registry
        .get_mut(&efd_id)
        .ok_or(SyscallError::BadFileDescriptor)?;

    if instance.counter == 0 {
        if instance.nonblock {
            return Err(SyscallError::WouldBlock);
        }
        // In a real implementation we would block the calling thread here
        // and wake it when a write occurs. For now, return WouldBlock.
        return Err(SyscallError::WouldBlock);
    }

    if instance.semaphore {
        instance.counter = instance.counter.saturating_sub(1);
        Ok(1)
    } else {
        let val = instance.counter;
        instance.counter = 0;
        Ok(val)
    }
}

/// Write to an eventfd. Adds `value` to the internal counter.
///
/// If the addition would overflow `u64::MAX - 1`, returns EAGAIN when
/// nonblock is set, otherwise blocks (simplified to EAGAIN for now).
pub fn eventfd_write(efd_id: u32, value: u64) -> SyscallResult {
    if value == u64::MAX {
        return Err(SyscallError::InvalidArgument);
    }

    let mut registry = EVENTFD_REGISTRY.lock();
    let instance = registry
        .get_mut(&efd_id)
        .ok_or(SyscallError::BadFileDescriptor)?;

    // Check for overflow (Linux caps at u64::MAX - 1)
    let max = u64::MAX - 1;
    if instance.counter > max - value {
        if instance.nonblock {
            return Err(SyscallError::WouldBlock);
        }
        return Err(SyscallError::WouldBlock);
    }

    instance.counter = instance.counter.saturating_add(value);
    Ok(0)
}

/// Query whether an eventfd is readable (counter > 0).
/// Used by epoll to check readiness without consuming data.
pub fn is_readable(efd_id: u32) -> bool {
    let registry = EVENTFD_REGISTRY.lock();
    registry.get(&efd_id).is_some_and(|i| i.counter > 0)
}

/// Query whether an eventfd is writable (counter < u64::MAX - 1).
pub fn is_writable(efd_id: u32) -> bool {
    let registry = EVENTFD_REGISTRY.lock();
    registry
        .get(&efd_id)
        .is_some_and(|i| i.counter < u64::MAX - 1)
}

/// Close (destroy) an eventfd instance.
pub fn eventfd_close(efd_id: u32) -> SyscallResult {
    let mut registry = EVENTFD_REGISTRY.lock();
    registry
        .remove(&efd_id)
        .ok_or(SyscallError::BadFileDescriptor)?;
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_eventfd_create_and_read() {
        // Reset state
        EVENTFD_REGISTRY.lock().clear();

        let id = eventfd_create(42, 0).unwrap() as u32;
        let val = eventfd_read(id).unwrap();
        assert_eq!(val, 42);

        // Counter should be 0 after read
        assert!(eventfd_read(id).is_err());
    }

    #[test]
    fn test_eventfd_semaphore_mode() {
        EVENTFD_REGISTRY.lock().clear();

        let id = eventfd_create(3, EFD_SEMAPHORE).unwrap() as u32;

        // Each read returns 1 and decrements
        assert_eq!(eventfd_read(id).unwrap(), 1);
        assert_eq!(eventfd_read(id).unwrap(), 1);
        assert_eq!(eventfd_read(id).unwrap(), 1);

        // Now counter is 0
        assert!(eventfd_read(id).is_err());
    }

    #[test]
    fn test_eventfd_write_accumulates() {
        EVENTFD_REGISTRY.lock().clear();

        let id = eventfd_create(0, 0).unwrap() as u32;
        eventfd_write(id, 10).unwrap();
        eventfd_write(id, 20).unwrap();

        let val = eventfd_read(id).unwrap();
        assert_eq!(val, 30);
    }

    #[test]
    fn test_eventfd_close() {
        EVENTFD_REGISTRY.lock().clear();

        let id = eventfd_create(0, 0).unwrap() as u32;
        eventfd_close(id).unwrap();

        // Should fail after close
        assert!(eventfd_read(id).is_err());
        assert!(eventfd_close(id).is_err());
    }

    #[test]
    fn test_eventfd_nonblock_on_empty() {
        EVENTFD_REGISTRY.lock().clear();

        let id = eventfd_create(0, EFD_NONBLOCK).unwrap() as u32;
        match eventfd_read(id) {
            Err(SyscallError::WouldBlock) => {} // expected
            other => panic!("Expected WouldBlock, got {:?}", other),
        }
    }

    #[test]
    fn test_eventfd_write_overflow() {
        EVENTFD_REGISTRY.lock().clear();

        let id = eventfd_create(0, EFD_NONBLOCK).unwrap() as u32;
        // Write near max
        eventfd_write(id, u64::MAX - 2).unwrap();
        // This should fail (would overflow past MAX-1)
        assert!(eventfd_write(id, 2).is_err());
    }

    #[test]
    fn test_eventfd_write_max_rejected() {
        EVENTFD_REGISTRY.lock().clear();

        let id = eventfd_create(0, 0).unwrap() as u32;
        assert!(eventfd_write(id, u64::MAX).is_err());
    }
}
