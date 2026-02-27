//! ARP (Address Resolution Protocol) implementation
//!
//! Provides ARP cache management and request/reply processing for
//! resolving IPv4 addresses to MAC addresses on the local network.

#![allow(dead_code)] // Phase 6 network stack -- functions called as stack matures

use alloc::{collections::BTreeMap, vec::Vec};

use spin::Mutex;

use crate::{
    error::KernelError,
    net::{Ipv4Address, MacAddress},
};

/// ARP hardware type: Ethernet
const ARP_HTYPE_ETHERNET: u16 = 1;
/// ARP protocol type: IPv4
const ARP_PTYPE_IPV4: u16 = 0x0800;
/// ARP operation: Request
const ARP_OP_REQUEST: u16 = 1;
/// ARP operation: Reply
const ARP_OP_REPLY: u16 = 2;
/// ARP header size for Ethernet/IPv4: 28 bytes
const ARP_PACKET_SIZE: usize = 28;

/// Maximum ARP cache entries
const ARP_CACHE_MAX: usize = 128;

/// ARP cache entry
#[derive(Debug, Clone)]
struct ArpEntry {
    mac: MacAddress,
    /// Tick count when this entry was created/refreshed
    timestamp: u64,
}

/// Global ARP cache: Ipv4Address -> MacAddress
static ARP_CACHE: Mutex<BTreeMap<Ipv4Address, ArpEntry>> = Mutex::new(BTreeMap::new());

/// Monotonic tick counter for cache aging
static ARP_TICK: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

/// Get current ARP tick (incremented by timer or manually)
fn current_tick() -> u64 {
    ARP_TICK.load(core::sync::atomic::Ordering::Relaxed)
}

/// Advance the ARP tick counter (called periodically)
pub fn tick() {
    ARP_TICK.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
}

/// Maximum age of an ARP entry in ticks before it is considered stale.
/// At ~1 tick/second this is about 5 minutes.
const ARP_ENTRY_MAX_AGE: u64 = 300;

/// Resolve an IPv4 address to a MAC address from the cache.
///
/// Returns `Some(MacAddress)` if the entry exists and is not stale,
/// otherwise `None` (caller should send an ARP request).
pub fn resolve(ip: Ipv4Address) -> Option<MacAddress> {
    let cache = ARP_CACHE.lock();
    if let Some(entry) = cache.get(&ip) {
        let age = current_tick().wrapping_sub(entry.timestamp);
        if age < ARP_ENTRY_MAX_AGE {
            return Some(entry.mac);
        }
    }
    None
}

/// Insert or update an ARP cache entry.
pub fn update_cache(ip: Ipv4Address, mac: MacAddress) {
    let mut cache = ARP_CACHE.lock();

    // Evict oldest entry if at capacity and this is a new key
    if cache.len() >= ARP_CACHE_MAX && !cache.contains_key(&ip) {
        // Remove the entry with the smallest (oldest) timestamp
        let oldest_key = cache
            .iter()
            .min_by_key(|(_, e)| e.timestamp)
            .map(|(k, _)| *k);
        if let Some(key) = oldest_key {
            cache.remove(&key);
        }
    }

    cache.insert(
        ip,
        ArpEntry {
            mac,
            timestamp: current_tick(),
        },
    );
}

/// Process an incoming ARP packet.
///
/// Handles both ARP requests (sends reply if the target is us) and
/// ARP replies (updates cache).
pub fn process_arp_packet(data: &[u8], our_mac: &MacAddress) -> Result<(), KernelError> {
    if data.len() < ARP_PACKET_SIZE {
        return Err(KernelError::InvalidArgument {
            name: "arp_packet",
            value: "too_short",
        });
    }

    let htype = u16::from_be_bytes([data[0], data[1]]);
    let ptype = u16::from_be_bytes([data[2], data[3]]);
    let hlen = data[4];
    let plen = data[5];
    let operation = u16::from_be_bytes([data[6], data[7]]);

    // Validate Ethernet/IPv4 ARP
    if htype != ARP_HTYPE_ETHERNET || ptype != ARP_PTYPE_IPV4 || hlen != 6 || plen != 4 {
        return Err(KernelError::InvalidArgument {
            name: "arp_format",
            value: "unsupported",
        });
    }

    // Parse sender and target addresses
    let mut sender_mac_bytes = [0u8; 6];
    sender_mac_bytes.copy_from_slice(&data[8..14]);
    let sender_mac = MacAddress(sender_mac_bytes);
    let sender_ip = Ipv4Address([data[14], data[15], data[16], data[17]]);

    let target_ip = Ipv4Address([data[24], data[25], data[26], data[27]]);

    // Always learn the sender's mapping
    update_cache(sender_ip, sender_mac);

    match operation {
        ARP_OP_REQUEST => {
            // Check if the request is for our IP
            let our_ip = get_interface_ip();
            if target_ip == our_ip {
                // Queue an ARP reply
                let reply = build_arp_reply(*our_mac, our_ip, sender_mac, sender_ip);
                send_arp_frame(&reply, *our_mac, sender_mac);
            }
        }
        ARP_OP_REPLY => {
            // Already updated cache above
            println!(
                "[ARP] Learned {}.{}.{}.{} -> {:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
                sender_ip.0[0],
                sender_ip.0[1],
                sender_ip.0[2],
                sender_ip.0[3],
                sender_mac.0[0],
                sender_mac.0[1],
                sender_mac.0[2],
                sender_mac.0[3],
                sender_mac.0[4],
                sender_mac.0[5],
            );
        }
        _ => {
            // Unknown operation, ignore
        }
    }

    Ok(())
}

/// Send an ARP request for the given target IP.
///
/// Broadcasts an ARP request on the local network.
pub fn send_arp_request(target_ip: Ipv4Address) {
    let our_mac = get_interface_mac();
    let our_ip = get_interface_ip();

    let packet = build_arp_request(our_mac, our_ip, target_ip);
    send_arp_frame(&packet, our_mac, MacAddress::BROADCAST);
}

/// Build a raw ARP request packet (28 bytes).
fn build_arp_request(
    sender_mac: MacAddress,
    sender_ip: Ipv4Address,
    target_ip: Ipv4Address,
) -> Vec<u8> {
    let mut pkt = Vec::with_capacity(ARP_PACKET_SIZE);

    // Hardware type: Ethernet
    pkt.extend_from_slice(&ARP_HTYPE_ETHERNET.to_be_bytes());
    // Protocol type: IPv4
    pkt.extend_from_slice(&ARP_PTYPE_IPV4.to_be_bytes());
    // Hardware address length
    pkt.push(6);
    // Protocol address length
    pkt.push(4);
    // Operation: Request
    pkt.extend_from_slice(&ARP_OP_REQUEST.to_be_bytes());
    // Sender hardware address
    pkt.extend_from_slice(&sender_mac.0);
    // Sender protocol address
    pkt.extend_from_slice(&sender_ip.0);
    // Target hardware address (zero for request)
    pkt.extend_from_slice(&[0u8; 6]);
    // Target protocol address
    pkt.extend_from_slice(&target_ip.0);

    pkt
}

/// Build a raw ARP reply packet (28 bytes).
fn build_arp_reply(
    sender_mac: MacAddress,
    sender_ip: Ipv4Address,
    target_mac: MacAddress,
    target_ip: Ipv4Address,
) -> Vec<u8> {
    let mut pkt = Vec::with_capacity(ARP_PACKET_SIZE);

    pkt.extend_from_slice(&ARP_HTYPE_ETHERNET.to_be_bytes());
    pkt.extend_from_slice(&ARP_PTYPE_IPV4.to_be_bytes());
    pkt.push(6);
    pkt.push(4);
    pkt.extend_from_slice(&ARP_OP_REPLY.to_be_bytes());
    pkt.extend_from_slice(&sender_mac.0);
    pkt.extend_from_slice(&sender_ip.0);
    pkt.extend_from_slice(&target_mac.0);
    pkt.extend_from_slice(&target_ip.0);

    pkt
}

/// Wrap an ARP packet in an Ethernet frame and transmit it.
fn send_arp_frame(arp_data: &[u8], src_mac: MacAddress, dst_mac: MacAddress) {
    let frame = super::ethernet::construct_frame(
        dst_mac,
        src_mac,
        super::ethernet::ETHERTYPE_ARP,
        arp_data,
    );

    // Transmit via the first available network device
    let _pkt = super::Packet::from_bytes(&frame);

    // Try to send through eth0-style device; fall back silently if unavailable
    super::device::with_device_mut("eth0", |dev| {
        let _ = dev.transmit(&_pkt);
    });
}

/// Get the currently configured interface IP address.
///
/// Reads from the IP interface configuration. Returns 0.0.0.0 if not
/// configured (pre-DHCP).
fn get_interface_ip() -> Ipv4Address {
    super::ip::get_interface_ip()
}

/// Get the MAC address of the primary network interface.
fn get_interface_mac() -> MacAddress {
    super::device::with_device("eth0", |dev| dev.mac_address()).unwrap_or(MacAddress::ZERO)
}

/// Get a snapshot of the ARP cache for display purposes.
pub fn get_cache_entries() -> Vec<(Ipv4Address, MacAddress)> {
    let cache = ARP_CACHE.lock();
    let now = current_tick();
    cache
        .iter()
        .filter(|(_, entry)| now.wrapping_sub(entry.timestamp) < ARP_ENTRY_MAX_AGE)
        .map(|(ip, entry)| (*ip, entry.mac))
        .collect()
}

/// Clear all ARP cache entries.
pub fn flush_cache() {
    let mut cache = ARP_CACHE.lock();
    cache.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_arp_cache_insert_and_resolve() {
        let ip = Ipv4Address::new(10, 0, 0, 1);
        let mac = MacAddress([0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF]);

        update_cache(ip, mac);
        assert_eq!(resolve(ip), Some(mac));
    }

    #[test]
    fn test_arp_request_build() {
        let sender_mac = MacAddress([0x52, 0x54, 0x00, 0x12, 0x34, 0x56]);
        let sender_ip = Ipv4Address::new(10, 0, 2, 15);
        let target_ip = Ipv4Address::new(10, 0, 2, 1);

        let pkt = build_arp_request(sender_mac, sender_ip, target_ip);
        assert_eq!(pkt.len(), ARP_PACKET_SIZE);

        // Check operation = Request (1)
        assert_eq!(u16::from_be_bytes([pkt[6], pkt[7]]), ARP_OP_REQUEST);
    }
}
