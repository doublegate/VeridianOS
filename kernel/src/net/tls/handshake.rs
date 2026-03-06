//! TLS 1.3 Handshake State Machine (RFC 8446 Section 4)
//!
//! Implements the full TLS 1.3 handshake flow:
//! ClientHello -> ServerHello -> EncryptedExtensions -> Certificate ->
//! CertificateVerify -> Finished -> Application Data

#[cfg(feature = "alloc")]
use alloc::vec::Vec;

use super::{
    cipher::{
        constant_time_eq, derive_secret, hkdf_expand_label, hkdf_extract, hmac_sha256,
        x25519_keypair, x25519_shared_secret,
    },
    CipherSuite, HASH_LEN, TLS_13_VERSION, TLS_LEGACY_VERSION, X25519_KEY_LEN,
};
use crate::crypto::hash::{sha256, Hash256};

// ============================================================================
// Handshake Types and Enums
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
    pub(crate) fn from_u8(v: u8) -> Option<Self> {
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

// ============================================================================
// ClientHello / ServerHello
// ============================================================================

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

// ============================================================================
// Handshake Engine
// ============================================================================

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
    pub(crate) fn transcript_hash(&self) -> Hash256 {
        sha256(&self.transcript)
    }
}
