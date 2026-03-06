//! iSCSI Initiator (RFC 7143)
//!
//! Implements an iSCSI initiator with login/logout session management,
//! SCSI command transport over TCP, PDU serialization, and text-mode
//! parameter negotiation.

#![allow(dead_code)]

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::{collections::BTreeMap, format, string::String, vec::Vec};

// ---------------------------------------------------------------------------
// iSCSI Protocol Constants
// ---------------------------------------------------------------------------

/// iSCSI TCP port (RFC 7143).
const ISCSI_PORT: u16 = 3260;

/// BHS (Basic Header Segment) length in bytes.
const BHS_LENGTH: usize = 48;

/// Maximum data segment length (default).
const DEFAULT_MAX_RECV_DATA_SEGMENT_LENGTH: u32 = 8192;

/// Default max burst length.
const DEFAULT_MAX_BURST_LENGTH: u32 = 262144;

/// Default first burst length.
const DEFAULT_FIRST_BURST_LENGTH: u32 = 65536;

// ---------------------------------------------------------------------------
// iSCSI Opcodes
// ---------------------------------------------------------------------------

/// iSCSI PDU opcodes (RFC 7143 Section 11.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum IscsiOpcode {
    NopOut = 0x00,
    ScsiCommand = 0x01,
    LoginReq = 0x03,
    TextReq = 0x04,
    DataOut = 0x05,
    Logout = 0x06,
    NopIn = 0x20,
    ScsiResponse = 0x21,
    LoginResp = 0x23,
    TextResp = 0x24,
    DataIn = 0x25,
    LogoutResp = 0x26,
    Reject = 0x3F,
}

impl IscsiOpcode {
    /// Convert from wire value (lower 6 bits).
    pub fn from_u8(v: u8) -> Option<Self> {
        match v & 0x3F {
            0x00 => Some(Self::NopOut),
            0x01 => Some(Self::ScsiCommand),
            0x03 => Some(Self::LoginReq),
            0x04 => Some(Self::TextReq),
            0x05 => Some(Self::DataOut),
            0x06 => Some(Self::Logout),
            0x20 => Some(Self::NopIn),
            0x21 => Some(Self::ScsiResponse),
            0x23 => Some(Self::LoginResp),
            0x24 => Some(Self::TextResp),
            0x25 => Some(Self::DataIn),
            0x26 => Some(Self::LogoutResp),
            0x3F => Some(Self::Reject),
            _ => None,
        }
    }

    /// Whether this is an initiator opcode (bit 5 clear).
    pub fn is_initiator(&self) -> bool {
        (*self as u8) & 0x20 == 0
    }
}

// ---------------------------------------------------------------------------
// SCSI Command Definitions
// ---------------------------------------------------------------------------

/// Common SCSI operation codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ScsiOpcode {
    TestUnitReady = 0x00,
    RequestSense = 0x03,
    Inquiry = 0x12,
    ReadCapacity10 = 0x25,
    Read10 = 0x28,
    Write10 = 0x2A,
}

/// SCSI Command Descriptor Block (CDB) with metadata.
#[derive(Debug, Clone)]
pub struct ScsiCommand {
    /// Command descriptor block (16 bytes for CDB16 compatibility).
    pub cdb: [u8; 16],
    /// Expected data transfer length.
    pub data_length: u32,
    /// Logical unit number.
    pub lun: u64,
    /// Whether this is a read command.
    pub is_read: bool,
    /// Whether this is a write command.
    pub is_write: bool,
}

impl ScsiCommand {
    /// Create TEST UNIT READY command.
    pub fn test_unit_ready(lun: u64) -> Self {
        let mut cdb = [0u8; 16];
        cdb[0] = ScsiOpcode::TestUnitReady as u8;
        Self {
            cdb,
            data_length: 0,
            lun,
            is_read: false,
            is_write: false,
        }
    }

    /// Create INQUIRY command.
    pub fn inquiry(lun: u64) -> Self {
        let mut cdb = [0u8; 16];
        cdb[0] = ScsiOpcode::Inquiry as u8;
        cdb[4] = 96; // Allocation length
        Self {
            cdb,
            data_length: 96,
            lun,
            is_read: true,
            is_write: false,
        }
    }

    /// Create READ CAPACITY (10) command.
    pub fn read_capacity_10(lun: u64) -> Self {
        let mut cdb = [0u8; 16];
        cdb[0] = ScsiOpcode::ReadCapacity10 as u8;
        Self {
            cdb,
            data_length: 8,
            lun,
            is_read: true,
            is_write: false,
        }
    }

    /// Create READ (10) command.
    pub fn read_10(lun: u64, lba: u32, block_count: u16, block_size: u32) -> Self {
        let mut cdb = [0u8; 16];
        cdb[0] = ScsiOpcode::Read10 as u8;
        cdb[2..6].copy_from_slice(&lba.to_be_bytes());
        cdb[7..9].copy_from_slice(&block_count.to_be_bytes());
        Self {
            cdb,
            data_length: block_count as u32 * block_size,
            lun,
            is_read: true,
            is_write: false,
        }
    }

    /// Create WRITE (10) command.
    pub fn write_10(lun: u64, lba: u32, block_count: u16, block_size: u32) -> Self {
        let mut cdb = [0u8; 16];
        cdb[0] = ScsiOpcode::Write10 as u8;
        cdb[2..6].copy_from_slice(&lba.to_be_bytes());
        cdb[7..9].copy_from_slice(&block_count.to_be_bytes());
        Self {
            cdb,
            data_length: block_count as u32 * block_size,
            lun,
            is_read: false,
            is_write: true,
        }
    }

    /// Create REQUEST SENSE command.
    pub fn request_sense(lun: u64) -> Self {
        let mut cdb = [0u8; 16];
        cdb[0] = ScsiOpcode::RequestSense as u8;
        cdb[4] = 252; // Allocation length
        Self {
            cdb,
            data_length: 252,
            lun,
            is_read: true,
            is_write: false,
        }
    }
}

// ---------------------------------------------------------------------------
// BHS (Basic Header Segment)
// ---------------------------------------------------------------------------

/// iSCSI Basic Header Segment (48 bytes, RFC 7143 Section 11.2).
#[derive(Debug, Clone)]
pub struct BhsHeader {
    /// Opcode (lower 6 bits) + flags (upper 2 bits).
    pub opcode: IscsiOpcode,
    /// Immediate delivery flag.
    pub immediate: bool,
    /// Final PDU flag.
    pub is_final: bool,
    /// Opcode-specific flags byte.
    pub flags: u8,
    /// Total AHS (Additional Header Segments) length in 4-byte words.
    pub total_ahs_length: u8,
    /// Data segment length (24-bit).
    pub data_segment_length: u32,
    /// Logical Unit Number.
    pub lun: u64,
    /// Initiator Task Tag.
    pub initiator_task_tag: u32,
    /// Opcode-specific fields (bytes 20-47, 28 bytes).
    pub specific: [u8; 28],
}

impl Default for BhsHeader {
    fn default() -> Self {
        Self {
            opcode: IscsiOpcode::NopOut,
            immediate: false,
            is_final: true,
            flags: 0,
            total_ahs_length: 0,
            data_segment_length: 0,
            lun: 0,
            initiator_task_tag: 0,
            specific: [0u8; 28],
        }
    }
}

impl BhsHeader {
    /// Create a new BHS for a given opcode.
    pub fn new(opcode: IscsiOpcode) -> Self {
        Self {
            opcode,
            is_final: true,
            ..Default::default()
        }
    }

    /// Serialize BHS to 48 bytes.
    pub fn serialize(&self) -> [u8; BHS_LENGTH] {
        let mut buf = [0u8; BHS_LENGTH];

        // Byte 0: immediate(1) + opcode(6)
        buf[0] = (self.opcode as u8) & 0x3F;
        if self.immediate {
            buf[0] |= 0x40;
        }

        // Byte 1: flags + final
        buf[1] = self.flags;
        if self.is_final {
            buf[1] |= 0x80;
        }

        // Byte 4: TotalAHSLength
        buf[4] = self.total_ahs_length;

        // Bytes 5-7: DataSegmentLength (24-bit big-endian)
        buf[5] = ((self.data_segment_length >> 16) & 0xFF) as u8;
        buf[6] = ((self.data_segment_length >> 8) & 0xFF) as u8;
        buf[7] = (self.data_segment_length & 0xFF) as u8;

        // Bytes 8-15: LUN
        buf[8..16].copy_from_slice(&self.lun.to_be_bytes());

        // Bytes 16-19: Initiator Task Tag
        buf[16..20].copy_from_slice(&self.initiator_task_tag.to_be_bytes());

        // Bytes 20-47: Opcode-specific
        buf[20..48].copy_from_slice(&self.specific);

        buf
    }

    /// Deserialize BHS from 48 bytes.
    pub fn deserialize(data: &[u8]) -> Option<Self> {
        if data.len() < BHS_LENGTH {
            return None;
        }

        let opcode = IscsiOpcode::from_u8(data[0] & 0x3F)?;
        let immediate = data[0] & 0x40 != 0;
        let is_final = data[1] & 0x80 != 0;
        let flags = data[1] & 0x7F;
        let total_ahs_length = data[4];
        let data_segment_length =
            ((data[5] as u32) << 16) | ((data[6] as u32) << 8) | (data[7] as u32);

        let lun = u64::from_be_bytes([
            data[8], data[9], data[10], data[11], data[12], data[13], data[14], data[15],
        ]);
        let initiator_task_tag = u32::from_be_bytes([data[16], data[17], data[18], data[19]]);

        let mut specific = [0u8; 28];
        specific.copy_from_slice(&data[20..48]);

        Some(Self {
            opcode,
            immediate,
            is_final,
            flags,
            total_ahs_length,
            data_segment_length,
            lun,
            initiator_task_tag,
            specific,
        })
    }

    /// Get data segment length including padding to 4-byte boundary.
    pub fn padded_data_length(&self) -> usize {
        let len = self.data_segment_length as usize;
        (len + 3) & !3
    }
}

// ---------------------------------------------------------------------------
// iSCSI Session State
// ---------------------------------------------------------------------------

/// iSCSI session states (RFC 7143 Section 8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    /// Not connected.
    Free,
    /// Login phase in progress.
    Login,
    /// Full Feature Phase (normal operation).
    FullFeature,
    /// Logout in progress.
    Logout,
}

/// iSCSI session.
#[cfg(feature = "alloc")]
#[derive(Debug)]
pub struct IscsiSession {
    /// Target name (IQN).
    pub target_name: String,
    /// Initiator name (IQN).
    pub initiator_name: String,
    /// Initiator Session ID (48-bit).
    pub isid: u64,
    /// Target Session Identifying Handle.
    pub tsih: u16,
    /// Command Sequence Number.
    pub cmd_sn: u32,
    /// Expected Status Sequence Number.
    pub exp_stat_sn: u32,
    /// Session state.
    pub state: SessionState,
    /// Negotiated MaxRecvDataSegmentLength.
    pub max_recv_data_segment_length: u32,
    /// Negotiated MaxBurstLength.
    pub max_burst_length: u32,
    /// Negotiated FirstBurstLength.
    pub first_burst_length: u32,
}

#[cfg(feature = "alloc")]
impl IscsiSession {
    /// Create a new session.
    pub fn new(initiator_name: &str, target_name: &str) -> Self {
        Self {
            target_name: String::from(target_name),
            initiator_name: String::from(initiator_name),
            isid: 0x0000_23D0_0000_0001, // Default ISID (EN format)
            tsih: 0,
            cmd_sn: 1,
            exp_stat_sn: 0,
            state: SessionState::Free,
            max_recv_data_segment_length: DEFAULT_MAX_RECV_DATA_SEGMENT_LENGTH,
            max_burst_length: DEFAULT_MAX_BURST_LENGTH,
            first_burst_length: DEFAULT_FIRST_BURST_LENGTH,
        }
    }

    /// Get next command sequence number and increment.
    pub fn next_cmd_sn(&mut self) -> u32 {
        let sn = self.cmd_sn;
        self.cmd_sn = self.cmd_sn.wrapping_add(1);
        sn
    }
}

// ---------------------------------------------------------------------------
// iSCSI Error
// ---------------------------------------------------------------------------

/// iSCSI error type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IscsiError {
    /// Not logged in.
    NotLoggedIn,
    /// Login failed.
    LoginFailed,
    /// Session not found.
    SessionNotFound,
    /// PDU parse error.
    PduError,
    /// SCSI command failed with check condition.
    ScsiError,
    /// Transport (network) error.
    TransportError,
    /// Target rejected the request.
    TargetError,
    /// Invalid parameter.
    InvalidParameter,
    /// Timeout.
    Timeout,
}

// ---------------------------------------------------------------------------
// SCSI Response
// ---------------------------------------------------------------------------

/// SCSI command response.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct ScsiResponse {
    /// SCSI status code.
    pub status: u8,
    /// Response data (for read commands).
    pub data: Vec<u8>,
    /// Sense data (if check condition).
    pub sense: Vec<u8>,
}

/// Device information from INQUIRY response.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct InquiryData {
    /// Peripheral device type (bits 4:0 of byte 0).
    pub device_type: u8,
    /// Vendor identification (bytes 8-15).
    pub vendor: String,
    /// Product identification (bytes 16-31).
    pub product: String,
    /// Product revision (bytes 32-35).
    pub revision: String,
}

/// Disk capacity from READ CAPACITY (10) response.
#[derive(Debug, Clone, Copy)]
pub struct DiskCapacity {
    /// Last logical block address.
    pub last_lba: u32,
    /// Block size in bytes.
    pub block_size: u32,
    /// Total capacity in bytes.
    pub total_bytes: u64,
}

// ---------------------------------------------------------------------------
// iSCSI Initiator
// ---------------------------------------------------------------------------

/// iSCSI initiator.
#[cfg(feature = "alloc")]
pub struct IscsiInitiator {
    /// Active sessions.
    sessions: Vec<IscsiSession>,
    /// Target portal address (IP:port).
    target_portal: String,
    /// Next initiator task tag.
    next_tag: u32,
}

#[cfg(feature = "alloc")]
impl IscsiInitiator {
    /// Create a new iSCSI initiator.
    pub fn new(portal: &str) -> Self {
        Self {
            sessions: Vec::new(),
            target_portal: String::from(portal),
            next_tag: 1,
        }
    }

    /// Perform login to a target.
    pub fn login(&mut self, initiator_name: &str, target_name: &str) -> Result<usize, IscsiError> {
        let mut session = IscsiSession::new(initiator_name, target_name);
        session.state = SessionState::Login;

        // Build Login Request PDU
        let mut bhs = BhsHeader::new(IscsiOpcode::LoginReq);
        bhs.immediate = true;
        bhs.flags = 0x07; // Transit + CSG=0 (SecurityNegotiation) + NSG=3 (FullFeature)
        bhs.initiator_task_tag = self.next_tag();

        // ISID in specific bytes 0-5
        let isid_bytes = session.isid.to_be_bytes();
        bhs.specific[0..6].copy_from_slice(&isid_bytes[2..8]);

        // CmdSN in specific bytes 8-11
        let cmd_sn = session.next_cmd_sn();
        bhs.specific[8..12].copy_from_slice(&cmd_sn.to_be_bytes());

        // Build login parameters
        let params = self.build_login_params(&session);
        bhs.data_segment_length = params.len() as u32;

        let _pdu = self.build_pdu(&bhs, &params);

        // In production: send PDU, receive login response, check status.
        // For security negotiation: may need multiple round-trips.
        // Stub: mark as logged in.
        session.state = SessionState::FullFeature;
        session.tsih = 1;

        let idx = self.sessions.len();
        self.sessions.push(session);
        Ok(idx)
    }

    /// Logout from a session.
    pub fn logout(&mut self, session_idx: usize) -> Result<(), IscsiError> {
        let tag = self.next_tag();

        let (cmd_sn, exp_stat_sn) = {
            let session = self
                .sessions
                .get_mut(session_idx)
                .ok_or(IscsiError::SessionNotFound)?;

            if session.state != SessionState::FullFeature {
                return Err(IscsiError::NotLoggedIn);
            }

            let sn = (session.next_cmd_sn(), session.exp_stat_sn);
            session.state = SessionState::Free;
            sn
        };

        let mut bhs = BhsHeader::new(IscsiOpcode::Logout);
        bhs.immediate = true;
        bhs.flags = 0x00; // Close session
        bhs.initiator_task_tag = tag;
        bhs.specific[0..4].copy_from_slice(&cmd_sn.to_be_bytes());
        bhs.specific[4..8].copy_from_slice(&exp_stat_sn.to_be_bytes());

        let _pdu = self.build_pdu(&bhs, &[]);
        Ok(())
    }

    /// Send a SCSI command and receive response.
    pub fn scsi_command(
        &mut self,
        session_idx: usize,
        cmd: &ScsiCommand,
        write_data: Option<&[u8]>,
    ) -> Result<ScsiResponse, IscsiError> {
        let tag = self.next_tag();

        // Extract session fields needed for BHS construction, then release borrow
        let (cmd_sn, exp_stat_sn) = {
            let session = self
                .sessions
                .get_mut(session_idx)
                .ok_or(IscsiError::SessionNotFound)?;

            if session.state != SessionState::FullFeature {
                return Err(IscsiError::NotLoggedIn);
            }

            (session.next_cmd_sn(), session.exp_stat_sn)
        };

        let mut bhs = BhsHeader::new(IscsiOpcode::ScsiCommand);
        bhs.is_final = true;
        bhs.lun = cmd.lun;
        bhs.initiator_task_tag = tag;

        // Flags: read/write bits
        let mut flags: u8 = 0;
        if cmd.is_read {
            flags |= 0x40;
        }
        if cmd.is_write {
            flags |= 0x20;
        }
        // ATTR = Simple (bits 2:0 = 0)
        bhs.flags = flags;

        // Expected Data Transfer Length in specific[0..4]
        bhs.specific[0..4].copy_from_slice(&cmd.data_length.to_be_bytes());

        // CmdSN in specific[4..8]
        bhs.specific[4..8].copy_from_slice(&cmd_sn.to_be_bytes());

        // ExpStatSN in specific[8..12]
        bhs.specific[8..12].copy_from_slice(&exp_stat_sn.to_be_bytes());

        // CDB in specific[12..28] (first 16 bytes of CDB)
        bhs.specific[12..28].copy_from_slice(&cmd.cdb);

        // Build PDU with optional write data
        let data = write_data.unwrap_or(&[]);
        if !data.is_empty() {
            bhs.data_segment_length = data.len() as u32;
        }

        let _pdu = self.build_pdu(&bhs, data);

        // Stub: return empty success response
        Ok(ScsiResponse {
            status: 0x00, // GOOD
            data: Vec::new(),
            sense: Vec::new(),
        })
    }

    /// Send INQUIRY command.
    pub fn inquiry(&mut self, session_idx: usize, lun: u64) -> Result<InquiryData, IscsiError> {
        let cmd = ScsiCommand::inquiry(lun);
        let response = self.scsi_command(session_idx, &cmd, None)?;

        if response.status != 0 {
            return Err(IscsiError::ScsiError);
        }

        // Parse INQUIRY data (if we had real data)
        Ok(InquiryData {
            device_type: if response.data.is_empty() {
                0
            } else {
                response.data[0] & 0x1F
            },
            vendor: Self::extract_string(&response.data, 8, 8),
            product: Self::extract_string(&response.data, 16, 16),
            revision: Self::extract_string(&response.data, 32, 4),
        })
    }

    /// Send READ CAPACITY (10) command.
    pub fn read_capacity(
        &mut self,
        session_idx: usize,
        lun: u64,
    ) -> Result<DiskCapacity, IscsiError> {
        let cmd = ScsiCommand::read_capacity_10(lun);
        let response = self.scsi_command(session_idx, &cmd, None)?;

        if response.status != 0 {
            return Err(IscsiError::ScsiError);
        }

        // Parse READ CAPACITY (10) response: 4 bytes LBA + 4 bytes block size
        let (last_lba, block_size) = if response.data.len() >= 8 {
            let lba = u32::from_be_bytes([
                response.data[0],
                response.data[1],
                response.data[2],
                response.data[3],
            ]);
            let bs = u32::from_be_bytes([
                response.data[4],
                response.data[5],
                response.data[6],
                response.data[7],
            ]);
            (lba, bs)
        } else {
            (0, 512)
        };

        Ok(DiskCapacity {
            last_lba,
            block_size,
            total_bytes: (last_lba as u64 + 1) * block_size as u64,
        })
    }

    /// Read blocks from target.
    pub fn read_blocks(
        &mut self,
        session_idx: usize,
        lun: u64,
        lba: u32,
        block_count: u16,
        block_size: u32,
    ) -> Result<Vec<u8>, IscsiError> {
        let cmd = ScsiCommand::read_10(lun, lba, block_count, block_size);
        let response = self.scsi_command(session_idx, &cmd, None)?;

        if response.status != 0 {
            return Err(IscsiError::ScsiError);
        }

        Ok(response.data)
    }

    /// Write blocks to target.
    pub fn write_blocks(
        &mut self,
        session_idx: usize,
        lun: u64,
        lba: u32,
        block_count: u16,
        block_size: u32,
        data: &[u8],
    ) -> Result<(), IscsiError> {
        let expected_len = block_count as u32 * block_size;
        if data.len() != expected_len as usize {
            return Err(IscsiError::InvalidParameter);
        }

        let cmd = ScsiCommand::write_10(lun, lba, block_count, block_size);
        let response = self.scsi_command(session_idx, &cmd, Some(data))?;

        if response.status != 0 {
            return Err(IscsiError::ScsiError);
        }

        Ok(())
    }

    /// Discover targets via SendTargets text request.
    pub fn discovery(&mut self, session_idx: usize) -> Result<Vec<String>, IscsiError> {
        let tag = self.next_tag();

        let (cmd_sn, exp_stat_sn) = {
            let session = self
                .sessions
                .get_mut(session_idx)
                .ok_or(IscsiError::SessionNotFound)?;

            if session.state != SessionState::FullFeature {
                return Err(IscsiError::NotLoggedIn);
            }

            (session.next_cmd_sn(), session.exp_stat_sn)
        };

        let mut bhs = BhsHeader::new(IscsiOpcode::TextReq);
        bhs.is_final = true;
        bhs.initiator_task_tag = tag;

        let text = b"SendTargets=All\0";
        bhs.data_segment_length = text.len() as u32;

        bhs.specific[0..4].copy_from_slice(&cmd_sn.to_be_bytes());
        bhs.specific[4..8].copy_from_slice(&exp_stat_sn.to_be_bytes());

        let _pdu = self.build_pdu(&bhs, text);

        // Stub: return empty target list
        Ok(Vec::new())
    }

    /// Build a complete iSCSI PDU from BHS and data segment.
    pub fn build_pdu(&self, bhs: &BhsHeader, data: &[u8]) -> Vec<u8> {
        let bhs_bytes = bhs.serialize();
        let padded_len = (data.len() + 3) & !3;

        let mut pdu = Vec::with_capacity(BHS_LENGTH + padded_len);
        pdu.extend_from_slice(&bhs_bytes);
        pdu.extend_from_slice(data);

        // Pad to 4-byte boundary
        while pdu.len() < BHS_LENGTH + padded_len {
            pdu.push(0);
        }

        pdu
    }

    /// Parse a PDU from raw bytes.
    pub fn parse_pdu(&self, data: &[u8]) -> Result<(BhsHeader, Vec<u8>), IscsiError> {
        let bhs = BhsHeader::deserialize(data).ok_or(IscsiError::PduError)?;
        let data_len = bhs.data_segment_length as usize;

        let data_start = BHS_LENGTH;
        let data_end = data_start + data_len;

        if data.len() < data_end {
            return Err(IscsiError::PduError);
        }

        let segment = data[data_start..data_end].to_vec();
        Ok((bhs, segment))
    }

    /// Build login text parameters.
    fn build_login_params(&self, session: &IscsiSession) -> Vec<u8> {
        let mut params = BTreeMap::new();
        params.insert("InitiatorName", session.initiator_name.as_str());
        params.insert("TargetName", session.target_name.as_str());
        params.insert("SessionType", "Normal");
        params.insert("AuthMethod", "None");

        let mut buf = Vec::with_capacity(256);
        for (key, value) in &params {
            let entry = format!("{}={}\0", key, value);
            buf.extend_from_slice(entry.as_bytes());
        }
        buf
    }

    /// Build operational parameter negotiation text.
    #[cfg(feature = "alloc")]
    pub fn build_operational_params(&self) -> Vec<u8> {
        let params = [
            ("MaxRecvDataSegmentLength", "65536"),
            ("MaxBurstLength", "262144"),
            ("FirstBurstLength", "65536"),
            ("MaxOutstandingR2T", "1"),
            ("InitialR2T", "Yes"),
            ("ImmediateData", "Yes"),
            ("DataPDUInOrder", "Yes"),
            ("DataSequenceInOrder", "Yes"),
            ("DefaultTime2Wait", "2"),
            ("DefaultTime2Retain", "0"),
            ("ErrorRecoveryLevel", "0"),
        ];

        let mut buf = Vec::with_capacity(512);
        for (key, value) in &params {
            let entry = format!("{}={}\0", key, value);
            buf.extend_from_slice(entry.as_bytes());
        }
        buf
    }

    /// Get next initiator task tag.
    fn next_tag(&mut self) -> u32 {
        let tag = self.next_tag;
        self.next_tag = self.next_tag.wrapping_add(1);
        tag
    }

    /// Extract an ASCII string from a byte buffer.
    fn extract_string(data: &[u8], offset: usize, len: usize) -> String {
        if data.len() < offset + len {
            return String::new();
        }
        let slice = &data[offset..offset + len];
        // Trim trailing spaces and nulls
        let trimmed = slice
            .iter()
            .rev()
            .skip_while(|&&b| b == b' ' || b == 0)
            .count();
        let end = offset + trimmed;
        String::from_utf8_lossy(&data[offset..end]).into_owned()
    }

    /// Get the target portal address.
    pub fn portal(&self) -> &str {
        &self.target_portal
    }

    /// Get the number of active sessions.
    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    /// Get a session by index.
    pub fn session(&self, idx: usize) -> Option<&IscsiSession> {
        self.sessions.get(idx)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_iscsi_opcode_from_u8() {
        assert_eq!(IscsiOpcode::from_u8(0x00), Some(IscsiOpcode::NopOut));
        assert_eq!(IscsiOpcode::from_u8(0x01), Some(IscsiOpcode::ScsiCommand));
        assert_eq!(IscsiOpcode::from_u8(0x21), Some(IscsiOpcode::ScsiResponse));
        assert_eq!(IscsiOpcode::from_u8(0x3F), Some(IscsiOpcode::Reject));
        assert_eq!(IscsiOpcode::from_u8(0x10), None);
    }

    #[test]
    fn test_opcode_is_initiator() {
        assert!(IscsiOpcode::NopOut.is_initiator());
        assert!(IscsiOpcode::ScsiCommand.is_initiator());
        assert!(!IscsiOpcode::NopIn.is_initiator());
        assert!(!IscsiOpcode::ScsiResponse.is_initiator());
    }

    #[test]
    fn test_scsi_test_unit_ready() {
        let cmd = ScsiCommand::test_unit_ready(0);
        assert_eq!(cmd.cdb[0], ScsiOpcode::TestUnitReady as u8);
        assert_eq!(cmd.data_length, 0);
        assert!(!cmd.is_read);
        assert!(!cmd.is_write);
    }

    #[test]
    fn test_scsi_inquiry() {
        let cmd = ScsiCommand::inquiry(0);
        assert_eq!(cmd.cdb[0], ScsiOpcode::Inquiry as u8);
        assert_eq!(cmd.data_length, 96);
        assert!(cmd.is_read);
    }

    #[test]
    fn test_scsi_read_10() {
        let cmd = ScsiCommand::read_10(0, 100, 8, 512);
        assert_eq!(cmd.cdb[0], ScsiOpcode::Read10 as u8);
        assert_eq!(cmd.data_length, 4096);
        assert!(cmd.is_read);
        // Check LBA encoding
        let lba = u32::from_be_bytes([cmd.cdb[2], cmd.cdb[3], cmd.cdb[4], cmd.cdb[5]]);
        assert_eq!(lba, 100);
    }

    #[test]
    fn test_scsi_write_10() {
        let cmd = ScsiCommand::write_10(0, 200, 4, 512);
        assert_eq!(cmd.cdb[0], ScsiOpcode::Write10 as u8);
        assert_eq!(cmd.data_length, 2048);
        assert!(cmd.is_write);
    }

    #[test]
    fn test_bhs_serialize_deserialize() {
        let mut bhs = BhsHeader::new(IscsiOpcode::ScsiCommand);
        bhs.immediate = true;
        bhs.lun = 42;
        bhs.initiator_task_tag = 0xDEAD_BEEF;
        bhs.data_segment_length = 4096;

        let bytes = bhs.serialize();
        assert_eq!(bytes.len(), BHS_LENGTH);

        let parsed = BhsHeader::deserialize(&bytes).unwrap();
        assert_eq!(parsed.opcode, IscsiOpcode::ScsiCommand);
        assert!(parsed.immediate);
        assert_eq!(parsed.lun, 42);
        assert_eq!(parsed.initiator_task_tag, 0xDEAD_BEEF);
        assert_eq!(parsed.data_segment_length, 4096);
    }

    #[test]
    fn test_bhs_too_short() {
        assert!(BhsHeader::deserialize(&[0; 10]).is_none());
    }

    #[test]
    fn test_bhs_padded_data_length() {
        let mut bhs = BhsHeader::new(IscsiOpcode::NopOut);
        bhs.data_segment_length = 5;
        assert_eq!(bhs.padded_data_length(), 8);

        bhs.data_segment_length = 8;
        assert_eq!(bhs.padded_data_length(), 8);

        bhs.data_segment_length = 0;
        assert_eq!(bhs.padded_data_length(), 0);
    }

    #[test]
    fn test_session_new() {
        let session = IscsiSession::new(
            "iqn.2026-03.os.veridian:init",
            "iqn.2026-03.com.target:disk1",
        );
        assert_eq!(session.state, SessionState::Free);
        assert_eq!(session.tsih, 0);
        assert_eq!(session.cmd_sn, 1);
    }

    #[test]
    fn test_session_cmd_sn_increment() {
        let mut session = IscsiSession::new(
            "iqn.2026-03.os.veridian:init",
            "iqn.2026-03.com.target:disk1",
        );
        assert_eq!(session.next_cmd_sn(), 1);
        assert_eq!(session.next_cmd_sn(), 2);
        assert_eq!(session.next_cmd_sn(), 3);
    }

    #[test]
    fn test_initiator_new() {
        let init = IscsiInitiator::new("192.168.1.100:3260");
        assert_eq!(init.portal(), "192.168.1.100:3260");
        assert_eq!(init.session_count(), 0);
    }

    #[test]
    fn test_initiator_build_pdu() {
        let init = IscsiInitiator::new("10.0.0.1:3260");
        let bhs = BhsHeader::new(IscsiOpcode::NopOut);
        let pdu = init.build_pdu(&bhs, &[1, 2, 3]);
        // BHS(48) + data(3) + pad(1) = 52
        assert_eq!(pdu.len(), 52);
    }
}
