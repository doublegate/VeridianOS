//! IP layer implementation
//!
//! Handles IPv4 packet construction, parsing, routing, and fragmentation.
//! Provides the foundation for TCP and UDP transport protocols.

use alloc::vec::Vec;

use spin::Mutex;

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

/// Interface IP configuration
#[allow(dead_code)] // Phase 6 network stack -- grows as DHCP/ifconfig configures interfaces
#[derive(Debug, Clone, Copy)]
pub struct InterfaceConfig {
    /// Assigned IP address (0.0.0.0 = unconfigured)
    pub ip_addr: Ipv4Address,
    /// Subnet mask
    pub subnet_mask: Ipv4Address,
    /// Default gateway
    pub gateway: Option<Ipv4Address>,
}

/// Global interface configuration (primary interface)
static INTERFACE_CONFIG: Mutex<InterfaceConfig> = Mutex::new(InterfaceConfig {
    ip_addr: Ipv4Address::ANY,
    subnet_mask: Ipv4Address::ANY,
    gateway: None,
});

/// Get the currently configured interface IP address.
pub fn get_interface_ip() -> Ipv4Address {
    INTERFACE_CONFIG.lock().ip_addr
}

/// Get the current interface configuration.
pub fn get_interface_config() -> InterfaceConfig {
    *INTERFACE_CONFIG.lock()
}

/// Set the interface IP configuration (called by DHCP or manual config).
pub fn set_interface_config(ip: Ipv4Address, mask: Ipv4Address, gw: Option<Ipv4Address>) {
    let mut config = INTERFACE_CONFIG.lock();
    config.ip_addr = ip;
    config.subnet_mask = mask;
    config.gateway = gw;

    println!(
        "[IP] Interface configured: {}.{}.{}.{}/{}.{}.{}.{}",
        ip.0[0], ip.0[1], ip.0[2], ip.0[3], mask.0[0], mask.0[1], mask.0[2], mask.0[3],
    );

    if let Some(gateway) = gw {
        println!(
            "[IP] Gateway: {}.{}.{}.{}",
            gateway.0[0], gateway.0[1], gateway.0[2], gateway.0[3],
        );
    }
}

/// Simple routing table protected by Mutex
static ROUTES: Mutex<Vec<RouteEntry>> = Mutex::new(Vec::new());

/// Add a route
pub fn add_route(entry: RouteEntry) {
    ROUTES.lock().push(entry);
}

/// Lookup route for destination
pub fn lookup_route(dest: Ipv4Address) -> Option<RouteEntry> {
    let routes = ROUTES.lock();
    for route in routes.iter() {
        let dest_masked = dest.to_u32() & route.netmask.to_u32();
        let route_masked = route.destination.to_u32() & route.netmask.to_u32();

        if dest_masked == route_masked {
            return Some(route.clone());
        }
    }
    None
}

/// Global IP identification counter for unique packet IDs
static IP_ID_COUNTER: core::sync::atomic::AtomicU16 = core::sync::atomic::AtomicU16::new(1);

/// Send IP packet
///
/// Constructs an IPv4 header, wraps the payload in an Ethernet frame,
/// and transmits via the appropriate network device.
pub fn send(dest: IpAddress, protocol: IpProtocol, data: &[u8]) -> Result<(), KernelError> {
    match dest {
        IpAddress::V4(dest_v4) => {
            // Use configured interface address (falls back to 0.0.0.0 pre-DHCP)
            let src = get_interface_ip();

            let mut header = Ipv4Header::new(src, dest_v4, protocol);
            header.total_length = (Ipv4Header::MIN_SIZE + data.len()) as u16;
            header.identification =
                IP_ID_COUNTER.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
            header.flags = 0x02; // Don't Fragment
            header.calculate_checksum();

            // Combine IP header + payload
            let header_bytes = header.to_bytes();
            let mut ip_packet = Vec::with_capacity(header_bytes.len() + data.len());
            ip_packet.extend_from_slice(&header_bytes);
            ip_packet.extend_from_slice(data);

            // Resolve destination MAC via ARP (or use broadcast for broadcast IP)
            let dst_mac = if dest_v4 == Ipv4Address::BROADCAST {
                super::MacAddress::BROADCAST
            } else {
                // Check ARP cache; if miss, send ARP request and use broadcast
                super::arp::resolve(dest_v4).unwrap_or_else(|| {
                    super::arp::send_arp_request(dest_v4);
                    super::MacAddress::BROADCAST
                })
            };

            let src_mac = super::device::with_device("eth0", |dev| dev.mac_address())
                .unwrap_or(super::MacAddress::ZERO);

            // Wrap in Ethernet frame
            let frame = super::ethernet::construct_frame(
                dst_mac,
                src_mac,
                super::ethernet::ETHERTYPE_IPV4,
                &ip_packet,
            );

            // Transmit
            let pkt = super::Packet::from_bytes(&frame);
            super::device::with_device_mut("eth0", |dev| {
                let _ = dev.transmit(&pkt);
            });

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

    #[test]
    fn test_ipv4_header() {
        let src = Ipv4Address::new(192, 168, 1, 1);
        let dst = Ipv4Address::new(192, 168, 1, 2);
        let header = Ipv4Header::new(src, dst, IpProtocol::Tcp);

        assert_eq!(header.version, 4);
        assert_eq!(header.protocol, 6);
        assert_eq!(header.source, src);
        assert_eq!(header.destination, dst);
    }

    #[test]
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
