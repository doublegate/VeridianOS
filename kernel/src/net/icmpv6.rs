//! ICMPv6 protocol implementation
//!
//! Provides ICMPv6 message handling for IPv6, including echo request/reply
//! (ping6), destination unreachable, packet too big, time exceeded, and
//! integration with NDP for neighbor/router discovery.

#![allow(dead_code)] // Phase 7 network stack -- functions called as stack matures

use alloc::vec::Vec;

use super::Ipv6Address;
use crate::error::KernelError;

// ============================================================================
// ICMPv6 Message Type Constants
// ============================================================================

// Error messages (types 0-127)
/// Destination Unreachable
pub const ICMPV6_DEST_UNREACHABLE: u8 = 1;
/// Packet Too Big
pub const ICMPV6_PACKET_TOO_BIG: u8 = 2;
/// Time Exceeded
pub const ICMPV6_TIME_EXCEEDED: u8 = 3;
/// Parameter Problem
pub const ICMPV6_PARAMETER_PROBLEM: u8 = 4;

// Informational messages (types 128-255)
/// Echo Request (ping)
pub const ICMPV6_ECHO_REQUEST: u8 = 128;
/// Echo Reply (pong)
pub const ICMPV6_ECHO_REPLY: u8 = 129;

// NDP message types (handled by ipv6::handle_ndp)
/// Router Solicitation
pub const ICMPV6_ROUTER_SOLICIT: u8 = 133;
/// Router Advertisement
pub const ICMPV6_ROUTER_ADVERT: u8 = 134;
/// Neighbor Solicitation
pub const ICMPV6_NEIGHBOR_SOLICIT: u8 = 135;
/// Neighbor Advertisement
pub const ICMPV6_NEIGHBOR_ADVERT: u8 = 136;

// Destination Unreachable codes
/// No route to destination
pub const ICMPV6_NO_ROUTE: u8 = 0;
/// Communication with destination administratively prohibited
pub const ICMPV6_ADMIN_PROHIBITED: u8 = 1;
/// Beyond scope of source address
pub const ICMPV6_BEYOND_SCOPE: u8 = 2;
/// Address unreachable
pub const ICMPV6_ADDR_UNREACHABLE: u8 = 3;
/// Port unreachable
pub const ICMPV6_PORT_UNREACHABLE: u8 = 4;

// Time Exceeded codes
/// Hop limit exceeded in transit
pub const ICMPV6_HOP_LIMIT_EXCEEDED: u8 = 0;
/// Fragment reassembly time exceeded
pub const ICMPV6_FRAGMENT_REASSEMBLY_EXCEEDED: u8 = 1;

/// Minimum ICMPv6 header size (type + code + checksum = 4 bytes)
pub const ICMPV6_HEADER_SIZE: usize = 4;

/// ICMPv6 echo header size (type + code + checksum + id + seq = 8 bytes)
pub const ICMPV6_ECHO_HEADER_SIZE: usize = 8;

// ============================================================================
// ICMPv6 Header
// ============================================================================

/// ICMPv6 message header (4 bytes minimum)
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct Icmpv6Header {
    /// ICMPv6 message type
    pub icmp_type: u8,
    /// Type-specific code
    pub code: u8,
    /// Checksum (covers pseudo-header + ICMPv6 message)
    pub checksum: u16,
}

impl Icmpv6Header {
    /// Parse an ICMPv6 header from bytes
    pub fn from_bytes(data: &[u8]) -> Result<Self, KernelError> {
        if data.len() < ICMPV6_HEADER_SIZE {
            return Err(KernelError::InvalidArgument {
                name: "icmpv6_header",
                value: "too_short",
            });
        }

        Ok(Self {
            icmp_type: data[0],
            code: data[1],
            checksum: u16::from_be_bytes([data[2], data[3]]),
        })
    }

    /// Serialize to bytes
    pub fn to_bytes(&self) -> [u8; ICMPV6_HEADER_SIZE] {
        let mut bytes = [0u8; ICMPV6_HEADER_SIZE];
        bytes[0] = self.icmp_type;
        bytes[1] = self.code;
        bytes[2..4].copy_from_slice(&self.checksum.to_be_bytes());
        bytes
    }
}

// ============================================================================
// ICMPv6 Message Handling
// ============================================================================

/// Handle an incoming ICMPv6 message.
///
/// Dispatches to the appropriate handler based on message type:
/// - Echo Request -> builds Echo Reply
/// - NDP messages -> delegates to ipv6::handle_ndp
/// - Error messages -> logs and drops (informational for now)
///
/// Returns an optional response packet to send back (fully wrapped in IPv6).
pub fn handle_icmpv6(
    src: &Ipv6Address,
    dst: &Ipv6Address,
    data: &[u8],
) -> Result<Option<Vec<u8>>, KernelError> {
    if data.len() < ICMPV6_HEADER_SIZE {
        return Err(KernelError::InvalidArgument {
            name: "icmpv6_packet",
            value: "too_short",
        });
    }

    let header = Icmpv6Header::from_bytes(data)?;

    // Verify checksum
    if !verify_checksum(src, dst, data) {
        return Err(KernelError::InvalidArgument {
            name: "icmpv6_checksum",
            value: "invalid",
        });
    }

    match header.icmp_type {
        ICMPV6_ECHO_REQUEST => handle_echo_request(src, dst, data),
        ICMPV6_ECHO_REPLY => {
            handle_echo_reply(src, data);
            Ok(None)
        }
        ICMPV6_DEST_UNREACHABLE => {
            handle_dest_unreachable(src, header.code, data);
            Ok(None)
        }
        ICMPV6_PACKET_TOO_BIG => {
            handle_packet_too_big(src, data);
            Ok(None)
        }
        ICMPV6_TIME_EXCEEDED => {
            handle_time_exceeded(src, header.code, data);
            Ok(None)
        }
        // NDP messages (133-137) -- delegate to IPv6 NDP handler
        ICMPV6_ROUTER_SOLICIT
        | ICMPV6_ROUTER_ADVERT
        | ICMPV6_NEIGHBOR_SOLICIT
        | ICMPV6_NEIGHBOR_ADVERT => {
            if let Some(reply_icmpv6) = super::ipv6::handle_ndp(src, dst, data)? {
                // NDP handler returns the ICMPv6 payload; wrap in IPv6
                let reply_src =
                    super::ipv6::select_source_address(src).unwrap_or(Ipv6Address::UNSPECIFIED);
                let reply_packet = super::ipv6::build_ipv6(
                    &reply_src,
                    src,
                    super::ipv6::NEXT_HEADER_ICMPV6,
                    &reply_icmpv6,
                );
                // Transmit the NDP reply
                let _ = super::ipv6::send(
                    &reply_src,
                    src,
                    super::ipv6::NEXT_HEADER_ICMPV6,
                    &reply_icmpv6,
                );
                // We already transmitted, so don't return a packet
                let _ = reply_packet;
                Ok(None)
            } else {
                Ok(None)
            }
        }
        _ => {
            // Unknown ICMPv6 type -- silently drop
            Ok(None)
        }
    }
}

/// Handle an Echo Request (ping) by building and sending an Echo Reply.
fn handle_echo_request(
    src: &Ipv6Address,
    dst: &Ipv6Address,
    data: &[u8],
) -> Result<Option<Vec<u8>>, KernelError> {
    if data.len() < ICMPV6_ECHO_HEADER_SIZE {
        return Err(KernelError::InvalidArgument {
            name: "icmpv6_echo",
            value: "too_short",
        });
    }

    // Extract echo identifier and sequence number
    let id = u16::from_be_bytes([data[4], data[5]]);
    let seq = u16::from_be_bytes([data[6], data[7]]);
    let echo_data = &data[ICMPV6_ECHO_HEADER_SIZE..];

    // Determine our source address for the reply
    let reply_src = super::ipv6::select_source_address(src).unwrap_or(*dst);

    // Build and send echo reply
    let reply = build_echo_reply(&reply_src, src, id, seq, echo_data);

    // Send the reply via IPv6
    let _ = super::ipv6::send(&reply_src, src, super::ipv6::NEXT_HEADER_ICMPV6, &reply);

    Ok(None)
}

/// Handle an Echo Reply -- log the response for ping6 command.
fn handle_echo_reply(src: &Ipv6Address, data: &[u8]) {
    if data.len() >= ICMPV6_ECHO_HEADER_SIZE {
        let id = u16::from_be_bytes([data[4], data[5]]);
        let seq = u16::from_be_bytes([data[6], data[7]]);
        let payload_len = data.len() - ICMPV6_ECHO_HEADER_SIZE;

        println!(
            "[ICMPv6] Echo reply from {}: id={} seq={} len={}",
            super::ipv6::format_ipv6_compressed(src),
            id,
            seq,
            payload_len,
        );

        // Update echo reply tracking for the ping6 command
        LAST_ECHO_REPLY.store(seq as u64, core::sync::atomic::Ordering::Relaxed);
    }
}

/// Handle Destination Unreachable message.
fn handle_dest_unreachable(src: &Ipv6Address, code: u8, _data: &[u8]) {
    let reason = match code {
        ICMPV6_NO_ROUTE => "no route to destination",
        ICMPV6_ADMIN_PROHIBITED => "administratively prohibited",
        ICMPV6_BEYOND_SCOPE => "beyond scope",
        ICMPV6_ADDR_UNREACHABLE => "address unreachable",
        ICMPV6_PORT_UNREACHABLE => "port unreachable",
        _ => "unknown",
    };
    println!(
        "[ICMPv6] Destination unreachable from {}: {} (code {})",
        super::ipv6::format_ipv6_compressed(src),
        reason,
        code,
    );
}

/// Handle Packet Too Big message.
fn handle_packet_too_big(src: &Ipv6Address, data: &[u8]) {
    if data.len() >= 8 {
        let mtu = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);
        println!(
            "[ICMPv6] Packet too big from {}: MTU={}",
            super::ipv6::format_ipv6_compressed(src),
            mtu,
        );
    }
}

/// Handle Time Exceeded message.
fn handle_time_exceeded(src: &Ipv6Address, code: u8, _data: &[u8]) {
    let reason = match code {
        ICMPV6_HOP_LIMIT_EXCEEDED => "hop limit exceeded",
        ICMPV6_FRAGMENT_REASSEMBLY_EXCEEDED => "fragment reassembly time exceeded",
        _ => "unknown",
    };
    println!(
        "[ICMPv6] Time exceeded from {}: {} (code {})",
        super::ipv6::format_ipv6_compressed(src),
        reason,
        code,
    );
}

// ============================================================================
// ICMPv6 Message Construction
// ============================================================================

/// Build an ICMPv6 Echo Reply message.
///
/// Returns the raw ICMPv6 message bytes (not wrapped in IPv6 header).
pub fn build_echo_reply(
    src: &Ipv6Address,
    dst: &Ipv6Address,
    id: u16,
    seq: u16,
    data: &[u8],
) -> Vec<u8> {
    let mut msg = Vec::with_capacity(ICMPV6_ECHO_HEADER_SIZE + data.len());

    // ICMPv6 header
    msg.push(ICMPV6_ECHO_REPLY); // Type
    msg.push(0); // Code
    msg.extend_from_slice(&[0u8; 2]); // Checksum (filled later)

    // Echo header
    msg.extend_from_slice(&id.to_be_bytes());
    msg.extend_from_slice(&seq.to_be_bytes());

    // Echo data
    msg.extend_from_slice(data);

    // Compute and fill checksum
    let checksum = super::ipv6::compute_icmpv6_checksum(&src.0, &dst.0, &msg);
    msg[2] = (checksum >> 8) as u8;
    msg[3] = (checksum & 0xff) as u8;

    msg
}

/// Build an ICMPv6 Echo Request message.
///
/// Returns the raw ICMPv6 message bytes (not wrapped in IPv6 header).
pub fn build_echo_request(
    src: &Ipv6Address,
    dst: &Ipv6Address,
    id: u16,
    seq: u16,
    data: &[u8],
) -> Vec<u8> {
    let mut msg = Vec::with_capacity(ICMPV6_ECHO_HEADER_SIZE + data.len());

    // ICMPv6 header
    msg.push(ICMPV6_ECHO_REQUEST); // Type
    msg.push(0); // Code
    msg.extend_from_slice(&[0u8; 2]); // Checksum (filled later)

    // Echo header
    msg.extend_from_slice(&id.to_be_bytes());
    msg.extend_from_slice(&seq.to_be_bytes());

    // Echo data
    msg.extend_from_slice(data);

    // Compute and fill checksum
    let checksum = super::ipv6::compute_icmpv6_checksum(&src.0, &dst.0, &msg);
    msg[2] = (checksum >> 8) as u8;
    msg[3] = (checksum & 0xff) as u8;

    msg
}

/// Build a Destination Unreachable message.
///
/// `invoking_packet` should be the start of the invoking IPv6 packet
/// (as much as possible without exceeding the minimum MTU).
pub fn build_dest_unreachable(
    src: &Ipv6Address,
    dst: &Ipv6Address,
    code: u8,
    invoking_packet: &[u8],
) -> Vec<u8> {
    // Maximum payload: ensure total ICMPv6 message fits in minimum IPv6 MTU
    let max_payload = super::ipv6::IPV6_MIN_MTU - super::ipv6::IPV6_HEADER_SIZE - 8;
    let payload_len = invoking_packet.len().min(max_payload);

    let mut msg = Vec::with_capacity(8 + payload_len);

    msg.push(ICMPV6_DEST_UNREACHABLE); // Type
    msg.push(code); // Code
    msg.extend_from_slice(&[0u8; 2]); // Checksum (filled later)
    msg.extend_from_slice(&[0u8; 4]); // Unused (must be zero)
    msg.extend_from_slice(&invoking_packet[..payload_len]);

    // Compute and fill checksum
    let checksum = super::ipv6::compute_icmpv6_checksum(&src.0, &dst.0, &msg);
    msg[2] = (checksum >> 8) as u8;
    msg[3] = (checksum & 0xff) as u8;

    msg
}

/// Build a Packet Too Big message.
pub fn build_packet_too_big(
    src: &Ipv6Address,
    dst: &Ipv6Address,
    mtu: u32,
    invoking_packet: &[u8],
) -> Vec<u8> {
    let max_payload = super::ipv6::IPV6_MIN_MTU - super::ipv6::IPV6_HEADER_SIZE - 8;
    let payload_len = invoking_packet.len().min(max_payload);

    let mut msg = Vec::with_capacity(8 + payload_len);

    msg.push(ICMPV6_PACKET_TOO_BIG); // Type
    msg.push(0); // Code (always 0)
    msg.extend_from_slice(&[0u8; 2]); // Checksum (filled later)
    msg.extend_from_slice(&mtu.to_be_bytes()); // MTU
    msg.extend_from_slice(&invoking_packet[..payload_len]);

    // Compute and fill checksum
    let checksum = super::ipv6::compute_icmpv6_checksum(&src.0, &dst.0, &msg);
    msg[2] = (checksum >> 8) as u8;
    msg[3] = (checksum & 0xff) as u8;

    msg
}

/// Build a Time Exceeded message.
pub fn build_time_exceeded(
    src: &Ipv6Address,
    dst: &Ipv6Address,
    code: u8,
    invoking_packet: &[u8],
) -> Vec<u8> {
    let max_payload = super::ipv6::IPV6_MIN_MTU - super::ipv6::IPV6_HEADER_SIZE - 8;
    let payload_len = invoking_packet.len().min(max_payload);

    let mut msg = Vec::with_capacity(8 + payload_len);

    msg.push(ICMPV6_TIME_EXCEEDED); // Type
    msg.push(code); // Code
    msg.extend_from_slice(&[0u8; 2]); // Checksum (filled later)
    msg.extend_from_slice(&[0u8; 4]); // Unused
    msg.extend_from_slice(&invoking_packet[..payload_len]);

    // Compute and fill checksum
    let checksum = super::ipv6::compute_icmpv6_checksum(&src.0, &dst.0, &msg);
    msg[2] = (checksum >> 8) as u8;
    msg[3] = (checksum & 0xff) as u8;

    msg
}

// ============================================================================
// Checksum Verification
// ============================================================================

/// Verify the checksum of an incoming ICMPv6 message.
///
/// Returns true if the checksum is valid (or zero, which some implementations
/// skip).
fn verify_checksum(src: &Ipv6Address, dst: &Ipv6Address, data: &[u8]) -> bool {
    if data.len() < ICMPV6_HEADER_SIZE {
        return false;
    }

    // Compute checksum over the entire message (with existing checksum field
    // included)
    let computed = super::ipv6::compute_icmpv6_checksum(&src.0, &dst.0, data);

    // A valid checksum should compute to 0 (since the stored checksum participates)
    computed == 0
}

/// Compute ICMPv6 checksum (delegates to ipv6 module).
///
/// This is a convenience wrapper for building ICMPv6 messages.
pub fn compute_icmpv6_checksum(src: &[u8; 16], dst: &[u8; 16], data: &[u8]) -> u16 {
    super::ipv6::compute_icmpv6_checksum(src, dst, data)
}

// ============================================================================
// Echo Reply Tracking (for ping6 command)
// ============================================================================

/// Last received echo reply sequence number (for ping6 display)
static LAST_ECHO_REPLY: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

/// Get the last received echo reply sequence number.
pub fn get_last_echo_reply_seq() -> u64 {
    LAST_ECHO_REPLY.load(core::sync::atomic::Ordering::Relaxed)
}

/// Reset the echo reply tracker.
pub fn reset_echo_reply_tracker() {
    LAST_ECHO_REPLY.store(0, core::sync::atomic::Ordering::Relaxed);
}

// ============================================================================
// ICMPv6 Statistics
// ============================================================================

/// ICMPv6 statistics
#[derive(Debug, Clone, Copy, Default)]
pub struct Icmpv6Stats {
    /// Echo requests received
    pub echo_requests_received: u64,
    /// Echo replies sent
    pub echo_replies_sent: u64,
    /// Echo replies received
    pub echo_replies_received: u64,
    /// Error messages received
    pub errors_received: u64,
    /// NDP messages processed
    pub ndp_messages: u64,
}

// ============================================================================
// Initialization
// ============================================================================

/// Initialize the ICMPv6 subsystem.
pub fn init() -> Result<(), KernelError> {
    println!("[ICMPv6] Initializing ICMPv6...");
    println!("[ICMPv6] ICMPv6 initialized");
    Ok(())
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_icmpv6_header_parse() {
        let data = [128u8, 0, 0x12, 0x34]; // Echo Request, code 0, checksum 0x1234
        let header = Icmpv6Header::from_bytes(&data).unwrap();
        assert_eq!(header.icmp_type, ICMPV6_ECHO_REQUEST);
        assert_eq!(header.code, 0);
        assert_eq!(header.checksum, 0x1234);
    }

    #[test]
    fn test_icmpv6_header_roundtrip() {
        let header = Icmpv6Header {
            icmp_type: ICMPV6_ECHO_REPLY,
            code: 0,
            checksum: 0xABCD,
        };
        let bytes = header.to_bytes();
        let parsed = Icmpv6Header::from_bytes(&bytes).unwrap();
        assert_eq!(parsed.icmp_type, header.icmp_type);
        assert_eq!(parsed.code, header.code);
        assert_eq!(parsed.checksum, header.checksum);
    }

    #[test]
    fn test_icmpv6_header_too_short() {
        let data = [128u8, 0];
        assert!(Icmpv6Header::from_bytes(&data).is_err());
    }

    #[test]
    fn test_build_echo_request() {
        let src = Ipv6Address::LOCALHOST;
        let dst = Ipv6Address([0xfe, 0x80, 0, 0, 0, 0, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8]);
        let data = b"ping6";
        let msg = build_echo_request(&src, &dst, 1, 1, data);

        assert_eq!(msg[0], ICMPV6_ECHO_REQUEST);
        assert_eq!(msg[1], 0); // code
                               // id = 1
        assert_eq!(u16::from_be_bytes([msg[4], msg[5]]), 1);
        // seq = 1
        assert_eq!(u16::from_be_bytes([msg[6], msg[7]]), 1);
        // Payload
        assert_eq!(&msg[8..], data);
        // Checksum should be non-zero
        let checksum = u16::from_be_bytes([msg[2], msg[3]]);
        assert_ne!(checksum, 0);
    }

    #[test]
    fn test_build_echo_reply() {
        let src = Ipv6Address([0xfe, 0x80, 0, 0, 0, 0, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8]);
        let dst = Ipv6Address::LOCALHOST;
        let data = b"pong6";
        let msg = build_echo_reply(&src, &dst, 42, 7, data);

        assert_eq!(msg[0], ICMPV6_ECHO_REPLY);
        assert_eq!(u16::from_be_bytes([msg[4], msg[5]]), 42);
        assert_eq!(u16::from_be_bytes([msg[6], msg[7]]), 7);
        assert_eq!(&msg[8..], data);
    }

    #[test]
    fn test_build_dest_unreachable() {
        let src = Ipv6Address::LOCALHOST;
        let dst = Ipv6Address([0xfe, 0x80, 0, 0, 0, 0, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8]);
        let invoking = [0u8; 64];
        let msg = build_dest_unreachable(&src, &dst, ICMPV6_PORT_UNREACHABLE, &invoking);

        assert_eq!(msg[0], ICMPV6_DEST_UNREACHABLE);
        assert_eq!(msg[1], ICMPV6_PORT_UNREACHABLE);
        assert!(msg.len() >= 8 + 64);
    }

    #[test]
    fn test_build_packet_too_big() {
        let src = Ipv6Address::LOCALHOST;
        let dst = Ipv6Address::LOCALHOST;
        let invoking = [0u8; 32];
        let msg = build_packet_too_big(&src, &dst, 1280, &invoking);

        assert_eq!(msg[0], ICMPV6_PACKET_TOO_BIG);
        assert_eq!(msg[1], 0);
        let mtu = u32::from_be_bytes([msg[4], msg[5], msg[6], msg[7]]);
        assert_eq!(mtu, 1280);
    }

    #[test]
    fn test_echo_reply_tracker() {
        reset_echo_reply_tracker();
        assert_eq!(get_last_echo_reply_seq(), 0);

        LAST_ECHO_REPLY.store(42, core::sync::atomic::Ordering::Relaxed);
        assert_eq!(get_last_echo_reply_seq(), 42);

        reset_echo_reply_tracker();
        assert_eq!(get_last_echo_reply_seq(), 0);
    }
}
