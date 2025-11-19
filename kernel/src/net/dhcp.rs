//! DHCP Client for Automatic Network Configuration
//!
//! Implements DHCPv4 protocol for obtaining IP addresses and network
//! configuration.

use alloc::vec::Vec;
use core::convert::TryInto;

use crate::{
    error::KernelError,
    net::{Ipv4Address, MacAddress},
};

/// DHCP message types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum DhcpMessageType {
    Discover = 1,
    Offer = 2,
    Request = 3,
    Decline = 4,
    Ack = 5,
    Nak = 6,
    Release = 7,
    Inform = 8,
}

/// DHCP operation codes
const DHCP_OP_BOOTREQUEST: u8 = 1;
const DHCP_OP_BOOTREPLY: u8 = 2;

/// DHCP hardware types
const DHCP_HTYPE_ETHERNET: u8 = 1;

/// DHCP magic cookie
const DHCP_MAGIC_COOKIE: u32 = 0x63825363;

/// DHCP option codes
const OPT_SUBNET_MASK: u8 = 1;
const OPT_ROUTER: u8 = 3;
const OPT_DNS_SERVER: u8 = 6;
const OPT_REQUESTED_IP: u8 = 50;
const OPT_LEASE_TIME: u8 = 51;
const OPT_MESSAGE_TYPE: u8 = 53;
const OPT_SERVER_ID: u8 = 54;
const OPT_PARAMETER_LIST: u8 = 55;
const OPT_END: u8 = 255;

/// DHCP packet structure
#[repr(C)]
#[derive(Debug, Clone)]
pub struct DhcpPacket {
    /// Operation code (1 = request, 2 = reply)
    pub op: u8,

    /// Hardware type (1 = Ethernet)
    pub htype: u8,

    /// Hardware address length
    pub hlen: u8,

    /// Hops
    pub hops: u8,

    /// Transaction ID
    pub xid: u32,

    /// Seconds elapsed
    pub secs: u16,

    /// Flags
    pub flags: u16,

    /// Client IP address
    pub ciaddr: Ipv4Address,

    /// Your (client) IP address
    pub yiaddr: Ipv4Address,

    /// Server IP address
    pub siaddr: Ipv4Address,

    /// Gateway IP address
    pub giaddr: Ipv4Address,

    /// Client hardware address (16 bytes)
    pub chaddr: [u8; 16],

    /// Server host name (64 bytes)
    pub sname: [u8; 64],

    /// Boot file name (128 bytes)
    pub file: [u8; 128],

    /// Options (variable length)
    pub options: Vec<u8>,
}

impl DhcpPacket {
    /// Create a new DHCP packet
    pub fn new(message_type: DhcpMessageType, mac_address: MacAddress, xid: u32) -> Self {
        let mut packet = Self {
            op: DHCP_OP_BOOTREQUEST,
            htype: DHCP_HTYPE_ETHERNET,
            hlen: 6,
            hops: 0,
            xid,
            secs: 0,
            flags: 0x8000, // Broadcast flag
            ciaddr: Ipv4Address::UNSPECIFIED,
            yiaddr: Ipv4Address::UNSPECIFIED,
            siaddr: Ipv4Address::UNSPECIFIED,
            giaddr: Ipv4Address::UNSPECIFIED,
            chaddr: [0; 16],
            sname: [0; 64],
            file: [0; 128],
            options: Vec::new(),
        };

        // Set client hardware address
        packet.chaddr[0..6].copy_from_slice(&mac_address.0);

        // Add magic cookie
        packet.add_option_u32(DHCP_MAGIC_COOKIE);

        // Add message type option
        packet.add_option_u8(OPT_MESSAGE_TYPE, message_type as u8);

        packet
    }

    /// Add a u8 option
    fn add_option_u8(&mut self, code: u8, value: u8) {
        self.options.push(code);
        self.options.push(1); // Length
        self.options.push(value);
    }

    /// Add a u32 option
    fn add_option_u32(&mut self, value: u32) {
        self.options.extend_from_slice(&value.to_be_bytes());
    }

    /// Add an IPv4 address option
    fn add_option_ipv4(&mut self, code: u8, addr: Ipv4Address) {
        self.options.push(code);
        self.options.push(4); // Length
        self.options.extend_from_slice(&addr.0);
    }

    /// Add parameter request list
    pub fn add_parameter_request_list(&mut self) {
        self.options.push(OPT_PARAMETER_LIST);
        self.options.push(3); // Length
        self.options.push(OPT_SUBNET_MASK);
        self.options.push(OPT_ROUTER);
        self.options.push(OPT_DNS_SERVER);
    }

    /// Finalize options
    pub fn finalize(&mut self) {
        self.options.push(OPT_END);
    }

    /// Serialize to bytes
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(236 + self.options.len());

        bytes.push(self.op);
        bytes.push(self.htype);
        bytes.push(self.hlen);
        bytes.push(self.hops);
        bytes.extend_from_slice(&self.xid.to_be_bytes());
        bytes.extend_from_slice(&self.secs.to_be_bytes());
        bytes.extend_from_slice(&self.flags.to_be_bytes());
        bytes.extend_from_slice(&self.ciaddr.0);
        bytes.extend_from_slice(&self.yiaddr.0);
        bytes.extend_from_slice(&self.siaddr.0);
        bytes.extend_from_slice(&self.giaddr.0);
        bytes.extend_from_slice(&self.chaddr);
        bytes.extend_from_slice(&self.sname);
        bytes.extend_from_slice(&self.file);
        bytes.extend_from_slice(&self.options);

        bytes
    }

    /// Parse from bytes
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, KernelError> {
        if bytes.len() < 236 {
            return Err(KernelError::InvalidArgument {
                name: "dhcp_packet_length",
                value: "too_short",
            });
        }

        let mut packet = Self {
            op: bytes[0],
            htype: bytes[1],
            hlen: bytes[2],
            hops: bytes[3],
            xid: u32::from_be_bytes(bytes[4..8].try_into().unwrap()),
            secs: u16::from_be_bytes(bytes[8..10].try_into().unwrap()),
            flags: u16::from_be_bytes(bytes[10..12].try_into().unwrap()),
            ciaddr: Ipv4Address(bytes[12..16].try_into().unwrap()),
            yiaddr: Ipv4Address(bytes[16..20].try_into().unwrap()),
            siaddr: Ipv4Address(bytes[20..24].try_into().unwrap()),
            giaddr: Ipv4Address(bytes[24..28].try_into().unwrap()),
            chaddr: bytes[28..44].try_into().unwrap(),
            sname: bytes[44..108].try_into().unwrap(),
            file: bytes[108..236].try_into().unwrap(),
            options: Vec::new(),
        };

        // Parse options
        if bytes.len() > 236 {
            packet.options = bytes[236..].to_vec();
        }

        Ok(packet)
    }

    /// Get message type from options
    pub fn get_message_type(&self) -> Option<DhcpMessageType> {
        let mut i = 4; // Skip magic cookie

        while i < self.options.len() {
            let code = self.options[i];
            if code == OPT_END {
                break;
            }

            if i + 1 >= self.options.len() {
                break;
            }

            let len = self.options[i + 1] as usize;
            if code == OPT_MESSAGE_TYPE && len == 1 && i + 2 < self.options.len() {
                let msg_type = self.options[i + 2];
                return match msg_type {
                    1 => Some(DhcpMessageType::Discover),
                    2 => Some(DhcpMessageType::Offer),
                    3 => Some(DhcpMessageType::Request),
                    4 => Some(DhcpMessageType::Decline),
                    5 => Some(DhcpMessageType::Ack),
                    6 => Some(DhcpMessageType::Nak),
                    7 => Some(DhcpMessageType::Release),
                    8 => Some(DhcpMessageType::Inform),
                    _ => None,
                };
            }

            i += 2 + len;
        }

        None
    }
}

/// DHCP client state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DhcpState {
    Init,
    Selecting,
    Requesting,
    Bound,
    Renewing,
    Rebinding,
}

/// DHCP client configuration
#[derive(Debug, Clone)]
pub struct DhcpConfig {
    pub ip_address: Ipv4Address,
    pub subnet_mask: Ipv4Address,
    pub router: Option<Ipv4Address>,
    pub dns_servers: Vec<Ipv4Address>,
    pub lease_time: u32,
    pub server_id: Ipv4Address,
}

/// DHCP client
pub struct DhcpClient {
    /// MAC address
    mac_address: MacAddress,

    /// Current state
    state: DhcpState,

    /// Transaction ID
    xid: u32,

    /// Current configuration
    config: Option<DhcpConfig>,
}

impl DhcpClient {
    /// Create a new DHCP client
    pub fn new(mac_address: MacAddress) -> Self {
        Self {
            mac_address,
            state: DhcpState::Init,
            xid: 0x12345678, // Would use random
            config: None,
        }
    }

    /// Create DHCP DISCOVER packet
    pub fn create_discover(&self) -> DhcpPacket {
        let mut packet = DhcpPacket::new(DhcpMessageType::Discover, self.mac_address, self.xid);
        packet.add_parameter_request_list();
        packet.finalize();
        packet
    }

    /// Create DHCP REQUEST packet
    pub fn create_request(&self, offered_ip: Ipv4Address, server_id: Ipv4Address) -> DhcpPacket {
        let mut packet = DhcpPacket::new(DhcpMessageType::Request, self.mac_address, self.xid);
        packet.add_option_ipv4(OPT_REQUESTED_IP, offered_ip);
        packet.add_option_ipv4(OPT_SERVER_ID, server_id);
        packet.add_parameter_request_list();
        packet.finalize();
        packet
    }

    /// Process DHCP OFFER
    pub fn process_offer(&mut self, _packet: &DhcpPacket) -> Result<(), KernelError> {
        // TODO: Parse offer, extract IP and server ID
        self.state = DhcpState::Requesting;
        Ok(())
    }

    /// Process DHCP ACK
    pub fn process_ack(&mut self, packet: &DhcpPacket) -> Result<(), KernelError> {
        // TODO: Parse ACK, configure interface
        self.state = DhcpState::Bound;

        println!("[DHCP] Received ACK - IP address configured");
        println!(
            "[DHCP] IP: {}.{}.{}.{}",
            packet.yiaddr.0[0], packet.yiaddr.0[1], packet.yiaddr.0[2], packet.yiaddr.0[3]
        );

        Ok(())
    }

    /// Start DHCP negotiation
    pub fn start(&mut self) -> Result<(), KernelError> {
        println!("[DHCP] Starting DHCP negotiation");

        // Create DISCOVER packet
        let discover = self.create_discover();
        let discover_bytes = discover.to_bytes();

        // TODO: Send via UDP to 255.255.255.255:67
        println!("[DHCP] Sending DISCOVER ({} bytes)", discover_bytes.len());

        self.state = DhcpState::Selecting;

        Ok(())
    }
}

/// Initialize DHCP client
pub fn init() -> Result<(), KernelError> {
    println!("[DHCP] DHCP client initialized");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test_case]
    fn test_dhcp_packet_creation() {
        let mac = MacAddress([0x52, 0x54, 0x00, 0x12, 0x34, 0x56]);
        let packet = DhcpPacket::new(DhcpMessageType::Discover, mac, 0x12345678);

        assert_eq!(packet.op, DHCP_OP_BOOTREQUEST);
        assert_eq!(packet.htype, DHCP_HTYPE_ETHERNET);
        assert_eq!(packet.hlen, 6);
    }

    #[test_case]
    fn test_dhcp_serialization() {
        let mac = MacAddress([0x52, 0x54, 0x00, 0x12, 0x34, 0x56]);
        let mut packet = DhcpPacket::new(DhcpMessageType::Discover, mac, 0x12345678);
        packet.finalize();

        let bytes = packet.to_bytes();
        assert!(bytes.len() >= 236);
    }
}
