//! Network operations for VeridianOS (stub).
//!
//! VeridianOS does not yet have a complete network stack in the kernel.
//! This module provides stub types and functions that will be implemented
//! once the network subsystem is available.
//!
//! Planned syscalls:
//! - socket, bind, listen, accept, connect
//! - send, recv, sendto, recvfrom
//! - setsockopt, getsockopt
//! - shutdown, close (reuses SYS_FILE_CLOSE)

use super::SyscallError;

// ============================================================================
// Socket types (future use)
// ============================================================================

/// Socket address family: IPv4.
pub const AF_INET: usize = 2;
/// Socket address family: IPv6.
pub const AF_INET6: usize = 10;
/// Socket address family: Unix domain.
pub const AF_UNIX: usize = 1;

/// Socket type: stream (TCP).
pub const SOCK_STREAM: usize = 1;
/// Socket type: datagram (UDP).
pub const SOCK_DGRAM: usize = 2;

// ============================================================================
// Stub Implementations
// ============================================================================

/// Create a socket (not yet implemented).
pub fn socket(_domain: usize, _sock_type: usize, _protocol: usize) -> Result<usize, SyscallError> {
    Err(SyscallError::InvalidSyscall)
}

/// Bind a socket to an address (not yet implemented).
pub fn bind(_fd: usize, _addr: *const u8, _addrlen: usize) -> Result<usize, SyscallError> {
    Err(SyscallError::InvalidSyscall)
}

/// Listen on a socket (not yet implemented).
pub fn listen(_fd: usize, _backlog: usize) -> Result<usize, SyscallError> {
    Err(SyscallError::InvalidSyscall)
}

/// Accept a connection on a socket (not yet implemented).
pub fn accept(
    _fd: usize,
    _addr: *mut u8,
    _addrlen: *mut usize,
) -> Result<usize, SyscallError> {
    Err(SyscallError::InvalidSyscall)
}

/// Connect to a remote address (not yet implemented).
pub fn connect(_fd: usize, _addr: *const u8, _addrlen: usize) -> Result<usize, SyscallError> {
    Err(SyscallError::InvalidSyscall)
}

/// Send data on a connected socket (not yet implemented).
pub fn send(
    _fd: usize,
    _buf: *const u8,
    _len: usize,
    _flags: usize,
) -> Result<usize, SyscallError> {
    Err(SyscallError::InvalidSyscall)
}

/// Receive data from a connected socket (not yet implemented).
pub fn recv(
    _fd: usize,
    _buf: *mut u8,
    _len: usize,
    _flags: usize,
) -> Result<usize, SyscallError> {
    Err(SyscallError::InvalidSyscall)
}
