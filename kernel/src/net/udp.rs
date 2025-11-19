//! UDP protocol implementation

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
    pub fn recv_from(&self, buffer: &mut [u8]) -> Result<(usize, SocketAddr), KernelError> {
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
