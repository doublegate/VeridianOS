//! Bluetooth HCI (Host Controller Interface) Driver
//!
//! Implements the HCI transport layer, command/event protocol, device
//! discovery, connection management, L2CAP basics, and SDP service discovery
//! stubs.
//!
//! Reference: Bluetooth Core Specification v5.4, Volume 4 (HCI)

#![allow(dead_code)]

#[cfg(feature = "alloc")]
#[allow(unused_imports)]
use alloc::vec::Vec;
use core::fmt;

use crate::error::KernelError;

// ---------------------------------------------------------------------------
// HCI Packet Types
// ---------------------------------------------------------------------------

/// HCI packet type indicators (UART H4 transport)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum HciPacketType {
    /// HCI Command packet (host -> controller)
    Command = 0x01,
    /// ACL Data packet (bidirectional)
    AclData = 0x02,
    /// SCO Data packet (bidirectional, synchronous)
    ScoData = 0x03,
    /// HCI Event packet (controller -> host)
    Event = 0x04,
}

impl HciPacketType {
    /// Parse packet type from raw byte
    pub fn from_u8(val: u8) -> Option<Self> {
        match val {
            0x01 => Some(Self::Command),
            0x02 => Some(Self::AclData),
            0x03 => Some(Self::ScoData),
            0x04 => Some(Self::Event),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// OpCode Groups (OGF) and OpCode Command Fields (OCF)
// ---------------------------------------------------------------------------

/// OpCode Group Field values
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Ogf {
    /// Link Control commands (connection management)
    LinkControl = 0x01,
    /// Link Policy commands
    LinkPolicy = 0x02,
    /// Controller & Baseband commands (configuration)
    ControllerBaseband = 0x03,
    /// Informational parameters (read-only queries)
    Informational = 0x04,
    /// Status parameters
    StatusParameters = 0x05,
    /// LE Controller commands
    LeController = 0x08,
}

impl Ogf {
    pub fn from_u8(val: u8) -> Option<Self> {
        match val {
            0x01 => Some(Self::LinkControl),
            0x02 => Some(Self::LinkPolicy),
            0x03 => Some(Self::ControllerBaseband),
            0x04 => Some(Self::Informational),
            0x05 => Some(Self::StatusParameters),
            0x08 => Some(Self::LeController),
            _ => None,
        }
    }
}

/// Build an HCI opcode from OGF and OCF fields
/// Format: bits [15:10] = OGF, bits [9:0] = OCF
pub const fn make_opcode(ogf: u8, ocf: u16) -> u16 {
    ((ogf as u16) << 10) | (ocf & 0x03FF)
}

/// Extract OGF from an HCI opcode
pub const fn opcode_ogf(opcode: u16) -> u8 {
    (opcode >> 10) as u8
}

/// Extract OCF from an HCI opcode
pub const fn opcode_ocf(opcode: u16) -> u16 {
    opcode & 0x03FF
}

// Well-known HCI command opcodes
/// HCI_Inquiry (OGF=0x01, OCF=0x0001)
pub const HCI_INQUIRY: u16 = make_opcode(0x01, 0x0001);
/// HCI_Create_Connection (OGF=0x01, OCF=0x0005)
pub const HCI_CREATE_CONNECTION: u16 = make_opcode(0x01, 0x0005);
/// HCI_Disconnect (OGF=0x01, OCF=0x0006)
pub const HCI_DISCONNECT: u16 = make_opcode(0x01, 0x0006);
/// HCI_Reset (OGF=0x03, OCF=0x0003)
pub const HCI_RESET: u16 = make_opcode(0x03, 0x0003);
/// HCI_Read_Local_Name (OGF=0x03, OCF=0x0014)
pub const HCI_READ_LOCAL_NAME: u16 = make_opcode(0x03, 0x0014);
/// HCI_Write_Scan_Enable (OGF=0x03, OCF=0x001A)
pub const HCI_WRITE_SCAN_ENABLE: u16 = make_opcode(0x03, 0x001A);
/// HCI_Read_BD_ADDR (OGF=0x04, OCF=0x0009)
pub const HCI_READ_BD_ADDR: u16 = make_opcode(0x04, 0x0009);

// ---------------------------------------------------------------------------
// HCI Command Packet
// ---------------------------------------------------------------------------

/// Maximum HCI command parameter length
pub const HCI_MAX_COMMAND_PARAMS: usize = 255;

/// HCI Command packet header + parameters
#[derive(Clone)]
pub struct HciCommand {
    /// Command opcode (OGF:OCF)
    pub opcode: u16,
    /// Parameter total length
    pub param_len: u8,
    /// Command parameters (up to 255 bytes)
    pub params: [u8; HCI_MAX_COMMAND_PARAMS],
}

impl fmt::Debug for HciCommand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HciCommand")
            .field("opcode", &format_args!("0x{:04X}", self.opcode))
            .field("ogf", &format_args!("0x{:02X}", opcode_ogf(self.opcode)))
            .field("ocf", &format_args!("0x{:03X}", opcode_ocf(self.opcode)))
            .field("param_len", &self.param_len)
            .finish()
    }
}

impl HciCommand {
    /// Create a new HCI command with no parameters
    pub fn new(opcode: u16) -> Self {
        Self {
            opcode,
            param_len: 0,
            params: [0u8; HCI_MAX_COMMAND_PARAMS],
        }
    }

    /// Create a new HCI command with parameters
    pub fn with_params(opcode: u16, params: &[u8]) -> Result<Self, KernelError> {
        if params.len() > HCI_MAX_COMMAND_PARAMS {
            return Err(KernelError::InvalidArgument {
                name: "params",
                value: "exceeds 255 bytes",
            });
        }
        let mut cmd = Self::new(opcode);
        cmd.param_len = params.len() as u8;
        cmd.params[..params.len()].copy_from_slice(params);
        Ok(cmd)
    }

    /// Serialize command to a byte buffer (H4 transport format)
    /// Returns number of bytes written
    pub fn serialize(&self, buf: &mut [u8]) -> Result<usize, KernelError> {
        let total = 1 + 3 + self.param_len as usize; // type + header + params
        if buf.len() < total {
            return Err(KernelError::InvalidArgument {
                name: "buffer",
                value: "too small for HCI command",
            });
        }
        buf[0] = HciPacketType::Command as u8;
        buf[1] = (self.opcode & 0xFF) as u8;
        buf[2] = (self.opcode >> 8) as u8;
        buf[3] = self.param_len;
        if self.param_len > 0 {
            buf[4..4 + self.param_len as usize]
                .copy_from_slice(&self.params[..self.param_len as usize]);
        }
        Ok(total)
    }
}

// ---------------------------------------------------------------------------
// HCI Event Packet
// ---------------------------------------------------------------------------

/// HCI Event codes
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum HciEventCode {
    /// Inquiry Complete
    InquiryComplete = 0x01,
    /// Inquiry Result
    InquiryResult = 0x02,
    /// Connection Complete
    ConnectionComplete = 0x03,
    /// Connection Request
    ConnectionRequest = 0x04,
    /// Disconnection Complete
    DisconnectionComplete = 0x05,
    /// Command Complete
    CommandComplete = 0x0E,
    /// Command Status
    CommandStatus = 0x0F,
    /// Number of Completed Packets
    NumberOfCompletedPackets = 0x13,
    /// Extended Inquiry Result
    ExtendedInquiryResult = 0x2F,
}

impl HciEventCode {
    pub fn from_u8(val: u8) -> Option<Self> {
        match val {
            0x01 => Some(Self::InquiryComplete),
            0x02 => Some(Self::InquiryResult),
            0x03 => Some(Self::ConnectionComplete),
            0x04 => Some(Self::ConnectionRequest),
            0x05 => Some(Self::DisconnectionComplete),
            0x0E => Some(Self::CommandComplete),
            0x0F => Some(Self::CommandStatus),
            0x13 => Some(Self::NumberOfCompletedPackets),
            0x2F => Some(Self::ExtendedInquiryResult),
            _ => None,
        }
    }
}

/// Maximum HCI event parameter length
pub const HCI_MAX_EVENT_PARAMS: usize = 255;

/// HCI Event packet
#[derive(Clone)]
pub struct HciEvent {
    /// Event code
    pub event_code: u8,
    /// Parameter total length
    pub param_len: u8,
    /// Event parameters
    pub params: [u8; HCI_MAX_EVENT_PARAMS],
}

impl fmt::Debug for HciEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HciEvent")
            .field("event_code", &format_args!("0x{:02X}", self.event_code))
            .field("param_len", &self.param_len)
            .finish()
    }
}

impl HciEvent {
    /// Create a new empty event
    pub fn new(event_code: u8) -> Self {
        Self {
            event_code,
            param_len: 0,
            params: [0u8; HCI_MAX_EVENT_PARAMS],
        }
    }

    /// Parse an HCI event from a raw buffer (excluding H4 type byte)
    pub fn parse(buf: &[u8]) -> Result<Self, KernelError> {
        if buf.len() < 2 {
            return Err(KernelError::InvalidArgument {
                name: "event_buffer",
                value: "too short for HCI event header",
            });
        }
        let event_code = buf[0];
        let param_len = buf[1];
        if buf.len() < 2 + param_len as usize {
            return Err(KernelError::InvalidArgument {
                name: "event_buffer",
                value: "truncated event parameters",
            });
        }
        let mut evt = Self::new(event_code);
        evt.param_len = param_len;
        if param_len > 0 {
            evt.params[..param_len as usize].copy_from_slice(&buf[2..2 + param_len as usize]);
        }
        Ok(evt)
    }

    /// Check if this is a Command Complete event
    pub fn is_command_complete(&self) -> bool {
        self.event_code == HciEventCode::CommandComplete as u8
    }

    /// Check if this is a Command Status event
    pub fn is_command_status(&self) -> bool {
        self.event_code == HciEventCode::CommandStatus as u8
    }

    /// For Command Complete events, extract the opcode that completed
    pub fn command_complete_opcode(&self) -> Option<u16> {
        if !self.is_command_complete() || self.param_len < 3 {
            return None;
        }
        // params[0] = num_hci_command_packets, params[1..3] = opcode (LE)
        Some(u16::from_le_bytes([self.params[1], self.params[2]]))
    }

    /// For Command Complete events, extract the status byte
    pub fn command_complete_status(&self) -> Option<u8> {
        if !self.is_command_complete() || self.param_len < 4 {
            return None;
        }
        Some(self.params[3])
    }

    /// For Command Status events, extract the status byte
    pub fn command_status(&self) -> Option<u8> {
        if !self.is_command_status() || self.param_len < 1 {
            return None;
        }
        Some(self.params[0])
    }
}

// ---------------------------------------------------------------------------
// ACL Data Packet
// ---------------------------------------------------------------------------

/// ACL packet boundary flags
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum AclBoundaryFlag {
    /// First non-automatically-flushable packet
    FirstNonFlushable = 0x00,
    /// Continuing fragment
    Continuing = 0x01,
    /// First automatically flushable packet
    FirstFlushable = 0x02,
}

/// ACL packet broadcast flag
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum AclBroadcastFlag {
    /// Point-to-point
    PointToPoint = 0x00,
    /// Active Broadcast
    ActiveBroadcast = 0x01,
}

/// Maximum ACL data payload
pub const ACL_MAX_DATA_LEN: usize = 1021;

/// ACL Data packet header
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AclHeader {
    /// Connection handle (12 bits)
    pub handle: u16,
    /// Packet boundary flag (2 bits)
    pub boundary: AclBoundaryFlag,
    /// Broadcast flag (2 bits)
    pub broadcast: AclBroadcastFlag,
    /// Data total length
    pub data_len: u16,
}

impl AclHeader {
    /// Parse ACL header from 4 bytes
    pub fn parse(buf: &[u8]) -> Result<Self, KernelError> {
        if buf.len() < 4 {
            return Err(KernelError::InvalidArgument {
                name: "acl_buffer",
                value: "too short for ACL header",
            });
        }
        let hdr_word = u16::from_le_bytes([buf[0], buf[1]]);
        let handle = hdr_word & 0x0FFF;
        let pb = (hdr_word >> 12) & 0x03;
        let bc = (hdr_word >> 14) & 0x03;
        let data_len = u16::from_le_bytes([buf[2], buf[3]]);

        let boundary = match pb {
            0x00 => AclBoundaryFlag::FirstNonFlushable,
            0x01 => AclBoundaryFlag::Continuing,
            0x02 => AclBoundaryFlag::FirstFlushable,
            _ => AclBoundaryFlag::FirstFlushable,
        };
        let broadcast = match bc {
            0x00 => AclBroadcastFlag::PointToPoint,
            0x01 => AclBroadcastFlag::ActiveBroadcast,
            _ => AclBroadcastFlag::PointToPoint,
        };

        Ok(Self {
            handle,
            boundary,
            broadcast,
            data_len,
        })
    }

    /// Serialize ACL header into 4 bytes
    pub fn serialize(&self, buf: &mut [u8]) -> Result<(), KernelError> {
        if buf.len() < 4 {
            return Err(KernelError::InvalidArgument {
                name: "buffer",
                value: "too small for ACL header",
            });
        }
        let hdr_word = (self.handle & 0x0FFF)
            | ((self.boundary as u16) << 12)
            | ((self.broadcast as u16) << 14);
        buf[0..2].copy_from_slice(&hdr_word.to_le_bytes());
        buf[2..4].copy_from_slice(&self.data_len.to_le_bytes());
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// BD_ADDR (Bluetooth Device Address)
// ---------------------------------------------------------------------------

/// Bluetooth device address (6 bytes, little-endian)
pub type BdAddr = [u8; 6];

/// Zero / unset BD_ADDR
pub const BD_ADDR_ZERO: BdAddr = [0u8; 6];

/// Format a BD_ADDR as colon-separated hex string
pub fn format_bd_addr(addr: &BdAddr) -> [u8; 17] {
    let hex = b"0123456789ABCDEF";
    let mut out = [0u8; 17];
    for i in 0..6 {
        let offset = (5 - i) * 3; // reverse byte order for display
        out[offset] = hex[(addr[i] >> 4) as usize];
        out[offset + 1] = hex[(addr[i] & 0x0F) as usize];
        if i < 5 {
            out[offset + 2] = b':';
        }
    }
    out
}

// ---------------------------------------------------------------------------
// HCI USB Transport
// ---------------------------------------------------------------------------

/// USB HCI transport endpoint configuration
#[derive(Debug, Clone, Copy)]
pub struct HciUsbTransport {
    /// USB device address
    pub device_addr: u8,
    /// Bulk OUT endpoint for HCI commands
    pub cmd_endpoint: u8,
    /// Bulk IN endpoint for HCI events
    pub evt_endpoint: u8,
    /// Bulk OUT endpoint for ACL data TX
    pub acl_tx_endpoint: u8,
    /// Bulk IN endpoint for ACL data RX
    pub acl_rx_endpoint: u8,
    /// Interrupt IN endpoint for events (alternative)
    pub intr_endpoint: u8,
    /// Whether the transport is active
    pub active: bool,
}

impl Default for HciUsbTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl HciUsbTransport {
    /// Create a new USB HCI transport (unconfigured)
    pub fn new() -> Self {
        Self {
            device_addr: 0,
            cmd_endpoint: 0x00,
            evt_endpoint: 0x81,
            acl_tx_endpoint: 0x02,
            acl_rx_endpoint: 0x82,
            intr_endpoint: 0x83,
            active: false,
        }
    }

    /// Configure the transport for a specific USB device
    pub fn configure(&mut self, device_addr: u8) {
        self.device_addr = device_addr;
        self.active = true;
    }

    /// Send an HCI command over USB bulk OUT
    pub fn send_command(&self, _cmd: &HciCommand) -> Result<(), KernelError> {
        if !self.active {
            return Err(KernelError::InvalidState {
                expected: "active",
                actual: "inactive",
            });
        }
        // In a real implementation, this would submit a USB bulk transfer
        // to the command endpoint. For now, this is a stub.
        Ok(())
    }

    /// Send ACL data over USB bulk OUT
    pub fn send_acl_data(&self, _header: &AclHeader, _data: &[u8]) -> Result<(), KernelError> {
        if !self.active {
            return Err(KernelError::InvalidState {
                expected: "active",
                actual: "inactive",
            });
        }
        Ok(())
    }

    /// Poll for an HCI event from USB interrupt/bulk IN endpoint
    pub fn poll_event(&self) -> Result<Option<HciEvent>, KernelError> {
        if !self.active {
            return Err(KernelError::InvalidState {
                expected: "active",
                actual: "inactive",
            });
        }
        // Stub: no real USB hardware, return None
        Ok(None)
    }

    /// Poll for ACL data from USB bulk IN endpoint
    pub fn poll_acl_data(
        &self,
    ) -> Result<Option<(AclHeader, [u8; ACL_MAX_DATA_LEN], usize)>, KernelError> {
        if !self.active {
            return Err(KernelError::InvalidState {
                expected: "active",
                actual: "inactive",
            });
        }
        Ok(None)
    }
}

// ---------------------------------------------------------------------------
// Device Discovery
// ---------------------------------------------------------------------------

/// General Inquiry Access Code LAP (GIAC)
pub const GIAC_LAP: u32 = 0x9E8B33;

/// Limited Inquiry Access Code LAP (LIAC)
pub const LIAC_LAP: u32 = 0x9E8B00;

/// Scan enable modes for HCI_Write_Scan_Enable
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ScanEnable {
    /// No scans enabled
    NoScans = 0x00,
    /// Inquiry scan enabled
    InquiryScanOnly = 0x01,
    /// Page scan enabled
    PageScanOnly = 0x02,
    /// Both inquiry and page scan enabled
    InquiryAndPageScan = 0x03,
}

/// Maximum number of discovered devices
pub const MAX_DISCOVERED_DEVICES: usize = 16;

/// A discovered Bluetooth device
#[derive(Debug, Clone, Copy)]
pub struct DiscoveredDevice {
    /// Device Bluetooth address
    pub addr: BdAddr,
    /// Page scan repetition mode
    pub page_scan_rep_mode: u8,
    /// Class of device (3 bytes packed into u32)
    pub class_of_device: u32,
    /// Clock offset
    pub clock_offset: u16,
    /// RSSI (signed, dBm)
    pub rssi: i8,
    /// Device name (from name resolution or EIR)
    pub name: [u8; 248],
    /// Length of valid name bytes
    pub name_len: usize,
    /// Whether this entry is occupied
    pub valid: bool,
}

impl DiscoveredDevice {
    pub const fn empty() -> Self {
        Self {
            addr: BD_ADDR_ZERO,
            page_scan_rep_mode: 0,
            class_of_device: 0,
            clock_offset: 0,
            rssi: 0,
            name: [0u8; 248],
            name_len: 0,
            valid: false,
        }
    }
}

/// Parse an Extended Inquiry Response (EIR) data block
/// Returns (name_bytes, name_len) if a Complete/Shortened Local Name is found
pub fn parse_eir_name(eir: &[u8]) -> Option<([u8; 248], usize)> {
    let mut offset = 0;
    while offset < eir.len() {
        let length = eir[offset] as usize;
        if length == 0 {
            break;
        }
        if offset + 1 + length > eir.len() {
            break;
        }
        let data_type = eir[offset + 1];
        // 0x08 = Shortened Local Name, 0x09 = Complete Local Name
        if data_type == 0x08 || data_type == 0x09 {
            let name_data = &eir[offset + 2..offset + 1 + length];
            let copy_len = name_data.len().min(248);
            let mut name = [0u8; 248];
            name[..copy_len].copy_from_slice(&name_data[..copy_len]);
            return Some((name, copy_len));
        }
        offset += 1 + length;
    }
    None
}

// ---------------------------------------------------------------------------
// Connection Management
// ---------------------------------------------------------------------------

/// Maximum number of simultaneous connections
pub const MAX_CONNECTIONS: usize = 8;

/// Connection state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    /// No connection
    Idle,
    /// Connection setup in progress
    Connecting,
    /// Connection established
    Connected,
    /// Disconnection in progress
    Disconnecting,
}

/// An active HCI connection
#[derive(Debug, Clone, Copy)]
pub struct HciConnection {
    /// Connection handle (assigned by controller)
    pub handle: u16,
    /// Remote device address
    pub remote_addr: BdAddr,
    /// Connection type (ACL = 0x01, SCO = 0x00)
    pub link_type: u8,
    /// Current state
    pub state: ConnectionState,
    /// Encryption enabled
    pub encrypted: bool,
    /// Link key (stub -- 16 bytes)
    pub link_key: [u8; 16],
    /// Whether a link key is present
    pub has_link_key: bool,
}

impl HciConnection {
    pub const fn empty() -> Self {
        Self {
            handle: 0,
            remote_addr: BD_ADDR_ZERO,
            link_type: 0x01,
            state: ConnectionState::Idle,
            encrypted: false,
            link_key: [0u8; 16],
            has_link_key: false,
        }
    }
}

// ---------------------------------------------------------------------------
// L2CAP (Logical Link Control and Adaptation Protocol) Basics
// ---------------------------------------------------------------------------

/// L2CAP signaling channel CID
pub const L2CAP_CID_SIGNALING: u16 = 0x0001;
/// L2CAP connectionless channel CID
pub const L2CAP_CID_CONNECTIONLESS: u16 = 0x0002;
/// L2CAP ATT (Attribute Protocol) fixed channel CID
pub const L2CAP_CID_ATT: u16 = 0x0004;
/// L2CAP LE Signaling channel CID
pub const L2CAP_CID_LE_SIGNALING: u16 = 0x0005;
/// L2CAP SMP (Security Manager Protocol) fixed channel CID
pub const L2CAP_CID_SMP: u16 = 0x0006;

/// L2CAP header (4 bytes: length + channel ID)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct L2capHeader {
    /// Payload length (excluding this header)
    pub length: u16,
    /// Channel ID
    pub cid: u16,
}

impl L2capHeader {
    /// Parse L2CAP header from 4 bytes
    pub fn parse(buf: &[u8]) -> Result<Self, KernelError> {
        if buf.len() < 4 {
            return Err(KernelError::InvalidArgument {
                name: "l2cap_buffer",
                value: "too short for L2CAP header",
            });
        }
        Ok(Self {
            length: u16::from_le_bytes([buf[0], buf[1]]),
            cid: u16::from_le_bytes([buf[2], buf[3]]),
        })
    }

    /// Serialize L2CAP header into 4 bytes
    pub fn serialize(&self, buf: &mut [u8]) -> Result<(), KernelError> {
        if buf.len() < 4 {
            return Err(KernelError::InvalidArgument {
                name: "buffer",
                value: "too small for L2CAP header",
            });
        }
        buf[0..2].copy_from_slice(&self.length.to_le_bytes());
        buf[2..4].copy_from_slice(&self.cid.to_le_bytes());
        Ok(())
    }
}

/// L2CAP signaling command codes
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum L2capSignalCode {
    /// Connection Request
    ConnectionRequest = 0x02,
    /// Connection Response
    ConnectionResponse = 0x03,
    /// Configuration Request
    ConfigurationRequest = 0x04,
    /// Configuration Response
    ConfigurationResponse = 0x05,
    /// Disconnection Request
    DisconnectionRequest = 0x06,
    /// Disconnection Response
    DisconnectionResponse = 0x07,
    /// Information Request
    InformationRequest = 0x0A,
    /// Information Response
    InformationResponse = 0x0B,
}

/// L2CAP signaling packet header (code + identifier + length)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct L2capSignalHeader {
    pub code: u8,
    pub identifier: u8,
    pub length: u16,
}

impl L2capSignalHeader {
    pub fn parse(buf: &[u8]) -> Result<Self, KernelError> {
        if buf.len() < 4 {
            return Err(KernelError::InvalidArgument {
                name: "signal_buffer",
                value: "too short for L2CAP signal header",
            });
        }
        Ok(Self {
            code: buf[0],
            identifier: buf[1],
            length: u16::from_le_bytes([buf[2], buf[3]]),
        })
    }
}

// ---------------------------------------------------------------------------
// SDP (Service Discovery Protocol) Stubs
// ---------------------------------------------------------------------------

/// SDP PDU IDs
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum SdpPduId {
    /// Error Response
    ErrorResponse = 0x01,
    /// Service Search Request
    ServiceSearchRequest = 0x02,
    /// Service Search Response
    ServiceSearchResponse = 0x03,
    /// Service Attribute Request
    ServiceAttributeRequest = 0x04,
    /// Service Attribute Response
    ServiceAttributeResponse = 0x05,
    /// Service Search Attribute Request
    ServiceSearchAttributeRequest = 0x06,
    /// Service Search Attribute Response
    ServiceSearchAttributeResponse = 0x07,
}

/// Common Bluetooth UUIDs (16-bit short form)
pub const UUID_SDP: u16 = 0x0001;
pub const UUID_RFCOMM: u16 = 0x0003;
pub const UUID_L2CAP: u16 = 0x0100;
pub const UUID_SERIAL_PORT: u16 = 0x1101;
pub const UUID_OBEX_PUSH: u16 = 0x1105;
pub const UUID_A2DP_SOURCE: u16 = 0x110A;
pub const UUID_A2DP_SINK: u16 = 0x110B;
pub const UUID_HFP: u16 = 0x111E;
pub const UUID_HID: u16 = 0x1124;

/// SDP service record (stub)
#[derive(Debug, Clone, Copy)]
pub struct SdpServiceRecord {
    /// Service record handle
    pub handle: u32,
    /// Primary service class UUID (16-bit)
    pub service_class_uuid: u16,
    /// Protocol descriptor (L2CAP PSM or RFCOMM channel)
    pub protocol_channel: u16,
    /// Whether this record is valid
    pub valid: bool,
}

impl SdpServiceRecord {
    pub const fn empty() -> Self {
        Self {
            handle: 0,
            service_class_uuid: 0,
            protocol_channel: 0,
            valid: false,
        }
    }
}

/// Maximum number of SDP service records
pub const MAX_SDP_RECORDS: usize = 8;

/// Check if a 16-bit UUID matches a service record
pub fn sdp_uuid_match(record: &SdpServiceRecord, uuid: u16) -> bool {
    record.valid && record.service_class_uuid == uuid
}

// ---------------------------------------------------------------------------
// Bluetooth Controller State Machine
// ---------------------------------------------------------------------------

/// Bluetooth controller state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControllerState {
    /// Controller is powered off
    Off,
    /// Controller is initializing (reset sequence)
    Initializing,
    /// Controller is ready for commands
    Ready,
    /// Inquiry scan is active
    Scanning,
    /// At least one connection is active
    Connected,
}

/// Statistics for the Bluetooth controller
#[derive(Debug, Clone, Copy, Default)]
pub struct BluetoothStats {
    pub commands_sent: u64,
    pub events_received: u64,
    pub acl_packets_sent: u64,
    pub acl_packets_received: u64,
    pub errors: u64,
}

/// Main Bluetooth HCI controller
pub struct BluetoothController {
    /// Current controller state
    state: ControllerState,
    /// Local BD_ADDR (read from controller)
    local_addr: BdAddr,
    /// Local device name
    local_name: [u8; 248],
    /// Length of valid local name bytes
    local_name_len: usize,
    /// USB HCI transport
    transport: HciUsbTransport,
    /// Active connections
    connections: [HciConnection; MAX_CONNECTIONS],
    /// Number of active connections
    connection_count: usize,
    /// Discovered devices (from inquiry)
    discovered: [DiscoveredDevice; MAX_DISCOVERED_DEVICES],
    /// Number of discovered devices
    discovered_count: usize,
    /// SDP service records (local)
    sdp_records: [SdpServiceRecord; MAX_SDP_RECORDS],
    /// Number of registered SDP records
    sdp_record_count: usize,
    /// Next L2CAP signaling identifier
    next_signal_id: u8,
    /// Statistics
    stats: BluetoothStats,
}

impl fmt::Debug for BluetoothController {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BluetoothController")
            .field("state", &self.state)
            .field("local_addr", &format_args!("{:?}", self.local_addr))
            .field("connections", &self.connection_count)
            .field("discovered", &self.discovered_count)
            .finish()
    }
}

impl Default for BluetoothController {
    fn default() -> Self {
        Self::new()
    }
}

impl BluetoothController {
    /// Create a new Bluetooth controller (uninitialized)
    pub fn new() -> Self {
        Self {
            state: ControllerState::Off,
            local_addr: BD_ADDR_ZERO,
            local_name: [0u8; 248],
            local_name_len: 0,
            transport: HciUsbTransport::new(),
            connections: [HciConnection::empty(); MAX_CONNECTIONS],
            connection_count: 0,
            discovered: {
                // const array init without requiring Copy on large struct
                let empty = DiscoveredDevice::empty();
                [empty; MAX_DISCOVERED_DEVICES]
            },
            discovered_count: 0,
            sdp_records: [SdpServiceRecord::empty(); MAX_SDP_RECORDS],
            sdp_record_count: 0,
            next_signal_id: 1,
            stats: BluetoothStats::default(),
        }
    }

    /// Get current controller state
    pub fn state(&self) -> ControllerState {
        self.state
    }

    /// Get the local BD_ADDR
    pub fn local_addr(&self) -> &BdAddr {
        &self.local_addr
    }

    /// Get controller statistics
    pub fn stats(&self) -> &BluetoothStats {
        &self.stats
    }

    /// Get number of active connections
    pub fn connection_count(&self) -> usize {
        self.connection_count
    }

    /// Get number of discovered devices
    pub fn discovered_count(&self) -> usize {
        self.discovered_count
    }

    // ----- Initialization and Reset -----

    /// Initialize the controller: configure USB transport and send HCI_Reset
    pub fn initialize(&mut self, usb_device_addr: u8) -> Result<(), KernelError> {
        if self.state != ControllerState::Off {
            return Err(KernelError::InvalidState {
                expected: "Off",
                actual: "not Off",
            });
        }

        self.state = ControllerState::Initializing;
        self.transport.configure(usb_device_addr);

        // Send HCI_Reset
        self.send_reset()?;

        self.state = ControllerState::Ready;
        Ok(())
    }

    /// Send HCI_Reset command
    pub fn send_reset(&mut self) -> Result<(), KernelError> {
        let cmd = HciCommand::new(HCI_RESET);
        self.transport.send_command(&cmd)?;
        self.stats.commands_sent += 1;
        Ok(())
    }

    /// Send HCI_Read_BD_ADDR command
    pub fn read_bd_addr(&mut self) -> Result<(), KernelError> {
        self.ensure_ready()?;
        let cmd = HciCommand::new(HCI_READ_BD_ADDR);
        self.transport.send_command(&cmd)?;
        self.stats.commands_sent += 1;
        Ok(())
    }

    /// Send HCI_Read_Local_Name command
    pub fn read_local_name(&mut self) -> Result<(), KernelError> {
        self.ensure_ready()?;
        let cmd = HciCommand::new(HCI_READ_LOCAL_NAME);
        self.transport.send_command(&cmd)?;
        self.stats.commands_sent += 1;
        Ok(())
    }

    /// Send HCI_Write_Scan_Enable command
    pub fn write_scan_enable(&mut self, mode: ScanEnable) -> Result<(), KernelError> {
        self.ensure_ready()?;
        let cmd = HciCommand::with_params(HCI_WRITE_SCAN_ENABLE, &[mode as u8])?;
        self.transport.send_command(&cmd)?;
        self.stats.commands_sent += 1;
        Ok(())
    }

    // ----- Device Discovery -----

    /// Start inquiry (device discovery)
    /// inquiry_length: N * 1.28s (range 1..=30)
    /// max_responses: max number of responses (0 = unlimited)
    pub fn start_inquiry(
        &mut self,
        inquiry_length: u8,
        max_responses: u8,
    ) -> Result<(), KernelError> {
        self.ensure_ready()?;

        // Clear previous results
        self.discovered_count = 0;
        for dev in &mut self.discovered {
            dev.valid = false;
        }

        // Build Inquiry command parameters:
        // LAP (3 bytes, GIAC), inquiry_length, max_responses
        let lap_bytes = GIAC_LAP.to_le_bytes();
        let params = [
            lap_bytes[0],
            lap_bytes[1],
            lap_bytes[2],
            inquiry_length.clamp(1, 30),
            max_responses,
        ];
        let cmd = HciCommand::with_params(HCI_INQUIRY, &params)?;
        self.transport.send_command(&cmd)?;
        self.stats.commands_sent += 1;

        self.state = ControllerState::Scanning;
        Ok(())
    }

    /// Process an Inquiry Result event and store discovered device
    pub fn handle_inquiry_result(
        &mut self,
        addr: BdAddr,
        page_scan_rep_mode: u8,
        class_of_device: u32,
        clock_offset: u16,
    ) {
        if self.discovered_count >= MAX_DISCOVERED_DEVICES {
            return;
        }

        // Check for duplicate
        for dev in &self.discovered[..self.discovered_count] {
            if dev.valid && dev.addr == addr {
                return;
            }
        }

        let slot = &mut self.discovered[self.discovered_count];
        slot.addr = addr;
        slot.page_scan_rep_mode = page_scan_rep_mode;
        slot.class_of_device = class_of_device;
        slot.clock_offset = clock_offset;
        slot.valid = true;
        self.discovered_count += 1;
    }

    /// Handle Inquiry Complete event
    pub fn handle_inquiry_complete(&mut self) {
        if self.state == ControllerState::Scanning {
            if self.connection_count > 0 {
                self.state = ControllerState::Connected;
            } else {
                self.state = ControllerState::Ready;
            }
        }
    }

    /// Get a discovered device by index
    pub fn get_discovered(&self, index: usize) -> Option<&DiscoveredDevice> {
        if index < self.discovered_count {
            let dev = &self.discovered[index];
            if dev.valid {
                return Some(dev);
            }
        }
        None
    }

    // ----- Connection Management -----

    /// Create a connection to a remote device
    pub fn create_connection(&mut self, addr: &BdAddr) -> Result<(), KernelError> {
        self.ensure_ready_or_connected()?;

        if self.connection_count >= MAX_CONNECTIONS {
            return Err(KernelError::ResourceExhausted {
                resource: "bluetooth connections",
            });
        }

        // Build HCI_Create_Connection parameters:
        // BD_ADDR (6), Packet_Type (2), Page_Scan_Rep_Mode (1),
        // Reserved (1), Clock_Offset (2), Allow_Role_Switch (1)
        let mut params = [0u8; 13];
        params[0..6].copy_from_slice(addr);
        // Packet type: DM1 + DH1 + DM3 + DH3 + DM5 + DH5
        params[6..8].copy_from_slice(&0xCC18u16.to_le_bytes());
        params[8] = 0x02; // R2 page scan repetition mode
        params[9] = 0x00; // reserved
        params[10..12].copy_from_slice(&0x0000u16.to_le_bytes()); // clock offset
        params[12] = 0x01; // allow role switch

        let cmd = HciCommand::with_params(HCI_CREATE_CONNECTION, &params)?;
        self.transport.send_command(&cmd)?;
        self.stats.commands_sent += 1;

        // Allocate a connection slot in Connecting state
        for conn in &mut self.connections {
            if conn.state == ConnectionState::Idle {
                conn.remote_addr = *addr;
                conn.state = ConnectionState::Connecting;
                break;
            }
        }

        Ok(())
    }

    /// Handle Connection Complete event from controller
    pub fn handle_connection_complete(
        &mut self,
        status: u8,
        handle: u16,
        addr: &BdAddr,
        link_type: u8,
    ) -> Result<(), KernelError> {
        // Find the connection slot for this address
        for conn in &mut self.connections {
            if conn.state == ConnectionState::Connecting && conn.remote_addr == *addr {
                if status == 0x00 {
                    conn.handle = handle;
                    conn.link_type = link_type;
                    conn.state = ConnectionState::Connected;
                    self.connection_count += 1;
                    self.state = ControllerState::Connected;
                } else {
                    conn.state = ConnectionState::Idle;
                    conn.remote_addr = BD_ADDR_ZERO;
                }
                return Ok(());
            }
        }
        Err(KernelError::NotFound {
            resource: "connection slot",
            id: 0,
        })
    }

    /// Disconnect from a remote device
    pub fn disconnect(&mut self, handle: u16, reason: u8) -> Result<(), KernelError> {
        // Find connection by handle
        let found = self.connections.iter_mut().any(|conn| {
            if conn.state == ConnectionState::Connected && conn.handle == handle {
                conn.state = ConnectionState::Disconnecting;
                true
            } else {
                false
            }
        });

        if !found {
            return Err(KernelError::NotFound {
                resource: "connection",
                id: handle as u64,
            });
        }

        let mut params = [0u8; 3];
        params[0..2].copy_from_slice(&handle.to_le_bytes());
        params[2] = reason;

        let cmd = HciCommand::with_params(HCI_DISCONNECT, &params)?;
        self.transport.send_command(&cmd)?;
        self.stats.commands_sent += 1;
        Ok(())
    }

    /// Handle Disconnection Complete event
    pub fn handle_disconnection_complete(&mut self, _status: u8, handle: u16) {
        for conn in &mut self.connections {
            if conn.handle == handle
                && (conn.state == ConnectionState::Connected
                    || conn.state == ConnectionState::Disconnecting)
            {
                *conn = HciConnection::empty();
                if self.connection_count > 0 {
                    self.connection_count -= 1;
                }
                break;
            }
        }

        if self.connection_count == 0 && self.state == ControllerState::Connected {
            self.state = ControllerState::Ready;
        }
    }

    /// Find a connection by handle
    pub fn find_connection(&self, handle: u16) -> Option<&HciConnection> {
        self.connections
            .iter()
            .find(|c| c.state == ConnectionState::Connected && c.handle == handle)
    }

    /// Store a link key for a connection (stub)
    pub fn store_link_key(&mut self, addr: &BdAddr, key: &[u8; 16]) {
        for conn in &mut self.connections {
            if conn.state == ConnectionState::Connected && conn.remote_addr == *addr {
                conn.link_key = *key;
                conn.has_link_key = true;
                return;
            }
        }
    }

    // ----- ACL Data -----

    /// Send ACL data to a connected device
    pub fn send_acl(&mut self, handle: u16, data: &[u8]) -> Result<(), KernelError> {
        let conn = self
            .connections
            .iter()
            .find(|c| c.state == ConnectionState::Connected && c.handle == handle);
        if conn.is_none() {
            return Err(KernelError::NotFound {
                resource: "connection",
                id: handle as u64,
            });
        }
        if data.len() > ACL_MAX_DATA_LEN {
            return Err(KernelError::InvalidArgument {
                name: "acl_data",
                value: "exceeds max ACL data length",
            });
        }

        let header = AclHeader {
            handle,
            boundary: AclBoundaryFlag::FirstFlushable,
            broadcast: AclBroadcastFlag::PointToPoint,
            data_len: data.len() as u16,
        };
        self.transport.send_acl_data(&header, data)?;
        self.stats.acl_packets_sent += 1;
        Ok(())
    }

    // ----- SDP Stubs -----

    /// Register a local SDP service record
    pub fn register_sdp_record(&mut self, uuid: u16, channel: u16) -> Result<u32, KernelError> {
        if self.sdp_record_count >= MAX_SDP_RECORDS {
            return Err(KernelError::ResourceExhausted {
                resource: "SDP service records",
            });
        }
        let handle = (self.sdp_record_count as u32) + 0x10000;
        let slot = &mut self.sdp_records[self.sdp_record_count];
        slot.handle = handle;
        slot.service_class_uuid = uuid;
        slot.protocol_channel = channel;
        slot.valid = true;
        self.sdp_record_count += 1;
        Ok(handle)
    }

    /// Search for a service by UUID in local records
    pub fn sdp_search_local(&self, uuid: u16) -> Option<&SdpServiceRecord> {
        self.sdp_records[..self.sdp_record_count]
            .iter()
            .find(|r| sdp_uuid_match(r, uuid))
    }

    // ----- Event Processing -----

    /// Process a received HCI event
    pub fn process_event(&mut self, event: &HciEvent) {
        self.stats.events_received += 1;

        match HciEventCode::from_u8(event.event_code) {
            Some(HciEventCode::CommandComplete) => {
                // Command completed -- status in params[3] if present
                if let Some(opcode) = event.command_complete_opcode() {
                    self.handle_command_complete(opcode, event);
                }
            }
            Some(HciEventCode::CommandStatus) => {
                // Command accepted/rejected -- status in params[0]
            }
            Some(HciEventCode::InquiryComplete) => {
                self.handle_inquiry_complete();
            }
            Some(HciEventCode::InquiryResult) => {
                self.process_inquiry_result(event);
            }
            Some(HciEventCode::ConnectionComplete) => {
                self.process_connection_complete(event);
            }
            Some(HciEventCode::DisconnectionComplete) => {
                self.process_disconnection_complete(event);
            }
            Some(HciEventCode::ExtendedInquiryResult) => {
                self.process_extended_inquiry_result(event);
            }
            _ => {
                // Unknown or unhandled event
            }
        }
    }

    /// Handle Command Complete event internals
    fn handle_command_complete(&mut self, opcode: u16, event: &HciEvent) {
        match opcode {
            HCI_READ_BD_ADDR => {
                // params: [num_cmds, opcode_lo, opcode_hi, status, BD_ADDR[6]]
                if event.param_len >= 10 {
                    let status = event.params[3];
                    if status == 0x00 {
                        self.local_addr.copy_from_slice(&event.params[4..10]);
                    }
                }
            }
            HCI_READ_LOCAL_NAME => {
                // params: [num_cmds, opcode_lo, opcode_hi, status, name[248]]
                if event.param_len >= 5 {
                    let status = event.params[3];
                    if status == 0x00 {
                        let name_start = 4;
                        let name_end = (event.param_len as usize).min(252);
                        let name_slice = &event.params[name_start..name_end];
                        // Find null terminator
                        let len = name_slice
                            .iter()
                            .position(|&b| b == 0)
                            .unwrap_or(name_slice.len());
                        let copy_len = len.min(248);
                        self.local_name[..copy_len].copy_from_slice(&name_slice[..copy_len]);
                        self.local_name_len = copy_len;
                    }
                }
            }
            HCI_RESET => {
                if event.param_len >= 4 && event.params[3] == 0x00 {
                    self.state = ControllerState::Ready;
                }
            }
            _ => {}
        }
    }

    /// Parse Inquiry Result event and store device info
    fn process_inquiry_result(&mut self, event: &HciEvent) {
        if event.param_len < 1 {
            return;
        }
        let num_responses = event.params[0] as usize;
        // Each response: BD_ADDR(6) + page_scan_rep(1) + reserved(2) + CoD(3) +
        // clock(2) = 14
        let mut offset = 1;
        for _ in 0..num_responses {
            if offset + 14 > event.param_len as usize {
                break;
            }
            let mut addr = BD_ADDR_ZERO;
            addr.copy_from_slice(&event.params[offset..offset + 6]);
            let psrm = event.params[offset + 6];
            // skip 2 reserved bytes
            let cod = u32::from_le_bytes([
                event.params[offset + 9],
                event.params[offset + 10],
                event.params[offset + 11],
                0,
            ]);
            let clock = u16::from_le_bytes([event.params[offset + 12], event.params[offset + 13]]);
            self.handle_inquiry_result(addr, psrm, cod, clock);
            offset += 14;
        }
    }

    /// Parse Connection Complete event
    fn process_connection_complete(&mut self, event: &HciEvent) {
        // params: status(1), handle(2), BD_ADDR(6), link_type(1), encryption(1)
        if event.param_len < 11 {
            return;
        }
        let status = event.params[0];
        let handle = u16::from_le_bytes([event.params[1], event.params[2]]);
        let mut addr = BD_ADDR_ZERO;
        addr.copy_from_slice(&event.params[3..9]);
        let link_type = event.params[9];
        let _ = self.handle_connection_complete(status, handle, &addr, link_type);
    }

    /// Parse Disconnection Complete event
    fn process_disconnection_complete(&mut self, event: &HciEvent) {
        // params: status(1), handle(2), reason(1)
        if event.param_len < 4 {
            return;
        }
        let status = event.params[0];
        let handle = u16::from_le_bytes([event.params[1], event.params[2]]);
        self.handle_disconnection_complete(status, handle);
    }

    /// Parse Extended Inquiry Result event
    fn process_extended_inquiry_result(&mut self, event: &HciEvent) {
        // params: num_responses(1) + BD_ADDR(6) + psrm(1) + reserved(1)
        //       + CoD(3) + clock(2) + rssi(1) + EIR(240)
        if event.param_len < 15 {
            return;
        }
        let mut addr = BD_ADDR_ZERO;
        addr.copy_from_slice(&event.params[1..7]);
        let psrm = event.params[7];
        let cod = u32::from_le_bytes([event.params[9], event.params[10], event.params[11], 0]);
        let clock = u16::from_le_bytes([event.params[12], event.params[13]]);
        let rssi = event.params[14] as i8;

        self.handle_inquiry_result(addr, psrm, cod, clock);

        // Try to extract name from EIR data
        if event.param_len > 15 {
            let eir_data = &event.params[15..event.param_len as usize];
            if let Some((name, name_len)) = parse_eir_name(eir_data) {
                // Update the discovered device with the name
                for dev in &mut self.discovered[..self.discovered_count] {
                    if dev.valid && dev.addr == addr {
                        dev.name = name;
                        dev.name_len = name_len;
                        dev.rssi = rssi;
                        break;
                    }
                }
            }
        }
    }

    /// Poll the transport and process any pending events
    pub fn poll(&mut self) -> Result<(), KernelError> {
        if self.state == ControllerState::Off {
            return Ok(());
        }

        // Poll for HCI events
        if let Some(event) = self.transport.poll_event()? {
            self.process_event(&event);
        }

        // Poll for ACL data
        if let Some((_header, _data, _len)) = self.transport.poll_acl_data()? {
            self.stats.acl_packets_received += 1;
        }

        Ok(())
    }

    // ----- Internal Helpers -----

    fn ensure_ready(&self) -> Result<(), KernelError> {
        match self.state {
            ControllerState::Ready | ControllerState::Scanning | ControllerState::Connected => {
                Ok(())
            }
            ControllerState::Off => Err(KernelError::InvalidState {
                expected: "Ready",
                actual: "Off",
            }),
            ControllerState::Initializing => Err(KernelError::InvalidState {
                expected: "Ready",
                actual: "Initializing",
            }),
        }
    }

    fn ensure_ready_or_connected(&self) -> Result<(), KernelError> {
        match self.state {
            ControllerState::Ready | ControllerState::Connected => Ok(()),
            _ => Err(KernelError::InvalidState {
                expected: "Ready or Connected",
                actual: "other",
            }),
        }
    }

    fn next_signal_identifier(&mut self) -> u8 {
        let id = self.next_signal_id;
        self.next_signal_id = self.next_signal_id.wrapping_add(1);
        if self.next_signal_id == 0 {
            self.next_signal_id = 1; // identifier must be non-zero
        }
        id
    }
}

// ---------------------------------------------------------------------------
// Module-Level Init
// ---------------------------------------------------------------------------

/// Initialize the HCI subsystem (called from bluetooth::init)
pub fn init() {
    crate::println!("[BT-HCI] HCI driver initialized");
}

// ---------------------------------------------------------------------------
// Unit Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_make_opcode() {
        // HCI_Reset: OGF=0x03, OCF=0x0003
        let op = make_opcode(0x03, 0x0003);
        assert_eq!(opcode_ogf(op), 0x03);
        assert_eq!(opcode_ocf(op), 0x0003);
        assert_eq!(op, HCI_RESET);
    }

    #[test]
    fn test_opcode_constants() {
        assert_eq!(opcode_ogf(HCI_INQUIRY), 0x01);
        assert_eq!(opcode_ocf(HCI_INQUIRY), 0x0001);
        assert_eq!(opcode_ogf(HCI_READ_BD_ADDR), 0x04);
        assert_eq!(opcode_ocf(HCI_READ_BD_ADDR), 0x0009);
        assert_eq!(opcode_ogf(HCI_WRITE_SCAN_ENABLE), 0x03);
        assert_eq!(opcode_ocf(HCI_WRITE_SCAN_ENABLE), 0x001A);
    }

    #[test]
    fn test_hci_packet_type_roundtrip() {
        assert_eq!(HciPacketType::from_u8(0x01), Some(HciPacketType::Command));
        assert_eq!(HciPacketType::from_u8(0x02), Some(HciPacketType::AclData));
        assert_eq!(HciPacketType::from_u8(0x03), Some(HciPacketType::ScoData));
        assert_eq!(HciPacketType::from_u8(0x04), Some(HciPacketType::Event));
        assert_eq!(HciPacketType::from_u8(0x05), None);
    }

    #[test]
    fn test_hci_command_no_params() {
        let cmd = HciCommand::new(HCI_RESET);
        assert_eq!(cmd.opcode, HCI_RESET);
        assert_eq!(cmd.param_len, 0);
    }

    #[test]
    fn test_hci_command_with_params() {
        let params = [0x03u8]; // inquiry + page scan
        let cmd = HciCommand::with_params(HCI_WRITE_SCAN_ENABLE, &params).unwrap();
        assert_eq!(cmd.opcode, HCI_WRITE_SCAN_ENABLE);
        assert_eq!(cmd.param_len, 1);
        assert_eq!(cmd.params[0], 0x03);
    }

    #[test]
    fn test_hci_command_serialize() {
        let cmd = HciCommand::new(HCI_RESET);
        let mut buf = [0u8; 16];
        let len = cmd.serialize(&mut buf).unwrap();
        assert_eq!(len, 4); // type(1) + header(3) + params(0)
        assert_eq!(buf[0], 0x01); // Command type
        assert_eq!(u16::from_le_bytes([buf[1], buf[2]]), HCI_RESET);
        assert_eq!(buf[3], 0); // param_len
    }

    #[test]
    fn test_hci_event_parse() {
        // Command Complete for HCI_Reset, status 0x00
        let raw = [
            0x0E, // event code: Command Complete
            0x04, // param_len
            0x01, // num_hci_command_packets
            0x03, 0x0C, // opcode (HCI_RESET = 0x0C03 LE)
            0x00, // status success
        ];
        let evt = HciEvent::parse(&raw).unwrap();
        assert_eq!(evt.event_code, 0x0E);
        assert_eq!(evt.param_len, 4);
        assert!(evt.is_command_complete());
        assert!(!evt.is_command_status());
        assert_eq!(evt.command_complete_opcode(), Some(HCI_RESET));
        assert_eq!(evt.command_complete_status(), Some(0x00));
    }

    #[test]
    fn test_acl_header_roundtrip() {
        let header = AclHeader {
            handle: 0x0042,
            boundary: AclBoundaryFlag::FirstFlushable,
            broadcast: AclBroadcastFlag::PointToPoint,
            data_len: 128,
        };
        let mut buf = [0u8; 4];
        header.serialize(&mut buf).unwrap();
        let parsed = AclHeader::parse(&buf).unwrap();
        assert_eq!(parsed.handle, 0x0042);
        assert_eq!(parsed.boundary, AclBoundaryFlag::FirstFlushable);
        assert_eq!(parsed.broadcast, AclBroadcastFlag::PointToPoint);
        assert_eq!(parsed.data_len, 128);
    }

    #[test]
    fn test_l2cap_header_roundtrip() {
        let hdr = L2capHeader {
            length: 48,
            cid: L2CAP_CID_ATT,
        };
        let mut buf = [0u8; 4];
        hdr.serialize(&mut buf).unwrap();
        let parsed = L2capHeader::parse(&buf).unwrap();
        assert_eq!(parsed.length, 48);
        assert_eq!(parsed.cid, L2CAP_CID_ATT);
    }

    #[test]
    fn test_controller_state_machine() {
        let mut ctrl = BluetoothController::new();
        assert_eq!(ctrl.state(), ControllerState::Off);

        // Initialize transitions Off -> Initializing -> Ready
        ctrl.initialize(1).unwrap();
        assert_eq!(ctrl.state(), ControllerState::Ready);

        // Cannot initialize twice
        assert!(ctrl.initialize(2).is_err());
    }

    #[test]
    fn test_inquiry_result_handling() {
        let mut ctrl = BluetoothController::new();
        ctrl.state = ControllerState::Scanning;

        let addr: BdAddr = [0x11, 0x22, 0x33, 0x44, 0x55, 0x66];
        ctrl.handle_inquiry_result(addr, 0x01, 0x001F00, 0x0000);
        assert_eq!(ctrl.discovered_count(), 1);

        let dev = ctrl.get_discovered(0).unwrap();
        assert_eq!(dev.addr, addr);
        assert_eq!(dev.class_of_device, 0x001F00);

        // Duplicate address should not add a second entry
        ctrl.handle_inquiry_result(addr, 0x01, 0x001F00, 0x0000);
        assert_eq!(ctrl.discovered_count(), 1);
    }

    #[test]
    fn test_eir_name_parsing() {
        // EIR data with Complete Local Name
        let eir = [
            0x05, // length=5
            0x09, // type = Complete Local Name
            b'T', b'e', b's', b't', // "Test"
            0x00, // terminator
        ];
        let (name, len) = parse_eir_name(&eir).unwrap();
        assert_eq!(len, 4);
        assert_eq!(&name[..4], b"Test");
    }

    #[test]
    fn test_sdp_record_registration() {
        let mut ctrl = BluetoothController::new();
        let handle = ctrl.register_sdp_record(UUID_SERIAL_PORT, 1).unwrap();
        assert!(handle >= 0x10000);

        let record = ctrl.sdp_search_local(UUID_SERIAL_PORT);
        assert!(record.is_some());
        assert_eq!(record.unwrap().protocol_channel, 1);

        // Non-existent UUID
        assert!(ctrl.sdp_search_local(UUID_HID).is_none());
    }

    #[test]
    fn test_connection_lifecycle() {
        let mut ctrl = BluetoothController::new();
        ctrl.state = ControllerState::Ready;
        ctrl.transport.active = true;

        let addr: BdAddr = [0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF];
        ctrl.create_connection(&addr).unwrap();

        // Simulate Connection Complete (success)
        ctrl.handle_connection_complete(0x00, 0x0040, &addr, 0x01)
            .unwrap();
        assert_eq!(ctrl.connection_count(), 1);
        assert_eq!(ctrl.state(), ControllerState::Connected);
        assert!(ctrl.find_connection(0x0040).is_some());

        // Disconnect
        ctrl.disconnect(0x0040, 0x13).unwrap(); // 0x13 = Remote User Terminated
        ctrl.handle_disconnection_complete(0x00, 0x0040);
        assert_eq!(ctrl.connection_count(), 0);
        assert_eq!(ctrl.state(), ControllerState::Ready);
    }

    #[test]
    fn test_format_bd_addr() {
        let addr: BdAddr = [0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF];
        let formatted = format_bd_addr(&addr);
        assert_eq!(&formatted, b"FF:EE:DD:CC:BB:AA");
    }
}
