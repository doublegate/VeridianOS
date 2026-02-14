//! IP layer implementation
//!
//! Handles IPv4 packet construction, parsing, routing, and fragmentation.
//! Provides the foundation for TCP and UDP transport protocols.

#![allow(static_mut_refs)]

use alloc::vec::Vec;

use super::{IpAddress, Ipv4Address};
use crate::error::KernelError;

/// IP protocol numbers
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum IpProtocol {
    Icmp = 1,
    Tcp = 6,
    Udp = 17,
}

/// IPv4 header
#[derive(Debug, Clone)]
pub struct Ipv4Header {
    pub version: u8,
    pub ihl: u8,
    pub tos: u8,
    pub total_length: u16,
    pub identification: u16,
    pub flags: u8,
    pub fragment_offset: u16,
    pub ttl: u8,
    pub protocol: u8,
    pub checksum: u16,
    pub source: Ipv4Address,
    pub destination: Ipv4Address,
}

impl Ipv4Header {
    pub const MIN_SIZE: usize = 20;

    pub fn new(src: Ipv4Address, dst: Ipv4Address, protocol: IpProtocol) -> Self {
        Self {
            version: 4,
            ihl: 5, // 5 * 4 = 20 bytes
            tos: 0,
            total_length: 0,
            identification: 0,
            flags: 0,
            fragment_offset: 0,
            ttl: 64,
            protocol: protocol as u8,
            checksum: 0,
            source: src,
            destination: dst,
        }
    }

    pub fn to_bytes(&self) -> [u8; 20] {
        let mut bytes = [0u8; 20];

        bytes[0] = (self.version << 4) | self.ihl;
        bytes[1] = self.tos;
        bytes[2..4].copy_from_slice(&self.total_length.to_be_bytes());
        bytes[4..6].copy_from_slice(&self.identification.to_be_bytes());
        bytes[6] = (self.flags << 5) | ((self.fragment_offset >> 8) as u8);
        bytes[7] = (self.fragment_offset & 0xFF) as u8;
        bytes[8] = self.ttl;
        bytes[9] = self.protocol;
        bytes[10..12].copy_from_slice(&self.checksum.to_be_bytes());
        bytes[12..16].copy_from_slice(&self.source.0);
        bytes[16..20].copy_from_slice(&self.destination.0);

        bytes
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, KernelError> {
        if bytes.len() < Self::MIN_SIZE {
            return Err(KernelError::InvalidArgument {
                name: "ip_header",
                value: "too_short",
            });
        }

        let version = bytes[0] >> 4;
        if version != 4 {
            return Err(KernelError::InvalidArgument {
                name: "ip_version",
                value: "not_ipv4",
            });
        }

        Ok(Self {
            version,
            ihl: bytes[0] & 0x0F,
            tos: bytes[1],
            total_length: u16::from_be_bytes([bytes[2], bytes[3]]),
            identification: u16::from_be_bytes([bytes[4], bytes[5]]),
            flags: bytes[6] >> 5,
            fragment_offset: u16::from_be_bytes([bytes[6] & 0x1F, bytes[7]]),
            ttl: bytes[8],
            protocol: bytes[9],
            checksum: u16::from_be_bytes([bytes[10], bytes[11]]),
            source: Ipv4Address([bytes[12], bytes[13], bytes[14], bytes[15]]),
            destination: Ipv4Address([bytes[16], bytes[17], bytes[18], bytes[19]]),
        })
    }

    /// Calculate checksum
    pub fn calculate_checksum(&mut self) {
        self.checksum = 0;
        let bytes = self.to_bytes();

        let mut sum: u32 = 0;
        for i in 0..10 {
            sum += u16::from_be_bytes([bytes[i * 2], bytes[i * 2 + 1]]) as u32;
        }

        while sum >> 16 != 0 {
            sum = (sum & 0xFFFF) + (sum >> 16);
        }

        self.checksum = !(sum as u16);
    }
}

/// Routing table entry
#[derive(Debug, Clone)]
pub struct RouteEntry {
    pub destination: Ipv4Address,
    pub netmask: Ipv4Address,
    pub gateway: Option<Ipv4Address>,
    pub interface: usize,
}

/// Simple routing table
static mut ROUTES: Vec<RouteEntry> = Vec::new();

/// Add a route
pub fn add_route(entry: RouteEntry) {
    // SAFETY: ROUTES is a static mut Vec modified during single-threaded kernel
    // init or controlled routing table updates. No concurrent access assumed.
    unsafe {
        ROUTES.push(entry);
    }
}

/// Lookup route for destination
pub fn lookup_route(dest: Ipv4Address) -> Option<RouteEntry> {
    // SAFETY: ROUTES is a static mut Vec read during route lookup. Read-only access
    // assumes no concurrent modification to the routing table.
    unsafe {
        for route in &ROUTES {
            let dest_masked = dest.to_u32() & route.netmask.to_u32();
            let route_masked = route.destination.to_u32() & route.netmask.to_u32();

            if dest_masked == route_masked {
                return Some(route.clone());
            }
        }
    }
    None
}

/// Send IP packet
pub fn send(dest: IpAddress, protocol: IpProtocol, data: &[u8]) -> Result<(), KernelError> {
    match dest {
        IpAddress::V4(dest_v4) => {
            let src = Ipv4Address::LOCALHOST; // TODO(phase4): Get source address from interface config

            let mut header = Ipv4Header::new(src, dest_v4, protocol);
            header.total_length = (Ipv4Header::MIN_SIZE + data.len()) as u16;
            header.calculate_checksum();

            // TODO(phase4): Route and send packet through network device
            super::update_stats_tx(header.total_length as usize);

            Ok(())
        }
        IpAddress::V6(_) => Err(KernelError::NotImplemented {
            feature: "ipv6_send",
        }),
    }
}

/// Initialize IP layer
pub fn init() -> Result<(), KernelError> {
    println!("[IP] Initializing IP layer...");

    // Add default loopback route
    add_route(RouteEntry {
        destination: Ipv4Address::new(127, 0, 0, 0),
        netmask: Ipv4Address::new(255, 0, 0, 0),
        gateway: None,
        interface: 0,
    });

    println!("[IP] IP layer initialized");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test_case]
    fn test_ipv4_header() {
        let src = Ipv4Address::new(192, 168, 1, 1);
        let dst = Ipv4Address::new(192, 168, 1, 2);
        let header = Ipv4Header::new(src, dst, IpProtocol::Tcp);

        assert_eq!(header.version, 4);
        assert_eq!(header.protocol, 6);
        assert_eq!(header.source, src);
        assert_eq!(header.destination, dst);
    }

    #[test_case]
    fn test_ipv4_header_roundtrip() {
        let src = Ipv4Address::new(10, 0, 0, 1);
        let dst = Ipv4Address::new(10, 0, 0, 2);
        let mut header = Ipv4Header::new(src, dst, IpProtocol::Udp);
        header.calculate_checksum();

        let bytes = header.to_bytes();
        let parsed = Ipv4Header::from_bytes(&bytes).unwrap();

        assert_eq!(parsed.source, src);
        assert_eq!(parsed.destination, dst);
        assert_eq!(parsed.protocol, 17);
    }
}
