//! Network extension syscall handlers (Phase 6).
//!
//! Syscalls 250-255: sendto, recvfrom, getsockname, getpeername,
//! setsockopt, getsockopt.

use super::{with_socket_fd, SyscallError, SyscallResult};
use crate::net::socket_fd::SocketHandle;

/// The socket behind `fd` in the caller's own file table.
fn socket_handle(fd: usize) -> Result<SocketHandle, SyscallError> {
    with_socket_fd(fd, |s| s.handle())
}

/// Write an AF_UNIX address with no path (`sun_family` only).
fn write_unnamed_unix_addr(addr_ptr: usize, len_ptr: usize) -> SyscallResult {
    // SAFETY: both pointers were validated by the caller (16 and 4 bytes).
    unsafe {
        core::ptr::write_unaligned(addr_ptr as *mut u16, 1); // AF_UNIX
        *(len_ptr as *mut u32) = 2;
    }
    Ok(0)
}

/// Send data to a specific address (UDP-style).
///
/// Linux ABI: `sendto(fd, buf, len, flags, dest_addr, addrlen)`
/// musl maps Linux 44 -> VeridianOS 250.
///
/// # Arguments
/// - `fd`: Socket file descriptor.
/// - `buf_ptr`: User-space data buffer.
/// - `buf_len`: Data length.
/// - `_flags`: Send flags (MSG_DONTWAIT, etc.) -- currently ignored.
/// - `addr_ptr`: User-space sockaddr pointer (arg5 in Linux ABI).
pub(super) fn sys_net_sendto(
    fd: usize,
    buf_ptr: usize,
    buf_len: usize,
    _flags: usize,
    addr_ptr: usize,
) -> SyscallResult {
    super::validate_user_buffer(buf_ptr, buf_len)?;
    // addr_len (Linux arg6) is not available due to 5-arg handler limit.
    // Infer from sa_family: AF_INET=16, AF_INET6=28, AF_UNIX=110. Default 128.
    let addr_len = if addr_ptr != 0 {
        infer_sockaddr_len(addr_ptr)?
    } else {
        0
    };
    if addr_ptr != 0 && addr_len > 0 {
        super::validate_user_buffer(addr_ptr, addr_len)?;
    }

    // SAFETY: buf_ptr validated by validate_user_buffer above as non-null and
    // within user-space.
    let data = unsafe { core::slice::from_raw_parts(buf_ptr as *const u8, buf_len) };

    let dest = if addr_ptr != 0 {
        Some(parse_sockaddr(addr_ptr, addr_len)?)
    } else {
        None
    };

    match socket_handle(fd)? {
        SocketHandle::Inet(id) => {
            crate::net::socket::sendto(id, data, dest.as_ref()).map_err(|_| SyscallError::IoError)
        }
        // send() is sendto() with no address; addressed Unix datagrams are
        // not supported.
        SocketHandle::Unix(_) if addr_ptr != 0 => Err(SyscallError::NotImplemented),
        SocketHandle::Unix(_) => {
            with_socket_fd(fd, |s| s.send(data, None))?.map_err(super::socket_err)
        }
    }
}

/// Receive data with sender address.
///
/// Linux ABI: `recvfrom(fd, buf, len, flags, src_addr, addrlen_ptr)`
/// musl maps Linux 45 -> VeridianOS 251.
///
/// # Arguments
/// - `fd`: Socket file descriptor.
/// - `buf_ptr`: User-space receive buffer.
/// - `buf_len`: Buffer capacity.
/// - `_flags`: Receive flags (MSG_DONTWAIT, etc.) -- currently ignored.
/// - `addr_ptr`: User-space sockaddr buffer (may be 0).
pub(super) fn sys_net_recvfrom(
    fd: usize,
    buf_ptr: usize,
    buf_len: usize,
    _flags: usize,
    addr_ptr: usize,
) -> SyscallResult {
    super::validate_user_buffer(buf_ptr, buf_len)?;

    // SAFETY: buf_ptr validated by validate_user_buffer above as non-null and
    // within user-space.
    let buf = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u8, buf_len) };

    let (n, src_addr) = match socket_handle(fd)? {
        SocketHandle::Inet(id) => {
            crate::net::socket::recvfrom(id, buf).map_err(|_| SyscallError::IoError)?
        }
        // recv() is recvfrom() with no address; a Unix peer has none to
        // report.
        SocketHandle::Unix(_) => (
            with_socket_fd(fd, |s| s.recv(buf))?
                .map_err(super::socket_err)?
                .0,
            None,
        ),
    };

    if let (true, Some(addr)) = (addr_ptr != 0, src_addr) {
        write_sockaddr(addr_ptr, &addr)?;
    }

    Ok(n)
}

/// Get the local address of a socket.
pub(super) fn sys_net_getsockname(fd: usize, addr_ptr: usize, len_ptr: usize) -> SyscallResult {
    super::validate_user_buffer(addr_ptr, 16)?;
    super::validate_user_buffer(len_ptr, core::mem::size_of::<u32>())?;

    let id = match socket_handle(fd)? {
        SocketHandle::Inet(id) => id,
        SocketHandle::Unix(_) => return write_unnamed_unix_addr(addr_ptr, len_ptr),
    };
    let addr = crate::net::socket::getsockname(id).map_err(|_| SyscallError::BadFileDescriptor)?;
    write_sockaddr(addr_ptr, &addr)?;

    // Write actual address length
    // SAFETY: len_ptr validated by validate_user_buffer above as non-null and
    // within user-space.
    unsafe {
        *(len_ptr as *mut u32) = 16;
    }

    Ok(0)
}

/// Get the remote address of a connected socket.
pub(super) fn sys_net_getpeername(fd: usize, addr_ptr: usize, len_ptr: usize) -> SyscallResult {
    super::validate_user_buffer(addr_ptr, 16)?;
    super::validate_user_buffer(len_ptr, core::mem::size_of::<u32>())?;

    let id = match socket_handle(fd)? {
        SocketHandle::Inet(id) => id,
        SocketHandle::Unix(_) => return write_unnamed_unix_addr(addr_ptr, len_ptr),
    };
    let addr = crate::net::socket::getpeername(id).map_err(|_| SyscallError::BadFileDescriptor)?;
    write_sockaddr(addr_ptr, &addr)?;

    // SAFETY: len_ptr validated by validate_user_buffer above as non-null and
    // within user-space.
    unsafe {
        *(len_ptr as *mut u32) = 16;
    }

    Ok(0)
}

/// Set a socket option.
pub(super) fn sys_net_setsockopt(
    fd: usize,
    level: usize,
    optname: usize,
    optval_ptr: usize,
    optlen: usize,
) -> SyscallResult {
    if optval_ptr != 0 && optlen > 0 {
        super::validate_user_buffer(optval_ptr, optlen)?;
    }
    match socket_handle(fd)? {
        SocketHandle::Inet(id) => {
            crate::net::socket::setsockopt(id, level as i32, optname as i32, optval_ptr, optlen)
                .map_err(|_| SyscallError::InvalidArgument)
        }
        // Unix sockets have no settable options yet; accept and ignore,
        // as the INET path does for options it does not model.
        SocketHandle::Unix(_) => Ok(0),
    }
}

/// Get a socket option.
pub(super) fn sys_net_getsockopt(
    fd: usize,
    level: usize,
    optname: usize,
    optval_ptr: usize,
) -> SyscallResult {
    if optval_ptr != 0 {
        super::validate_user_buffer(optval_ptr, 4)?;
    }
    match socket_handle(fd)? {
        SocketHandle::Inet(id) => {
            crate::net::socket::getsockopt(id, level as i32, optname as i32, optval_ptr)
                .map_err(|_| SyscallError::InvalidArgument)
        }
        SocketHandle::Unix(_) => Ok(0),
    }
}

/// Infer sockaddr length from sa_family when the actual length is unavailable
/// (e.g., sendto where arg6 is lost due to 5-arg handler limit).
fn infer_sockaddr_len(addr_ptr: usize) -> Result<usize, SyscallError> {
    // The family field is read before the full length is known, so it is
    // validated on its own first (NET-SEC-02).
    super::validate_user_buffer(addr_ptr, core::mem::size_of::<u16>())?;
    // SAFETY: the two bytes at addr_ptr were validated as user memory above;
    // read_unaligned because a user sockaddr carries no alignment guarantee.
    let family = unsafe { core::ptr::read_unaligned(addr_ptr as *const u16) };
    Ok(match family {
        2 => 16,  // AF_INET: sizeof(sockaddr_in)
        10 => 28, // AF_INET6: sizeof(sockaddr_in6)
        1 => 110, // AF_UNIX: sizeof(sockaddr_un)
        _ => 128, // Conservative default
    })
}

/// Parse a sockaddr_in from user space.
///
/// The bytes are copied out with the fault-handled user-copy routine and
/// decoded from a local buffer: the user pointer need not be aligned, and
/// dereferencing it as `*const u16`/`*const u32` was undefined behaviour on
/// an unaligned address (review of the v0.26.0 stack, PR #7).
fn parse_sockaddr(
    addr_ptr: usize,
    _addr_len: usize,
) -> Result<crate::net::SocketAddr, SyscallError> {
    // struct sockaddr_in { u16 family, u16 port_be, u32 addr_be, u8 zero[8] }
    let mut raw = [0u8; 8];
    super::userspace::read_user_bytes(addr_ptr, &mut raw)?;
    let family = u16::from_ne_bytes([raw[0], raw[1]]);
    if family != 2 {
        // AF_INET = 2
        return Err(SyscallError::InvalidArgument);
    }
    let port = u16::from_be_bytes([raw[2], raw[3]]);
    let addr_bytes = [raw[4], raw[5], raw[6], raw[7]];

    Ok(crate::net::SocketAddr {
        ip: crate::net::IpAddress::V4(crate::net::Ipv4Address(addr_bytes)),
        port,
    })
}

/// Encode `addr` as a 16-byte `struct sockaddr_in`.
fn sockaddr_in_bytes(addr: &crate::net::SocketAddr) -> [u8; 16] {
    let ip = match &addr.ip {
        crate::net::IpAddress::V4(v4) => v4.0,
        _ => [0, 0, 0, 0],
    };
    let mut out = [0u8; 16];
    out[0..2].copy_from_slice(&2u16.to_ne_bytes()); // AF_INET
    out[2..4].copy_from_slice(&addr.port.to_be_bytes());
    out[4..8].copy_from_slice(&ip);
    out
}

/// Write a SocketAddr as sockaddr_in to user space, through the
/// fault-handled user-copy routine (no unaligned raw stores).
pub(super) fn write_sockaddr(
    addr_ptr: usize,
    addr: &crate::net::SocketAddr,
) -> Result<(), SyscallError> {
    super::userspace::write_user_bytes(addr_ptr, &sockaddr_in_bytes(addr))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sockaddr_in_encoding_matches_the_c_layout() {
        let addr = crate::net::SocketAddr {
            ip: crate::net::IpAddress::V4(crate::net::Ipv4Address([10, 0, 2, 15])),
            port: 8080,
        };
        let b = sockaddr_in_bytes(&addr);
        assert_eq!(u16::from_ne_bytes([b[0], b[1]]), 2);
        assert_eq!(&b[2..4], &8080u16.to_be_bytes());
        assert_eq!(&b[4..8], &[10, 0, 2, 15]);
        assert!(b[8..].iter().all(|&z| z == 0));
    }

    /// NET-SEC-02: the family must not be read from an address that fails
    /// user-pointer validation (here, a kernel-half address).
    #[test]
    fn infer_sockaddr_len_rejects_kernel_pointer() {
        assert!(infer_sockaddr_len(0xFFFF_8000_0000_1000).is_err());
    }

    #[test]
    fn infer_sockaddr_len_maps_families() {
        for (family, expected) in [(2u16, 16usize), (10, 28), (1, 110), (99, 128)] {
            let addr = [family, 0u16];
            assert_eq!(infer_sockaddr_len(addr.as_ptr() as usize), Ok(expected));
        }
    }
}
