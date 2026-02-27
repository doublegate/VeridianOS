//! DHCP Client for Automatic Network Configuration
//!
//! Implements DHCPv4 protocol for obtaining IP addresses and network
//! configuration.

// DHCP client

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
#[allow(dead_code)] // DHCP protocol constant per RFC 2131
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
#[allow(dead_code)] // DHCP option per RFC 2132
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
            // All try_into() calls below convert fixed-size slices to arrays.
            // The length check above (bytes.len() >= 236) guarantees all slices
            // are the correct size, so these conversions cannot fail.
            xid: u32::from_be_bytes(bytes[4..8].try_into().expect("DHCP xid slice")),
            secs: u16::from_be_bytes(bytes[8..10].try_into().expect("DHCP secs slice")),
            flags: u16::from_be_bytes(bytes[10..12].try_into().expect("DHCP flags slice")),
            ciaddr: Ipv4Address(bytes[12..16].try_into().expect("DHCP ciaddr slice")),
            yiaddr: Ipv4Address(bytes[16..20].try_into().expect("DHCP yiaddr slice")),
            siaddr: Ipv4Address(bytes[20..24].try_into().expect("DHCP siaddr slice")),
            giaddr: Ipv4Address(bytes[24..28].try_into().expect("DHCP giaddr slice")),
            chaddr: bytes[28..44].try_into().expect("DHCP chaddr slice"),
            sname: bytes[44..108].try_into().expect("DHCP sname slice"),
            file: bytes[108..236].try_into().expect("DHCP file slice"),
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
    #[allow(dead_code)] // Read during DHCP lease renewal (Phase 6)
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

    /// Process DHCP OFFER -- parse options and transition to Requesting.
    pub fn process_offer(&mut self, packet: &DhcpPacket) -> Result<(), KernelError> {
        if self.state != DhcpState::Selecting {
            return Err(KernelError::InvalidState {
                expected: "Selecting",
                actual: "Other",
            });
        }

        let options = parse_dhcp_options(&packet.options);
        let offered_ip = packet.yiaddr;
        let server_id = options.server_id.unwrap_or(packet.siaddr);

        println!(
            "[DHCP] Received OFFER: {}.{}.{}.{} from server {}.{}.{}.{}",
            offered_ip.0[0],
            offered_ip.0[1],
            offered_ip.0[2],
            offered_ip.0[3],
            server_id.0[0],
            server_id.0[1],
            server_id.0[2],
            server_id.0[3],
        );

        // Send REQUEST for the offered IP
        let request = self.create_request(offered_ip, server_id);
        let request_bytes = request.to_bytes();
        send_dhcp_packet(&request_bytes);

        self.state = DhcpState::Requesting;
        Ok(())
    }

    /// Process DHCP ACK -- configure network interface with obtained
    /// parameters.
    pub fn process_ack(&mut self, packet: &DhcpPacket) -> Result<(), KernelError> {
        if self.state != DhcpState::Requesting {
            return Err(KernelError::InvalidState {
                expected: "Requesting",
                actual: "Other",
            });
        }

        let options = parse_dhcp_options(&packet.options);

        let ip = packet.yiaddr;
        let subnet = options
            .subnet_mask
            .unwrap_or(Ipv4Address::new(255, 255, 255, 0));
        let gateway = options.router;
        let lease = options.lease_time.unwrap_or(3600);

        let config = DhcpConfig {
            ip_address: ip,
            subnet_mask: subnet,
            router: gateway,
            dns_servers: options.dns_servers,
            lease_time: lease,
            server_id: options.server_id.unwrap_or(packet.siaddr),
        };

        println!(
            "[DHCP] ACK: IP {}.{}.{}.{} mask {}.{}.{}.{} lease {}s",
            ip.0[0],
            ip.0[1],
            ip.0[2],
            ip.0[3],
            subnet.0[0],
            subnet.0[1],
            subnet.0[2],
            subnet.0[3],
            lease,
        );
        if let Some(gw) = gateway {
            println!(
                "[DHCP] Gateway: {}.{}.{}.{}",
                gw.0[0], gw.0[1], gw.0[2], gw.0[3]
            );
        }

        // Configure the IP layer with the obtained address
        super::ip::set_interface_config(ip, subnet, gateway);

        // Add default route via gateway
        if let Some(gw) = gateway {
            super::ip::add_route(super::ip::RouteEntry {
                destination: Ipv4Address::new(0, 0, 0, 0),
                netmask: Ipv4Address::new(0, 0, 0, 0),
                gateway: Some(gw),
                interface: 0,
            });
        }

        self.config = Some(config);
        self.state = DhcpState::Bound;

        Ok(())
    }

    /// Process an incoming DHCP response packet.
    ///
    /// Dispatches to `process_offer` or `process_ack` based on the
    /// message type option.
    pub fn process_response(&mut self, data: &[u8]) -> Result<(), KernelError> {
        let packet = DhcpPacket::from_bytes(data)?;

        // Verify transaction ID matches
        if packet.xid != self.xid {
            return Ok(()); // Not for us
        }

        match packet.get_message_type() {
            Some(DhcpMessageType::Offer) => self.process_offer(&packet),
            Some(DhcpMessageType::Ack) => self.process_ack(&packet),
            Some(DhcpMessageType::Nak) => {
                println!("[DHCP] Received NAK, restarting negotiation");
                self.state = DhcpState::Init;
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// Get current DHCP state
    pub fn state(&self) -> DhcpState {
        self.state
    }

    /// Get current configuration (if bound)
    pub fn config(&self) -> Option<&DhcpConfig> {
        self.config.as_ref()
    }

    /// Start DHCP negotiation -- sends DISCOVER via UDP broadcast.
    pub fn start(&mut self) -> Result<(), KernelError> {
        println!("[DHCP] Starting DHCP negotiation");

        let discover = self.create_discover();
        let discover_bytes = discover.to_bytes();

        println!("[DHCP] Sending DISCOVER ({} bytes)", discover_bytes.len());
        send_dhcp_packet(&discover_bytes);

        self.state = DhcpState::Selecting;
        Ok(())
    }
}

/// Parsed DHCP options
#[derive(Debug, Default)]
struct ParsedDhcpOptions {
    subnet_mask: Option<Ipv4Address>,
    router: Option<Ipv4Address>,
    dns_servers: Vec<Ipv4Address>,
    lease_time: Option<u32>,
    server_id: Option<Ipv4Address>,
}

/// Parse DHCP options from the options byte array (after magic cookie).
fn parse_dhcp_options(options: &[u8]) -> ParsedDhcpOptions {
    let mut result = ParsedDhcpOptions::default();
    let mut i = 4; // Skip magic cookie (first 4 bytes)

    while i < options.len() {
        let code = options[i];
        if code == OPT_END {
            break;
        }
        if code == 0 {
            // Padding
            i += 1;
            continue;
        }
        if i + 1 >= options.len() {
            break;
        }
        let len = options[i + 1] as usize;
        if i + 2 + len > options.len() {
            break;
        }
        let data = &options[i + 2..i + 2 + len];

        match code {
            OPT_SUBNET_MASK if len == 4 => {
                result.subnet_mask = Some(Ipv4Address([data[0], data[1], data[2], data[3]]));
            }
            OPT_ROUTER if len >= 4 => {
                result.router = Some(Ipv4Address([data[0], data[1], data[2], data[3]]));
            }
            OPT_DNS_SERVER if len >= 4 => {
                for chunk in data.chunks_exact(4) {
                    result
                        .dns_servers
                        .push(Ipv4Address([chunk[0], chunk[1], chunk[2], chunk[3]]));
                }
            }
            OPT_LEASE_TIME if len == 4 => {
                result.lease_time = Some(u32::from_be_bytes([data[0], data[1], data[2], data[3]]));
            }
            OPT_SERVER_ID if len == 4 => {
                result.server_id = Some(Ipv4Address([data[0], data[1], data[2], data[3]]));
            }
            _ => {} // Unknown option, skip
        }

        i += 2 + len;
    }

    result
}

/// Send a DHCP packet via UDP broadcast (0.0.0.0:68 -> 255.255.255.255:67).
fn send_dhcp_packet(data: &[u8]) {
    let src = super::SocketAddr::v4(Ipv4Address::ANY, 68);
    let dst = super::SocketAddr::v4(Ipv4Address::BROADCAST, 67);
    let _ = super::udp::send_packet(src, dst, data);
}

/// Global DHCP client instance
static DHCP_CLIENT: spin::Mutex<Option<DhcpClient>> = spin::Mutex::new(None);

/// Start DHCP on the primary interface.
pub fn start_dhcp() -> Result<(), KernelError> {
    let mac =
        super::device::with_device("eth0", |dev| dev.mac_address()).unwrap_or(MacAddress::ZERO);

    let mut lock = DHCP_CLIENT.lock();
    let client = lock.get_or_insert_with(|| DhcpClient::new(mac));
    client.start()
}

/// Get current DHCP state for display.
pub fn get_dhcp_state() -> Option<DhcpState> {
    let lock = DHCP_CLIENT.lock();
    lock.as_ref().map(|c| c.state())
}

/// Initialize DHCP client
pub fn init() -> Result<(), KernelError> {
    println!("[DHCP] DHCP client initialized");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dhcp_packet_creation() {
        let mac = MacAddress([0x52, 0x54, 0x00, 0x12, 0x34, 0x56]);
        let packet = DhcpPacket::new(DhcpMessageType::Discover, mac, 0x12345678);

        assert_eq!(packet.op, DHCP_OP_BOOTREQUEST);
        assert_eq!(packet.htype, DHCP_HTYPE_ETHERNET);
        assert_eq!(packet.hlen, 6);
    }

    #[test]
    fn test_dhcp_serialization() {
        let mac = MacAddress([0x52, 0x54, 0x00, 0x12, 0x34, 0x56]);
        let mut packet = DhcpPacket::new(DhcpMessageType::Discover, mac, 0x12345678);
        packet.finalize();

        let bytes = packet.to_bytes();
        assert!(bytes.len() >= 236);
    }
}
