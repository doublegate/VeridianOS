//! TPM 2.0 Command Structures
//!
//! TPM command and response packet formats per TPM 2.0 specification.
//!
//! ## TPM 2.0 Command Format
//!
//! All TPM commands follow this structure:
//! ```text
//! +-------------------+
//! | Tag (2 bytes)     |  TPM_ST_SESSIONS or TPM_ST_NO_SESSIONS
//! +-------------------+
//! | Size (4 bytes)    |  Total packet size
//! +-------------------+
//! | Command (4 bytes) |  TPM_CC_* command code
//! +-------------------+
//! | Parameters        |  Command-specific
//! +-------------------+
//! ```

use alloc::{vec, vec::Vec};

/// TPM Structure Tags
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TpmStructureTag {
    /// No sessions in command/response
    NoSessions = 0x8001,
    /// Command/response has sessions
    Sessions = 0x8002,
}

/// TPM Command Codes (partial list)
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TpmCommandCode {
    Startup = 0x00000144,
    Shutdown = 0x00000145,
    SelfTest = 0x00000143,
    GetCapability = 0x0000017A,
    GetRandom = 0x0000017B,
    PcrRead = 0x0000017E,
    PcrExtend = 0x00000182,
    Create = 0x00000153,
    Load = 0x00000157,
    Sign = 0x0000015D,
    VerifySignature = 0x00000177,
    Quote = 0x00000158,
    CreatePrimary = 0x00000131,
}

/// TPM Response Codes
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TpmResponseCode {
    Success = 0x00000000,
    Failure = 0x00000101,
    BadTag = 0x0000001E,
    Retry = 0x00000922,
    Yielded = 0x00000908,
    Canceled = 0x00000909,
}

impl TpmResponseCode {
    pub fn from_u32(value: u32) -> Self {
        match value {
            0x00000000 => TpmResponseCode::Success,
            0x00000101 => TpmResponseCode::Failure,
            0x0000001E => TpmResponseCode::BadTag,
            0x00000922 => TpmResponseCode::Retry,
            0x00000908 => TpmResponseCode::Yielded,
            0x00000909 => TpmResponseCode::Canceled,
            _ => TpmResponseCode::Failure,
        }
    }

    pub fn is_success(&self) -> bool {
        matches!(self, TpmResponseCode::Success)
    }
}

/// TPM Startup Types
#[repr(u16)]
#[derive(Debug, Clone, Copy)]
pub enum TpmStartupType {
    Clear = 0x0000,
    State = 0x0001,
}

/// TPM Command Header
#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct TpmCommandHeader {
    pub tag: u16,     // TpmStructureTag
    pub size: u32,    // Total command size in bytes
    pub command: u32, // TpmCommandCode
}

impl TpmCommandHeader {
    pub fn new(tag: TpmStructureTag, command: TpmCommandCode, size: u32) -> Self {
        Self {
            tag: tag as u16,
            size: size.to_be(),
            command: (command as u32).to_be(),
        }
    }
}

/// TPM Response Header
#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct TpmResponseHeader {
    pub tag: u16,
    pub size: u32,
    pub response_code: u32,
}

impl TpmResponseHeader {
    pub fn parse(data: &[u8]) -> Option<Self> {
        if data.len() < 10 {
            return None;
        }

        Some(Self {
            tag: u16::from_be_bytes([data[0], data[1]]),
            size: u32::from_be_bytes([data[2], data[3], data[4], data[5]]),
            response_code: u32::from_be_bytes([data[6], data[7], data[8], data[9]]),
        })
    }

    pub fn response_code(&self) -> TpmResponseCode {
        TpmResponseCode::from_u32(u32::from_be(self.response_code))
    }
}

/// TPM_Startup Command
pub struct TpmStartupCommand {
    startup_type: TpmStartupType,
}

impl TpmStartupCommand {
    pub fn new(startup_type: TpmStartupType) -> Self {
        Self { startup_type }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();

        // Header (10 bytes)
        let header = TpmCommandHeader::new(
            TpmStructureTag::NoSessions,
            TpmCommandCode::Startup,
            12, // 10 (header) + 2 (startup type)
        );

        bytes.extend_from_slice(&header.tag.to_be_bytes());
        bytes.extend_from_slice(&header.size.to_be_bytes());
        bytes.extend_from_slice(&header.command.to_be_bytes());

        // Startup type (2 bytes)
        bytes.extend_from_slice(&(self.startup_type as u16).to_be_bytes());

        bytes
    }
}

/// TPM_GetRandom Command
pub struct TpmGetRandomCommand {
    bytes_requested: u16,
}

impl TpmGetRandomCommand {
    pub fn new(bytes_requested: u16) -> Self {
        Self { bytes_requested }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();

        // Header
        let header = TpmCommandHeader::new(
            TpmStructureTag::NoSessions,
            TpmCommandCode::GetRandom,
            12, // 10 (header) + 2 (bytes requested)
        );

        bytes.extend_from_slice(&header.tag.to_be_bytes());
        bytes.extend_from_slice(&header.size.to_be_bytes());
        bytes.extend_from_slice(&header.command.to_be_bytes());

        // Bytes requested
        bytes.extend_from_slice(&self.bytes_requested.to_be_bytes());

        bytes
    }
}

/// TPM_GetRandom Response
pub struct TpmGetRandomResponse {
    pub random_bytes: Vec<u8>,
}

impl TpmGetRandomResponse {
    pub fn parse(data: &[u8]) -> Result<Self, &'static str> {
        if data.len() < 12 {
            // 10 (header) + 2 (size)
            return Err("Response too short");
        }

        let header = TpmResponseHeader::parse(data).ok_or("Invalid response header")?;

        if !header.response_code().is_success() {
            return Err("TPM command failed");
        }

        // Parse random bytes size
        let bytes_len = u16::from_be_bytes([data[10], data[11]]) as usize;

        if data.len() < 12 + bytes_len {
            return Err("Invalid random bytes length");
        }

        let random_bytes = data[12..12 + bytes_len].to_vec();

        Ok(Self { random_bytes })
    }
}

/// TPM_PCR_Read Command
pub struct TpmPcrReadCommand {
    pcr_selection: PcrSelection,
}

#[derive(Debug, Clone)]
pub struct PcrSelection {
    pub hash_alg: u16,       // TPM_ALG_* (e.g., SHA256 = 0x000B)
    pub pcr_bitmap: Vec<u8>, // Bitmap of selected PCRs
}

impl TpmPcrReadCommand {
    pub fn new(hash_alg: u16, pcr_indices: &[u8]) -> Self {
        // Create bitmap from PCR indices
        let mut bitmap = vec![0u8; 3]; // Support up to 24 PCRs

        for &pcr in pcr_indices {
            if (pcr as usize) < 24 {
                let byte_idx = (pcr / 8) as usize;
                let bit_idx = pcr % 8;
                bitmap[byte_idx] |= 1 << bit_idx;
            }
        }

        Self {
            pcr_selection: PcrSelection {
                hash_alg,
                pcr_bitmap: bitmap,
            },
        }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();

        // Header
        let size = 10 + 4 + 2 + 1 + self.pcr_selection.pcr_bitmap.len();
        let header = TpmCommandHeader::new(
            TpmStructureTag::NoSessions,
            TpmCommandCode::PcrRead,
            size as u32,
        );

        bytes.extend_from_slice(&header.tag.to_be_bytes());
        bytes.extend_from_slice(&header.size.to_be_bytes());
        bytes.extend_from_slice(&header.command.to_be_bytes());

        // PCR selection count (1)
        bytes.extend_from_slice(&1u32.to_be_bytes());

        // Hash algorithm
        bytes.extend_from_slice(&self.pcr_selection.hash_alg.to_be_bytes());

        // Size of select
        bytes.push(self.pcr_selection.pcr_bitmap.len() as u8);

        // PCR bitmap
        bytes.extend_from_slice(&self.pcr_selection.pcr_bitmap);

        bytes
    }
}

/// TPM Hash Algorithms
pub mod hash_alg {
    pub const SHA1: u16 = 0x0004;
    pub const SHA256: u16 = 0x000B;
    pub const SHA384: u16 = 0x000C;
    pub const SHA512: u16 = 0x000D;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test_case]
    fn test_startup_command() {
        let cmd = TpmStartupCommand::new(TpmStartupType::Clear);
        let bytes = cmd.to_bytes();

        assert_eq!(bytes.len(), 12);
        assert_eq!(u16::from_be_bytes([bytes[0], bytes[1]]), 0x8001); // NoSessions
    }

    #[test_case]
    fn test_get_random_command() {
        let cmd = TpmGetRandomCommand::new(32);
        let bytes = cmd.to_bytes();

        assert_eq!(bytes.len(), 12);
        assert_eq!(u16::from_be_bytes([bytes[10], bytes[11]]), 32);
    }

    #[test_case]
    fn test_response_header_parsing() {
        let data = [
            0x80, 0x01, // Tag
            0x00, 0x00, 0x00, 0x0A, // Size = 10
            0x00, 0x00, 0x00, 0x00, // Success
        ];

        let header = TpmResponseHeader::parse(&data).unwrap();
        assert!(header.response_code().is_success());
    }
}
