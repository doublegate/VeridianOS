//! Network extension syscall handlers (Phase 6).
//!
//! Syscalls 250-255: sendto, recvfrom, getsockname, getpeername,
//! setsockopt, getsockopt.

use super::{SyscallError, SyscallResult};

/// Send data to a specific address (UDP-style).
///
/// # Arguments
/// - `fd`: Socket file descriptor.
/// - `buf_ptr`: User-space data buffer.
/// - `buf_len`: Data length.
/// - `addr_ptr`: User-space sockaddr pointer.
/// - `addr_len`: Address structure length.
pub(super) fn sys_net_sendto(
    fd: usize,
    buf_ptr: usize,
    buf_len: usize,
    addr_ptr: usize,
    addr_len: usize,
) -> SyscallResult {
    super::validate_user_buffer(buf_ptr, buf_len)?;
    if addr_ptr != 0 {
        super::validate_user_buffer(addr_ptr, addr_len)?;
    }

    let data = unsafe { core::slice::from_raw_parts(buf_ptr as *const u8, buf_len) };

    let dest = if addr_ptr != 0 {
        Some(parse_sockaddr(addr_ptr, addr_len)?)
    } else {
        None
    };

    crate::net::socket::sendto(fd, data, dest.as_ref()).map_err(|_| SyscallError::IoError)
}

/// Receive data with sender address.
///
/// # Arguments
/// - `fd`: Socket file descriptor.
/// - `buf_ptr`: User-space receive buffer.
/// - `buf_len`: Buffer capacity.
/// - `addr_ptr`: User-space sockaddr buffer (may be 0).
pub(super) fn sys_net_recvfrom(
    fd: usize,
    buf_ptr: usize,
    buf_len: usize,
    addr_ptr: usize,
) -> SyscallResult {
    super::validate_user_buffer(buf_ptr, buf_len)?;

    let buf = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u8, buf_len) };

    let (n, src_addr) = crate::net::socket::recvfrom(fd, buf).map_err(|_| SyscallError::IoError)?;

    if let (true, Some(addr)) = (addr_ptr != 0, src_addr) {
        write_sockaddr(addr_ptr, &addr)?;
    }

    Ok(n)
}

/// Get the local address of a socket.
pub(super) fn sys_net_getsockname(fd: usize, addr_ptr: usize, len_ptr: usize) -> SyscallResult {
    super::validate_user_buffer(addr_ptr, 16)?;
    super::validate_user_buffer(len_ptr, core::mem::size_of::<u32>())?;

    let addr = crate::net::socket::getsockname(fd).map_err(|_| SyscallError::BadFileDescriptor)?;
    write_sockaddr(addr_ptr, &addr)?;

    // Write actual address length
    unsafe {
        *(len_ptr as *mut u32) = 16;
    }

    Ok(0)
}

/// Get the remote address of a connected socket.
pub(super) fn sys_net_getpeername(fd: usize, addr_ptr: usize, len_ptr: usize) -> SyscallResult {
    super::validate_user_buffer(addr_ptr, 16)?;
    super::validate_user_buffer(len_ptr, core::mem::size_of::<u32>())?;

    let addr = crate::net::socket::getpeername(fd).map_err(|_| SyscallError::BadFileDescriptor)?;
    write_sockaddr(addr_ptr, &addr)?;

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
    crate::net::socket::setsockopt(fd, level as i32, optname as i32, optval_ptr, optlen)
        .map_err(|_| SyscallError::InvalidArgument)
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
    crate::net::socket::getsockopt(fd, level as i32, optname as i32, optval_ptr)
        .map_err(|_| SyscallError::InvalidArgument)
}

/// Parse a sockaddr_in from user space.
fn parse_sockaddr(
    addr_ptr: usize,
    _addr_len: usize,
) -> Result<crate::net::SocketAddr, SyscallError> {
    // struct sockaddr_in { u16 family, u16 port_be, u32 addr_be, u8 zero[8] }
    let family = unsafe { *(addr_ptr as *const u16) };
    if family != 2 {
        // AF_INET = 2
        return Err(SyscallError::InvalidArgument);
    }
    let port_be = unsafe { *((addr_ptr + 2) as *const u16) };
    let addr_be = unsafe { *((addr_ptr + 4) as *const u32) };

    let port = u16::from_be(port_be);
    let addr_bytes = addr_be.to_be_bytes();

    Ok(crate::net::SocketAddr {
        ip: crate::net::IpAddress::V4(crate::net::Ipv4Address(addr_bytes)),
        port,
    })
}

/// Write a SocketAddr as sockaddr_in to user space.
fn write_sockaddr(addr_ptr: usize, addr: &crate::net::SocketAddr) -> Result<(), SyscallError> {
    let bytes = match &addr.ip {
        crate::net::IpAddress::V4(v4) => v4.0,
        _ => [0, 0, 0, 0],
    };

    // struct sockaddr_in: family(2) + port_be(2) + addr_be(4) + zero(8)
    unsafe {
        *(addr_ptr as *mut u16) = 2; // AF_INET
        *((addr_ptr + 2) as *mut u16) = addr.port.to_be();
        *((addr_ptr + 4) as *mut u32) = u32::from_be_bytes(bytes);
        // Zero padding
        core::ptr::write_bytes((addr_ptr + 8) as *mut u8, 0, 8);
    }

    Ok(())
}
