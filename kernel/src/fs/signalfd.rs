//! signalfd -- signals read from a file descriptor (N-233).
//!
//! A signalfd has a signal mask. Reading it takes signals pending for the
//! reading thread -- its own and its process's -- that are in the mask, and
//! returns one `struct signalfd_siginfo` (128 bytes) per signal, as many as
//! the buffer holds; those signals are consumed, not delivered. Programs
//! block the signals they read this way (D-Bus, systemd-style services, Qt's
//! Unix signal handling); a signal that is not blocked may be delivered
//! first. As on Linux, the pending set is the reader's, so a signalfd
//! inherited across fork reads the child's signals.
//!
//! Reads never block here: SFD_NONBLOCK is a property of the open file, and
//! a blocking read waits in the generic read path. Every signal generated
//! wakes I/O waiters while a signalfd exists ([`signal_generated`]), so
//! read, poll and epoll sleep until one arrives.

use alloc::{sync::Arc, vec::Vec};
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use super::{DirEntry, Metadata, NodeType, Permissions, VfsNode};
use crate::error::KernelError;

/// SFD_NONBLOCK: Return EAGAIN instead of blocking on empty read.
pub const SFD_NONBLOCK: u32 = 0x800;
/// SFD_CLOEXEC: Set close-on-exec.
pub const SFD_CLOEXEC: u32 = 0x80000;

/// Linux's `struct signalfd_siginfo` (128 bytes).
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct SignalfdSiginfo {
    /// Signal number.
    pub ssi_signo: u32,
    /// Error number (unused, 0).
    pub ssi_errno: i32,
    /// Signal code (SI_USER, 0: the kernel records no sender yet).
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

const _: () = assert!(core::mem::size_of::<SignalfdSiginfo>() == 128);

impl SignalfdSiginfo {
    fn for_signal(signo: usize) -> Self {
        // SAFETY: all-zero bytes are a valid value of this repr(C) struct of
        // integers.
        let mut info: Self = unsafe { core::mem::zeroed() };
        info.ssi_signo = signo as u32;
        info
    }

    fn as_bytes(&self) -> &[u8; 128] {
        // SAFETY: the struct is repr(C), exactly 128 bytes (checked above)
        // and has no padding with undefined contents (it was zeroed).
        unsafe { &*(self as *const Self as *const [u8; 128]) }
    }
}

/// Live signalfds: only while there is one does generating a signal wake
/// I/O waiters.
static SIGNALFDS: AtomicUsize = AtomicUsize::new(0);

/// A signal was made pending: wake I/O waiters, so a read, poll or epoll
/// on a signalfd sees it. Free when no signalfd exists.
pub fn signal_generated() {
    if SIGNALFDS.load(Ordering::Acquire) > 0 {
        #[cfg(feature = "alloc")]
        crate::sched::dispatch::io_event();
    }
}

/// The mask a signalfd keeps: SIGKILL and SIGSTOP are silently left out,
/// as Linux does.
pub fn sanitize_mask(mask: u64) -> u64 {
    mask & !crate::process::signals::UNBLOCKABLE
}

/// A signalfd.
pub struct SignalFdNode {
    mask: AtomicU64,
}

impl SignalFdNode {
    pub fn new(mask: u64) -> Self {
        SIGNALFDS.fetch_add(1, Ordering::AcqRel);
        Self {
            mask: AtomicU64::new(sanitize_mask(mask)),
        }
    }

    /// Replace the mask (signalfd4 on an existing signalfd).
    pub fn set_mask(&self, mask: u64) {
        self.mask.store(sanitize_mask(mask), Ordering::Release);
        signal_generated();
    }

    fn mask(&self) -> u64 {
        self.mask.load(Ordering::Acquire)
    }
}

impl Drop for SignalFdNode {
    fn drop(&mut self) {
        SIGNALFDS.fetch_sub(1, Ordering::AcqRel);
    }
}

impl VfsNode for SignalFdNode {
    fn node_type(&self) -> NodeType {
        NodeType::CharDevice
    }

    fn is_stream(&self) -> bool {
        true
    }

    fn read(&self, _offset: usize, buffer: &mut [u8]) -> Result<usize, KernelError> {
        if buffer.len() < 128 {
            return Err(KernelError::InvalidArgument {
                name: "buflen",
                value: "must be at least 128 bytes for signalfd",
            });
        }
        let mask = self.mask();
        let mut done = 0;
        for chunk in buffer.chunks_exact_mut(128) {
            let Some(signo) = crate::process::signals::take_pending_in(mask) else {
                break;
            };
            chunk.copy_from_slice(SignalfdSiginfo::for_signal(signo).as_bytes());
            done += 128;
        }
        if done == 0 {
            return Err(KernelError::WouldBlock);
        }
        Ok(done)
    }

    fn write(&self, _offset: usize, _data: &[u8]) -> Result<usize, KernelError> {
        // signalfd is not writable via write(2)
        Err(KernelError::PermissionDenied {
            operation: "write signalfd",
        })
    }

    fn poll_readiness(&self) -> u16 {
        if crate::process::signals::pending_in(self.mask()) {
            0x0001 // POLLIN
        } else {
            0
        }
    }

    fn wakes_io_waiters(&self) -> bool {
        true // every signal generated wakes waiters (signal_generated)
    }

    fn as_any(&self) -> Option<&dyn core::any::Any> {
        Some(self)
    }

    fn metadata(&self) -> Result<Metadata, KernelError> {
        Ok(Metadata {
            size: 0,
            node_type: NodeType::CharDevice,
            permissions: Permissions::from_mode(0o600),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::signals::sig_bit;

    #[test]
    fn mask_leaves_out_sigkill_and_sigstop() {
        let all = u64::MAX;
        let kept = sanitize_mask(all);
        assert_eq!(kept & sig_bit(9), 0);
        assert_eq!(kept & sig_bit(19), 0);
        assert_eq!(kept | sig_bit(9) | sig_bit(19), all);
        let node = SignalFdNode::new(sig_bit(10) | sig_bit(9));
        assert_eq!(node.mask(), sig_bit(10));
        node.set_mask(sig_bit(19) | sig_bit(12));
        assert_eq!(node.mask(), sig_bit(12));
    }

    #[test]
    fn siginfo_is_linux_layout() {
        let info = SignalfdSiginfo::for_signal(10);
        let bytes = info.as_bytes();
        assert_eq!(&bytes[..4], &10u32.to_le_bytes());
        assert!(bytes[4..].iter().all(|&b| b == 0));
    }

    /// A buffer too small for one record is EINVAL; with no current
    /// process (host tests) nothing is pending, so a read is EAGAIN.
    #[test]
    fn read_needs_room_and_reports_nothing_pending() {
        let node = SignalFdNode::new(sig_bit(10));
        let mut small = [0u8; 64];
        assert!(matches!(
            node.read(0, &mut small),
            Err(KernelError::InvalidArgument { .. })
        ));
        let mut buf = [0u8; 256];
        assert!(matches!(
            node.read(0, &mut buf),
            Err(KernelError::WouldBlock)
        ));
        assert_eq!(node.poll_readiness(), 0);
    }
}
