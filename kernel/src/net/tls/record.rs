//! TLS 1.3 Record Layer (RFC 8446 Section 5)
//!
//! Handles record framing, fragment reassembly, and encrypted record
//! wrapping/unwrapping for TLS 1.3.

#[cfg(feature = "alloc")]
use alloc::vec::Vec;

use super::{
    cipher::{aead_decrypt, aead_encrypt},
    CipherSuite, AEAD_TAG_LEN, MAX_RECORD_SIZE, NONCE_LEN, TLS_LEGACY_VERSION,
};

// ============================================================================
// Content Types
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
    pub(crate) fn from_u8(v: u8) -> Option<Self> {
        match v {
            20 => Some(Self::ChangeCipherSpec),
            21 => Some(Self::Alert),
            22 => Some(Self::Handshake),
            23 => Some(Self::ApplicationData),
            _ => None,
        }
    }
}

// ============================================================================
// Record Header
// ============================================================================

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

// ============================================================================
// TLS Record
// ============================================================================

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

// ============================================================================
// Fragment Reassembly
// ============================================================================

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

// ============================================================================
// Encrypted Record Operations
// ============================================================================

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
pub(crate) fn compute_nonce(iv: &[u8; NONCE_LEN], seq_num: u64) -> [u8; NONCE_LEN] {
    let mut nonce = *iv;
    let seq_bytes = seq_num.to_be_bytes();
    // XOR sequence number into the last 8 bytes of the IV
    for i in 0..8 {
        nonce[NONCE_LEN - 8 + i] ^= seq_bytes[i];
    }
    nonce
}
