//! Socket API implementation
//!
//! Provides BSD-style socket interface for TCP and UDP communication.
//! Supports stream (TCP) and datagram (UDP) sockets with standard
//! bind, listen, accept, connect, send, and receive operations.

#![allow(clippy::manual_clamp)]

use alloc::{collections::VecDeque, vec::Vec};
use core::sync::atomic::{AtomicUsize, Ordering};

use spin::Mutex;

use super::{IpAddress, SocketAddr};
use crate::error::KernelError;

/// Socket domain (address family)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocketDomain {
    /// IPv4 Internet protocols
    Inet,
    /// IPv6 Internet protocols
    Inet6,
    /// Unix domain sockets
    Unix,
}

/// Socket type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocketType {
    /// Stream socket (TCP)
    Stream,
    /// Datagram socket (UDP)
    Dgram,
    /// Raw socket
    Raw,
}

/// Socket protocol
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocketProtocol {
    /// Default protocol for socket type
    Default,
    /// TCP
    Tcp,
    /// UDP
    Udp,
    /// ICMP
    Icmp,
}

/// Socket state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocketState {
    Unbound,
    Bound,
    Listening,
    Connected,
    Closed,
}

/// Socket options
#[derive(Debug, Clone, Copy)]
pub struct SocketOptions {
    pub reuse_addr: bool,
    pub reuse_port: bool,
    pub broadcast: bool,
    pub keepalive: bool,
    pub recv_buffer_size: usize,
    pub send_buffer_size: usize,
    pub recv_timeout_ms: Option<u64>,
    pub send_timeout_ms: Option<u64>,
}

impl Default for SocketOptions {
    fn default() -> Self {
        Self {
            reuse_addr: false,
            reuse_port: false,
            broadcast: false,
            keepalive: false,
            recv_buffer_size: 65536,
            send_buffer_size: 65536,
            recv_timeout_ms: None,
            send_timeout_ms: None,
        }
    }
}

/// Incoming connection for accept queue
#[derive(Debug, Clone)]
pub struct PendingConnection {
    /// Remote address
    pub remote_addr: SocketAddr,
    /// Connection sequence number
    pub seq_num: u32,
}

/// Generic socket handle
#[derive(Debug, Clone)]
pub struct Socket {
    pub id: usize,
    pub domain: SocketDomain,
    pub socket_type: SocketType,
    pub protocol: SocketProtocol,
    pub state: SocketState,
    pub local_addr: Option<SocketAddr>,
    pub remote_addr: Option<SocketAddr>,
    pub options: SocketOptions,
    /// Receive buffer
    recv_buffer: Vec<u8>,
    /// Send buffer
    send_buffer: Vec<u8>,
    /// Listen backlog (for listening sockets)
    backlog: usize,
}

impl Socket {
    /// Create a new socket
    pub fn new(
        domain: SocketDomain,
        socket_type: SocketType,
        protocol: SocketProtocol,
    ) -> Result<Self, KernelError> {
        // Validate domain/type/protocol combination
        match (domain, socket_type, protocol) {
            (SocketDomain::Inet, SocketType::Stream, SocketProtocol::Tcp)
            | (SocketDomain::Inet, SocketType::Stream, SocketProtocol::Default)
            | (SocketDomain::Inet, SocketType::Dgram, SocketProtocol::Udp)
            | (SocketDomain::Inet, SocketType::Dgram, SocketProtocol::Default)
            | (SocketDomain::Inet, SocketType::Raw, _)
            | (SocketDomain::Inet6, SocketType::Stream, SocketProtocol::Tcp)
            | (SocketDomain::Inet6, SocketType::Stream, SocketProtocol::Default)
            | (SocketDomain::Inet6, SocketType::Dgram, SocketProtocol::Udp)
            | (SocketDomain::Inet6, SocketType::Dgram, SocketProtocol::Default)
            | (SocketDomain::Inet6, SocketType::Raw, _) => {}
            _ => {
                return Err(KernelError::InvalidArgument {
                    name: "socket_combination",
                    value: "unsupported",
                })
            }
        }

        Ok(Self {
            id: 0, // Will be assigned by socket table
            domain,
            socket_type,
            protocol,
            state: SocketState::Unbound,
            local_addr: None,
            remote_addr: None,
            options: SocketOptions::default(),
            recv_buffer: Vec::new(),
            send_buffer: Vec::new(),
            backlog: 0,
        })
    }

    /// Bind socket to local address
    pub fn bind(&mut self, addr: SocketAddr) -> Result<(), KernelError> {
        if self.state != SocketState::Unbound {
            return Err(KernelError::InvalidState {
                expected: "unbound",
                actual: "already_bound",
            });
        }

        // Check if address is already in use (unless SO_REUSEADDR)
        if !self.options.reuse_addr && is_address_in_use(&addr) {
            return Err(KernelError::ResourceExhausted {
                resource: "socket_address",
            });
        }

        self.local_addr = Some(addr);
        self.state = SocketState::Bound;
        Ok(())
    }

    /// Listen for connections (TCP only)
    pub fn listen(&mut self, backlog: usize) -> Result<(), KernelError> {
        if self.socket_type != SocketType::Stream {
            return Err(KernelError::InvalidArgument {
                name: "socket_type",
                value: "not_stream",
            });
        }

        if self.state != SocketState::Bound {
            return Err(KernelError::InvalidState {
                expected: "bound",
                actual: "not_bound",
            });
        }

        // Create listening queue with backlog size
        self.backlog = backlog.max(1).min(128); // Clamp between 1 and 128

        // Register in the listen queue manager
        if let Some(addr) = self.local_addr {
            register_listening_socket(self.id, addr, self.backlog);
        }

        self.state = SocketState::Listening;
        Ok(())
    }

    /// Connect to remote address
    pub fn connect(&mut self, addr: SocketAddr) -> Result<(), KernelError> {
        match self.state {
            SocketState::Unbound | SocketState::Bound => {}
            _ => {
                return Err(KernelError::InvalidState {
                    expected: "unbound_or_bound",
                    actual: "other",
                })
            }
        }

        // Auto-bind if not bound
        if self.state == SocketState::Unbound {
            let local_addr = match addr.ip() {
                IpAddress::V4(_) => SocketAddr::v4(super::Ipv4Address::UNSPECIFIED, 0),
                IpAddress::V6(_) => SocketAddr::v6(super::Ipv6Address::UNSPECIFIED, 0),
            };
            self.bind(local_addr)?;
        }

        self.remote_addr = Some(addr);

        // Initiate connection based on socket type
        match self.socket_type {
            SocketType::Stream => {
                // TCP connection - initiate 3-way handshake
                // In a full implementation, this would send SYN and wait for SYN-ACK
                // For now, we simulate immediate connection (loopback/local)
                self.state = SocketState::Connected;

                // Allocate receive buffer according to options
                self.recv_buffer.reserve(self.options.recv_buffer_size);
                self.send_buffer.reserve(self.options.send_buffer_size);
            }
            SocketType::Dgram => {
                // UDP is connectionless, just record the default destination
                self.state = SocketState::Connected;
            }
            SocketType::Raw => {
                self.state = SocketState::Connected;
            }
        }

        Ok(())
    }

    /// Accept incoming connection (TCP only)
    pub fn accept(&self) -> Result<(Socket, SocketAddr), KernelError> {
        if self.socket_type != SocketType::Stream {
            return Err(KernelError::InvalidArgument {
                name: "socket_type",
                value: "not_stream",
            });
        }

        if self.state != SocketState::Listening {
            return Err(KernelError::InvalidState {
                expected: "listening",
                actual: "not_listening",
            });
        }

        // Try to get a pending connection from the listen queue
        if let Some(pending) = dequeue_pending_connection(self.id) {
            // Create a new socket for the accepted connection
            let mut new_socket = Socket::new(self.domain, self.socket_type, self.protocol)?;
            new_socket.local_addr = self.local_addr;
            new_socket.remote_addr = Some(pending.remote_addr);
            new_socket.state = SocketState::Connected;
            new_socket
                .recv_buffer
                .reserve(self.options.recv_buffer_size);
            new_socket
                .send_buffer
                .reserve(self.options.send_buffer_size);
            new_socket.options = self.options;

            Ok((new_socket, pending.remote_addr))
        } else {
            // No pending connections
            Err(KernelError::WouldBlock)
        }
    }

    /// Send data
    pub fn send(&mut self, data: &[u8], _flags: u32) -> Result<usize, KernelError> {
        if self.state != SocketState::Connected {
            return Err(KernelError::InvalidState {
                expected: "connected",
                actual: "not_connected",
            });
        }

        let remote = self.remote_addr.ok_or(KernelError::InvalidState {
            expected: "remote_addr_set",
            actual: "no_remote_addr",
        })?;

        match self.socket_type {
            SocketType::Stream => {
                // TCP send - buffer the data for transmission
                let send_len = data
                    .len()
                    .min(self.options.send_buffer_size - self.send_buffer.len());
                if send_len == 0 && !data.is_empty() {
                    return Err(KernelError::WouldBlock);
                }
                self.send_buffer.extend_from_slice(&data[..send_len]);

                // Signal TCP layer to transmit buffered data
                super::tcp::transmit_data(self.id, &self.send_buffer, remote);
                let sent = self.send_buffer.len();
                self.send_buffer.clear();

                Ok(sent)
            }
            SocketType::Dgram => {
                // UDP send
                super::udp::UdpSocket::new().send_to(data, remote)
            }
            SocketType::Raw => Err(KernelError::NotImplemented {
                feature: "raw_socket_send",
            }),
        }
    }

    /// Send data to specific address (UDP)
    pub fn send_to(
        &self,
        data: &[u8],
        dest: SocketAddr,
        _flags: u32,
    ) -> Result<usize, KernelError> {
        if self.socket_type != SocketType::Dgram {
            return Err(KernelError::InvalidArgument {
                name: "socket_type",
                value: "not_dgram",
            });
        }

        // Send via UDP
        super::udp::UdpSocket::new().send_to(data, dest)
    }

    /// Receive data
    pub fn recv(&mut self, buffer: &mut [u8], _flags: u32) -> Result<usize, KernelError> {
        if self.state != SocketState::Connected {
            return Err(KernelError::InvalidState {
                expected: "connected",
                actual: "not_connected",
            });
        }

        // Check if we have data in the receive buffer
        if self.recv_buffer.is_empty() {
            // Try to receive from the network layer
            let received = super::tcp::receive_data(self.id, &mut self.recv_buffer);
            if received == 0 {
                return Err(KernelError::WouldBlock);
            }
        }

        // Copy data to user buffer
        let copy_len = buffer.len().min(self.recv_buffer.len());
        buffer[..copy_len].copy_from_slice(&self.recv_buffer[..copy_len]);
        self.recv_buffer.drain(..copy_len);

        Ok(copy_len)
    }

    /// Receive data with source address
    pub fn recv_from(
        &mut self,
        buffer: &mut [u8],
        _flags: u32,
    ) -> Result<(usize, SocketAddr), KernelError> {
        if self.state == SocketState::Unbound {
            return Err(KernelError::InvalidState {
                expected: "bound",
                actual: "unbound",
            });
        }

        // For connected sockets, use the remote address
        if let Some(remote) = self.remote_addr {
            let len = self.recv(buffer, _flags)?;
            return Ok((len, remote));
        }

        // For UDP, receive with source address
        if self.socket_type == SocketType::Dgram {
            let (len, from_addr) = super::udp::receive_from(self.id, buffer)?;
            return Ok((len, from_addr));
        }

        Err(KernelError::WouldBlock)
    }

    /// Close socket
    pub fn close(&mut self) -> Result<(), KernelError> {
        // Clean up based on socket state
        match self.state {
            SocketState::Connected => {
                if self.socket_type == SocketType::Stream {
                    // Initiate TCP close sequence (FIN)
                    super::tcp::close_connection(self.id);
                }
            }
            SocketState::Listening => {
                // Unregister from listen queue
                unregister_listening_socket(self.id);
            }
            _ => {}
        }

        // Clear buffers
        self.recv_buffer.clear();
        self.send_buffer.clear();

        self.state = SocketState::Closed;
        Ok(())
    }

    /// Set socket option
    pub fn set_option(&mut self, option: SocketOption) -> Result<(), KernelError> {
        match option {
            SocketOption::ReuseAddr(val) => self.options.reuse_addr = val,
            SocketOption::ReusePort(val) => self.options.reuse_port = val,
            SocketOption::Broadcast(val) => self.options.broadcast = val,
            SocketOption::KeepAlive(val) => self.options.keepalive = val,
            SocketOption::RecvBufferSize(val) => self.options.recv_buffer_size = val,
            SocketOption::SendBufferSize(val) => self.options.send_buffer_size = val,
            SocketOption::RecvTimeout(val) => self.options.recv_timeout_ms = val,
            SocketOption::SendTimeout(val) => self.options.send_timeout_ms = val,
        }
        Ok(())
    }
}

/// Socket option values
#[derive(Debug, Clone)]
pub enum SocketOption {
    ReuseAddr(bool),
    ReusePort(bool),
    Broadcast(bool),
    KeepAlive(bool),
    RecvBufferSize(usize),
    SendBufferSize(usize),
    RecvTimeout(Option<u64>),
    SendTimeout(Option<u64>),
}

/// Socket table for managing all sockets
static SOCKET_TABLE: Mutex<Option<Vec<Socket>>> = Mutex::new(None);
static NEXT_SOCKET_ID: AtomicUsize = AtomicUsize::new(1);

/// Initialize socket subsystem
pub fn init() -> Result<(), KernelError> {
    println!("[SOCKET] Initializing socket subsystem...");

    let mut table = SOCKET_TABLE.lock();
    *table = Some(Vec::new());

    println!("[SOCKET] Socket subsystem initialized");
    Ok(())
}

/// Create a new socket and return its ID
pub fn create_socket(
    domain: SocketDomain,
    socket_type: SocketType,
    protocol: SocketProtocol,
) -> Result<usize, KernelError> {
    let mut socket = Socket::new(domain, socket_type, protocol)?;

    let id = NEXT_SOCKET_ID.fetch_add(1, Ordering::Relaxed);
    socket.id = id;

    let mut table = SOCKET_TABLE.lock();
    if let Some(ref mut sockets) = *table {
        sockets.push(socket);
        Ok(id)
    } else {
        Err(KernelError::InvalidState {
            expected: "initialized",
            actual: "not_initialized",
        })
    }
}

/// Execute a closure with a socket by ID (immutable access)
pub fn with_socket<R, F: FnOnce(&Socket) -> R>(id: usize, f: F) -> Result<R, KernelError> {
    let table = SOCKET_TABLE.lock();
    if let Some(ref sockets) = *table {
        sockets
            .iter()
            .find(|s| s.id == id)
            .map(f)
            .ok_or(KernelError::InvalidArgument {
                name: "socket_id",
                value: "not_found",
            })
    } else {
        Err(KernelError::InvalidState {
            expected: "initialized",
            actual: "not_initialized",
        })
    }
}

/// Execute a closure with a socket by ID (mutable access)
pub fn with_socket_mut<R, F: FnOnce(&mut Socket) -> R>(id: usize, f: F) -> Result<R, KernelError> {
    let mut table = SOCKET_TABLE.lock();
    if let Some(ref mut sockets) = *table {
        sockets
            .iter_mut()
            .find(|s| s.id == id)
            .map(f)
            .ok_or(KernelError::InvalidArgument {
                name: "socket_id",
                value: "not_found",
            })
    } else {
        Err(KernelError::InvalidState {
            expected: "initialized",
            actual: "not_initialized",
        })
    }
}

/// Listening socket registry for tracking bound addresses and accept queues
struct ListeningSocketEntry {
    socket_id: usize,
    addr: SocketAddr,
    backlog: usize,
    pending_connections: VecDeque<PendingConnection>,
}

static LISTENING_SOCKETS: Mutex<Vec<ListeningSocketEntry>> = Mutex::new(Vec::new());
static BOUND_ADDRESSES: Mutex<Vec<SocketAddr>> = Mutex::new(Vec::new());

/// Check if an address is already in use
fn is_address_in_use(addr: &SocketAddr) -> bool {
    let bound = BOUND_ADDRESSES.lock();
    bound.iter().any(|a| a == addr)
}

/// Register a listening socket in the global registry
fn register_listening_socket(socket_id: usize, addr: SocketAddr, backlog: usize) {
    // Add to bound addresses
    {
        let mut bound = BOUND_ADDRESSES.lock();
        if !bound.iter().any(|a| a == &addr) {
            bound.push(addr);
        }
    }

    // Add to listening sockets
    let mut listeners = LISTENING_SOCKETS.lock();
    listeners.push(ListeningSocketEntry {
        socket_id,
        addr,
        backlog,
        pending_connections: VecDeque::with_capacity(backlog),
    });
}

/// Unregister a listening socket
fn unregister_listening_socket(socket_id: usize) {
    let mut listeners = LISTENING_SOCKETS.lock();
    if let Some(pos) = listeners.iter().position(|e| e.socket_id == socket_id) {
        let entry = listeners.remove(pos);

        // Remove from bound addresses
        let mut bound = BOUND_ADDRESSES.lock();
        if let Some(pos) = bound.iter().position(|a| a == &entry.addr) {
            bound.remove(pos);
        }
    }
}

/// Dequeue a pending connection from a listening socket
fn dequeue_pending_connection(socket_id: usize) -> Option<PendingConnection> {
    let mut listeners = LISTENING_SOCKETS.lock();
    for entry in listeners.iter_mut() {
        if entry.socket_id == socket_id {
            return entry.pending_connections.pop_front();
        }
    }
    None
}

/// Queue a new connection to a listening socket (called by TCP layer)
pub fn queue_pending_connection(
    addr: SocketAddr,
    remote: SocketAddr,
    seq_num: u32,
) -> Result<(), KernelError> {
    let mut listeners = LISTENING_SOCKETS.lock();
    for entry in listeners.iter_mut() {
        if entry.addr == addr {
            if entry.pending_connections.len() < entry.backlog {
                entry.pending_connections.push_back(PendingConnection {
                    remote_addr: remote,
                    seq_num,
                });
                return Ok(());
            } else {
                return Err(KernelError::ResourceExhausted {
                    resource: "listen_backlog",
                });
            }
        }
    }
    Err(KernelError::InvalidArgument {
        name: "listen_addr",
        value: "not_listening",
    })
}

/// Close a socket by ID
pub fn close_socket(id: usize) -> Result<(), KernelError> {
    with_socket_mut(id, |socket| socket.close())?
}

// -----------------------------------------------------------------------
// Free-function wrappers for syscall layer (Phase 6 network extensions)
// -----------------------------------------------------------------------

/// Send data to a specific address (for UDP sockets).
pub fn sendto(
    id: usize,
    data: &[u8],
    dest: Option<&crate::net::SocketAddr>,
) -> Result<usize, KernelError> {
    with_socket_mut(id, |socket| {
        if let Some(addr) = dest {
            socket.send_to(data, *addr, 0)
        } else {
            socket.send(data, 0)
        }
    })?
}

/// Receive data with sender address.
pub fn recvfrom(
    id: usize,
    buf: &mut [u8],
) -> Result<(usize, Option<crate::net::SocketAddr>), KernelError> {
    let result = with_socket_mut(id, |socket| socket.recv_from(buf, 0))??;
    Ok((result.0, Some(result.1)))
}

/// Get the local address of a socket.
pub fn getsockname(id: usize) -> Result<crate::net::SocketAddr, KernelError> {
    with_socket(id, |socket| {
        socket.local_addr.ok_or(KernelError::InvalidState {
            expected: "bound socket",
            actual: "unbound",
        })
    })?
}

/// Get the remote address of a connected socket.
pub fn getpeername(id: usize) -> Result<crate::net::SocketAddr, KernelError> {
    with_socket(id, |socket| {
        socket.remote_addr.ok_or(KernelError::InvalidState {
            expected: "connected socket",
            actual: "not connected",
        })
    })?
}

/// Set a socket option.
pub fn setsockopt(
    _id: usize,
    _level: i32,
    _optname: i32,
    _optval_ptr: usize,
    _optlen: usize,
) -> Result<usize, KernelError> {
    // Stub: accept all options without error
    Ok(0)
}

/// Get a socket option.
pub fn getsockopt(
    _id: usize,
    _level: i32,
    _optname: i32,
    _optval_ptr: usize,
) -> Result<usize, KernelError> {
    // Stub: return 0 for all options
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::{Ipv4Address, Ipv6Address};

    #[test]
    fn test_socket_creation() {
        let socket =
            Socket::new(SocketDomain::Inet, SocketType::Stream, SocketProtocol::Tcp).unwrap();
        assert_eq!(socket.state, SocketState::Unbound);
        assert_eq!(socket.socket_type, SocketType::Stream);
    }

    #[test]
    fn test_socket_creation_inet6() {
        let socket =
            Socket::new(SocketDomain::Inet6, SocketType::Stream, SocketProtocol::Tcp).unwrap();
        assert_eq!(socket.state, SocketState::Unbound);
        assert_eq!(socket.domain, SocketDomain::Inet6);
        assert_eq!(socket.socket_type, SocketType::Stream);
    }

    #[test]
    fn test_socket_creation_inet6_udp() {
        let socket =
            Socket::new(SocketDomain::Inet6, SocketType::Dgram, SocketProtocol::Udp).unwrap();
        assert_eq!(socket.domain, SocketDomain::Inet6);
        assert_eq!(socket.socket_type, SocketType::Dgram);
    }

    #[test]
    fn test_socket_bind() {
        let mut socket =
            Socket::new(SocketDomain::Inet, SocketType::Stream, SocketProtocol::Tcp).unwrap();
        let addr = SocketAddr::v4(Ipv4Address::LOCALHOST, 8080);

        assert_eq!(socket.state, SocketState::Unbound);
        socket.bind(addr).unwrap();
        assert_eq!(socket.state, SocketState::Bound);
        assert_eq!(socket.local_addr, Some(addr));
    }

    #[test]
    fn test_socket_bind_inet6() {
        let mut socket =
            Socket::new(SocketDomain::Inet6, SocketType::Stream, SocketProtocol::Tcp).unwrap();
        let addr = SocketAddr::v6(Ipv6Address::LOCALHOST, 8080);

        assert_eq!(socket.state, SocketState::Unbound);
        socket.bind(addr).unwrap();
        assert_eq!(socket.state, SocketState::Bound);
        assert_eq!(socket.local_addr, Some(addr));
    }
}
