//! Ethernet frame parsing and construction
//!
//! Implements IEEE 802.3 Ethernet frame handling for the network stack.
//! Supports parsing incoming frames and constructing outgoing frames
//! with proper MAC addressing and EtherType identification.

#![allow(dead_code)] // Phase 6 network stack -- functions called as stack matures

use alloc::vec::Vec;

use crate::{error::KernelError, net::MacAddress};

/// Ethernet frame header size: dst(6) + src(6) + ethertype(2) = 14 bytes
pub const ETHERNET_HEADER_SIZE: usize = 14;

/// Minimum Ethernet frame payload (excluding header)
pub const ETHERNET_MIN_PAYLOAD: usize = 46;

/// Maximum Ethernet frame payload (standard MTU)
pub const ETHERNET_MAX_PAYLOAD: usize = 1500;

/// EtherType constants
pub const ETHERTYPE_IPV4: u16 = 0x0800;
pub const ETHERTYPE_ARP: u16 = 0x0806;
pub const ETHERTYPE_IPV6: u16 = 0x86DD;

/// Parsed Ethernet frame
#[derive(Debug, Clone)]
pub struct EthernetFrame<'a> {
    /// Destination MAC address
    pub dst_mac: MacAddress,
    /// Source MAC address
    pub src_mac: MacAddress,
    /// EtherType field
    pub ethertype: u16,
    /// Payload (reference to data after the header)
    pub payload: &'a [u8],
}

/// Parse an Ethernet frame from raw bytes.
///
/// Returns an `EthernetFrame` with references into the original buffer
/// for zero-copy payload access.
pub fn parse_frame(data: &[u8]) -> Result<EthernetFrame<'_>, KernelError> {
    if data.len() < ETHERNET_HEADER_SIZE {
        return Err(KernelError::InvalidArgument {
            name: "ethernet_frame",
            value: "too_short",
        });
    }

    let mut dst = [0u8; 6];
    let mut src = [0u8; 6];
    dst.copy_from_slice(&data[0..6]);
    src.copy_from_slice(&data[6..12]);
    let ethertype = u16::from_be_bytes([data[12], data[13]]);

    Ok(EthernetFrame {
        dst_mac: MacAddress(dst),
        src_mac: MacAddress(src),
        ethertype,
        payload: &data[ETHERNET_HEADER_SIZE..],
    })
}

/// Construct an Ethernet frame from components.
///
/// Builds a complete frame with header and payload, suitable for
/// transmission via a network device.
pub fn construct_frame(
    dst: MacAddress,
    src: MacAddress,
    ethertype: u16,
    payload: &[u8],
) -> Vec<u8> {
    let mut frame = Vec::with_capacity(ETHERNET_HEADER_SIZE + payload.len());

    // Destination MAC
    frame.extend_from_slice(&dst.0);
    // Source MAC
    frame.extend_from_slice(&src.0);
    // EtherType
    frame.extend_from_slice(&ethertype.to_be_bytes());
    // Payload
    frame.extend_from_slice(payload);

    frame
}

/// Check if a MAC address is a broadcast address (FF:FF:FF:FF:FF:FF)
pub fn is_broadcast(mac: &MacAddress) -> bool {
    *mac == MacAddress::BROADCAST
}

/// Check if a MAC address is an IPv6 multicast address (33:33:xx:xx:xx:xx)
pub fn is_ipv6_multicast(mac: &MacAddress) -> bool {
    mac.0[0] == 0x33 && mac.0[1] == 0x33
}

/// Check if a MAC address matches or is broadcast or is IPv6 multicast
pub fn is_for_us(frame_dst: &MacAddress, our_mac: &MacAddress) -> bool {
    *frame_dst == *our_mac || is_broadcast(frame_dst) || is_ipv6_multicast(frame_dst)
}

/// Dispatch a received Ethernet frame to the appropriate protocol handler.
///
/// Routes frames to ARP or IP based on the EtherType field.
pub fn dispatch_frame(data: &[u8], our_mac: &MacAddress) -> Result<(), KernelError> {
    let frame = parse_frame(data)?;

    // Drop frames not addressed to us
    if !is_for_us(&frame.dst_mac, our_mac) {
        return Ok(());
    }

    match frame.ethertype {
        ETHERTYPE_ARP => {
            super::arp::process_arp_packet(frame.payload, our_mac)?;
        }
        ETHERTYPE_IPV4 => {
            // Parse IP header to get protocol and addresses, then dispatch
            if frame.payload.len() >= super::ip::Ipv4Header::MIN_SIZE {
                let ip_header = super::ip::Ipv4Header::from_bytes(frame.payload)?;
                let header_len = (ip_header.ihl as usize) * 4;
                if frame.payload.len() >= header_len {
                    let ip_payload = &frame.payload[header_len..];
                    let src = super::IpAddress::V4(ip_header.source);
                    let dst = super::IpAddress::V4(ip_header.destination);

                    match ip_header.protocol {
                        6 => {
                            // TCP
                            let _ = super::tcp::process_packet(src, dst, ip_payload);
                        }
                        17 => {
                            // UDP
                            let _ = super::udp::process_packet(src, dst, ip_payload);
                        }
                        _ => {
                            // Unknown protocol, drop
                        }
                    }
                }
            }
        }
        ETHERTYPE_IPV6 => {
            // Parse and dispatch IPv6 packet
            super::ipv6::process_packet(frame.payload)?;
        }
        _ => {
            // Unknown EtherType, silently drop
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_construct_and_parse() {
        let dst = MacAddress([0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]);
        let src = MacAddress([0x52, 0x54, 0x00, 0x12, 0x34, 0x56]);
        let payload = b"Hello, Ethernet!";

        let frame = construct_frame(dst, src, ETHERTYPE_IPV4, payload);
        assert_eq!(frame.len(), ETHERNET_HEADER_SIZE + payload.len());

        let parsed = parse_frame(&frame).unwrap();
        assert_eq!(parsed.dst_mac, dst);
        assert_eq!(parsed.src_mac, src);
        assert_eq!(parsed.ethertype, ETHERTYPE_IPV4);
        assert_eq!(parsed.payload, payload);
    }

    #[test]
    fn test_parse_too_short() {
        let short = [0u8; 10];
        assert!(parse_frame(&short).is_err());
    }

    #[test]
    fn test_is_broadcast() {
        assert!(is_broadcast(&MacAddress::BROADCAST));
        assert!(!is_broadcast(&MacAddress::ZERO));
    }
}
