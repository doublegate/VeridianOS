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

pub mod certificate;
pub mod cipher;
pub mod handshake;
pub mod record;

// Re-export public API
pub use certificate::{TrustStore, X509Certificate};
pub use cipher::{
    hkdf_expand, hkdf_expand_label, hkdf_extract, hmac_sha256, x25519_keypair, x25519_shared_secret,
};
pub use handshake::{
    ClientHello, HandshakeEngine, HandshakeState, HandshakeType, NamedGroup, ServerHello,
    SignatureScheme,
};
pub use record::{
    decrypt_record, encrypt_record, ContentType, FragmentBuffer, RecordHeader, TlsRecord,
};

// ============================================================================
// Constants
// ============================================================================

/// Maximum TLS record payload size (2^14 = 16384 bytes)
pub(crate) const MAX_RECORD_SIZE: usize = 16384;

/// TLS 1.3 protocol version
pub(crate) const TLS_13_VERSION: u16 = 0x0304;

/// Legacy TLS 1.2 version used in record headers
pub(crate) const TLS_LEGACY_VERSION: u16 = 0x0303;

/// AEAD tag length for both AES-128-GCM and ChaCha20-Poly1305
pub(crate) const AEAD_TAG_LEN: usize = 16;

/// HKDF-SHA256 hash output length
pub(crate) const HASH_LEN: usize = 32;

/// X25519 key size (scalar and point)
pub(crate) const X25519_KEY_LEN: usize = 32;

/// IV/nonce length for TLS 1.3 AEAD ciphers
pub(crate) const NONCE_LEN: usize = 12;

/// AES-128 key length
pub(crate) const AES_128_KEY_LEN: usize = 16;

/// ChaCha20 key length
pub(crate) const CHACHA20_KEY_LEN: usize = 32;

// ============================================================================
// Cipher Suite
// ============================================================================

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

// ============================================================================
// Alert Protocol
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

// ============================================================================
// Session Ticket Resumption
// ============================================================================

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
// TLS Connection Errors
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

// ============================================================================
// TLS Connection API
// ============================================================================

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

    use super::{
        certificate::asn1_parse_tlv,
        cipher::{
            aes128_gcm_decrypt, aes128_gcm_encrypt, chacha20_crypt, chacha20_poly1305_decrypt,
            chacha20_poly1305_encrypt, x25519_scalar_mult,
        },
        record::compute_nonce,
        *,
    };

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

        let x25519_basepoint: [u8; 32] = {
            let mut b = [0u8; 32];
            b[0] = 9;
            b
        };
        let result = x25519_scalar_mult(&scalar, &x25519_basepoint);
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

        let x25519_basepoint: [u8; 32] = {
            let mut b = [0u8; 32];
            b[0] = 9;
            b
        };
        let alice_pk = x25519_scalar_mult(&alice_sk, &x25519_basepoint);
        let bob_pk = x25519_scalar_mult(&bob_sk, &x25519_basepoint);

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
        assert_eq!(tag, 0x30);
        assert_eq!(start, 2);
        assert_eq!(len, 3);
    }

    #[test]
    fn test_asn1_parse_tlv_long_form() {
        // SEQUENCE with 128 bytes of content (long form: 0x81, 0x80)
        let mut data = vec![0x30, 0x81, 0x80];
        data.extend_from_slice(&[0u8; 128]);
        let (tag, start, len) = asn1_parse_tlv(&data).unwrap();
        assert_eq!(tag, 0x30);
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
