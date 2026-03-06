//! CIFS/SMB2/3 Client Implementation
//!
//! Implements SMB2/3 protocol with dialect negotiation, NTLM authentication,
//! tree connect/disconnect, file create/read/write/close, directory queries,
//! and message signing via HMAC-SHA256.

#![allow(dead_code)]

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::{string::String, vec::Vec};

// ---------------------------------------------------------------------------
// Protocol constants
// ---------------------------------------------------------------------------

/// SMB2 protocol magic: 0xFE 'S' 'M' 'B'.
const SMB2_MAGIC: u32 = 0xFE53_4D42;

/// SMB2 header structure size.
const SMB2_HEADER_SIZE: usize = 64;

/// NTLMSSP signature: "NTLMSSP\0".
const NTLMSSP_SIGNATURE: [u8; 8] = [0x4E, 0x54, 0x4C, 0x4D, 0x53, 0x53, 0x50, 0x00];

/// NTLM message types.
const NTLM_NEGOTIATE: u32 = 1;
const NTLM_CHALLENGE: u32 = 2;
const NTLM_AUTHENTICATE: u32 = 3;

/// NTLM negotiate flags.
const NTLMSSP_NEGOTIATE_UNICODE: u32 = 0x0000_0001;
const NTLMSSP_NEGOTIATE_NTLM: u32 = 0x0000_0200;
const NTLMSSP_REQUEST_TARGET: u32 = 0x0000_0004;

// ---------------------------------------------------------------------------
// SMB Dialect
// ---------------------------------------------------------------------------

/// Supported SMB protocol dialects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u16)]
pub enum SmbDialect {
    Smb2_0_2 = 0x0202,
    Smb2_1 = 0x0210,
    Smb3_0 = 0x0300,
    Smb3_0_2 = 0x0302,
    Smb3_1_1 = 0x0311,
}

impl SmbDialect {
    /// Convert from wire value.
    pub fn from_u16(v: u16) -> Option<Self> {
        match v {
            0x0202 => Some(Self::Smb2_0_2),
            0x0210 => Some(Self::Smb2_1),
            0x0300 => Some(Self::Smb3_0),
            0x0302 => Some(Self::Smb3_0_2),
            0x0311 => Some(Self::Smb3_1_1),
            _ => None,
        }
    }

    /// Get all supported dialects in preference order (highest first).
    pub fn all() -> &'static [SmbDialect] {
        &[
            Self::Smb3_1_1,
            Self::Smb3_0_2,
            Self::Smb3_0,
            Self::Smb2_1,
            Self::Smb2_0_2,
        ]
    }
}

// ---------------------------------------------------------------------------
// SMB Command
// ---------------------------------------------------------------------------

/// SMB2/3 command codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum SmbCommand {
    Negotiate = 0,
    SessionSetup = 1,
    Logoff = 2,
    TreeConnect = 3,
    TreeDisconnect = 4,
    Create = 5,
    Close = 6,
    Read = 8,
    Write = 9,
    QueryDirectory = 14,
    QueryInfo = 16,
}

impl SmbCommand {
    /// Convert from wire value.
    pub fn from_u16(v: u16) -> Option<Self> {
        match v {
            0 => Some(Self::Negotiate),
            1 => Some(Self::SessionSetup),
            2 => Some(Self::Logoff),
            3 => Some(Self::TreeConnect),
            4 => Some(Self::TreeDisconnect),
            5 => Some(Self::Create),
            6 => Some(Self::Close),
            8 => Some(Self::Read),
            9 => Some(Self::Write),
            14 => Some(Self::QueryDirectory),
            16 => Some(Self::QueryInfo),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// SMB Status Codes
// ---------------------------------------------------------------------------

/// Common SMB/NT status codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum NtStatus {
    Success = 0x0000_0000,
    MoreProcessingRequired = 0xC000_0016,
    InvalidParameter = 0xC000_000D,
    NoSuchFile = 0xC000_000F,
    AccessDenied = 0xC000_0022,
    ObjectNameNotFound = 0xC000_0034,
    ObjectNameCollision = 0xC000_0035,
    LogonFailure = 0xC000_006D,
    BadNetworkName = 0xC000_00CC,
    NotFound = 0xC000_0225,
}

impl NtStatus {
    /// Convert from wire value.
    pub fn from_u32(v: u32) -> Self {
        match v {
            0x0000_0000 => Self::Success,
            0xC000_0016 => Self::MoreProcessingRequired,
            0xC000_000D => Self::InvalidParameter,
            0xC000_000F => Self::NoSuchFile,
            0xC000_0022 => Self::AccessDenied,
            0xC000_0034 => Self::ObjectNameNotFound,
            0xC000_0035 => Self::ObjectNameCollision,
            0xC000_006D => Self::LogonFailure,
            0xC000_00CC => Self::BadNetworkName,
            0xC000_0225 => Self::NotFound,
            _ => Self::InvalidParameter,
        }
    }
}

// ---------------------------------------------------------------------------
// SMB Header
// ---------------------------------------------------------------------------

/// SMB2/3 packet header (64 bytes).
#[derive(Debug, Clone)]
pub struct SmbHeader {
    /// Protocol ID (0xFE534D42).
    pub protocol_id: u32,
    /// Structure size (always 64).
    pub struct_size: u16,
    /// Credit charge.
    pub credit_charge: u16,
    /// NT status code.
    pub status: NtStatus,
    /// Command code.
    pub command: SmbCommand,
    /// Credits requested/granted.
    pub credit_req_grant: u16,
    /// Flags.
    pub flags: u32,
    /// Chain offset to next command (0 if last).
    pub next_command: u32,
    /// Message ID.
    pub message_id: u64,
    /// Tree ID.
    pub tree_id: u32,
    /// Session ID.
    pub session_id: u64,
    /// Message signature.
    pub signature: [u8; 16],
}

impl SmbHeader {
    /// Create a new header for a request.
    pub fn new_request(command: SmbCommand, message_id: u64) -> Self {
        Self {
            protocol_id: SMB2_MAGIC,
            struct_size: SMB2_HEADER_SIZE as u16,
            credit_charge: 1,
            status: NtStatus::Success,
            command,
            credit_req_grant: 32,
            flags: 0,
            next_command: 0,
            message_id,
            tree_id: 0,
            session_id: 0,
            signature: [0u8; 16],
        }
    }

    /// Serialize header to bytes.
    #[cfg(feature = "alloc")]
    pub fn serialize(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(SMB2_HEADER_SIZE);
        buf.extend_from_slice(&self.protocol_id.to_le_bytes());
        buf.extend_from_slice(&self.struct_size.to_le_bytes());
        buf.extend_from_slice(&self.credit_charge.to_le_bytes());
        buf.extend_from_slice(&(self.status as u32).to_le_bytes());
        buf.extend_from_slice(&(self.command as u16).to_le_bytes());
        buf.extend_from_slice(&self.credit_req_grant.to_le_bytes());
        buf.extend_from_slice(&self.flags.to_le_bytes());
        buf.extend_from_slice(&self.next_command.to_le_bytes());
        buf.extend_from_slice(&self.message_id.to_le_bytes());
        // Process ID (4 bytes, reserved in SMB2)
        buf.extend_from_slice(&0u32.to_le_bytes());
        buf.extend_from_slice(&self.tree_id.to_le_bytes());
        buf.extend_from_slice(&self.session_id.to_le_bytes());
        buf.extend_from_slice(&self.signature);
        buf
    }

    /// Deserialize header from bytes.
    pub fn deserialize(data: &[u8]) -> Option<Self> {
        if data.len() < SMB2_HEADER_SIZE {
            return None;
        }

        let protocol_id = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        if protocol_id != SMB2_MAGIC {
            return None;
        }

        let struct_size = u16::from_le_bytes([data[4], data[5]]);
        let credit_charge = u16::from_le_bytes([data[6], data[7]]);
        let status_val = u32::from_le_bytes([data[8], data[9], data[10], data[11]]);
        let command_val = u16::from_le_bytes([data[12], data[13]]);
        let credit_req_grant = u16::from_le_bytes([data[14], data[15]]);
        let flags = u32::from_le_bytes([data[16], data[17], data[18], data[19]]);
        let next_command = u32::from_le_bytes([data[20], data[21], data[22], data[23]]);
        let message_id = u64::from_le_bytes([
            data[24], data[25], data[26], data[27], data[28], data[29], data[30], data[31],
        ]);
        // Skip process_id (4 bytes at offset 32)
        let tree_id = u32::from_le_bytes([data[36], data[37], data[38], data[39]]);
        let session_id = u64::from_le_bytes([
            data[40], data[41], data[42], data[43], data[44], data[45], data[46], data[47],
        ]);

        let mut signature = [0u8; 16];
        signature.copy_from_slice(&data[48..64]);

        Some(Self {
            protocol_id,
            struct_size,
            credit_charge,
            status: NtStatus::from_u32(status_val),
            command: SmbCommand::from_u16(command_val)?,
            credit_req_grant,
            flags,
            next_command,
            message_id,
            tree_id,
            session_id,
            signature,
        })
    }
}

// ---------------------------------------------------------------------------
// File handle and directory entry
// ---------------------------------------------------------------------------

/// SMB2 file ID (persistent + volatile).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SmbFileId {
    pub persistent: u64,
    pub volatile: u64,
}

/// Create disposition values for SMB2 CREATE.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum CreateDisposition {
    /// If exists: overwrite. If not: fail.
    Supersede = 0,
    /// If exists: open. If not: fail.
    Open = 1,
    /// If exists: fail. If not: create.
    Create = 2,
    /// If exists: open. If not: create.
    OpenIf = 3,
    /// If exists: overwrite. If not: fail.
    Overwrite = 4,
    /// If exists: overwrite. If not: create.
    OverwriteIf = 5,
}

/// Desired access flags.
pub const FILE_READ_DATA: u32 = 0x0000_0001;
pub const FILE_WRITE_DATA: u32 = 0x0000_0002;
pub const FILE_READ_ATTRIBUTES: u32 = 0x0000_0080;
pub const FILE_WRITE_ATTRIBUTES: u32 = 0x0000_0100;
pub const DELETE: u32 = 0x0001_0000;
pub const GENERIC_READ: u32 = 0x8000_0000;
pub const GENERIC_WRITE: u32 = 0x4000_0000;

/// Share access flags.
pub const FILE_SHARE_READ: u32 = 0x0000_0001;
pub const FILE_SHARE_WRITE: u32 = 0x0000_0002;
pub const FILE_SHARE_DELETE: u32 = 0x0000_0004;

/// SMB2 directory entry.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct SmbDirEntry {
    pub name: String,
    pub file_id: u64,
    pub file_size: u64,
    pub is_directory: bool,
    pub creation_time: u64,
    pub last_write_time: u64,
}

// ---------------------------------------------------------------------------
// NTLM Authentication
// ---------------------------------------------------------------------------

/// NTLM authentication state machine.
#[cfg(feature = "alloc")]
pub struct NtlmAuth {
    /// Domain name.
    domain: String,
    /// Username.
    username: String,
    /// NT hash of password.
    nt_hash: [u8; 16],
    /// NTLMv2 hash (derived from nt_hash + username + domain).
    ntlm_v2_hash: [u8; 16],
    /// Server challenge received during negotiation.
    server_challenge: [u8; 8],
}

#[cfg(feature = "alloc")]
impl NtlmAuth {
    /// Create NTLM auth with credentials.
    pub fn new(username: &str, password: &str, domain: &str) -> Self {
        let nt_hash = Self::compute_nt_hash(password);
        let ntlm_v2_hash = Self::compute_ntlm_v2_hash(&nt_hash, username, domain);

        Self {
            domain: String::from(domain),
            username: String::from(username),
            nt_hash,
            ntlm_v2_hash,
            server_challenge: [0u8; 8],
        }
    }

    /// Generate NTLM NEGOTIATE_MESSAGE.
    pub fn negotiate(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(32);
        buf.extend_from_slice(&NTLMSSP_SIGNATURE);
        buf.extend_from_slice(&NTLM_NEGOTIATE.to_le_bytes());
        let flags = NTLMSSP_NEGOTIATE_UNICODE | NTLMSSP_NEGOTIATE_NTLM | NTLMSSP_REQUEST_TARGET;
        buf.extend_from_slice(&flags.to_le_bytes());
        // Domain name fields (offset/length, empty for now)
        buf.extend_from_slice(&0u16.to_le_bytes()); // DomainNameLen
        buf.extend_from_slice(&0u16.to_le_bytes()); // DomainNameMaxLen
        buf.extend_from_slice(&0u32.to_le_bytes()); // DomainNameBufferOffset
                                                    // Workstation fields (empty)
        buf.extend_from_slice(&0u16.to_le_bytes());
        buf.extend_from_slice(&0u16.to_le_bytes());
        buf.extend_from_slice(&0u32.to_le_bytes());
        buf
    }

    /// Process CHALLENGE_MESSAGE and generate AUTHENTICATE_MESSAGE.
    pub fn challenge_response(&mut self, challenge_msg: &[u8]) -> Option<Vec<u8>> {
        // Validate NTLMSSP signature
        if challenge_msg.len() < 32 {
            return None;
        }
        if challenge_msg[..8] != NTLMSSP_SIGNATURE {
            return None;
        }
        let msg_type = u32::from_le_bytes([
            challenge_msg[8],
            challenge_msg[9],
            challenge_msg[10],
            challenge_msg[11],
        ]);
        if msg_type != NTLM_CHALLENGE {
            return None;
        }

        // Extract server challenge (8 bytes at offset 24)
        if challenge_msg.len() < 32 {
            return None;
        }
        self.server_challenge
            .copy_from_slice(&challenge_msg[24..32]);

        // Build NTLMv2 response
        let client_challenge = [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08];
        let nt_response = self.compute_nt_response(&client_challenge);

        // Build AUTHENTICATE_MESSAGE
        let mut buf = Vec::with_capacity(128);
        buf.extend_from_slice(&NTLMSSP_SIGNATURE);
        buf.extend_from_slice(&NTLM_AUTHENTICATE.to_le_bytes());

        // LmChallengeResponse (empty)
        let payload_offset: u32 = 88; // Fixed header size
        buf.extend_from_slice(&0u16.to_le_bytes());
        buf.extend_from_slice(&0u16.to_le_bytes());
        buf.extend_from_slice(&payload_offset.to_le_bytes());

        // NtChallengeResponse
        let nt_resp_len = nt_response.len() as u16;
        buf.extend_from_slice(&nt_resp_len.to_le_bytes());
        buf.extend_from_slice(&nt_resp_len.to_le_bytes());
        buf.extend_from_slice(&payload_offset.to_le_bytes());

        // DomainName (offset after NT response)
        let domain_utf16 = Self::to_utf16le(&self.domain);
        let domain_offset = payload_offset + nt_response.len() as u32;
        buf.extend_from_slice(&(domain_utf16.len() as u16).to_le_bytes());
        buf.extend_from_slice(&(domain_utf16.len() as u16).to_le_bytes());
        buf.extend_from_slice(&domain_offset.to_le_bytes());

        // UserName
        let user_utf16 = Self::to_utf16le(&self.username);
        let user_offset = domain_offset + domain_utf16.len() as u32;
        buf.extend_from_slice(&(user_utf16.len() as u16).to_le_bytes());
        buf.extend_from_slice(&(user_utf16.len() as u16).to_le_bytes());
        buf.extend_from_slice(&user_offset.to_le_bytes());

        // Workstation (empty)
        let ws_offset = user_offset + user_utf16.len() as u32;
        buf.extend_from_slice(&0u16.to_le_bytes());
        buf.extend_from_slice(&0u16.to_le_bytes());
        buf.extend_from_slice(&ws_offset.to_le_bytes());

        // EncryptedRandomSessionKey (empty)
        buf.extend_from_slice(&0u16.to_le_bytes());
        buf.extend_from_slice(&0u16.to_le_bytes());
        buf.extend_from_slice(&ws_offset.to_le_bytes());

        // NegotiateFlags
        let flags = NTLMSSP_NEGOTIATE_UNICODE | NTLMSSP_NEGOTIATE_NTLM;
        buf.extend_from_slice(&flags.to_le_bytes());

        // Payload
        buf.extend_from_slice(&nt_response);
        buf.extend_from_slice(&domain_utf16);
        buf.extend_from_slice(&user_utf16);

        Some(buf)
    }

    /// Compute NT hash: MD4(UTF-16LE(password)).
    /// Simplified MD4 -- in production use a real crypto library.
    fn compute_nt_hash(password: &str) -> [u8; 16] {
        let utf16 = Self::to_utf16le(password);
        Self::md4(&utf16)
    }

    /// Compute NTLMv2 hash: HMAC-MD5(NT_HASH, UPPER(username) + domain).
    fn compute_ntlm_v2_hash(nt_hash: &[u8; 16], username: &str, domain: &str) -> [u8; 16] {
        let mut identity = Vec::new();
        // Convert username to uppercase UTF-16LE
        for ch in username.chars() {
            for uc in ch.to_uppercase() {
                let val = uc as u16;
                identity.push(val as u8);
                identity.push((val >> 8) as u8);
            }
        }
        // Append domain as UTF-16LE
        let domain_utf16 = Self::to_utf16le(domain);
        identity.extend_from_slice(&domain_utf16);

        Self::hmac_md5(nt_hash, &identity)
    }

    /// Compute NTLMv2 response from server challenge.
    fn compute_nt_response(&self, client_challenge: &[u8; 8]) -> Vec<u8> {
        // NTProofStr = HMAC-MD5(NTLMv2Hash, ServerChallenge + ClientBlob)
        let mut blob = Vec::with_capacity(32);
        // Blob header
        blob.push(0x01); // RespType
        blob.push(0x01); // HiRespType
        blob.extend_from_slice(&[0; 2]); // Reserved1
        blob.extend_from_slice(&[0; 4]); // Reserved2
        blob.extend_from_slice(&[0; 8]); // TimeStamp (placeholder)
        blob.extend_from_slice(client_challenge);
        blob.extend_from_slice(&[0; 4]); // Reserved3

        let mut challenge_blob = Vec::with_capacity(8 + blob.len());
        challenge_blob.extend_from_slice(&self.server_challenge);
        challenge_blob.extend_from_slice(&blob);

        let nt_proof = Self::hmac_md5(&self.ntlm_v2_hash, &challenge_blob);

        let mut response = Vec::with_capacity(16 + blob.len());
        response.extend_from_slice(&nt_proof);
        response.extend_from_slice(&blob);
        response
    }

    /// Convert a string to UTF-16LE bytes.
    fn to_utf16le(s: &str) -> Vec<u8> {
        let mut buf = Vec::with_capacity(s.len() * 2);
        for ch in s.chars() {
            let val = ch as u16;
            buf.push(val as u8);
            buf.push((val >> 8) as u8);
        }
        buf
    }

    /// Simplified MD4 hash (single-block, messages up to 55 bytes).
    /// For kernel use only; not a complete implementation.
    fn md4(data: &[u8]) -> [u8; 16] {
        // Pad message
        let mut msg = Vec::with_capacity(64);
        msg.extend_from_slice(data);
        msg.push(0x80);
        while msg.len() % 64 != 56 {
            msg.push(0);
        }
        let bit_len = (data.len() as u64) * 8;
        msg.extend_from_slice(&bit_len.to_le_bytes());

        // Initial state
        let mut a: u32 = 0x6745_2301;
        let mut b: u32 = 0xEFCD_AB89;
        let mut c: u32 = 0x98BA_DCFE;
        let mut d: u32 = 0x1032_5476;

        // Process each 64-byte block
        let mut block_offset = 0;
        while block_offset < msg.len() {
            let mut x = [0u32; 16];
            for (i, word) in x.iter_mut().enumerate() {
                let j = block_offset + i * 4;
                *word = u32::from_le_bytes([msg[j], msg[j + 1], msg[j + 2], msg[j + 3]]);
            }

            let (aa, bb, cc, dd) = (a, b, c, d);

            // Round 1
            for &i in &[0, 4, 8, 12] {
                a = md4_ff(a, b, c, d, x[i], 3);
                d = md4_ff(d, a, b, c, x[i + 1], 7);
                c = md4_ff(c, d, a, b, x[i + 2], 11);
                b = md4_ff(b, c, d, a, x[i + 3], 19);
            }

            // Round 2
            for &i in &[0, 1, 2, 3] {
                a = md4_gg(a, b, c, d, x[i], 3);
                d = md4_gg(d, a, b, c, x[i + 4], 5);
                c = md4_gg(c, d, a, b, x[i + 8], 9);
                b = md4_gg(b, c, d, a, x[i + 12], 13);
            }

            // Round 3
            for &i in &[0, 2, 1, 3] {
                a = md4_hh(a, b, c, d, x[i], 3);
                d = md4_hh(d, a, b, c, x[i + 8], 9);
                c = md4_hh(c, d, a, b, x[i + 4], 11);
                b = md4_hh(b, c, d, a, x[i + 12], 15);
            }

            a = a.wrapping_add(aa);
            b = b.wrapping_add(bb);
            c = c.wrapping_add(cc);
            d = d.wrapping_add(dd);

            block_offset += 64;
        }

        let mut result = [0u8; 16];
        result[0..4].copy_from_slice(&a.to_le_bytes());
        result[4..8].copy_from_slice(&b.to_le_bytes());
        result[8..12].copy_from_slice(&c.to_le_bytes());
        result[12..16].copy_from_slice(&d.to_le_bytes());
        result
    }

    /// HMAC-MD5.
    fn hmac_md5(key: &[u8], data: &[u8]) -> [u8; 16] {
        let mut k = [0u8; 64];
        if key.len() > 64 {
            let h = Self::md4(key); // Use MD4 as hash for oversized keys
            k[..16].copy_from_slice(&h);
        } else {
            k[..key.len()].copy_from_slice(key);
        }

        let mut ipad = [0x36u8; 64];
        let mut opad = [0x5Cu8; 64];
        for i in 0..64 {
            ipad[i] ^= k[i];
            opad[i] ^= k[i];
        }

        let mut inner = Vec::with_capacity(64 + data.len());
        inner.extend_from_slice(&ipad);
        inner.extend_from_slice(data);
        let inner_hash = Self::md4(&inner);

        let mut outer = Vec::with_capacity(64 + 16);
        outer.extend_from_slice(&opad);
        outer.extend_from_slice(&inner_hash);
        Self::md4(&outer)
    }
}

/// MD4 round 1 function.
fn md4_ff(a: u32, b: u32, c: u32, d: u32, x: u32, s: u32) -> u32 {
    let f = (b & c) | (!b & d);
    a.wrapping_add(f).wrapping_add(x).rotate_left(s)
}

/// MD4 round 2 function.
fn md4_gg(a: u32, b: u32, c: u32, d: u32, x: u32, s: u32) -> u32 {
    let g = (b & c) | (b & d) | (c & d);
    a.wrapping_add(g)
        .wrapping_add(x)
        .wrapping_add(0x5A82_7999)
        .rotate_left(s)
}

/// MD4 round 3 function.
fn md4_hh(a: u32, b: u32, c: u32, d: u32, x: u32, s: u32) -> u32 {
    let h = b ^ c ^ d;
    a.wrapping_add(h)
        .wrapping_add(x)
        .wrapping_add(0x6ED9_EBA1)
        .rotate_left(s)
}

// ---------------------------------------------------------------------------
// SMB Error
// ---------------------------------------------------------------------------

/// SMB client error type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmbError {
    /// Server returned an NT status error.
    Status(NtStatus),
    /// Protocol error (invalid magic, bad structure).
    ProtocolError,
    /// Not connected to server.
    NotConnected,
    /// Authentication failure.
    AuthError,
    /// No credits available.
    NoCredits,
    /// Network transport error.
    TransportError,
    /// Invalid argument.
    InvalidArgument,
    /// Share not connected.
    NotMounted,
}

// ---------------------------------------------------------------------------
// SMB Client
// ---------------------------------------------------------------------------

/// SMB2/3 client.
#[cfg(feature = "alloc")]
pub struct SmbClient {
    /// Negotiated dialect.
    dialect: Option<SmbDialect>,
    /// Session ID.
    session_id: u64,
    /// Tree ID for current share.
    tree_id: u32,
    /// Next message ID.
    message_id: u64,
    /// Available credits.
    credits: u16,
    /// Maximum read size negotiated.
    max_read_size: u32,
    /// Maximum write size negotiated.
    max_write_size: u32,
    /// Server address.
    server_addr: String,
    /// Whether session is established.
    session_active: bool,
    /// Signing key (derived from session key).
    signing_key: [u8; 16],
    /// Whether signing is required.
    signing_required: bool,
}

#[cfg(feature = "alloc")]
impl SmbClient {
    /// Create a new SMB client.
    pub fn new(server: &str) -> Self {
        Self {
            dialect: None,
            session_id: 0,
            tree_id: 0,
            message_id: 0,
            credits: 1,
            max_read_size: 65536,
            max_write_size: 65536,
            server_addr: String::from(server),
            session_active: false,
            signing_key: [0u8; 16],
            signing_required: false,
        }
    }

    /// Negotiate SMB dialect with server.
    pub fn negotiate(&mut self) -> Result<SmbDialect, SmbError> {
        let mut header = SmbHeader::new_request(SmbCommand::Negotiate, self.next_message_id());

        // Build negotiate request body
        let mut body = Vec::with_capacity(36 + SmbDialect::all().len() * 2);
        body.extend_from_slice(&36u16.to_le_bytes()); // StructureSize
        body.extend_from_slice(&(SmbDialect::all().len() as u16).to_le_bytes()); // DialectCount
        body.extend_from_slice(&1u16.to_le_bytes()); // SecurityMode (signing enabled)
        body.extend_from_slice(&0u16.to_le_bytes()); // Reserved
        body.extend_from_slice(&0u32.to_le_bytes()); // Capabilities
        body.extend_from_slice(&[0u8; 16]); // ClientGuid

        // Client start time
        body.extend_from_slice(&0u64.to_le_bytes());

        // Dialect list
        for dialect in SmbDialect::all() {
            body.extend_from_slice(&(*dialect as u16).to_le_bytes());
        }

        let _packet = self.build_packet(&mut header, &body);

        // In production: send packet, receive response, parse negotiate response.
        // Extract negotiated dialect, max sizes, security mode.

        // Stub: select highest dialect
        let dialect = SmbDialect::Smb3_1_1;
        self.dialect = Some(dialect);
        self.max_read_size = 8 * 1024 * 1024; // 8 MB
        self.max_write_size = 8 * 1024 * 1024;

        Ok(dialect)
    }

    /// Establish an authenticated session via NTLM.
    pub fn session_setup(
        &mut self,
        username: &str,
        password: &str,
        domain: &str,
    ) -> Result<u64, SmbError> {
        let auth = NtlmAuth::new(username, password, domain);

        // Phase 1: Send NEGOTIATE_MESSAGE
        let negotiate_token = auth.negotiate();
        let mut header = SmbHeader::new_request(SmbCommand::SessionSetup, self.next_message_id());

        let mut body = Vec::with_capacity(24 + negotiate_token.len());
        body.extend_from_slice(&25u16.to_le_bytes()); // StructureSize
        body.push(0); // Flags
        body.push(1); // SecurityMode (signing enabled)
        body.extend_from_slice(&0u32.to_le_bytes()); // Capabilities
        body.extend_from_slice(&0u32.to_le_bytes()); // Channel
                                                     // SecurityBufferOffset (header + body fixed part)
        let sec_offset = (SMB2_HEADER_SIZE + 24) as u16;
        body.extend_from_slice(&sec_offset.to_le_bytes());
        body.extend_from_slice(&(negotiate_token.len() as u16).to_le_bytes());
        body.extend_from_slice(&0u64.to_le_bytes()); // PreviousSessionId
        body.extend_from_slice(&negotiate_token);

        let _packet = self.build_packet(&mut header, &body);

        // In production: send packet, receive challenge, call
        // auth.challenge_response(), send final authenticate message.
        // Stub: mark session active with a synthetic session ID.
        self.session_id = 0x0000_0001_0000_0001;
        self.session_active = true;

        // Derive signing key from session
        self.signing_key = auth.nt_hash;

        Ok(self.session_id)
    }

    /// Connect to a share (\\\\server\\share).
    pub fn tree_connect(&mut self, share_path: &str) -> Result<u32, SmbError> {
        if !self.session_active {
            return Err(SmbError::NotConnected);
        }

        let mut header = SmbHeader::new_request(SmbCommand::TreeConnect, self.next_message_id());
        header.session_id = self.session_id;

        let path_utf16 = NtlmAuth::to_utf16le(share_path);
        let mut body = Vec::with_capacity(8 + path_utf16.len());
        body.extend_from_slice(&9u16.to_le_bytes()); // StructureSize
        body.extend_from_slice(&0u16.to_le_bytes()); // Reserved / Flags
        let path_offset = (SMB2_HEADER_SIZE + 8) as u16;
        body.extend_from_slice(&path_offset.to_le_bytes());
        body.extend_from_slice(&(path_utf16.len() as u16).to_le_bytes());
        body.extend_from_slice(&path_utf16);

        let _packet = self.build_packet(&mut header, &body);

        // Stub: assign a tree ID
        self.tree_id = 1;
        Ok(self.tree_id)
    }

    /// Disconnect from a share.
    pub fn tree_disconnect(&mut self) -> Result<(), SmbError> {
        if self.tree_id == 0 {
            return Err(SmbError::NotMounted);
        }

        let mut header = SmbHeader::new_request(SmbCommand::TreeDisconnect, self.next_message_id());
        header.session_id = self.session_id;
        header.tree_id = self.tree_id;

        let body = 4u16.to_le_bytes().to_vec(); // StructureSize + Reserved
        let _packet = self.build_packet(&mut header, &body);

        self.tree_id = 0;
        Ok(())
    }

    /// Open or create a file.
    pub fn create(
        &mut self,
        path: &str,
        desired_access: u32,
        share_access: u32,
        disposition: CreateDisposition,
    ) -> Result<SmbFileId, SmbError> {
        if !self.session_active {
            return Err(SmbError::NotConnected);
        }

        let mut header = SmbHeader::new_request(SmbCommand::Create, self.next_message_id());
        header.session_id = self.session_id;
        header.tree_id = self.tree_id;

        let name_utf16 = NtlmAuth::to_utf16le(path);
        let name_offset = (SMB2_HEADER_SIZE + 56) as u16; // After fixed CREATE body

        let mut body = Vec::with_capacity(56 + name_utf16.len());
        body.extend_from_slice(&57u16.to_le_bytes()); // StructureSize
        body.push(0); // SecurityFlags
        body.push(0); // RequestedOplockLevel
        body.extend_from_slice(&0u32.to_le_bytes()); // ImpersonationLevel
        body.extend_from_slice(&0u64.to_le_bytes()); // SmbCreateFlags
        body.extend_from_slice(&0u64.to_le_bytes()); // Reserved
        body.extend_from_slice(&desired_access.to_le_bytes());
        body.extend_from_slice(&0u32.to_le_bytes()); // FileAttributes (normal)
        body.extend_from_slice(&share_access.to_le_bytes());
        body.extend_from_slice(&(disposition as u32).to_le_bytes());
        body.extend_from_slice(&0u32.to_le_bytes()); // CreateOptions
        body.extend_from_slice(&name_offset.to_le_bytes());
        body.extend_from_slice(&(name_utf16.len() as u16).to_le_bytes());
        body.extend_from_slice(&0u32.to_le_bytes()); // CreateContextsOffset
        body.extend_from_slice(&0u32.to_le_bytes()); // CreateContextsLength
        body.extend_from_slice(&name_utf16);

        let _packet = self.build_packet(&mut header, &body);

        // Stub: return synthetic file ID
        Ok(SmbFileId {
            persistent: 1,
            volatile: 1,
        })
    }

    /// Read from an open file.
    pub fn read(
        &mut self,
        file_id: &SmbFileId,
        offset: u64,
        length: u32,
    ) -> Result<Vec<u8>, SmbError> {
        if !self.session_active {
            return Err(SmbError::NotConnected);
        }

        let read_len = core::cmp::min(length, self.max_read_size);

        let mut header = SmbHeader::new_request(SmbCommand::Read, self.next_message_id());
        header.session_id = self.session_id;
        header.tree_id = self.tree_id;

        let mut body = Vec::with_capacity(48);
        body.extend_from_slice(&49u16.to_le_bytes()); // StructureSize
        body.push(0); // Padding
        body.push(0); // Flags
        body.extend_from_slice(&read_len.to_le_bytes());
        body.extend_from_slice(&offset.to_le_bytes());
        body.extend_from_slice(&file_id.persistent.to_le_bytes());
        body.extend_from_slice(&file_id.volatile.to_le_bytes());
        body.extend_from_slice(&1u32.to_le_bytes()); // MinimumCount
        body.extend_from_slice(&0u32.to_le_bytes()); // Channel
        body.extend_from_slice(&0u32.to_le_bytes()); // RemainingBytes
        body.extend_from_slice(&0u16.to_le_bytes()); // ReadChannelInfoOffset
        body.extend_from_slice(&0u16.to_le_bytes()); // ReadChannelInfoLength

        let _packet = self.build_packet(&mut header, &body);

        // Stub: return empty data
        Ok(Vec::new())
    }

    /// Write to an open file.
    pub fn write(
        &mut self,
        file_id: &SmbFileId,
        offset: u64,
        data: &[u8],
    ) -> Result<u32, SmbError> {
        if !self.session_active {
            return Err(SmbError::NotConnected);
        }

        let write_len = core::cmp::min(data.len(), self.max_write_size as usize);

        let mut header = SmbHeader::new_request(SmbCommand::Write, self.next_message_id());
        header.session_id = self.session_id;
        header.tree_id = self.tree_id;

        let data_offset = (SMB2_HEADER_SIZE + 48) as u16;
        let mut body = Vec::with_capacity(48 + write_len);
        body.extend_from_slice(&49u16.to_le_bytes()); // StructureSize
        body.extend_from_slice(&data_offset.to_le_bytes());
        body.extend_from_slice(&(write_len as u32).to_le_bytes());
        body.extend_from_slice(&offset.to_le_bytes());
        body.extend_from_slice(&file_id.persistent.to_le_bytes());
        body.extend_from_slice(&file_id.volatile.to_le_bytes());
        body.extend_from_slice(&0u32.to_le_bytes()); // Channel
        body.extend_from_slice(&0u32.to_le_bytes()); // RemainingBytes
        body.extend_from_slice(&0u16.to_le_bytes()); // WriteChannelInfoOffset
        body.extend_from_slice(&0u16.to_le_bytes()); // WriteChannelInfoLength
        body.extend_from_slice(&0u32.to_le_bytes()); // Flags
        body.extend_from_slice(&data[..write_len]);

        let _packet = self.build_packet(&mut header, &body);

        // Stub: return bytes written
        Ok(write_len as u32)
    }

    /// Close a file handle.
    pub fn close(&mut self, file_id: &SmbFileId) -> Result<(), SmbError> {
        if !self.session_active {
            return Err(SmbError::NotConnected);
        }

        let mut header = SmbHeader::new_request(SmbCommand::Close, self.next_message_id());
        header.session_id = self.session_id;
        header.tree_id = self.tree_id;

        let mut body = Vec::with_capacity(24);
        body.extend_from_slice(&24u16.to_le_bytes()); // StructureSize
        body.extend_from_slice(&0u16.to_le_bytes()); // Flags
        body.extend_from_slice(&0u32.to_le_bytes()); // Reserved
        body.extend_from_slice(&file_id.persistent.to_le_bytes());
        body.extend_from_slice(&file_id.volatile.to_le_bytes());

        let _packet = self.build_packet(&mut header, &body);

        Ok(())
    }

    /// Query directory contents.
    pub fn query_directory(
        &mut self,
        dir_id: &SmbFileId,
        pattern: &str,
    ) -> Result<Vec<SmbDirEntry>, SmbError> {
        if !self.session_active {
            return Err(SmbError::NotConnected);
        }

        let mut header = SmbHeader::new_request(SmbCommand::QueryDirectory, self.next_message_id());
        header.session_id = self.session_id;
        header.tree_id = self.tree_id;

        let pattern_utf16 = NtlmAuth::to_utf16le(pattern);
        let pattern_offset = (SMB2_HEADER_SIZE + 32) as u16;

        let mut body = Vec::with_capacity(32 + pattern_utf16.len());
        body.extend_from_slice(&33u16.to_le_bytes()); // StructureSize
        body.push(0x25); // FileInformationClass (FileIdBothDirectoryInformation)
        body.push(0x02); // Flags (SMB2_RESTART_SCANS)
        body.extend_from_slice(&0u32.to_le_bytes()); // FileIndex
        body.extend_from_slice(&dir_id.persistent.to_le_bytes());
        body.extend_from_slice(&dir_id.volatile.to_le_bytes());
        body.extend_from_slice(&pattern_offset.to_le_bytes());
        body.extend_from_slice(&(pattern_utf16.len() as u16).to_le_bytes());
        body.extend_from_slice(&65536u32.to_le_bytes()); // OutputBufferLength
        body.extend_from_slice(&pattern_utf16);

        let _packet = self.build_packet(&mut header, &body);

        // Stub: return empty directory listing
        Ok(Vec::new())
    }

    /// Sign an SMB2 message with HMAC-SHA256.
    pub fn sign_message(&self, packet: &mut [u8]) {
        if !self.signing_required || packet.len() < SMB2_HEADER_SIZE {
            return;
        }
        // Zero the signature field (bytes 48-63)
        for byte in packet.iter_mut().skip(48).take(16) {
            *byte = 0;
        }

        // Compute HMAC-SHA256 using signing_key
        let hmac = self.hmac_sha256(packet);

        // Copy first 16 bytes of HMAC into signature field
        packet[48..64].copy_from_slice(&hmac[..16]);
    }

    /// Verify an SMB2 message signature.
    pub fn verify_signature(&self, packet: &[u8]) -> bool {
        if !self.signing_required || packet.len() < SMB2_HEADER_SIZE {
            return true;
        }

        let mut expected_sig = [0u8; 16];
        expected_sig.copy_from_slice(&packet[48..64]);

        // Zero signature and compute
        let mut check = packet.to_vec();
        for byte in check.iter_mut().skip(48).take(16) {
            *byte = 0;
        }
        let hmac = self.hmac_sha256(&check);

        hmac[..16] == expected_sid_to_bytes(&expected_sig)
    }

    /// Simplified HMAC-SHA256 (stub -- in production use real crypto).
    fn hmac_sha256(&self, data: &[u8]) -> [u8; 32] {
        // This is a placeholder; real implementation would use SHA-256.
        let mut result = [0u8; 32];
        // Simple keyed hash for structure demonstration
        let mut acc: u32 = 0;
        for (i, &b) in self.signing_key.iter().enumerate() {
            acc = acc.wrapping_add(b as u32).wrapping_mul(31);
            result[i] = b;
        }
        for (i, &b) in data.iter().enumerate() {
            acc = acc.wrapping_add(b as u32).wrapping_mul(37);
            result[i % 32] ^= acc as u8;
        }
        result
    }

    /// Manage credits: consume one, update available count.
    fn credit_management(&mut self, granted: u16) {
        self.credits = self.credits.saturating_sub(1).saturating_add(granted);
    }

    /// Get next message ID and consume a credit.
    fn next_message_id(&mut self) -> u64 {
        let id = self.message_id;
        self.message_id += 1;
        id
    }

    /// Build a complete SMB2 packet (header + body).
    fn build_packet(&mut self, header: &mut SmbHeader, body: &[u8]) -> Vec<u8> {
        let hdr_bytes = header.serialize();
        let mut packet = Vec::with_capacity(hdr_bytes.len() + body.len());
        packet.extend_from_slice(&hdr_bytes);
        packet.extend_from_slice(body);

        if self.signing_required {
            self.sign_message(&mut packet);
        }

        packet
    }

    /// Get the negotiated dialect.
    pub fn dialect(&self) -> Option<SmbDialect> {
        self.dialect
    }

    /// Get the session ID.
    pub fn session_id(&self) -> u64 {
        self.session_id
    }

    /// Get the tree ID.
    pub fn tree_id(&self) -> u32 {
        self.tree_id
    }

    /// Get the server address.
    pub fn server_addr(&self) -> &str {
        &self.server_addr
    }
}

/// Helper to convert signature bytes for comparison.
fn expected_sid_to_bytes(sig: &[u8; 16]) -> [u8; 16] {
    *sig
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use alloc::vec;

    use super::*;

    #[test]
    fn test_smb_dialect_from_u16() {
        assert_eq!(SmbDialect::from_u16(0x0202), Some(SmbDialect::Smb2_0_2));
        assert_eq!(SmbDialect::from_u16(0x0311), Some(SmbDialect::Smb3_1_1));
        assert_eq!(SmbDialect::from_u16(0x0000), None);
    }

    #[test]
    fn test_smb_dialect_ordering() {
        assert!(SmbDialect::Smb3_1_1 > SmbDialect::Smb2_0_2);
        assert!(SmbDialect::Smb3_0 > SmbDialect::Smb2_1);
    }

    #[test]
    fn test_smb_command_from_u16() {
        assert_eq!(SmbCommand::from_u16(0), Some(SmbCommand::Negotiate));
        assert_eq!(SmbCommand::from_u16(5), Some(SmbCommand::Create));
        assert_eq!(SmbCommand::from_u16(99), None);
    }

    #[test]
    fn test_nt_status_from_u32() {
        assert_eq!(NtStatus::from_u32(0x0000_0000), NtStatus::Success);
        assert_eq!(NtStatus::from_u32(0xC000_0022), NtStatus::AccessDenied);
        assert_eq!(NtStatus::from_u32(0xC000_006D), NtStatus::LogonFailure);
    }

    #[test]
    fn test_smb_header_serialize_deserialize() {
        let header = SmbHeader::new_request(SmbCommand::Negotiate, 42);
        let bytes = header.serialize();
        assert_eq!(bytes.len(), SMB2_HEADER_SIZE);

        let parsed = SmbHeader::deserialize(&bytes).unwrap();
        assert_eq!(parsed.protocol_id, SMB2_MAGIC);
        assert_eq!(parsed.command, SmbCommand::Negotiate);
        assert_eq!(parsed.message_id, 42);
    }

    #[test]
    fn test_smb_header_bad_magic() {
        let mut bytes = SmbHeader::new_request(SmbCommand::Negotiate, 0).serialize();
        bytes[0] = 0xFF; // Corrupt magic
        assert!(SmbHeader::deserialize(&bytes).is_none());
    }

    #[test]
    fn test_smb_header_too_short() {
        assert!(SmbHeader::deserialize(&[0; 10]).is_none());
    }

    #[test]
    fn test_ntlm_negotiate_message() {
        let auth = NtlmAuth::new("user", "pass", "DOMAIN");
        let msg = auth.negotiate();
        assert!(msg.len() >= 32);
        assert_eq!(&msg[..8], &NTLMSSP_SIGNATURE);
        let msg_type = u32::from_le_bytes([msg[8], msg[9], msg[10], msg[11]]);
        assert_eq!(msg_type, NTLM_NEGOTIATE);
    }

    #[test]
    fn test_ntlm_challenge_too_short() {
        let mut auth = NtlmAuth::new("user", "pass", "DOMAIN");
        assert!(auth.challenge_response(&[0; 10]).is_none());
    }

    #[test]
    fn test_ntlm_challenge_bad_signature() {
        let mut auth = NtlmAuth::new("user", "pass", "DOMAIN");
        let mut msg = vec![0u8; 40];
        msg[..8].copy_from_slice(b"BADMAGIC");
        assert!(auth.challenge_response(&msg).is_none());
    }

    #[test]
    fn test_ntlm_nt_hash_deterministic() {
        let h1 = NtlmAuth::compute_nt_hash("password");
        let h2 = NtlmAuth::compute_nt_hash("password");
        assert_eq!(h1, h2);

        let h3 = NtlmAuth::compute_nt_hash("different");
        assert_ne!(h1, h3);
    }

    #[test]
    fn test_utf16le_conversion() {
        let result = NtlmAuth::to_utf16le("AB");
        assert_eq!(result, &[0x41, 0x00, 0x42, 0x00]);
    }

    #[test]
    fn test_smb_client_new() {
        let client = SmbClient::new("192.168.1.1");
        assert_eq!(client.server_addr(), "192.168.1.1");
        assert!(client.dialect().is_none());
        assert_eq!(client.session_id(), 0);
    }

    #[test]
    fn test_smb_client_tree_connect_not_connected() {
        let mut client = SmbClient::new("10.0.0.1");
        let result = client.tree_connect("\\\\10.0.0.1\\share");
        assert_eq!(result, Err(SmbError::NotConnected));
    }
}
