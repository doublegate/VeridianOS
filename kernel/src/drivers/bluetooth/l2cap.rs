//! Bluetooth L2CAP (Logical Link Control and Adaptation Protocol)
//!
//! Implements the L2CAP layer for Bluetooth, providing connection-oriented
//! and connectionless data transport between upper-layer protocols and the
//! HCI/baseband layer. Supports segmentation and reassembly (SAR), MTU
//! negotiation, and signaling command processing.
//!
//! Reference: Bluetooth Core Specification v5.4, Volume 3, Part A (L2CAP)

#![allow(dead_code)]

#[cfg(feature = "alloc")]
use alloc::collections::BTreeMap;
#[cfg(feature = "alloc")]
use alloc::vec::Vec;

use crate::error::KernelError;

// ---------------------------------------------------------------------------
// L2CAP Channel IDs
// ---------------------------------------------------------------------------

/// L2CAP Signaling channel (CID 0x0001)
pub const CID_SIGNALING: u16 = 0x0001;

/// L2CAP Connectionless channel (CID 0x0002)
pub const CID_CONNECTIONLESS: u16 = 0x0002;

/// L2CAP ATT fixed channel (CID 0x0004)
pub const CID_ATT: u16 = 0x0004;

/// L2CAP LE Signaling channel (CID 0x0005)
pub const CID_LE_SIGNALING: u16 = 0x0005;

/// L2CAP SMP channel (CID 0x0006)
pub const CID_SMP: u16 = 0x0006;

/// First dynamically allocated CID
pub const CID_DYNAMIC_START: u16 = 0x0040;

/// Last dynamically allocated CID
pub const CID_DYNAMIC_END: u16 = 0xFFFF;

// ---------------------------------------------------------------------------
// Well-known PSM values
// ---------------------------------------------------------------------------

/// SDP PSM
pub const PSM_SDP: u16 = 0x0001;

/// RFCOMM PSM
pub const PSM_RFCOMM: u16 = 0x0003;

/// BNEP (Bluetooth Network Encapsulation Protocol) PSM
pub const PSM_BNEP: u16 = 0x000F;

/// HID Control PSM
pub const PSM_HID_CONTROL: u16 = 0x0011;

/// HID Interrupt PSM
pub const PSM_HID_INTERRUPT: u16 = 0x0013;

/// AVCTP PSM (Audio/Video Control Transport Protocol)
pub const PSM_AVCTP: u16 = 0x0017;

/// AVDTP PSM (Audio/Video Distribution Transport Protocol)
pub const PSM_AVDTP: u16 = 0x0019;

// ---------------------------------------------------------------------------
// L2CAP Signaling Command Codes
// ---------------------------------------------------------------------------

/// L2CAP signaling command codes
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum SignalingCode {
    /// Command Reject
    CommandReject = 0x01,
    /// Connection Request
    ConnectionReq = 0x02,
    /// Connection Response
    ConnectionResp = 0x03,
    /// Configuration Request
    ConfigReq = 0x04,
    /// Configuration Response
    ConfigResp = 0x05,
    /// Disconnection Request
    DisconnReq = 0x06,
    /// Disconnection Response
    DisconnResp = 0x07,
    /// Echo Request
    EchoReq = 0x08,
    /// Echo Response
    EchoResp = 0x09,
    /// Information Request
    InfoReq = 0x0A,
    /// Information Response
    InfoResp = 0x0B,
}

impl SignalingCode {
    /// Parse signaling code from raw byte
    pub fn from_u8(val: u8) -> Option<Self> {
        match val {
            0x01 => Some(Self::CommandReject),
            0x02 => Some(Self::ConnectionReq),
            0x03 => Some(Self::ConnectionResp),
            0x04 => Some(Self::ConfigReq),
            0x05 => Some(Self::ConfigResp),
            0x06 => Some(Self::DisconnReq),
            0x07 => Some(Self::DisconnResp),
            0x08 => Some(Self::EchoReq),
            0x09 => Some(Self::EchoResp),
            0x0A => Some(Self::InfoReq),
            0x0B => Some(Self::InfoResp),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Connection Response Result Codes
// ---------------------------------------------------------------------------

/// L2CAP Connection Response result codes
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum ConnectionResult {
    /// Connection successful
    Success = 0x0000,
    /// Connection pending
    Pending = 0x0001,
    /// Connection refused - PSM not supported
    PsmNotSupported = 0x0002,
    /// Connection refused - security block
    SecurityBlock = 0x0003,
    /// Connection refused - no resources available
    NoResources = 0x0004,
}

/// L2CAP Configuration Response result codes
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum ConfigResult {
    /// Configuration successful
    Success = 0x0000,
    /// Failure - unacceptable parameters
    UnacceptableParams = 0x0001,
    /// Failure - rejected
    Rejected = 0x0002,
    /// Failure - unknown options
    UnknownOptions = 0x0003,
}

// ---------------------------------------------------------------------------
// L2CAP PDU
// ---------------------------------------------------------------------------

/// Maximum L2CAP payload size (before segmentation)
pub const L2CAP_MAX_PAYLOAD: usize = 65535;

/// Default L2CAP MTU
pub const L2CAP_DEFAULT_MTU: u16 = 672;

/// Minimum L2CAP MTU
pub const L2CAP_MIN_MTU: u16 = 48;

/// L2CAP header size (length + CID)
pub const L2CAP_HEADER_SIZE: usize = 4;

/// L2CAP Protocol Data Unit
#[derive(Debug, Clone)]
pub struct L2capPdu {
    /// Payload length (excluding header)
    pub length: u16,
    /// Channel ID
    pub channel_id: u16,
    /// Payload data
    #[cfg(feature = "alloc")]
    pub payload: Vec<u8>,
    #[cfg(not(feature = "alloc"))]
    pub payload: [u8; L2CAP_DEFAULT_MTU as usize],
    #[cfg(not(feature = "alloc"))]
    pub payload_len: usize,
}

impl L2capPdu {
    /// Create a new L2CAP PDU
    #[cfg(feature = "alloc")]
    pub fn new(channel_id: u16, data: &[u8]) -> Self {
        Self {
            length: data.len() as u16,
            channel_id,
            payload: Vec::from(data),
        }
    }

    /// Create a new L2CAP PDU (no-alloc)
    #[cfg(not(feature = "alloc"))]
    pub fn new(channel_id: u16, data: &[u8]) -> Self {
        let copy_len = data.len().min(L2CAP_DEFAULT_MTU as usize);
        let mut payload = [0u8; L2CAP_DEFAULT_MTU as usize];
        payload[..copy_len].copy_from_slice(&data[..copy_len]);
        Self {
            length: copy_len as u16,
            channel_id,
            payload,
            payload_len: copy_len,
        }
    }

    /// Serialize PDU to buffer, returns bytes written
    pub fn serialize(&self, buf: &mut [u8]) -> Result<usize, KernelError> {
        let total = L2CAP_HEADER_SIZE + self.length as usize;
        if buf.len() < total {
            return Err(KernelError::InvalidArgument {
                name: "buffer",
                value: "too small for L2CAP PDU",
            });
        }
        buf[0..2].copy_from_slice(&self.length.to_le_bytes());
        buf[2..4].copy_from_slice(&self.channel_id.to_le_bytes());
        #[cfg(feature = "alloc")]
        {
            buf[4..4 + self.length as usize].copy_from_slice(&self.payload[..self.length as usize]);
        }
        #[cfg(not(feature = "alloc"))]
        {
            buf[4..4 + self.length as usize].copy_from_slice(&self.payload[..self.length as usize]);
        }
        Ok(total)
    }

    /// Parse a PDU from raw buffer
    #[cfg(feature = "alloc")]
    pub fn parse(buf: &[u8]) -> Result<Self, KernelError> {
        if buf.len() < L2CAP_HEADER_SIZE {
            return Err(KernelError::InvalidArgument {
                name: "buffer",
                value: "too short for L2CAP header",
            });
        }
        let length = u16::from_le_bytes([buf[0], buf[1]]);
        let channel_id = u16::from_le_bytes([buf[2], buf[3]]);
        if buf.len() < L2CAP_HEADER_SIZE + length as usize {
            return Err(KernelError::InvalidArgument {
                name: "buffer",
                value: "truncated L2CAP payload",
            });
        }
        let payload = Vec::from(&buf[4..4 + length as usize]);
        Ok(Self {
            length,
            channel_id,
            payload,
        })
    }
}

// ---------------------------------------------------------------------------
// L2CAP Signaling Command
// ---------------------------------------------------------------------------

/// L2CAP signaling command (within the signaling channel)
#[derive(Debug, Clone)]
pub struct SignalingCommand {
    /// Command code
    pub code: u8,
    /// Identifier (for request/response matching)
    pub identifier: u8,
    /// Command data length
    pub data_length: u16,
    /// Command-specific data
    #[cfg(feature = "alloc")]
    pub data: Vec<u8>,
    #[cfg(not(feature = "alloc"))]
    pub data: [u8; 64],
    #[cfg(not(feature = "alloc"))]
    pub data_len: usize,
}

impl SignalingCommand {
    /// Create a new signaling command
    #[cfg(feature = "alloc")]
    pub fn new(code: u8, identifier: u8, data: &[u8]) -> Self {
        Self {
            code,
            identifier,
            data_length: data.len() as u16,
            data: Vec::from(data),
        }
    }

    /// Create a Connection Request command
    #[cfg(feature = "alloc")]
    pub fn connection_request(identifier: u8, psm: u16, source_cid: u16) -> Self {
        let mut data = [0u8; 4];
        data[0..2].copy_from_slice(&psm.to_le_bytes());
        data[2..4].copy_from_slice(&source_cid.to_le_bytes());
        Self::new(SignalingCode::ConnectionReq as u8, identifier, &data)
    }

    /// Create a Connection Response command
    #[cfg(feature = "alloc")]
    pub fn connection_response(
        identifier: u8,
        dest_cid: u16,
        source_cid: u16,
        result: ConnectionResult,
        status: u16,
    ) -> Self {
        let mut data = [0u8; 8];
        data[0..2].copy_from_slice(&dest_cid.to_le_bytes());
        data[2..4].copy_from_slice(&source_cid.to_le_bytes());
        data[4..6].copy_from_slice(&(result as u16).to_le_bytes());
        data[6..8].copy_from_slice(&status.to_le_bytes());
        Self::new(SignalingCode::ConnectionResp as u8, identifier, &data)
    }

    /// Create a Configuration Request command
    #[cfg(feature = "alloc")]
    pub fn config_request(identifier: u8, dest_cid: u16, mtu: u16) -> Self {
        // Flags (2 bytes) + MTU option (type=0x01, len=2, value=mtu)
        let mut data = [0u8; 8];
        data[0..2].copy_from_slice(&dest_cid.to_le_bytes());
        data[2..4].copy_from_slice(&0u16.to_le_bytes()); // flags
        data[4] = 0x01; // MTU option type
        data[5] = 0x02; // MTU option length
        data[6..8].copy_from_slice(&mtu.to_le_bytes());
        Self::new(SignalingCode::ConfigReq as u8, identifier, &data)
    }

    /// Create a Disconnection Request command
    #[cfg(feature = "alloc")]
    pub fn disconnection_request(identifier: u8, dest_cid: u16, source_cid: u16) -> Self {
        let mut data = [0u8; 4];
        data[0..2].copy_from_slice(&dest_cid.to_le_bytes());
        data[2..4].copy_from_slice(&source_cid.to_le_bytes());
        Self::new(SignalingCode::DisconnReq as u8, identifier, &data)
    }

    /// Serialize command to buffer, returns bytes written
    pub fn serialize(&self, buf: &mut [u8]) -> Result<usize, KernelError> {
        let total = 4 + self.data_length as usize;
        if buf.len() < total {
            return Err(KernelError::InvalidArgument {
                name: "buffer",
                value: "too small for signaling command",
            });
        }
        buf[0] = self.code;
        buf[1] = self.identifier;
        buf[2..4].copy_from_slice(&self.data_length.to_le_bytes());
        #[cfg(feature = "alloc")]
        {
            buf[4..4 + self.data_length as usize]
                .copy_from_slice(&self.data[..self.data_length as usize]);
        }
        #[cfg(not(feature = "alloc"))]
        {
            buf[4..4 + self.data_length as usize]
                .copy_from_slice(&self.data[..self.data_length as usize]);
        }
        Ok(total)
    }

    /// Parse a signaling command from raw buffer
    #[cfg(feature = "alloc")]
    pub fn parse(buf: &[u8]) -> Result<Self, KernelError> {
        if buf.len() < 4 {
            return Err(KernelError::InvalidArgument {
                name: "buffer",
                value: "too short for signaling command header",
            });
        }
        let code = buf[0];
        let identifier = buf[1];
        let data_length = u16::from_le_bytes([buf[2], buf[3]]);
        if buf.len() < 4 + data_length as usize {
            return Err(KernelError::InvalidArgument {
                name: "buffer",
                value: "truncated signaling command data",
            });
        }
        let data = Vec::from(&buf[4..4 + data_length as usize]);
        Ok(Self {
            code,
            identifier,
            data_length,
            data,
        })
    }
}

// ---------------------------------------------------------------------------
// L2CAP Channel State Machine
// ---------------------------------------------------------------------------

/// L2CAP channel state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelState {
    /// Channel is closed / not allocated
    Closed,
    /// Waiting for local connect decision
    WaitConnect,
    /// Connection request sent, awaiting response
    WaitConnectRsp,
    /// Configuration phase (both sides exchange config)
    Config,
    /// Channel is open and data can be transferred
    Open,
    /// Disconnection request sent, awaiting response
    WaitDisconnect,
}

// ---------------------------------------------------------------------------
// L2CAP Channel
// ---------------------------------------------------------------------------

/// An L2CAP logical channel
#[derive(Debug, Clone)]
pub struct L2capChannel {
    /// Local Channel ID
    pub local_cid: u16,
    /// Remote Channel ID (assigned by peer)
    pub remote_cid: u16,
    /// Current channel state
    pub state: ChannelState,
    /// Protocol/Service Multiplexer (identifies upper layer)
    pub psm: u16,
    /// Maximum Transmission Unit (negotiated)
    pub mtu: u16,
    /// Remote peer's MTU
    pub remote_mtu: u16,
    /// Flush timeout in milliseconds (0xFFFF = infinite)
    pub flush_timeout: u16,
    /// Whether local configuration is complete
    pub local_config_done: bool,
    /// Whether remote configuration is complete
    pub remote_config_done: bool,
    /// HCI connection handle this channel belongs to
    pub hci_handle: u16,
    /// Reassembly buffer for incoming fragments
    #[cfg(feature = "alloc")]
    pub reassembly_buf: Vec<u8>,
    /// Expected total length for reassembly
    pub reassembly_expected: u16,
}

impl L2capChannel {
    /// Create a new L2CAP channel
    pub fn new(local_cid: u16, psm: u16, hci_handle: u16) -> Self {
        Self {
            local_cid,
            remote_cid: 0,
            state: ChannelState::Closed,
            psm,
            mtu: L2CAP_DEFAULT_MTU,
            remote_mtu: L2CAP_DEFAULT_MTU,
            flush_timeout: 0xFFFF,
            local_config_done: false,
            remote_config_done: false,
            hci_handle,
            #[cfg(feature = "alloc")]
            reassembly_buf: Vec::new(),
            reassembly_expected: 0,
        }
    }

    /// Check if the channel is open for data transfer
    pub fn is_open(&self) -> bool {
        self.state == ChannelState::Open
    }

    /// Check if both sides have completed configuration
    pub fn is_configured(&self) -> bool {
        self.local_config_done && self.remote_config_done
    }
}

// ---------------------------------------------------------------------------
// SAR (Segmentation and Reassembly)
// ---------------------------------------------------------------------------

/// A single SAR fragment
#[derive(Debug, Clone)]
pub struct SarFragment {
    /// Whether this is the first fragment
    pub is_first: bool,
    /// Fragment data
    #[cfg(feature = "alloc")]
    pub data: Vec<u8>,
    #[cfg(not(feature = "alloc"))]
    pub data: [u8; L2CAP_DEFAULT_MTU as usize],
    #[cfg(not(feature = "alloc"))]
    pub data_len: usize,
}

/// Segment data into MTU-sized L2CAP PDU fragments
///
/// The first fragment includes the L2CAP header (length + CID).
/// Subsequent fragments are continuation packets.
#[cfg(feature = "alloc")]
pub fn segment_data(channel_id: u16, data: &[u8], mtu: u16) -> Vec<SarFragment> {
    let mut fragments = Vec::new();
    let effective_mtu = mtu as usize;

    if data.is_empty() {
        // Empty payload: single PDU with zero-length body
        let mut hdr = [0u8; L2CAP_HEADER_SIZE];
        hdr[0..2].copy_from_slice(&0u16.to_le_bytes());
        hdr[2..4].copy_from_slice(&channel_id.to_le_bytes());
        fragments.push(SarFragment {
            is_first: true,
            data: Vec::from(&hdr[..]),
        });
        return fragments;
    }

    // First fragment: L2CAP header + as much data as fits in MTU
    let total_length = data.len() as u16;
    let first_data_len = effective_mtu
        .saturating_sub(L2CAP_HEADER_SIZE)
        .min(data.len());
    let mut first = Vec::with_capacity(L2CAP_HEADER_SIZE + first_data_len);
    first.extend_from_slice(&total_length.to_le_bytes());
    first.extend_from_slice(&channel_id.to_le_bytes());
    first.extend_from_slice(&data[..first_data_len]);
    fragments.push(SarFragment {
        is_first: true,
        data: first,
    });

    // Remaining fragments: continuation packets (no L2CAP header)
    let mut offset = first_data_len;
    while offset < data.len() {
        let chunk_len = effective_mtu.min(data.len() - offset);
        fragments.push(SarFragment {
            is_first: false,
            data: Vec::from(&data[offset..offset + chunk_len]),
        });
        offset += chunk_len;
    }

    fragments
}

/// Reassemble fragments into a complete L2CAP payload
#[cfg(feature = "alloc")]
pub fn reassemble_fragments(fragments: &[SarFragment]) -> Result<Vec<u8>, KernelError> {
    if fragments.is_empty() {
        return Err(KernelError::InvalidArgument {
            name: "fragments",
            value: "empty fragment list",
        });
    }
    if !fragments[0].is_first {
        return Err(KernelError::InvalidArgument {
            name: "fragments",
            value: "first fragment not marked as first",
        });
    }

    // First fragment contains L2CAP header
    let first = &fragments[0].data;
    if first.len() < L2CAP_HEADER_SIZE {
        return Err(KernelError::InvalidArgument {
            name: "first_fragment",
            value: "too short for L2CAP header",
        });
    }
    let total_length = u16::from_le_bytes([first[0], first[1]]) as usize;

    let mut result = Vec::with_capacity(total_length);
    // Data portion of first fragment (after header)
    result.extend_from_slice(&first[L2CAP_HEADER_SIZE..]);

    // Continuation fragments
    for frag in &fragments[1..] {
        result.extend_from_slice(&frag.data);
    }

    if result.len() < total_length {
        return Err(KernelError::InvalidArgument {
            name: "fragments",
            value: "insufficient data for declared length",
        });
    }

    result.truncate(total_length);
    Ok(result)
}

// ---------------------------------------------------------------------------
// L2CAP Manager
// ---------------------------------------------------------------------------

/// L2CAP connection and channel manager
#[cfg(feature = "alloc")]
pub struct L2capManager {
    /// Active channels indexed by local CID
    channels: BTreeMap<u16, L2capChannel>,
    /// Next available dynamic CID
    next_cid: u16,
    /// Next signaling identifier
    next_identifier: u8,
}

#[cfg(feature = "alloc")]
impl Default for L2capManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "alloc")]
impl L2capManager {
    /// Create a new L2CAP manager
    pub fn new() -> Self {
        Self {
            channels: BTreeMap::new(),
            next_cid: CID_DYNAMIC_START,
            next_identifier: 1,
        }
    }

    /// Allocate a new dynamic CID
    fn alloc_cid(&mut self) -> Result<u16, KernelError> {
        let start = self.next_cid;
        loop {
            let cid = self.next_cid;
            self.next_cid = if cid == CID_DYNAMIC_END {
                CID_DYNAMIC_START
            } else {
                cid + 1
            };
            if !self.channels.contains_key(&cid) {
                return Ok(cid);
            }
            // Wrapped around without finding a free CID
            if self.next_cid == start {
                return Err(KernelError::ResourceExhausted {
                    resource: "L2CAP CIDs",
                });
            }
        }
    }

    /// Allocate the next signaling identifier
    fn alloc_identifier(&mut self) -> u8 {
        let id = self.next_identifier;
        self.next_identifier = self.next_identifier.wrapping_add(1);
        if self.next_identifier == 0 {
            self.next_identifier = 1; // 0 is reserved
        }
        id
    }

    /// Get number of active channels
    pub fn channel_count(&self) -> usize {
        self.channels.len()
    }

    /// Get a channel by local CID
    pub fn get_channel(&self, local_cid: u16) -> Option<&L2capChannel> {
        self.channels.get(&local_cid)
    }

    /// Get a mutable reference to a channel by local CID
    pub fn get_channel_mut(&mut self, local_cid: u16) -> Option<&mut L2capChannel> {
        self.channels.get_mut(&local_cid)
    }

    /// Open a new L2CAP channel for the given PSM on the given HCI connection
    ///
    /// Returns (local_cid, SignalingCommand to send to peer)
    pub fn open_channel(
        &mut self,
        psm: u16,
        hci_handle: u16,
    ) -> Result<(u16, SignalingCommand), KernelError> {
        let local_cid = self.alloc_cid()?;
        let mut channel = L2capChannel::new(local_cid, psm, hci_handle);
        channel.state = ChannelState::WaitConnectRsp;

        let identifier = self.alloc_identifier();
        let cmd = SignalingCommand::connection_request(identifier, psm, local_cid);

        self.channels.insert(local_cid, channel);
        Ok((local_cid, cmd))
    }

    /// Close an existing L2CAP channel
    ///
    /// Returns a SignalingCommand (Disconnection Request) to send to peer
    pub fn close_channel(&mut self, local_cid: u16) -> Result<SignalingCommand, KernelError> {
        let channel = self
            .channels
            .get_mut(&local_cid)
            .ok_or(KernelError::InvalidArgument {
                name: "local_cid",
                value: "channel not found",
            })?;

        if channel.state == ChannelState::Closed {
            return Err(KernelError::InvalidState {
                expected: "not Closed",
                actual: "Closed",
            });
        }

        let remote_cid = channel.remote_cid;
        channel.state = ChannelState::WaitDisconnect;
        let identifier = self.alloc_identifier();

        Ok(SignalingCommand::disconnection_request(
            identifier, remote_cid, local_cid,
        ))
    }

    /// Remove a channel entirely (after disconnection completes)
    pub fn remove_channel(&mut self, local_cid: u16) -> Option<L2capChannel> {
        self.channels.remove(&local_cid)
    }

    /// Send data on an open channel, segmenting into PDU fragments if needed
    ///
    /// Returns a list of SAR fragments ready to be sent over HCI ACL
    pub fn send_data(&self, local_cid: u16, data: &[u8]) -> Result<Vec<SarFragment>, KernelError> {
        let channel = self
            .channels
            .get(&local_cid)
            .ok_or(KernelError::InvalidArgument {
                name: "local_cid",
                value: "channel not found",
            })?;

        if channel.state != ChannelState::Open {
            return Err(KernelError::InvalidState {
                expected: "Open",
                actual: "not Open",
            });
        }

        // Segment using the remote peer's MTU
        Ok(segment_data(channel.remote_cid, data, channel.remote_mtu))
    }

    /// Receive and reassemble data for a channel
    ///
    /// Feeds a raw fragment into the reassembly buffer. Returns completed
    /// payload when all fragments have been received.
    pub fn receive_data(
        &mut self,
        local_cid: u16,
        fragment: &[u8],
        is_first: bool,
    ) -> Result<Option<Vec<u8>>, KernelError> {
        let channel = self
            .channels
            .get_mut(&local_cid)
            .ok_or(KernelError::InvalidArgument {
                name: "local_cid",
                value: "channel not found",
            })?;

        if channel.state != ChannelState::Open {
            return Err(KernelError::InvalidState {
                expected: "Open",
                actual: "not Open",
            });
        }

        if is_first {
            // First fragment contains L2CAP header
            if fragment.len() < L2CAP_HEADER_SIZE {
                return Err(KernelError::InvalidArgument {
                    name: "fragment",
                    value: "first fragment too short for L2CAP header",
                });
            }
            let total_length = u16::from_le_bytes([fragment[0], fragment[1]]);
            channel.reassembly_expected = total_length;
            channel.reassembly_buf.clear();
            channel
                .reassembly_buf
                .extend_from_slice(&fragment[L2CAP_HEADER_SIZE..]);
        } else {
            // Continuation fragment
            channel.reassembly_buf.extend_from_slice(fragment);
        }

        // Check if reassembly is complete
        if channel.reassembly_buf.len() >= channel.reassembly_expected as usize {
            let mut result = Vec::new();
            core::mem::swap(&mut result, &mut channel.reassembly_buf);
            result.truncate(channel.reassembly_expected as usize);
            channel.reassembly_expected = 0;
            Ok(Some(result))
        } else {
            Ok(None)
        }
    }

    /// Process an incoming L2CAP signaling command
    ///
    /// Returns an optional response command to send back
    pub fn process_signaling(
        &mut self,
        cmd: &SignalingCommand,
    ) -> Result<Option<SignalingCommand>, KernelError> {
        let code = SignalingCode::from_u8(cmd.code);

        match code {
            Some(SignalingCode::ConnectionReq) => self.handle_connection_request(cmd),
            Some(SignalingCode::ConnectionResp) => self.handle_connection_response(cmd),
            Some(SignalingCode::ConfigReq) => self.handle_config_request(cmd),
            Some(SignalingCode::ConfigResp) => self.handle_config_response(cmd),
            Some(SignalingCode::DisconnReq) => self.handle_disconnection_request(cmd),
            Some(SignalingCode::DisconnResp) => self.handle_disconnection_response(cmd),
            Some(SignalingCode::EchoReq) => {
                // Echo: reflect back with same identifier
                Ok(Some(SignalingCommand::new(
                    SignalingCode::EchoResp as u8,
                    cmd.identifier,
                    &cmd.data,
                )))
            }
            Some(SignalingCode::InfoReq) => {
                // Information Request: respond with "not supported"
                let mut data = [0u8; 4];
                // Copy info type from request
                if cmd.data.len() >= 2 {
                    data[0..2].copy_from_slice(&cmd.data[0..2]);
                }
                data[2..4].copy_from_slice(&0x0001u16.to_le_bytes()); // Not supported
                Ok(Some(SignalingCommand::new(
                    SignalingCode::InfoResp as u8,
                    cmd.identifier,
                    &data,
                )))
            }
            _ => {
                // Unknown or unhandled: Command Reject
                let mut data = [0u8; 2];
                data[0..2].copy_from_slice(&0x0000u16.to_le_bytes()); // Not understood
                Ok(Some(SignalingCommand::new(
                    SignalingCode::CommandReject as u8,
                    cmd.identifier,
                    &data,
                )))
            }
        }
    }

    /// Handle incoming Connection Request
    fn handle_connection_request(
        &mut self,
        cmd: &SignalingCommand,
    ) -> Result<Option<SignalingCommand>, KernelError> {
        if cmd.data.len() < 4 {
            return Err(KernelError::InvalidArgument {
                name: "connection_request",
                value: "data too short",
            });
        }
        let psm = u16::from_le_bytes([cmd.data[0], cmd.data[1]]);
        let source_cid = u16::from_le_bytes([cmd.data[2], cmd.data[3]]);

        // Allocate a local CID for this incoming connection
        match self.alloc_cid() {
            Ok(local_cid) => {
                let mut channel = L2capChannel::new(local_cid, psm, 0);
                channel.remote_cid = source_cid;
                channel.state = ChannelState::Config;
                self.channels.insert(local_cid, channel);

                Ok(Some(SignalingCommand::connection_response(
                    cmd.identifier,
                    local_cid,
                    source_cid,
                    ConnectionResult::Success,
                    0,
                )))
            }
            Err(_) => Ok(Some(SignalingCommand::connection_response(
                cmd.identifier,
                0,
                source_cid,
                ConnectionResult::NoResources,
                0,
            ))),
        }
    }

    /// Handle incoming Connection Response
    fn handle_connection_response(
        &mut self,
        cmd: &SignalingCommand,
    ) -> Result<Option<SignalingCommand>, KernelError> {
        if cmd.data.len() < 8 {
            return Err(KernelError::InvalidArgument {
                name: "connection_response",
                value: "data too short",
            });
        }
        let dest_cid = u16::from_le_bytes([cmd.data[0], cmd.data[1]]);
        let source_cid = u16::from_le_bytes([cmd.data[2], cmd.data[3]]);
        let result = u16::from_le_bytes([cmd.data[4], cmd.data[5]]);

        // Find the channel by source_cid (our local CID)
        let channel_mtu = if let Some(channel) = self.channels.get_mut(&source_cid) {
            if result == ConnectionResult::Success as u16 {
                channel.remote_cid = dest_cid;
                channel.state = ChannelState::Config;
                Some(channel.mtu)
            } else {
                channel.state = ChannelState::Closed;
                None
            }
        } else {
            None
        };

        if let Some(mtu) = channel_mtu {
            let id = self.alloc_identifier();
            return Ok(Some(SignalingCommand::config_request(id, dest_cid, mtu)));
        }
        Ok(None)
    }

    /// Handle incoming Configuration Request
    fn handle_config_request(
        &mut self,
        cmd: &SignalingCommand,
    ) -> Result<Option<SignalingCommand>, KernelError> {
        if cmd.data.len() < 4 {
            return Err(KernelError::InvalidArgument {
                name: "config_request",
                value: "data too short",
            });
        }
        let dest_cid = u16::from_le_bytes([cmd.data[0], cmd.data[1]]);
        // flags at [2..4]

        // Parse MTU option if present
        let mut remote_mtu = L2CAP_DEFAULT_MTU;
        if cmd.data.len() >= 8 && cmd.data[4] == 0x01 && cmd.data[5] == 0x02 {
            remote_mtu = u16::from_le_bytes([cmd.data[6], cmd.data[7]]);
            if remote_mtu < L2CAP_MIN_MTU {
                remote_mtu = L2CAP_MIN_MTU;
            }
        }

        if let Some(channel) = self.channels.get_mut(&dest_cid) {
            channel.remote_mtu = remote_mtu;
            channel.remote_config_done = true;

            // Transition to Open if both sides configured
            if channel.local_config_done && channel.remote_config_done {
                channel.state = ChannelState::Open;
            }

            // Config Response: success
            let mut resp_data = [0u8; 6];
            resp_data[0..2].copy_from_slice(&channel.remote_cid.to_le_bytes());
            resp_data[2..4].copy_from_slice(&0u16.to_le_bytes()); // flags
            resp_data[4..6].copy_from_slice(&(ConfigResult::Success as u16).to_le_bytes());
            Ok(Some(SignalingCommand::new(
                SignalingCode::ConfigResp as u8,
                cmd.identifier,
                &resp_data,
            )))
        } else {
            Ok(None)
        }
    }

    /// Handle incoming Configuration Response
    fn handle_config_response(
        &mut self,
        cmd: &SignalingCommand,
    ) -> Result<Option<SignalingCommand>, KernelError> {
        if cmd.data.len() < 6 {
            return Err(KernelError::InvalidArgument {
                name: "config_response",
                value: "data too short",
            });
        }
        let source_cid = u16::from_le_bytes([cmd.data[0], cmd.data[1]]);
        // flags at [2..4]
        let result = u16::from_le_bytes([cmd.data[4], cmd.data[5]]);

        // Find channel by remote_cid matching source_cid
        for channel in self.channels.values_mut() {
            if channel.remote_cid == source_cid {
                if result == ConfigResult::Success as u16 {
                    channel.local_config_done = true;
                    if channel.local_config_done && channel.remote_config_done {
                        channel.state = ChannelState::Open;
                    }
                }
                break;
            }
        }
        Ok(None)
    }

    /// Handle incoming Disconnection Request
    fn handle_disconnection_request(
        &mut self,
        cmd: &SignalingCommand,
    ) -> Result<Option<SignalingCommand>, KernelError> {
        if cmd.data.len() < 4 {
            return Err(KernelError::InvalidArgument {
                name: "disconnection_request",
                value: "data too short",
            });
        }
        let dest_cid = u16::from_le_bytes([cmd.data[0], cmd.data[1]]);
        let source_cid = u16::from_le_bytes([cmd.data[2], cmd.data[3]]);

        if let Some(channel) = self.channels.get_mut(&dest_cid) {
            channel.state = ChannelState::Closed;
        }

        // Respond with Disconnection Response
        let mut resp_data = [0u8; 4];
        resp_data[0..2].copy_from_slice(&dest_cid.to_le_bytes());
        resp_data[2..4].copy_from_slice(&source_cid.to_le_bytes());
        Ok(Some(SignalingCommand::new(
            SignalingCode::DisconnResp as u8,
            cmd.identifier,
            &resp_data,
        )))
    }

    /// Handle incoming Disconnection Response
    fn handle_disconnection_response(
        &mut self,
        cmd: &SignalingCommand,
    ) -> Result<Option<SignalingCommand>, KernelError> {
        if cmd.data.len() < 4 {
            return Err(KernelError::InvalidArgument {
                name: "disconnection_response",
                value: "data too short",
            });
        }
        let dest_cid = u16::from_le_bytes([cmd.data[0], cmd.data[1]]);

        if let Some(channel) = self.channels.get_mut(&dest_cid) {
            channel.state = ChannelState::Closed;
        }
        Ok(None)
    }

    /// Configure a channel's MTU and flush timeout
    pub fn configure_channel(
        &mut self,
        local_cid: u16,
        mtu: u16,
        flush_timeout: u16,
    ) -> Result<(), KernelError> {
        let channel = self
            .channels
            .get_mut(&local_cid)
            .ok_or(KernelError::InvalidArgument {
                name: "local_cid",
                value: "channel not found",
            })?;

        if mtu < L2CAP_MIN_MTU {
            return Err(KernelError::InvalidArgument {
                name: "mtu",
                value: "below minimum L2CAP MTU (48)",
            });
        }

        channel.mtu = mtu;
        channel.flush_timeout = flush_timeout;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #[cfg(feature = "alloc")]
    #[allow(unused_imports)]
    use alloc::vec;

    use super::*;

    #[test]
    fn test_signaling_code_roundtrip() {
        assert_eq!(
            SignalingCode::from_u8(0x02),
            Some(SignalingCode::ConnectionReq)
        );
        assert_eq!(
            SignalingCode::from_u8(0x03),
            Some(SignalingCode::ConnectionResp)
        );
        assert_eq!(SignalingCode::from_u8(0x04), Some(SignalingCode::ConfigReq));
        assert_eq!(
            SignalingCode::from_u8(0x05),
            Some(SignalingCode::ConfigResp)
        );
        assert_eq!(
            SignalingCode::from_u8(0x06),
            Some(SignalingCode::DisconnReq)
        );
        assert_eq!(
            SignalingCode::from_u8(0x07),
            Some(SignalingCode::DisconnResp)
        );
        assert_eq!(SignalingCode::from_u8(0x0A), Some(SignalingCode::InfoReq));
        assert_eq!(SignalingCode::from_u8(0x0B), Some(SignalingCode::InfoResp));
        assert_eq!(SignalingCode::from_u8(0xFF), None);
    }

    #[test]
    fn test_channel_state_defaults() {
        let ch = L2capChannel::new(0x0040, PSM_RFCOMM, 1);
        assert_eq!(ch.state, ChannelState::Closed);
        assert_eq!(ch.mtu, L2CAP_DEFAULT_MTU);
        assert_eq!(ch.flush_timeout, 0xFFFF);
        assert!(!ch.is_open());
        assert!(!ch.is_configured());
    }

    #[test]
    fn test_pdu_serialize_parse() {
        let data = [0x01, 0x02, 0x03, 0x04];
        let pdu = L2capPdu::new(0x0040, &data);
        assert_eq!(pdu.length, 4);
        assert_eq!(pdu.channel_id, 0x0040);

        let mut buf = [0u8; 32];
        let written = pdu.serialize(&mut buf).unwrap();
        assert_eq!(written, 8); // 4 header + 4 data
        assert_eq!(u16::from_le_bytes([buf[0], buf[1]]), 4);
        assert_eq!(u16::from_le_bytes([buf[2], buf[3]]), 0x0040);
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_pdu_parse_roundtrip() {
        let data = [0xAA, 0xBB, 0xCC];
        let pdu = L2capPdu::new(0x0041, &data);
        let mut buf = [0u8; 32];
        let written = pdu.serialize(&mut buf).unwrap();
        let parsed = L2capPdu::parse(&buf[..written]).unwrap();
        assert_eq!(parsed.length, 3);
        assert_eq!(parsed.channel_id, 0x0041);
        assert_eq!(parsed.payload, data);
    }

    #[test]
    fn test_pdu_serialize_buffer_too_small() {
        let data = [0u8; 10];
        let pdu = L2capPdu::new(0x0040, &data);
        let mut buf = [0u8; 8]; // needs 14
        assert!(pdu.serialize(&mut buf).is_err());
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_signaling_command_serialize_parse() {
        let cmd = SignalingCommand::connection_request(1, PSM_RFCOMM, 0x0040);
        let mut buf = [0u8; 32];
        let written = cmd.serialize(&mut buf).unwrap();
        assert_eq!(written, 8); // 4 header + 4 data

        let parsed = SignalingCommand::parse(&buf[..written]).unwrap();
        assert_eq!(parsed.code, SignalingCode::ConnectionReq as u8);
        assert_eq!(parsed.identifier, 1);
        assert_eq!(parsed.data_length, 4);
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_segment_small_data() {
        let data = [0x01, 0x02, 0x03];
        let fragments = segment_data(0x0040, &data, 672);
        assert_eq!(fragments.len(), 1);
        assert!(fragments[0].is_first);
        // First fragment: 4 header + 3 data
        assert_eq!(fragments[0].data.len(), 7);
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_segment_large_data() {
        // Data larger than MTU should produce multiple fragments
        let mtu = 10u16;
        let data = [0xAA; 20];
        let fragments = segment_data(0x0040, &data, mtu);
        assert!(fragments.len() > 1);
        assert!(fragments[0].is_first);
        for frag in &fragments[1..] {
            assert!(!frag.is_first);
        }
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_segment_empty_data() {
        let fragments = segment_data(0x0040, &[], 672);
        assert_eq!(fragments.len(), 1);
        assert!(fragments[0].is_first);
        assert_eq!(fragments[0].data.len(), 4); // header only
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_reassemble_single_fragment() {
        let data = [0x01, 0x02, 0x03];
        let fragments = segment_data(0x0040, &data, 672);
        let result = reassemble_fragments(&fragments).unwrap();
        assert_eq!(result, data);
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_reassemble_multiple_fragments() {
        let data = [0xAA; 20];
        let fragments = segment_data(0x0040, &data, 10);
        let result = reassemble_fragments(&fragments).unwrap();
        assert_eq!(result, data);
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_manager_open_close_channel() {
        let mut mgr = L2capManager::new();
        let (cid, cmd) = mgr.open_channel(PSM_RFCOMM, 1).unwrap();
        assert!(cid >= CID_DYNAMIC_START);
        assert_eq!(cmd.code, SignalingCode::ConnectionReq as u8);
        assert_eq!(mgr.channel_count(), 1);

        let channel = mgr.get_channel(cid).unwrap();
        assert_eq!(channel.state, ChannelState::WaitConnectRsp);
        assert_eq!(channel.psm, PSM_RFCOMM);

        // Close
        let disc_cmd = mgr.close_channel(cid).unwrap();
        assert_eq!(disc_cmd.code, SignalingCode::DisconnReq as u8);
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_manager_configure_channel() {
        let mut mgr = L2capManager::new();
        let (cid, _) = mgr.open_channel(PSM_SDP, 1).unwrap();

        mgr.configure_channel(cid, 1024, 5000).unwrap();
        let ch = mgr.get_channel(cid).unwrap();
        assert_eq!(ch.mtu, 1024);
        assert_eq!(ch.flush_timeout, 5000);
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_manager_configure_mtu_too_small() {
        let mut mgr = L2capManager::new();
        let (cid, _) = mgr.open_channel(PSM_SDP, 1).unwrap();
        assert!(mgr.configure_channel(cid, 10, 0xFFFF).is_err());
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_manager_process_echo_request() {
        let mut mgr = L2capManager::new();
        let echo = SignalingCommand::new(SignalingCode::EchoReq as u8, 5, &[0x01, 0x02]);
        let resp = mgr.process_signaling(&echo).unwrap().unwrap();
        assert_eq!(resp.code, SignalingCode::EchoResp as u8);
        assert_eq!(resp.identifier, 5);
        assert_eq!(resp.data, echo.data);
    }
}
