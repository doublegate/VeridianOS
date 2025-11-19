//! Socket API implementation

use alloc::vec::Vec;

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
            | (SocketDomain::Inet, SocketType::Raw, _) => {}
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

        // TODO: Check if address is already in use (unless SO_REUSEADDR)

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

        // TODO: Create listening queue with backlog size

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
                IpAddress::V6(_) => {
                    return Err(KernelError::NotImplemented {
                        feature: "ipv6_auto_bind",
                    })
                }
            };
            self.bind(local_addr)?;
        }

        self.remote_addr = Some(addr);
        self.state = SocketState::Connected;

        // TODO: Actually initiate connection (TCP SYN, etc.)

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

        // TODO: Actually accept from listening queue

        Err(KernelError::WouldBlock)
    }

    /// Send data
    pub fn send(&self, data: &[u8], flags: u32) -> Result<usize, KernelError> {
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
                // TCP send
                // TODO: Actually send via TCP
                Ok(data.len())
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
    pub fn send_to(&self, data: &[u8], dest: SocketAddr, flags: u32) -> Result<usize, KernelError> {
        if self.socket_type != SocketType::Dgram {
            return Err(KernelError::InvalidArgument {
                name: "socket_type",
                value: "not_dgram",
            });
        }

        // TODO: Actually send via UDP
        Ok(data.len())
    }

    /// Receive data
    pub fn recv(&self, buffer: &mut [u8], flags: u32) -> Result<usize, KernelError> {
        if self.state != SocketState::Connected {
            return Err(KernelError::InvalidState {
                expected: "connected",
                actual: "not_connected",
            });
        }

        // TODO: Actually receive data
        Ok(0)
    }

    /// Receive data with source address
    pub fn recv_from(
        &self,
        buffer: &mut [u8],
        flags: u32,
    ) -> Result<(usize, SocketAddr), KernelError> {
        if self.state == SocketState::Unbound {
            return Err(KernelError::InvalidState {
                expected: "bound",
                actual: "unbound",
            });
        }

        // TODO: Actually receive data
        Ok((0, self.local_addr.unwrap()))
    }

    /// Close socket
    pub fn close(&mut self) -> Result<(), KernelError> {
        // TODO: Clean up resources, close connections

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
static mut SOCKET_TABLE: Option<Vec<Socket>> = None;
static mut NEXT_SOCKET_ID: usize = 1;

/// Initialize socket subsystem
pub fn init() -> Result<(), KernelError> {
    println!("[SOCKET] Initializing socket subsystem...");

    unsafe {
        SOCKET_TABLE = Some(Vec::new());
    }

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

    unsafe {
        let id = NEXT_SOCKET_ID;
        NEXT_SOCKET_ID += 1;

        socket.id = id;

        if let Some(ref mut table) = SOCKET_TABLE {
            table.push(socket);
            Ok(id)
        } else {
            Err(KernelError::InvalidState {
                expected: "initialized",
                actual: "not_initialized",
            })
        }
    }
}

/// Get socket by ID
pub fn get_socket(id: usize) -> Result<&'static Socket, KernelError> {
    unsafe {
        if let Some(ref table) = SOCKET_TABLE {
            table
                .iter()
                .find(|s| s.id == id)
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
}

/// Get mutable socket by ID
pub fn get_socket_mut(id: usize) -> Result<&'static mut Socket, KernelError> {
    unsafe {
        if let Some(ref mut table) = SOCKET_TABLE {
            table
                .iter_mut()
                .find(|s| s.id == id)
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::Ipv4Address;

    #[test_case]
    fn test_socket_creation() {
        let socket =
            Socket::new(SocketDomain::Inet, SocketType::Stream, SocketProtocol::Tcp).unwrap();
        assert_eq!(socket.state, SocketState::Unbound);
        assert_eq!(socket.socket_type, SocketType::Stream);
    }

    #[test_case]
    fn test_socket_bind() {
        let mut socket =
            Socket::new(SocketDomain::Inet, SocketType::Stream, SocketProtocol::Tcp).unwrap();
        let addr = SocketAddr::v4(Ipv4Address::LOCALHOST, 8080);

        assert_eq!(socket.state, SocketState::Unbound);
        socket.bind(addr).unwrap();
        assert_eq!(socket.state, SocketState::Bound);
        assert_eq!(socket.local_addr, Some(addr));
    }
}
