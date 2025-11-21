//! UDP protocol implementation

use alloc::{collections::BTreeMap, vec::Vec};

use spin::Mutex;

use super::{IpAddress, SocketAddr};
use crate::error::KernelError;

/// UDP header
#[derive(Debug, Clone)]
pub struct UdpHeader {
    pub source_port: u16,
    pub dest_port: u16,
    pub length: u16,
    pub checksum: u16,
}

impl UdpHeader {
    pub const SIZE: usize = 8;

    pub fn new(src_port: u16, dst_port: u16, data_len: usize) -> Self {
        Self {
            source_port: src_port,
            dest_port: dst_port,
            length: (Self::SIZE + data_len) as u16,
            checksum: 0,
        }
    }

    pub fn to_bytes(&self) -> [u8; 8] {
        let mut bytes = [0u8; 8];
        bytes[0..2].copy_from_slice(&self.source_port.to_be_bytes());
        bytes[2..4].copy_from_slice(&self.dest_port.to_be_bytes());
        bytes[4..6].copy_from_slice(&self.length.to_be_bytes());
        bytes[6..8].copy_from_slice(&self.checksum.to_be_bytes());
        bytes
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, KernelError> {
        if bytes.len() < Self::SIZE {
            return Err(KernelError::InvalidArgument {
                name: "udp_header",
                value: "too_short",
            });
        }

        Ok(Self {
            source_port: u16::from_be_bytes([bytes[0], bytes[1]]),
            dest_port: u16::from_be_bytes([bytes[2], bytes[3]]),
            length: u16::from_be_bytes([bytes[4], bytes[5]]),
            checksum: u16::from_be_bytes([bytes[6], bytes[7]]),
        })
    }

    /// Calculate UDP checksum
    pub fn calculate_checksum(&mut self, src: IpAddress, dst: IpAddress, data: &[u8]) {
        self.checksum = 0;

        // UDP checksum includes pseudo-header
        let mut sum: u32 = 0;

        // Add pseudo-header (source IP, dest IP, protocol, length)
        if let (IpAddress::V4(src_v4), IpAddress::V4(dst_v4)) = (src, dst) {
            sum += u16::from_be_bytes([src_v4.0[0], src_v4.0[1]]) as u32;
            sum += u16::from_be_bytes([src_v4.0[2], src_v4.0[3]]) as u32;
            sum += u16::from_be_bytes([dst_v4.0[0], dst_v4.0[1]]) as u32;
            sum += u16::from_be_bytes([dst_v4.0[2], dst_v4.0[3]]) as u32;
            sum += 17u32; // Protocol (UDP)
            sum += self.length as u32;
        }

        // Add UDP header
        let header_bytes = self.to_bytes();
        for i in 0..4 {
            sum += u16::from_be_bytes([header_bytes[i * 2], header_bytes[i * 2 + 1]]) as u32;
        }

        // Add data
        for chunk in data.chunks(2) {
            if chunk.len() == 2 {
                sum += u16::from_be_bytes([chunk[0], chunk[1]]) as u32;
            } else {
                sum += (chunk[0] as u32) << 8;
            }
        }

        // Fold 32-bit sum to 16 bits
        while sum >> 16 != 0 {
            sum = (sum & 0xFFFF) + (sum >> 16);
        }

        self.checksum = !(sum as u16);
    }
}

/// UDP socket
#[derive(Debug, Clone)]
pub struct UdpSocket {
    pub local: SocketAddr,
    pub remote: Option<SocketAddr>,
    pub bound: bool,
}

impl UdpSocket {
    pub fn new() -> Self {
        Self {
            local: SocketAddr::v4(super::Ipv4Address::UNSPECIFIED, 0),
            remote: None,
            bound: false,
        }
    }
}

impl Default for UdpSocket {
    fn default() -> Self {
        Self::new()
    }
}

impl UdpSocket {
    /// Bind to local address
    pub fn bind(&mut self, addr: SocketAddr) -> Result<(), KernelError> {
        if self.bound {
            return Err(KernelError::InvalidState {
                expected: "unbound",
                actual: "bound",
            });
        }

        self.local = addr;
        self.bound = true;
        Ok(())
    }

    /// Connect to remote address (optional for UDP)
    pub fn connect(&mut self, addr: SocketAddr) -> Result<(), KernelError> {
        if !self.bound {
            return Err(KernelError::InvalidState {
                expected: "bound",
                actual: "unbound",
            });
        }

        self.remote = Some(addr);
        Ok(())
    }

    /// Send data to specific address
    pub fn send_to(&self, data: &[u8], dest: SocketAddr) -> Result<usize, KernelError> {
        if !self.bound {
            return Err(KernelError::InvalidState {
                expected: "bound",
                actual: "unbound",
            });
        }

        // Create UDP header
        let src_port = self.local.port();
        let dst_port = dest.port();
        let mut header = UdpHeader::new(src_port, dst_port, data.len());

        // Calculate checksum
        header.calculate_checksum(self.local.ip(), dest.ip(), data);

        // Send via IP layer
        super::ip::send(dest.ip(), super::ip::IpProtocol::Udp, data)?;

        Ok(data.len())
    }

    /// Send data to connected address
    pub fn send(&self, data: &[u8]) -> Result<usize, KernelError> {
        if let Some(remote) = self.remote {
            self.send_to(data, remote)
        } else {
            Err(KernelError::InvalidState {
                expected: "connected",
                actual: "not_connected",
            })
        }
    }

    /// Receive data
    pub fn recv_from(&self, _buffer: &mut [u8]) -> Result<(usize, SocketAddr), KernelError> {
        if !self.bound {
            return Err(KernelError::InvalidState {
                expected: "bound",
                actual: "unbound",
            });
        }

        // TODO: Actually receive data from network stack
        // For now, return empty result
        Ok((0, self.local))
    }

    /// Receive data (from connected address)
    pub fn recv(&self, buffer: &mut [u8]) -> Result<usize, KernelError> {
        let (len, _) = self.recv_from(buffer)?;
        Ok(len)
    }
}

/// Initialize UDP
pub fn init() -> Result<(), KernelError> {
    println!("[UDP] Initializing UDP protocol...");
    println!("[UDP] UDP initialized");
    Ok(())
}

// ============================================================================
// Socket Layer Interface
// ============================================================================

/// Received UDP datagram with source address
struct ReceivedDatagram {
    data: Vec<u8>,
    from: SocketAddr,
}

/// UDP receive buffer per socket
struct UdpSocketBuffer {
    local_addr: SocketAddr,
    recv_queue: Vec<ReceivedDatagram>,
    max_queue_size: usize,
}

/// Global UDP socket buffers
static UDP_SOCKETS: Mutex<BTreeMap<usize, UdpSocketBuffer>> = Mutex::new(BTreeMap::new());

/// Register a UDP socket for receiving
pub fn register_socket(socket_id: usize, local_addr: SocketAddr) {
    let mut sockets = UDP_SOCKETS.lock();
    sockets.insert(
        socket_id,
        UdpSocketBuffer {
            local_addr,
            recv_queue: Vec::new(),
            max_queue_size: 64,
        },
    );
}

/// Unregister a UDP socket
pub fn unregister_socket(socket_id: usize) {
    let mut sockets = UDP_SOCKETS.lock();
    sockets.remove(&socket_id);
}

/// Receive data from a UDP socket (called by socket layer)
pub fn receive_from(
    socket_id: usize,
    buffer: &mut [u8],
) -> Result<(usize, SocketAddr), KernelError> {
    let mut sockets = UDP_SOCKETS.lock();

    if let Some(sock_buf) = sockets.get_mut(&socket_id) {
        if let Some(datagram) = sock_buf.recv_queue.pop() {
            let copy_len = buffer.len().min(datagram.data.len());
            buffer[..copy_len].copy_from_slice(&datagram.data[..copy_len]);
            return Ok((copy_len, datagram.from));
        }
        return Err(KernelError::WouldBlock);
    }

    Err(KernelError::InvalidArgument {
        name: "socket_id",
        value: "not_found",
    })
}

/// Process incoming UDP packet (called by IP layer)
pub fn process_packet(
    src_addr: IpAddress,
    dst_addr: IpAddress,
    data: &[u8],
) -> Result<(), KernelError> {
    if data.len() < UdpHeader::SIZE {
        return Err(KernelError::InvalidArgument {
            name: "udp_packet",
            value: "too_short",
        });
    }

    // Parse UDP header
    let header = UdpHeader::from_bytes(data)?;

    // Validate length
    if data.len() < header.length as usize {
        return Err(KernelError::InvalidArgument {
            name: "udp_length",
            value: "mismatch",
        });
    }

    // Extract payload
    let payload = &data[UdpHeader::SIZE..header.length as usize];
    let src = SocketAddr::new(src_addr, header.source_port);
    let _dst = SocketAddr::new(dst_addr, header.dest_port);

    // Find matching socket by destination port
    let mut sockets = UDP_SOCKETS.lock();
    for (_socket_id, sock_buf) in sockets.iter_mut() {
        if sock_buf.local_addr.port() == header.dest_port || sock_buf.local_addr.port() == 0 {
            // Check queue size
            if sock_buf.recv_queue.len() < sock_buf.max_queue_size {
                sock_buf.recv_queue.push(ReceivedDatagram {
                    data: payload.to_vec(),
                    from: src,
                });

                #[cfg(feature = "net_debug")]
                println!(
                    "[UDP] Queued {} bytes from {:?} for socket {} (port {})",
                    payload.len(),
                    src,
                    socket_id,
                    dst.port()
                );

                return Ok(());
            } else {
                #[cfg(feature = "net_debug")]
                println!("[UDP] Socket {} queue full, dropping packet", socket_id);
                return Err(KernelError::ResourceExhausted {
                    resource: "udp_queue",
                });
            }
        }
    }

    #[cfg(feature = "net_debug")]
    println!(
        "[UDP] No socket for port {}, dropping packet",
        header.dest_port
    );

    Ok(())
}

/// Send UDP packet (internal implementation)
pub fn send_packet(src: SocketAddr, dst: SocketAddr, data: &[u8]) -> Result<usize, KernelError> {
    // Create UDP header
    let mut header = UdpHeader::new(src.port(), dst.port(), data.len());

    // Calculate checksum
    header.calculate_checksum(src.ip(), dst.ip(), data);

    // Build packet: header + data
    let header_bytes = header.to_bytes();
    let mut packet = Vec::with_capacity(UdpHeader::SIZE + data.len());
    packet.extend_from_slice(&header_bytes);
    packet.extend_from_slice(data);

    // Send via IP layer
    super::ip::send(dst.ip(), super::ip::IpProtocol::Udp, &packet)?;

    Ok(data.len())
}

/// Get UDP statistics
pub fn get_stats() -> UdpStats {
    let sockets = UDP_SOCKETS.lock();
    let mut total_queued = 0;
    for sock in sockets.values() {
        total_queued += sock.recv_queue.len();
    }

    UdpStats {
        active_sockets: sockets.len(),
        datagrams_queued: total_queued,
        datagrams_sent: 0,    // Would track in real implementation
        datagrams_recv: 0,    // Would track in real implementation
        datagrams_dropped: 0, // Would track in real implementation
    }
}

/// UDP statistics
#[derive(Debug, Clone, Copy, Default)]
pub struct UdpStats {
    pub active_sockets: usize,
    pub datagrams_queued: usize,
    pub datagrams_sent: u64,
    pub datagrams_recv: u64,
    pub datagrams_dropped: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::Ipv4Address;

    #[test_case]
    fn test_udp_header() {
        let header = UdpHeader::new(8080, 80, 100);
        assert_eq!(header.source_port, 8080);
        assert_eq!(header.dest_port, 80);
        assert_eq!(header.length, 108); // 8 + 100
    }

    #[test_case]
    fn test_udp_header_roundtrip() {
        let header = UdpHeader::new(1234, 5678, 50);
        let bytes = header.to_bytes();
        let parsed = UdpHeader::from_bytes(&bytes).unwrap();

        assert_eq!(parsed.source_port, 1234);
        assert_eq!(parsed.dest_port, 5678);
        assert_eq!(parsed.length, 58);
    }

    #[test_case]
    fn test_udp_socket() {
        let mut socket = UdpSocket::new();
        let addr = SocketAddr::v4(Ipv4Address::LOCALHOST, 8080);

        assert!(!socket.bound);
        socket.bind(addr).unwrap();
        assert!(socket.bound);
    }
}
