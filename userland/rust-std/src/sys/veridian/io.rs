//! I/O primitives for VeridianOS.
//!
//! Provides structured I/O types that mirror `std::io`:
//!
//! - `Stdin` / `Stdout` / `Stderr` -- typed wrappers around fd 0, 1, 2
//! - `AnonPipe` -- an anonymous pipe created via `pipe()`
//! - `IoSlice` / `IoSliceMut` -- scatter/gather I/O buffers
//! - `Read` / `Write` traits (simplified versions)

extern crate alloc;

use super::{fd::OwnedFd, fs, SyscallError};

/// Standard input file descriptor.
pub const STDIN_FD: usize = 0;
/// Standard output file descriptor.
pub const STDOUT_FD: usize = 1;
/// Standard error file descriptor.
pub const STDERR_FD: usize = 2;

// ============================================================================
// Stdin
// ============================================================================

/// Handle to standard input (file descriptor 0).
///
/// Unlike `File`, `Stdin` does not close fd 0 on drop.
pub struct Stdin {
    _priv: (),
}

/// Obtain a handle to standard input.
pub fn stdin() -> Stdin {
    Stdin { _priv: () }
}

impl Stdin {
    /// Read bytes into `buf`.
    pub fn read(&self, buf: &mut [u8]) -> Result<usize, SyscallError> {
        fs::read(STDIN_FD, buf.as_mut_ptr(), buf.len())
    }

    /// Read a line (until `\n` or EOF) into the buffer.
    ///
    /// Returns the number of bytes read (including the newline if present).
    pub fn read_line(&self, buf: &mut alloc::vec::Vec<u8>) -> Result<usize, SyscallError> {
        let mut total = 0;
        let mut byte = [0u8; 1];
        loop {
            let n = self.read(&mut byte)?;
            if n == 0 {
                break;
            }
            buf.push(byte[0]);
            total += 1;
            if byte[0] == b'\n' {
                break;
            }
        }
        Ok(total)
    }
}

impl core::fmt::Debug for Stdin {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Stdin").finish()
    }
}

// ============================================================================
// Stdout
// ============================================================================

/// Handle to standard output (file descriptor 1).
pub struct Stdout {
    _priv: (),
}

/// Obtain a handle to standard output.
pub fn stdout() -> Stdout {
    Stdout { _priv: () }
}

impl Stdout {
    /// Write bytes.
    pub fn write(&self, data: &[u8]) -> Result<usize, SyscallError> {
        fs::write(STDOUT_FD, data.as_ptr(), data.len())
    }

    /// Write all bytes.
    pub fn write_all(&self, data: &[u8]) -> Result<(), SyscallError> {
        write_all(STDOUT_FD, data)?;
        Ok(())
    }

    /// Flush (no-op for VeridianOS serial/pipe I/O which is unbuffered).
    pub fn flush(&self) -> Result<(), SyscallError> {
        Ok(())
    }
}

impl core::fmt::Debug for Stdout {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Stdout").finish()
    }
}

// ============================================================================
// Stderr
// ============================================================================

/// Handle to standard error (file descriptor 2).
pub struct Stderr {
    _priv: (),
}

/// Obtain a handle to standard error.
pub fn stderr() -> Stderr {
    Stderr { _priv: () }
}

impl Stderr {
    /// Write bytes.
    pub fn write(&self, data: &[u8]) -> Result<usize, SyscallError> {
        fs::write(STDERR_FD, data.as_ptr(), data.len())
    }

    /// Write all bytes.
    pub fn write_all(&self, data: &[u8]) -> Result<(), SyscallError> {
        write_all(STDERR_FD, data)?;
        Ok(())
    }

    /// Flush (no-op).
    pub fn flush(&self) -> Result<(), SyscallError> {
        Ok(())
    }
}

impl core::fmt::Debug for Stderr {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Stderr").finish()
    }
}

// ============================================================================
// Convenience functions (preserved from original API)
// ============================================================================

/// Read from stdin into the provided buffer.
pub fn stdin_read(buf: &mut [u8]) -> Result<usize, SyscallError> {
    fs::read(STDIN_FD, buf.as_mut_ptr(), buf.len())
}

/// Write a byte slice to stdout.
pub fn stdout_write(data: &[u8]) -> Result<usize, SyscallError> {
    fs::write(STDOUT_FD, data.as_ptr(), data.len())
}

/// Write a byte slice to stderr.
pub fn stderr_write(data: &[u8]) -> Result<usize, SyscallError> {
    fs::write(STDERR_FD, data.as_ptr(), data.len())
}

/// Write all bytes to a file descriptor, retrying on partial writes.
pub fn write_all(fd: usize, data: &[u8]) -> Result<usize, SyscallError> {
    let mut written = 0;
    while written < data.len() {
        let remaining = &data[written..];
        let n = fs::write(fd, remaining.as_ptr(), remaining.len())?;
        if n == 0 {
            return Err(SyscallError::InvalidState);
        }
        written += n;
    }
    Ok(written)
}

/// Print a string to stdout (no newline).
pub fn print(s: &str) -> Result<usize, SyscallError> {
    write_all(STDOUT_FD, s.as_bytes())
}

/// Print a string to stdout with a trailing newline.
pub fn println(s: &str) -> Result<usize, SyscallError> {
    let n1 = write_all(STDOUT_FD, s.as_bytes())?;
    let n2 = write_all(STDOUT_FD, b"\n")?;
    Ok(n1 + n2)
}

/// Print a string to stderr (no newline).
pub fn eprint(s: &str) -> Result<usize, SyscallError> {
    write_all(STDERR_FD, s.as_bytes())
}

/// Print a string to stderr with a trailing newline.
pub fn eprintln(s: &str) -> Result<usize, SyscallError> {
    let n1 = write_all(STDERR_FD, s.as_bytes())?;
    let n2 = write_all(STDERR_FD, b"\n")?;
    Ok(n1 + n2)
}

// ============================================================================
// AnonPipe
// ============================================================================

/// An anonymous pipe created via `pipe()`.
///
/// Contains a read end and a write end.  Both are closed on drop.
pub struct AnonPipe {
    /// The read end of the pipe.
    read_fd: OwnedFd,
    /// The write end of the pipe.
    write_fd: OwnedFd,
}

impl AnonPipe {
    /// Create a new anonymous pipe.
    pub fn new() -> Result<Self, SyscallError> {
        let mut fds: [i32; 2] = [0; 2];
        fs::pipe(&mut fds)?;
        // SAFETY: pipe() returned two valid fds.
        unsafe {
            Ok(AnonPipe {
                read_fd: OwnedFd::from_raw(fds[0] as usize),
                write_fd: OwnedFd::from_raw(fds[1] as usize),
            })
        }
    }

    /// Read from the pipe.
    pub fn read(&self, buf: &mut [u8]) -> Result<usize, SyscallError> {
        self.read_fd.read(buf)
    }

    /// Write to the pipe.
    pub fn write(&self, data: &[u8]) -> Result<usize, SyscallError> {
        self.write_fd.write(data)
    }

    /// Get the raw read-end file descriptor.
    pub fn read_fd(&self) -> usize {
        self.read_fd.raw()
    }

    /// Get the raw write-end file descriptor.
    pub fn write_fd(&self) -> usize {
        self.write_fd.raw()
    }

    /// Take ownership of the read end (the pipe keeps the write end).
    ///
    /// After this call, the read end will NOT be closed when the `AnonPipe`
    /// is dropped.  The caller owns it.
    pub fn take_read_fd(&mut self) -> OwnedFd {
        let fd = self.read_fd.raw();
        let taken = unsafe { OwnedFd::from_raw(fd) };
        // Replace the original with a dummy that won't close anything useful.
        // We dup the write fd as a placeholder -- it'll be closed on drop,
        // but that's harmless since the original write fd is still held.
        // Actually, the cleanest approach: just forget the old one.
        let old_fd =
            core::mem::replace(&mut self.read_fd, unsafe { OwnedFd::from_raw(usize::MAX) });
        core::mem::forget(old_fd);
        taken
    }

    /// Take ownership of the write end.
    pub fn take_write_fd(&mut self) -> OwnedFd {
        let fd = self.write_fd.raw();
        let taken = unsafe { OwnedFd::from_raw(fd) };
        let old_fd =
            core::mem::replace(&mut self.write_fd, unsafe { OwnedFd::from_raw(usize::MAX) });
        core::mem::forget(old_fd);
        taken
    }
}

impl core::fmt::Debug for AnonPipe {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AnonPipe")
            .field("read_fd", &self.read_fd)
            .field("write_fd", &self.write_fd)
            .finish()
    }
}

// ============================================================================
// IoSlice / IoSliceMut (vectored I/O)
// ============================================================================

/// A buffer for vectored writes.
///
/// Wraps a `&[u8]` in a layout compatible with the POSIX `struct iovec`.
#[repr(C)]
pub struct IoSlice<'a> {
    /// Pointer to data.
    iov_base: *const u8,
    /// Length of data.
    iov_len: usize,
    /// Lifetime tie.
    _marker: core::marker::PhantomData<&'a [u8]>,
}

impl<'a> IoSlice<'a> {
    /// Create from a byte slice.
    #[inline]
    pub fn new(buf: &'a [u8]) -> Self {
        IoSlice {
            iov_base: buf.as_ptr(),
            iov_len: buf.len(),
            _marker: core::marker::PhantomData,
        }
    }

    /// View the underlying slice.
    #[inline]
    pub fn as_slice(&self) -> &'a [u8] {
        // SAFETY: We hold a reference to the original slice.
        unsafe { core::slice::from_raw_parts(self.iov_base, self.iov_len) }
    }
}

/// A buffer for vectored reads.
#[repr(C)]
pub struct IoSliceMut<'a> {
    /// Pointer to buffer.
    iov_base: *mut u8,
    /// Capacity.
    iov_len: usize,
    /// Lifetime tie.
    _marker: core::marker::PhantomData<&'a mut [u8]>,
}

impl<'a> IoSliceMut<'a> {
    /// Create from a mutable byte slice.
    #[inline]
    pub fn new(buf: &'a mut [u8]) -> Self {
        IoSliceMut {
            iov_base: buf.as_mut_ptr(),
            iov_len: buf.len(),
            _marker: core::marker::PhantomData,
        }
    }

    /// View the underlying mutable slice.
    #[inline]
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        // SAFETY: We hold a mutable reference to the original slice.
        unsafe { core::slice::from_raw_parts_mut(self.iov_base, self.iov_len) }
    }

    /// View as immutable slice.
    #[inline]
    pub fn as_slice(&self) -> &[u8] {
        unsafe { core::slice::from_raw_parts(self.iov_base, self.iov_len) }
    }
}
