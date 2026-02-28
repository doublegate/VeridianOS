//! Network operations for VeridianOS.
//!
//! Provides TCP and UDP socket types that bridge to the VeridianOS kernel
//! network stack (implemented in Phase 6):
//!
//! - `TcpStream` -- connected TCP socket
//! - `TcpListener` -- listening TCP socket
//! - `UdpSocket` -- UDP datagram socket
//! - `SocketAddr` / `IpAddr` / `Ipv4Addr` -- address types
//!
//! Syscall mappings:
//! - `socket`      -> SYS_SOCKET_CREATE (220)
//! - `bind`        -> SYS_SOCKET_BIND (221)
//! - `listen`      -> SYS_SOCKET_LISTEN (222)
//! - `connect`     -> SYS_SOCKET_CONNECT (223)
//! - `accept`      -> SYS_SOCKET_ACCEPT (224)
//! - `send`        -> SYS_SOCKET_SEND (225)
//! - `recv`        -> SYS_SOCKET_RECV (226)
//! - `close`       -> SYS_SOCKET_CLOSE (227)
//! - `sendto`      -> SYS_NET_SENDTO (250)
//! - `recvfrom`    -> SYS_NET_RECVFROM (251)
//! - `getsockname` -> SYS_NET_GETSOCKNAME (252)
//! - `getpeername` -> SYS_NET_GETPEERNAME (253)
//! - `setsockopt`  -> SYS_NET_SETSOCKOPT (254)
//! - `getsockopt`  -> SYS_NET_GETSOCKOPT (255)

extern crate alloc;

use super::{
    fd::SharedFd, syscall1, syscall2, syscall3, syscall5, syscall_result, SyscallError,
    SYS_NET_GETPEERNAME, SYS_NET_GETSOCKNAME, SYS_NET_GETSOCKOPT, SYS_NET_RECVFROM, SYS_NET_SENDTO,
    SYS_NET_SETSOCKOPT, SYS_SOCKET_ACCEPT, SYS_SOCKET_BIND, SYS_SOCKET_CLOSE, SYS_SOCKET_CONNECT,
    SYS_SOCKET_CREATE, SYS_SOCKET_LISTEN, SYS_SOCKET_RECV, SYS_SOCKET_SEND,
};

// ============================================================================
// Socket constants
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

/// Shutdown: no more reads.
pub const SHUT_RD: usize = 0;
/// Shutdown: no more writes.
pub const SHUT_WR: usize = 1;
/// Shutdown: no more reads or writes.
pub const SHUT_RDWR: usize = 2;

/// Socket option level: socket.
pub const SOL_SOCKET: usize = 1;
/// Socket option level: TCP.
pub const IPPROTO_TCP: usize = 6;

/// Socket option: reuse address.
pub const SO_REUSEADDR: usize = 2;
/// Socket option: send buffer size.
pub const SO_SNDBUF: usize = 7;
/// Socket option: receive buffer size.
pub const SO_RCVBUF: usize = 8;
/// Socket option: keepalive.
pub const SO_KEEPALIVE: usize = 9;
/// Socket option: error.
pub const SO_ERROR: usize = 4;

/// TCP option: disable Nagle.
pub const TCP_NODELAY: usize = 1;

// ============================================================================
// Low-level syscall wrappers
// ============================================================================

/// Create a socket.
pub fn socket(domain: usize, sock_type: usize, _protocol: usize) -> Result<usize, SyscallError> {
    // VeridianOS SYS_SOCKET_CREATE takes (domain, sock_type).
    let ret = unsafe { syscall2(SYS_SOCKET_CREATE, domain, sock_type) };
    syscall_result(ret)
}

/// Bind a socket to an address.
pub fn bind(fd: usize, addr: *const u8, addrlen: usize) -> Result<usize, SyscallError> {
    let ret = unsafe { syscall3(SYS_SOCKET_BIND, fd, addr as usize, addrlen) };
    syscall_result(ret)
}

/// Listen on a socket.
pub fn listen(fd: usize, backlog: usize) -> Result<usize, SyscallError> {
    let ret = unsafe { syscall2(SYS_SOCKET_LISTEN, fd, backlog) };
    syscall_result(ret)
}

/// Accept a connection on a socket.
pub fn accept(fd: usize) -> Result<usize, SyscallError> {
    let ret = unsafe { syscall1(SYS_SOCKET_ACCEPT, fd) };
    syscall_result(ret)
}

/// Connect to a remote address.
pub fn connect(fd: usize, addr: *const u8, addrlen: usize) -> Result<usize, SyscallError> {
    let ret = unsafe { syscall3(SYS_SOCKET_CONNECT, fd, addr as usize, addrlen) };
    syscall_result(ret)
}

/// Send data on a connected socket.
pub fn send(fd: usize, buf: *const u8, len: usize, _flags: usize) -> Result<usize, SyscallError> {
    let ret = unsafe { syscall3(SYS_SOCKET_SEND, fd, buf as usize, len) };
    syscall_result(ret)
}

/// Receive data from a connected socket.
pub fn recv(fd: usize, buf: *mut u8, len: usize, _flags: usize) -> Result<usize, SyscallError> {
    let ret = unsafe { syscall3(SYS_SOCKET_RECV, fd, buf as usize, len) };
    syscall_result(ret)
}

/// Close a socket.
pub fn socket_close(fd: usize) -> Result<usize, SyscallError> {
    let ret = unsafe { syscall1(SYS_SOCKET_CLOSE, fd) };
    syscall_result(ret)
}

/// Send data to a specific address (UDP).
pub fn sendto(
    fd: usize,
    buf: *const u8,
    len: usize,
    addr: *const u8,
    addrlen: usize,
) -> Result<usize, SyscallError> {
    let ret = unsafe {
        syscall5(
            SYS_NET_SENDTO,
            fd,
            buf as usize,
            len,
            addr as usize,
            addrlen,
        )
    };
    syscall_result(ret)
}

/// Receive data and sender address (UDP).
pub fn recvfrom(fd: usize, buf: *mut u8, len: usize, addr: *mut u8) -> Result<usize, SyscallError> {
    let ret = unsafe { super::syscall4(SYS_NET_RECVFROM, fd, buf as usize, len, addr as usize) };
    syscall_result(ret)
}

/// Get local socket address.
pub fn getsockname(fd: usize, addr: *mut u8, addrlen: *mut usize) -> Result<usize, SyscallError> {
    let ret = unsafe { syscall3(SYS_NET_GETSOCKNAME, fd, addr as usize, addrlen as usize) };
    syscall_result(ret)
}

/// Get peer socket address.
pub fn getpeername(fd: usize, addr: *mut u8, addrlen: *mut usize) -> Result<usize, SyscallError> {
    let ret = unsafe { syscall3(SYS_NET_GETPEERNAME, fd, addr as usize, addrlen as usize) };
    syscall_result(ret)
}

/// Set a socket option.
pub fn setsockopt(
    fd: usize,
    level: usize,
    optname: usize,
    optval: *const u8,
    optlen: usize,
) -> Result<usize, SyscallError> {
    let ret = unsafe {
        syscall5(
            SYS_NET_SETSOCKOPT,
            fd,
            level,
            optname,
            optval as usize,
            optlen,
        )
    };
    syscall_result(ret)
}

/// Get a socket option.
pub fn getsockopt(
    fd: usize,
    level: usize,
    optname: usize,
    optval: *mut u8,
) -> Result<usize, SyscallError> {
    let ret = unsafe { super::syscall4(SYS_NET_GETSOCKOPT, fd, level, optname, optval as usize) };
    syscall_result(ret)
}

// ============================================================================
// Address types
// ============================================================================

/// An IPv4 address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Ipv4Addr {
    octets: [u8; 4],
}

impl Ipv4Addr {
    /// Create a new IPv4 address.
    pub const fn new(a: u8, b: u8, c: u8, d: u8) -> Self {
        Ipv4Addr {
            octets: [a, b, c, d],
        }
    }

    /// The loopback address (127.0.0.1).
    pub const LOCALHOST: Self = Self::new(127, 0, 0, 1);

    /// The unspecified address (0.0.0.0).
    pub const UNSPECIFIED: Self = Self::new(0, 0, 0, 0);

    /// The broadcast address (255.255.255.255).
    pub const BROADCAST: Self = Self::new(255, 255, 255, 255);

    /// Return the four octets.
    pub const fn octets(&self) -> [u8; 4] {
        self.octets
    }

    /// Is this the loopback address?
    pub fn is_loopback(&self) -> bool {
        self.octets[0] == 127
    }

    /// Is this the unspecified address?
    pub fn is_unspecified(&self) -> bool {
        self.octets == [0, 0, 0, 0]
    }

    /// Convert to network byte order (big-endian u32).
    pub fn to_be_u32(&self) -> u32 {
        u32::from_be_bytes(self.octets)
    }

    /// Create from network byte order u32.
    pub fn from_be_u32(val: u32) -> Self {
        Ipv4Addr {
            octets: val.to_be_bytes(),
        }
    }
}

impl core::fmt::Display for Ipv4Addr {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "{}.{}.{}.{}",
            self.octets[0], self.octets[1], self.octets[2], self.octets[3]
        )
    }
}

/// An IP address (v4 or v6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpAddr {
    V4(Ipv4Addr),
    // V6 not yet supported.
}

impl IpAddr {
    /// Create from an IPv4 address.
    pub const fn v4(a: u8, b: u8, c: u8, d: u8) -> Self {
        IpAddr::V4(Ipv4Addr::new(a, b, c, d))
    }
}

impl core::fmt::Display for IpAddr {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            IpAddr::V4(v4) => v4.fmt(f),
        }
    }
}

/// A socket address (IP + port).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SocketAddr {
    pub ip: IpAddr,
    pub port: u16,
}

impl SocketAddr {
    /// Create a new socket address.
    pub const fn new(ip: IpAddr, port: u16) -> Self {
        SocketAddr { ip, port }
    }

    /// Create from IPv4 components.
    pub const fn v4(a: u8, b: u8, c: u8, d: u8, port: u16) -> Self {
        SocketAddr {
            ip: IpAddr::V4(Ipv4Addr::new(a, b, c, d)),
            port,
        }
    }
}

impl core::fmt::Display for SocketAddr {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}:{}", self.ip, self.port)
    }
}

/// Kernel sockaddr_in structure (must match kernel layout).
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct SockaddrIn {
    pub sin_family: u16,
    pub sin_port: u16, // network byte order (big-endian)
    pub sin_addr: u32, // network byte order
    pub sin_zero: [u8; 8],
}

impl SockaddrIn {
    /// Create from a SocketAddr.
    pub fn from_socket_addr(addr: &SocketAddr) -> Self {
        let ip_be = match addr.ip {
            IpAddr::V4(v4) => v4.to_be_u32(),
        };
        SockaddrIn {
            sin_family: AF_INET as u16,
            sin_port: addr.port.to_be(),
            sin_addr: ip_be,
            sin_zero: [0; 8],
        }
    }

    /// Convert back to a SocketAddr.
    pub fn to_socket_addr(&self) -> SocketAddr {
        let ip = Ipv4Addr::from_be_u32(self.sin_addr);
        let port = u16::from_be(self.sin_port);
        SocketAddr::new(IpAddr::V4(ip), port)
    }
}

// ============================================================================
// TcpStream
// ============================================================================

/// A connected TCP socket.
pub struct TcpStream {
    fd: SharedFd,
}

impl TcpStream {
    /// Connect to a remote address.
    pub fn connect(addr: &SocketAddr) -> Result<Self, SyscallError> {
        let fd_num = socket(AF_INET, SOCK_STREAM, 0)?;
        let sa = SockaddrIn::from_socket_addr(addr);
        let result = connect(
            fd_num,
            &sa as *const SockaddrIn as *const u8,
            core::mem::size_of::<SockaddrIn>(),
        );
        if let Err(e) = result {
            let _ = socket_close(fd_num);
            return Err(e);
        }
        Ok(TcpStream {
            fd: unsafe { SharedFd::from_raw(fd_num) },
        })
    }

    /// Wrap an existing connected socket fd.
    ///
    /// # Safety
    /// The fd must be a valid, connected TCP socket.
    pub unsafe fn from_raw_fd(fd: usize) -> Self {
        TcpStream {
            fd: unsafe { SharedFd::from_raw(fd) },
        }
    }

    /// Read from the stream.
    pub fn read(&self, buf: &mut [u8]) -> Result<usize, SyscallError> {
        recv(self.fd.raw(), buf.as_mut_ptr(), buf.len(), 0)
    }

    /// Write to the stream.
    pub fn write(&self, data: &[u8]) -> Result<usize, SyscallError> {
        send(self.fd.raw(), data.as_ptr(), data.len(), 0)
    }

    /// Write all bytes, retrying on partial writes.
    pub fn write_all(&self, data: &[u8]) -> Result<(), SyscallError> {
        let mut written = 0;
        while written < data.len() {
            let n = self.write(&data[written..])?;
            if n == 0 {
                return Err(SyscallError::BrokenPipe);
            }
            written += n;
        }
        Ok(())
    }

    /// Shut down part or all of the connection.
    pub fn shutdown(&self, how: usize) -> Result<(), SyscallError> {
        // VeridianOS uses SYS_SOCKET_CLOSE for full shutdown.
        // For partial shutdown, we would need a dedicated syscall.
        // For now, treat SHUT_RDWR as close.
        if how == SHUT_RDWR {
            socket_close(self.fd.raw())?;
        }
        Ok(())
    }

    /// Get the peer's address.
    pub fn peer_addr(&self) -> Result<SocketAddr, SyscallError> {
        let mut sa = SockaddrIn::default();
        let mut len = core::mem::size_of::<SockaddrIn>();
        getpeername(
            self.fd.raw(),
            &mut sa as *mut SockaddrIn as *mut u8,
            &mut len,
        )?;
        Ok(sa.to_socket_addr())
    }

    /// Get the local address.
    pub fn local_addr(&self) -> Result<SocketAddr, SyscallError> {
        let mut sa = SockaddrIn::default();
        let mut len = core::mem::size_of::<SockaddrIn>();
        getsockname(
            self.fd.raw(),
            &mut sa as *mut SockaddrIn as *mut u8,
            &mut len,
        )?;
        Ok(sa.to_socket_addr())
    }

    /// Set TCP_NODELAY.
    pub fn set_nodelay(&self, nodelay: bool) -> Result<(), SyscallError> {
        let val: i32 = if nodelay { 1 } else { 0 };
        setsockopt(
            self.fd.raw(),
            IPPROTO_TCP,
            TCP_NODELAY,
            &val as *const i32 as *const u8,
            core::mem::size_of::<i32>(),
        )?;
        Ok(())
    }

    /// Set non-blocking mode.
    ///
    /// Currently a no-op placeholder -- requires fcntl or ioctl support.
    pub fn set_nonblocking(&self, _nonblocking: bool) -> Result<(), SyscallError> {
        // TODO(phase7): implement via fcntl(F_SETFL, O_NONBLOCK)
        Ok(())
    }

    /// Duplicate the stream.
    pub fn try_clone(&self) -> Result<TcpStream, SyscallError> {
        Ok(TcpStream {
            fd: self.fd.try_clone()?,
        })
    }

    /// Get the raw fd.
    pub fn raw_fd(&self) -> usize {
        self.fd.raw()
    }
}

impl core::fmt::Debug for TcpStream {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TcpStream").field("fd", &self.fd).finish()
    }
}

// ============================================================================
// TcpListener
// ============================================================================

/// A TCP socket listening for connections.
pub struct TcpListener {
    fd: SharedFd,
}

impl TcpListener {
    /// Create a listener bound to the given address.
    pub fn bind(addr: &SocketAddr) -> Result<Self, SyscallError> {
        let fd_num = socket(AF_INET, SOCK_STREAM, 0)?;

        // Set SO_REUSEADDR.
        let one: i32 = 1;
        let _ = setsockopt(
            fd_num,
            SOL_SOCKET,
            SO_REUSEADDR,
            &one as *const i32 as *const u8,
            core::mem::size_of::<i32>(),
        );

        let sa = SockaddrIn::from_socket_addr(addr);
        let result = bind(
            fd_num,
            &sa as *const SockaddrIn as *const u8,
            core::mem::size_of::<SockaddrIn>(),
        );
        if let Err(e) = result {
            let _ = socket_close(fd_num);
            return Err(e);
        }

        let result = listen(fd_num, 128);
        if let Err(e) = result {
            let _ = socket_close(fd_num);
            return Err(e);
        }

        Ok(TcpListener {
            fd: unsafe { SharedFd::from_raw(fd_num) },
        })
    }

    /// Accept a new connection, returning the stream and peer address.
    pub fn accept(&self) -> Result<(TcpStream, SocketAddr), SyscallError> {
        let new_fd = accept(self.fd.raw())?;
        let stream = TcpStream {
            fd: unsafe { SharedFd::from_raw(new_fd) },
        };
        let peer = stream.peer_addr().unwrap_or(SocketAddr::v4(0, 0, 0, 0, 0));
        Ok((stream, peer))
    }

    /// Get the local address this listener is bound to.
    pub fn local_addr(&self) -> Result<SocketAddr, SyscallError> {
        let mut sa = SockaddrIn::default();
        let mut len = core::mem::size_of::<SockaddrIn>();
        getsockname(
            self.fd.raw(),
            &mut sa as *mut SockaddrIn as *mut u8,
            &mut len,
        )?;
        Ok(sa.to_socket_addr())
    }

    /// Duplicate the listener.
    pub fn try_clone(&self) -> Result<TcpListener, SyscallError> {
        Ok(TcpListener {
            fd: self.fd.try_clone()?,
        })
    }

    /// Set non-blocking mode (placeholder).
    pub fn set_nonblocking(&self, _nonblocking: bool) -> Result<(), SyscallError> {
        Ok(())
    }

    /// Get the raw fd.
    pub fn raw_fd(&self) -> usize {
        self.fd.raw()
    }
}

impl core::fmt::Debug for TcpListener {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TcpListener").field("fd", &self.fd).finish()
    }
}

// ============================================================================
// UdpSocket
// ============================================================================

/// A UDP datagram socket.
pub struct UdpSocket {
    fd: SharedFd,
}

impl UdpSocket {
    /// Create a UDP socket bound to the given address.
    pub fn bind(addr: &SocketAddr) -> Result<Self, SyscallError> {
        let fd_num = socket(AF_INET, SOCK_DGRAM, 0)?;
        let sa = SockaddrIn::from_socket_addr(addr);
        let result = bind(
            fd_num,
            &sa as *const SockaddrIn as *const u8,
            core::mem::size_of::<SockaddrIn>(),
        );
        if let Err(e) = result {
            let _ = socket_close(fd_num);
            return Err(e);
        }
        Ok(UdpSocket {
            fd: unsafe { SharedFd::from_raw(fd_num) },
        })
    }

    /// Send data to a specific address.
    pub fn send_to(&self, buf: &[u8], addr: &SocketAddr) -> Result<usize, SyscallError> {
        let sa = SockaddrIn::from_socket_addr(addr);
        sendto(
            self.fd.raw(),
            buf.as_ptr(),
            buf.len(),
            &sa as *const SockaddrIn as *const u8,
            core::mem::size_of::<SockaddrIn>(),
        )
    }

    /// Receive data and the sender's address.
    pub fn recv_from(&self, buf: &mut [u8]) -> Result<(usize, SocketAddr), SyscallError> {
        let mut sa = SockaddrIn::default();
        let n = recvfrom(
            self.fd.raw(),
            buf.as_mut_ptr(),
            buf.len(),
            &mut sa as *mut SockaddrIn as *mut u8,
        )?;
        Ok((n, sa.to_socket_addr()))
    }

    /// Connect to a default destination (for `send` / `recv` without address).
    pub fn connect(&self, addr: &SocketAddr) -> Result<(), SyscallError> {
        let sa = SockaddrIn::from_socket_addr(addr);
        connect(
            self.fd.raw(),
            &sa as *const SockaddrIn as *const u8,
            core::mem::size_of::<SockaddrIn>(),
        )?;
        Ok(())
    }

    /// Send to the connected address.
    pub fn send(&self, buf: &[u8]) -> Result<usize, SyscallError> {
        send(self.fd.raw(), buf.as_ptr(), buf.len(), 0)
    }

    /// Receive from the connected address.
    pub fn recv(&self, buf: &mut [u8]) -> Result<usize, SyscallError> {
        recv(self.fd.raw(), buf.as_mut_ptr(), buf.len(), 0)
    }

    /// Get the local address.
    pub fn local_addr(&self) -> Result<SocketAddr, SyscallError> {
        let mut sa = SockaddrIn::default();
        let mut len = core::mem::size_of::<SockaddrIn>();
        getsockname(
            self.fd.raw(),
            &mut sa as *mut SockaddrIn as *mut u8,
            &mut len,
        )?;
        Ok(sa.to_socket_addr())
    }

    /// Set non-blocking mode (placeholder).
    pub fn set_nonblocking(&self, _nonblocking: bool) -> Result<(), SyscallError> {
        Ok(())
    }

    /// Duplicate the socket.
    pub fn try_clone(&self) -> Result<UdpSocket, SyscallError> {
        Ok(UdpSocket {
            fd: self.fd.try_clone()?,
        })
    }

    /// Get the raw fd.
    pub fn raw_fd(&self) -> usize {
        self.fd.raw()
    }
}

impl core::fmt::Debug for UdpSocket {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("UdpSocket").field("fd", &self.fd).finish()
    }
}
