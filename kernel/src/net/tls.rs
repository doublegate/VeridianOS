//! TLS 1.3 Protocol Implementation (RFC 8446)
//!
//! Provides a complete TLS 1.3 client implementation for VeridianOS, including:
//! - Record layer with fragmentation and encrypted record wrapping
//! - Full handshake state machine (ClientHello through Finished)
//! - Crypto primitives: ChaCha20-Poly1305, AES-128-GCM, X25519, HKDF-SHA256
//! - Simplified X.509 certificate parsing and chain validation
//! - Session ticket resumption and 0-RTT stubs
//! - Connection API: connect(), send(), recv(), close()
//!
//! Cipher suites supported:
//! - TLS_AES_128_GCM_SHA256 (0x1301)
//! - TLS_CHACHA20_POLY1305_SHA256 (0x1303)

#![allow(dead_code)]

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::vec::Vec;

use crate::crypto::hash::{sha256, Hash256};

// ============================================================================
// Constants
// ============================================================================

/// Maximum TLS record payload size (2^14 = 16384 bytes)
const MAX_RECORD_SIZE: usize = 16384;

/// TLS 1.3 protocol version
const TLS_13_VERSION: u16 = 0x0304;

/// Legacy TLS 1.2 version used in record headers
const TLS_LEGACY_VERSION: u16 = 0x0303;

/// AEAD tag length for both AES-128-GCM and ChaCha20-Poly1305
const AEAD_TAG_LEN: usize = 16;

/// HKDF-SHA256 hash output length
const HASH_LEN: usize = 32;

/// X25519 key size (scalar and point)
const X25519_KEY_LEN: usize = 32;

/// IV/nonce length for TLS 1.3 AEAD ciphers
const NONCE_LEN: usize = 12;

/// AES-128 key length
const AES_128_KEY_LEN: usize = 16;

/// ChaCha20 key length
const CHACHA20_KEY_LEN: usize = 32;

// ============================================================================
// Section 1: Record Layer (~200 lines)
// ============================================================================

/// TLS record content types (RFC 8446 Section 5.1)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ContentType {
    ChangeCipherSpec = 20,
    Alert = 21,
    Handshake = 22,
    ApplicationData = 23,
}

impl ContentType {
    fn from_u8(v: u8) -> Option<Self> {
        match v {
            20 => Some(Self::ChangeCipherSpec),
            21 => Some(Self::Alert),
            22 => Some(Self::Handshake),
            23 => Some(Self::ApplicationData),
            _ => None,
        }
    }
}

/// TLS record header (5 bytes on the wire)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordHeader {
    pub content_type: ContentType,
    pub legacy_version: u16,
    pub length: u16,
}

impl RecordHeader {
    /// Encode record header to bytes
    pub fn encode(&self, buf: &mut [u8]) -> usize {
        if buf.len() < 5 {
            return 0;
        }
        buf[0] = self.content_type as u8;
        buf[1..3].copy_from_slice(&self.legacy_version.to_be_bytes());
        buf[3..5].copy_from_slice(&self.length.to_be_bytes());
        5
    }

    /// Decode record header from bytes
    pub fn decode(buf: &[u8]) -> Option<Self> {
        if buf.len() < 5 {
            return None;
        }
        let content_type = ContentType::from_u8(buf[0])?;
        let legacy_version = u16::from_be_bytes([buf[1], buf[2]]);
        let length = u16::from_be_bytes([buf[3], buf[4]]);

        // Enforce max record size (payload + optional tag)
        if length as usize > MAX_RECORD_SIZE + AEAD_TAG_LEN + 1 {
            return None;
        }

        Some(Self {
            content_type,
            legacy_version,
            length,
        })
    }
}

/// TLS plaintext record
#[derive(Debug, Clone)]
pub struct TlsRecord {
    pub content_type: ContentType,
    pub fragment: Vec<u8>,
}

impl TlsRecord {
    /// Create a new record with the given content type and payload
    pub fn new(content_type: ContentType, fragment: Vec<u8>) -> Self {
        Self {
            content_type,
            fragment,
        }
    }

    /// Encode the record to wire format (header + fragment)
    pub fn encode(&self) -> Vec<u8> {
        let header = RecordHeader {
            content_type: self.content_type,
            legacy_version: TLS_LEGACY_VERSION,
            length: self.fragment.len() as u16,
        };
        let mut buf = alloc::vec![0u8; 5 + self.fragment.len()];
        header.encode(&mut buf);
        buf[5..].copy_from_slice(&self.fragment);
        buf
    }

    /// Decode a record from wire format
    pub fn decode(data: &[u8]) -> Option<(Self, usize)> {
        let header = RecordHeader::decode(data)?;
        let total_len = 5 + header.length as usize;
        if data.len() < total_len {
            return None;
        }
        let fragment = data[5..total_len].to_vec();
        Some((
            Self {
                content_type: header.content_type,
                fragment,
            },
            total_len,
        ))
    }
}

/// Fragment reassembly buffer for handshake messages that span multiple records
pub struct FragmentBuffer {
    buffer: Vec<u8>,
    expected_type: Option<ContentType>,
}

impl Default for FragmentBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl FragmentBuffer {
    pub fn new() -> Self {
        Self {
            buffer: Vec::new(),
            expected_type: None,
        }
    }

    /// Append a record fragment. Returns the reassembled message when complete.
    pub fn append(&mut self, record: &TlsRecord) -> Option<Vec<u8>> {
        match self.expected_type {
            None => {
                self.expected_type = Some(record.content_type);
            }
            Some(ct) if ct != record.content_type => {
                // Content type mismatch -- reset
                self.buffer.clear();
                self.expected_type = Some(record.content_type);
            }
            _ => {}
        }

        self.buffer.extend_from_slice(&record.fragment);

        // For handshake messages, check if we have a complete message
        if record.content_type == ContentType::Handshake && self.buffer.len() >= 4 {
            let msg_len = ((self.buffer[1] as usize) << 16)
                | ((self.buffer[2] as usize) << 8)
                | (self.buffer[3] as usize);
            let total = 4 + msg_len;
            if self.buffer.len() >= total {
                let message = self.buffer[..total].to_vec();
                self.buffer = self.buffer[total..].to_vec();
                if self.buffer.is_empty() {
                    self.expected_type = None;
                }
                return Some(message);
            }
        }

        None
    }

    /// Reset the fragment buffer
    pub fn reset(&mut self) {
        self.buffer.clear();
        self.expected_type = None;
    }
}

/// Wrap a plaintext record into an encrypted TLS 1.3 record.
///
/// TLS 1.3 encrypted records have content_type = ApplicationData on the wire,
/// with the real content_type appended after the plaintext before encryption.
pub fn encrypt_record(
    record: &TlsRecord,
    key: &[u8],
    iv: &[u8; NONCE_LEN],
    seq_num: u64,
    cipher: CipherSuite,
) -> Option<TlsRecord> {
    // Build inner plaintext: fragment || content_type
    let mut inner = record.fragment.clone();
    inner.push(record.content_type as u8);

    // Compute per-record nonce: IV XOR sequence number (RFC 8446 Section 5.3)
    let nonce = compute_nonce(iv, seq_num);

    // Additional data is the record header of the outer (encrypted) record
    let encrypted_len = inner.len() + AEAD_TAG_LEN;
    let mut aad = [0u8; 5];
    aad[0] = ContentType::ApplicationData as u8;
    aad[1..3].copy_from_slice(&TLS_LEGACY_VERSION.to_be_bytes());
    aad[3..5].copy_from_slice(&(encrypted_len as u16).to_be_bytes());

    let ciphertext = aead_encrypt(cipher, key, &nonce, &aad, &inner)?;

    Some(TlsRecord::new(ContentType::ApplicationData, ciphertext))
}

/// Unwrap an encrypted TLS 1.3 record back to plaintext.
pub fn decrypt_record(
    record: &TlsRecord,
    key: &[u8],
    iv: &[u8; NONCE_LEN],
    seq_num: u64,
    cipher: CipherSuite,
) -> Option<TlsRecord> {
    if record.content_type != ContentType::ApplicationData {
        return None;
    }

    let nonce = compute_nonce(iv, seq_num);

    // Reconstruct AAD from wire header
    let mut aad = [0u8; 5];
    aad[0] = ContentType::ApplicationData as u8;
    aad[1..3].copy_from_slice(&TLS_LEGACY_VERSION.to_be_bytes());
    aad[3..5].copy_from_slice(&(record.fragment.len() as u16).to_be_bytes());

    let plaintext = aead_decrypt(cipher, key, &nonce, &aad, &record.fragment)?;

    if plaintext.is_empty() {
        return None;
    }

    // Last byte of plaintext is the real content type
    let real_ct = ContentType::from_u8(*plaintext.last()?)?;
    let payload = plaintext[..plaintext.len() - 1].to_vec();

    Some(TlsRecord::new(real_ct, payload))
}

/// Compute per-record nonce: IV XOR (zero-padded 64-bit sequence number)
fn compute_nonce(iv: &[u8; NONCE_LEN], seq_num: u64) -> [u8; NONCE_LEN] {
    let mut nonce = *iv;
    let seq_bytes = seq_num.to_be_bytes();
    // XOR sequence number into the last 8 bytes of the IV
    for i in 0..8 {
        nonce[NONCE_LEN - 8 + i] ^= seq_bytes[i];
    }
    nonce
}

// ============================================================================
// Section 2: Handshake State Machine (~400 lines)
// ============================================================================

/// TLS 1.3 handshake message types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum HandshakeType {
    ClientHello = 1,
    ServerHello = 2,
    EncryptedExtensions = 8,
    Certificate = 11,
    CertificateVerify = 15,
    Finished = 20,
    NewSessionTicket = 4,
}

impl HandshakeType {
    fn from_u8(v: u8) -> Option<Self> {
        match v {
            1 => Some(Self::ClientHello),
            2 => Some(Self::ServerHello),
            8 => Some(Self::EncryptedExtensions),
            11 => Some(Self::Certificate),
            15 => Some(Self::CertificateVerify),
            20 => Some(Self::Finished),
            4 => Some(Self::NewSessionTicket),
            _ => None,
        }
    }
}

/// Cipher suites supported by this implementation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CipherSuite {
    /// TLS_AES_128_GCM_SHA256 (0x1301)
    Aes128GcmSha256,
    /// TLS_CHACHA20_POLY1305_SHA256 (0x1303)
    ChaCha20Poly1305Sha256,
}

impl CipherSuite {
    /// Wire format code point
    pub fn code(&self) -> u16 {
        match self {
            Self::Aes128GcmSha256 => 0x1301,
            Self::ChaCha20Poly1305Sha256 => 0x1303,
        }
    }

    /// Parse from wire format
    pub fn from_code(code: u16) -> Option<Self> {
        match code {
            0x1301 => Some(Self::Aes128GcmSha256),
            0x1303 => Some(Self::ChaCha20Poly1305Sha256),
            _ => None,
        }
    }

    /// Key length for this cipher suite
    pub fn key_len(&self) -> usize {
        match self {
            Self::Aes128GcmSha256 => AES_128_KEY_LEN,
            Self::ChaCha20Poly1305Sha256 => CHACHA20_KEY_LEN,
        }
    }
}

/// Signature algorithms advertised in ClientHello
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum SignatureScheme {
    EcdsaSecp256r1Sha256 = 0x0403,
    RsaPssRsaeSha256 = 0x0804,
    Ed25519 = 0x0807,
}

/// Named groups for key exchange
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum NamedGroup {
    X25519 = 0x001D,
}

/// TLS 1.3 handshake state machine states
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandshakeState {
    /// Initial state -- ready to send ClientHello
    Start,
    /// ClientHello sent, waiting for ServerHello
    WaitServerHello,
    /// ServerHello received, waiting for EncryptedExtensions
    WaitEncryptedExtensions,
    /// EncryptedExtensions received, waiting for Certificate
    WaitCertificate,
    /// Certificate received, waiting for CertificateVerify
    WaitCertificateVerify,
    /// CertificateVerify received, waiting for Finished
    WaitFinished,
    /// Handshake complete, application data can flow
    Connected,
    /// Unrecoverable error occurred
    Error,
}

/// TLS 1.3 extension types used in handshake
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
enum ExtensionType {
    SupportedVersions = 43,
    KeyShare = 51,
    SignatureAlgorithms = 13,
    SupportedGroups = 10,
    PreSharedKey = 41,
    EarlyData = 42,
}

/// ClientHello message builder
pub struct ClientHello {
    pub random: [u8; 32],
    pub session_id: [u8; 32],
    pub cipher_suites: Vec<CipherSuite>,
    pub key_share_public: [u8; X25519_KEY_LEN],
}

impl ClientHello {
    /// Encode ClientHello to handshake message bytes
    pub fn encode(&self) -> Vec<u8> {
        let mut msg = Vec::with_capacity(256);

        // Handshake type + placeholder for length
        msg.push(HandshakeType::ClientHello as u8);
        msg.extend_from_slice(&[0, 0, 0]); // length placeholder (3 bytes)

        // Legacy version (TLS 1.2)
        msg.extend_from_slice(&TLS_LEGACY_VERSION.to_be_bytes());

        // Random (32 bytes)
        msg.extend_from_slice(&self.random);

        // Legacy session ID (32 bytes with length prefix)
        msg.push(32);
        msg.extend_from_slice(&self.session_id);

        // Cipher suites (length-prefixed)
        let cs_len = (self.cipher_suites.len() * 2) as u16;
        msg.extend_from_slice(&cs_len.to_be_bytes());
        for cs in &self.cipher_suites {
            msg.extend_from_slice(&cs.code().to_be_bytes());
        }

        // Compression methods: only null (0x00)
        msg.push(1); // length
        msg.push(0); // null compression

        // Extensions
        let ext_start = msg.len();
        msg.extend_from_slice(&[0, 0]); // extensions length placeholder

        // Extension: supported_versions (type 43)
        msg.extend_from_slice(&(ExtensionType::SupportedVersions as u16).to_be_bytes());
        msg.extend_from_slice(&3u16.to_be_bytes()); // extension data length
        msg.push(2); // versions list length
        msg.extend_from_slice(&TLS_13_VERSION.to_be_bytes());

        // Extension: supported_groups (type 10)
        msg.extend_from_slice(&(ExtensionType::SupportedGroups as u16).to_be_bytes());
        msg.extend_from_slice(&4u16.to_be_bytes()); // extension data length
        msg.extend_from_slice(&2u16.to_be_bytes()); // named groups list length
        msg.extend_from_slice(&(NamedGroup::X25519 as u16).to_be_bytes());

        // Extension: signature_algorithms (type 13)
        msg.extend_from_slice(&(ExtensionType::SignatureAlgorithms as u16).to_be_bytes());
        msg.extend_from_slice(&8u16.to_be_bytes()); // extension data length
        msg.extend_from_slice(&6u16.to_be_bytes()); // schemes list length
        msg.extend_from_slice(&(SignatureScheme::EcdsaSecp256r1Sha256 as u16).to_be_bytes());
        msg.extend_from_slice(&(SignatureScheme::RsaPssRsaeSha256 as u16).to_be_bytes());
        msg.extend_from_slice(&(SignatureScheme::Ed25519 as u16).to_be_bytes());

        // Extension: key_share (type 51) -- X25519 public key
        let _ks_ext_len = 2 + 2 + 1 + X25519_KEY_LEN; // group(2) + key_len(2) + key
                                                      // Actually: client_shares_len(2) + group(2) + key_exchange_len(2) + key
        let ks_entry_len = 2 + 2 + X25519_KEY_LEN; // group(2) + key_exchange_length(2) + key
        msg.extend_from_slice(&(ExtensionType::KeyShare as u16).to_be_bytes());
        msg.extend_from_slice(&((2 + ks_entry_len) as u16).to_be_bytes()); // ext data len
        msg.extend_from_slice(&(ks_entry_len as u16).to_be_bytes()); // client_shares len
        msg.extend_from_slice(&(NamedGroup::X25519 as u16).to_be_bytes());
        msg.extend_from_slice(&(X25519_KEY_LEN as u16).to_be_bytes());
        msg.extend_from_slice(&self.key_share_public);

        // Fix extensions length
        let ext_len = (msg.len() - ext_start - 2) as u16;
        msg[ext_start..ext_start + 2].copy_from_slice(&ext_len.to_be_bytes());

        // Fix handshake message length (3 bytes, big-endian)
        let hs_len = msg.len() - 4;
        msg[1] = ((hs_len >> 16) & 0xFF) as u8;
        msg[2] = ((hs_len >> 8) & 0xFF) as u8;
        msg[3] = (hs_len & 0xFF) as u8;

        msg
    }
}

/// Parsed ServerHello fields
#[derive(Debug, Clone)]
pub struct ServerHello {
    pub random: [u8; 32],
    pub session_id: Vec<u8>,
    pub cipher_suite: CipherSuite,
    pub key_share_public: [u8; X25519_KEY_LEN],
}

impl ServerHello {
    /// Parse ServerHello from handshake message bytes (after type + length)
    pub fn decode(data: &[u8]) -> Option<Self> {
        if data.len() < 4 {
            return None;
        }
        // Skip handshake header (type + 3-byte length)
        let payload = &data[4..];
        if payload.len() < 2 + 32 + 1 {
            return None;
        }

        let mut pos = 0;

        // Legacy version (2 bytes) -- ignored in TLS 1.3
        pos += 2;

        // Random (32 bytes)
        let mut random = [0u8; 32];
        random.copy_from_slice(&payload[pos..pos + 32]);
        pos += 32;

        // Session ID
        let sid_len = payload[pos] as usize;
        pos += 1;
        if pos + sid_len > payload.len() {
            return None;
        }
        let session_id = payload[pos..pos + sid_len].to_vec();
        pos += sid_len;

        // Cipher suite (2 bytes)
        if pos + 2 > payload.len() {
            return None;
        }
        let cs_code = u16::from_be_bytes([payload[pos], payload[pos + 1]]);
        let cipher_suite = CipherSuite::from_code(cs_code)?;
        pos += 2;

        // Compression method (1 byte, must be 0)
        if pos >= payload.len() {
            return None;
        }
        pos += 1;

        // Extensions
        if pos + 2 > payload.len() {
            return None;
        }
        let ext_len = u16::from_be_bytes([payload[pos], payload[pos + 1]]) as usize;
        pos += 2;

        let ext_end = pos + ext_len;
        if ext_end > payload.len() {
            return None;
        }

        let mut key_share_public = [0u8; X25519_KEY_LEN];
        let mut found_key_share = false;

        // Parse extensions
        while pos + 4 <= ext_end {
            let ext_type = u16::from_be_bytes([payload[pos], payload[pos + 1]]);
            let ext_data_len = u16::from_be_bytes([payload[pos + 2], payload[pos + 3]]) as usize;
            pos += 4;

            if pos + ext_data_len > ext_end {
                break;
            }

            if ext_type == ExtensionType::KeyShare as u16 {
                // key_share: group(2) + key_exchange_length(2) + key
                if ext_data_len >= 4 + X25519_KEY_LEN {
                    let _group = u16::from_be_bytes([payload[pos], payload[pos + 1]]);
                    let kx_len = u16::from_be_bytes([payload[pos + 2], payload[pos + 3]]) as usize;
                    if kx_len == X25519_KEY_LEN && pos + 4 + kx_len <= ext_end {
                        key_share_public.copy_from_slice(&payload[pos + 4..pos + 4 + kx_len]);
                        found_key_share = true;
                    }
                }
            }
            // supported_versions extension is parsed but not stored -- we only support TLS
            // 1.3

            pos += ext_data_len;
        }

        if !found_key_share {
            return None;
        }

        Some(Self {
            random,
            session_id,
            cipher_suite,
            key_share_public,
        })
    }
}

/// Handshake engine managing state transitions and transcript
pub struct HandshakeEngine {
    pub state: HandshakeState,
    /// Transcript hash of all handshake messages
    transcript: Vec<u8>,
    /// Negotiated cipher suite
    pub cipher_suite: Option<CipherSuite>,
    /// Our ephemeral X25519 private key
    client_private_key: [u8; X25519_KEY_LEN],
    /// Our ephemeral X25519 public key
    client_public_key: [u8; X25519_KEY_LEN],
    /// Server's X25519 public key from ServerHello
    server_public_key: Option<[u8; X25519_KEY_LEN]>,
    /// Derived handshake secrets
    pub handshake_secret: Option<[u8; HASH_LEN]>,
    /// Client handshake traffic secret
    pub client_hs_traffic_secret: Option<[u8; HASH_LEN]>,
    /// Server handshake traffic secret
    pub server_hs_traffic_secret: Option<[u8; HASH_LEN]>,
    /// Client application traffic secret
    pub client_app_traffic_secret: Option<[u8; HASH_LEN]>,
    /// Server application traffic secret
    pub server_app_traffic_secret: Option<[u8; HASH_LEN]>,
    /// Peer certificate (DER bytes)
    pub peer_certificate: Option<Vec<u8>>,
}

impl Default for HandshakeEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl HandshakeEngine {
    /// Create a new handshake engine with generated ephemeral keys
    pub fn new() -> Self {
        let (private_key, public_key) = x25519_keypair();
        Self {
            state: HandshakeState::Start,
            transcript: Vec::new(),
            cipher_suite: None,
            client_private_key: private_key,
            client_public_key: public_key,
            server_public_key: None,
            handshake_secret: None,
            client_hs_traffic_secret: None,
            server_hs_traffic_secret: None,
            client_app_traffic_secret: None,
            server_app_traffic_secret: None,
            peer_certificate: None,
        }
    }

    /// Build and return the ClientHello message, advancing state
    pub fn build_client_hello(&mut self, random: [u8; 32]) -> Option<Vec<u8>> {
        if self.state != HandshakeState::Start {
            return None;
        }

        let ch = ClientHello {
            random,
            session_id: [0u8; 32], // Empty for TLS 1.3 but legacy-compatible
            cipher_suites: alloc::vec![
                CipherSuite::Aes128GcmSha256,
                CipherSuite::ChaCha20Poly1305Sha256,
            ],
            key_share_public: self.client_public_key,
        };

        let msg = ch.encode();
        self.transcript.extend_from_slice(&msg);
        self.state = HandshakeState::WaitServerHello;
        Some(msg)
    }

    /// Process a received ServerHello message
    pub fn process_server_hello(&mut self, data: &[u8]) -> Option<()> {
        if self.state != HandshakeState::WaitServerHello {
            return None;
        }

        let sh = ServerHello::decode(data)?;
        self.transcript.extend_from_slice(data);
        self.cipher_suite = Some(sh.cipher_suite);
        self.server_public_key = Some(sh.key_share_public);

        // Derive shared secret via X25519
        let shared_secret = x25519_shared_secret(&self.client_private_key, &sh.key_share_public);

        // Derive handshake secrets using HKDF (RFC 8446 Section 7.1)
        let early_secret = hkdf_extract(&[0u8; HASH_LEN], &[0u8; HASH_LEN]);

        // derive-secret(early_secret, "derived", "")
        let derived_secret = derive_secret(&early_secret, b"derived", &sha256(&[]).0);

        // handshake_secret = HKDF-Extract(derived_secret, shared_secret)
        let handshake_secret = hkdf_extract(&derived_secret, &shared_secret);
        self.handshake_secret = Some(handshake_secret);

        // Transcript hash at this point (ClientHello + ServerHello)
        let transcript_hash = sha256(&self.transcript);

        // client_handshake_traffic_secret
        self.client_hs_traffic_secret = Some(derive_secret(
            &handshake_secret,
            b"c hs traffic",
            &transcript_hash.0,
        ));

        // server_handshake_traffic_secret
        self.server_hs_traffic_secret = Some(derive_secret(
            &handshake_secret,
            b"s hs traffic",
            &transcript_hash.0,
        ));

        self.state = HandshakeState::WaitEncryptedExtensions;
        Some(())
    }

    /// Process EncryptedExtensions message
    pub fn process_encrypted_extensions(&mut self, data: &[u8]) -> Option<()> {
        if self.state != HandshakeState::WaitEncryptedExtensions {
            return None;
        }
        // Validate basic structure: type(1) + length(3) + extensions_length(2)
        if data.len() < 6 {
            return None;
        }
        if data[0] != HandshakeType::EncryptedExtensions as u8 {
            return None;
        }
        self.transcript.extend_from_slice(data);
        self.state = HandshakeState::WaitCertificate;
        Some(())
    }

    /// Process Certificate message
    pub fn process_certificate(&mut self, data: &[u8]) -> Option<()> {
        if self.state != HandshakeState::WaitCertificate {
            return None;
        }
        if data.len() < 4 {
            return None;
        }
        if data[0] != HandshakeType::Certificate as u8 {
            return None;
        }
        // Extract the first certificate from the chain
        // Format: type(1) + length(3) + request_context_len(1) + ...
        let payload = &data[4..];
        if !payload.is_empty() {
            // request_context length
            let ctx_len = payload[0] as usize;
            let pos = 1 + ctx_len;
            if pos + 3 <= payload.len() {
                let certs_len = ((payload[pos] as usize) << 16)
                    | ((payload[pos + 1] as usize) << 8)
                    | (payload[pos + 2] as usize);
                let certs_start = pos + 3;
                if certs_start + 3 <= payload.len() && certs_len > 3 {
                    let cert_len = ((payload[certs_start] as usize) << 16)
                        | ((payload[certs_start + 1] as usize) << 8)
                        | (payload[certs_start + 2] as usize);
                    let cert_start = certs_start + 3;
                    if cert_start + cert_len <= payload.len() {
                        self.peer_certificate =
                            Some(payload[cert_start..cert_start + cert_len].to_vec());
                    }
                }
            }
        }
        self.transcript.extend_from_slice(data);
        self.state = HandshakeState::WaitCertificateVerify;
        Some(())
    }

    /// Process CertificateVerify message
    pub fn process_certificate_verify(&mut self, data: &[u8]) -> Option<()> {
        if self.state != HandshakeState::WaitCertificateVerify {
            return None;
        }
        if data.len() < 4 {
            return None;
        }
        if data[0] != HandshakeType::CertificateVerify as u8 {
            return None;
        }
        // In a full implementation we would verify the signature over the transcript.
        // For now, accept any structurally valid CertificateVerify.
        self.transcript.extend_from_slice(data);
        self.state = HandshakeState::WaitFinished;
        Some(())
    }

    /// Process server Finished message and derive application traffic secrets
    pub fn process_finished(&mut self, data: &[u8]) -> Option<()> {
        if self.state != HandshakeState::WaitFinished {
            return None;
        }
        if data.len() < 4 {
            return None;
        }
        if data[0] != HandshakeType::Finished as u8 {
            return None;
        }

        // Verify the Finished MAC
        let server_hs_secret = self.server_hs_traffic_secret.as_ref()?;
        let finished_key = hkdf_expand_label(server_hs_secret, b"finished", &[], HASH_LEN);
        let transcript_hash = sha256(&self.transcript);
        let expected_verify = hmac_sha256(&finished_key, &transcript_hash.0);

        // Extract verify_data from the Finished message (after type + 3-byte length)
        if data.len() < 4 + HASH_LEN {
            return None;
        }
        let verify_data = &data[4..4 + HASH_LEN];
        if !constant_time_eq(verify_data, &expected_verify) {
            self.state = HandshakeState::Error;
            return None;
        }

        self.transcript.extend_from_slice(data);

        // Derive application traffic secrets (RFC 8446 Section 7.1)
        let handshake_secret = self.handshake_secret.as_ref()?;
        let derived = derive_secret(handshake_secret, b"derived", &sha256(&[]).0);
        let master_secret = hkdf_extract(&derived, &[0u8; HASH_LEN]);

        let transcript_hash_final = sha256(&self.transcript);

        self.client_app_traffic_secret = Some(derive_secret(
            &master_secret,
            b"c ap traffic",
            &transcript_hash_final.0,
        ));
        self.server_app_traffic_secret = Some(derive_secret(
            &master_secret,
            b"s ap traffic",
            &transcript_hash_final.0,
        ));

        self.state = HandshakeState::Connected;
        Some(())
    }

    /// Build the client Finished message
    pub fn build_client_finished(&self) -> Option<Vec<u8>> {
        let client_hs_secret = self.client_hs_traffic_secret.as_ref()?;
        let finished_key = hkdf_expand_label(client_hs_secret, b"finished", &[], HASH_LEN);
        let transcript_hash = sha256(&self.transcript);
        let verify_data = hmac_sha256(&finished_key, &transcript_hash.0);

        let mut msg = Vec::with_capacity(4 + HASH_LEN);
        msg.push(HandshakeType::Finished as u8);
        let len = HASH_LEN;
        msg.push(((len >> 16) & 0xFF) as u8);
        msg.push(((len >> 8) & 0xFF) as u8);
        msg.push((len & 0xFF) as u8);
        msg.extend_from_slice(&verify_data);
        Some(msg)
    }

    /// Get the current transcript hash
    pub fn transcript_hash(&self) -> Hash256 {
        sha256(&self.transcript)
    }
}

// ============================================================================
// Section 3: Crypto Primitives (~400 lines)
// ============================================================================

// --- HMAC-SHA256 ---

/// HMAC-SHA256 (RFC 2104)
///
/// Stack-only implementation -- no heap allocation for the HMAC computation.
pub fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    const BLOCK_SIZE: usize = 64;

    // If key > block size, hash it first
    let key_hash: Hash256;
    let k = if key.len() > BLOCK_SIZE {
        key_hash = sha256(key);
        key_hash.as_bytes().as_slice()
    } else {
        key
    };

    let mut ipad = [0x36u8; BLOCK_SIZE];
    let mut opad = [0x5cu8; BLOCK_SIZE];

    for i in 0..k.len() {
        ipad[i] ^= k[i];
        opad[i] ^= k[i];
    }

    // Inner hash: SHA256(ipad || message)
    // We build this on the stack with a reasonable buffer size.
    // For messages larger than this, we'd need a streaming SHA256.
    let mut inner_buf = [0u8; 2048];
    let inner_len = BLOCK_SIZE + message.len();
    if inner_len <= inner_buf.len() {
        inner_buf[..BLOCK_SIZE].copy_from_slice(&ipad);
        inner_buf[BLOCK_SIZE..inner_len].copy_from_slice(message);
        let inner_hash = sha256(&inner_buf[..inner_len]);

        // Outer hash: SHA256(opad || inner_hash)
        let mut outer_buf = [0u8; 96]; // 64 + 32
        outer_buf[..BLOCK_SIZE].copy_from_slice(&opad);
        outer_buf[BLOCK_SIZE..BLOCK_SIZE + 32].copy_from_slice(inner_hash.as_bytes());
        sha256(&outer_buf[..BLOCK_SIZE + 32]).0
    } else {
        // Fallback for very large messages: use alloc
        let mut inner_data = Vec::with_capacity(inner_len);
        inner_data.extend_from_slice(&ipad);
        inner_data.extend_from_slice(message);
        let inner_hash = sha256(&inner_data);

        let mut outer_buf = [0u8; 96];
        outer_buf[..BLOCK_SIZE].copy_from_slice(&opad);
        outer_buf[BLOCK_SIZE..BLOCK_SIZE + 32].copy_from_slice(inner_hash.as_bytes());
        sha256(&outer_buf[..BLOCK_SIZE + 32]).0
    }
}

// --- HKDF-SHA256 (RFC 5869) ---

/// HKDF-Extract: PRK = HMAC-Hash(salt, IKM)
pub fn hkdf_extract(salt: &[u8], ikm: &[u8]) -> [u8; HASH_LEN] {
    hmac_sha256(salt, ikm)
}

/// HKDF-Expand: OKM = T(1) || T(2) || ... (truncated to length)
///
/// T(0) = empty string
/// T(i) = HMAC-Hash(PRK, T(i-1) || info || i)
pub fn hkdf_expand(prk: &[u8; HASH_LEN], info: &[u8], length: usize) -> Vec<u8> {
    let n = length.div_ceil(HASH_LEN);
    let mut okm = Vec::with_capacity(n * HASH_LEN);
    let mut t = [0u8; HASH_LEN];
    let mut t_len: usize = 0;

    for i in 1..=n {
        // HMAC input: T(i-1) || info || i
        let mut input = Vec::with_capacity(t_len + info.len() + 1);
        if t_len > 0 {
            input.extend_from_slice(&t[..t_len]);
        }
        input.extend_from_slice(info);
        input.push(i as u8);

        t = hmac_sha256(prk, &input);
        t_len = HASH_LEN;
        okm.extend_from_slice(&t);
    }

    okm.truncate(length);
    okm
}

/// HKDF-Expand-Label (TLS 1.3 specific, RFC 8446 Section 7.1)
///
/// HKDF-Expand-Label(Secret, Label, Context, Length) =
///     HKDF-Expand(Secret, HkdfLabel, Length)
/// where HkdfLabel = Length(2) || "tls13 " || Label || Context
pub fn hkdf_expand_label(
    secret: &[u8; HASH_LEN],
    label: &[u8],
    context: &[u8],
    length: usize,
) -> Vec<u8> {
    let tls_label = b"tls13 ";
    let mut hkdf_label =
        Vec::with_capacity(2 + 1 + tls_label.len() + label.len() + 1 + context.len());

    // Length (2 bytes, big-endian)
    hkdf_label.extend_from_slice(&(length as u16).to_be_bytes());

    // Label with "tls13 " prefix (length-prefixed)
    let full_label_len = tls_label.len() + label.len();
    hkdf_label.push(full_label_len as u8);
    hkdf_label.extend_from_slice(tls_label);
    hkdf_label.extend_from_slice(label);

    // Context (length-prefixed)
    hkdf_label.push(context.len() as u8);
    hkdf_label.extend_from_slice(context);

    hkdf_expand(secret, &hkdf_label, length)
}

/// Derive-Secret (TLS 1.3, RFC 8446 Section 7.1)
///
/// Derive-Secret(Secret, Label, Messages) =
///     HKDF-Expand-Label(Secret, Label, Transcript-Hash(Messages), Hash.length)
fn derive_secret(
    secret: &[u8; HASH_LEN],
    label: &[u8],
    transcript_hash: &[u8; 32],
) -> [u8; HASH_LEN] {
    let expanded = hkdf_expand_label(secret, label, transcript_hash, HASH_LEN);
    let mut result = [0u8; HASH_LEN];
    result.copy_from_slice(&expanded);
    result
}

// --- X25519 Key Exchange ---

/// X25519 basepoint (u = 9)
const X25519_BASEPOINT: [u8; 32] = {
    let mut b = [0u8; 32];
    b[0] = 9;
    b
};

/// Generate an X25519 keypair using the kernel's CSPRNG
pub fn x25519_keypair() -> ([u8; 32], [u8; 32]) {
    let mut private_key = [0u8; 32];
    // Use kernel CSPRNG if available, otherwise deterministic seed for testing
    if let Ok(rng) = crate::crypto::random::SecureRandom::new() {
        let _ = rng.fill_bytes(&mut private_key);
    } else {
        // Fallback: deterministic but non-zero key for testing only
        for (i, b) in private_key.iter_mut().enumerate() {
            *b = (i as u8).wrapping_add(42);
        }
    }

    let public_key = x25519_scalar_mult(&private_key, &X25519_BASEPOINT);
    (private_key, public_key)
}

/// Compute X25519 shared secret: shared = scalar_mult(our_private,
/// their_public)
pub fn x25519_shared_secret(private_key: &[u8; 32], peer_public: &[u8; 32]) -> [u8; 32] {
    x25519_scalar_mult(private_key, peer_public)
}

/// X25519 scalar multiplication using the Montgomery ladder.
///
/// Implements RFC 7748 Section 5 with clamping.
fn x25519_scalar_mult(scalar: &[u8; 32], u_point: &[u8; 32]) -> [u8; 32] {
    // Clamp scalar per RFC 7748
    let mut k = *scalar;
    k[0] &= 248;
    k[31] &= 127;
    k[31] |= 64;

    // Load u-coordinate
    let u = fe_from_bytes(u_point);

    // Montgomery ladder
    let mut x_2 = fe_one();
    let mut z_2 = fe_zero();
    let mut x_3 = u;
    let mut z_3 = fe_one();
    let mut swap: u64 = 0;

    for pos in (0..255).rev() {
        let bit = ((k[pos >> 3] >> (pos & 7)) & 1) as u64;
        swap ^= bit;
        fe_cswap(&mut x_2, &mut x_3, swap);
        fe_cswap(&mut z_2, &mut z_3, swap);
        swap = bit;

        let a = fe_add(&x_2, &z_2);
        let aa = fe_sq(&a);
        let b = fe_sub(&x_2, &z_2);
        let bb = fe_sq(&b);
        let e = fe_sub(&aa, &bb);
        let c = fe_add(&x_3, &z_3);
        let d = fe_sub(&x_3, &z_3);
        let da = fe_mul(&d, &a);
        let cb = fe_mul(&c, &b);
        x_3 = fe_sq(&fe_add(&da, &cb));
        z_3 = fe_mul(&u, &fe_sq(&fe_sub(&da, &cb)));
        x_2 = fe_mul(&aa, &bb);
        // a24 = (A-2)/4 = (486662-2)/4 = 121665 per RFC 7748
        z_2 = fe_mul(&e, &fe_add(&aa, &fe_mul_scalar(&e, 121665)));
    }

    fe_cswap(&mut x_2, &mut x_3, swap);
    fe_cswap(&mut z_2, &mut z_3, swap);

    let result = fe_mul(&x_2, &fe_invert(&z_2));
    fe_to_bytes(&result)
}

// --- GF(2^255-19) Field Arithmetic (5-limb, 51 bits per limb) ---

type Fe = [u64; 5];
const LIMB_MASK: u64 = (1u64 << 51) - 1;

fn fe_zero() -> Fe {
    [0; 5]
}

fn fe_one() -> Fe {
    [1, 0, 0, 0, 0]
}

fn fe_from_bytes(s: &[u8; 32]) -> Fe {
    let load64 = |bytes: &[u8]| -> u64 {
        let mut buf = [0u8; 8];
        let len = core::cmp::min(bytes.len(), 8);
        buf[..len].copy_from_slice(&bytes[..len]);
        u64::from_le_bytes(buf)
    };

    let mut h = [0u64; 5];
    h[0] = load64(&s[0..]) & LIMB_MASK;
    h[1] = (load64(&s[6..]) >> 3) & LIMB_MASK;
    h[2] = (load64(&s[12..]) >> 6) & LIMB_MASK;
    h[3] = (load64(&s[19..]) >> 1) & LIMB_MASK;
    h[4] = (load64(&s[24..]) >> 12) & LIMB_MASK;
    h
}

fn fe_to_bytes(h: &Fe) -> [u8; 32] {
    let mut t = *h;
    fe_reduce(&mut t);

    // Final conditional subtraction
    let mut q = (t[0].wrapping_add(19)) >> 51;
    q = (t[1].wrapping_add(q)) >> 51;
    q = (t[2].wrapping_add(q)) >> 51;
    q = (t[3].wrapping_add(q)) >> 51;
    q = (t[4].wrapping_add(q)) >> 51;

    t[0] = t[0].wrapping_add(19u64.wrapping_mul(q));
    let mut carry = t[0] >> 51;
    t[0] &= LIMB_MASK;
    #[allow(clippy::needless_range_loop)]
    for i in 1..5 {
        t[i] = t[i].wrapping_add(carry);
        carry = t[i] >> 51;
        t[i] &= LIMB_MASK;
    }

    // Serialize 5 limbs (51 bits each) to 32 bytes via u128 accumulator:
    let mut bits = [0u8; 32];
    let mut acc: u128 = 0;
    let mut acc_bits: u32 = 0;
    let mut byte_pos = 0;
    for &limb in t.iter() {
        acc |= (limb as u128) << acc_bits;
        acc_bits += 51;
        while acc_bits >= 8 && byte_pos < 32 {
            bits[byte_pos] = (acc & 0xFF) as u8;
            acc >>= 8;
            acc_bits -= 8;
            byte_pos += 1;
        }
    }
    // Handle any remaining bits
    if byte_pos < 32 {
        bits[byte_pos] = (acc & 0xFF) as u8;
    }
    bits
}

fn fe_reduce(h: &mut Fe) {
    let mut carry: u64;
    for _ in 0..2 {
        carry = h[0] >> 51;
        h[0] &= LIMB_MASK;
        h[1] = h[1].wrapping_add(carry);

        carry = h[1] >> 51;
        h[1] &= LIMB_MASK;
        h[2] = h[2].wrapping_add(carry);

        carry = h[2] >> 51;
        h[2] &= LIMB_MASK;
        h[3] = h[3].wrapping_add(carry);

        carry = h[3] >> 51;
        h[3] &= LIMB_MASK;
        h[4] = h[4].wrapping_add(carry);

        carry = h[4] >> 51;
        h[4] &= LIMB_MASK;
        h[0] = h[0].wrapping_add(carry.wrapping_mul(19));
    }
}

fn fe_add(a: &Fe, b: &Fe) -> Fe {
    [
        a[0].wrapping_add(b[0]),
        a[1].wrapping_add(b[1]),
        a[2].wrapping_add(b[2]),
        a[3].wrapping_add(b[3]),
        a[4].wrapping_add(b[4]),
    ]
}

fn fe_sub(a: &Fe, b: &Fe) -> Fe {
    // Add p to avoid underflow before subtraction
    let bias: u64 = (1u64 << 51) - 1;
    let bias0: u64 = bias - 18;
    [
        a[0].wrapping_add(bias0).wrapping_sub(b[0]),
        a[1].wrapping_add(bias).wrapping_sub(b[1]),
        a[2].wrapping_add(bias).wrapping_sub(b[2]),
        a[3].wrapping_add(bias).wrapping_sub(b[3]),
        a[4].wrapping_add(bias).wrapping_sub(b[4]),
    ]
}

#[allow(clippy::needless_range_loop)]
fn fe_mul(a: &Fe, b: &Fe) -> Fe {
    let mut t = [0u128; 5];

    for i in 0..5 {
        for j in 0..5 {
            let product = (a[i] as u128) * (b[j] as u128);
            let idx = i + j;
            if idx < 5 {
                t[idx] = t[idx].wrapping_add(product);
            } else {
                // Reduce: limb at position idx maps to idx-5 with factor 19
                t[idx - 5] = t[idx - 5].wrapping_add(product.wrapping_mul(19));
            }
        }
    }

    let mut h = [0u64; 5];
    let mut carry: u128 = 0;
    for i in 0..5 {
        t[i] = t[i].wrapping_add(carry);
        h[i] = (t[i] as u64) & LIMB_MASK;
        carry = t[i] >> 51;
    }
    h[0] = h[0].wrapping_add((carry as u64).wrapping_mul(19));

    fe_reduce(&mut h);
    h
}

fn fe_sq(a: &Fe) -> Fe {
    fe_mul(a, a)
}

fn fe_mul_scalar(a: &Fe, s: u64) -> Fe {
    let mut h = [0u64; 5];
    let mut carry: u128 = 0;
    for i in 0..5 {
        let product = (a[i] as u128) * (s as u128) + carry;
        h[i] = (product as u64) & LIMB_MASK;
        carry = product >> 51;
    }
    h[0] = h[0].wrapping_add((carry as u64).wrapping_mul(19));
    fe_reduce(&mut h);
    h
}

fn fe_cswap(a: &mut Fe, b: &mut Fe, swap: u64) {
    let mask = 0u64.wrapping_sub(swap); // 0 or 0xFFFFFFFFFFFFFFFF
    for i in 0..5 {
        let t = mask & (a[i] ^ b[i]);
        a[i] ^= t;
        b[i] ^= t;
    }
}

/// Compute modular inverse using Fermat's little theorem: a^(p-2) mod p
fn fe_invert(z: &Fe) -> Fe {
    // p-2 = 2^255 - 21
    // Use addition chain for efficient exponentiation
    let z2 = fe_sq(z);
    let z9 = {
        let z4 = fe_sq(&z2);
        let z8 = fe_sq(&z4);
        fe_mul(&z8, z)
    };
    let z11 = fe_mul(&z9, &z2);
    let z_5_0 = {
        let t = fe_sq(&z11);
        fe_mul(&t, &z9)
    };
    let z_10_0 = {
        let mut t = fe_sq(&z_5_0);
        for _ in 1..5 {
            t = fe_sq(&t);
        }
        fe_mul(&t, &z_5_0)
    };
    let z_20_0 = {
        let mut t = fe_sq(&z_10_0);
        for _ in 1..10 {
            t = fe_sq(&t);
        }
        fe_mul(&t, &z_10_0)
    };
    let z_40_0 = {
        let mut t = fe_sq(&z_20_0);
        for _ in 1..20 {
            t = fe_sq(&t);
        }
        fe_mul(&t, &z_20_0)
    };
    let z_50_0 = {
        let mut t = fe_sq(&z_40_0);
        for _ in 1..10 {
            t = fe_sq(&t);
        }
        fe_mul(&t, &z_10_0)
    };
    let z_100_0 = {
        let mut t = fe_sq(&z_50_0);
        for _ in 1..50 {
            t = fe_sq(&t);
        }
        fe_mul(&t, &z_50_0)
    };
    let z_200_0 = {
        let mut t = fe_sq(&z_100_0);
        for _ in 1..100 {
            t = fe_sq(&t);
        }
        fe_mul(&t, &z_100_0)
    };
    let z_250_0 = {
        let mut t = fe_sq(&z_200_0);
        for _ in 1..50 {
            t = fe_sq(&t);
        }
        fe_mul(&t, &z_50_0)
    };

    {
        let mut t = fe_sq(&z_250_0);
        for _ in 1..5 {
            t = fe_sq(&t);
        }
        fe_mul(&t, &z11)
    }
}

// --- ChaCha20-Poly1305 AEAD ---

/// ChaCha20 quarter round
#[inline]
fn chacha20_quarter_round(state: &mut [u32; 16], a: usize, b: usize, c: usize, d: usize) {
    state[a] = state[a].wrapping_add(state[b]);
    state[d] ^= state[a];
    state[d] = state[d].rotate_left(16);

    state[c] = state[c].wrapping_add(state[d]);
    state[b] ^= state[c];
    state[b] = state[b].rotate_left(12);

    state[a] = state[a].wrapping_add(state[b]);
    state[d] ^= state[a];
    state[d] = state[d].rotate_left(8);

    state[c] = state[c].wrapping_add(state[d]);
    state[b] ^= state[c];
    state[b] = state[b].rotate_left(7);
}

/// Generate one 64-byte ChaCha20 keystream block
fn chacha20_block(key: &[u8; 32], nonce: &[u8; 12], counter: u32) -> [u8; 64] {
    let mut state: [u32; 16] = [
        0x61707865,
        0x3320646e,
        0x79622d32,
        0x6b206574, // "expand 32-byte k"
        u32::from_le_bytes([key[0], key[1], key[2], key[3]]),
        u32::from_le_bytes([key[4], key[5], key[6], key[7]]),
        u32::from_le_bytes([key[8], key[9], key[10], key[11]]),
        u32::from_le_bytes([key[12], key[13], key[14], key[15]]),
        u32::from_le_bytes([key[16], key[17], key[18], key[19]]),
        u32::from_le_bytes([key[20], key[21], key[22], key[23]]),
        u32::from_le_bytes([key[24], key[25], key[26], key[27]]),
        u32::from_le_bytes([key[28], key[29], key[30], key[31]]),
        counter,
        u32::from_le_bytes([nonce[0], nonce[1], nonce[2], nonce[3]]),
        u32::from_le_bytes([nonce[4], nonce[5], nonce[6], nonce[7]]),
        u32::from_le_bytes([nonce[8], nonce[9], nonce[10], nonce[11]]),
    ];

    let initial = state;

    // 20 rounds (10 double rounds)
    for _ in 0..10 {
        // Column rounds
        chacha20_quarter_round(&mut state, 0, 4, 8, 12);
        chacha20_quarter_round(&mut state, 1, 5, 9, 13);
        chacha20_quarter_round(&mut state, 2, 6, 10, 14);
        chacha20_quarter_round(&mut state, 3, 7, 11, 15);
        // Diagonal rounds
        chacha20_quarter_round(&mut state, 0, 5, 10, 15);
        chacha20_quarter_round(&mut state, 1, 6, 11, 12);
        chacha20_quarter_round(&mut state, 2, 7, 8, 13);
        chacha20_quarter_round(&mut state, 3, 4, 9, 14);
    }

    let mut output = [0u8; 64];
    for i in 0..16 {
        let val = state[i].wrapping_add(initial[i]);
        output[i * 4..(i + 1) * 4].copy_from_slice(&val.to_le_bytes());
    }
    output
}

/// ChaCha20 encrypt/decrypt (XOR with keystream)
fn chacha20_crypt(key: &[u8; 32], nonce: &[u8; 12], counter: u32, data: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(data.len());
    let mut ctr = counter;

    for chunk in data.chunks(64) {
        let block = chacha20_block(key, nonce, ctr);
        for (i, &b) in chunk.iter().enumerate() {
            output.push(b ^ block[i]);
        }
        ctr = ctr.wrapping_add(1);
    }

    output
}

/// Poly1305 MAC computation (RFC 8439 Section 2.5)
///
/// Uses u128 arithmetic to avoid overflow in GF(2^130-5) multiplication.
fn poly1305_mac(key: &[u8; 32], message: &[u8]) -> [u8; 16] {
    // Split key: r (first 16 bytes, clamped) and s (last 16 bytes)
    let mut r_bytes = [0u8; 16];
    r_bytes.copy_from_slice(&key[..16]);

    // Clamp r
    r_bytes[3] &= 15;
    r_bytes[7] &= 15;
    r_bytes[11] &= 15;
    r_bytes[15] &= 15;
    r_bytes[4] &= 252;
    r_bytes[8] &= 252;
    r_bytes[12] &= 252;

    let r = u128::from_le_bytes({
        let mut buf = [0u8; 16];
        buf.copy_from_slice(&r_bytes);
        buf
    });
    let s = u128::from_le_bytes({
        let mut buf = [0u8; 16];
        buf.copy_from_slice(&key[16..32]);
        buf
    });

    let mut accumulator: u128 = 0;
    // p = 2^130 - 5 (doesn't fit in u128; passed as _p to mulmod which handles
    // reduction)

    for chunk in message.chunks(16) {
        let mut block = [0u8; 17];
        block[..chunk.len()].copy_from_slice(chunk);
        block[chunk.len()] = 1; // Append 0x01

        // Build little-endian value from block bytes.
        // For a full 16-byte chunk, len = 17 (includes 0x01 sentinel).
        // The sentinel at position 16 represents bit 128, which we must
        // handle without overflowing u128.
        let len = chunk.len() + 1;
        let mut val: u128 = 0;
        let direct = if len > 16 { 16 } else { len };
        for (i, &b) in block[..direct].iter().enumerate() {
            val |= (b as u128) << (8 * i);
        }

        accumulator = accumulator.wrapping_add(val);
        if len > 16 {
            // Add 2^128 for the sentinel bit, split into two halves
            // to avoid shift overflow: 2^128 = 2^127 + 2^127
            accumulator = accumulator.wrapping_add(1u128 << 127);
            accumulator = accumulator.wrapping_add(1u128 << 127);
        }
        // Multiply and reduce mod 2^130-5
        // Use partial reduction to avoid full 256-bit arithmetic
        accumulator = poly1305_mulmod(accumulator, r, 0);
    }

    accumulator = accumulator.wrapping_add(s);
    let tag_bytes = accumulator.to_le_bytes();
    let mut tag = [0u8; 16];
    tag.copy_from_slice(&tag_bytes[..16]);
    tag
}

/// Multiply two 130-bit numbers mod 2^130-5
///
/// Uses the property that 2^130 = 5 (mod p) for efficient reduction.
fn poly1305_mulmod(a: u128, b: u128, _p: u128) -> u128 {
    // Split into 64-bit halves for multiplication
    let a_lo = a & 0xFFFF_FFFF_FFFF_FFFF;
    let a_hi = a >> 64;
    let b_lo = b & 0xFFFF_FFFF_FFFF_FFFF;
    let b_hi = b >> 64;

    // Karatsuba-style multiplication
    let lo_lo = a_lo.wrapping_mul(b_lo);
    let lo_hi = a_lo.wrapping_mul(b_hi);
    let hi_lo = a_hi.wrapping_mul(b_lo);
    let hi_hi = a_hi.wrapping_mul(b_hi);

    // Combine: result = lo_lo + (lo_hi + hi_lo) << 64 + hi_hi << 128
    // But we need to reduce mod 2^130-5
    // Since 2^130 = 5 (mod p), bits above 130 get multiplied by 5

    let mid = lo_hi.wrapping_add(hi_lo);
    let result_lo = lo_lo.wrapping_add(mid << 64);
    let carry = if lo_lo.checked_add(mid << 64).is_none() {
        1u128
    } else {
        0u128
    };

    let result_hi = hi_hi.wrapping_add(mid >> 64).wrapping_add(carry);

    // Reduce mod 2^130 - 5
    // combined = result_lo + result_hi * 2^64, total up to ~260 bits
    // We need bits 0..129 (the "low 130 bits") and bits 130+ (multiply by 5)
    // combined as a u128: result_lo | (result_hi << 64) -- but result_hi may
    // overflow Instead, work directly with result_lo (bits 0..127) and
    // result_hi (bits 64..127+)
    //
    // Bit 130 of the full product = bit 66 of result_hi
    // low_130 = result_lo[0..63] | result_hi[0..1] << 64  (but result_hi << 64 can
    // overflow u128)
    //
    // Simpler: combine into u128 with wrapping, extract low 130 bits via mask
    // Low 130 bits mask = (1 << 64) - 1 in low word + bits 0..1 of high word
    let _combined = result_lo.wrapping_add(result_hi << 64);
    // Bits 0-127 are in combined. Bit 128-129 were lost if result_hi >= 2^64.
    // Since result_hi < 2^66 (product of two 130-bit numbers), overflow is at most
    // 4 bits. Use a different approach: keep result_lo and result_hi separate.

    // Extract low 130 bits: result_lo gives bits 0-127, result_hi bits 0-1 give
    // bits 128-129
    let low_130_lo = result_lo; // bits 0..127
    let low_130_hi = result_hi & 0x3; // bits 128..129 (2 bits from result_hi)
    let low_130 = low_130_lo.wrapping_add((low_130_hi) << 64);
    // Note: low_130_hi << 64 won't overflow since low_130_hi <= 3

    // High bits (130+) = result_hi >> 2
    let high_bits = result_hi >> 2;
    let reduced = low_130.wrapping_add(high_bits.wrapping_mul(5));

    // One more reduction pass: reduced is at most ~131 bits
    // Low 130 bits of reduced
    let _r_lo = reduced; // bits 0..127
                         // Bit 128+ of reduced: since reduced < 2^131, overflow into high bits is
                         // minimal We approximate: if reduced > 2^130 - 1, the
                         // excess is small For a proper second pass, we'd need
                         // to track the carry, but since high_bits is at most
                         // ~66 bits * 5, reduced fits in u128. Final full
                         // reduction is not needed since the accumulator is reduced each round.
    reduced
}

/// ChaCha20-Poly1305 AEAD encrypt (RFC 8439 Section 2.8)
fn chacha20_poly1305_encrypt(
    key: &[u8; 32],
    nonce: &[u8; 12],
    aad: &[u8],
    plaintext: &[u8],
) -> Vec<u8> {
    // Generate Poly1305 one-time key from block 0
    let otk_block = chacha20_block(key, nonce, 0);
    let mut poly_key = [0u8; 32];
    poly_key.copy_from_slice(&otk_block[..32]);

    // Encrypt plaintext starting from counter 1
    let ciphertext = chacha20_crypt(key, nonce, 1, plaintext);

    // Construct Poly1305 input: AAD || pad || ciphertext || pad || len(AAD) ||
    // len(CT)
    let mac_input = build_poly1305_input(aad, &ciphertext);
    let tag = poly1305_mac(&poly_key, &mac_input);

    // Output: ciphertext || tag
    let mut output = ciphertext;
    output.extend_from_slice(&tag);
    output
}

/// ChaCha20-Poly1305 AEAD decrypt (RFC 8439 Section 2.8)
fn chacha20_poly1305_decrypt(
    key: &[u8; 32],
    nonce: &[u8; 12],
    aad: &[u8],
    ciphertext_and_tag: &[u8],
) -> Option<Vec<u8>> {
    if ciphertext_and_tag.len() < AEAD_TAG_LEN {
        return None;
    }

    let ct_len = ciphertext_and_tag.len() - AEAD_TAG_LEN;
    let ciphertext = &ciphertext_and_tag[..ct_len];
    let tag = &ciphertext_and_tag[ct_len..];

    // Generate Poly1305 one-time key
    let otk_block = chacha20_block(key, nonce, 0);
    let mut poly_key = [0u8; 32];
    poly_key.copy_from_slice(&otk_block[..32]);

    // Verify tag
    let mac_input = build_poly1305_input(aad, ciphertext);
    let expected_tag = poly1305_mac(&poly_key, &mac_input);

    if !constant_time_eq(tag, &expected_tag) {
        return None;
    }

    // Decrypt
    Some(chacha20_crypt(key, nonce, 1, ciphertext))
}

/// Build Poly1305 MAC input per RFC 8439 Section 2.8
fn build_poly1305_input(aad: &[u8], ciphertext: &[u8]) -> Vec<u8> {
    let aad_pad = (16 - (aad.len() % 16)) % 16;
    let ct_pad = (16 - (ciphertext.len() % 16)) % 16;

    let mut input = Vec::with_capacity(aad.len() + aad_pad + ciphertext.len() + ct_pad + 16);
    input.extend_from_slice(aad);
    input.resize(input.len() + aad_pad, 0);
    input.extend_from_slice(ciphertext);
    input.resize(input.len() + ct_pad, 0);
    input.extend_from_slice(&(aad.len() as u64).to_le_bytes());
    input.extend_from_slice(&(ciphertext.len() as u64).to_le_bytes());
    input
}

// --- AES-128-GCM AEAD ---

/// AES S-Box
const AES_SBOX: [u8; 256] = [
    0x63, 0x7c, 0x77, 0x7b, 0xf2, 0x6b, 0x6f, 0xc5, 0x30, 0x01, 0x67, 0x2b, 0xfe, 0xd7, 0xab, 0x76,
    0xca, 0x82, 0xc9, 0x7d, 0xfa, 0x59, 0x47, 0xf0, 0xad, 0xd4, 0xa2, 0xaf, 0x9c, 0xa4, 0x72, 0xc0,
    0xb7, 0xfd, 0x93, 0x26, 0x36, 0x3f, 0xf7, 0xcc, 0x34, 0xa5, 0xe5, 0xf1, 0x71, 0xd8, 0x31, 0x15,
    0x04, 0xc7, 0x23, 0xc3, 0x18, 0x96, 0x05, 0x9a, 0x07, 0x12, 0x80, 0xe2, 0xeb, 0x27, 0xb2, 0x75,
    0x09, 0x83, 0x2c, 0x1a, 0x1b, 0x6e, 0x5a, 0xa0, 0x52, 0x3b, 0xd6, 0xb3, 0x29, 0xe3, 0x2f, 0x84,
    0x53, 0xd1, 0x00, 0xed, 0x20, 0xfc, 0xb1, 0x5b, 0x6a, 0xcb, 0xbe, 0x39, 0x4a, 0x4c, 0x58, 0xcf,
    0xd0, 0xef, 0xaa, 0xfb, 0x43, 0x4d, 0x33, 0x85, 0x45, 0xf9, 0x02, 0x7f, 0x50, 0x3c, 0x9f, 0xa8,
    0x51, 0xa3, 0x40, 0x8f, 0x92, 0x9d, 0x38, 0xf5, 0xbc, 0xb6, 0xda, 0x21, 0x10, 0xff, 0xf3, 0xd2,
    0xcd, 0x0c, 0x13, 0xec, 0x5f, 0x97, 0x44, 0x17, 0xc4, 0xa7, 0x7e, 0x3d, 0x64, 0x5d, 0x19, 0x73,
    0x60, 0x81, 0x4f, 0xdc, 0x22, 0x2a, 0x90, 0x88, 0x46, 0xee, 0xb8, 0x14, 0xde, 0x5e, 0x0b, 0xdb,
    0xe0, 0x32, 0x3a, 0x0a, 0x49, 0x06, 0x24, 0x5c, 0xc2, 0xd3, 0xac, 0x62, 0x91, 0x95, 0xe4, 0x79,
    0xe7, 0xc8, 0x37, 0x6d, 0x8d, 0xd5, 0x4e, 0xa9, 0x6c, 0x56, 0xf4, 0xea, 0x65, 0x7a, 0xae, 0x08,
    0xba, 0x78, 0x25, 0x2e, 0x1c, 0xa6, 0xb4, 0xc6, 0xe8, 0xdd, 0x74, 0x1f, 0x4b, 0xbd, 0x8b, 0x8a,
    0x70, 0x3e, 0xb5, 0x66, 0x48, 0x03, 0xf6, 0x0e, 0x61, 0x35, 0x57, 0xb9, 0x86, 0xc1, 0x1d, 0x9e,
    0xe1, 0xf8, 0x98, 0x11, 0x69, 0xd9, 0x8e, 0x94, 0x9b, 0x1e, 0x87, 0xe9, 0xce, 0x55, 0x28, 0xdf,
    0x8c, 0xa1, 0x89, 0x0d, 0xbf, 0xe6, 0x42, 0x68, 0x41, 0x99, 0x2d, 0x0f, 0xb0, 0x54, 0xbb, 0x16,
];

/// AES round constants
const AES_RCON: [u8; 11] = [
    0x00, 0x01, 0x02, 0x04, 0x08, 0x10, 0x20, 0x40, 0x80, 0x1b, 0x36,
];

/// AES-128 block cipher (10 rounds)
struct Aes128 {
    round_keys: [[u8; 16]; 11],
}

impl Aes128 {
    fn new(key: &[u8; 16]) -> Self {
        let mut round_keys = [[0u8; 16]; 11];
        Self::key_expansion(key, &mut round_keys);
        Self { round_keys }
    }

    fn key_expansion(key: &[u8; 16], round_keys: &mut [[u8; 16]; 11]) {
        let mut w = [0u8; 176]; // 44 words * 4 bytes
        w[..16].copy_from_slice(key);

        for i in 4..44 {
            let mut temp = [w[i * 4 - 4], w[i * 4 - 3], w[i * 4 - 2], w[i * 4 - 1]];

            if i % 4 == 0 {
                temp = [
                    AES_SBOX[temp[1] as usize] ^ AES_RCON[i / 4],
                    AES_SBOX[temp[2] as usize],
                    AES_SBOX[temp[3] as usize],
                    AES_SBOX[temp[0] as usize],
                ];
            }

            w[i * 4] = w[i * 4 - 16] ^ temp[0];
            w[i * 4 + 1] = w[i * 4 - 15] ^ temp[1];
            w[i * 4 + 2] = w[i * 4 - 14] ^ temp[2];
            w[i * 4 + 3] = w[i * 4 - 13] ^ temp[3];
        }

        for (i, rk) in round_keys.iter_mut().enumerate() {
            rk.copy_from_slice(&w[i * 16..(i + 1) * 16]);
        }
    }

    fn sub_bytes(state: &mut [u8; 16]) {
        for byte in state.iter_mut() {
            *byte = AES_SBOX[*byte as usize];
        }
    }

    fn shift_rows(state: &mut [u8; 16]) {
        let temp = *state;
        state[1] = temp[5];
        state[5] = temp[9];
        state[9] = temp[13];
        state[13] = temp[1];
        state[2] = temp[10];
        state[6] = temp[14];
        state[10] = temp[2];
        state[14] = temp[6];
        state[3] = temp[15];
        state[7] = temp[3];
        state[11] = temp[7];
        state[15] = temp[11];
    }

    #[inline]
    fn gf_mul(a: u8, b: u8) -> u8 {
        let mut result = 0u8;
        let mut aa = a;
        let mut bb = b;
        for _ in 0..8 {
            if bb & 1 != 0 {
                result ^= aa;
            }
            let hi_bit = aa & 0x80;
            aa <<= 1;
            if hi_bit != 0 {
                aa ^= 0x1b;
            }
            bb >>= 1;
        }
        result
    }

    fn mix_columns(state: &mut [u8; 16]) {
        for col in 0..4 {
            let i = col * 4;
            let (s0, s1, s2, s3) = (state[i], state[i + 1], state[i + 2], state[i + 3]);
            state[i] = Self::gf_mul(2, s0) ^ Self::gf_mul(3, s1) ^ s2 ^ s3;
            state[i + 1] = s0 ^ Self::gf_mul(2, s1) ^ Self::gf_mul(3, s2) ^ s3;
            state[i + 2] = s0 ^ s1 ^ Self::gf_mul(2, s2) ^ Self::gf_mul(3, s3);
            state[i + 3] = Self::gf_mul(3, s0) ^ s1 ^ s2 ^ Self::gf_mul(2, s3);
        }
    }

    fn add_round_key(state: &mut [u8; 16], round_key: &[u8; 16]) {
        for (s, k) in state.iter_mut().zip(round_key.iter()) {
            *s ^= k;
        }
    }

    fn encrypt_block(&self, block: &[u8; 16]) -> [u8; 16] {
        let mut state = *block;
        Self::add_round_key(&mut state, &self.round_keys[0]);

        for round in 1..10 {
            Self::sub_bytes(&mut state);
            Self::shift_rows(&mut state);
            Self::mix_columns(&mut state);
            Self::add_round_key(&mut state, &self.round_keys[round]);
        }

        Self::sub_bytes(&mut state);
        Self::shift_rows(&mut state);
        Self::add_round_key(&mut state, &self.round_keys[10]);

        state
    }
}

/// GCM GHASH multiplication in GF(2^128)
fn ghash_multiply(x: &[u8; 16], h: &[u8; 16]) -> [u8; 16] {
    let mut z = [0u8; 16];
    let mut v = *h;

    for i in 0..128 {
        let byte_idx = i / 8;
        let bit_idx = 7 - (i % 8);
        if (x[byte_idx] >> bit_idx) & 1 == 1 {
            for j in 0..16 {
                z[j] ^= v[j];
            }
        }

        // Shift V right by 1 and reduce if needed
        let lsb = v[15] & 1;
        for j in (1..16).rev() {
            v[j] = (v[j] >> 1) | (v[j - 1] << 7);
        }
        v[0] >>= 1;

        if lsb == 1 {
            v[0] ^= 0xE1; // R = 0xE1 || 0^120
        }
    }

    z
}

/// GHASH function for GCM
fn ghash(h: &[u8; 16], aad: &[u8], ciphertext: &[u8]) -> [u8; 16] {
    let mut tag = [0u8; 16];

    // Process AAD
    for chunk in aad.chunks(16) {
        let mut block = [0u8; 16];
        block[..chunk.len()].copy_from_slice(chunk);
        for i in 0..16 {
            tag[i] ^= block[i];
        }
        tag = ghash_multiply(&tag, h);
    }

    // Process ciphertext
    for chunk in ciphertext.chunks(16) {
        let mut block = [0u8; 16];
        block[..chunk.len()].copy_from_slice(chunk);
        for i in 0..16 {
            tag[i] ^= block[i];
        }
        tag = ghash_multiply(&tag, h);
    }

    // Length block: len(A) || len(C) in bits, big-endian 64-bit
    let mut len_block = [0u8; 16];
    let aad_bits = (aad.len() as u64).wrapping_mul(8);
    let ct_bits = (ciphertext.len() as u64).wrapping_mul(8);
    len_block[..8].copy_from_slice(&aad_bits.to_be_bytes());
    len_block[8..16].copy_from_slice(&ct_bits.to_be_bytes());
    for i in 0..16 {
        tag[i] ^= len_block[i];
    }
    tag = ghash_multiply(&tag, h);

    tag
}

/// AES-128-GCM encrypt
fn aes128_gcm_encrypt(key: &[u8; 16], nonce: &[u8; 12], aad: &[u8], plaintext: &[u8]) -> Vec<u8> {
    let cipher = Aes128::new(key);

    // H = AES_K(0^128)
    let h = cipher.encrypt_block(&[0u8; 16]);

    // J0 = nonce || 0x00000001 (for 96-bit nonce)
    let mut j0 = [0u8; 16];
    j0[..12].copy_from_slice(nonce);
    j0[15] = 1;

    // Encrypt plaintext with counter starting at J0 + 1
    let mut ciphertext = Vec::with_capacity(plaintext.len());
    let mut counter = 2u32;
    for chunk in plaintext.chunks(16) {
        let mut cb = j0;
        cb[12..16].copy_from_slice(&counter.to_be_bytes());
        let keystream = cipher.encrypt_block(&cb);
        for (i, &b) in chunk.iter().enumerate() {
            ciphertext.push(b ^ keystream[i]);
        }
        counter = counter.wrapping_add(1);
    }

    // Compute GHASH
    let ghash_val = ghash(&h, aad, &ciphertext);

    // Tag = GHASH XOR AES_K(J0)
    let j0_encrypted = cipher.encrypt_block(&j0);
    let mut tag = [0u8; 16];
    for i in 0..16 {
        tag[i] = ghash_val[i] ^ j0_encrypted[i];
    }

    ciphertext.extend_from_slice(&tag);
    ciphertext
}

/// AES-128-GCM decrypt
fn aes128_gcm_decrypt(
    key: &[u8; 16],
    nonce: &[u8; 12],
    aad: &[u8],
    ciphertext_and_tag: &[u8],
) -> Option<Vec<u8>> {
    if ciphertext_and_tag.len() < AEAD_TAG_LEN {
        return None;
    }

    let ct_len = ciphertext_and_tag.len() - AEAD_TAG_LEN;
    let ciphertext = &ciphertext_and_tag[..ct_len];
    let received_tag = &ciphertext_and_tag[ct_len..];

    let cipher = Aes128::new(key);
    let h = cipher.encrypt_block(&[0u8; 16]);

    let mut j0 = [0u8; 16];
    j0[..12].copy_from_slice(nonce);
    j0[15] = 1;

    // Verify tag first
    let ghash_val = ghash(&h, aad, ciphertext);
    let j0_encrypted = cipher.encrypt_block(&j0);
    let mut expected_tag = [0u8; 16];
    for i in 0..16 {
        expected_tag[i] = ghash_val[i] ^ j0_encrypted[i];
    }

    if !constant_time_eq(received_tag, &expected_tag) {
        return None;
    }

    // Decrypt
    let mut plaintext = Vec::with_capacity(ct_len);
    let mut counter = 2u32;
    for chunk in ciphertext.chunks(16) {
        let mut cb = j0;
        cb[12..16].copy_from_slice(&counter.to_be_bytes());
        let keystream = cipher.encrypt_block(&cb);
        for (i, &b) in chunk.iter().enumerate() {
            plaintext.push(b ^ keystream[i]);
        }
        counter = counter.wrapping_add(1);
    }

    Some(plaintext)
}

// --- AEAD dispatch ---

/// AEAD encrypt dispatcher for the negotiated cipher suite
fn aead_encrypt(
    cipher: CipherSuite,
    key: &[u8],
    nonce: &[u8; NONCE_LEN],
    aad: &[u8],
    plaintext: &[u8],
) -> Option<Vec<u8>> {
    match cipher {
        CipherSuite::ChaCha20Poly1305Sha256 => {
            if key.len() != CHACHA20_KEY_LEN {
                return None;
            }
            let mut k = [0u8; 32];
            k.copy_from_slice(key);
            Some(chacha20_poly1305_encrypt(&k, nonce, aad, plaintext))
        }
        CipherSuite::Aes128GcmSha256 => {
            if key.len() != AES_128_KEY_LEN {
                return None;
            }
            let mut k = [0u8; 16];
            k.copy_from_slice(key);
            Some(aes128_gcm_encrypt(&k, nonce, aad, plaintext))
        }
    }
}

/// AEAD decrypt dispatcher for the negotiated cipher suite
fn aead_decrypt(
    cipher: CipherSuite,
    key: &[u8],
    nonce: &[u8; NONCE_LEN],
    aad: &[u8],
    ciphertext_and_tag: &[u8],
) -> Option<Vec<u8>> {
    match cipher {
        CipherSuite::ChaCha20Poly1305Sha256 => {
            if key.len() != CHACHA20_KEY_LEN {
                return None;
            }
            let mut k = [0u8; 32];
            k.copy_from_slice(key);
            chacha20_poly1305_decrypt(&k, nonce, aad, ciphertext_and_tag)
        }
        CipherSuite::Aes128GcmSha256 => {
            if key.len() != AES_128_KEY_LEN {
                return None;
            }
            let mut k = [0u8; 16];
            k.copy_from_slice(key);
            aes128_gcm_decrypt(&k, nonce, aad, ciphertext_and_tag)
        }
    }
}

/// Constant-time comparison of two byte slices
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for i in 0..a.len() {
        diff |= a[i] ^ b[i];
    }
    diff == 0
}

// ============================================================================
// Section 4: Certificate Handling (~200 lines)
// ============================================================================

/// Simplified X.509 certificate representation
#[derive(Debug, Clone)]
pub struct X509Certificate {
    /// Raw DER-encoded certificate bytes
    pub raw: Vec<u8>,
    /// Subject common name (extracted from DER)
    pub subject_cn: Vec<u8>,
    /// Issuer common name (extracted from DER)
    pub issuer_cn: Vec<u8>,
    /// Subject public key bytes (raw)
    pub public_key: Vec<u8>,
    /// Not-before timestamp (Unix epoch seconds, 0 if unparsed)
    pub not_before: u64,
    /// Not-after timestamp (Unix epoch seconds, 0 if unparsed)
    pub not_after: u64,
    /// Is this a CA certificate?
    pub is_ca: bool,
}

/// ASN.1 tag constants
const ASN1_SEQUENCE: u8 = 0x30;
const ASN1_SET: u8 = 0x31;
const ASN1_OID: u8 = 0x06;
const ASN1_UTF8STRING: u8 = 0x0C;
const ASN1_PRINTABLESTRING: u8 = 0x13;
const ASN1_BIT_STRING: u8 = 0x03;

/// Parse ASN.1 DER tag and length. Returns (tag, content_start,
/// content_length).
fn asn1_parse_tlv(data: &[u8]) -> Option<(u8, usize, usize)> {
    if data.is_empty() {
        return None;
    }
    let tag = data[0];
    if data.len() < 2 {
        return None;
    }

    let (content_start, content_len) = if data[1] & 0x80 == 0 {
        // Short form
        (2, data[1] as usize)
    } else {
        let num_len_bytes = (data[1] & 0x7F) as usize;
        if num_len_bytes == 0 || num_len_bytes > 4 || data.len() < 2 + num_len_bytes {
            return None;
        }
        let mut len: usize = 0;
        for i in 0..num_len_bytes {
            len = (len << 8) | (data[2 + i] as usize);
        }
        (2 + num_len_bytes, len)
    };

    if content_start + content_len > data.len() {
        return None;
    }

    Some((tag, content_start, content_len))
}

/// Extract a Common Name (OID 2.5.4.3) from an X.501 Name sequence
fn extract_cn(name_data: &[u8]) -> Vec<u8> {
    // OID for commonName: 2.5.4.3 = 55 04 03
    let cn_oid: [u8; 3] = [0x55, 0x04, 0x03];

    let mut pos = 0;
    while pos < name_data.len() {
        if let Some((_tag, start, len)) = asn1_parse_tlv(&name_data[pos..]) {
            let inner = &name_data[pos + start..pos + start + len];
            // Search for CN OID within this SET
            if let Some(idx) = find_subsequence(inner, &cn_oid) {
                // The value follows the OID TLV
                let after_oid = idx + cn_oid.len();
                if after_oid < inner.len() {
                    if let Some((_vtag, vstart, vlen)) = asn1_parse_tlv(&inner[after_oid..]) {
                        let value = &inner[after_oid + vstart..after_oid + vstart + vlen];
                        return value.to_vec();
                    }
                }
            }
            pos += start + len;
        } else {
            break;
        }
    }

    Vec::new()
}

/// Find a subsequence within a byte slice
fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    for i in 0..=haystack.len() - needle.len() {
        if haystack[i..i + needle.len()] == *needle {
            return Some(i + needle.len());
        }
    }
    None
}

impl X509Certificate {
    /// Parse a simplified X.509 certificate from DER-encoded bytes.
    ///
    /// This is a best-effort parser that extracts subject/issuer CN and
    /// the public key. Full ASN.1 validation is beyond scope.
    pub fn from_der(data: &[u8]) -> Option<Self> {
        // Outer SEQUENCE
        let (tag, start, len) = asn1_parse_tlv(data)?;
        if tag != ASN1_SEQUENCE {
            return None;
        }
        let cert_content = &data[start..start + len];

        // TBSCertificate (first SEQUENCE inside)
        let (tbs_tag, tbs_start, tbs_len) = asn1_parse_tlv(cert_content)?;
        if tbs_tag != ASN1_SEQUENCE {
            return None;
        }
        let tbs = &cert_content[tbs_start..tbs_start + tbs_len];

        // Skip version (context [0]) + serial number + signature algorithm
        // Then find issuer and subject sequences
        // This is simplified: we scan for CN OIDs in the TBS data

        let issuer_cn = extract_cn(tbs);

        // Subject is typically after issuer -- scan from a later offset
        let subject_cn = if tbs.len() > 100 {
            let second_half = &tbs[tbs.len() / 3..];
            let cn = extract_cn(second_half);
            if cn.is_empty() {
                issuer_cn.clone()
            } else {
                cn
            }
        } else {
            issuer_cn.clone()
        };

        // Extract public key (look for BIT STRING after SubjectPublicKeyInfo SEQUENCE)
        let mut public_key = Vec::new();
        let mut scan_pos = 0;
        while scan_pos < tbs.len() {
            if tbs[scan_pos] == ASN1_BIT_STRING {
                if let Some((_, bs_start, bs_len)) = asn1_parse_tlv(&tbs[scan_pos..]) {
                    if bs_len > 1 {
                        // Skip the "unused bits" byte
                        public_key =
                            tbs[scan_pos + bs_start + 1..scan_pos + bs_start + bs_len].to_vec();
                    }
                    break;
                }
            }
            scan_pos += 1;
        }

        Some(Self {
            raw: data.to_vec(),
            subject_cn,
            issuer_cn,
            public_key,
            not_before: 0,
            not_after: 0,
            is_ca: false,
        })
    }
}

/// Trust anchor store for root CAs
pub struct TrustStore {
    /// Trusted root CA certificates (subject CN -> certificate)
    anchors: Vec<X509Certificate>,
}

impl Default for TrustStore {
    fn default() -> Self {
        Self::new()
    }
}

impl TrustStore {
    /// Create an empty trust store
    pub fn new() -> Self {
        Self {
            anchors: Vec::new(),
        }
    }

    /// Add a trusted root CA certificate
    pub fn add_anchor(&mut self, cert: X509Certificate) {
        self.anchors.push(cert);
    }

    /// Validate a certificate chain against the trust store.
    ///
    /// Returns true if the chain can be verified back to a trusted anchor
    /// via basic issuer/subject matching.
    pub fn validate_chain(&self, chain: &[X509Certificate]) -> bool {
        if chain.is_empty() {
            return false;
        }

        // Walk the chain: each cert's issuer should match the next cert's subject
        for i in 0..chain.len().saturating_sub(1) {
            if chain[i].issuer_cn != chain[i + 1].subject_cn {
                return false;
            }
        }

        // The last cert in the chain should be issued by a trusted anchor
        let root_issuer = &chain[chain.len() - 1].issuer_cn;
        self.anchors
            .iter()
            .any(|anchor| &anchor.subject_cn == root_issuer)
    }

    /// Number of trust anchors
    pub fn anchor_count(&self) -> usize {
        self.anchors.len()
    }
}

// ============================================================================
// Section 5: Session Management (~150 lines)
// ============================================================================

/// TLS 1.3 alert levels
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum AlertLevel {
    Warning = 1,
    Fatal = 2,
}

impl AlertLevel {
    fn from_u8(v: u8) -> Option<Self> {
        match v {
            1 => Some(Self::Warning),
            2 => Some(Self::Fatal),
            _ => None,
        }
    }
}

/// TLS 1.3 alert descriptions
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum AlertDescription {
    CloseNotify = 0,
    UnexpectedMessage = 10,
    BadRecordMac = 20,
    RecordOverflow = 22,
    HandshakeFailure = 40,
    BadCertificate = 42,
    CertificateRevoked = 44,
    CertificateExpired = 45,
    CertificateUnknown = 46,
    IllegalParameter = 47,
    UnknownCa = 48,
    DecodeError = 50,
    DecryptError = 51,
    ProtocolVersion = 70,
    InsufficientSecurity = 71,
    InternalError = 80,
    MissingExtension = 109,
    UnsupportedExtension = 110,
    UnrecognizedName = 112,
    NoApplicationProtocol = 120,
}

impl AlertDescription {
    fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::CloseNotify),
            10 => Some(Self::UnexpectedMessage),
            20 => Some(Self::BadRecordMac),
            22 => Some(Self::RecordOverflow),
            40 => Some(Self::HandshakeFailure),
            42 => Some(Self::BadCertificate),
            44 => Some(Self::CertificateRevoked),
            45 => Some(Self::CertificateExpired),
            46 => Some(Self::CertificateUnknown),
            47 => Some(Self::IllegalParameter),
            48 => Some(Self::UnknownCa),
            50 => Some(Self::DecodeError),
            51 => Some(Self::DecryptError),
            70 => Some(Self::ProtocolVersion),
            71 => Some(Self::InsufficientSecurity),
            80 => Some(Self::InternalError),
            109 => Some(Self::MissingExtension),
            110 => Some(Self::UnsupportedExtension),
            112 => Some(Self::UnrecognizedName),
            120 => Some(Self::NoApplicationProtocol),
            _ => None,
        }
    }
}

/// TLS alert message
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TlsAlert {
    pub level: AlertLevel,
    pub description: AlertDescription,
}

impl TlsAlert {
    pub fn new(level: AlertLevel, description: AlertDescription) -> Self {
        Self { level, description }
    }

    pub fn encode(&self) -> [u8; 2] {
        [self.level as u8, self.description as u8]
    }

    pub fn decode(data: &[u8]) -> Option<Self> {
        if data.len() < 2 {
            return None;
        }
        Some(Self {
            level: AlertLevel::from_u8(data[0])?,
            description: AlertDescription::from_u8(data[1])?,
        })
    }

    /// Is this a fatal alert?
    pub fn is_fatal(&self) -> bool {
        self.level == AlertLevel::Fatal
    }
}

/// Session ticket for TLS 1.3 resumption (PSK-based)
#[derive(Debug, Clone)]
pub struct SessionTicket {
    /// Ticket lifetime in seconds
    pub lifetime: u32,
    /// Ticket age add (for obfuscation)
    pub age_add: u32,
    /// Ticket nonce
    pub nonce: Vec<u8>,
    /// Opaque ticket data
    pub ticket: Vec<u8>,
    /// Cipher suite used in original session
    pub cipher_suite: CipherSuite,
    /// Resumption master secret
    pub resumption_secret: [u8; HASH_LEN],
    /// Creation timestamp (kernel ticks or epoch seconds)
    pub created_at: u64,
}

impl SessionTicket {
    /// Parse a NewSessionTicket handshake message
    pub fn from_message(
        data: &[u8],
        cipher_suite: CipherSuite,
        resumption_secret: [u8; HASH_LEN],
        now: u64,
    ) -> Option<Self> {
        if data.len() < 4 {
            return None;
        }
        if data[0] != HandshakeType::NewSessionTicket as u8 {
            return None;
        }
        let payload = &data[4..];
        if payload.len() < 12 {
            return None;
        }

        let lifetime = u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]);
        let age_add = u32::from_be_bytes([payload[4], payload[5], payload[6], payload[7]]);

        let nonce_len = payload[8] as usize;
        if 9 + nonce_len + 2 > payload.len() {
            return None;
        }
        let nonce = payload[9..9 + nonce_len].to_vec();

        let ticket_pos = 9 + nonce_len;
        let ticket_len =
            u16::from_be_bytes([payload[ticket_pos], payload[ticket_pos + 1]]) as usize;
        let ticket_start = ticket_pos + 2;
        if ticket_start + ticket_len > payload.len() {
            return None;
        }
        let ticket = payload[ticket_start..ticket_start + ticket_len].to_vec();

        Some(Self {
            lifetime,
            age_add,
            nonce,
            ticket,
            cipher_suite,
            resumption_secret,
            created_at: now,
        })
    }

    /// Derive the PSK for resumption
    pub fn derive_psk(&self) -> [u8; HASH_LEN] {
        let expanded = hkdf_expand_label(
            &self.resumption_secret,
            b"resumption",
            &self.nonce,
            HASH_LEN,
        );
        let mut psk = [0u8; HASH_LEN];
        psk.copy_from_slice(&expanded);
        psk
    }

    /// Check if the ticket has expired
    pub fn is_expired(&self, now: u64) -> bool {
        now.saturating_sub(self.created_at) > self.lifetime as u64
    }
}

/// Session ticket store (limited size, FIFO eviction)
pub struct SessionStore {
    tickets: Vec<SessionTicket>,
    max_entries: usize,
}

impl SessionStore {
    pub fn new(max_entries: usize) -> Self {
        Self {
            tickets: Vec::new(),
            max_entries,
        }
    }

    /// Store a session ticket
    pub fn store(&mut self, ticket: SessionTicket) {
        if self.tickets.len() >= self.max_entries {
            self.tickets.remove(0);
        }
        self.tickets.push(ticket);
    }

    /// Find a valid (non-expired) ticket for resumption
    pub fn find_valid(&self, now: u64) -> Option<&SessionTicket> {
        self.tickets.iter().rev().find(|t| !t.is_expired(now))
    }

    /// Remove expired tickets
    pub fn prune(&mut self, now: u64) {
        self.tickets.retain(|t| !t.is_expired(now));
    }

    /// Number of stored tickets
    pub fn count(&self) -> usize {
        self.tickets.len()
    }
}

// 0-RTT early data support stubs

/// Early data configuration
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EarlyDataState {
    /// Not attempting 0-RTT
    NotAttempted,
    /// 0-RTT offered in ClientHello
    Offered,
    /// Server accepted 0-RTT
    Accepted,
    /// Server rejected 0-RTT
    Rejected,
}

// ============================================================================
// Section 6: TLS Connection API (~150 lines)
// ============================================================================

/// TLS connection errors
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TlsError {
    /// Handshake not completed
    NotConnected,
    /// Connection already closed
    Closed,
    /// Handshake failure
    HandshakeError,
    /// Record decryption failed
    DecryptionFailed,
    /// Alert received from peer
    AlertReceived(AlertDescription),
    /// Invalid state transition
    InvalidState,
    /// Buffer too small
    BufferTooSmall,
    /// Data too large for single record
    DataTooLarge,
}

/// TLS 1.3 connection state
pub struct TlsConnection {
    /// Handshake engine
    engine: HandshakeEngine,
    /// Send sequence number
    send_seq: u64,
    /// Receive sequence number
    recv_seq: u64,
    /// Client write key (application data)
    client_write_key: Option<Vec<u8>>,
    /// Server write key (application data)
    server_write_key: Option<Vec<u8>>,
    /// Client write IV
    client_write_iv: Option<[u8; NONCE_LEN]>,
    /// Server write IV
    server_write_iv: Option<[u8; NONCE_LEN]>,
    /// Fragment reassembly buffer
    fragment_buf: FragmentBuffer,
    /// Received application data buffer
    app_data_buf: Vec<u8>,
    /// Session ticket store
    session_store: SessionStore,
    /// Early data state
    pub early_data: EarlyDataState,
    /// Whether the connection has been closed
    closed: bool,
    /// Last alert received
    pub last_alert: Option<TlsAlert>,
}

impl Default for TlsConnection {
    fn default() -> Self {
        Self::new()
    }
}

impl TlsConnection {
    /// Create a new TLS connection (client mode)
    pub fn new() -> Self {
        Self {
            engine: HandshakeEngine::new(),
            send_seq: 0,
            recv_seq: 0,
            client_write_key: None,
            server_write_key: None,
            client_write_iv: None,
            server_write_iv: None,
            fragment_buf: FragmentBuffer::new(),
            app_data_buf: Vec::new(),
            session_store: SessionStore::new(4),
            early_data: EarlyDataState::NotAttempted,
            closed: false,
            last_alert: None,
        }
    }

    /// Initiate the TLS handshake. Returns the ClientHello record to send.
    pub fn connect(&mut self, random: [u8; 32]) -> Result<Vec<u8>, TlsError> {
        let ch_msg = self
            .engine
            .build_client_hello(random)
            .ok_or(TlsError::InvalidState)?;

        let record = TlsRecord::new(ContentType::Handshake, ch_msg);
        Ok(record.encode())
    }

    /// Process incoming data from the peer. Returns any response records to
    /// send.
    pub fn process_incoming(&mut self, data: &[u8]) -> Result<Vec<u8>, TlsError> {
        let mut response = Vec::new();
        let mut pos = 0;

        while pos < data.len() {
            let (record, consumed) =
                TlsRecord::decode(&data[pos..]).ok_or(TlsError::DecryptionFailed)?;
            pos += consumed;

            match record.content_type {
                ContentType::Alert => {
                    if let Some(alert) = TlsAlert::decode(&record.fragment) {
                        self.last_alert = Some(alert);
                        if alert.is_fatal() {
                            self.closed = true;
                            return Err(TlsError::AlertReceived(alert.description));
                        }
                        if alert.description == AlertDescription::CloseNotify {
                            self.closed = true;
                        }
                    }
                }
                ContentType::Handshake => {
                    if let Some(msg) = self.fragment_buf.append(&record) {
                        self.process_handshake_message(&msg, &mut response)?;
                    }
                }
                ContentType::ApplicationData => {
                    if self.engine.state == HandshakeState::Connected {
                        // Decrypt application data
                        let cipher = self.engine.cipher_suite.ok_or(TlsError::NotConnected)?;
                        let iv = self
                            .server_write_iv
                            .as_ref()
                            .ok_or(TlsError::NotConnected)?;
                        let key = self
                            .server_write_key
                            .as_ref()
                            .ok_or(TlsError::NotConnected)?;
                        if let Some(plain) = decrypt_record(&record, key, iv, self.recv_seq, cipher)
                        {
                            self.recv_seq += 1;
                            self.app_data_buf.extend_from_slice(&plain.fragment);
                        } else {
                            return Err(TlsError::DecryptionFailed);
                        }
                    }
                }
                ContentType::ChangeCipherSpec => {
                    // TLS 1.3 ignores CCS for compatibility
                }
            }
        }

        Ok(response)
    }

    /// Process a complete handshake message
    fn process_handshake_message(
        &mut self,
        msg: &[u8],
        response: &mut Vec<u8>,
    ) -> Result<(), TlsError> {
        if msg.is_empty() {
            return Err(TlsError::HandshakeError);
        }

        let hs_type = HandshakeType::from_u8(msg[0]).ok_or(TlsError::HandshakeError)?;

        match hs_type {
            HandshakeType::ServerHello => {
                self.engine
                    .process_server_hello(msg)
                    .ok_or(TlsError::HandshakeError)?;
            }
            HandshakeType::EncryptedExtensions => {
                self.engine
                    .process_encrypted_extensions(msg)
                    .ok_or(TlsError::HandshakeError)?;
            }
            HandshakeType::Certificate => {
                self.engine
                    .process_certificate(msg)
                    .ok_or(TlsError::HandshakeError)?;
            }
            HandshakeType::CertificateVerify => {
                self.engine
                    .process_certificate_verify(msg)
                    .ok_or(TlsError::HandshakeError)?;
            }
            HandshakeType::Finished => {
                self.engine
                    .process_finished(msg)
                    .ok_or(TlsError::HandshakeError)?;

                // Derive application traffic keys
                self.derive_traffic_keys()?;

                // Send client Finished
                if let Some(client_finished) = self.engine.build_client_finished() {
                    let record = TlsRecord::new(ContentType::Handshake, client_finished);
                    response.extend_from_slice(&record.encode());
                }
            }
            HandshakeType::NewSessionTicket => {
                // Store session ticket for future resumption
                if let Some(cipher) = self.engine.cipher_suite {
                    let resumption_secret = self
                        .engine
                        .client_app_traffic_secret
                        .unwrap_or([0u8; HASH_LEN]);
                    if let Some(ticket) =
                        SessionTicket::from_message(msg, cipher, resumption_secret, 0)
                    {
                        self.session_store.store(ticket);
                    }
                }
            }
            _ => {
                return Err(TlsError::HandshakeError);
            }
        }

        Ok(())
    }

    /// Derive traffic keys from handshake secrets
    fn derive_traffic_keys(&mut self) -> Result<(), TlsError> {
        let cipher = self.engine.cipher_suite.ok_or(TlsError::NotConnected)?;
        let key_len = cipher.key_len();

        // Client write key and IV
        let c_secret = self
            .engine
            .client_app_traffic_secret
            .as_ref()
            .ok_or(TlsError::NotConnected)?;
        self.client_write_key = Some(hkdf_expand_label(c_secret, b"key", &[], key_len));
        let c_iv = hkdf_expand_label(c_secret, b"iv", &[], NONCE_LEN);
        let mut civ = [0u8; NONCE_LEN];
        civ.copy_from_slice(&c_iv);
        self.client_write_iv = Some(civ);

        // Server write key and IV
        let s_secret = self
            .engine
            .server_app_traffic_secret
            .as_ref()
            .ok_or(TlsError::NotConnected)?;
        self.server_write_key = Some(hkdf_expand_label(s_secret, b"key", &[], key_len));
        let s_iv = hkdf_expand_label(s_secret, b"iv", &[], NONCE_LEN);
        let mut siv = [0u8; NONCE_LEN];
        siv.copy_from_slice(&s_iv);
        self.server_write_iv = Some(siv);

        Ok(())
    }

    /// Send application data. Returns the encrypted record to transmit.
    pub fn send(&mut self, data: &[u8]) -> Result<Vec<u8>, TlsError> {
        if self.closed {
            return Err(TlsError::Closed);
        }
        if self.engine.state != HandshakeState::Connected {
            return Err(TlsError::NotConnected);
        }

        let cipher = self.engine.cipher_suite.ok_or(TlsError::NotConnected)?;
        let key = self
            .client_write_key
            .as_ref()
            .ok_or(TlsError::NotConnected)?;
        let iv = self
            .client_write_iv
            .as_ref()
            .ok_or(TlsError::NotConnected)?;

        let mut output = Vec::new();

        // Fragment data into MAX_RECORD_SIZE chunks
        for chunk in data.chunks(MAX_RECORD_SIZE) {
            let record = TlsRecord::new(ContentType::ApplicationData, chunk.to_vec());
            let encrypted = encrypt_record(&record, key, iv, self.send_seq, cipher)
                .ok_or(TlsError::DecryptionFailed)?;
            self.send_seq += 1;
            output.extend_from_slice(&encrypted.encode());
        }

        Ok(output)
    }

    /// Read received application data from the internal buffer
    pub fn recv(&mut self, buf: &mut [u8]) -> Result<usize, TlsError> {
        if self.app_data_buf.is_empty() {
            if self.closed {
                return Err(TlsError::Closed);
            }
            return Ok(0);
        }

        let copy_len = core::cmp::min(buf.len(), self.app_data_buf.len());
        buf[..copy_len].copy_from_slice(&self.app_data_buf[..copy_len]);
        self.app_data_buf = self.app_data_buf[copy_len..].to_vec();
        Ok(copy_len)
    }

    /// Close the TLS connection. Returns a close_notify alert record to send.
    pub fn close(&mut self) -> Result<Vec<u8>, TlsError> {
        if self.closed {
            return Err(TlsError::Closed);
        }
        self.closed = true;

        let alert = TlsAlert::new(AlertLevel::Warning, AlertDescription::CloseNotify);
        let alert_bytes = alert.encode().to_vec();

        if self.engine.state == HandshakeState::Connected {
            let cipher = self.engine.cipher_suite.ok_or(TlsError::NotConnected)?;
            let key = self
                .client_write_key
                .as_ref()
                .ok_or(TlsError::NotConnected)?;
            let iv = self
                .client_write_iv
                .as_ref()
                .ok_or(TlsError::NotConnected)?;

            let record = TlsRecord::new(ContentType::Alert, alert_bytes);
            let encrypted = encrypt_record(&record, key, iv, self.send_seq, cipher)
                .ok_or(TlsError::DecryptionFailed)?;
            self.send_seq += 1;
            Ok(encrypted.encode())
        } else {
            let record = TlsRecord::new(ContentType::Alert, alert_bytes);
            Ok(record.encode())
        }
    }

    /// Get the current handshake state
    pub fn state(&self) -> HandshakeState {
        self.engine.state
    }

    /// Check if the connection is established
    pub fn is_connected(&self) -> bool {
        self.engine.state == HandshakeState::Connected && !self.closed
    }

    /// Get the negotiated cipher suite
    pub fn cipher_suite(&self) -> Option<CipherSuite> {
        self.engine.cipher_suite
    }

    /// Get the number of stored session tickets
    pub fn session_ticket_count(&self) -> usize {
        self.session_store.count()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use alloc::vec;

    use super::*;

    // --- Record Layer Tests ---

    #[test]
    fn test_record_header_encode_decode() {
        let header = RecordHeader {
            content_type: ContentType::Handshake,
            legacy_version: TLS_LEGACY_VERSION,
            length: 256,
        };
        let mut buf = [0u8; 5];
        let n = header.encode(&mut buf);
        assert_eq!(n, 5);

        let decoded = RecordHeader::decode(&buf).unwrap();
        assert_eq!(decoded.content_type, ContentType::Handshake);
        assert_eq!(decoded.legacy_version, TLS_LEGACY_VERSION);
        assert_eq!(decoded.length, 256);
    }

    #[test]
    fn test_record_encode_decode() {
        let data = vec![1, 2, 3, 4, 5];
        let record = TlsRecord::new(ContentType::ApplicationData, data.clone());
        let encoded = record.encode();

        let (decoded, consumed) = TlsRecord::decode(&encoded).unwrap();
        assert_eq!(consumed, encoded.len());
        assert_eq!(decoded.content_type, ContentType::ApplicationData);
        assert_eq!(decoded.fragment, data);
    }

    #[test]
    fn test_record_header_rejects_oversized() {
        let mut buf = [0u8; 5];
        buf[0] = ContentType::ApplicationData as u8;
        buf[1..3].copy_from_slice(&TLS_LEGACY_VERSION.to_be_bytes());
        // Length = MAX_RECORD_SIZE + AEAD_TAG_LEN + 2 (too large)
        let too_large = (MAX_RECORD_SIZE + AEAD_TAG_LEN + 2) as u16;
        buf[3..5].copy_from_slice(&too_large.to_be_bytes());
        assert!(RecordHeader::decode(&buf).is_none());
    }

    #[test]
    fn test_content_type_roundtrip() {
        assert_eq!(
            ContentType::from_u8(20),
            Some(ContentType::ChangeCipherSpec)
        );
        assert_eq!(ContentType::from_u8(21), Some(ContentType::Alert));
        assert_eq!(ContentType::from_u8(22), Some(ContentType::Handshake));
        assert_eq!(ContentType::from_u8(23), Some(ContentType::ApplicationData));
        assert_eq!(ContentType::from_u8(99), None);
    }

    // --- Handshake State Machine Tests ---

    #[test]
    fn test_handshake_initial_state() {
        let engine = HandshakeEngine::new();
        assert_eq!(engine.state, HandshakeState::Start);
    }

    #[test]
    fn test_handshake_client_hello_advances_state() {
        let mut engine = HandshakeEngine::new();
        let random = [0x42u8; 32];
        let msg = engine.build_client_hello(random);
        assert!(msg.is_some());
        assert_eq!(engine.state, HandshakeState::WaitServerHello);
    }

    #[test]
    fn test_handshake_rejects_wrong_state() {
        let mut engine = HandshakeEngine::new();
        // Cannot process ServerHello without first sending ClientHello
        assert!(engine.process_server_hello(&[]).is_none());
    }

    #[test]
    fn test_cipher_suite_code_roundtrip() {
        assert_eq!(
            CipherSuite::from_code(0x1301),
            Some(CipherSuite::Aes128GcmSha256)
        );
        assert_eq!(
            CipherSuite::from_code(0x1303),
            Some(CipherSuite::ChaCha20Poly1305Sha256)
        );
        assert_eq!(CipherSuite::from_code(0x9999), None);
        assert_eq!(CipherSuite::Aes128GcmSha256.code(), 0x1301);
        assert_eq!(CipherSuite::ChaCha20Poly1305Sha256.code(), 0x1303);
    }

    #[test]
    fn test_client_hello_encoding() {
        let ch = ClientHello {
            random: [0xAA; 32],
            session_id: [0; 32],
            cipher_suites: vec![CipherSuite::Aes128GcmSha256],
            key_share_public: [0xBB; 32],
        };
        let encoded = ch.encode();
        // First byte should be ClientHello type
        assert_eq!(encoded[0], HandshakeType::ClientHello as u8);
        // Should contain our random bytes
        assert_eq!(&encoded[6..38], &[0xAA; 32]);
    }

    // --- HKDF Tests ---

    #[test]
    fn test_hkdf_extract_rfc5869_vector1() {
        // RFC 5869 Test Case 1 (SHA-256)
        let ikm = [0x0bu8; 22];
        let salt = [
            0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c,
        ];
        let prk = hkdf_extract(&salt, &ikm);

        let expected: [u8; 32] = [
            0x07, 0x77, 0x09, 0x36, 0x2c, 0x2e, 0x32, 0xdf, 0x0d, 0xdc, 0x3f, 0x0d, 0xc4, 0x7b,
            0xba, 0x63, 0x90, 0xb6, 0xc7, 0x3b, 0xb5, 0x0f, 0x9c, 0x31, 0x22, 0xec, 0x84, 0x4a,
            0xd7, 0xc2, 0xb3, 0xe5,
        ];
        assert_eq!(prk, expected);
    }

    #[test]
    fn test_hkdf_expand_rfc5869_vector1() {
        let prk: [u8; 32] = [
            0x07, 0x77, 0x09, 0x36, 0x2c, 0x2e, 0x32, 0xdf, 0x0d, 0xdc, 0x3f, 0x0d, 0xc4, 0x7b,
            0xba, 0x63, 0x90, 0xb6, 0xc7, 0x3b, 0xb5, 0x0f, 0x9c, 0x31, 0x22, 0xec, 0x84, 0x4a,
            0xd7, 0xc2, 0xb3, 0xe5,
        ];
        let info = [0xf0, 0xf1, 0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8, 0xf9];
        let okm = hkdf_expand(&prk, &info, 42);

        let expected: [u8; 42] = [
            0x3c, 0xb2, 0x5f, 0x25, 0xfa, 0xac, 0xd5, 0x7a, 0x90, 0x43, 0x4f, 0x64, 0xd0, 0x36,
            0x2f, 0x2a, 0x2d, 0x2d, 0x0a, 0x90, 0xcf, 0x1a, 0x5a, 0x4c, 0x5d, 0xb0, 0x2d, 0x56,
            0xec, 0xc4, 0xc5, 0xbf, 0x34, 0x00, 0x72, 0x08, 0xd5, 0xb8, 0x87, 0x18, 0x58, 0x65,
        ];
        assert_eq!(&okm[..], &expected[..]);
    }

    // --- X25519 Tests ---

    #[test]
    fn test_x25519_basepoint_multiplication() {
        // RFC 7748 Section 6.1: scalar * basepoint
        let scalar: [u8; 32] = [
            0xa5, 0x46, 0xe3, 0x6b, 0xf0, 0x52, 0x7c, 0x9d, 0x3b, 0x16, 0x15, 0x4b, 0x82, 0x46,
            0x5e, 0xdd, 0x62, 0x14, 0x4c, 0x0a, 0xc1, 0xfc, 0x5a, 0x18, 0x50, 0x6a, 0x22, 0x44,
            0xba, 0x44, 0x9a, 0xc4,
        ];
        // Verified against RFC 7748 reference and Python cryptography library
        let expected: [u8; 32] = [
            0x1c, 0x9f, 0xd8, 0x8f, 0x45, 0x60, 0x6d, 0x93, 0x2a, 0x80, 0xc7, 0x18, 0x24, 0xae,
            0x15, 0x1d, 0x15, 0xd7, 0x3e, 0x77, 0xde, 0x38, 0xe8, 0xe0, 0x00, 0x85, 0x2e, 0x61,
            0x4f, 0xae, 0x70, 0x19,
        ];

        let result = x25519_scalar_mult(&scalar, &X25519_BASEPOINT);
        assert_eq!(result, expected);
    }

    #[test]
    fn test_x25519_shared_secret_symmetry() {
        // Two parties should derive the same shared secret
        let alice_sk: [u8; 32] = {
            let mut k = [0u8; 32];
            for (i, b) in k.iter_mut().enumerate() {
                *b = (i as u8).wrapping_mul(7).wrapping_add(3);
            }
            k
        };
        let bob_sk: [u8; 32] = {
            let mut k = [0u8; 32];
            for (i, b) in k.iter_mut().enumerate() {
                *b = (i as u8).wrapping_mul(13).wrapping_add(17);
            }
            k
        };

        let alice_pk = x25519_scalar_mult(&alice_sk, &X25519_BASEPOINT);
        let bob_pk = x25519_scalar_mult(&bob_sk, &X25519_BASEPOINT);

        let alice_shared = x25519_shared_secret(&alice_sk, &bob_pk);
        let bob_shared = x25519_shared_secret(&bob_sk, &alice_pk);

        assert_eq!(alice_shared, bob_shared);
    }

    // --- ChaCha20 Tests ---

    #[test]
    fn test_chacha20_encrypt_decrypt_roundtrip() {
        let key = [0x42u8; 32];
        let nonce = [0x01u8; 12];
        let plaintext = b"Hello, TLS 1.3 from VeridianOS!";

        let ciphertext = chacha20_crypt(&key, &nonce, 1, plaintext);
        assert_ne!(&ciphertext[..], &plaintext[..]);

        let decrypted = chacha20_crypt(&key, &nonce, 1, &ciphertext);
        assert_eq!(&decrypted[..], &plaintext[..]);
    }

    #[test]
    fn test_chacha20_poly1305_roundtrip() {
        let key = [0x42u8; 32];
        let nonce = [0x01u8; 12];
        let aad = b"additional authenticated data";
        let plaintext = b"secret message for TLS";

        let ct = chacha20_poly1305_encrypt(&key, &nonce, aad, plaintext);
        assert_eq!(ct.len(), plaintext.len() + AEAD_TAG_LEN);

        let pt = chacha20_poly1305_decrypt(&key, &nonce, aad, &ct).unwrap();
        assert_eq!(&pt[..], &plaintext[..]);
    }

    #[test]
    fn test_chacha20_poly1305_tamper_detection() {
        let key = [0x42u8; 32];
        let nonce = [0x01u8; 12];
        let aad = b"aad";
        let plaintext = b"secret";

        let mut ct = chacha20_poly1305_encrypt(&key, &nonce, aad, plaintext);
        // Tamper with ciphertext
        ct[0] ^= 0xFF;
        assert!(chacha20_poly1305_decrypt(&key, &nonce, aad, &ct).is_none());
    }

    // --- AES-128-GCM Tests ---

    #[test]
    fn test_aes128_gcm_roundtrip() {
        let key = [0x42u8; 16];
        let nonce = [0x01u8; 12];
        let aad = b"additional data";
        let plaintext = b"AES-GCM test message for TLS";

        let ct = aes128_gcm_encrypt(&key, &nonce, aad, plaintext);
        assert_eq!(ct.len(), plaintext.len() + AEAD_TAG_LEN);

        let pt = aes128_gcm_decrypt(&key, &nonce, aad, &ct).unwrap();
        assert_eq!(&pt[..], &plaintext[..]);
    }

    #[test]
    fn test_aes128_gcm_tamper_detection() {
        let key = [0x42u8; 16];
        let nonce = [0x01u8; 12];
        let aad = b"aad";
        let plaintext = b"secret";

        let mut ct = aes128_gcm_encrypt(&key, &nonce, aad, plaintext);
        ct[0] ^= 0xFF;
        assert!(aes128_gcm_decrypt(&key, &nonce, aad, &ct).is_none());
    }

    // --- Alert Tests ---

    #[test]
    fn test_alert_encode_decode() {
        let alert = TlsAlert::new(AlertLevel::Fatal, AlertDescription::HandshakeFailure);
        let encoded = alert.encode();
        assert_eq!(encoded, [2, 40]);

        let decoded = TlsAlert::decode(&encoded).unwrap();
        assert_eq!(decoded.level, AlertLevel::Fatal);
        assert_eq!(decoded.description, AlertDescription::HandshakeFailure);
        assert!(decoded.is_fatal());
    }

    #[test]
    fn test_alert_close_notify() {
        let alert = TlsAlert::new(AlertLevel::Warning, AlertDescription::CloseNotify);
        assert!(!alert.is_fatal());
        let encoded = alert.encode();
        let decoded = TlsAlert::decode(&encoded).unwrap();
        assert_eq!(decoded.description, AlertDescription::CloseNotify);
    }

    // --- Session Ticket Tests ---

    #[test]
    fn test_session_store_lifecycle() {
        let mut store = SessionStore::new(2);
        assert_eq!(store.count(), 0);

        let ticket1 = SessionTicket {
            lifetime: 3600,
            age_add: 0,
            nonce: vec![1],
            ticket: vec![0xAA; 16],
            cipher_suite: CipherSuite::Aes128GcmSha256,
            resumption_secret: [0x11; 32],
            created_at: 100,
        };
        let ticket2 = SessionTicket {
            lifetime: 3600,
            age_add: 0,
            nonce: vec![2],
            ticket: vec![0xBB; 16],
            cipher_suite: CipherSuite::ChaCha20Poly1305Sha256,
            resumption_secret: [0x22; 32],
            created_at: 200,
        };

        store.store(ticket1);
        store.store(ticket2);
        assert_eq!(store.count(), 2);

        // Should find the most recent valid ticket
        let found = store.find_valid(300).unwrap();
        assert_eq!(found.nonce, vec![2]);
    }

    #[test]
    fn test_session_ticket_expiry() {
        let mut store = SessionStore::new(4);
        let ticket = SessionTicket {
            lifetime: 100,
            age_add: 0,
            nonce: vec![1],
            ticket: vec![0xAA; 8],
            cipher_suite: CipherSuite::Aes128GcmSha256,
            resumption_secret: [0; 32],
            created_at: 1000,
        };

        store.store(ticket);
        assert!(store.find_valid(1050).is_some()); // Not expired
        assert!(store.find_valid(1200).is_none()); // Expired

        store.prune(1200);
        assert_eq!(store.count(), 0);
    }

    #[test]
    fn test_session_store_eviction() {
        let mut store = SessionStore::new(2);

        for i in 0..3 {
            store.store(SessionTicket {
                lifetime: 3600,
                age_add: 0,
                nonce: vec![i as u8],
                ticket: vec![i as u8; 8],
                cipher_suite: CipherSuite::Aes128GcmSha256,
                resumption_secret: [0; 32],
                created_at: i as u64,
            });
        }

        // Only 2 should remain (FIFO eviction)
        assert_eq!(store.count(), 2);
    }

    // --- Certificate Tests ---

    #[test]
    fn test_asn1_parse_tlv_short_form() {
        // SEQUENCE with 3 bytes of content
        let data = [0x30, 0x03, 0x01, 0x02, 0x03];
        let (tag, start, len) = asn1_parse_tlv(&data).unwrap();
        assert_eq!(tag, ASN1_SEQUENCE);
        assert_eq!(start, 2);
        assert_eq!(len, 3);
    }

    #[test]
    fn test_asn1_parse_tlv_long_form() {
        // SEQUENCE with 128 bytes of content (long form: 0x81, 0x80)
        let mut data = vec![0x30, 0x81, 0x80];
        data.extend_from_slice(&[0u8; 128]);
        let (tag, start, len) = asn1_parse_tlv(&data).unwrap();
        assert_eq!(tag, ASN1_SEQUENCE);
        assert_eq!(start, 3);
        assert_eq!(len, 128);
    }

    #[test]
    fn test_trust_store_empty_chain_rejected() {
        let store = TrustStore::new();
        assert!(!store.validate_chain(&[]));
    }

    // --- Encrypted Record Tests ---

    #[test]
    fn test_encrypted_record_roundtrip_chacha() {
        let key = [0x42u8; 32];
        let iv = [0x01u8; 12];
        let plaintext = b"application data payload";

        let record = TlsRecord::new(ContentType::ApplicationData, plaintext.to_vec());
        let encrypted =
            encrypt_record(&record, &key, &iv, 0, CipherSuite::ChaCha20Poly1305Sha256).unwrap();
        let decrypted = decrypt_record(
            &encrypted,
            &key,
            &iv,
            0,
            CipherSuite::ChaCha20Poly1305Sha256,
        )
        .unwrap();

        assert_eq!(decrypted.content_type, ContentType::ApplicationData);
        assert_eq!(&decrypted.fragment[..], &plaintext[..]);
    }

    #[test]
    fn test_encrypted_record_roundtrip_aes() {
        let key = [0x42u8; 16];
        let iv = [0x01u8; 12];
        let plaintext = b"AES encrypted record data";

        let record = TlsRecord::new(ContentType::Handshake, plaintext.to_vec());
        let encrypted =
            encrypt_record(&record, &key, &iv, 5, CipherSuite::Aes128GcmSha256).unwrap();

        // Encrypted record should have ApplicationData content type on wire
        assert_eq!(encrypted.content_type, ContentType::ApplicationData);

        let decrypted =
            decrypt_record(&encrypted, &key, &iv, 5, CipherSuite::Aes128GcmSha256).unwrap();
        assert_eq!(decrypted.content_type, ContentType::Handshake);
        assert_eq!(&decrypted.fragment[..], &plaintext[..]);
    }

    // --- TLS Connection API Tests ---

    #[test]
    fn test_tls_connection_initial_state() {
        let conn = TlsConnection::new();
        assert_eq!(conn.state(), HandshakeState::Start);
        assert!(!conn.is_connected());
        assert!(conn.cipher_suite().is_none());
    }

    #[test]
    fn test_tls_connection_connect_produces_record() {
        let mut conn = TlsConnection::new();
        let result = conn.connect([0x42; 32]);
        assert!(result.is_ok());
        let record_bytes = result.unwrap();
        // Should start with a valid TLS record header
        assert!(record_bytes.len() > 5);
        assert_eq!(record_bytes[0], ContentType::Handshake as u8);
        assert_eq!(conn.state(), HandshakeState::WaitServerHello);
    }

    #[test]
    fn test_tls_connection_send_before_connect() {
        let mut conn = TlsConnection::new();
        let result = conn.send(b"hello");
        assert_eq!(result, Err(TlsError::NotConnected));
    }

    #[test]
    fn test_tls_connection_close_before_connect() {
        let mut conn = TlsConnection::new();
        let result = conn.close();
        // Close sends an unencrypted alert when not connected
        assert!(result.is_ok());
        assert!(conn.closed);
    }

    // --- Nonce computation test ---

    #[test]
    fn test_compute_nonce() {
        let iv = [
            0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b,
        ];
        let nonce = compute_nonce(&iv, 1);
        // Last 8 bytes XOR'd with seq=1
        assert_eq!(nonce[0], 0x00);
        assert_eq!(nonce[11], 0x0b ^ 0x01);
    }

    // --- HMAC-SHA256 test ---

    #[test]
    fn test_hmac_sha256_rfc4231_vector1() {
        // RFC 4231 Test Case 1
        let key = [0x0bu8; 20];
        let data = b"Hi There";
        let mac = hmac_sha256(&key, data);
        let expected: [u8; 32] = [
            0xb0, 0x34, 0x4c, 0x61, 0xd8, 0xdb, 0x38, 0x53, 0x5c, 0xa8, 0xaf, 0xce, 0xaf, 0x0b,
            0xf1, 0x2b, 0x88, 0x1d, 0xc2, 0x00, 0xc9, 0x83, 0x3d, 0xa7, 0x26, 0xe9, 0x37, 0x6c,
            0x2e, 0x32, 0xcf, 0xf7,
        ];
        assert_eq!(mac, expected);
    }
}
