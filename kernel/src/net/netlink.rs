//! Netlink-style interface for userland-kernel network configuration IPC
//!
//! Provides a message-passing mechanism between userland network daemons
//! (such as the NetworkManager shim) and the kernel network stack.
//! Messages use an integer-based serialization format suitable for
//! no-std environments.
//!
//! # Message Format
//!
//! Each netlink message is serialized as a fixed-size header followed by
//! a variable-length payload:
//!
//! ```text
//! +--------+--------+--------+--------+--------+--------+--...--+
//! | type   | flags  | seq    | pid    | length | payload        |
//! | u16    | u16    | u32    | u32    | u32    | [u8; length]   |
//! +--------+--------+--------+--------+--------+--...--+
//! ```
//!
//! # Supported Operations
//!
//! - Link management: bring interfaces up/down, query link state
//! - Address management: add/remove IPv4/IPv6 addresses
//! - Route management: add/remove routes, set default gateway
//! - Device enumeration: list interfaces and their properties

#![allow(dead_code)]

use alloc::{string::String, vec, vec::Vec};

use spin::Mutex;

use super::{Ipv4Address, MacAddress};
use crate::error::KernelError;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum netlink message payload size
const MAX_PAYLOAD_SIZE: usize = 4096;

/// Maximum number of pending messages in a socket queue
const MAX_QUEUE_DEPTH: usize = 64;

/// Maximum number of open netlink sockets
const MAX_SOCKETS: usize = 32;

/// Maximum interface name length
const MAX_IFNAME_LEN: usize = 16;

// ---------------------------------------------------------------------------
// Netlink message types
// ---------------------------------------------------------------------------

/// Netlink message type codes
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum NetlinkMessageType {
    /// No-op / padding
    Noop = 0,

    /// Error response
    Error = 1,

    /// Acknowledgement
    Done = 2,

    /// Bring a network interface up
    LinkUp = 16,

    /// Bring a network interface down
    LinkDown = 17,

    /// Query link state for an interface
    GetLink = 18,

    /// Response: link information
    NewLink = 19,

    /// Delete a link (not commonly used)
    DelLink = 20,

    /// Add an IPv4/IPv6 address to an interface
    AddrAdd = 32,

    /// Remove an address from an interface
    AddrDel = 33,

    /// Query addresses on an interface
    GetAddr = 34,

    /// Response: address information
    NewAddr = 35,

    /// Add a route
    RouteAdd = 48,

    /// Delete a route
    RouteDel = 49,

    /// Query routes
    GetRoute = 50,

    /// Response: route information
    NewRoute = 51,

    /// Enumerate all network interfaces
    GetLinks = 64,

    /// Enumerate all addresses
    GetAddrs = 65,

    /// Enumerate all routes
    GetRoutes = 66,
}

impl NetlinkMessageType {
    /// Convert from raw u16 value
    pub fn from_u16(val: u16) -> Option<Self> {
        match val {
            0 => Some(Self::Noop),
            1 => Some(Self::Error),
            2 => Some(Self::Done),
            16 => Some(Self::LinkUp),
            17 => Some(Self::LinkDown),
            18 => Some(Self::GetLink),
            19 => Some(Self::NewLink),
            20 => Some(Self::DelLink),
            32 => Some(Self::AddrAdd),
            33 => Some(Self::AddrDel),
            34 => Some(Self::GetAddr),
            35 => Some(Self::NewAddr),
            48 => Some(Self::RouteAdd),
            49 => Some(Self::RouteDel),
            50 => Some(Self::GetRoute),
            51 => Some(Self::NewRoute),
            64 => Some(Self::GetLinks),
            65 => Some(Self::GetAddrs),
            66 => Some(Self::GetRoutes),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Netlink message flags
// ---------------------------------------------------------------------------

/// Message flags (bitmask)
pub mod flags {
    /// Request flag -- message is a request
    pub const NLM_F_REQUEST: u16 = 0x0001;
    /// Multi-part message
    pub const NLM_F_MULTI: u16 = 0x0002;
    /// Acknowledge flag
    pub const NLM_F_ACK: u16 = 0x0004;
    /// Dump all entries
    pub const NLM_F_DUMP: u16 = 0x0100;
    /// Create if not existing
    pub const NLM_F_CREATE: u16 = 0x0200;
    /// Replace existing entry
    pub const NLM_F_REPLACE: u16 = 0x0400;
}

// ---------------------------------------------------------------------------
// Netlink message header
// ---------------------------------------------------------------------------

/// Fixed-size netlink message header (16 bytes)
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct NetlinkHeader {
    /// Message type
    pub msg_type: u16,
    /// Flags
    pub flags: u16,
    /// Sequence number (for request/response matching)
    pub seq: u32,
    /// Sender process ID (0 = kernel)
    pub pid: u32,
    /// Payload length in bytes
    pub payload_len: u32,
}

impl NetlinkHeader {
    /// Header size in bytes
    pub const SIZE: usize = 16;

    /// Create a new header
    pub const fn new(msg_type: u16, flags: u16, seq: u32, pid: u32) -> Self {
        Self {
            msg_type,
            flags,
            seq,
            pid,
            payload_len: 0,
        }
    }

    /// Serialize header to bytes (little-endian)
    pub fn serialize(&self, buf: &mut [u8]) -> Result<usize, KernelError> {
        if buf.len() < Self::SIZE {
            return Err(KernelError::InvalidArgument {
                name: "netlink",
                value: "invalid",
            });
        }

        buf[0..2].copy_from_slice(&self.msg_type.to_le_bytes());
        buf[2..4].copy_from_slice(&self.flags.to_le_bytes());
        buf[4..8].copy_from_slice(&self.seq.to_le_bytes());
        buf[8..12].copy_from_slice(&self.pid.to_le_bytes());
        buf[12..16].copy_from_slice(&self.payload_len.to_le_bytes());

        Ok(Self::SIZE)
    }

    /// Deserialize header from bytes (little-endian)
    pub fn deserialize(buf: &[u8]) -> Result<Self, KernelError> {
        if buf.len() < Self::SIZE {
            return Err(KernelError::InvalidArgument {
                name: "netlink",
                value: "invalid",
            });
        }

        Ok(Self {
            msg_type: u16::from_le_bytes([buf[0], buf[1]]),
            flags: u16::from_le_bytes([buf[2], buf[3]]),
            seq: u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]),
            pid: u32::from_le_bytes([buf[8], buf[9], buf[10], buf[11]]),
            payload_len: u32::from_le_bytes([buf[12], buf[13], buf[14], buf[15]]),
        })
    }
}

// ---------------------------------------------------------------------------
// Netlink message
// ---------------------------------------------------------------------------

/// Complete netlink message (header + payload)
#[derive(Debug, Clone)]
pub struct NetlinkMessage {
    /// Message header
    pub header: NetlinkHeader,
    /// Variable-length payload
    pub payload: Vec<u8>,
}

impl NetlinkMessage {
    /// Create a new message with empty payload
    pub fn new(msg_type: NetlinkMessageType, flags: u16, seq: u32, pid: u32) -> Self {
        Self {
            header: NetlinkHeader::new(msg_type as u16, flags, seq, pid),
            payload: Vec::new(),
        }
    }

    /// Create an error response
    pub fn error(seq: u32, pid: u32, errno: i32) -> Self {
        let mut msg = Self::new(NetlinkMessageType::Error, 0, seq, pid);
        msg.payload = errno.to_le_bytes().to_vec();
        msg.header.payload_len = 4;
        msg
    }

    /// Create a Done (end-of-dump) message
    pub fn done(seq: u32, pid: u32) -> Self {
        Self::new(NetlinkMessageType::Done, 0, seq, pid)
    }

    /// Set payload from bytes
    pub fn set_payload(&mut self, data: &[u8]) -> Result<(), KernelError> {
        if data.len() > MAX_PAYLOAD_SIZE {
            return Err(KernelError::InvalidArgument {
                name: "netlink",
                value: "invalid",
            });
        }
        self.payload = data.to_vec();
        self.header.payload_len = data.len() as u32;
        Ok(())
    }

    /// Total serialized size
    pub fn total_size(&self) -> usize {
        NetlinkHeader::SIZE
            .checked_add(self.payload.len())
            .unwrap_or(NetlinkHeader::SIZE)
    }

    /// Serialize the complete message to bytes
    pub fn serialize(&self) -> Result<Vec<u8>, KernelError> {
        let total = self.total_size();
        let mut buf = vec![0u8; total];

        self.header.serialize(&mut buf[..NetlinkHeader::SIZE])?;

        if !self.payload.is_empty() {
            buf[NetlinkHeader::SIZE..].copy_from_slice(&self.payload);
        }

        Ok(buf)
    }

    /// Deserialize a complete message from bytes
    pub fn deserialize(buf: &[u8]) -> Result<Self, KernelError> {
        let header = NetlinkHeader::deserialize(buf)?;
        let payload_len = header.payload_len as usize;

        if payload_len > MAX_PAYLOAD_SIZE {
            return Err(KernelError::InvalidArgument {
                name: "netlink",
                value: "invalid",
            });
        }

        let total =
            NetlinkHeader::SIZE
                .checked_add(payload_len)
                .ok_or(KernelError::InvalidArgument {
                    name: "netlink",
                    value: "invalid",
                })?;

        if buf.len() < total {
            return Err(KernelError::InvalidArgument {
                name: "netlink",
                value: "invalid",
            });
        }

        let payload = if payload_len > 0 {
            buf[NetlinkHeader::SIZE..total].to_vec()
        } else {
            Vec::new()
        };

        Ok(Self { header, payload })
    }
}

// ---------------------------------------------------------------------------
// Link info payload
// ---------------------------------------------------------------------------

/// Network interface information (serialized in NewLink responses)
#[derive(Debug, Clone)]
pub struct LinkInfo {
    /// Interface index
    pub index: u32,
    /// Interface name
    pub name: String,
    /// MAC address
    pub mac: MacAddress,
    /// MTU
    pub mtu: u32,
    /// Flags (IFF_UP, IFF_RUNNING, etc.)
    pub flags: u32,
    /// Interface type (1=Ethernet, 801=Wi-Fi)
    pub if_type: u16,
    /// Link speed in Mbps (0 if unknown)
    pub speed: u32,
}

impl LinkInfo {
    /// Serialize to payload bytes
    pub fn serialize(&self) -> Vec<u8> {
        let name_bytes = self.name.as_bytes();
        let name_len = name_bytes.len().min(MAX_IFNAME_LEN);

        // Fixed layout: index(4) + name_len(2) + name(16) + mac(6) + mtu(4)
        //               + flags(4) + if_type(2) + speed(4) = 42 bytes
        let mut buf = vec![0u8; 42];

        buf[0..4].copy_from_slice(&self.index.to_le_bytes());
        buf[4..6].copy_from_slice(&(name_len as u16).to_le_bytes());
        buf[6..6 + name_len].copy_from_slice(&name_bytes[..name_len]);
        // bytes 6..22 = name (padded)
        buf[22..28].copy_from_slice(&self.mac.0);
        buf[28..32].copy_from_slice(&self.mtu.to_le_bytes());
        buf[32..36].copy_from_slice(&self.flags.to_le_bytes());
        buf[36..38].copy_from_slice(&self.if_type.to_le_bytes());
        buf[38..42].copy_from_slice(&self.speed.to_le_bytes());

        buf
    }

    /// Deserialize from payload bytes
    pub fn deserialize(buf: &[u8]) -> Result<Self, KernelError> {
        if buf.len() < 42 {
            return Err(KernelError::InvalidArgument {
                name: "netlink",
                value: "invalid",
            });
        }

        let index = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
        let name_len = u16::from_le_bytes([buf[4], buf[5]]) as usize;
        let name_len = name_len.min(MAX_IFNAME_LEN);

        let name = core::str::from_utf8(&buf[6..6 + name_len])
            .map_err(|_| KernelError::InvalidArgument {
                name: "netlink",
                value: "invalid",
            })?
            .into();

        let mut mac_bytes = [0u8; 6];
        mac_bytes.copy_from_slice(&buf[22..28]);

        let mtu = u32::from_le_bytes([buf[28], buf[29], buf[30], buf[31]]);
        let flags = u32::from_le_bytes([buf[32], buf[33], buf[34], buf[35]]);
        let if_type = u16::from_le_bytes([buf[36], buf[37]]);
        let speed = u32::from_le_bytes([buf[38], buf[39], buf[40], buf[41]]);

        Ok(Self {
            index,
            name,
            mac: MacAddress(mac_bytes),
            mtu,
            flags,
            if_type,
            speed,
        })
    }
}

// ---------------------------------------------------------------------------
// Address info payload
// ---------------------------------------------------------------------------

/// IP address entry (serialized in NewAddr responses)
#[derive(Debug, Clone)]
pub struct AddrInfo {
    /// Interface index
    pub index: u32,
    /// Address family (2=AF_INET, 10=AF_INET6)
    pub family: u8,
    /// Prefix length (e.g. 24 for /24)
    pub prefix_len: u8,
    /// IPv4 address (if family == 2)
    pub addr_v4: Ipv4Address,
}

impl AddrInfo {
    /// Serialize to payload bytes
    pub fn serialize(&self) -> Vec<u8> {
        // index(4) + family(1) + prefix(1) + addr(4) = 10 bytes
        let mut buf = vec![0u8; 10];
        buf[0..4].copy_from_slice(&self.index.to_le_bytes());
        buf[4] = self.family;
        buf[5] = self.prefix_len;
        buf[6..10].copy_from_slice(&self.addr_v4.0);
        buf
    }

    /// Deserialize from payload bytes
    pub fn deserialize(buf: &[u8]) -> Result<Self, KernelError> {
        if buf.len() < 10 {
            return Err(KernelError::InvalidArgument {
                name: "netlink",
                value: "invalid",
            });
        }

        let index = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
        let family = buf[4];
        let prefix_len = buf[5];
        let addr_v4 = Ipv4Address([buf[6], buf[7], buf[8], buf[9]]);

        Ok(Self {
            index,
            family,
            prefix_len,
            addr_v4,
        })
    }
}

// ---------------------------------------------------------------------------
// Route info payload
// ---------------------------------------------------------------------------

/// Route entry (serialized in NewRoute responses)
#[derive(Debug, Clone)]
pub struct RouteInfo {
    /// Destination network
    pub dest: Ipv4Address,
    /// Destination prefix length
    pub dest_prefix: u8,
    /// Gateway address
    pub gateway: Ipv4Address,
    /// Output interface index
    pub oif_index: u32,
    /// Route metric
    pub metric: u32,
}

impl RouteInfo {
    /// Serialize to payload bytes
    pub fn serialize(&self) -> Vec<u8> {
        // dest(4) + prefix(1) + gateway(4) + oif(4) + metric(4) = 17 bytes
        let mut buf = vec![0u8; 17];
        buf[0..4].copy_from_slice(&self.dest.0);
        buf[4] = self.dest_prefix;
        buf[5..9].copy_from_slice(&self.gateway.0);
        buf[9..13].copy_from_slice(&self.oif_index.to_le_bytes());
        buf[13..17].copy_from_slice(&self.metric.to_le_bytes());
        buf
    }

    /// Deserialize from payload bytes
    pub fn deserialize(buf: &[u8]) -> Result<Self, KernelError> {
        if buf.len() < 17 {
            return Err(KernelError::InvalidArgument {
                name: "netlink",
                value: "invalid",
            });
        }

        Ok(Self {
            dest: Ipv4Address([buf[0], buf[1], buf[2], buf[3]]),
            dest_prefix: buf[4],
            gateway: Ipv4Address([buf[5], buf[6], buf[7], buf[8]]),
            oif_index: u32::from_le_bytes([buf[9], buf[10], buf[11], buf[12]]),
            metric: u32::from_le_bytes([buf[13], buf[14], buf[15], buf[16]]),
        })
    }
}

// ---------------------------------------------------------------------------
// Netlink socket
// ---------------------------------------------------------------------------

/// Netlink socket for message passing between kernel and userland
pub struct NetlinkSocket {
    /// Socket ID
    id: u32,
    /// Process ID of the owner
    pid: u32,
    /// Receive queue
    rx_queue: Vec<NetlinkMessage>,
    /// Sequence counter
    next_seq: u32,
}

impl NetlinkSocket {
    /// Create a new netlink socket
    pub fn new(id: u32, pid: u32) -> Self {
        Self {
            id,
            pid,
            rx_queue: Vec::new(),
            next_seq: 1,
        }
    }

    /// Get the socket ID
    pub fn id(&self) -> u32 {
        self.id
    }

    /// Get the owning process ID
    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// Allocate next sequence number
    pub fn next_seq(&mut self) -> u32 {
        let seq = self.next_seq;
        self.next_seq = self.next_seq.wrapping_add(1);
        seq
    }

    /// Check if there are pending messages
    pub fn has_pending(&self) -> bool {
        !self.rx_queue.is_empty()
    }

    /// Number of pending messages
    pub fn pending_count(&self) -> usize {
        self.rx_queue.len()
    }
}

// ---------------------------------------------------------------------------
// Global netlink socket registry
// ---------------------------------------------------------------------------

struct NetlinkRegistry {
    sockets: Vec<NetlinkSocket>,
    next_id: u32,
}

impl NetlinkRegistry {
    const fn new() -> Self {
        Self {
            sockets: Vec::new(),
            next_id: 1,
        }
    }
}

static REGISTRY: Mutex<NetlinkRegistry> = Mutex::new(NetlinkRegistry::new());

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Send a netlink message (userland -> kernel)
///
/// Processes the message and enqueues any response(s) on the socket's
/// receive queue.
pub fn netlink_send(socket_id: u32, msg: &NetlinkMessage) -> Result<(), KernelError> {
    let mut registry = REGISTRY.lock();

    let socket = registry
        .sockets
        .iter_mut()
        .find(|s| s.id == socket_id)
        .ok_or(KernelError::InvalidArgument {
            name: "netlink",
            value: "invalid",
        })?;

    let msg_type =
        NetlinkMessageType::from_u16(msg.header.msg_type).ok_or(KernelError::InvalidArgument {
            name: "netlink",
            value: "invalid",
        })?;

    match msg_type {
        NetlinkMessageType::LinkUp | NetlinkMessageType::LinkDown => {
            // Extract interface name from payload
            if msg.payload.len() < MAX_IFNAME_LEN {
                // TODO: delegate to net::device to bring interface up/down
                let ack = NetlinkMessage::done(msg.header.seq, 0);
                if socket.rx_queue.len() < MAX_QUEUE_DEPTH {
                    socket.rx_queue.push(ack);
                }
            } else {
                let err = NetlinkMessage::error(msg.header.seq, 0, -22); // EINVAL
                if socket.rx_queue.len() < MAX_QUEUE_DEPTH {
                    socket.rx_queue.push(err);
                }
            }
        }
        NetlinkMessageType::GetLinks => {
            // TODO: enumerate devices from net::device registry
            // For now, send a Done message
            let done = NetlinkMessage::done(msg.header.seq, 0);
            if socket.rx_queue.len() < MAX_QUEUE_DEPTH {
                socket.rx_queue.push(done);
            }
        }
        NetlinkMessageType::AddrAdd | NetlinkMessageType::AddrDel => {
            // TODO: delegate to IP configuration
            let ack = NetlinkMessage::done(msg.header.seq, 0);
            if socket.rx_queue.len() < MAX_QUEUE_DEPTH {
                socket.rx_queue.push(ack);
            }
        }
        NetlinkMessageType::GetAddrs => {
            // TODO: enumerate addresses
            let done = NetlinkMessage::done(msg.header.seq, 0);
            if socket.rx_queue.len() < MAX_QUEUE_DEPTH {
                socket.rx_queue.push(done);
            }
        }
        NetlinkMessageType::RouteAdd | NetlinkMessageType::RouteDel => {
            // TODO: delegate to routing table
            let ack = NetlinkMessage::done(msg.header.seq, 0);
            if socket.rx_queue.len() < MAX_QUEUE_DEPTH {
                socket.rx_queue.push(ack);
            }
        }
        NetlinkMessageType::GetRoutes => {
            // TODO: enumerate routes
            let done = NetlinkMessage::done(msg.header.seq, 0);
            if socket.rx_queue.len() < MAX_QUEUE_DEPTH {
                socket.rx_queue.push(done);
            }
        }
        _ => {
            let err = NetlinkMessage::error(msg.header.seq, 0, -95); // EOPNOTSUPP
            if socket.rx_queue.len() < MAX_QUEUE_DEPTH {
                socket.rx_queue.push(err);
            }
        }
    }

    Ok(())
}

/// Receive a netlink message (kernel -> userland)
///
/// Returns the next pending message from the socket's receive queue,
/// or None if the queue is empty.
pub fn netlink_recv(socket_id: u32) -> Result<Option<NetlinkMessage>, KernelError> {
    let mut registry = REGISTRY.lock();

    let socket = registry
        .sockets
        .iter_mut()
        .find(|s| s.id == socket_id)
        .ok_or(KernelError::InvalidArgument {
            name: "netlink",
            value: "invalid",
        })?;

    if socket.rx_queue.is_empty() {
        return Ok(None);
    }

    Ok(Some(socket.rx_queue.remove(0)))
}

/// Open a new netlink socket
///
/// Returns the socket ID for use with netlink_send/netlink_recv.
pub fn netlink_open(pid: u32) -> Result<u32, KernelError> {
    let mut registry = REGISTRY.lock();

    if registry.sockets.len() >= MAX_SOCKETS {
        return Err(KernelError::ResourceExhausted {
            resource: "netlink_sockets",
        });
    }

    let id = registry.next_id;
    registry.next_id = registry.next_id.wrapping_add(1);

    let socket = NetlinkSocket::new(id, pid);
    registry.sockets.push(socket);

    Ok(id)
}

/// Close a netlink socket
pub fn netlink_close(socket_id: u32) -> Result<(), KernelError> {
    let mut registry = REGISTRY.lock();

    let pos = registry
        .sockets
        .iter()
        .position(|s| s.id == socket_id)
        .ok_or(KernelError::InvalidArgument {
            name: "netlink",
            value: "invalid",
        })?;

    registry.sockets.remove(pos);
    Ok(())
}

/// Initialize the netlink subsystem
pub fn init() -> Result<(), KernelError> {
    // Registry is already initialized via const fn
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use alloc::vec;

    use super::*;

    #[test]
    fn test_header_serialize_deserialize() {
        let header = NetlinkHeader::new(
            NetlinkMessageType::GetLinks as u16,
            flags::NLM_F_REQUEST | flags::NLM_F_DUMP,
            42,
            1000,
        );

        let mut buf = [0u8; NetlinkHeader::SIZE];
        header.serialize(&mut buf).unwrap();

        let decoded = NetlinkHeader::deserialize(&buf).unwrap();
        assert_eq!(decoded.msg_type, NetlinkMessageType::GetLinks as u16);
        assert_eq!(decoded.flags, flags::NLM_F_REQUEST | flags::NLM_F_DUMP);
        assert_eq!(decoded.seq, 42);
        assert_eq!(decoded.pid, 1000);
    }

    #[test]
    fn test_message_serialize_deserialize() {
        let mut msg =
            NetlinkMessage::new(NetlinkMessageType::AddrAdd, flags::NLM_F_REQUEST, 1, 100);

        let addr = AddrInfo {
            index: 2,
            family: 2,
            prefix_len: 24,
            addr_v4: Ipv4Address::new(192, 168, 1, 100),
        };
        msg.set_payload(&addr.serialize()).unwrap();

        let bytes = msg.serialize().unwrap();
        let decoded = NetlinkMessage::deserialize(&bytes).unwrap();

        assert_eq!(decoded.header.msg_type, NetlinkMessageType::AddrAdd as u16);
        assert_eq!(decoded.payload.len(), 10);

        let decoded_addr = AddrInfo::deserialize(&decoded.payload).unwrap();
        assert_eq!(decoded_addr.index, 2);
        assert_eq!(decoded_addr.prefix_len, 24);
        assert_eq!(decoded_addr.addr_v4, Ipv4Address::new(192, 168, 1, 100));
    }

    #[test]
    fn test_link_info_serialize_deserialize() {
        let link = LinkInfo {
            index: 1,
            name: String::from("eth0"),
            mac: MacAddress::new([0x00, 0x11, 0x22, 0x33, 0x44, 0x55]),
            mtu: 1500,
            flags: 0x1043, // UP | RUNNING | BROADCAST | MULTICAST
            if_type: 1,
            speed: 1000,
        };

        let bytes = link.serialize();
        let decoded = LinkInfo::deserialize(&bytes).unwrap();

        assert_eq!(decoded.index, 1);
        assert_eq!(decoded.name, "eth0");
        assert_eq!(
            decoded.mac,
            MacAddress::new([0x00, 0x11, 0x22, 0x33, 0x44, 0x55])
        );
        assert_eq!(decoded.mtu, 1500);
        assert_eq!(decoded.flags, 0x1043);
        assert_eq!(decoded.if_type, 1);
        assert_eq!(decoded.speed, 1000);
    }

    #[test]
    fn test_route_info_serialize_deserialize() {
        let route = RouteInfo {
            dest: Ipv4Address::new(0, 0, 0, 0),
            dest_prefix: 0,
            gateway: Ipv4Address::new(192, 168, 1, 1),
            oif_index: 2,
            metric: 100,
        };

        let bytes = route.serialize();
        let decoded = RouteInfo::deserialize(&bytes).unwrap();

        assert_eq!(decoded.dest, Ipv4Address::new(0, 0, 0, 0));
        assert_eq!(decoded.dest_prefix, 0);
        assert_eq!(decoded.gateway, Ipv4Address::new(192, 168, 1, 1));
        assert_eq!(decoded.oif_index, 2);
        assert_eq!(decoded.metric, 100);
    }

    #[test]
    fn test_netlink_socket_open_close() {
        let id = netlink_open(1234).unwrap();
        assert!(id > 0);
        netlink_close(id).unwrap();
    }

    #[test]
    fn test_netlink_send_recv() {
        let id = netlink_open(5678).unwrap();

        let msg = NetlinkMessage::new(
            NetlinkMessageType::GetLinks,
            flags::NLM_F_REQUEST | flags::NLM_F_DUMP,
            1,
            5678,
        );

        netlink_send(id, &msg).unwrap();

        let response = netlink_recv(id).unwrap();
        assert!(response.is_some());

        let resp = response.unwrap();
        assert_eq!(resp.header.msg_type, NetlinkMessageType::Done as u16);

        netlink_close(id).unwrap();
    }

    #[test]
    fn test_netlink_recv_empty() {
        let id = netlink_open(9999).unwrap();
        let response = netlink_recv(id).unwrap();
        assert!(response.is_none());
        netlink_close(id).unwrap();
    }

    #[test]
    fn test_message_type_from_u16() {
        assert_eq!(
            NetlinkMessageType::from_u16(16),
            Some(NetlinkMessageType::LinkUp)
        );
        assert_eq!(
            NetlinkMessageType::from_u16(48),
            Some(NetlinkMessageType::RouteAdd)
        );
        assert_eq!(NetlinkMessageType::from_u16(999), None);
    }

    #[test]
    fn test_error_message() {
        let err = NetlinkMessage::error(42, 100, -22);
        assert_eq!(err.header.msg_type, NetlinkMessageType::Error as u16);
        assert_eq!(err.payload.len(), 4);

        let errno = i32::from_le_bytes([
            err.payload[0],
            err.payload[1],
            err.payload[2],
            err.payload[3],
        ]);
        assert_eq!(errno, -22);
    }
}
