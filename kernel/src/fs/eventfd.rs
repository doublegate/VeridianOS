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
/// If blocking mode, busy-waits with scheduler yield until counter > 0
/// (capped at 30s to prevent permanent hangs).
pub fn eventfd_read(efd_id: u32) -> Result<u64, SyscallError> {
    let start = crate::timer::get_uptime_ms();
    const MAX_BLOCK_MS: u64 = 30_000;

    loop {
        let mut registry = EVENTFD_REGISTRY.lock();
        let instance = registry
            .get_mut(&efd_id)
            .ok_or(SyscallError::BadFileDescriptor)?;

        if instance.counter > 0 {
            return if instance.semaphore {
                instance.counter = instance.counter.saturating_sub(1);
                Ok(1)
            } else {
                let val = instance.counter;
                instance.counter = 0;
                Ok(val)
            };
        }

        if instance.nonblock {
            return Err(SyscallError::WouldBlock);
        }

        // Release lock before yielding
        drop(registry);

        if crate::timer::get_uptime_ms() - start >= MAX_BLOCK_MS {
            return Err(SyscallError::WouldBlock);
        }

        crate::sched::yield_cpu();
    }
}

/// Write to an eventfd. Adds `value` to the internal counter.
///
/// If the addition would overflow `u64::MAX - 1`, returns EAGAIN when
/// nonblock is set, otherwise busy-waits until a read drains the counter
/// enough (capped at 30s).
pub fn eventfd_write(efd_id: u32, value: u64) -> SyscallResult {
    if value == u64::MAX {
        return Err(SyscallError::InvalidArgument);
    }

    let start = crate::timer::get_uptime_ms();
    const MAX_BLOCK_MS: u64 = 30_000;
    let max = u64::MAX - 1;

    loop {
        let mut registry = EVENTFD_REGISTRY.lock();
        let instance = registry
            .get_mut(&efd_id)
            .ok_or(SyscallError::BadFileDescriptor)?;

        if instance.counter <= max - value {
            instance.counter = instance.counter.saturating_add(value);
            return Ok(0);
        }

        if instance.nonblock {
            return Err(SyscallError::WouldBlock);
        }

        drop(registry);

        if crate::timer::get_uptime_ms() - start >= MAX_BLOCK_MS {
            return Err(SyscallError::WouldBlock);
        }

        crate::sched::yield_cpu();
    }
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

// ── VfsNode adapter ────────────────────────────────────────────────────

use alloc::{sync::Arc, vec::Vec};

use super::{DirEntry, Metadata, NodeType, Permissions, VfsNode};
use crate::error::KernelError;

/// VfsNode wrapper around an eventfd instance.
///
/// This allows eventfd to be inserted into a process's file table so that
/// standard read()/write()/close()/epoll work on it. musl's eventfd2()
/// syscall expects a real file descriptor.
pub struct EventFdNode {
    efd_id: u32,
}

impl EventFdNode {
    pub fn new(efd_id: u32) -> Self {
        Self { efd_id }
    }
}

impl VfsNode for EventFdNode {
    fn node_type(&self) -> NodeType {
        NodeType::CharDevice
    }

    fn read(&self, _offset: usize, buffer: &mut [u8]) -> Result<usize, KernelError> {
        if buffer.len() < 8 {
            return Err(KernelError::InvalidArgument {
                name: "buflen",
                value: "must be at least 8 bytes for eventfd",
            });
        }
        let val = eventfd_read(self.efd_id).map_err(|e| match e {
            SyscallError::WouldBlock => KernelError::WouldBlock,
            _ => KernelError::FsError(crate::error::FsError::BadFileDescriptor),
        })?;
        buffer[..8].copy_from_slice(&val.to_le_bytes());
        Ok(8)
    }

    fn write(&self, _offset: usize, data: &[u8]) -> Result<usize, KernelError> {
        if data.len() < 8 {
            return Err(KernelError::InvalidArgument {
                name: "buflen",
                value: "must be at least 8 bytes for eventfd",
            });
        }
        let val =
            u64::from_le_bytes(
                data[..8]
                    .try_into()
                    .map_err(|_| KernelError::InvalidArgument {
                        name: "data",
                        value: "invalid byte slice for u64",
                    })?,
            );
        eventfd_write(self.efd_id, val).map_err(|e| match e {
            SyscallError::WouldBlock => KernelError::WouldBlock,
            SyscallError::InvalidArgument => KernelError::InvalidArgument {
                name: "value",
                value: "u64::MAX is not a valid eventfd value",
            },
            _ => KernelError::FsError(crate::error::FsError::BadFileDescriptor),
        })?;
        Ok(8)
    }

    fn poll_readiness(&self) -> u16 {
        let mut events = 0u16;
        if is_readable(self.efd_id) {
            events |= 0x0001; // POLLIN
        }
        if is_writable(self.efd_id) {
            events |= 0x0004; // POLLOUT
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

    fn truncate(&self, _size: usize) -> Result<(), KernelError> {
        Err(KernelError::PermissionDenied {
            operation: "truncate eventfd",
        })
    }
}

impl Drop for EventFdNode {
    fn drop(&mut self) {
        let _ = eventfd_close(self.efd_id);
    }
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
