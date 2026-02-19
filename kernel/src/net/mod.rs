//! Network stack for VeridianOS
//!
//! Provides TCP/IP networking capabilities including:
//! - IP layer (IPv4/IPv6)
//! - TCP protocol
//! - UDP protocol
//! - Socket API
//! - Network device abstraction

pub mod device;
pub mod dhcp;
pub mod dma_pool;
pub mod integration;
pub mod ip;
pub mod socket;
pub mod tcp;
pub mod udp;
pub mod zero_copy;

use alloc::vec::Vec;

use crate::error::KernelError;

/// MAC address (6 bytes)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MacAddress(pub [u8; 6]);

impl MacAddress {
    pub const BROADCAST: Self = Self([0xFF; 6]);
    pub const ZERO: Self = Self([0x00; 6]);

    pub const fn new(bytes: [u8; 6]) -> Self {
        Self(bytes)
    }
}

/// IPv4 address (4 bytes)
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Ipv4Address(pub [u8; 4]);

impl Ipv4Address {
    pub const LOCALHOST: Self = Self([127, 0, 0, 1]);
    pub const BROADCAST: Self = Self([255, 255, 255, 255]);
    pub const ANY: Self = Self([0, 0, 0, 0]);
    pub const UNSPECIFIED: Self = Self([0, 0, 0, 0]);

    pub const fn new(a: u8, b: u8, c: u8, d: u8) -> Self {
        Self([a, b, c, d])
    }

    pub fn from_u32(addr: u32) -> Self {
        Self(addr.to_be_bytes())
    }

    pub fn to_u32(&self) -> u32 {
        u32::from_be_bytes(self.0)
    }
}

/// IPv6 address (16 bytes)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ipv6Address(pub [u8; 16]);

impl Ipv6Address {
    pub const LOCALHOST: Self = Self([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]);
    pub const ANY: Self = Self([0; 16]);
}

/// IP address (v4 or v6)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpAddress {
    V4(Ipv4Address),
    V6(Ipv6Address),
}

/// Port number
pub type Port = u16;

/// Socket address (IP + port)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SocketAddr {
    pub ip: IpAddress,
    pub port: Port,
}

impl SocketAddr {
    pub fn new(ip: IpAddress, port: Port) -> Self {
        Self { ip, port }
    }

    pub fn v4(addr: Ipv4Address, port: Port) -> Self {
        Self {
            ip: IpAddress::V4(addr),
            port,
        }
    }

    pub fn ip(&self) -> IpAddress {
        self.ip
    }

    pub fn port(&self) -> Port {
        self.port
    }
}

/// Network packet
#[derive(Clone)]
pub struct Packet {
    data: Vec<u8>,
    length: usize,
}

impl Packet {
    pub fn new(size: usize) -> Self {
        Self {
            data: alloc::vec![0u8; size],
            length: 0,
        }
    }

    pub fn from_bytes(bytes: &[u8]) -> Self {
        Self {
            data: bytes.to_vec(),
            length: bytes.len(),
        }
    }

    pub fn data(&self) -> &[u8] {
        &self.data[..self.length]
    }

    pub fn len(&self) -> usize {
        self.length
    }

    pub fn is_empty(&self) -> bool {
        self.length == 0
    }

    pub fn set_length(&mut self, len: usize) {
        self.length = len.min(self.data.len());
    }
}

/// Network statistics
#[derive(Debug, Default, Clone, Copy)]
pub struct NetworkStats {
    pub packets_sent: u64,
    pub packets_received: u64,
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub errors: u64,
}

static STATS: spin::Mutex<NetworkStats> = spin::Mutex::new(NetworkStats {
    packets_sent: 0,
    packets_received: 0,
    bytes_sent: 0,
    bytes_received: 0,
    errors: 0,
});

/// Update network statistics
pub fn update_stats_tx(bytes: usize) {
    let mut stats = STATS.lock();
    stats.packets_sent += 1;
    stats.bytes_sent += bytes as u64;
}

pub fn update_stats_rx(bytes: usize) {
    let mut stats = STATS.lock();
    stats.packets_received += 1;
    stats.bytes_received += bytes as u64;
}

pub fn get_stats() -> NetworkStats {
    *STATS.lock()
}

/// Initialize network stack
pub fn init() -> Result<(), KernelError> {
    println!("[NET] Initializing network stack...");

    // Initialize DMA buffer pool for zero-copy networking
    dma_pool::init_network_pool(256)?;

    // Initialize device layer
    device::init()?;

    // Initialize IP layer
    ip::init()?;

    // Initialize TCP
    tcp::init()?;

    // Initialize UDP
    udp::init()?;

    // Initialize socket layer
    socket::init()?;

    // Register hardware network drivers
    if let Err(_e) = integration::register_drivers() {
        println!(
            "[NET] Warning: driver registration failed (non-fatal): {:?}",
            _e
        );
    }

    println!("[NET] Network stack initialized");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ipv4_address() {
        let addr = Ipv4Address::new(192, 168, 1, 1);
        assert_eq!(addr.0, [192, 168, 1, 1]);
    }

    #[test]
    fn test_mac_address() {
        let mac = MacAddress::new([0x00, 0x11, 0x22, 0x33, 0x44, 0x55]);
        assert_eq!(mac.0[0], 0x00);
        assert_eq!(mac.0[5], 0x55);
    }

    #[test]
    fn test_packet() {
        let data = b"Hello, Network!";
        let pkt = Packet::from_bytes(data);
        assert_eq!(pkt.data(), data);
    }
}
