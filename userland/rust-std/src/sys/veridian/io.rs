//! I/O primitives for VeridianOS.
//!
//! Provides stdin (fd 0), stdout (fd 1), and stderr (fd 2) abstractions
//! over VeridianOS file descriptors.

use super::fs;
use super::SyscallError;

/// Standard input file descriptor.
pub const STDIN_FD: usize = 0;
/// Standard output file descriptor.
pub const STDOUT_FD: usize = 1;
/// Standard error file descriptor.
pub const STDERR_FD: usize = 2;

/// Read from stdin into the provided buffer.
///
/// Returns the number of bytes actually read (0 at EOF).
pub fn stdin_read(buf: &mut [u8]) -> Result<usize, SyscallError> {
    fs::read(STDIN_FD, buf.as_mut_ptr(), buf.len())
}

/// Write a byte slice to stdout.
///
/// Returns the number of bytes written.
pub fn stdout_write(data: &[u8]) -> Result<usize, SyscallError> {
    fs::write(STDOUT_FD, data.as_ptr(), data.len())
}

/// Write a byte slice to stderr.
///
/// Returns the number of bytes written.
pub fn stderr_write(data: &[u8]) -> Result<usize, SyscallError> {
    fs::write(STDERR_FD, data.as_ptr(), data.len())
}

/// Write all bytes to a file descriptor, retrying on partial writes.
///
/// Returns the total number of bytes written (always `data.len()` on success).
pub fn write_all(fd: usize, data: &[u8]) -> Result<usize, SyscallError> {
    let mut written = 0;
    while written < data.len() {
        let remaining = &data[written..];
        let n = fs::write(fd, remaining.as_ptr(), remaining.len())?;
        if n == 0 {
            // Would block or pipe broken -- should not happen for stdout/stderr
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
