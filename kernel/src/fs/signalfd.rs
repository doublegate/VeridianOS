//! signalfd -- Signal notification file descriptor
//!
//! Provides a file descriptor for receiving signals via read(2) instead
//! of signal handlers. Integrable with epoll/poll for unified I/O event
//! loops. Used by D-Bus daemon and systemd-compatible service managers.
//!
//! ## Syscall Interface
//! - `signalfd_create(fd, mask_ptr, mask_size, flags) -> fd` (syscall 334)
//! - Read via standard `read(2)` on returned fd
//!
//! ## Semantics
//! - **read**: Returns one or more `SignalfdSiginfo` structs (128 bytes each)
//!   for pending signals in the mask. Signals consumed by signalfd are removed
//!   from the process's pending set.
//! - Creating with fd == -1 creates a new signalfd; passing an existing fd
//!   updates the signal mask.

#![allow(dead_code)]

use alloc::{collections::BTreeMap, vec::Vec};
use core::sync::atomic::{AtomicU64, Ordering};

use spin::Mutex;

use crate::syscall::{SyscallError, SyscallResult};

/// Maximum number of signalfd instances system-wide.
const MAX_SIGNALFD_INSTANCES: usize = 1024;

/// SFD_NONBLOCK: Return EAGAIN instead of blocking on empty read.
pub const SFD_NONBLOCK: u32 = 0x800;
/// SFD_CLOEXEC: Set close-on-exec.
pub const SFD_CLOEXEC: u32 = 0x80000;

/// Maximum signal number we track.
const MAX_SIGNAL: usize = 64;

/// Signal information returned by read(2) on a signalfd.
/// Matches Linux's `struct signalfd_siginfo` layout (128 bytes).
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct SignalfdSiginfo {
    /// Signal number.
    pub ssi_signo: u32,
    /// Error number (unused, 0).
    pub ssi_errno: i32,
    /// Signal code.
    pub ssi_code: i32,
    /// Sending PID.
    pub ssi_pid: u32,
    /// Sending UID.
    pub ssi_uid: u32,
    /// File descriptor (for SIGIO).
    pub ssi_fd: i32,
    /// Kernel timer ID.
    pub ssi_tid: u32,
    /// Band event (for SIGIO).
    pub ssi_band: u32,
    /// POSIX timer overrun count.
    pub ssi_overrun: u32,
    /// Trap number.
    pub ssi_trapno: u32,
    /// Exit status or signal (for SIGCHLD).
    pub ssi_status: i32,
    /// Integer sent by sigqueue.
    pub ssi_int: i32,
    /// Pointer sent by sigqueue.
    pub ssi_ptr: u64,
    /// User CPU time consumed (for SIGCHLD).
    pub ssi_utime: u64,
    /// System CPU time consumed (for SIGCHLD).
    pub ssi_stime: u64,
    /// Address that generated signal (for hardware signals).
    pub ssi_addr: u64,
    /// Address LSB (for SIGBUS).
    pub ssi_addr_lsb: u16,
    /// Padding to 128 bytes.
    _pad: [u8; 46],
}

impl Default for SignalfdSiginfo {
    fn default() -> Self {
        // SAFETY: SignalfdSiginfo is repr(C) and all-zeros is valid.
        unsafe { core::mem::zeroed() }
    }
}

/// Signal mask -- a bitmask of signal numbers to monitor.
#[derive(Debug, Clone, Default)]
pub struct SigSet {
    bits: u64,
}

impl SigSet {
    pub fn new() -> Self {
        Self { bits: 0 }
    }

    pub fn is_set(&self, signum: u32) -> bool {
        if signum == 0 || signum as usize > MAX_SIGNAL {
            return false;
        }
        (self.bits & (1u64 << (signum - 1))) != 0
    }

    pub fn set(&mut self, signum: u32) {
        if signum > 0 && (signum as usize) <= MAX_SIGNAL {
            self.bits |= 1u64 << (signum - 1);
        }
    }

    pub fn from_raw(bits: u64) -> Self {
        Self { bits }
    }

    pub fn raw(&self) -> u64 {
        self.bits
    }
}

/// Pending signal queued for delivery via signalfd.
#[derive(Debug, Clone)]
struct PendingSignal {
    signum: u32,
    sender_pid: u32,
}

/// Internal signalfd instance.
struct SignalFdInstance {
    /// Signal mask -- which signals to accept.
    mask: SigSet,
    /// Whether non-blocking mode is active.
    nonblock: bool,
    /// Queue of pending signals.
    pending: Vec<PendingSignal>,
    /// Owner process ID.
    owner_pid: u64,
}

/// Global registry of signalfd instances.
static SIGNALFD_REGISTRY: Mutex<BTreeMap<u32, SignalFdInstance>> = Mutex::new(BTreeMap::new());

/// Next ID for signalfd allocation.
static NEXT_SIGNALFD_ID: AtomicU64 = AtomicU64::new(1);

/// Create or update a signalfd.
///
/// # Arguments
/// - `fd`: -1 to create new, or existing signalfd ID to update mask.
/// - `mask`: Signal mask (bitmask of signals to monitor).
/// - `flags`: Combination of `SFD_NONBLOCK`, `SFD_CLOEXEC`.
///
/// # Returns
/// The signalfd ID on success.
pub fn signalfd_create(fd: i32, mask: u64, flags: u32) -> SyscallResult {
    let pid = crate::process::current_process()
        .map(|p| p.pid.0)
        .unwrap_or(0);

    if fd != -1 {
        // Update existing signalfd mask
        let sfd_id = fd as u32;
        let mut registry = SIGNALFD_REGISTRY.lock();
        let instance = registry
            .get_mut(&sfd_id)
            .ok_or(SyscallError::BadFileDescriptor)?;
        instance.mask = SigSet::from_raw(mask);
        return Ok(sfd_id as usize);
    }

    // Create new
    let nonblock = (flags & SFD_NONBLOCK) != 0;

    let instance = SignalFdInstance {
        mask: SigSet::from_raw(mask),
        nonblock,
        pending: Vec::new(),
        owner_pid: pid,
    };

    let id = NEXT_SIGNALFD_ID.fetch_add(1, Ordering::Relaxed) as u32;

    let mut registry = SIGNALFD_REGISTRY.lock();
    if registry.len() >= MAX_SIGNALFD_INSTANCES {
        return Err(SyscallError::OutOfMemory);
    }
    registry.insert(id, instance);
    Ok(id as usize)
}

/// Deliver a signal to all signalfds of a given process that have
/// the signal in their mask.
///
/// Called from the kernel's signal delivery path.
pub fn deliver_signal(target_pid: u64, signum: u32, sender_pid: u32) {
    let mut registry = SIGNALFD_REGISTRY.lock();
    for instance in registry.values_mut() {
        if instance.owner_pid == target_pid && instance.mask.is_set(signum) {
            instance.pending.push(PendingSignal { signum, sender_pid });
        }
    }
}

/// Read one pending signal from a signalfd.
///
/// Returns a `SignalfdSiginfo` struct for the oldest pending signal
/// in the mask, or EAGAIN/WouldBlock if no signals pending.
pub fn signalfd_read(sfd_id: u32) -> Result<SignalfdSiginfo, SyscallError> {
    let start = crate::timer::get_uptime_ms();
    const MAX_BLOCK_MS: u64 = 30_000;

    loop {
        let mut registry = SIGNALFD_REGISTRY.lock();
        let instance = registry
            .get_mut(&sfd_id)
            .ok_or(SyscallError::BadFileDescriptor)?;

        if !instance.pending.is_empty() {
            let sig = instance.pending.remove(0);
            return Ok(SignalfdSiginfo {
                ssi_signo: sig.signum,
                ssi_pid: sig.sender_pid,
                ..SignalfdSiginfo::default()
            });
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

/// Query whether a signalfd has pending signals.
/// Used by epoll to check readiness without consuming data.
pub fn is_readable(sfd_id: u32) -> bool {
    let registry = SIGNALFD_REGISTRY.lock();
    registry.get(&sfd_id).is_some_and(|i| !i.pending.is_empty())
}

/// Close (destroy) a signalfd instance.
pub fn signalfd_close(sfd_id: u32) -> SyscallResult {
    let mut registry = SIGNALFD_REGISTRY.lock();
    registry
        .remove(&sfd_id)
        .ok_or(SyscallError::BadFileDescriptor)?;
    Ok(0)
}

// ── VfsNode adapter ────────────────────────────────────────────────────

use alloc::sync::Arc;

use super::{DirEntry, Metadata, NodeType, Permissions, VfsNode};
use crate::error::KernelError;

/// VfsNode wrapper around a signalfd instance.
///
/// This allows signalfd to be inserted into a process's file table so that
/// standard read()/close()/epoll work on it. musl's signalfd4() syscall
/// expects a real file descriptor.
pub struct SignalFdNode {
    sfd_id: u32,
}

impl SignalFdNode {
    pub fn new(sfd_id: u32) -> Self {
        Self { sfd_id }
    }

    /// Get the internal signalfd ID (needed for mask updates).
    pub fn sfd_id(&self) -> u32 {
        self.sfd_id
    }
}

impl VfsNode for SignalFdNode {
    fn node_type(&self) -> NodeType {
        NodeType::CharDevice
    }

    fn read(&self, _offset: usize, buffer: &mut [u8]) -> Result<usize, KernelError> {
        if buffer.len() < 128 {
            return Err(KernelError::InvalidArgument {
                name: "buflen",
                value: "must be at least 128 bytes for signalfd",
            });
        }
        let info = signalfd_read(self.sfd_id).map_err(|e| match e {
            SyscallError::WouldBlock => KernelError::WouldBlock,
            _ => KernelError::FsError(crate::error::FsError::BadFileDescriptor),
        })?;
        // SAFETY: SignalfdSiginfo is repr(C) and exactly 128 bytes.
        let bytes = unsafe {
            core::slice::from_raw_parts(&info as *const SignalfdSiginfo as *const u8, 128)
        };
        buffer[..128].copy_from_slice(bytes);
        Ok(128)
    }

    fn write(&self, _offset: usize, _data: &[u8]) -> Result<usize, KernelError> {
        // signalfd is not writable via write(2)
        Err(KernelError::PermissionDenied {
            operation: "write signalfd",
        })
    }

    fn poll_readiness(&self) -> u16 {
        let mut events = 0u16;
        if is_readable(self.sfd_id) {
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

    fn truncate(&self, _size: usize) -> Result<(), KernelError> {
        Err(KernelError::PermissionDenied {
            operation: "truncate signalfd",
        })
    }
}

impl Drop for SignalFdNode {
    fn drop(&mut self) {
        let _ = signalfd_close(self.sfd_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sigset_basic() {
        let mut s = SigSet::new();
        assert!(!s.is_set(1));
        s.set(1); // SIGHUP
        assert!(s.is_set(1));
        assert!(!s.is_set(2));
    }

    #[test]
    fn test_sigset_from_raw() {
        let s = SigSet::from_raw(0b110); // signals 2 and 3
        assert!(!s.is_set(1));
        assert!(s.is_set(2));
        assert!(s.is_set(3));
        assert!(!s.is_set(4));
    }

    #[test]
    fn test_sigset_boundary() {
        let mut s = SigSet::new();
        s.set(0); // invalid -- should be no-op
        assert!(!s.is_set(0));
        s.set(64);
        assert!(s.is_set(64));
        s.set(65); // out of range -- no-op
        assert!(!s.is_set(65));
    }

    #[test]
    fn test_signalfd_create_and_close() {
        SIGNALFD_REGISTRY.lock().clear();

        let id = signalfd_create(-1, 0b10, SFD_NONBLOCK).unwrap() as u32;
        signalfd_close(id).unwrap();
        assert!(signalfd_read(id).is_err());
    }

    #[test]
    fn test_signalfd_update_mask() {
        SIGNALFD_REGISTRY.lock().clear();

        let id = signalfd_create(-1, 0b10, 0).unwrap();
        // Update mask to include signal 3
        signalfd_create(id as i32, 0b110, 0).unwrap();

        // Verify via deliver + read
        deliver_signal(0, 3, 42);
        let info = signalfd_read(id as u32).unwrap();
        assert_eq!(info.ssi_signo, 3);
        assert_eq!(info.ssi_pid, 42);
    }

    #[test]
    fn test_signalfd_deliver_and_read() {
        SIGNALFD_REGISTRY.lock().clear();

        // Mask signal 2 (SIGINT)
        let id = signalfd_create(-1, 0b10, 0).unwrap() as u32;

        // No signals yet
        assert!(signalfd_read(id).is_err());

        // Deliver signal 2 to pid 0
        deliver_signal(0, 2, 100);

        let info = signalfd_read(id).unwrap();
        assert_eq!(info.ssi_signo, 2);
        assert_eq!(info.ssi_pid, 100);

        // Queue is now empty
        assert!(signalfd_read(id).is_err());
    }

    #[test]
    fn test_signalfd_ignores_unmasked() {
        SIGNALFD_REGISTRY.lock().clear();

        // Only mask signal 1
        let id = signalfd_create(-1, 0b1, SFD_NONBLOCK).unwrap() as u32;

        // Deliver signal 2 (not in mask)
        deliver_signal(0, 2, 50);

        // Should have nothing
        assert!(signalfd_read(id).is_err());
    }

    #[test]
    fn test_signalfd_siginfo_size() {
        assert_eq!(core::mem::size_of::<SignalfdSiginfo>(), 128);
    }
}
