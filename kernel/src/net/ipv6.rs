//! IPv6 protocol implementation
//!
//! Provides IPv6 packet construction, parsing, address utilities, NDP (Neighbor
//! Discovery Protocol) cache management, and dual-stack configuration for
//! simultaneous IPv4/IPv6 networking.

#![allow(dead_code)] // Phase 7 network stack -- functions called as stack matures

use alloc::{collections::BTreeMap, format, string::String, vec::Vec};
use core::sync::atomic::{AtomicU64, Ordering};

use spin::RwLock;

use super::{Ipv6Address, MacAddress};
use crate::{error::KernelError, sync::once_lock::GlobalState};

// ============================================================================
// Constants
// ============================================================================

/// IPv6 header size in bytes (fixed, unlike IPv4)
pub const IPV6_HEADER_SIZE: usize = 40;

/// IPv6 version number
pub const IPV6_VERSION: u8 = 6;

/// Default hop limit (equivalent to IPv4 TTL)
pub const DEFAULT_HOP_LIMIT: u8 = 64;

/// IPv6 minimum MTU (all links must support at least 1280 bytes)
pub const IPV6_MIN_MTU: usize = 1280;

/// Next header protocol numbers
pub const NEXT_HEADER_HOP_BY_HOP: u8 = 0;
pub const NEXT_HEADER_TCP: u8 = 6;
pub const NEXT_HEADER_UDP: u8 = 17;
pub const NEXT_HEADER_ICMPV6: u8 = 58;
pub const NEXT_HEADER_NO_NEXT: u8 = 59;
pub const NEXT_HEADER_FRAGMENT: u8 = 44;

/// ICMPv6 NDP message types
pub const ICMPV6_ROUTER_SOLICIT: u8 = 133;
pub const ICMPV6_ROUTER_ADVERT: u8 = 134;
pub const ICMPV6_NEIGHBOR_SOLICIT: u8 = 135;
pub const ICMPV6_NEIGHBOR_ADVERT: u8 = 136;
pub const ICMPV6_REDIRECT: u8 = 137;

/// NDP option types
pub const NDP_OPT_SOURCE_LINK_ADDR: u8 = 1;
pub const NDP_OPT_TARGET_LINK_ADDR: u8 = 2;
pub const NDP_OPT_PREFIX_INFO: u8 = 3;
pub const NDP_OPT_MTU: u8 = 5;

/// NDP cache limits
const NDP_CACHE_MAX: usize = 128;

/// NDP entry max age in ticks (approximately 30 seconds in REACHABLE state)
const NDP_REACHABLE_TIME: u64 = 30;

/// NDP stale entry timeout in ticks (approximately 10 minutes)
const NDP_STALE_TIMEOUT: u64 = 600;

/// EtherType for IPv6
pub const ETHERTYPE_IPV6: u16 = 0x86DD;

// ============================================================================
// IPv6 Header
// ============================================================================

/// IPv6 packet header (40 bytes fixed)
///
/// Layout:
///   - version_tc_flow (4 bytes): 4-bit version, 8-bit traffic class, 20-bit
///     flow label
///   - payload_length (2 bytes): length of payload after this header
///     (big-endian)
///   - next_header (1 byte): identifies the type of header immediately
///     following
///   - hop_limit (1 byte): decremented by 1 at each forwarding node
///   - source (16 bytes): source IPv6 address
///   - destination (16 bytes): destination IPv6 address
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct Ipv6Header {
    /// Version (4 bits), Traffic Class (8 bits), Flow Label (20 bits) --
    /// network byte order
    pub version_tc_flow: u32,
    /// Payload length in bytes (big-endian, does not include this header)
    pub payload_length: u16,
    /// Next header protocol number (TCP=6, UDP=17, ICMPv6=58)
    pub next_header: u8,
    /// Hop limit (TTL equivalent)
    pub hop_limit: u8,
    /// Source IPv6 address (16 bytes)
    pub source: [u8; 16],
    /// Destination IPv6 address (16 bytes)
    pub destination: [u8; 16],
}

impl Ipv6Header {
    /// Create a new IPv6 header with default values
    pub fn new(src: &Ipv6Address, dst: &Ipv6Address, next_header: u8) -> Self {
        // version=6, traffic_class=0, flow_label=0
        let version_tc_flow: u32 = (IPV6_VERSION as u32) << 28;

        Self {
            version_tc_flow,
            payload_length: 0,
            next_header,
            hop_limit: DEFAULT_HOP_LIMIT,
            source: src.0,
            destination: dst.0,
        }
    }

    /// Get the IP version field (should always be 6)
    pub fn version(&self) -> u8 {
        ((self.version_tc_flow >> 28) & 0x0F) as u8
    }

    /// Get the traffic class field
    pub fn traffic_class(&self) -> u8 {
        ((self.version_tc_flow >> 20) & 0xFF) as u8
    }

    /// Get the flow label field
    pub fn flow_label(&self) -> u32 {
        self.version_tc_flow & 0x000F_FFFF
    }

    /// Serialize header to bytes (big-endian)
    pub fn to_bytes(&self) -> [u8; IPV6_HEADER_SIZE] {
        let mut bytes = [0u8; IPV6_HEADER_SIZE];

        // Version + Traffic Class + Flow Label (4 bytes, big-endian)
        let vtf_be = self.version_tc_flow.to_be_bytes();
        bytes[0..4].copy_from_slice(&vtf_be);

        // Payload Length (2 bytes, big-endian)
        bytes[4..6].copy_from_slice(&self.payload_length.to_be_bytes());

        // Next Header
        bytes[6] = self.next_header;

        // Hop Limit
        bytes[7] = self.hop_limit;

        // Source address (16 bytes)
        bytes[8..24].copy_from_slice(&self.source);

        // Destination address (16 bytes)
        bytes[24..40].copy_from_slice(&self.destination);

        bytes
    }
}

// ============================================================================
// IPv6 Packet Parsing and Construction
// ============================================================================

/// Parse an IPv6 packet from raw bytes.
///
/// Returns the parsed header and a slice of the payload data.
pub fn parse_ipv6(data: &[u8]) -> Result<(Ipv6Header, &[u8]), KernelError> {
    if data.len() < IPV6_HEADER_SIZE {
        return Err(KernelError::InvalidArgument {
            name: "ipv6_packet",
            value: "too_short",
        });
    }

    // Parse version/traffic class/flow label (4 bytes, big-endian)
    let version_tc_flow = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);

    let version = (version_tc_flow >> 28) as u8;
    if version != IPV6_VERSION {
        return Err(KernelError::InvalidArgument {
            name: "ipv6_version",
            value: "not_ipv6",
        });
    }

    let payload_length = u16::from_be_bytes([data[4], data[5]]);
    let next_header = data[6];
    let hop_limit = data[7];

    let mut source = [0u8; 16];
    let mut destination = [0u8; 16];
    source.copy_from_slice(&data[8..24]);
    destination.copy_from_slice(&data[24..40]);

    let header = Ipv6Header {
        version_tc_flow,
        payload_length,
        next_header,
        hop_limit,
        source,
        destination,
    };

    // Extract payload (bounded by payload_length and available data)
    let payload_end = IPV6_HEADER_SIZE + (payload_length as usize);
    let actual_end = payload_end.min(data.len());
    let payload = &data[IPV6_HEADER_SIZE..actual_end];

    Ok((header, payload))
}

/// Build an IPv6 packet with the given parameters.
///
/// Returns a complete IPv6 packet (header + payload) as a `Vec<u8>`.
pub fn build_ipv6(
    src: &Ipv6Address,
    dst: &Ipv6Address,
    next_header: u8,
    payload: &[u8],
) -> Vec<u8> {
    let mut header = Ipv6Header::new(src, dst, next_header);
    header.payload_length = payload.len() as u16;

    let header_bytes = header.to_bytes();
    let mut packet = Vec::with_capacity(IPV6_HEADER_SIZE + payload.len());
    packet.extend_from_slice(&header_bytes);
    packet.extend_from_slice(payload);

    packet
}

// ============================================================================
// IPv6 Address Utilities
// ============================================================================

/// Check if an IPv6 address is link-local (fe80::/10)
pub fn is_link_local(addr: &Ipv6Address) -> bool {
    addr.0[0] == 0xfe && (addr.0[1] & 0xc0) == 0x80
}

/// Check if an IPv6 address is multicast (ff00::/8)
pub fn is_multicast(addr: &Ipv6Address) -> bool {
    addr.0[0] == 0xff
}

/// Check if an IPv6 address is the loopback address (::1)
pub fn is_loopback(addr: &Ipv6Address) -> bool {
    *addr == Ipv6Address::LOCALHOST
}

/// Check if an IPv6 address is the unspecified address (::)
pub fn is_unspecified(addr: &Ipv6Address) -> bool {
    *addr == Ipv6Address::UNSPECIFIED
}

/// Check if an IPv6 address is a global unicast address (2000::/3)
pub fn is_global_unicast(addr: &Ipv6Address) -> bool {
    (addr.0[0] & 0xe0) == 0x20
}

/// Check if an IPv6 address is a unique local address (fc00::/7)
pub fn is_unique_local(addr: &Ipv6Address) -> bool {
    (addr.0[0] & 0xfe) == 0xfc
}

/// Check if an IPv6 address is an IPv4-mapped IPv6 address (::ffff:0:0/96)
pub fn is_ipv4_mapped(addr: &Ipv6Address) -> bool {
    addr.0[0..10] == [0; 10] && addr.0[10] == 0xff && addr.0[11] == 0xff
}

/// Compute the solicited-node multicast address for a given unicast address.
///
/// Format: ff02::1:ffXX:XXXX where XX:XXXX are the last 3 bytes of the unicast
/// address.
pub fn solicited_node_multicast(addr: &Ipv6Address) -> Ipv6Address {
    Ipv6Address([
        0xff, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0xff, addr.0[13],
        addr.0[14], addr.0[15],
    ])
}

/// Generate a link-local address from a MAC address using EUI-64.
///
/// Converts a 48-bit MAC to a 64-bit interface ID by inserting FF:FE
/// and flipping the universal/local bit, then prepending fe80::/10.
pub fn link_local_from_mac(mac: &MacAddress) -> Ipv6Address {
    let mut addr = [0u8; 16];

    // fe80::/10 prefix
    addr[0] = 0xfe;
    addr[1] = 0x80;
    // bytes 2..7 are zero (padding)

    // EUI-64: insert FF:FE in the middle of the MAC, flip U/L bit
    addr[8] = mac.0[0] ^ 0x02; // flip universal/local bit
    addr[9] = mac.0[1];
    addr[10] = mac.0[2];
    addr[11] = 0xff;
    addr[12] = 0xfe;
    addr[13] = mac.0[3];
    addr[14] = mac.0[4];
    addr[15] = mac.0[5];

    Ipv6Address(addr)
}

/// Format an IPv6 address as a human-readable string.
///
/// Produces the colon-hex notation (e.g., "fe80:0:0:0:200:ff:fe00:1").
/// Does not apply zero-compression (::) for simplicity.
pub fn format_ipv6(addr: &Ipv6Address) -> String {
    let b = &addr.0;
    format!(
        "{:x}:{:x}:{:x}:{:x}:{:x}:{:x}:{:x}:{:x}",
        u16::from_be_bytes([b[0], b[1]]),
        u16::from_be_bytes([b[2], b[3]]),
        u16::from_be_bytes([b[4], b[5]]),
        u16::from_be_bytes([b[6], b[7]]),
        u16::from_be_bytes([b[8], b[9]]),
        u16::from_be_bytes([b[10], b[11]]),
        u16::from_be_bytes([b[12], b[13]]),
        u16::from_be_bytes([b[14], b[15]]),
    )
}

/// Format an IPv6 address with zero-compression (::).
///
/// Finds the longest run of consecutive zero groups and replaces it with "::".
pub fn format_ipv6_compressed(addr: &Ipv6Address) -> String {
    let b = &addr.0;
    let groups: [u16; 8] = [
        u16::from_be_bytes([b[0], b[1]]),
        u16::from_be_bytes([b[2], b[3]]),
        u16::from_be_bytes([b[4], b[5]]),
        u16::from_be_bytes([b[6], b[7]]),
        u16::from_be_bytes([b[8], b[9]]),
        u16::from_be_bytes([b[10], b[11]]),
        u16::from_be_bytes([b[12], b[13]]),
        u16::from_be_bytes([b[14], b[15]]),
    ];

    // Find the longest run of consecutive zero groups
    let mut best_start = 0usize;
    let mut best_len = 0usize;
    let mut cur_start = 0usize;
    let mut cur_len = 0usize;

    for (i, &group) in groups.iter().enumerate() {
        if group == 0 {
            if cur_len == 0 {
                cur_start = i;
            }
            cur_len += 1;
            if cur_len > best_len {
                best_start = cur_start;
                best_len = cur_len;
            }
        } else {
            cur_len = 0;
        }
    }

    // No compression worthwhile if fewer than 2 consecutive zero groups
    if best_len < 2 {
        return format_ipv6(addr);
    }

    let mut parts: Vec<String> = Vec::new();
    let mut i = 0usize;
    let mut compressed = false;

    while i < 8 {
        if i == best_start && !compressed {
            // Insert the :: marker
            if i == 0 {
                parts.push(String::new()); // leading empty for ::
            }
            parts.push(String::new()); // the :: itself
            if best_start + best_len == 8 {
                parts.push(String::new()); // trailing empty for ::
            }
            i += best_len;
            compressed = true;
        } else {
            parts.push(format!("{:x}", groups[i]));
            i += 1;
        }
    }

    parts.join(":")
}

/// Convert a multicast IPv6 address to its Ethernet multicast MAC address.
///
/// Multicast MAC = 33:33:XX:XX:XX:XX where XX are the low 4 bytes of the IPv6
/// address.
pub fn multicast_mac(addr: &Ipv6Address) -> MacAddress {
    MacAddress([0x33, 0x33, addr.0[12], addr.0[13], addr.0[14], addr.0[15]])
}

// ============================================================================
// NDP (Neighbor Discovery Protocol)
// ============================================================================

/// NDP neighbor cache entry state (RFC 4861 Section 7.3.2)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NdpState {
    /// Address resolution is in progress; waiting for NA response
    Incomplete,
    /// Positive confirmation that the path is working
    Reachable,
    /// No positive confirmation received recently; still usable
    Stale,
    /// Stale entry waiting for upper-layer reachability confirmation
    Delay,
    /// Actively probing with Neighbor Solicitation messages
    Probe,
}

/// NDP neighbor cache entry
#[derive(Debug, Clone)]
pub struct NdpEntry {
    /// Resolved MAC address (may be unset in Incomplete state)
    pub mac: MacAddress,
    /// Current state of this entry
    pub state: NdpState,
    /// Tick count when this entry was created or last confirmed
    pub timestamp: u64,
    /// Number of NS probes sent (for Incomplete/Probe states)
    pub probe_count: u8,
}

/// NDP neighbor cache
pub struct NdpCache {
    entries: BTreeMap<Ipv6Address, NdpEntry>,
}

impl Default for NdpCache {
    fn default() -> Self {
        Self::new()
    }
}

impl NdpCache {
    /// Create a new empty NDP cache
    pub const fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
        }
    }

    /// Look up a neighbor's MAC address.
    ///
    /// Returns `Some(MacAddress)` if the entry is in Reachable or Stale state.
    pub fn lookup(&self, addr: &Ipv6Address) -> Option<MacAddress> {
        if let Some(entry) = self.entries.get(addr) {
            match entry.state {
                NdpState::Reachable | NdpState::Stale | NdpState::Delay | NdpState::Probe => {
                    Some(entry.mac)
                }
                NdpState::Incomplete => None,
            }
        } else {
            None
        }
    }

    /// Insert or update a neighbor cache entry.
    pub fn update(&mut self, addr: Ipv6Address, mac: MacAddress, state: NdpState) {
        let now = current_tick();

        // Evict oldest if at capacity
        if self.entries.len() >= NDP_CACHE_MAX && !self.entries.contains_key(&addr) {
            let oldest_key = self
                .entries
                .iter()
                .min_by_key(|(_, e)| e.timestamp)
                .map(|(k, _)| *k);
            if let Some(key) = oldest_key {
                self.entries.remove(&key);
            }
        }

        self.entries.insert(
            addr,
            NdpEntry {
                mac,
                state,
                timestamp: now,
                probe_count: 0,
            },
        );
    }

    /// Mark an entry as Incomplete (address resolution started).
    pub fn mark_incomplete(&mut self, addr: Ipv6Address) {
        let now = current_tick();
        self.entries.insert(
            addr,
            NdpEntry {
                mac: MacAddress::ZERO,
                state: NdpState::Incomplete,
                timestamp: now,
                probe_count: 1,
            },
        );
    }

    /// Transition a Reachable entry to Stale if its reachable time has expired.
    pub fn age_entries(&mut self) {
        let now = current_tick();
        for entry in self.entries.values_mut() {
            match entry.state {
                NdpState::Reachable => {
                    if now.wrapping_sub(entry.timestamp) > NDP_REACHABLE_TIME {
                        entry.state = NdpState::Stale;
                    }
                }
                NdpState::Stale => {
                    if now.wrapping_sub(entry.timestamp) > NDP_STALE_TIMEOUT {
                        entry.state = NdpState::Incomplete;
                    }
                }
                _ => {}
            }
        }
    }

    /// Remove an entry from the cache.
    pub fn remove(&mut self, addr: &Ipv6Address) {
        self.entries.remove(addr);
    }

    /// Get all cache entries for display.
    pub fn get_entries(&self) -> Vec<(Ipv6Address, MacAddress, NdpState)> {
        self.entries
            .iter()
            .map(|(addr, entry)| (*addr, entry.mac, entry.state))
            .collect()
    }

    /// Clear the entire cache.
    pub fn flush(&mut self) {
        self.entries.clear();
    }

    /// Number of entries in the cache.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Check if the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

// ============================================================================
// NDP Message Construction
// ============================================================================

/// Build a Neighbor Solicitation message.
///
/// Used to discover the link-layer address of a neighbor or verify
/// reachability. Includes the ICMPv6 header (type 135) + target address +
/// source link-layer option.
pub fn ndp_solicit(src: &Ipv6Address, target: &Ipv6Address, src_mac: &MacAddress) -> Vec<u8> {
    // ICMPv6 Neighbor Solicitation:
    //   Type (1) + Code (1) + Checksum (2) + Reserved (4) + Target Address (16)
    //   + Source Link-Layer Address Option (8)
    let mut msg = Vec::with_capacity(32);

    // ICMPv6 header
    msg.push(ICMPV6_NEIGHBOR_SOLICIT); // Type
    msg.push(0); // Code
    msg.extend_from_slice(&[0u8; 2]); // Checksum (filled later)
    msg.extend_from_slice(&[0u8; 4]); // Reserved

    // Target address
    msg.extend_from_slice(&target.0);

    // Source Link-Layer Address option (Type=1, Length=1 (in units of 8 bytes))
    msg.push(NDP_OPT_SOURCE_LINK_ADDR);
    msg.push(1); // Length in 8-byte units
    msg.extend_from_slice(&src_mac.0);

    // Compute and fill checksum
    let dst = solicited_node_multicast(target);
    let checksum = compute_icmpv6_checksum(&src.0, &dst.0, &msg);
    msg[2] = (checksum >> 8) as u8;
    msg[3] = (checksum & 0xff) as u8;

    msg
}

/// Build a Neighbor Advertisement message.
///
/// Sent in response to a Neighbor Solicitation, or unsolicited to announce
/// changes.
pub fn ndp_advertise(
    src: &Ipv6Address,
    dst: &Ipv6Address,
    target: &Ipv6Address,
    mac: &MacAddress,
    solicited: bool,
    override_flag: bool,
) -> Vec<u8> {
    // ICMPv6 Neighbor Advertisement:
    //   Type (1) + Code (1) + Checksum (2) + Flags+Reserved (4) + Target Address
    // (16)
    //   + Target Link-Layer Address Option (8)
    let mut msg = Vec::with_capacity(32);

    // ICMPv6 header
    msg.push(ICMPV6_NEIGHBOR_ADVERT); // Type
    msg.push(0); // Code
    msg.extend_from_slice(&[0u8; 2]); // Checksum (filled later)

    // Flags: R (Router) = 0, S (Solicited), O (Override)
    let mut flags: u8 = 0;
    if solicited {
        flags |= 0x40; // S flag
    }
    if override_flag {
        flags |= 0x20; // O flag
    }
    msg.push(flags);
    msg.extend_from_slice(&[0u8; 3]); // Rest of reserved field

    // Target address
    msg.extend_from_slice(&target.0);

    // Target Link-Layer Address option (Type=2, Length=1)
    msg.push(NDP_OPT_TARGET_LINK_ADDR);
    msg.push(1); // Length in 8-byte units
    msg.extend_from_slice(&mac.0);

    // Compute and fill checksum
    let checksum = compute_icmpv6_checksum(&src.0, &dst.0, &msg);
    msg[2] = (checksum >> 8) as u8;
    msg[3] = (checksum & 0xff) as u8;

    msg
}

/// Build a Router Solicitation message.
///
/// Sent by hosts at startup to request Router Advertisement from routers.
pub fn ndp_router_solicit(src: &Ipv6Address, src_mac: &MacAddress) -> Vec<u8> {
    // ICMPv6 Router Solicitation:
    //   Type (1) + Code (1) + Checksum (2) + Reserved (4)
    //   + Source Link-Layer Address Option (8)
    let mut msg = Vec::with_capacity(16);

    msg.push(ICMPV6_ROUTER_SOLICIT); // Type
    msg.push(0); // Code
    msg.extend_from_slice(&[0u8; 2]); // Checksum (filled later)
    msg.extend_from_slice(&[0u8; 4]); // Reserved

    // Source Link-Layer Address option
    if !is_unspecified(src) {
        msg.push(NDP_OPT_SOURCE_LINK_ADDR);
        msg.push(1); // Length in 8-byte units
        msg.extend_from_slice(&src_mac.0);
    }

    // Compute and fill checksum
    let dst = Ipv6Address::ALL_ROUTERS_LINK_LOCAL;
    let checksum = compute_icmpv6_checksum(&src.0, &dst.0, &msg);
    msg[2] = (checksum >> 8) as u8;
    msg[3] = (checksum & 0xff) as u8;

    msg
}

/// Handle an incoming NDP message.
///
/// Processes Neighbor Solicitations, Neighbor Advertisements,
/// Router Solicitations, and Router Advertisements.
///
/// Returns an optional response packet (ICMPv6 payload, not wrapped in IPv6).
pub fn handle_ndp(
    src_addr: &Ipv6Address,
    dst_addr: &Ipv6Address,
    data: &[u8],
) -> Result<Option<Vec<u8>>, KernelError> {
    if data.len() < 4 {
        return Err(KernelError::InvalidArgument {
            name: "ndp_message",
            value: "too_short",
        });
    }

    let icmp_type = data[0];

    match icmp_type {
        ICMPV6_NEIGHBOR_SOLICIT => handle_neighbor_solicitation(src_addr, dst_addr, data),
        ICMPV6_NEIGHBOR_ADVERT => handle_neighbor_advertisement(src_addr, data),
        ICMPV6_ROUTER_SOLICIT => {
            // We are not a router -- ignore
            Ok(None)
        }
        ICMPV6_ROUTER_ADVERT => handle_router_advertisement(src_addr, data),
        _ => {
            // Unknown NDP type, ignore
            Ok(None)
        }
    }
}

/// Process an incoming Neighbor Solicitation.
fn handle_neighbor_solicitation(
    src_addr: &Ipv6Address,
    _dst_addr: &Ipv6Address,
    data: &[u8],
) -> Result<Option<Vec<u8>>, KernelError> {
    // NS format: Type(1) + Code(1) + Checksum(2) + Reserved(4) + Target(16) +
    // Options...
    if data.len() < 24 {
        return Err(KernelError::InvalidArgument {
            name: "ndp_ns",
            value: "too_short",
        });
    }

    let mut target = [0u8; 16];
    target.copy_from_slice(&data[8..24]);
    let target_addr = Ipv6Address(target);

    // Extract source link-layer address option if present
    let source_mac = parse_link_layer_option(&data[24..], NDP_OPT_SOURCE_LINK_ADDR);

    // Update NDP cache with sender's info
    if let Some(mac) = source_mac {
        if !is_unspecified(src_addr) {
            IPV6_STATE.with_mut(|state| {
                let mut s = state.write();
                s.ndp_cache.update(*src_addr, mac, NdpState::Stale);
            });
        }
    }

    // Check if the target address is one of our addresses
    let is_our_addr = IPV6_STATE
        .with(|state| {
            let s = state.read();
            s.config
                .ipv6_addresses
                .iter()
                .any(|a| a.address == target_addr)
        })
        .unwrap_or(false);

    if !is_our_addr {
        return Ok(None);
    }

    // Build Neighbor Advertisement response
    let our_mac = get_interface_mac();
    let reply_dst = if is_unspecified(src_addr) {
        Ipv6Address::ALL_NODES_LINK_LOCAL
    } else {
        *src_addr
    };

    let na = ndp_advertise(
        &target_addr,
        &reply_dst,
        &target_addr,
        &our_mac,
        !is_unspecified(src_addr), // solicited flag
        true,                      // override flag
    );

    Ok(Some(na))
}

/// Process an incoming Neighbor Advertisement.
fn handle_neighbor_advertisement(
    _src_addr: &Ipv6Address,
    data: &[u8],
) -> Result<Option<Vec<u8>>, KernelError> {
    // NA format: Type(1) + Code(1) + Checksum(2) + Flags(1) + Reserved(3) +
    // Target(16) + Options
    if data.len() < 24 {
        return Err(KernelError::InvalidArgument {
            name: "ndp_na",
            value: "too_short",
        });
    }

    let _flags = data[4];

    let mut target = [0u8; 16];
    target.copy_from_slice(&data[8..24]);
    let target_addr = Ipv6Address(target);

    // Extract target link-layer address option
    let target_mac = parse_link_layer_option(&data[24..], NDP_OPT_TARGET_LINK_ADDR);

    // Update NDP cache
    if let Some(mac) = target_mac {
        IPV6_STATE.with_mut(|state| {
            let mut s = state.write();
            s.ndp_cache.update(target_addr, mac, NdpState::Reachable);
        });
    }

    Ok(None)
}

/// Process an incoming Router Advertisement.
fn handle_router_advertisement(
    src_addr: &Ipv6Address,
    data: &[u8],
) -> Result<Option<Vec<u8>>, KernelError> {
    // RA format: Type(1) + Code(1) + Checksum(2) + Hop Limit(1) + Flags(1)
    //            + Router Lifetime(2) + Reachable Time(4) + Retrans Timer(4) +
    //              Options...
    if data.len() < 16 {
        return Err(KernelError::InvalidArgument {
            name: "ndp_ra",
            value: "too_short",
        });
    }

    let cur_hop_limit = data[4];
    let _flags = data[5];
    let router_lifetime = u16::from_be_bytes([data[6], data[7]]);

    // Update hop limit if specified
    if cur_hop_limit != 0 {
        IPV6_STATE.with_mut(|state| {
            let mut s = state.write();
            s.hop_limit = cur_hop_limit;
        });
    }

    // Update NDP cache for the router
    let source_mac = parse_link_layer_option(&data[16..], NDP_OPT_SOURCE_LINK_ADDR);
    if let Some(mac) = source_mac {
        IPV6_STATE.with_mut(|state| {
            let mut s = state.write();
            s.ndp_cache.update(*src_addr, mac, NdpState::Reachable);
        });
    }

    // Parse prefix information options for SLAAC
    parse_prefix_options(&data[16..], router_lifetime);

    Ok(None)
}

/// Parse NDP options to find a link-layer address option of the given type.
fn parse_link_layer_option(options: &[u8], opt_type: u8) -> Option<MacAddress> {
    let mut offset = 0;
    while offset + 2 <= options.len() {
        let otype = options[offset];
        let olen = options[offset + 1] as usize;
        if olen == 0 {
            break; // Prevent infinite loop on malformed options
        }
        let opt_bytes = olen * 8;
        if offset + opt_bytes > options.len() {
            break;
        }
        if otype == opt_type && opt_bytes >= 8 {
            let mut mac_bytes = [0u8; 6];
            mac_bytes.copy_from_slice(&options[offset + 2..offset + 8]);
            return Some(MacAddress(mac_bytes));
        }
        offset += opt_bytes;
    }
    None
}

/// Parse prefix information options from a Router Advertisement.
fn parse_prefix_options(options: &[u8], _router_lifetime: u16) {
    let mut offset = 0;
    while offset + 2 <= options.len() {
        let otype = options[offset];
        let olen = options[offset + 1] as usize;
        if olen == 0 {
            break;
        }
        let opt_bytes = olen * 8;
        if offset + opt_bytes > options.len() {
            break;
        }

        if otype == NDP_OPT_PREFIX_INFO && opt_bytes >= 32 {
            let prefix_len = options[offset + 2];
            let flags = options[offset + 3];
            let autonomous = (flags & 0x40) != 0;

            if autonomous && prefix_len == 64 {
                let mut prefix = [0u8; 16];
                prefix.copy_from_slice(&options[offset + 16..offset + 32]);

                // Perform SLAAC: combine prefix with EUI-64 interface ID
                let our_mac = get_interface_mac();
                let ll = link_local_from_mac(&our_mac);
                // Copy interface ID from link-local (bytes 8..16)
                let mut addr_bytes = prefix;
                addr_bytes[8..16].copy_from_slice(&ll.0[8..16]);
                let new_addr = Ipv6Address(addr_bytes);

                IPV6_STATE.with_mut(|state| {
                    let mut s = state.write();
                    let exists = s
                        .config
                        .ipv6_addresses
                        .iter()
                        .any(|a| a.address == new_addr);
                    if !exists {
                        s.config.ipv6_addresses.push(Ipv6InterfaceAddr {
                            address: new_addr,
                            prefix_len,
                            scope: Ipv6Scope::Global,
                        });
                        println!(
                            "[IPv6] SLAAC: configured global address {}",
                            format_ipv6_compressed(&new_addr)
                        );
                    }
                });
            }
        }

        offset += opt_bytes;
    }
}

// ============================================================================
// ICMPv6 Checksum
// ============================================================================

/// Compute ICMPv6 checksum using the IPv6 pseudo-header.
///
/// The pseudo-header includes: source address (16), destination address (16),
/// upper-layer packet length (4), and next header (4, zero-padded).
pub fn compute_icmpv6_checksum(src: &[u8; 16], dst: &[u8; 16], data: &[u8]) -> u16 {
    let mut sum: u32 = 0;

    // Pseudo-header: source address
    for chunk in src.chunks(2) {
        sum += u16::from_be_bytes([chunk[0], chunk[1]]) as u32;
    }

    // Pseudo-header: destination address
    for chunk in dst.chunks(2) {
        sum += u16::from_be_bytes([chunk[0], chunk[1]]) as u32;
    }

    // Pseudo-header: upper-layer packet length (32-bit)
    let length = data.len() as u32;
    sum += length >> 16;
    sum += length & 0xFFFF;

    // Pseudo-header: next header (ICMPv6 = 58), zero-padded to 32 bits
    sum += NEXT_HEADER_ICMPV6 as u32;

    // ICMPv6 data (with checksum field zeroed -- caller must have zeroed bytes
    // [2..4])
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

    !(sum as u16)
}

// ============================================================================
// Dual-Stack Configuration
// ============================================================================

/// IPv6 address scope
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ipv6Scope {
    /// Link-local (fe80::/10)
    LinkLocal,
    /// Global unicast (2000::/3)
    Global,
    /// Site-local (deprecated, fec0::/10)
    SiteLocal,
}

/// IPv6 interface address with prefix length and scope
#[derive(Debug, Clone)]
pub struct Ipv6InterfaceAddr {
    /// The IPv6 address
    pub address: Ipv6Address,
    /// Prefix length (e.g., 64, 128)
    pub prefix_len: u8,
    /// Address scope
    pub scope: Ipv6Scope,
}

/// Dual-stack network configuration
#[derive(Debug, Clone)]
pub struct DualStackConfig {
    /// Whether IPv4 is enabled on this interface
    pub ipv4_enabled: bool,
    /// Whether IPv6 is enabled on this interface
    pub ipv6_enabled: bool,
    /// Whether to prefer IPv6 for dual-stack connections
    pub prefer_ipv6: bool,
    /// List of configured IPv6 addresses on this interface
    pub ipv6_addresses: Vec<Ipv6InterfaceAddr>,
}

impl DualStackConfig {
    /// Create a new dual-stack configuration with defaults
    pub fn new() -> Self {
        Self {
            ipv4_enabled: true,
            ipv6_enabled: true,
            prefer_ipv6: true,
            ipv6_addresses: Vec::new(),
        }
    }

    /// Get the primary link-local address, if any
    pub fn link_local_addr(&self) -> Option<&Ipv6InterfaceAddr> {
        self.ipv6_addresses
            .iter()
            .find(|a| a.scope == Ipv6Scope::LinkLocal)
    }

    /// Get the primary global address, if any
    pub fn global_addr(&self) -> Option<&Ipv6InterfaceAddr> {
        self.ipv6_addresses
            .iter()
            .find(|a| a.scope == Ipv6Scope::Global)
    }

    /// Get all addresses of a given scope
    pub fn addresses_by_scope(&self, scope: Ipv6Scope) -> Vec<&Ipv6InterfaceAddr> {
        self.ipv6_addresses
            .iter()
            .filter(|a| a.scope == scope)
            .collect()
    }
}

impl Default for DualStackConfig {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Global IPv6 State
// ============================================================================

/// Complete IPv6 subsystem state
pub struct Ipv6State {
    /// Dual-stack configuration
    pub config: DualStackConfig,
    /// NDP neighbor cache
    pub ndp_cache: NdpCache,
    /// Default hop limit for outgoing packets
    pub hop_limit: u8,
}

/// Global IPv6 state (protected by RwLock for concurrent access)
static IPV6_STATE: GlobalState<RwLock<Ipv6State>> = GlobalState::new();

/// Monotonic tick counter for NDP cache aging
static NDP_TICK: AtomicU64 = AtomicU64::new(0);

/// Get the current NDP tick value
fn current_tick() -> u64 {
    NDP_TICK.load(Ordering::Relaxed)
}

/// Advance the NDP tick counter (called periodically by timer)
pub fn tick() {
    NDP_TICK.fetch_add(1, Ordering::Relaxed);
}

// ============================================================================
// Public API
// ============================================================================

/// Initialize the IPv6 subsystem.
///
/// Creates the global state with a link-local address derived from the
/// primary network interface's MAC address.
pub fn init() -> Result<(), KernelError> {
    println!("[IPv6] Initializing IPv6 subsystem...");

    let our_mac = get_interface_mac();
    let link_local = link_local_from_mac(&our_mac);

    let mut config = DualStackConfig::new();
    config.ipv6_addresses.push(Ipv6InterfaceAddr {
        address: link_local,
        prefix_len: 10,
        scope: Ipv6Scope::LinkLocal,
    });

    let state = Ipv6State {
        config,
        ndp_cache: NdpCache::new(),
        hop_limit: DEFAULT_HOP_LIMIT,
    };

    IPV6_STATE
        .init(RwLock::new(state))
        .map_err(|_| KernelError::AlreadyExists {
            resource: "ipv6_state",
            id: 0,
        })?;

    println!(
        "[IPv6] Link-local address: {}",
        format_ipv6_compressed(&link_local)
    );
    println!("[IPv6] IPv6 subsystem initialized");

    Ok(())
}

/// Look up a neighbor's MAC address in the NDP cache.
pub fn ndp_lookup(addr: &Ipv6Address) -> Option<MacAddress> {
    IPV6_STATE
        .with(|state| {
            let s = state.read();
            s.ndp_cache.lookup(addr)
        })
        .flatten()
}

/// Get the current dual-stack configuration.
pub fn get_config() -> Option<DualStackConfig> {
    IPV6_STATE.with(|state| {
        let s = state.read();
        s.config.clone()
    })
}

/// Get NDP cache entries for display.
pub fn get_ndp_entries() -> Vec<(Ipv6Address, MacAddress, NdpState)> {
    IPV6_STATE
        .with(|state| {
            let s = state.read();
            s.ndp_cache.get_entries()
        })
        .unwrap_or_default()
}

/// Flush the NDP cache.
pub fn flush_ndp_cache() {
    IPV6_STATE.with_mut(|state| {
        let mut s = state.write();
        s.ndp_cache.flush();
    });
}

/// Get the current hop limit.
pub fn get_hop_limit() -> u8 {
    IPV6_STATE
        .with(|state| {
            let s = state.read();
            s.hop_limit
        })
        .unwrap_or(DEFAULT_HOP_LIMIT)
}

/// Send an IPv6 packet.
///
/// Wraps the payload in an IPv6 header, resolves the destination MAC via NDP
/// (or uses multicast MAC), and transmits via the Ethernet layer.
pub fn send(
    src: &Ipv6Address,
    dst: &Ipv6Address,
    next_header: u8,
    payload: &[u8],
) -> Result<(), KernelError> {
    let packet = build_ipv6(src, dst, next_header, payload);

    // Resolve destination MAC address
    let dst_mac = if is_multicast(dst) {
        multicast_mac(dst)
    } else {
        ndp_lookup(dst).unwrap_or_else(|| {
            // Start NDP resolution and use multicast for now
            let src_mac = get_interface_mac();
            let ns = ndp_solicit(src, dst, &src_mac);
            let sol_dst = solicited_node_multicast(dst);
            let ns_packet = build_ipv6(src, &sol_dst, NEXT_HEADER_ICMPV6, &ns);
            let sol_mac = multicast_mac(&sol_dst);

            // Transmit NS packet
            let frame =
                super::ethernet::construct_frame(sol_mac, src_mac, ETHERTYPE_IPV6, &ns_packet);
            let pkt = super::Packet::from_bytes(&frame);
            super::device::with_device_mut("eth0", |dev| {
                let _ = dev.transmit(&pkt);
            });

            // Mark as incomplete in the cache
            IPV6_STATE.with_mut(|state| {
                let mut s = state.write();
                s.ndp_cache.mark_incomplete(*dst);
            });

            // Use broadcast for now; the response will update the cache
            MacAddress::BROADCAST
        })
    };

    let src_mac = get_interface_mac();
    let frame = super::ethernet::construct_frame(dst_mac, src_mac, ETHERTYPE_IPV6, &packet);
    let pkt = super::Packet::from_bytes(&frame);
    super::device::with_device_mut("eth0", |dev| {
        let _ = dev.transmit(&pkt);
    });

    super::update_stats_tx(IPV6_HEADER_SIZE + payload.len());

    Ok(())
}

/// Get the source address appropriate for a given destination.
///
/// If the destination is link-local, returns our link-local address.
/// Otherwise, returns a global address if available.
pub fn select_source_address(dst: &Ipv6Address) -> Option<Ipv6Address> {
    IPV6_STATE
        .with(|state| {
            let s = state.read();
            if is_link_local(dst) || is_multicast(dst) {
                s.config.link_local_addr().map(|a| a.address)
            } else {
                s.config
                    .global_addr()
                    .or_else(|| s.config.link_local_addr())
                    .map(|a| a.address)
            }
        })
        .flatten()
}

/// Process an incoming IPv6 packet (dispatched from the Ethernet layer).
pub fn process_packet(data: &[u8]) -> Result<(), KernelError> {
    let (header, payload) = parse_ipv6(data)?;

    let src = Ipv6Address(header.source);
    let dst = Ipv6Address(header.destination);

    match header.next_header {
        NEXT_HEADER_ICMPV6 => {
            super::icmpv6::handle_icmpv6(&src, &dst, payload)?;
        }
        NEXT_HEADER_TCP => {
            let src_ip = super::IpAddress::V6(src);
            let dst_ip = super::IpAddress::V6(dst);
            let _ = super::tcp::process_packet(src_ip, dst_ip, payload);
        }
        NEXT_HEADER_UDP => {
            let src_ip = super::IpAddress::V6(src);
            let dst_ip = super::IpAddress::V6(dst);
            let _ = super::udp::process_packet(src_ip, dst_ip, payload);
        }
        _ => {
            // Unknown next header -- silently drop
        }
    }

    Ok(())
}

// ============================================================================
// Internal Helpers
// ============================================================================

/// Get the MAC address of the primary network interface.
fn get_interface_mac() -> MacAddress {
    super::device::with_device("eth0", |dev| dev.mac_address()).unwrap_or(MacAddress::ZERO)
}

// ============================================================================
// IPv6 Statistics
// ============================================================================

/// IPv6 statistics
#[derive(Debug, Clone, Copy, Default)]
pub struct Ipv6Stats {
    /// Number of IPv6 addresses configured
    pub addresses_configured: usize,
    /// Number of NDP cache entries
    pub ndp_cache_entries: usize,
    /// Current hop limit
    pub hop_limit: u8,
    /// Whether dual-stack is active
    pub dual_stack_active: bool,
}

/// Get IPv6 statistics.
pub fn get_stats() -> Ipv6Stats {
    IPV6_STATE
        .with(|state| {
            let s = state.read();
            Ipv6Stats {
                addresses_configured: s.config.ipv6_addresses.len(),
                ndp_cache_entries: s.ndp_cache.len(),
                hop_limit: s.hop_limit,
                dual_stack_active: s.config.ipv4_enabled && s.config.ipv6_enabled,
            }
        })
        .unwrap_or_default()
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ipv6_header_roundtrip() {
        let src = Ipv6Address([
            0xfe, 0x80, 0, 0, 0, 0, 0, 0, 0x02, 0x00, 0x00, 0xff, 0xfe, 0x00, 0x00, 0x01,
        ]);
        let dst = Ipv6Address([
            0xfe, 0x80, 0, 0, 0, 0, 0, 0, 0x02, 0x00, 0x00, 0xff, 0xfe, 0x00, 0x00, 0x02,
        ]);
        let payload = b"Hello IPv6!";
        let packet = build_ipv6(&src, &dst, NEXT_HEADER_TCP, payload);

        let (header, parsed_payload) = parse_ipv6(&packet).unwrap();
        assert_eq!(header.version(), IPV6_VERSION);
        assert_eq!(header.next_header, NEXT_HEADER_TCP);
        assert_eq!(header.hop_limit, DEFAULT_HOP_LIMIT);
        assert_eq!(header.source, src.0);
        assert_eq!(header.destination, dst.0);
        assert_eq!(parsed_payload, payload);
    }

    #[test]
    fn test_ipv6_parse_too_short() {
        let short = [0u8; 10];
        assert!(parse_ipv6(&short).is_err());
    }

    #[test]
    fn test_ipv6_parse_wrong_version() {
        let mut packet = [0u8; IPV6_HEADER_SIZE];
        // Set version to 4 instead of 6
        packet[0] = 0x40;
        assert!(parse_ipv6(&packet).is_err());
    }

    #[test]
    fn test_is_link_local() {
        let ll = Ipv6Address([0xfe, 0x80, 0, 0, 0, 0, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8]);
        assert!(is_link_local(&ll));

        let global = Ipv6Address([0x20, 0x01, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]);
        assert!(!is_link_local(&global));
    }

    #[test]
    fn test_is_multicast() {
        let mc = Ipv6Address([0xff, 0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]);
        assert!(is_multicast(&mc));
        assert!(!is_multicast(&Ipv6Address::LOCALHOST));
    }

    #[test]
    fn test_is_loopback() {
        assert!(is_loopback(&Ipv6Address::LOCALHOST));
        assert!(!is_loopback(&Ipv6Address::UNSPECIFIED));
    }

    #[test]
    fn test_is_unspecified() {
        assert!(is_unspecified(&Ipv6Address::UNSPECIFIED));
        assert!(!is_unspecified(&Ipv6Address::LOCALHOST));
    }

    #[test]
    fn test_is_global_unicast() {
        let global = Ipv6Address([0x20, 0x01, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]);
        assert!(is_global_unicast(&global));
        assert!(!is_global_unicast(&Ipv6Address::LOCALHOST));
    }

    #[test]
    fn test_solicited_node_multicast() {
        let addr = Ipv6Address([
            0xfe, 0x80, 0, 0, 0, 0, 0, 0, 0x02, 0x00, 0x00, 0xff, 0xfe, 0xab, 0xcd, 0xef,
        ]);
        let sol = solicited_node_multicast(&addr);
        assert_eq!(sol.0[0], 0xff);
        assert_eq!(sol.0[1], 0x02);
        assert_eq!(sol.0[11], 0x01);
        assert_eq!(sol.0[12], 0xff);
        assert_eq!(sol.0[13], 0xab);
        assert_eq!(sol.0[14], 0xcd);
        assert_eq!(sol.0[15], 0xef);
    }

    #[test]
    fn test_link_local_from_mac() {
        let mac = MacAddress([0x52, 0x54, 0x00, 0x12, 0x34, 0x56]);
        let ll = link_local_from_mac(&mac);
        assert_eq!(ll.0[0], 0xfe);
        assert_eq!(ll.0[1], 0x80);
        assert_eq!(ll.0[8], 0x52 ^ 0x02);
        assert_eq!(ll.0[9], 0x54);
        assert_eq!(ll.0[10], 0x00);
        assert_eq!(ll.0[11], 0xff);
        assert_eq!(ll.0[12], 0xfe);
        assert_eq!(ll.0[13], 0x12);
        assert_eq!(ll.0[14], 0x34);
        assert_eq!(ll.0[15], 0x56);
        assert!(is_link_local(&ll));
    }

    #[test]
    fn test_multicast_mac() {
        let mc = Ipv6Address([0xff, 0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]);
        let mac = multicast_mac(&mc);
        assert_eq!(mac.0[0], 0x33);
        assert_eq!(mac.0[1], 0x33);
        assert_eq!(mac.0[2], 0x00);
        assert_eq!(mac.0[3], 0x00);
        assert_eq!(mac.0[4], 0x00);
        assert_eq!(mac.0[5], 0x01);
    }

    #[test]
    fn test_format_ipv6() {
        let addr = Ipv6Address::LOCALHOST;
        let formatted = format_ipv6(&addr);
        assert_eq!(formatted, "0:0:0:0:0:0:0:1");
    }

    #[test]
    fn test_format_ipv6_compressed_loopback() {
        let addr = Ipv6Address::LOCALHOST;
        let formatted = format_ipv6_compressed(&addr);
        assert_eq!(formatted, "::1");
    }

    #[test]
    fn test_format_ipv6_compressed_unspecified() {
        let addr = Ipv6Address::UNSPECIFIED;
        let formatted = format_ipv6_compressed(&addr);
        assert_eq!(formatted, "::");
    }

    #[test]
    fn test_icmpv6_checksum() {
        let src = [0u8; 16];
        let dst = [0u8; 16];
        let data = [0u8; 4];
        let cksum = compute_icmpv6_checksum(&src, &dst, &data);
        assert_ne!(cksum, 0);
    }

    #[test]
    fn test_ndp_cache_basic() {
        let mut cache = NdpCache::new();
        let addr = Ipv6Address([0xfe, 0x80, 0, 0, 0, 0, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8]);
        let mac = MacAddress([0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF]);

        assert!(cache.lookup(&addr).is_none());

        cache.update(addr, mac, NdpState::Reachable);
        assert_eq!(cache.lookup(&addr), Some(mac));
        assert_eq!(cache.len(), 1);

        cache.flush();
        assert!(cache.is_empty());
    }

    #[test]
    fn test_ndp_cache_incomplete() {
        let mut cache = NdpCache::new();
        let addr = Ipv6Address([0xfe, 0x80, 0, 0, 0, 0, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8]);

        cache.mark_incomplete(addr);
        assert!(cache.lookup(&addr).is_none());
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn test_dual_stack_config() {
        let mut config = DualStackConfig::new();
        assert!(config.ipv4_enabled);
        assert!(config.ipv6_enabled);
        assert!(config.prefer_ipv6);
        assert!(config.link_local_addr().is_none());

        config.ipv6_addresses.push(Ipv6InterfaceAddr {
            address: Ipv6Address([0xfe, 0x80, 0, 0, 0, 0, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8]),
            prefix_len: 10,
            scope: Ipv6Scope::LinkLocal,
        });

        assert!(config.link_local_addr().is_some());
        assert!(config.global_addr().is_none());
    }

    #[test]
    fn test_is_ipv4_mapped() {
        let mapped = Ipv6Address([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff, 192, 168, 1, 1]);
        assert!(is_ipv4_mapped(&mapped));
        assert!(!is_ipv4_mapped(&Ipv6Address::LOCALHOST));
    }
}
