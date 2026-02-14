//! TPM 2.0 Command Structures
//!
//! TPM command and response packet formats per TPM 2.0 specification (Part 3).
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
//!
//! ## Supported Commands
//!
//! - `TPM2_Startup` / `TPM2_Shutdown` -- lifecycle
//! - `TPM2_GetRandom` -- hardware RNG
//! - `TPM2_PCR_Read` / `TPM2_PCR_Extend` -- measured boot
//! - `TPM2_SelfTest` -- POST diagnostics
//! - `TPM2_GetCapability` -- feature query

use alloc::{vec, vec::Vec};

use crate::error::KernelError;

/// TPM Structure Tags (TPM_ST)
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TpmStructureTag {
    /// No sessions in command/response
    NoSessions = 0x8001,
    /// Command/response has sessions
    Sessions = 0x8002,
}

/// TPM Command Codes (TPM_CC, partial list)
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

/// TPM Response Codes (TPM_RC)
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

/// TPM Startup Types (TPM_SU)
#[repr(u16)]
#[derive(Debug, Clone, Copy)]
pub enum TpmStartupType {
    /// TPM2_Startup(CLEAR) -- reset all PCRs, clear state
    Clear = 0x0000,
    /// TPM2_Startup(STATE) -- restore saved state
    State = 0x0001,
}

/// TPM Shutdown Types (TPM_SU)
#[repr(u16)]
#[derive(Debug, Clone, Copy)]
pub enum TpmShutdownType {
    /// TPM2_Shutdown(CLEAR) -- discard state
    Clear = 0x0000,
    /// TPM2_Shutdown(STATE) -- save state for resume
    State = 0x0001,
}

// ============================================================================
// Marshaling helpers
// ============================================================================

/// Marshal a command header into big-endian bytes.
///
/// The header is always 10 bytes: tag(2) + size(4) + command_code(4).
fn marshal_header(tag: TpmStructureTag, command: TpmCommandCode, total_size: u32) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(10);
    bytes.extend_from_slice(&(tag as u16).to_be_bytes());
    bytes.extend_from_slice(&total_size.to_be_bytes());
    bytes.extend_from_slice(&(command as u32).to_be_bytes());
    bytes
}

/// Marshal a generic TPM command into a byte buffer.
///
/// Takes the command code, tag, and parameter bytes and produces a complete
/// command packet with the correct header.
pub fn marshal_command(tag: TpmStructureTag, command: TpmCommandCode, params: &[u8]) -> Vec<u8> {
    let total_size = (10 + params.len()) as u32;
    let mut bytes = marshal_header(tag, command, total_size);
    bytes.extend_from_slice(params);
    bytes
}

/// Parse a TPM response buffer and extract the response code and payload.
///
/// Returns `(response_code, payload)` where payload is everything after the
/// 10-byte header. Returns `None` if the buffer is too short.
pub fn parse_response(data: &[u8]) -> Option<(TpmResponseCode, &[u8])> {
    let header = TpmResponseHeader::parse(data)?;
    let total_size = header.size as usize;

    if data.len() < total_size || total_size < 10 {
        return None;
    }

    let code = header.response_code();
    let payload = &data[10..total_size];
    Some((code, payload))
}

// ============================================================================
// Header structures
// ============================================================================

/// TPM Command Header (10 bytes, big-endian on wire)
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

/// TPM Response Header (10 bytes, big-endian on wire)
#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct TpmResponseHeader {
    pub tag: u16,
    pub size: u32,
    pub response_code: u32,
}

impl TpmResponseHeader {
    /// Parse a response header from a byte slice (at least 10 bytes).
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

    /// Decode the response code enum from the raw field.
    pub fn response_code(&self) -> TpmResponseCode {
        // The field was stored in big-endian by parse(), so it is already host-order.
        TpmResponseCode::from_u32(self.response_code)
    }
}

// ============================================================================
// TPM2_Startup
// ============================================================================

/// TPM2_Startup command
pub struct TpmStartupCommand {
    startup_type: TpmStartupType,
}

impl TpmStartupCommand {
    pub fn new(startup_type: TpmStartupType) -> Self {
        Self { startup_type }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let params = (self.startup_type as u16).to_be_bytes();
        marshal_command(
            TpmStructureTag::NoSessions,
            TpmCommandCode::Startup,
            &params,
        )
    }
}

// ============================================================================
// TPM2_Shutdown
// ============================================================================

/// TPM2_Shutdown command
pub struct TpmShutdownCommand {
    shutdown_type: TpmShutdownType,
}

impl TpmShutdownCommand {
    pub fn new(shutdown_type: TpmShutdownType) -> Self {
        Self { shutdown_type }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let params = (self.shutdown_type as u16).to_be_bytes();
        marshal_command(
            TpmStructureTag::NoSessions,
            TpmCommandCode::Shutdown,
            &params,
        )
    }
}

// ============================================================================
// TPM2_SelfTest
// ============================================================================

/// TPM2_SelfTest command
pub struct TpmSelfTestCommand {
    /// If true, run full self-test; if false, incremental only
    full_test: bool,
}

impl TpmSelfTestCommand {
    pub fn new(full_test: bool) -> Self {
        Self { full_test }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let params = [if self.full_test { 1u8 } else { 0u8 }];
        marshal_command(
            TpmStructureTag::NoSessions,
            TpmCommandCode::SelfTest,
            &params,
        )
    }
}

// ============================================================================
// TPM2_GetRandom
// ============================================================================

/// TPM2_GetRandom command
pub struct TpmGetRandomCommand {
    bytes_requested: u16,
}

impl TpmGetRandomCommand {
    pub fn new(bytes_requested: u16) -> Self {
        Self { bytes_requested }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let params = self.bytes_requested.to_be_bytes();
        marshal_command(
            TpmStructureTag::NoSessions,
            TpmCommandCode::GetRandom,
            &params,
        )
    }
}

/// TPM2_GetRandom response parser
pub struct TpmGetRandomResponse {
    pub random_bytes: Vec<u8>,
}

impl TpmGetRandomResponse {
    /// Parse a TPM2_GetRandom response.
    ///
    /// Format: header(10) + randomBytesCount(2) + randomBytes(N)
    pub fn parse(data: &[u8]) -> Result<Self, KernelError> {
        if data.len() < 12 {
            return Err(KernelError::InvalidArgument {
                name: "tpm_response",
                value: "response too short",
            });
        }

        let header = TpmResponseHeader::parse(data).ok_or(KernelError::InvalidArgument {
            name: "tpm_response",
            value: "invalid response header",
        })?;

        if !header.response_code().is_success() {
            return Err(KernelError::HardwareError {
                device: "TPM",
                code: header.response_code() as u32,
            });
        }

        let bytes_len = u16::from_be_bytes([data[10], data[11]]) as usize;

        if data.len() < 12 + bytes_len {
            return Err(KernelError::InvalidArgument {
                name: "tpm_response",
                value: "invalid random bytes length",
            });
        }

        let random_bytes = data[12..12 + bytes_len].to_vec();

        Ok(Self { random_bytes })
    }
}

// ============================================================================
// TPM2_PCR_Read
// ============================================================================

/// PCR selection structure (TPMS_PCR_SELECTION)
#[derive(Debug, Clone)]
pub struct PcrSelection {
    /// Hash algorithm (TPM_ALG_*), e.g., SHA256 = 0x000B
    pub hash_alg: u16,
    /// Bitmap of selected PCRs (3 bytes = up to 24 PCRs)
    pub pcr_bitmap: Vec<u8>,
}

/// TPM2_PCR_Read command
pub struct TpmPcrReadCommand {
    pcr_selection: PcrSelection,
}

impl TpmPcrReadCommand {
    /// Create a PCR_Read command for the given hash algorithm and PCR indices.
    pub fn new(hash_alg: u16, pcr_indices: &[u8]) -> Self {
        let mut bitmap = vec![0u8; 3];

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
        // Parameters: pcrSelectionIn (TPML_PCR_SELECTION)
        //   count(4) + [ hashAlg(2) + sizeOfSelect(1) + pcrSelect(N) ]
        let mut params = Vec::new();

        // Count of selections (1)
        params.extend_from_slice(&1u32.to_be_bytes());

        // Hash algorithm
        params.extend_from_slice(&self.pcr_selection.hash_alg.to_be_bytes());

        // Size of select bitmap
        params.push(self.pcr_selection.pcr_bitmap.len() as u8);

        // PCR bitmap
        params.extend_from_slice(&self.pcr_selection.pcr_bitmap);

        marshal_command(
            TpmStructureTag::NoSessions,
            TpmCommandCode::PcrRead,
            &params,
        )
    }
}

/// TPM2_PCR_Read response parser
pub struct TpmPcrReadResponse {
    /// PCR update counter
    pub pcr_update_counter: u32,
    /// The PCR digest values returned
    pub pcr_values: Vec<Vec<u8>>,
}

impl TpmPcrReadResponse {
    /// Parse a TPM2_PCR_Read response.
    ///
    /// Format: header(10) + pcrUpdateCounter(4) + pcrSelectionOut(var) +
    ///         pcrValues: count(4) + [ size(2) + digest(N) ]*
    pub fn parse(data: &[u8]) -> Result<Self, KernelError> {
        if data.len() < 14 {
            return Err(KernelError::InvalidArgument {
                name: "tpm_pcr_response",
                value: "response too short for PCR_Read",
            });
        }

        let header = TpmResponseHeader::parse(data).ok_or(KernelError::InvalidArgument {
            name: "tpm_pcr_response",
            value: "invalid response header",
        })?;
        if !header.response_code().is_success() {
            return Err(KernelError::HardwareError {
                device: "TPM",
                code: header.response_code() as u32,
            });
        }

        let pcr_update_counter = u32::from_be_bytes([data[10], data[11], data[12], data[13]]);

        // Skip pcrSelectionOut: count(4) + [ hashAlg(2) + sizeOfSelect(1) + select(N) ]
        let mut offset = 14;
        if data.len() < offset + 4 {
            return Err(KernelError::InvalidArgument {
                name: "tpm_pcr_response",
                value: "response too short for selection count",
            });
        }
        let sel_count = u32::from_be_bytes([
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
        ]) as usize;
        offset += 4;

        for _ in 0..sel_count {
            if data.len() < offset + 3 {
                return Err(KernelError::InvalidArgument {
                    name: "tpm_pcr_response",
                    value: "response too short for selection entry",
                });
            }
            offset += 2; // hashAlg
            let select_size = data[offset] as usize;
            offset += 1 + select_size;
        }

        // Parse pcrValues (TPML_DIGEST)
        if data.len() < offset + 4 {
            return Err(KernelError::InvalidArgument {
                name: "tpm_pcr_response",
                value: "response too short for digest count",
            });
        }
        let digest_count = u32::from_be_bytes([
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
        ]) as usize;
        offset += 4;

        let mut pcr_values = Vec::with_capacity(digest_count);
        for _ in 0..digest_count {
            if data.len() < offset + 2 {
                return Err(KernelError::InvalidArgument {
                    name: "tpm_pcr_response",
                    value: "response too short for digest size",
                });
            }
            let digest_size = u16::from_be_bytes([data[offset], data[offset + 1]]) as usize;
            offset += 2;
            if data.len() < offset + digest_size {
                return Err(KernelError::InvalidArgument {
                    name: "tpm_pcr_response",
                    value: "response too short for digest data",
                });
            }
            pcr_values.push(data[offset..offset + digest_size].to_vec());
            offset += digest_size;
        }

        Ok(Self {
            pcr_update_counter,
            pcr_values,
        })
    }
}

// ============================================================================
// TPM2_PCR_Extend
// ============================================================================

/// TPM2_PCR_Extend command
///
/// Extends a PCR with a SHA-256 measurement digest.
/// This command requires a session (TPM_ST_SESSIONS) because the PCR handle
/// is an authorization handle.
pub struct TpmPcrExtendCommand {
    /// PCR index to extend (0-23)
    pcr_index: u8,
    /// SHA-256 digest to extend into the PCR
    digest: [u8; 32],
}

impl TpmPcrExtendCommand {
    pub fn new(pcr_index: u8, digest: &[u8; 32]) -> Self {
        Self {
            pcr_index,
            digest: *digest,
        }
    }

    /// Marshal the TPM2_PCR_Extend command.
    ///
    /// Wire format (TPM_ST_SESSIONS because pcrHandle is an auth handle):
    ///   header(10) + pcrHandle(4) + authSize(4) + authSession(9) +
    ///   digests: count(4) + [ hashAlg(2) + digest(32) ]
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut params = Vec::new();

        // pcrHandle (4 bytes) -- PCR handles are 0x00000000 + index
        let pcr_handle: u32 = self.pcr_index as u32;
        params.extend_from_slice(&pcr_handle.to_be_bytes());

        // Authorization area (for TPM_ST_SESSIONS)
        // Minimal password session (TPM_RS_PW):
        //   authorizationSize(4) = 9 (the size of the session block below)
        //   sessionHandle(4) = TPM_RS_PW = 0x40000009
        //   nonceCaller(2) = size(2)=0
        //   sessionAttributes(1) = 0x01 (continueSession)
        //   hmac(2) = size(2)=0
        let auth_session_size: u32 = 9; // 4 + 2 + 1 + 2
        params.extend_from_slice(&auth_session_size.to_be_bytes());

        // sessionHandle = TPM_RS_PW
        params.extend_from_slice(&0x40000009u32.to_be_bytes());
        // nonceCaller = empty (size = 0)
        params.extend_from_slice(&0u16.to_be_bytes());
        // sessionAttributes = continueSession
        params.push(0x01);
        // hmac = empty (size = 0)
        params.extend_from_slice(&0u16.to_be_bytes());

        // TPML_DIGEST_VALUES: count(4) + [ TPMT_HA: hashAlg(2) + digest ]
        let digest_count: u32 = 1; // One digest (SHA-256)
        params.extend_from_slice(&digest_count.to_be_bytes());

        // hashAlg = SHA-256
        params.extend_from_slice(&super::tpm_commands::hash_alg::SHA256.to_be_bytes());

        // digest (32 bytes)
        params.extend_from_slice(&self.digest);

        marshal_command(
            TpmStructureTag::Sessions,
            TpmCommandCode::PcrExtend,
            &params,
        )
    }
}

// ============================================================================
// TPM2_GetCapability
// ============================================================================

/// TPM2_GetCapability command for querying TPM properties.
pub struct TpmGetCapabilityCommand {
    /// Capability group (e.g., TPM_CAP_TPM_PROPERTIES = 0x00000006)
    capability: u32,
    /// Property within the group
    property: u32,
    /// Maximum number of properties to return
    property_count: u32,
}

impl TpmGetCapabilityCommand {
    pub fn new(capability: u32, property: u32, property_count: u32) -> Self {
        Self {
            capability,
            property,
            property_count,
        }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut params = Vec::new();
        params.extend_from_slice(&self.capability.to_be_bytes());
        params.extend_from_slice(&self.property.to_be_bytes());
        params.extend_from_slice(&self.property_count.to_be_bytes());

        marshal_command(
            TpmStructureTag::NoSessions,
            TpmCommandCode::GetCapability,
            &params,
        )
    }
}

/// Well-known capability constants
pub mod capability {
    /// Query TPM properties
    pub const TPM_CAP_TPM_PROPERTIES: u32 = 0x00000006;
    /// Query supported algorithms
    pub const TPM_CAP_ALGS: u32 = 0x00000000;
    /// Query PCR properties
    pub const TPM_CAP_PCRS: u32 = 0x00000005;

    /// Property: TPM family indicator
    pub const PT_FAMILY_INDICATOR: u32 = 0x00000100;
    /// Property: TPM firmware version 1
    pub const PT_FIRMWARE_VERSION_1: u32 = 0x00000111;
    /// Property: TPM firmware version 2
    pub const PT_FIRMWARE_VERSION_2: u32 = 0x00000112;
    /// Property: manufacturer
    pub const PT_MANUFACTURER: u32 = 0x00000105;
}

/// TPM Hash Algorithms (TPM_ALG_*)
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
                                                                      // Size = 12
        assert_eq!(
            u32::from_be_bytes([bytes[2], bytes[3], bytes[4], bytes[5]]),
            12
        );
        // Command = Startup (0x144)
        assert_eq!(
            u32::from_be_bytes([bytes[6], bytes[7], bytes[8], bytes[9]]),
            0x144
        );
        // StartupType = Clear (0x0000)
        assert_eq!(u16::from_be_bytes([bytes[10], bytes[11]]), 0x0000);
    }

    #[test_case]
    fn test_shutdown_command() {
        let cmd = TpmShutdownCommand::new(TpmShutdownType::State);
        let bytes = cmd.to_bytes();

        assert_eq!(bytes.len(), 12);
        // Command = Shutdown (0x145)
        assert_eq!(
            u32::from_be_bytes([bytes[6], bytes[7], bytes[8], bytes[9]]),
            0x145
        );
        // ShutdownType = State (0x0001)
        assert_eq!(u16::from_be_bytes([bytes[10], bytes[11]]), 0x0001);
    }

    #[test_case]
    fn test_get_random_command() {
        let cmd = TpmGetRandomCommand::new(32);
        let bytes = cmd.to_bytes();

        assert_eq!(bytes.len(), 12);
        assert_eq!(u16::from_be_bytes([bytes[10], bytes[11]]), 32);
    }

    #[test_case]
    fn test_pcr_extend_command() {
        let digest = [0xABu8; 32];
        let cmd = TpmPcrExtendCommand::new(7, &digest);
        let bytes = cmd.to_bytes();

        // Tag should be Sessions (0x8002) for PCR_Extend
        assert_eq!(u16::from_be_bytes([bytes[0], bytes[1]]), 0x8002);

        // Command code = PCR_Extend (0x182)
        assert_eq!(
            u32::from_be_bytes([bytes[6], bytes[7], bytes[8], bytes[9]]),
            0x182
        );

        // PCR handle = 7
        assert_eq!(
            u32::from_be_bytes([bytes[10], bytes[11], bytes[12], bytes[13]]),
            7
        );
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

    #[test_case]
    fn test_marshal_command_generic() {
        let params = [0x01, 0x02, 0x03, 0x04];
        let bytes = marshal_command(
            TpmStructureTag::NoSessions,
            TpmCommandCode::GetCapability,
            &params,
        );

        assert_eq!(bytes.len(), 14); // 10 header + 4 params
        assert_eq!(
            u32::from_be_bytes([bytes[2], bytes[3], bytes[4], bytes[5]]),
            14 // total size
        );
    }

    #[test_case]
    fn test_parse_response() {
        let data = [
            0x80, 0x01, // Tag
            0x00, 0x00, 0x00, 0x0E, // Size = 14
            0x00, 0x00, 0x00, 0x00, // Success
            0xDE, 0xAD, 0xBE, 0xEF, // payload
        ];

        let (code, payload) = parse_response(&data).unwrap();
        assert!(code.is_success());
        assert_eq!(payload, &[0xDE, 0xAD, 0xBE, 0xEF]);
    }

    #[test_case]
    fn test_get_random_response_parse() {
        let data = [
            0x80, 0x01, // Tag
            0x00, 0x00, 0x00, 0x10, // Size = 16
            0x00, 0x00, 0x00, 0x00, // Success
            0x00, 0x04, // 4 random bytes
            0xDE, 0xAD, 0xBE, 0xEF, // random data
        ];

        let response = TpmGetRandomResponse::parse(&data).unwrap();
        assert_eq!(response.random_bytes, vec![0xDE, 0xAD, 0xBE, 0xEF]);
    }

    #[test_case]
    fn test_self_test_command() {
        let cmd = TpmSelfTestCommand::new(true);
        let bytes = cmd.to_bytes();

        assert_eq!(bytes.len(), 11); // 10 header + 1 byte (fullTest)
        assert_eq!(
            u32::from_be_bytes([bytes[6], bytes[7], bytes[8], bytes[9]]),
            0x143 // SelfTest
        );
        assert_eq!(bytes[10], 1); // fullTest = true
    }
}
