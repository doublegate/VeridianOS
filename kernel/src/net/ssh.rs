//! SSH-2.0 Server Implementation (RFC 4253, RFC 4252, RFC 4254)
//!
//! Provides an SSH server for VeridianOS with:
//! - SSH-2.0 binary packet protocol and version exchange
//! - Key exchange via curve25519-sha256 (ECDH)
//! - Password and Ed25519 public key authentication
//! - Channel multiplexing with flow control (window adjust)
//! - PTY allocation, shell/exec sessions, env passing
//! - Session state machine from version exchange through disconnect

#![allow(dead_code)]

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::{collections::BTreeMap, string::String, vec, vec::Vec};

// ============================================================================
// SSH Message Type Constants (RFC 4253 / 4252 / 4254)
// ============================================================================

pub const SSH_MSG_DISCONNECT: u8 = 1;
pub const SSH_MSG_IGNORE: u8 = 2;
pub const SSH_MSG_UNIMPLEMENTED: u8 = 3;
pub const SSH_MSG_DEBUG: u8 = 4;
pub const SSH_MSG_SERVICE_REQUEST: u8 = 5;
pub const SSH_MSG_SERVICE_ACCEPT: u8 = 6;

pub const SSH_MSG_KEXINIT: u8 = 20;
pub const SSH_MSG_NEWKEYS: u8 = 21;

pub const SSH_MSG_KEX_ECDH_INIT: u8 = 30;
pub const SSH_MSG_KEX_ECDH_REPLY: u8 = 31;

pub const SSH_MSG_USERAUTH_REQUEST: u8 = 50;
pub const SSH_MSG_USERAUTH_FAILURE: u8 = 51;
pub const SSH_MSG_USERAUTH_SUCCESS: u8 = 52;
pub const SSH_MSG_USERAUTH_BANNER: u8 = 53;
pub const SSH_MSG_USERAUTH_PK_OK: u8 = 60;

pub const SSH_MSG_GLOBAL_REQUEST: u8 = 80;
pub const SSH_MSG_REQUEST_SUCCESS: u8 = 81;
pub const SSH_MSG_REQUEST_FAILURE: u8 = 82;

pub const SSH_MSG_CHANNEL_OPEN: u8 = 90;
pub const SSH_MSG_CHANNEL_OPEN_CONFIRMATION: u8 = 91;
pub const SSH_MSG_CHANNEL_OPEN_FAILURE: u8 = 92;
pub const SSH_MSG_CHANNEL_WINDOW_ADJUST: u8 = 93;
pub const SSH_MSG_CHANNEL_DATA: u8 = 94;
pub const SSH_MSG_CHANNEL_EXTENDED_DATA: u8 = 95;
pub const SSH_MSG_CHANNEL_EOF: u8 = 96;
pub const SSH_MSG_CHANNEL_CLOSE: u8 = 97;
pub const SSH_MSG_CHANNEL_REQUEST: u8 = 98;
pub const SSH_MSG_CHANNEL_SUCCESS: u8 = 99;
pub const SSH_MSG_CHANNEL_FAILURE: u8 = 100;

// ============================================================================
// SSH Disconnect Reason Codes (RFC 4253 Section 11.1)
// ============================================================================

pub const SSH_DISCONNECT_HOST_NOT_ALLOWED: u32 = 1;
pub const SSH_DISCONNECT_PROTOCOL_ERROR: u32 = 2;
pub const SSH_DISCONNECT_KEY_EXCHANGE_FAILED: u32 = 3;
pub const SSH_DISCONNECT_RESERVED: u32 = 4;
pub const SSH_DISCONNECT_MAC_ERROR: u32 = 5;
pub const SSH_DISCONNECT_COMPRESSION_ERROR: u32 = 6;
pub const SSH_DISCONNECT_SERVICE_NOT_AVAILABLE: u32 = 7;
pub const SSH_DISCONNECT_PROTOCOL_VERSION_NOT_SUPPORTED: u32 = 8;
pub const SSH_DISCONNECT_HOST_KEY_NOT_VERIFIABLE: u32 = 9;
pub const SSH_DISCONNECT_CONNECTION_LOST: u32 = 10;
pub const SSH_DISCONNECT_BY_APPLICATION: u32 = 11;
pub const SSH_DISCONNECT_TOO_MANY_CONNECTIONS: u32 = 12;
pub const SSH_DISCONNECT_AUTH_CANCELLED_BY_USER: u32 = 13;
pub const SSH_DISCONNECT_NO_MORE_AUTH_METHODS: u32 = 14;
pub const SSH_DISCONNECT_ILLEGAL_USER_NAME: u32 = 15;

// ============================================================================
// SSH Channel Open Failure Codes (RFC 4254 Section 5.1)
// ============================================================================

pub const SSH_OPEN_ADMINISTRATIVELY_PROHIBITED: u32 = 1;
pub const SSH_OPEN_CONNECT_FAILED: u32 = 2;
pub const SSH_OPEN_UNKNOWN_CHANNEL_TYPE: u32 = 3;
pub const SSH_OPEN_RESOURCE_SHORTAGE: u32 = 4;

// ============================================================================
// Protocol Constants
// ============================================================================

/// SSH-2.0 server identification string
pub const SSH_VERSION_STRING: &[u8] = b"SSH-2.0-VeridianOS_1.0\r\n";

/// Maximum SSH packet payload size (256 KB)
const MAX_PACKET_SIZE: usize = 262144;

/// Minimum padding length
const MIN_PADDING: usize = 4;

/// Block size for unencrypted packets
const BLOCK_SIZE_CLEAR: usize = 8;

/// Default initial channel window size (2 MB)
const DEFAULT_WINDOW_SIZE: u32 = 2 * 1024 * 1024;

/// Maximum channel packet data size (32 KB)
const MAX_CHANNEL_DATA_SIZE: u32 = 32768;

/// Default SSH listen port
pub const SSH_DEFAULT_PORT: u16 = 22;

/// Maximum concurrent sessions per server
const MAX_SESSIONS: usize = 64;

/// Maximum authentication attempts before disconnect
const MAX_AUTH_ATTEMPTS: u32 = 6;

/// Key length for curve25519
const CURVE25519_KEY_LEN: usize = 32;

/// Ed25519 signature length
const ED25519_SIG_LEN: usize = 64;

/// Ed25519 public key length
const ED25519_PUB_LEN: usize = 32;

/// Maximum version string length (RFC 4253)
const MAX_VERSION_LEN: usize = 255;

/// HMAC-SHA256 tag length
const MAC_LEN: usize = 32;

// ============================================================================
// Section 1: SSH Transport Layer (~300 lines)
// ============================================================================

/// SSH error types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SshError {
    /// Invalid or malformed packet
    InvalidPacket,
    /// Version exchange failed
    VersionMismatch,
    /// Key exchange failure
    KeyExchangeFailed,
    /// Authentication failed
    AuthenticationFailed,
    /// Maximum auth attempts exceeded
    TooManyAuthAttempts,
    /// Channel not found
    ChannelNotFound,
    /// Channel already exists
    ChannelExists,
    /// Window exhausted (flow control)
    WindowExhausted,
    /// Buffer too small
    BufferTooSmall,
    /// Connection closed
    ConnectionClosed,
    /// Invalid state transition
    InvalidState,
    /// Service not available
    ServiceNotAvailable,
    /// Packet too large
    PacketTooLarge,
    /// Invalid MAC
    MacVerifyFailed,
    /// Protocol error
    ProtocolError,
    /// Resource shortage
    ResourceShortage,
    /// Invalid channel type
    InvalidChannelType,
    /// PTY allocation failed
    PtyAllocationFailed,
    /// Session limit reached
    SessionLimitReached,
}

/// SSH server session state machine
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    /// Initial state: waiting for client version string
    VersionExchange,
    /// Key exchange in progress
    KeyExchange,
    /// Key exchange complete, waiting for NEWKEYS
    NewKeysExpected,
    /// Authenticated session, user interaction
    Authentication,
    /// Fully connected and channels active
    Connected,
    /// Session being torn down
    Disconnected,
}

/// Algorithms negotiated during key exchange
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NegotiatedAlgorithms {
    pub kex_algorithm: AlgorithmId,
    pub host_key_algorithm: AlgorithmId,
    pub encryption_c2s: AlgorithmId,
    pub encryption_s2c: AlgorithmId,
    pub mac_c2s: AlgorithmId,
    pub mac_s2c: AlgorithmId,
    pub compression_c2s: AlgorithmId,
    pub compression_s2c: AlgorithmId,
}

impl Default for NegotiatedAlgorithms {
    fn default() -> Self {
        Self::new()
    }
}

impl NegotiatedAlgorithms {
    pub fn new() -> Self {
        Self {
            kex_algorithm: AlgorithmId::Curve25519Sha256,
            host_key_algorithm: AlgorithmId::SshEd25519,
            encryption_c2s: AlgorithmId::Chacha20Poly1305,
            encryption_s2c: AlgorithmId::Chacha20Poly1305,
            mac_c2s: AlgorithmId::HmacSha256,
            mac_s2c: AlgorithmId::HmacSha256,
            compression_c2s: AlgorithmId::None,
            compression_s2c: AlgorithmId::None,
        }
    }
}

/// Supported algorithm identifiers
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlgorithmId {
    Curve25519Sha256,
    SshEd25519,
    Chacha20Poly1305,
    Aes256Ctr,
    HmacSha256,
    None,
}

impl AlgorithmId {
    /// Convert algorithm to its SSH wire name
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Self::Curve25519Sha256 => "curve25519-sha256",
            Self::SshEd25519 => "ssh-ed25519",
            Self::Chacha20Poly1305 => "chacha20-poly1305@openssh.com",
            Self::Aes256Ctr => "aes256-ctr",
            Self::HmacSha256 => "hmac-sha256",
            Self::None => "none",
        }
    }

    /// Parse from SSH wire name
    pub(crate) fn parse_name(s: &str) -> Option<Self> {
        match s {
            "curve25519-sha256" | "curve25519-sha256@libssh.org" => Some(Self::Curve25519Sha256),
            "ssh-ed25519" => Some(Self::SshEd25519),
            "chacha20-poly1305@openssh.com" => Some(Self::Chacha20Poly1305),
            "aes256-ctr" => Some(Self::Aes256Ctr),
            "hmac-sha256" => Some(Self::HmacSha256),
            "none" => Some(Self::None),
            _ => None,
        }
    }

    /// Map this algorithm to a `CipherSuite` from the shared crypto module.
    ///
    /// Returns `None` for algorithms that are not AEAD cipher suites
    /// (key exchange, host key, MAC, compression) or for AES-CTR which
    /// is not yet available as a `CipherSuite` variant.
    pub(crate) fn as_cipher_suite(&self) -> Option<crate::crypto::cipher_suite::CipherSuite> {
        match self {
            Self::Chacha20Poly1305 => {
                Some(crate::crypto::cipher_suite::CipherSuite::ChaCha20Poly1305)
            }
            Self::Aes256Ctr => None, // AES-CTR is not an AEAD cipher suite
            _ => None,
        }
    }
}

/// SSH version information parsed from identification string
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionInfo {
    /// Protocol version (must be "2.0")
    pub protocol_version: [u8; 3],
    /// Software version string
    pub software_version: Vec<u8>,
    /// Optional comment
    pub comment: Vec<u8>,
}

impl VersionInfo {
    /// Parse an SSH identification string (e.g., "SSH-2.0-OpenSSH_9.0\r\n")
    pub(crate) fn parse(data: &[u8]) -> Option<Self> {
        // Must start with "SSH-"
        if data.len() < 8 || &data[..4] != b"SSH-" {
            return None;
        }

        // Find end (strip \r\n or \n)
        let end = data.iter().position(|&b| b == b'\n').unwrap_or(data.len());
        let line = if end > 0 && data[end - 1] == b'\r' {
            &data[..end - 1]
        } else {
            &data[..end]
        };

        if line.len() > MAX_VERSION_LEN {
            return None;
        }

        // Protocol version: "2.0" required
        if line.len() < 7 || &line[4..7] != b"2.0" {
            return None;
        }

        let protocol_version = [b'2', b'.', b'0'];

        // Expect dash after version
        if line.len() < 8 || line[7] != b'-' {
            return None;
        }

        // Software version extends to space or end
        let sw_start = 8;
        let sw_end = line[sw_start..]
            .iter()
            .position(|&b| b == b' ')
            .map(|p| sw_start + p)
            .unwrap_or(line.len());

        let software_version = line[sw_start..sw_end].to_vec();

        let comment = if sw_end < line.len() {
            line[sw_end + 1..].to_vec()
        } else {
            Vec::new()
        };

        Some(Self {
            protocol_version,
            software_version,
            comment,
        })
    }

    /// Encode to wire format (with trailing \r\n)
    pub(crate) fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(64);
        buf.extend_from_slice(b"SSH-");
        buf.extend_from_slice(&self.protocol_version);
        buf.push(b'-');
        buf.extend_from_slice(&self.software_version);
        if !self.comment.is_empty() {
            buf.push(b' ');
            buf.extend_from_slice(&self.comment);
        }
        buf.extend_from_slice(b"\r\n");
        buf
    }
}

/// SSH binary packet (RFC 4253 Section 6)
///
/// Wire format:
///   packet_length (u32) || padding_length (u8) || payload || random_padding ||
/// MAC
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshPacket {
    /// Message payload bytes
    pub payload: Vec<u8>,
    /// Random padding (at least 4 bytes, total packet multiple of block size)
    pub padding: Vec<u8>,
}

impl SshPacket {
    /// Create a new packet from payload
    pub fn new(payload: Vec<u8>) -> Self {
        let padding = Self::compute_padding(payload.len(), BLOCK_SIZE_CLEAR);
        Self { payload, padding }
    }

    /// Compute required padding for the given payload and block size
    fn compute_padding(payload_len: usize, block_size: usize) -> Vec<u8> {
        // packet_length (4) + padding_length (1) + payload + padding must be multiple
        // of block_size padding must be at least MIN_PADDING bytes
        let unpadded = 1 + payload_len; // padding_length byte + payload
        let remainder = (4 + unpadded) % block_size;
        let mut pad_len = if remainder == 0 {
            0
        } else {
            block_size - remainder
        };
        if pad_len < MIN_PADDING {
            pad_len += block_size;
        }
        // Use deterministic padding for reproducibility (real impl would use random)
        vec![0u8; pad_len]
    }

    /// Encode packet to wire format (without MAC)
    pub(crate) fn encode(&self) -> Vec<u8> {
        let padding_length = self.padding.len() as u8;
        let packet_length = (1 + self.payload.len() + self.padding.len()) as u32;
        let total = 4 + packet_length as usize;

        let mut buf = Vec::with_capacity(total);
        buf.extend_from_slice(&packet_length.to_be_bytes());
        buf.push(padding_length);
        buf.extend_from_slice(&self.payload);
        buf.extend_from_slice(&self.padding);
        buf
    }

    /// Decode packet from wire format (without MAC verification)
    pub(crate) fn decode(data: &[u8]) -> Result<(Self, usize), SshError> {
        if data.len() < 5 {
            return Err(SshError::InvalidPacket);
        }

        let packet_length = u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as usize;

        if !(2..=MAX_PACKET_SIZE).contains(&packet_length) {
            return Err(SshError::PacketTooLarge);
        }

        let total = 4 + packet_length;
        if data.len() < total {
            return Err(SshError::InvalidPacket);
        }

        let padding_length = data[4] as usize;
        if padding_length < MIN_PADDING || padding_length >= packet_length {
            return Err(SshError::InvalidPacket);
        }

        let payload_length = packet_length - 1 - padding_length;
        let payload = data[5..5 + payload_length].to_vec();
        let padding = data[5 + payload_length..5 + payload_length + padding_length].to_vec();

        Ok((Self { payload, padding }, total))
    }

    /// Get the message type byte (first byte of payload)
    pub(crate) fn message_type(&self) -> Option<u8> {
        self.payload.first().copied()
    }
}

/// KEXINIT message (SSH_MSG_KEXINIT, type 20)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KexInitMessage {
    /// Random 16-byte cookie
    pub cookie: [u8; 16],
    /// Kex algorithms
    pub kex_algorithms: Vec<u8>,
    /// Server host key algorithms
    pub server_host_key_algorithms: Vec<u8>,
    /// Encryption algorithms client-to-server
    pub encryption_algorithms_c2s: Vec<u8>,
    /// Encryption algorithms server-to-client
    pub encryption_algorithms_s2c: Vec<u8>,
    /// MAC algorithms client-to-server
    pub mac_algorithms_c2s: Vec<u8>,
    /// MAC algorithms server-to-client
    pub mac_algorithms_s2c: Vec<u8>,
    /// Compression algorithms client-to-server
    pub compression_algorithms_c2s: Vec<u8>,
    /// Compression algorithms server-to-client
    pub compression_algorithms_s2c: Vec<u8>,
    /// Languages client-to-server
    pub languages_c2s: Vec<u8>,
    /// Languages server-to-client
    pub languages_s2c: Vec<u8>,
    /// First kex packet follows
    pub first_kex_packet_follows: bool,
}

impl KexInitMessage {
    /// Create a server KEXINIT with default supported algorithms
    pub fn new_server(cookie: [u8; 16]) -> Self {
        Self {
            cookie,
            kex_algorithms: Self::encode_name_list(&["curve25519-sha256"]),
            server_host_key_algorithms: Self::encode_name_list(&["ssh-ed25519"]),
            encryption_algorithms_c2s: Self::encode_name_list(&[
                "chacha20-poly1305@openssh.com",
                "aes256-ctr",
            ]),
            encryption_algorithms_s2c: Self::encode_name_list(&[
                "chacha20-poly1305@openssh.com",
                "aes256-ctr",
            ]),
            mac_algorithms_c2s: Self::encode_name_list(&["hmac-sha256"]),
            mac_algorithms_s2c: Self::encode_name_list(&["hmac-sha256"]),
            compression_algorithms_c2s: Self::encode_name_list(&["none"]),
            compression_algorithms_s2c: Self::encode_name_list(&["none"]),
            languages_c2s: Self::encode_name_list(&[]),
            languages_s2c: Self::encode_name_list(&[]),
            first_kex_packet_follows: false,
        }
    }

    /// Encode a name-list as SSH wire bytes (comma-separated, length-prefixed)
    fn encode_name_list(names: &[&str]) -> Vec<u8> {
        let mut joined = Vec::new();
        for (i, name) in names.iter().enumerate() {
            if i > 0 {
                joined.push(b',');
            }
            joined.extend_from_slice(name.as_bytes());
        }
        joined
    }

    /// Encode KEXINIT message to payload bytes (including msg type byte)
    pub(crate) fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(512);
        buf.push(SSH_MSG_KEXINIT);
        buf.extend_from_slice(&self.cookie);

        // Helper to write a name-list with u32 length prefix
        fn write_name_list(buf: &mut Vec<u8>, data: &[u8]) {
            buf.extend_from_slice(&(data.len() as u32).to_be_bytes());
            buf.extend_from_slice(data);
        }

        write_name_list(&mut buf, &self.kex_algorithms);
        write_name_list(&mut buf, &self.server_host_key_algorithms);
        write_name_list(&mut buf, &self.encryption_algorithms_c2s);
        write_name_list(&mut buf, &self.encryption_algorithms_s2c);
        write_name_list(&mut buf, &self.mac_algorithms_c2s);
        write_name_list(&mut buf, &self.mac_algorithms_s2c);
        write_name_list(&mut buf, &self.compression_algorithms_c2s);
        write_name_list(&mut buf, &self.compression_algorithms_s2c);
        write_name_list(&mut buf, &self.languages_c2s);
        write_name_list(&mut buf, &self.languages_s2c);

        buf.push(if self.first_kex_packet_follows { 1 } else { 0 });
        // Reserved u32
        buf.extend_from_slice(&0u32.to_be_bytes());

        buf
    }

    /// Decode KEXINIT from payload bytes (after msg type byte is consumed)
    pub(crate) fn decode(data: &[u8]) -> Result<Self, SshError> {
        if data.len() < 17 {
            return Err(SshError::InvalidPacket);
        }

        // First byte should be SSH_MSG_KEXINIT
        if data[0] != SSH_MSG_KEXINIT {
            return Err(SshError::InvalidPacket);
        }

        let mut cookie = [0u8; 16];
        cookie.copy_from_slice(&data[1..17]);

        let mut pos = 17;

        fn read_name_list(data: &[u8], pos: &mut usize) -> Result<Vec<u8>, SshError> {
            if *pos + 4 > data.len() {
                return Err(SshError::InvalidPacket);
            }
            let len =
                u32::from_be_bytes([data[*pos], data[*pos + 1], data[*pos + 2], data[*pos + 3]])
                    as usize;
            *pos += 4;
            if *pos + len > data.len() {
                return Err(SshError::InvalidPacket);
            }
            let result = data[*pos..*pos + len].to_vec();
            *pos += len;
            Ok(result)
        }

        let kex_algorithms = read_name_list(data, &mut pos)?;
        let server_host_key_algorithms = read_name_list(data, &mut pos)?;
        let encryption_algorithms_c2s = read_name_list(data, &mut pos)?;
        let encryption_algorithms_s2c = read_name_list(data, &mut pos)?;
        let mac_algorithms_c2s = read_name_list(data, &mut pos)?;
        let mac_algorithms_s2c = read_name_list(data, &mut pos)?;
        let compression_algorithms_c2s = read_name_list(data, &mut pos)?;
        let compression_algorithms_s2c = read_name_list(data, &mut pos)?;
        let languages_c2s = read_name_list(data, &mut pos)?;
        let languages_s2c = read_name_list(data, &mut pos)?;

        let first_kex_packet_follows = if pos < data.len() {
            data[pos] != 0
        } else {
            false
        };

        Ok(Self {
            cookie,
            kex_algorithms,
            server_host_key_algorithms,
            encryption_algorithms_c2s,
            encryption_algorithms_s2c,
            mac_algorithms_c2s,
            mac_algorithms_s2c,
            compression_algorithms_c2s,
            compression_algorithms_s2c,
            languages_c2s,
            languages_s2c,
            first_kex_packet_follows,
        })
    }
}

/// Key exchange state for curve25519-sha256
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KexState {
    /// Our (server) ephemeral private key
    pub server_ephemeral_private: [u8; CURVE25519_KEY_LEN],
    /// Our (server) ephemeral public key
    pub server_ephemeral_public: [u8; CURVE25519_KEY_LEN],
    /// Client ephemeral public key (from SSH_MSG_KEX_ECDH_INIT)
    pub client_ephemeral_public: [u8; CURVE25519_KEY_LEN],
    /// Shared secret K
    pub shared_secret: [u8; CURVE25519_KEY_LEN],
    /// Exchange hash H (session ID on first exchange)
    pub exchange_hash: [u8; 32],
    /// Whether key exchange is complete
    pub complete: bool,
}

impl Default for KexState {
    fn default() -> Self {
        Self::new()
    }
}

impl KexState {
    pub fn new() -> Self {
        Self {
            server_ephemeral_private: [0u8; CURVE25519_KEY_LEN],
            server_ephemeral_public: [0u8; CURVE25519_KEY_LEN],
            client_ephemeral_public: [0u8; CURVE25519_KEY_LEN],
            shared_secret: [0u8; CURVE25519_KEY_LEN],
            exchange_hash: [0u8; 32],
            complete: false,
        }
    }

    /// Set the server ephemeral key pair
    pub(crate) fn set_server_keys(&mut self, private: [u8; 32], public: [u8; 32]) {
        self.server_ephemeral_private = private;
        self.server_ephemeral_public = public;
    }

    /// Set the client's ephemeral public key from KEX_ECDH_INIT
    pub(crate) fn set_client_public(&mut self, key: [u8; 32]) {
        self.client_ephemeral_public = key;
    }
}

/// SSH transport encryption keys derived from key exchange
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransportKeys {
    /// Encryption key client-to-server
    pub enc_key_c2s: [u8; 32],
    /// Encryption key server-to-client
    pub enc_key_s2c: [u8; 32],
    /// Integrity key client-to-server
    pub mac_key_c2s: [u8; 32],
    /// Integrity key server-to-client
    pub mac_key_s2c: [u8; 32],
    /// IV client-to-server
    pub iv_c2s: [u8; 12],
    /// IV server-to-client
    pub iv_s2c: [u8; 12],
}

impl Default for TransportKeys {
    fn default() -> Self {
        Self::new()
    }
}

impl TransportKeys {
    pub fn new() -> Self {
        Self {
            enc_key_c2s: [0u8; 32],
            enc_key_s2c: [0u8; 32],
            mac_key_c2s: [0u8; 32],
            mac_key_s2c: [0u8; 32],
            iv_c2s: [0u8; 12],
            iv_s2c: [0u8; 12],
        }
    }
}

/// Encode a disconnect message
pub(crate) fn encode_disconnect(reason_code: u32, description: &str) -> Vec<u8> {
    let desc_bytes = description.as_bytes();
    let mut buf = Vec::with_capacity(16 + desc_bytes.len());
    buf.push(SSH_MSG_DISCONNECT);
    buf.extend_from_slice(&reason_code.to_be_bytes());
    // description string (length-prefixed)
    buf.extend_from_slice(&(desc_bytes.len() as u32).to_be_bytes());
    buf.extend_from_slice(desc_bytes);
    // language tag (empty)
    buf.extend_from_slice(&0u32.to_be_bytes());
    buf
}

/// Encode a NEWKEYS message
pub(crate) fn encode_newkeys() -> Vec<u8> {
    vec![SSH_MSG_NEWKEYS]
}

/// Encode a service accept message
pub(crate) fn encode_service_accept(service_name: &str) -> Vec<u8> {
    let name_bytes = service_name.as_bytes();
    let mut buf = Vec::with_capacity(5 + name_bytes.len());
    buf.push(SSH_MSG_SERVICE_ACCEPT);
    buf.extend_from_slice(&(name_bytes.len() as u32).to_be_bytes());
    buf.extend_from_slice(name_bytes);
    buf
}

/// Parse a service request, return service name
pub(crate) fn parse_service_request(payload: &[u8]) -> Result<Vec<u8>, SshError> {
    if payload.is_empty() || payload[0] != SSH_MSG_SERVICE_REQUEST {
        return Err(SshError::InvalidPacket);
    }
    if payload.len() < 5 {
        return Err(SshError::InvalidPacket);
    }
    let len = u32::from_be_bytes([payload[1], payload[2], payload[3], payload[4]]) as usize;
    if payload.len() < 5 + len {
        return Err(SshError::InvalidPacket);
    }
    Ok(payload[5..5 + len].to_vec())
}

// ============================================================================
// Section 2: Authentication (~250 lines)
// ============================================================================

/// Authentication method
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthMethod {
    /// No authentication
    None,
    /// Password authentication
    Password,
    /// Public key authentication
    PublicKey,
    /// Keyboard-interactive
    KeyboardInteractive,
}

impl AuthMethod {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Password => "password",
            Self::PublicKey => "publickey",
            Self::KeyboardInteractive => "keyboard-interactive",
        }
    }

    pub(crate) fn from_bytes(data: &[u8]) -> Option<Self> {
        match data {
            b"none" => Some(Self::None),
            b"password" => Some(Self::Password),
            b"publickey" => Some(Self::PublicKey),
            b"keyboard-interactive" => Some(Self::KeyboardInteractive),
            _ => None,
        }
    }
}

/// Authentication state tracking
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthState {
    /// Username being authenticated
    pub username: Vec<u8>,
    /// Number of failed attempts
    pub attempts: u32,
    /// Maximum allowed attempts
    pub max_attempts: u32,
    /// Authenticated successfully
    pub authenticated: bool,
    /// Service name requested
    pub service_name: Vec<u8>,
    /// Allowed methods
    pub allowed_methods: Vec<AuthMethod>,
    /// Partial success flag
    pub partial_success: bool,
}

impl Default for AuthState {
    fn default() -> Self {
        Self::new()
    }
}

impl AuthState {
    pub fn new() -> Self {
        Self {
            username: Vec::new(),
            attempts: 0,
            max_attempts: MAX_AUTH_ATTEMPTS,
            authenticated: false,
            service_name: Vec::new(),
            allowed_methods: vec![AuthMethod::Password, AuthMethod::PublicKey],
            partial_success: false,
        }
    }

    /// Record a failed attempt, return whether more attempts are allowed
    pub(crate) fn record_failure(&mut self) -> bool {
        self.attempts += 1;
        self.attempts < self.max_attempts
    }

    /// Mark authentication as successful
    pub(crate) fn mark_success(&mut self) {
        self.authenticated = true;
    }

    /// Check if authentication attempts are exhausted
    pub(crate) fn exhausted(&self) -> bool {
        self.attempts >= self.max_attempts
    }
}

/// Parsed userauth request
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserauthRequest {
    /// Username
    pub username: Vec<u8>,
    /// Service name
    pub service_name: Vec<u8>,
    /// Authentication method
    pub method: AuthMethod,
    /// Method-specific data
    pub method_data: AuthMethodData,
}

/// Method-specific authentication data
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthMethodData {
    /// No additional data
    None,
    /// Password authentication data
    Password {
        /// The password
        password: Vec<u8>,
    },
    /// Public key authentication data
    PublicKey {
        /// Whether this is a real auth (true) or a query (false)
        has_signature: bool,
        /// Algorithm name (e.g., "ssh-ed25519")
        algorithm: Vec<u8>,
        /// Public key blob
        public_key: Vec<u8>,
        /// Signature (only if has_signature is true)
        signature: Vec<u8>,
    },
}

/// Parse a userauth request from payload
pub(crate) fn parse_userauth_request(payload: &[u8]) -> Result<UserauthRequest, SshError> {
    if payload.is_empty() || payload[0] != SSH_MSG_USERAUTH_REQUEST {
        return Err(SshError::InvalidPacket);
    }

    let mut pos = 1;

    // Read username
    let username = read_ssh_string(payload, &mut pos)?;
    // Read service name
    let service_name = read_ssh_string(payload, &mut pos)?;
    // Read method name
    let method_name = read_ssh_string(payload, &mut pos)?;

    let method = AuthMethod::from_bytes(&method_name).ok_or(SshError::InvalidPacket)?;

    let method_data = match method {
        AuthMethod::None => AuthMethodData::None,
        AuthMethod::Password => {
            // boolean: FALSE (no password change)
            if pos >= payload.len() {
                return Err(SshError::InvalidPacket);
            }
            let _change_password = payload[pos];
            pos += 1;
            let password = read_ssh_string(payload, &mut pos)?;
            AuthMethodData::Password { password }
        }
        AuthMethod::PublicKey => {
            if pos >= payload.len() {
                return Err(SshError::InvalidPacket);
            }
            let has_signature = payload[pos] != 0;
            pos += 1;
            let algorithm = read_ssh_string(payload, &mut pos)?;
            let public_key = read_ssh_string(payload, &mut pos)?;
            let signature = if has_signature {
                read_ssh_string(payload, &mut pos)?
            } else {
                Vec::new()
            };
            AuthMethodData::PublicKey {
                has_signature,
                algorithm,
                public_key,
                signature,
            }
        }
        AuthMethod::KeyboardInteractive => AuthMethodData::None,
    };

    Ok(UserauthRequest {
        username,
        service_name,
        method,
        method_data,
    })
}

/// Encode a userauth failure message
pub(crate) fn encode_userauth_failure(methods: &[AuthMethod], partial_success: bool) -> Vec<u8> {
    let mut name_list = Vec::new();
    for (i, method) in methods.iter().enumerate() {
        if i > 0 {
            name_list.push(b',');
        }
        name_list.extend_from_slice(method.as_str().as_bytes());
    }

    let mut buf = Vec::with_capacity(8 + name_list.len());
    buf.push(SSH_MSG_USERAUTH_FAILURE);
    buf.extend_from_slice(&(name_list.len() as u32).to_be_bytes());
    buf.extend_from_slice(&name_list);
    buf.push(if partial_success { 1 } else { 0 });
    buf
}

/// Encode a userauth success message
pub(crate) fn encode_userauth_success() -> Vec<u8> {
    vec![SSH_MSG_USERAUTH_SUCCESS]
}

/// Encode a userauth PK_OK (server accepts this key for auth)
pub(crate) fn encode_userauth_pk_ok(algorithm: &[u8], public_key: &[u8]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(9 + algorithm.len() + public_key.len());
    buf.push(SSH_MSG_USERAUTH_PK_OK);
    write_ssh_string(&mut buf, algorithm);
    write_ssh_string(&mut buf, public_key);
    buf
}

/// Encode a banner message
pub(crate) fn encode_banner(message: &str) -> Vec<u8> {
    let msg_bytes = message.as_bytes();
    let mut buf = Vec::with_capacity(9 + msg_bytes.len());
    buf.push(SSH_MSG_USERAUTH_BANNER);
    write_ssh_string(&mut buf, msg_bytes);
    // language tag (empty)
    write_ssh_string(&mut buf, b"");
    buf
}

// ============================================================================
// Section 3: Channel Management (~300 lines)
// ============================================================================

/// Channel type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelType {
    /// Interactive session
    Session,
    /// Direct TCP/IP forwarding
    DirectTcpip,
    /// Forwarded TCP/IP
    ForwardedTcpip,
}

impl ChannelType {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Self::Session => "session",
            Self::DirectTcpip => "direct-tcpip",
            Self::ForwardedTcpip => "forwarded-tcpip",
        }
    }

    pub(crate) fn from_bytes(data: &[u8]) -> Option<Self> {
        match data {
            b"session" => Some(Self::Session),
            b"direct-tcpip" => Some(Self::DirectTcpip),
            b"forwarded-tcpip" => Some(Self::ForwardedTcpip),
            _ => None,
        }
    }
}

/// Channel state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelState {
    /// Channel open request sent/received
    Opening,
    /// Channel confirmed and active
    Open,
    /// EOF sent on this channel
    EofSent,
    /// EOF received on this channel
    EofReceived,
    /// Close sent
    CloseSent,
    /// Close received
    CloseReceived,
    /// Fully closed
    Closed,
}

/// SSH channel
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Channel {
    /// Local channel ID
    pub local_id: u32,
    /// Remote (peer) channel ID
    pub remote_id: u32,
    /// Channel type
    pub channel_type: ChannelType,
    /// Current state
    pub state: ChannelState,
    /// Local window size remaining (how much data we can receive)
    pub local_window: u32,
    /// Remote window size remaining (how much data we can send)
    pub remote_window: u32,
    /// Local maximum packet size
    pub local_max_packet: u32,
    /// Remote maximum packet size
    pub remote_max_packet: u32,
    /// Whether a PTY has been allocated
    pub pty_allocated: bool,
    /// Associated PTY info (if allocated)
    pub pty_info: Option<PtyInfo>,
    /// Whether a shell or exec has been started
    pub session_started: bool,
    /// Environment variables set via "env" requests
    pub env_vars: Vec<(Vec<u8>, Vec<u8>)>,
    /// EOF received from remote
    pub eof_received: bool,
    /// EOF sent to remote
    pub eof_sent: bool,
}

impl Channel {
    /// Create a new channel in the Opening state
    pub fn new(local_id: u32, channel_type: ChannelType) -> Self {
        Self {
            local_id,
            remote_id: 0,
            channel_type,
            state: ChannelState::Opening,
            local_window: DEFAULT_WINDOW_SIZE,
            remote_window: 0,
            local_max_packet: MAX_CHANNEL_DATA_SIZE,
            remote_max_packet: 0,
            pty_allocated: false,
            pty_info: None,
            session_started: false,
            env_vars: Vec::new(),
            eof_received: false,
            eof_sent: false,
        }
    }

    /// Confirm channel open from remote
    pub(crate) fn confirm(&mut self, remote_id: u32, remote_window: u32, remote_max_packet: u32) {
        self.remote_id = remote_id;
        self.remote_window = remote_window;
        self.remote_max_packet = remote_max_packet;
        self.state = ChannelState::Open;
    }

    /// Consume send window (returns true if enough window available)
    pub(crate) fn consume_send_window(&mut self, amount: u32) -> bool {
        if self.remote_window >= amount {
            self.remote_window -= amount;
            true
        } else {
            false
        }
    }

    /// Consume receive window
    pub(crate) fn consume_recv_window(&mut self, amount: u32) -> bool {
        if self.local_window >= amount {
            self.local_window -= amount;
            true
        } else {
            false
        }
    }

    /// Adjust local window (increase capacity to receive more data)
    pub(crate) fn adjust_local_window(&mut self, increment: u32) {
        self.local_window = self.local_window.saturating_add(increment);
    }

    /// Adjust remote window (peer sent WINDOW_ADJUST)
    pub(crate) fn adjust_remote_window(&mut self, increment: u32) {
        self.remote_window = self.remote_window.saturating_add(increment);
    }

    /// Mark EOF received
    pub(crate) fn mark_eof_received(&mut self) {
        self.eof_received = true;
        if self.state == ChannelState::Open {
            self.state = ChannelState::EofReceived;
        }
    }

    /// Mark EOF sent
    pub(crate) fn mark_eof_sent(&mut self) {
        self.eof_sent = true;
        if self.state == ChannelState::Open {
            self.state = ChannelState::EofSent;
        }
    }

    /// Mark channel as closed
    pub(crate) fn close(&mut self) {
        self.state = ChannelState::Closed;
    }
}

/// Channel table managing multiple channels
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelTable {
    /// Channels indexed by local ID
    channels: BTreeMap<u32, Channel>,
    /// Next local channel ID to allocate
    next_id: u32,
}

impl Default for ChannelTable {
    fn default() -> Self {
        Self::new()
    }
}

impl ChannelTable {
    pub fn new() -> Self {
        Self {
            channels: BTreeMap::new(),
            next_id: 0,
        }
    }

    /// Allocate a new channel, returning its local ID
    pub(crate) fn open(&mut self, channel_type: ChannelType) -> Result<u32, SshError> {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        let channel = Channel::new(id, channel_type);
        self.channels.insert(id, channel);
        Ok(id)
    }

    /// Get a channel by local ID
    pub(crate) fn get(&self, local_id: u32) -> Option<&Channel> {
        self.channels.get(&local_id)
    }

    /// Get a mutable channel by local ID
    pub(crate) fn get_mut(&mut self, local_id: u32) -> Option<&mut Channel> {
        self.channels.get_mut(&local_id)
    }

    /// Remove a channel
    pub(crate) fn remove(&mut self, local_id: u32) -> Option<Channel> {
        self.channels.remove(&local_id)
    }

    /// Number of active channels
    pub(crate) fn count(&self) -> usize {
        self.channels.len()
    }

    /// Check if a channel exists
    pub(crate) fn contains(&self, local_id: u32) -> bool {
        self.channels.contains_key(&local_id)
    }
}

/// Encode a channel open message
pub(crate) fn encode_channel_open(
    channel_type: ChannelType,
    sender_channel: u32,
    initial_window: u32,
    max_packet: u32,
) -> Vec<u8> {
    let type_str = channel_type.as_str().as_bytes();
    let mut buf = Vec::with_capacity(20 + type_str.len());
    buf.push(SSH_MSG_CHANNEL_OPEN);
    write_ssh_string(&mut buf, type_str);
    buf.extend_from_slice(&sender_channel.to_be_bytes());
    buf.extend_from_slice(&initial_window.to_be_bytes());
    buf.extend_from_slice(&max_packet.to_be_bytes());
    buf
}

/// Parse a channel open request
pub(crate) fn parse_channel_open(payload: &[u8]) -> Result<(ChannelType, u32, u32, u32), SshError> {
    if payload.is_empty() || payload[0] != SSH_MSG_CHANNEL_OPEN {
        return Err(SshError::InvalidPacket);
    }
    let mut pos = 1;
    let type_name = read_ssh_string(payload, &mut pos)?;
    let channel_type = ChannelType::from_bytes(&type_name).ok_or(SshError::InvalidChannelType)?;

    if pos + 12 > payload.len() {
        return Err(SshError::InvalidPacket);
    }
    let sender_channel = read_u32(payload, &mut pos)?;
    let initial_window = read_u32(payload, &mut pos)?;
    let max_packet = read_u32(payload, &mut pos)?;

    Ok((channel_type, sender_channel, initial_window, max_packet))
}

/// Encode channel open confirmation
pub(crate) fn encode_channel_open_confirmation(
    recipient_channel: u32,
    sender_channel: u32,
    initial_window: u32,
    max_packet: u32,
) -> Vec<u8> {
    let mut buf = Vec::with_capacity(17);
    buf.push(SSH_MSG_CHANNEL_OPEN_CONFIRMATION);
    buf.extend_from_slice(&recipient_channel.to_be_bytes());
    buf.extend_from_slice(&sender_channel.to_be_bytes());
    buf.extend_from_slice(&initial_window.to_be_bytes());
    buf.extend_from_slice(&max_packet.to_be_bytes());
    buf
}

/// Encode channel open failure
pub(crate) fn encode_channel_open_failure(
    recipient_channel: u32,
    reason_code: u32,
    description: &str,
) -> Vec<u8> {
    let desc_bytes = description.as_bytes();
    let mut buf = Vec::with_capacity(17 + desc_bytes.len());
    buf.push(SSH_MSG_CHANNEL_OPEN_FAILURE);
    buf.extend_from_slice(&recipient_channel.to_be_bytes());
    buf.extend_from_slice(&reason_code.to_be_bytes());
    write_ssh_string(&mut buf, desc_bytes);
    // language tag (empty)
    write_ssh_string(&mut buf, b"");
    buf
}

/// Encode channel data
pub(crate) fn encode_channel_data(recipient_channel: u32, data: &[u8]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(9 + data.len());
    buf.push(SSH_MSG_CHANNEL_DATA);
    buf.extend_from_slice(&recipient_channel.to_be_bytes());
    write_ssh_string(&mut buf, data);
    buf
}

/// Parse channel data, returns (recipient_channel, data)
pub(crate) fn parse_channel_data(payload: &[u8]) -> Result<(u32, Vec<u8>), SshError> {
    if payload.is_empty() || payload[0] != SSH_MSG_CHANNEL_DATA {
        return Err(SshError::InvalidPacket);
    }
    let mut pos = 1;
    let recipient_channel = read_u32(payload, &mut pos)?;
    let data = read_ssh_string(payload, &mut pos)?;
    Ok((recipient_channel, data))
}

/// Encode channel extended data (e.g., stderr)
pub(crate) fn encode_channel_extended_data(
    recipient_channel: u32,
    data_type: u32,
    data: &[u8],
) -> Vec<u8> {
    let mut buf = Vec::with_capacity(13 + data.len());
    buf.push(SSH_MSG_CHANNEL_EXTENDED_DATA);
    buf.extend_from_slice(&recipient_channel.to_be_bytes());
    buf.extend_from_slice(&data_type.to_be_bytes());
    write_ssh_string(&mut buf, data);
    buf
}

/// Encode channel window adjust
pub(crate) fn encode_window_adjust(recipient_channel: u32, bytes_to_add: u32) -> Vec<u8> {
    let mut buf = Vec::with_capacity(9);
    buf.push(SSH_MSG_CHANNEL_WINDOW_ADJUST);
    buf.extend_from_slice(&recipient_channel.to_be_bytes());
    buf.extend_from_slice(&bytes_to_add.to_be_bytes());
    buf
}

/// Parse channel window adjust, returns (recipient_channel, bytes_to_add)
pub(crate) fn parse_window_adjust(payload: &[u8]) -> Result<(u32, u32), SshError> {
    if payload.is_empty() || payload[0] != SSH_MSG_CHANNEL_WINDOW_ADJUST {
        return Err(SshError::InvalidPacket);
    }
    let mut pos = 1;
    let recipient_channel = read_u32(payload, &mut pos)?;
    let bytes_to_add = read_u32(payload, &mut pos)?;
    Ok((recipient_channel, bytes_to_add))
}

/// Encode channel EOF
pub(crate) fn encode_channel_eof(recipient_channel: u32) -> Vec<u8> {
    let mut buf = Vec::with_capacity(5);
    buf.push(SSH_MSG_CHANNEL_EOF);
    buf.extend_from_slice(&recipient_channel.to_be_bytes());
    buf
}

/// Encode channel close
pub(crate) fn encode_channel_close(recipient_channel: u32) -> Vec<u8> {
    let mut buf = Vec::with_capacity(5);
    buf.push(SSH_MSG_CHANNEL_CLOSE);
    buf.extend_from_slice(&recipient_channel.to_be_bytes());
    buf
}

/// Encode a channel request
pub(crate) fn encode_channel_request(
    recipient_channel: u32,
    request_type: &str,
    want_reply: bool,
    data: &[u8],
) -> Vec<u8> {
    let type_bytes = request_type.as_bytes();
    let mut buf = Vec::with_capacity(10 + type_bytes.len() + data.len());
    buf.push(SSH_MSG_CHANNEL_REQUEST);
    buf.extend_from_slice(&recipient_channel.to_be_bytes());
    write_ssh_string(&mut buf, type_bytes);
    buf.push(if want_reply { 1 } else { 0 });
    buf.extend_from_slice(data);
    buf
}

/// Parsed channel request
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelRequest {
    /// Recipient channel
    pub recipient_channel: u32,
    /// Request type string
    pub request_type: Vec<u8>,
    /// Whether the sender wants a reply
    pub want_reply: bool,
    /// Type-specific data (remaining bytes)
    pub data: Vec<u8>,
}

/// Parse a channel request
pub(crate) fn parse_channel_request(payload: &[u8]) -> Result<ChannelRequest, SshError> {
    if payload.is_empty() || payload[0] != SSH_MSG_CHANNEL_REQUEST {
        return Err(SshError::InvalidPacket);
    }
    let mut pos = 1;
    let recipient_channel = read_u32(payload, &mut pos)?;
    let request_type = read_ssh_string(payload, &mut pos)?;
    if pos >= payload.len() {
        return Err(SshError::InvalidPacket);
    }
    let want_reply = payload[pos] != 0;
    pos += 1;
    let data = if pos < payload.len() {
        payload[pos..].to_vec()
    } else {
        Vec::new()
    };

    Ok(ChannelRequest {
        recipient_channel,
        request_type,
        want_reply,
        data,
    })
}

// ============================================================================
// Section 4: Session Management (~200 lines)
// ============================================================================

/// PTY terminal information
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PtyInfo {
    /// Terminal type (e.g., "xterm-256color")
    pub term_type: Vec<u8>,
    /// Terminal width in columns
    pub width_cols: u32,
    /// Terminal height in rows
    pub height_rows: u32,
    /// Terminal width in pixels
    pub width_pixels: u32,
    /// Terminal height in pixels
    pub height_pixels: u32,
    /// Terminal modes (encoded as per RFC 4254)
    pub terminal_modes: Vec<u8>,
}

impl PtyInfo {
    /// Parse a pty-req request data blob
    pub(crate) fn parse(data: &[u8]) -> Result<Self, SshError> {
        let mut pos = 0;
        let term_type = read_ssh_string(data, &mut pos)?;
        let width_cols = read_u32(data, &mut pos)?;
        let height_rows = read_u32(data, &mut pos)?;
        let width_pixels = read_u32(data, &mut pos)?;
        let height_pixels = read_u32(data, &mut pos)?;
        let terminal_modes = read_ssh_string(data, &mut pos)?;

        Ok(Self {
            term_type,
            width_cols,
            height_rows,
            width_pixels,
            height_pixels,
            terminal_modes,
        })
    }

    /// Encode PTY info to wire format (for pty-req channel request data)
    pub(crate) fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(32 + self.term_type.len() + self.terminal_modes.len());
        write_ssh_string(&mut buf, &self.term_type);
        buf.extend_from_slice(&self.width_cols.to_be_bytes());
        buf.extend_from_slice(&self.height_rows.to_be_bytes());
        buf.extend_from_slice(&self.width_pixels.to_be_bytes());
        buf.extend_from_slice(&self.height_pixels.to_be_bytes());
        write_ssh_string(&mut buf, &self.terminal_modes);
        buf
    }
}

/// Exec request: single command execution
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecRequest {
    /// Command to execute
    pub command: Vec<u8>,
}

impl ExecRequest {
    pub(crate) fn parse(data: &[u8]) -> Result<Self, SshError> {
        let mut pos = 0;
        let command = read_ssh_string(data, &mut pos)?;
        Ok(Self { command })
    }
}

/// Environment variable request
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvRequest {
    /// Variable name
    pub name: Vec<u8>,
    /// Variable value
    pub value: Vec<u8>,
}

impl EnvRequest {
    pub(crate) fn parse(data: &[u8]) -> Result<Self, SshError> {
        let mut pos = 0;
        let name = read_ssh_string(data, &mut pos)?;
        let value = read_ssh_string(data, &mut pos)?;
        Ok(Self { name, value })
    }
}

/// Exit status payload (for "exit-status" channel request)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExitStatus {
    pub code: u32,
}

impl ExitStatus {
    pub(crate) fn encode(&self) -> Vec<u8> {
        self.code.to_be_bytes().to_vec()
    }

    pub(crate) fn parse(data: &[u8]) -> Result<Self, SshError> {
        if data.len() < 4 {
            return Err(SshError::InvalidPacket);
        }
        let code = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
        Ok(Self { code })
    }
}

/// Exit signal payload (for "exit-signal" channel request)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExitSignal {
    /// Signal name (without "SIG" prefix, e.g., "TERM", "KILL")
    pub signal_name: Vec<u8>,
    /// Core dumped flag
    pub core_dumped: bool,
    /// Error message
    pub error_message: Vec<u8>,
    /// Language tag
    pub language_tag: Vec<u8>,
}

impl ExitSignal {
    pub(crate) fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(
            16 + self.signal_name.len() + self.error_message.len() + self.language_tag.len(),
        );
        write_ssh_string(&mut buf, &self.signal_name);
        buf.push(if self.core_dumped { 1 } else { 0 });
        write_ssh_string(&mut buf, &self.error_message);
        write_ssh_string(&mut buf, &self.language_tag);
        buf
    }

    pub(crate) fn parse(data: &[u8]) -> Result<Self, SshError> {
        let mut pos = 0;
        let signal_name = read_ssh_string(data, &mut pos)?;
        if pos >= data.len() {
            return Err(SshError::InvalidPacket);
        }
        let core_dumped = data[pos] != 0;
        pos += 1;
        let error_message = read_ssh_string(data, &mut pos)?;
        let language_tag = read_ssh_string(data, &mut pos)?;
        Ok(Self {
            signal_name,
            core_dumped,
            error_message,
            language_tag,
        })
    }
}

/// Shell session state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellSessionState {
    /// Waiting for PTY or shell request
    Idle,
    /// PTY allocated, waiting for shell/exec
    PtyAllocated,
    /// Shell or command running
    Running,
    /// Session completed with exit status
    Exited,
}

/// Represents a shell/exec session on a channel
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellSession {
    /// Channel ID this session is bound to
    pub channel_id: u32,
    /// Session state
    pub state: ShellSessionState,
    /// PTY info (if allocated)
    pub pty: Option<PtyInfo>,
    /// Command for exec requests (None for interactive shell)
    pub command: Option<Vec<u8>>,
    /// Environment variables
    pub environment: Vec<(Vec<u8>, Vec<u8>)>,
    /// Exit code (set when process exits)
    pub exit_code: Option<u32>,
    /// Exit signal (set if process killed by signal)
    pub exit_signal: Option<Vec<u8>>,
}

impl ShellSession {
    /// Create a new idle shell session
    pub fn new(channel_id: u32) -> Self {
        Self {
            channel_id,
            state: ShellSessionState::Idle,
            pty: None,
            command: None,
            environment: Vec::new(),
            exit_code: None,
            exit_signal: None,
        }
    }

    /// Allocate a PTY for this session
    pub(crate) fn allocate_pty(&mut self, pty_info: PtyInfo) -> Result<(), SshError> {
        if self.state != ShellSessionState::Idle {
            return Err(SshError::InvalidState);
        }
        self.pty = Some(pty_info);
        self.state = ShellSessionState::PtyAllocated;
        Ok(())
    }

    /// Start a shell session
    pub(crate) fn start_shell(&mut self) -> Result<(), SshError> {
        match self.state {
            ShellSessionState::Idle | ShellSessionState::PtyAllocated => {
                self.state = ShellSessionState::Running;
                Ok(())
            }
            _ => Err(SshError::InvalidState),
        }
    }

    /// Start an exec session
    pub fn start_exec(&mut self, command: Vec<u8>) -> Result<(), SshError> {
        match self.state {
            ShellSessionState::Idle | ShellSessionState::PtyAllocated => {
                self.command = Some(command);
                self.state = ShellSessionState::Running;
                Ok(())
            }
            _ => Err(SshError::InvalidState),
        }
    }

    /// Add an environment variable
    pub(crate) fn set_env(&mut self, name: Vec<u8>, value: Vec<u8>) {
        self.environment.push((name, value));
    }

    /// Mark session as exited
    pub(crate) fn mark_exited(&mut self, code: u32) {
        self.exit_code = Some(code);
        self.state = ShellSessionState::Exited;
    }

    /// Mark session as killed by signal
    pub(crate) fn mark_signaled(&mut self, signal: Vec<u8>) {
        self.exit_signal = Some(signal);
        self.state = ShellSessionState::Exited;
    }
}

// ============================================================================
// Section 5: SSH Server (~150 lines)
// ============================================================================

/// Host key pair (Ed25519)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostKeyPair {
    /// Ed25519 public key (32 bytes)
    pub public_key: [u8; ED25519_PUB_LEN],
    /// Ed25519 private key (64 bytes: seed + public)
    pub private_key: [u8; 64],
}

impl HostKeyPair {
    /// Create a host key pair from raw bytes
    pub fn new(public_key: [u8; 32], private_key: [u8; 64]) -> Self {
        Self {
            public_key,
            private_key,
        }
    }

    /// Encode public key in SSH wire format ("ssh-ed25519" || key_data)
    pub(crate) fn encode_public_key(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(51);
        write_ssh_string(&mut buf, b"ssh-ed25519");
        write_ssh_string(&mut buf, &self.public_key);
        buf
    }
}

/// SSH session (one per connected client)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshSession {
    /// Unique session identifier
    pub session_id: u32,
    /// Current state
    pub state: SessionState,
    /// Authentication state
    pub auth: AuthState,
    /// Key exchange state
    pub kex: KexState,
    /// Transport keys (after key exchange)
    pub transport_keys: TransportKeys,
    /// Negotiated algorithms
    pub algorithms: NegotiatedAlgorithms,
    /// Channel table
    pub channels: ChannelTable,
    /// Shell sessions indexed by channel ID
    pub shell_sessions: BTreeMap<u32, ShellSession>,
    /// Session ID (exchange hash of first key exchange)
    pub session_hash: [u8; 32],
    /// Client version string
    pub client_version: Vec<u8>,
    /// Server version string
    pub server_version: Vec<u8>,
    /// Client KEXINIT payload (for exchange hash computation)
    pub client_kexinit: Vec<u8>,
    /// Server KEXINIT payload (for exchange hash computation)
    pub server_kexinit: Vec<u8>,
    /// Packet sequence number (send)
    pub send_seq: u32,
    /// Packet sequence number (recv)
    pub recv_seq: u32,
}

impl SshSession {
    /// Create a new session in VersionExchange state
    pub fn new(session_id: u32) -> Self {
        Self {
            session_id,
            state: SessionState::VersionExchange,
            auth: AuthState::new(),
            kex: KexState::new(),
            transport_keys: TransportKeys::new(),
            algorithms: NegotiatedAlgorithms::new(),
            channels: ChannelTable::new(),
            shell_sessions: BTreeMap::new(),
            session_hash: [0u8; 32],
            client_version: Vec::new(),
            server_version: SSH_VERSION_STRING[..SSH_VERSION_STRING.len() - 2].to_vec(),
            client_kexinit: Vec::new(),
            server_kexinit: Vec::new(),
            send_seq: 0,
            recv_seq: 0,
        }
    }

    /// Process version exchange
    pub(crate) fn process_version(&mut self, data: &[u8]) -> Result<SessionState, SshError> {
        if self.state != SessionState::VersionExchange {
            return Err(SshError::InvalidState);
        }

        let version = VersionInfo::parse(data).ok_or(SshError::VersionMismatch)?;
        if &version.protocol_version != b"2.0" {
            return Err(SshError::VersionMismatch);
        }

        self.client_version = data.to_vec();
        self.state = SessionState::KeyExchange;
        Ok(self.state)
    }

    /// Transition to NewKeysExpected after KEXINIT exchange
    pub(crate) fn begin_key_exchange(&mut self) -> Result<(), SshError> {
        if self.state != SessionState::KeyExchange {
            return Err(SshError::InvalidState);
        }
        Ok(())
    }

    /// Process NEWKEYS message, transition to Authentication
    pub(crate) fn process_newkeys(&mut self) -> Result<SessionState, SshError> {
        if self.state != SessionState::KeyExchange && self.state != SessionState::NewKeysExpected {
            return Err(SshError::InvalidState);
        }
        self.state = SessionState::Authentication;
        Ok(self.state)
    }

    /// Process successful authentication, transition to Connected
    pub(crate) fn authenticate_success(&mut self) -> Result<SessionState, SshError> {
        if self.state != SessionState::Authentication {
            return Err(SshError::InvalidState);
        }
        self.auth.mark_success();
        self.state = SessionState::Connected;
        Ok(self.state)
    }

    /// Disconnect the session
    pub(crate) fn disconnect(&mut self) {
        self.state = SessionState::Disconnected;
    }

    /// Process a channel request (pty-req, shell, exec, env)
    pub(crate) fn handle_channel_request(
        &mut self,
        request: &ChannelRequest,
    ) -> Result<bool, SshError> {
        let channel = self
            .channels
            .get_mut(request.recipient_channel)
            .ok_or(SshError::ChannelNotFound)?;

        match request.request_type.as_slice() {
            b"pty-req" => {
                let pty_info = PtyInfo::parse(&request.data)?;
                channel.pty_allocated = true;
                channel.pty_info = Some(pty_info.clone());

                // Create/update shell session
                let session = self
                    .shell_sessions
                    .entry(request.recipient_channel)
                    .or_insert_with(|| ShellSession::new(request.recipient_channel));
                session.allocate_pty(pty_info)?;
                Ok(true)
            }
            b"shell" => {
                channel.session_started = true;
                let session = self
                    .shell_sessions
                    .entry(request.recipient_channel)
                    .or_insert_with(|| ShellSession::new(request.recipient_channel));
                session.start_shell()?;
                Ok(true)
            }
            b"exec" => {
                let exec_req = ExecRequest::parse(&request.data)?;
                channel.session_started = true;
                let session = self
                    .shell_sessions
                    .entry(request.recipient_channel)
                    .or_insert_with(|| ShellSession::new(request.recipient_channel));
                session.start_exec(exec_req.command)?;
                Ok(true)
            }
            b"env" => {
                let env_req = EnvRequest::parse(&request.data)?;
                let session = self
                    .shell_sessions
                    .entry(request.recipient_channel)
                    .or_insert_with(|| ShellSession::new(request.recipient_channel));
                session.set_env(env_req.name, env_req.value);
                Ok(true)
            }
            b"exit-status" => {
                let exit = ExitStatus::parse(&request.data)?;
                if let Some(session) = self.shell_sessions.get_mut(&request.recipient_channel) {
                    session.mark_exited(exit.code);
                }
                Ok(true)
            }
            b"exit-signal" => {
                let signal = ExitSignal::parse(&request.data)?;
                if let Some(session) = self.shell_sessions.get_mut(&request.recipient_channel) {
                    session.mark_signaled(signal.signal_name);
                }
                Ok(true)
            }
            _ => {
                // Unknown request type
                Ok(false)
            }
        }
    }

    /// Increment send sequence counter
    pub(crate) fn next_send_seq(&mut self) -> u32 {
        let seq = self.send_seq;
        self.send_seq = self.send_seq.wrapping_add(1);
        seq
    }

    /// Increment recv sequence counter
    pub(crate) fn next_recv_seq(&mut self) -> u32 {
        let seq = self.recv_seq;
        self.recv_seq = self.recv_seq.wrapping_add(1);
        seq
    }
}

/// SSH server configuration and state
#[derive(Debug, Clone)]
pub struct SshServer {
    /// Server host key
    pub host_key: HostKeyPair,
    /// Listen port
    pub port: u16,
    /// Maximum concurrent sessions
    pub max_sessions: usize,
    /// Active sessions
    pub sessions: BTreeMap<u32, SshSession>,
    /// Next session ID
    pub next_session_id: u32,
    /// Server banner (optional, sent before auth)
    pub banner: Option<String>,
}

impl SshServer {
    /// Create a new SSH server
    pub fn new(host_key: HostKeyPair, port: u16) -> Self {
        Self {
            host_key,
            port,
            max_sessions: MAX_SESSIONS,
            sessions: BTreeMap::new(),
            next_session_id: 0,
            banner: None,
        }
    }

    /// Set the server banner message
    pub(crate) fn set_banner(&mut self, banner: String) {
        self.banner = Some(banner);
    }

    /// Accept a new connection, creating a session
    pub(crate) fn accept_connection(&mut self) -> Result<u32, SshError> {
        if self.sessions.len() >= self.max_sessions {
            return Err(SshError::SessionLimitReached);
        }
        let id = self.next_session_id;
        self.next_session_id = self.next_session_id.wrapping_add(1);
        let session = SshSession::new(id);
        self.sessions.insert(id, session);
        Ok(id)
    }

    /// Get a session by ID
    pub(crate) fn get_session(&self, session_id: u32) -> Option<&SshSession> {
        self.sessions.get(&session_id)
    }

    /// Get a mutable session by ID
    pub(crate) fn get_session_mut(&mut self, session_id: u32) -> Option<&mut SshSession> {
        self.sessions.get_mut(&session_id)
    }

    /// Remove a disconnected session
    pub(crate) fn remove_session(&mut self, session_id: u32) -> Option<SshSession> {
        self.sessions.remove(&session_id)
    }

    /// Number of active sessions
    pub(crate) fn active_sessions(&self) -> usize {
        self.sessions.len()
    }
}

// ============================================================================
// Wire encoding/decoding helpers
// ============================================================================

/// Read a length-prefixed SSH string from a buffer, advancing pos
fn read_ssh_string(data: &[u8], pos: &mut usize) -> Result<Vec<u8>, SshError> {
    if *pos + 4 > data.len() {
        return Err(SshError::InvalidPacket);
    }
    let len =
        u32::from_be_bytes([data[*pos], data[*pos + 1], data[*pos + 2], data[*pos + 3]]) as usize;
    *pos += 4;
    if *pos + len > data.len() {
        return Err(SshError::InvalidPacket);
    }
    let result = data[*pos..*pos + len].to_vec();
    *pos += len;
    Ok(result)
}

/// Write a length-prefixed SSH string to a buffer
fn write_ssh_string(buf: &mut Vec<u8>, data: &[u8]) {
    buf.extend_from_slice(&(data.len() as u32).to_be_bytes());
    buf.extend_from_slice(data);
}

/// Read a u32 from a buffer, advancing pos
fn read_u32(data: &[u8], pos: &mut usize) -> Result<u32, SshError> {
    if *pos + 4 > data.len() {
        return Err(SshError::InvalidPacket);
    }
    let val = u32::from_be_bytes([data[*pos], data[*pos + 1], data[*pos + 2], data[*pos + 3]]);
    *pos += 4;
    Ok(val)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use alloc::vec;

    use super::*;

    // -----------------------------------------------------------------------
    // Version string parsing
    // -----------------------------------------------------------------------

    #[test]
    fn test_version_parse_basic() {
        let input = b"SSH-2.0-OpenSSH_9.0\r\n";
        let v = VersionInfo::parse(input).unwrap();
        assert_eq!(&v.protocol_version, b"2.0");
        assert_eq!(&v.software_version, b"OpenSSH_9.0");
        assert!(v.comment.is_empty());
    }

    #[test]
    fn test_version_parse_with_comment() {
        let input = b"SSH-2.0-VeridianOS_1.0 custom comment\r\n";
        let v = VersionInfo::parse(input).unwrap();
        assert_eq!(&v.software_version, b"VeridianOS_1.0");
        assert_eq!(&v.comment, b"custom comment");
    }

    #[test]
    fn test_version_parse_no_crlf() {
        let input = b"SSH-2.0-TestClient\n";
        let v = VersionInfo::parse(input).unwrap();
        assert_eq!(&v.software_version, b"TestClient");
    }

    #[test]
    fn test_version_parse_invalid_prefix() {
        assert!(VersionInfo::parse(b"TLS-2.0-Client\r\n").is_none());
    }

    #[test]
    fn test_version_parse_wrong_protocol() {
        assert!(VersionInfo::parse(b"SSH-1.0-OldClient\r\n").is_none());
    }

    #[test]
    fn test_version_encode_roundtrip() {
        let input = b"SSH-2.0-TestServer\r\n";
        let v = VersionInfo::parse(input).unwrap();
        let encoded = v.encode();
        assert_eq!(&encoded, input);
    }

    // -----------------------------------------------------------------------
    // Binary packet encode/decode
    // -----------------------------------------------------------------------

    #[test]
    fn test_packet_encode_decode_roundtrip() {
        let payload = vec![SSH_MSG_IGNORE, 0x01, 0x02, 0x03];
        let packet = SshPacket::new(payload.clone());
        let encoded = packet.encode();

        let (decoded, consumed) = SshPacket::decode(&encoded).unwrap();
        assert_eq!(consumed, encoded.len());
        assert_eq!(decoded.payload, payload);
    }

    #[test]
    fn test_packet_minimum_padding() {
        let payload = vec![SSH_MSG_IGNORE];
        let packet = SshPacket::new(payload);
        assert!(packet.padding.len() >= MIN_PADDING);
    }

    #[test]
    fn test_packet_alignment() {
        let payload = vec![0u8; 17];
        let packet = SshPacket::new(payload);
        let encoded = packet.encode();
        // Total encoded length (excluding MAC) should be multiple of block size
        assert_eq!(encoded.len() % BLOCK_SIZE_CLEAR, 0);
    }

    #[test]
    fn test_packet_decode_too_short() {
        let data = [0u8; 3];
        assert_eq!(SshPacket::decode(&data), Err(SshError::InvalidPacket));
    }

    #[test]
    fn test_packet_decode_truncated() {
        // packet_length=100 but only 10 bytes available
        let mut data = vec![0, 0, 0, 100, 4];
        data.extend_from_slice(&[0u8; 5]);
        assert_eq!(SshPacket::decode(&data), Err(SshError::InvalidPacket));
    }

    #[test]
    fn test_packet_message_type() {
        let packet = SshPacket::new(vec![SSH_MSG_KEXINIT, 0, 0]);
        assert_eq!(packet.message_type(), Some(SSH_MSG_KEXINIT));
    }

    #[test]
    fn test_packet_empty_payload_message_type() {
        let packet = SshPacket {
            payload: Vec::new(),
            padding: vec![0u8; 4],
        };
        assert_eq!(packet.message_type(), None);
    }

    // -----------------------------------------------------------------------
    // KEXINIT message
    // -----------------------------------------------------------------------

    #[test]
    fn test_kexinit_encode_decode_roundtrip() {
        let cookie = [0xAA; 16];
        let msg = KexInitMessage::new_server(cookie);
        let encoded = msg.encode();

        assert_eq!(encoded[0], SSH_MSG_KEXINIT);
        assert_eq!(&encoded[1..17], &cookie);

        let decoded = KexInitMessage::decode(&encoded).unwrap();
        assert_eq!(decoded.cookie, cookie);
        assert_eq!(decoded.kex_algorithms, msg.kex_algorithms);
        assert_eq!(
            decoded.server_host_key_algorithms,
            msg.server_host_key_algorithms
        );
        assert!(!decoded.first_kex_packet_follows);
    }

    #[test]
    fn test_kexinit_decode_wrong_type() {
        let data = vec![SSH_MSG_NEWKEYS; 20];
        assert_eq!(KexInitMessage::decode(&data), Err(SshError::InvalidPacket));
    }

    #[test]
    fn test_kexinit_decode_too_short() {
        let data = vec![SSH_MSG_KEXINIT; 10];
        assert_eq!(KexInitMessage::decode(&data), Err(SshError::InvalidPacket));
    }

    // -----------------------------------------------------------------------
    // Channel open/confirm/close lifecycle
    // -----------------------------------------------------------------------

    #[test]
    fn test_channel_lifecycle() {
        let mut table = ChannelTable::new();

        // Open
        let id = table.open(ChannelType::Session).unwrap();
        assert_eq!(id, 0);
        assert_eq!(table.count(), 1);

        // Confirm
        {
            let ch = table.get_mut(id).unwrap();
            assert_eq!(ch.state, ChannelState::Opening);
            ch.confirm(42, DEFAULT_WINDOW_SIZE, MAX_CHANNEL_DATA_SIZE);
            assert_eq!(ch.state, ChannelState::Open);
            assert_eq!(ch.remote_id, 42);
        }

        // EOF
        {
            let ch = table.get_mut(id).unwrap();
            ch.mark_eof_received();
            assert_eq!(ch.state, ChannelState::EofReceived);
        }

        // Close
        {
            let ch = table.get_mut(id).unwrap();
            ch.close();
            assert_eq!(ch.state, ChannelState::Closed);
        }

        // Remove
        let removed = table.remove(id);
        assert!(removed.is_some());
        assert_eq!(table.count(), 0);
    }

    #[test]
    fn test_channel_open_multiple() {
        let mut table = ChannelTable::new();
        let id1 = table.open(ChannelType::Session).unwrap();
        let id2 = table.open(ChannelType::Session).unwrap();
        assert_ne!(id1, id2);
        assert_eq!(table.count(), 2);
    }

    // -----------------------------------------------------------------------
    // Window adjust tracking
    // -----------------------------------------------------------------------

    #[test]
    fn test_window_adjust() {
        let mut ch = Channel::new(0, ChannelType::Session);
        ch.confirm(1, 1024, 512);

        assert!(ch.consume_send_window(512));
        assert_eq!(ch.remote_window, 512);
        assert!(ch.consume_send_window(512));
        assert_eq!(ch.remote_window, 0);
        assert!(!ch.consume_send_window(1));

        ch.adjust_remote_window(2048);
        assert_eq!(ch.remote_window, 2048);
    }

    #[test]
    fn test_window_adjust_saturation() {
        let mut ch = Channel::new(0, ChannelType::Session);
        ch.remote_window = u32::MAX - 10;
        ch.adjust_remote_window(100);
        assert_eq!(ch.remote_window, u32::MAX);
    }

    #[test]
    fn test_local_window_consume() {
        let mut ch = Channel::new(0, ChannelType::Session);
        let initial = ch.local_window;
        assert!(ch.consume_recv_window(1024));
        assert_eq!(ch.local_window, initial - 1024);

        ch.adjust_local_window(2048);
        assert_eq!(ch.local_window, initial - 1024 + 2048);
    }

    // -----------------------------------------------------------------------
    // Window adjust message encode/parse
    // -----------------------------------------------------------------------

    #[test]
    fn test_window_adjust_encode_parse() {
        let encoded = encode_window_adjust(7, 65536);
        let (ch, bytes) = parse_window_adjust(&encoded).unwrap();
        assert_eq!(ch, 7);
        assert_eq!(bytes, 65536);
    }

    // -----------------------------------------------------------------------
    // Auth request parsing
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_password_auth() {
        let mut payload = vec![SSH_MSG_USERAUTH_REQUEST];
        // username: "alice"
        payload.extend_from_slice(&5u32.to_be_bytes());
        payload.extend_from_slice(b"alice");
        // service: "ssh-connection"
        payload.extend_from_slice(&14u32.to_be_bytes());
        payload.extend_from_slice(b"ssh-connection");
        // method: "password"
        payload.extend_from_slice(&8u32.to_be_bytes());
        payload.extend_from_slice(b"password");
        // change password: false
        payload.push(0);
        // password: "secret"
        payload.extend_from_slice(&6u32.to_be_bytes());
        payload.extend_from_slice(b"secret");

        let req = parse_userauth_request(&payload).unwrap();
        assert_eq!(&req.username, b"alice");
        assert_eq!(&req.service_name, b"ssh-connection");
        assert_eq!(req.method, AuthMethod::Password);
        match &req.method_data {
            AuthMethodData::Password { password } => assert_eq!(password, b"secret"),
            _ => panic!("Expected password auth data"),
        }
    }

    #[test]
    fn test_parse_pubkey_auth_query() {
        let mut payload = vec![SSH_MSG_USERAUTH_REQUEST];
        // username
        payload.extend_from_slice(&3u32.to_be_bytes());
        payload.extend_from_slice(b"bob");
        // service
        payload.extend_from_slice(&14u32.to_be_bytes());
        payload.extend_from_slice(b"ssh-connection");
        // method
        payload.extend_from_slice(&9u32.to_be_bytes());
        payload.extend_from_slice(b"publickey");
        // has_signature: false
        payload.push(0);
        // algorithm
        payload.extend_from_slice(&11u32.to_be_bytes());
        payload.extend_from_slice(b"ssh-ed25519");
        // public key blob (dummy 32 bytes)
        let fake_key = [0x42u8; 32];
        payload.extend_from_slice(&32u32.to_be_bytes());
        payload.extend_from_slice(&fake_key);

        let req = parse_userauth_request(&payload).unwrap();
        assert_eq!(req.method, AuthMethod::PublicKey);
        match &req.method_data {
            AuthMethodData::PublicKey {
                has_signature,
                algorithm,
                public_key,
                signature,
            } => {
                assert!(!(*has_signature));
                assert_eq!(algorithm, b"ssh-ed25519");
                assert_eq!(public_key.len(), 32);
                assert!(signature.is_empty());
            }
            _ => panic!("Expected public key auth data"),
        }
    }

    // -----------------------------------------------------------------------
    // Session state transitions
    // -----------------------------------------------------------------------

    #[test]
    fn test_session_state_transitions() {
        let mut session = SshSession::new(0);
        assert_eq!(session.state, SessionState::VersionExchange);

        // Version exchange
        let new_state = session.process_version(b"SSH-2.0-TestClient\r\n").unwrap();
        assert_eq!(new_state, SessionState::KeyExchange);

        // NEWKEYS
        let new_state = session.process_newkeys().unwrap();
        assert_eq!(new_state, SessionState::Authentication);

        // Auth success
        let new_state = session.authenticate_success().unwrap();
        assert_eq!(new_state, SessionState::Connected);

        // Disconnect
        session.disconnect();
        assert_eq!(session.state, SessionState::Disconnected);
    }

    #[test]
    fn test_session_invalid_state_transition() {
        let mut session = SshSession::new(0);
        // Cannot go directly to newkeys from version exchange
        assert_eq!(session.process_newkeys(), Err(SshError::InvalidState));
    }

    #[test]
    fn test_session_version_mismatch() {
        let mut session = SshSession::new(0);
        assert_eq!(
            session.process_version(b"SSH-1.0-OldClient\r\n"),
            Err(SshError::VersionMismatch)
        );
    }

    // -----------------------------------------------------------------------
    // Channel data framing
    // -----------------------------------------------------------------------

    #[test]
    fn test_channel_data_encode_parse() {
        let data = b"Hello, SSH world!";
        let encoded = encode_channel_data(5, data);
        let (ch, parsed_data) = parse_channel_data(&encoded).unwrap();
        assert_eq!(ch, 5);
        assert_eq!(&parsed_data, data);
    }

    #[test]
    fn test_channel_data_empty() {
        let encoded = encode_channel_data(0, b"");
        let (ch, parsed_data) = parse_channel_data(&encoded).unwrap();
        assert_eq!(ch, 0);
        assert!(parsed_data.is_empty());
    }

    // -----------------------------------------------------------------------
    // Disconnect message
    // -----------------------------------------------------------------------

    #[test]
    fn test_disconnect_encode() {
        let msg = encode_disconnect(SSH_DISCONNECT_BY_APPLICATION, "goodbye");
        assert_eq!(msg[0], SSH_MSG_DISCONNECT);
        let reason = u32::from_be_bytes([msg[1], msg[2], msg[3], msg[4]]);
        assert_eq!(reason, SSH_DISCONNECT_BY_APPLICATION);
    }

    // -----------------------------------------------------------------------
    // PTY request parsing
    // -----------------------------------------------------------------------

    #[test]
    fn test_pty_request_parse() {
        let mut data = Vec::new();
        // term type: "xterm-256color"
        write_ssh_string(&mut data, b"xterm-256color");
        // width cols
        data.extend_from_slice(&80u32.to_be_bytes());
        // height rows
        data.extend_from_slice(&24u32.to_be_bytes());
        // width pixels
        data.extend_from_slice(&640u32.to_be_bytes());
        // height pixels
        data.extend_from_slice(&480u32.to_be_bytes());
        // terminal modes (empty)
        write_ssh_string(&mut data, b"");

        let pty = PtyInfo::parse(&data).unwrap();
        assert_eq!(&pty.term_type, b"xterm-256color");
        assert_eq!(pty.width_cols, 80);
        assert_eq!(pty.height_rows, 24);
        assert_eq!(pty.width_pixels, 640);
        assert_eq!(pty.height_pixels, 480);
    }

    #[test]
    fn test_pty_encode_roundtrip() {
        let pty = PtyInfo {
            term_type: b"vt100".to_vec(),
            width_cols: 132,
            height_rows: 43,
            width_pixels: 0,
            height_pixels: 0,
            terminal_modes: vec![0],
        };
        let encoded = pty.encode();
        let decoded = PtyInfo::parse(&encoded).unwrap();
        assert_eq!(decoded.term_type, pty.term_type);
        assert_eq!(decoded.width_cols, pty.width_cols);
        assert_eq!(decoded.height_rows, pty.height_rows);
    }

    // -----------------------------------------------------------------------
    // Auth state tracking
    // -----------------------------------------------------------------------

    #[test]
    fn test_auth_state_attempts() {
        let mut auth = AuthState::new();
        assert!(!auth.exhausted());

        for _ in 0..MAX_AUTH_ATTEMPTS - 1 {
            assert!(auth.record_failure());
        }
        assert!(!auth.record_failure());
        assert!(auth.exhausted());
    }

    #[test]
    fn test_auth_failure_message() {
        let methods = [AuthMethod::Password, AuthMethod::PublicKey];
        let msg = encode_userauth_failure(&methods, false);
        assert_eq!(msg[0], SSH_MSG_USERAUTH_FAILURE);
        // Last byte is partial_success = 0
        assert_eq!(*msg.last().unwrap(), 0);
    }

    // -----------------------------------------------------------------------
    // Service request
    // -----------------------------------------------------------------------

    #[test]
    fn test_service_request_parse() {
        let mut payload = vec![SSH_MSG_SERVICE_REQUEST];
        payload.extend_from_slice(&12u32.to_be_bytes());
        payload.extend_from_slice(b"ssh-userauth");
        let name = parse_service_request(&payload).unwrap();
        assert_eq!(&name, b"ssh-userauth");
    }

    // -----------------------------------------------------------------------
    // Channel request parsing
    // -----------------------------------------------------------------------

    #[test]
    fn test_channel_request_parse() {
        let encoded = encode_channel_request(3, "shell", true, b"");
        let req = parse_channel_request(&encoded).unwrap();
        assert_eq!(req.recipient_channel, 3);
        assert_eq!(&req.request_type, b"shell");
        assert!(req.want_reply);
        assert!(req.data.is_empty());
    }

    #[test]
    fn test_exec_request_parse() {
        let mut exec_data = Vec::new();
        write_ssh_string(&mut exec_data, b"ls -la /tmp");
        let encoded = encode_channel_request(1, "exec", true, &exec_data);
        let req = parse_channel_request(&encoded).unwrap();
        assert_eq!(&req.request_type, b"exec");

        let exec = ExecRequest::parse(&req.data).unwrap();
        assert_eq!(&exec.command, b"ls -la /tmp");
    }

    // -----------------------------------------------------------------------
    // Channel open encode/parse
    // -----------------------------------------------------------------------

    #[test]
    fn test_channel_open_encode_parse() {
        let encoded = encode_channel_open(
            ChannelType::Session,
            0,
            DEFAULT_WINDOW_SIZE,
            MAX_CHANNEL_DATA_SIZE,
        );
        let (ct, sender, window, max_pkt) = parse_channel_open(&encoded).unwrap();
        assert_eq!(ct, ChannelType::Session);
        assert_eq!(sender, 0);
        assert_eq!(window, DEFAULT_WINDOW_SIZE);
        assert_eq!(max_pkt, MAX_CHANNEL_DATA_SIZE);
    }

    // -----------------------------------------------------------------------
    // Shell session management
    // -----------------------------------------------------------------------

    #[test]
    fn test_shell_session_lifecycle() {
        let mut session = ShellSession::new(0);
        assert_eq!(session.state, ShellSessionState::Idle);

        let pty = PtyInfo {
            term_type: b"xterm".to_vec(),
            width_cols: 80,
            height_rows: 24,
            width_pixels: 0,
            height_pixels: 0,
            terminal_modes: Vec::new(),
        };
        session.allocate_pty(pty).unwrap();
        assert_eq!(session.state, ShellSessionState::PtyAllocated);

        session.start_shell().unwrap();
        assert_eq!(session.state, ShellSessionState::Running);

        session.mark_exited(0);
        assert_eq!(session.state, ShellSessionState::Exited);
        assert_eq!(session.exit_code, Some(0));
    }

    #[test]
    fn test_shell_session_exec() {
        let mut session = ShellSession::new(1);
        session.start_exec(b"uname -a".to_vec()).unwrap();
        assert_eq!(session.state, ShellSessionState::Running);
        assert_eq!(session.command.as_deref(), Some(b"uname -a".as_slice()));
    }

    #[test]
    fn test_shell_session_invalid_transition() {
        let mut session = ShellSession::new(0);
        session.start_shell().unwrap();
        // Cannot allocate PTY after shell started
        let pty = PtyInfo {
            term_type: b"vt100".to_vec(),
            width_cols: 80,
            height_rows: 24,
            width_pixels: 0,
            height_pixels: 0,
            terminal_modes: Vec::new(),
        };
        assert_eq!(session.allocate_pty(pty), Err(SshError::InvalidState));
    }

    // -----------------------------------------------------------------------
    // SSH server
    // -----------------------------------------------------------------------

    #[test]
    fn test_server_accept_connection() {
        let host_key = HostKeyPair::new([0x11; 32], [0x22; 64]);
        let mut server = SshServer::new(host_key, 22);
        let id = server.accept_connection().unwrap();
        assert_eq!(id, 0);
        assert_eq!(server.active_sessions(), 1);

        let session = server.get_session(id).unwrap();
        assert_eq!(session.state, SessionState::VersionExchange);
    }

    #[test]
    fn test_server_session_limit() {
        let host_key = HostKeyPair::new([0x11; 32], [0x22; 64]);
        let mut server = SshServer::new(host_key, 22);
        server.max_sessions = 2;

        server.accept_connection().unwrap();
        server.accept_connection().unwrap();
        assert_eq!(
            server.accept_connection(),
            Err(SshError::SessionLimitReached)
        );
    }

    // -----------------------------------------------------------------------
    // Exit status/signal
    // -----------------------------------------------------------------------

    #[test]
    fn test_exit_status_encode_parse() {
        let status = ExitStatus { code: 42 };
        let encoded = status.encode();
        let parsed = ExitStatus::parse(&encoded).unwrap();
        assert_eq!(parsed.code, 42);
    }

    #[test]
    fn test_exit_signal_encode_parse() {
        let sig = ExitSignal {
            signal_name: b"TERM".to_vec(),
            core_dumped: false,
            error_message: b"terminated".to_vec(),
            language_tag: b"".to_vec(),
        };
        let encoded = sig.encode();
        let parsed = ExitSignal::parse(&encoded).unwrap();
        assert_eq!(&parsed.signal_name, b"TERM");
        assert!(!parsed.core_dumped);
        assert_eq!(&parsed.error_message, b"terminated");
    }

    // -----------------------------------------------------------------------
    // Env request
    // -----------------------------------------------------------------------

    #[test]
    fn test_env_request_parse() {
        let mut data = Vec::new();
        write_ssh_string(&mut data, b"LANG");
        write_ssh_string(&mut data, b"en_US.UTF-8");
        let env = EnvRequest::parse(&data).unwrap();
        assert_eq!(&env.name, b"LANG");
        assert_eq!(&env.value, b"en_US.UTF-8");
    }

    // -----------------------------------------------------------------------
    // Host key encoding
    // -----------------------------------------------------------------------

    #[test]
    fn test_host_key_encode() {
        let hk = HostKeyPair::new([0xAA; 32], [0xBB; 64]);
        let encoded = hk.encode_public_key();
        // Should contain "ssh-ed25519" prefix
        let mut pos = 0;
        let algo = read_ssh_string(&encoded, &mut pos).unwrap();
        assert_eq!(&algo, b"ssh-ed25519");
        let key = read_ssh_string(&encoded, &mut pos).unwrap();
        assert_eq!(key.len(), 32);
    }

    // -----------------------------------------------------------------------
    // Sequence counters
    // -----------------------------------------------------------------------

    #[test]
    fn test_sequence_counters() {
        let mut session = SshSession::new(0);
        assert_eq!(session.next_send_seq(), 0);
        assert_eq!(session.next_send_seq(), 1);
        assert_eq!(session.next_recv_seq(), 0);
        assert_eq!(session.next_recv_seq(), 1);
    }

    // -----------------------------------------------------------------------
    // Banner
    // -----------------------------------------------------------------------

    #[test]
    fn test_banner_encode() {
        let msg = encode_banner("Welcome to VeridianOS SSH\r\n");
        assert_eq!(msg[0], SSH_MSG_USERAUTH_BANNER);
    }

    // -----------------------------------------------------------------------
    // Algorithm negotiation
    // -----------------------------------------------------------------------

    #[test]
    fn test_algorithm_from_str() {
        assert_eq!(
            AlgorithmId::parse_name("curve25519-sha256"),
            Some(AlgorithmId::Curve25519Sha256)
        );
        assert_eq!(
            AlgorithmId::parse_name("ssh-ed25519"),
            Some(AlgorithmId::SshEd25519)
        );
        assert_eq!(AlgorithmId::parse_name("unknown-algo"), None);
    }

    #[test]
    fn test_algorithm_roundtrip() {
        let algos = [
            AlgorithmId::Curve25519Sha256,
            AlgorithmId::SshEd25519,
            AlgorithmId::Chacha20Poly1305,
            AlgorithmId::Aes256Ctr,
            AlgorithmId::HmacSha256,
            AlgorithmId::None,
        ];
        for algo in &algos {
            let name = algo.as_str();
            let parsed = AlgorithmId::parse_name(name).unwrap();
            assert_eq!(*algo, parsed);
        }
    }
}
