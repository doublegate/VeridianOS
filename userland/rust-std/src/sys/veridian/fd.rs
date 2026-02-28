//! File descriptor management for VeridianOS.
//!
//! Provides `SharedFd`, a reference-counted file descriptor wrapper that
//! automatically closes the underlying fd when all references are dropped.
//! This is the foundation type used by `File`, `TcpStream`, `UdpSocket`,
//! and other I/O types.

extern crate alloc;

use core::sync::atomic::{AtomicUsize, Ordering};

use super::{fs, SyscallError};

// ============================================================================
// SharedFd -- reference-counted file descriptor
// ============================================================================

/// Inner state of a reference-counted file descriptor.
///
/// Heap-allocated via a static bump allocator (no `alloc` crate dependency
/// required in the hot path -- callers use `alloc::boxed::Box` when available).
struct FdInner {
    /// The raw operating system file descriptor.
    fd: usize,
    /// Reference count.  Starts at 1 on creation.
    refcount: AtomicUsize,
}

/// A reference-counted wrapper around an OS file descriptor.
///
/// When the last `SharedFd` referencing a given descriptor is dropped the
/// descriptor is closed via `SYS_FILE_CLOSE`.
///
/// `SharedFd` is `Send` and `Sync` -- the file descriptor itself is just an
/// integer and the kernel handles concurrent access safely.
pub struct SharedFd {
    /// Pointer to the heap-allocated inner state.
    inner: *mut FdInner,
}

// SAFETY: File descriptors are plain integers.  Concurrent read/write on the
// same fd is well-defined at the OS level (the kernel serializes as needed).
// The reference count uses atomics for thread safety.
unsafe impl Send for SharedFd {}
unsafe impl Sync for SharedFd {}

impl SharedFd {
    /// Wrap a raw file descriptor in a new `SharedFd` with refcount 1.
    ///
    /// # Safety
    /// The caller must own the file descriptor (i.e. it was returned by a
    /// syscall such as `open` or `pipe`) and must not close it manually.
    pub unsafe fn from_raw(fd: usize) -> Self {
        // Allocate the inner state.  We use `alloc::boxed::Box` which is
        // available because the workspace enables `build-std = [... "alloc"]`.
        let inner = alloc::boxed::Box::into_raw(alloc::boxed::Box::new(FdInner {
            fd,
            refcount: AtomicUsize::new(1),
        }));
        SharedFd { inner }
    }

    /// Return the raw file descriptor number.
    #[inline]
    pub fn raw(&self) -> usize {
        // SAFETY: `inner` is always valid while `SharedFd` exists.
        unsafe { (*self.inner).fd }
    }

    /// Read from the file descriptor into `buf`.
    ///
    /// Returns the number of bytes read.
    pub fn read(&self, buf: &mut [u8]) -> Result<usize, SyscallError> {
        fs::read(self.raw(), buf.as_mut_ptr(), buf.len())
    }

    /// Write `data` to the file descriptor.
    ///
    /// Returns the number of bytes written.
    pub fn write(&self, data: &[u8]) -> Result<usize, SyscallError> {
        fs::write(self.raw(), data.as_ptr(), data.len())
    }

    /// Duplicate the underlying file descriptor (via `dup`).
    pub fn try_clone(&self) -> Result<SharedFd, SyscallError> {
        let new_fd = fs::dup(self.raw())?;
        // SAFETY: `dup` returned a new valid fd that we now own.
        Ok(unsafe { SharedFd::from_raw(new_fd) })
    }

    /// Consume the `SharedFd` and return the raw fd *without* closing it.
    ///
    /// The caller is now responsible for closing the descriptor.
    pub fn into_raw(self) -> usize {
        let fd = self.raw();
        // Decrement refcount.  If we are the sole owner the inner is freed
        // but we skip the close().
        // SAFETY: inner is valid.
        unsafe {
            let prev = (*self.inner).refcount.fetch_sub(1, Ordering::Release);
            if prev == 1 {
                core::sync::atomic::fence(Ordering::Acquire);
                // Free the allocation without closing the fd.
                drop(alloc::boxed::Box::from_raw(self.inner));
            }
        }
        core::mem::forget(self);
        fd
    }
}

impl Clone for SharedFd {
    fn clone(&self) -> Self {
        // SAFETY: inner is valid while any SharedFd exists.
        unsafe {
            (*self.inner).refcount.fetch_add(1, Ordering::Relaxed);
        }
        SharedFd { inner: self.inner }
    }
}

impl Drop for SharedFd {
    fn drop(&mut self) {
        // SAFETY: inner is valid.
        unsafe {
            let prev = (*self.inner).refcount.fetch_sub(1, Ordering::Release);
            if prev == 1 {
                // Last reference -- close the fd and free inner.
                core::sync::atomic::fence(Ordering::Acquire);
                let _ = fs::close((*self.inner).fd);
                drop(alloc::boxed::Box::from_raw(self.inner));
            }
        }
    }
}

impl core::fmt::Debug for SharedFd {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SharedFd").field("fd", &self.raw()).finish()
    }
}

// ============================================================================
// OwnedFd -- single-owner variant (no refcounting overhead)
// ============================================================================

/// A uniquely-owned file descriptor.
///
/// Unlike `SharedFd`, `OwnedFd` cannot be cloned (use `try_clone()` to
/// `dup()` the underlying fd).  It closes the fd on drop.
pub struct OwnedFd {
    fd: usize,
}

impl OwnedFd {
    /// Wrap a raw file descriptor.
    ///
    /// # Safety
    /// The caller must own the fd.
    #[inline]
    pub unsafe fn from_raw(fd: usize) -> Self {
        OwnedFd { fd }
    }

    /// Return the raw fd number.
    #[inline]
    pub fn raw(&self) -> usize {
        self.fd
    }

    /// Duplicate via `dup`.
    pub fn try_clone(&self) -> Result<OwnedFd, SyscallError> {
        let new_fd = fs::dup(self.fd)?;
        Ok(OwnedFd { fd: new_fd })
    }

    /// Consume without closing.
    #[inline]
    pub fn into_raw(self) -> usize {
        let fd = self.fd;
        core::mem::forget(self);
        fd
    }

    /// Read from this fd.
    pub fn read(&self, buf: &mut [u8]) -> Result<usize, SyscallError> {
        fs::read(self.fd, buf.as_mut_ptr(), buf.len())
    }

    /// Write to this fd.
    pub fn write(&self, data: &[u8]) -> Result<usize, SyscallError> {
        fs::write(self.fd, data.as_ptr(), data.len())
    }
}

impl Drop for OwnedFd {
    fn drop(&mut self) {
        let _ = fs::close(self.fd);
    }
}

impl core::fmt::Debug for OwnedFd {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("OwnedFd").field("fd", &self.fd).finish()
    }
}

// SAFETY: File descriptors are plain integers; the kernel handles concurrency.
unsafe impl Send for OwnedFd {}
unsafe impl Sync for OwnedFd {}
