//! IP layer implementation
//!
//! Handles IPv4 packet construction, parsing, routing, and fragmentation.
//! Provides the foundation for TCP and UDP transport protocols.

use alloc::{string::String, vec::Vec};

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

    /// Length of this header in bytes (`ihl` counts 32-bit words).
    pub fn header_len(&self) -> usize {
        self.ihl as usize * 4
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
        if (bytes[0] & 0x0F) < 5 {
            return Err(KernelError::InvalidArgument {
                name: "ip_ihl",
                value: "below_minimum",
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

/// Parse a received IPv4 packet and return its header and payload.
///
/// The payload is bounded by the header's `total_length`, not by the
/// buffer: link layers pad short frames (Ethernet to 60 bytes), and that
/// padding is not part of the datagram. Rejects lengths that are smaller
/// than the header or larger than the bytes actually received.
pub fn split_packet(bytes: &[u8]) -> Result<(Ipv4Header, &[u8]), KernelError> {
    let header = Ipv4Header::from_bytes(bytes)?;
    let header_len = header.header_len();
    let total_len = header.total_length as usize;
    if header_len > bytes.len() || total_len < header_len || total_len > bytes.len() {
        return Err(KernelError::InvalidArgument {
            name: "ip_length",
            value: "inconsistent",
        });
    }
    Ok((header, &bytes[header_len..total_len]))
}

/// Routing table entry
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteEntry {
    pub destination: Ipv4Address,
    pub netmask: Ipv4Address,
    /// Next hop; `None` for a directly connected network.
    pub gateway: Option<Ipv4Address>,
    /// Name of the device the route leaves through ("eth0", "lo0").
    pub interface: String,
}

impl RouteEntry {
    fn matches(&self, dest: Ipv4Address) -> bool {
        let mask = self.netmask.to_u32();
        dest.to_u32() & mask == self.destination.to_u32() & mask
    }

    fn prefix_len(&self) -> u32 {
        self.netmask.to_u32().count_ones()
    }
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
///
/// Also installs the routes the configuration implies on the primary
/// interface: the connected subnet and, with a gateway, the default route.
/// Both replace earlier routes for the same prefix, so lease renewals do not
/// pile up duplicates.
pub fn set_interface_config(ip: Ipv4Address, mask: Ipv4Address, gw: Option<Ipv4Address>) {
    {
        let mut config = INTERFACE_CONFIG.lock();
        config.ip_addr = ip;
        config.subnet_mask = mask;
        config.gateway = gw;
    }

    if let Some(iface) = super::device::primary_device_name() {
        if mask != Ipv4Address::ANY {
            add_route(RouteEntry {
                destination: Ipv4Address::from_u32(ip.to_u32() & mask.to_u32()),
                netmask: mask,
                gateway: None,
                interface: iface.clone(),
            });
        }
        if let Some(gateway) = gw {
            add_route(RouteEntry {
                destination: Ipv4Address::ANY,
                netmask: Ipv4Address::ANY,
                gateway: Some(gateway),
                interface: iface,
            });
        }
    }

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

/// Add a route, replacing any route for the same prefix.
pub fn add_route(entry: RouteEntry) {
    let mut routes = ROUTES.lock();
    routes.retain(|r| !(r.destination == entry.destination && r.netmask == entry.netmask));
    routes.push(entry);
}

/// Lookup route for destination: the longest matching prefix (NET-ARCH-01;
/// this used to return the first match in insertion order).
pub fn lookup_route(dest: Ipv4Address) -> Option<RouteEntry> {
    best_route(&ROUTES.lock(), dest).cloned()
}

fn best_route(routes: &[RouteEntry], dest: Ipv4Address) -> Option<&RouteEntry> {
    routes
        .iter()
        .filter(|r| r.matches(dest))
        .max_by_key(|r| r.prefix_len())
}

/// The interface and next-hop address for `dest`. Without a matching route
/// the destination is assumed to be on the primary interface's link.
fn next_hop(dest: Ipv4Address) -> Option<(String, Ipv4Address)> {
    match lookup_route(dest) {
        Some(route) => Some((route.interface, route.gateway.unwrap_or(dest))),
        None => super::device::primary_device_name().map(|iface| (iface, dest)),
    }
}

/// Get all routing table entries (used by `route` shell command).
pub fn get_routes() -> Vec<RouteEntry> {
    ROUTES.lock().clone()
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

            // Choose the interface and next hop from the routing table: an
            // off-link destination is reached through its gateway, so ARP
            // asks for the gateway's address, not the destination's.
            let (iface, hop) = if dest_v4 == Ipv4Address::BROADCAST {
                match super::device::primary_device_name() {
                    Some(iface) => (iface, dest_v4),
                    None => return Ok(()),
                }
            } else {
                match next_hop(dest_v4) {
                    Some(route) => route,
                    // No usable interface (none up yet): nothing to send on.
                    None => return Ok(()),
                }
            };

            // Resolve the next hop's MAC via ARP (broadcast for broadcast IP)
            let dst_mac = if hop == Ipv4Address::BROADCAST {
                super::MacAddress::BROADCAST
            } else {
                // Check ARP cache; if miss, send ARP request and use broadcast
                super::arp::resolve(hop).unwrap_or_else(|| {
                    super::arp::send_arp_request(hop);
                    super::MacAddress::BROADCAST
                })
            };

            let src_mac = super::device::with_device(&iface, |dev| dev.mac_address())
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
            super::device::with_device_mut(&iface, |dev| {
                let _ = dev.transmit(&pkt);
            });

            super::update_stats_tx(header.total_length as usize);

            Ok(())
        }
        IpAddress::V6(dest_v6) => {
            // Delegate to the IPv6 module
            let src = super::ipv6::select_source_address(&dest_v6)
                .unwrap_or(super::Ipv6Address::UNSPECIFIED);
            let next_header = match protocol {
                IpProtocol::Tcp => super::ipv6::NEXT_HEADER_TCP,
                IpProtocol::Udp => super::ipv6::NEXT_HEADER_UDP,
                IpProtocol::Icmp => super::ipv6::NEXT_HEADER_ICMPV6,
            };
            super::ipv6::send(&src, &dest_v6, next_header, data)
        }
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
        interface: String::from("lo0"),
    });

    println!("[IP] IP layer initialized");
    Ok(())
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use super::*;

    fn route(dest: [u8; 4], mask: [u8; 4], gw: Option<[u8; 4]>, iface: &str) -> RouteEntry {
        RouteEntry {
            destination: Ipv4Address(dest),
            netmask: Ipv4Address(mask),
            gateway: gw.map(Ipv4Address),
            interface: String::from(iface),
        }
    }

    /// NET-ARCH-01: the most specific route wins whatever the insertion
    /// order (a default route added first used to capture everything).
    #[test]
    fn longest_prefix_match() {
        let routes = [
            route([0, 0, 0, 0], [0, 0, 0, 0], Some([10, 0, 2, 2]), "eth1"),
            route([10, 0, 2, 0], [255, 255, 255, 0], None, "eth1"),
            route([127, 0, 0, 0], [255, 0, 0, 0], None, "lo0"),
        ];
        let pick = |a: [u8; 4]| best_route(&routes, Ipv4Address(a)).unwrap().clone();
        assert_eq!(pick([10, 0, 2, 15]).gateway, None, "on-link: direct");
        assert_eq!(
            pick([93, 184, 216, 34]).gateway,
            Some(Ipv4Address([10, 0, 2, 2]))
        );
        assert_eq!(pick([127, 0, 0, 1]).interface, "lo0");
        assert!(best_route(&routes[1..], Ipv4Address([8, 8, 8, 8])).is_none());
    }

    #[test]
    fn add_route_replaces_same_prefix() {
        let before = get_routes().len();
        let r = |gw| route([0, 0, 0, 0], [0, 0, 0, 0], Some(gw), "test0");
        add_route(r([192, 0, 2, 1]));
        add_route(r([192, 0, 2, 254]));
        let routes = get_routes();
        assert_eq!(routes.len(), before + 1, "a renewal replaces, not appends");
        assert!(routes.contains(&r([192, 0, 2, 254])));
        ROUTES.lock().retain(|x| x.interface != "test0");
    }

    fn packet(ihl: u8, total_length: u16, payload: &[u8], padding: usize) -> Vec<u8> {
        let mut header = Ipv4Header::new(
            Ipv4Address([10, 0, 2, 2]),
            Ipv4Address([10, 0, 2, 15]),
            IpProtocol::Udp,
        );
        header.ihl = ihl;
        header.total_length = total_length;
        let mut bytes = Vec::from(header.to_bytes());
        bytes.extend_from_slice(payload);
        bytes.resize(bytes.len() + padding, 0);
        bytes
    }

    #[test]
    fn test_split_packet_excludes_link_padding() {
        // A 4-byte payload in a minimum-size Ethernet frame arrives with
        // padding; only `total_length` bytes belong to the datagram.
        let bytes = packet(5, 24, &[1, 2, 3, 4], 22);
        let (_, payload) = split_packet(&bytes).unwrap();
        assert_eq!(payload, &[1, 2, 3, 4]);
    }

    #[test]
    fn test_split_packet_rejects_malformed_lengths() {
        // IHL below the 5-word minimum header.
        assert!(split_packet(&packet(4, 24, &[1, 2, 3, 4], 0)).is_err());
        // total_length shorter than the header it describes.
        assert!(split_packet(&packet(5, 19, &[1, 2, 3, 4], 0)).is_err());
        // total_length longer than the bytes received.
        assert!(split_packet(&packet(5, 100, &[1, 2, 3, 4], 0)).is_err());
        // IHL claims options that were not received.
        assert!(split_packet(&packet(15, 24, &[1, 2, 3, 4], 0)).is_err());
    }

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
