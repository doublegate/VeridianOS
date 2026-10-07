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
///
/// Both stores go through the fault-handled user-copy routines: neither
/// user pointer is guaranteed to be aligned or mapped (review of the v0.26.0
/// stack, PRs #7 and #10).
fn write_unnamed_unix_addr(addr_ptr: usize, len_ptr: usize) -> SyscallResult {
    super::userspace::write_user_bytes(addr_ptr, &1u16.to_ne_bytes())?; // AF_UNIX
    super::userspace::write_user::<u32>(len_ptr, 2)?;
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

    // The datagram is copied in whole through the fault-tolerant reader (N-43).
    let data =
        super::userspace::read_user_vec(buf_ptr, buf_len, super::userspace::MAX_USER_MESSAGE)?;
    let data = &data[..];

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

    // Received into a kernel buffer, then copied out once (N-43).
    let mut kbuf = alloc::vec![0u8; buf_len.min(super::userspace::MAX_USER_MESSAGE)];
    let buf = &mut kbuf[..];

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

    super::userspace::write_user_bytes(buf_ptr, &kbuf[..n])?;
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

    // Write actual address length (unaligned-safe, fault-handled).
    super::userspace::write_user::<u32>(len_ptr, 16)?;

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

    super::userspace::write_user::<u32>(len_ptr, 16)?;

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
///
/// Linux ABI: `getsockopt(fd, level, optname, optval, optlen_ptr)`.
pub(super) fn sys_net_getsockopt(
    fd: usize,
    level: usize,
    optname: usize,
    optval_ptr: usize,
    optlen_ptr: usize,
) -> SyscallResult {
    match socket_handle(fd)? {
        SocketHandle::Inet(id) => {
            if optval_ptr != 0 {
                super::validate_user_buffer(optval_ptr, 4)?;
            }
            let r = crate::net::socket::getsockopt(id, level as i32, optname as i32, optval_ptr)
                .map_err(|_| SyscallError::InvalidArgument)?;
            // The INET layer writes a 4-byte value.
            if optlen_ptr != 0 {
                super::userspace::write_user::<u32>(optlen_ptr, 4)?;
            }
            Ok(r)
        }
        // This used to report success without writing optval (review of
        // the v0.26.0 stack, PR #10).
        SocketHandle::Unix(id) => {
            let ty =
                crate::net::unix_socket::socket_type(id).ok_or(SyscallError::BadFileDescriptor)?;
            let value = unix_sockopt(level, optname, ty)?.to_ne_bytes();
            // As Linux: copy at most *optlen bytes and store the length
            // copied. A NULL optlen is tolerated (the full int is written),
            // matching the INET path, which never reads it.
            let n = if optlen_ptr != 0 {
                sockopt_copy_len(super::userspace::read_user::<u32>(optlen_ptr)?)?
            } else {
                value.len()
            };
            super::userspace::write_user_bytes(optval_ptr, &value[..n])?;
            if optlen_ptr != 0 {
                super::userspace::write_user::<u32>(optlen_ptr, n as u32)?;
            }
            Ok(0)
        }
    }
}

const SOL_SOCKET: usize = 1;
const SO_TYPE: usize = 3;
const SO_ERROR: usize = 4;

/// The `int` value of option (`level`, `optname`) on a Unix socket of type
/// `ty`. Only SOL_SOCKET SO_ERROR (no pending error is tracked, so 0) and
/// SO_TYPE are modelled; anything else is ENOPROTOOPT.
fn unix_sockopt(
    level: usize,
    optname: usize,
    ty: crate::net::unix_socket::UnixSocketType,
) -> Result<i32, SyscallError> {
    use crate::net::unix_socket::UnixSocketType;
    match (level, optname) {
        (SOL_SOCKET, SO_ERROR) => Ok(0),
        (SOL_SOCKET, SO_TYPE) => Ok(match ty {
            UnixSocketType::Stream => 1,   // SOCK_STREAM
            UnixSocketType::Datagram => 2, // SOCK_DGRAM
        }),
        _ => Err(SyscallError::ProtocolOptionNotAvailable),
    }
}

/// Bytes of an `int` option to copy for a caller's `*optlen` (a socklen_t
/// read as signed, as Linux does): negative is EINVAL, larger is clamped.
fn sockopt_copy_len(optlen: u32) -> Result<usize, SyscallError> {
    let len = optlen as i32;
    if len < 0 {
        return Err(SyscallError::InvalidArgument);
    }
    Ok((len as usize).min(core::mem::size_of::<i32>()))
}

/// Infer sockaddr length from sa_family when the actual length is unavailable
/// (e.g., sendto where arg6 is lost due to 5-arg handler limit).
fn infer_sockaddr_len(addr_ptr: usize) -> Result<usize, SyscallError> {
    // The family field is read before the full length is known, so only its
    // two bytes are copied (NET-SEC-02). read_user_bytes validates the range
    // and is fault-handled; a raw read_unaligned was neither (review of the
    // v0.26.0 stack, PR #7).
    let mut fam = [0u8; 2];
    super::userspace::read_user_bytes(addr_ptr, &mut fam)?;
    let family = u16::from_ne_bytes(fam);
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
pub(super) fn sockaddr_in_bytes(addr: &crate::net::SocketAddr) -> [u8; 16] {
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

    /// getsockopt on a Unix socket answers SO_ERROR and SO_TYPE and fails
    /// everything else with ENOPROTOOPT (review of the v0.26.0 stack,
    /// PR #10).
    #[test]
    fn unix_sockopt_values() {
        use crate::net::unix_socket::UnixSocketType;
        assert_eq!(unix_sockopt(1, 4, UnixSocketType::Stream), Ok(0));
        assert_eq!(unix_sockopt(1, 3, UnixSocketType::Stream), Ok(1));
        assert_eq!(unix_sockopt(1, 3, UnixSocketType::Datagram), Ok(2));
        assert_eq!(
            unix_sockopt(1, 2, UnixSocketType::Stream),
            Err(SyscallError::ProtocolOptionNotAvailable)
        );
        assert_eq!(
            unix_sockopt(6, 4, UnixSocketType::Stream),
            Err(SyscallError::ProtocolOptionNotAvailable)
        );
        assert_eq!(
            super::super::linux_compat::to_linux_errno(SyscallError::ProtocolOptionNotAvailable),
            -92
        );
    }

    #[test]
    fn sockopt_copy_len_truncates_like_linux() {
        assert_eq!(sockopt_copy_len(8), Ok(4));
        assert_eq!(sockopt_copy_len(4), Ok(4));
        assert_eq!(sockopt_copy_len(1), Ok(1));
        assert_eq!(sockopt_copy_len(0), Ok(0));
        assert_eq!(
            sockopt_copy_len(u32::MAX),
            Err(SyscallError::InvalidArgument)
        );
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

    /// A user sockaddr has no alignment guarantee (review of the v0.26.0
    /// stack, PR #7).
    #[test]
    fn infer_sockaddr_len_unaligned() {
        let mut buf = [0u8; 4];
        buf[1..3].copy_from_slice(&2u16.to_ne_bytes());
        assert_eq!(infer_sockaddr_len(buf.as_ptr() as usize + 1), Ok(16));
    }

    /// `socklen_t *` from user space may be misaligned; an aligned `u32`
    /// store through it is undefined behaviour (review of the v0.26.0
    /// stack, PRs #7 and #10).
    #[test]
    fn write_unnamed_unix_addr_misaligned_len() {
        let mut addr = [0xAAu8; 5];
        let mut len = [0xAAu8; 8];
        let addr_ptr = addr.as_mut_ptr() as usize + 1;
        let len_ptr = len.as_mut_ptr() as usize + 1;
        assert_eq!(write_unnamed_unix_addr(addr_ptr, len_ptr), Ok(0));
        assert_eq!(u16::from_ne_bytes([addr[1], addr[2]]), 1);
        assert_eq!(u32::from_ne_bytes([len[1], len[2], len[3], len[4]]), 2);
        assert_eq!(len[0], 0xAA);
        assert_eq!(len[5], 0xAA);
    }

    #[test]
    fn write_unnamed_unix_addr_rejects_kernel_pointers() {
        let mut len = [0u8; 4];
        assert!(write_unnamed_unix_addr(0xFFFF_8000_0000_1000, len.as_mut_ptr() as usize).is_err());
        let mut addr = [0u8; 2];
        assert!(
            write_unnamed_unix_addr(addr.as_mut_ptr() as usize, 0xFFFF_8000_0000_1000).is_err()
        );
    }
}
