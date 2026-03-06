//! Unified cipher suite abstraction for protocol crypto
//!
//! Provides `CipherSuite` enum and supporting types (`HmacAlgorithm`,
//! `KdfAlgorithm`) that SSH, QUIC, and WireGuard can use instead of inlining
//! crypto primitives.

#![allow(dead_code)]

use alloc::vec::Vec;

use super::CryptoResult;

// ============================================================================
// AEAD Cipher Suites
// ============================================================================

/// Unified AEAD cipher suite abstraction
///
/// Delegates to the existing `SymmetricCipher` implementations in
/// `crypto::symmetric`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CipherSuite {
    /// ChaCha20-Poly1305 (RFC 8439)
    ChaCha20Poly1305,
    /// AES-256-GCM (NIST SP 800-38D)
    Aes256Gcm,
}

impl CipherSuite {
    /// Encrypt with AEAD: returns ciphertext || tag
    pub fn encrypt_aead(
        &self,
        key: &[u8],
        nonce: &[u8],
        _aad: &[u8],
        plaintext: &[u8],
    ) -> CryptoResult<Vec<u8>> {
        use super::symmetric::SymmetricCipher;
        match self {
            Self::ChaCha20Poly1305 => {
                let cipher = super::symmetric::ChaCha20Poly1305::new(key)?;
                cipher.encrypt(plaintext, nonce)
            }
            Self::Aes256Gcm => {
                let cipher = super::symmetric::Aes256Gcm::new(key)?;
                cipher.encrypt(plaintext, nonce)
            }
        }
    }

    /// Decrypt with AEAD: input is ciphertext || tag, returns plaintext
    pub fn decrypt_aead(
        &self,
        key: &[u8],
        nonce: &[u8],
        _aad: &[u8],
        ciphertext_and_tag: &[u8],
    ) -> CryptoResult<Vec<u8>> {
        use super::symmetric::SymmetricCipher;
        match self {
            Self::ChaCha20Poly1305 => {
                let cipher = super::symmetric::ChaCha20Poly1305::new(key)?;
                cipher.decrypt(ciphertext_and_tag, nonce)
            }
            Self::Aes256Gcm => {
                let cipher = super::symmetric::Aes256Gcm::new(key)?;
                cipher.decrypt(ciphertext_and_tag, nonce)
            }
        }
    }

    /// Key size in bytes
    pub fn key_size(&self) -> usize {
        match self {
            Self::ChaCha20Poly1305 => 32,
            Self::Aes256Gcm => 32,
        }
    }

    /// Nonce size in bytes
    pub fn nonce_size(&self) -> usize {
        match self {
            Self::ChaCha20Poly1305 => 12,
            Self::Aes256Gcm => 12,
        }
    }

    /// Authentication tag size in bytes
    pub fn tag_size(&self) -> usize {
        match self {
            Self::ChaCha20Poly1305 => 16,
            Self::Aes256Gcm => 16,
        }
    }

    /// Algorithm name string
    pub fn algorithm_name(&self) -> &'static str {
        match self {
            Self::ChaCha20Poly1305 => "ChaCha20-Poly1305",
            Self::Aes256Gcm => "AES-256-GCM",
        }
    }
}

// ============================================================================
// HMAC Algorithms
// ============================================================================

/// HMAC algorithm variants
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HmacAlgorithm {
    /// HMAC-SHA-256
    HmacSha256,
    /// HMAC-BLAKE2s (RFC 2104 construction with BLAKE2s)
    HmacBlake2s,
}

impl HmacAlgorithm {
    /// Compute HMAC over data with the given key
    pub fn compute(&self, key: &[u8], data: &[u8]) -> [u8; 32] {
        match self {
            Self::HmacSha256 => hmac_sha256(key, data),
            Self::HmacBlake2s => hmac_blake2s(key, data),
        }
    }

    /// Output size in bytes
    pub fn output_size(&self) -> usize {
        match self {
            Self::HmacSha256 => 32,
            Self::HmacBlake2s => 32,
        }
    }
}

/// HMAC-SHA-256 implementation (RFC 2104)
fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    let block_size = 64;
    let mut padded_key = [0u8; 64];

    if key.len() > block_size {
        let hash = super::hash::sha256(key);
        padded_key[..32].copy_from_slice(hash.as_bytes());
    } else {
        padded_key[..key.len()].copy_from_slice(key);
    }

    // Inner hash: H((key XOR ipad) || data)
    let mut ipad = [0x36u8; 64];
    for i in 0..64 {
        ipad[i] ^= padded_key[i];
    }
    let mut inner_data = Vec::with_capacity(64 + data.len());
    inner_data.extend_from_slice(&ipad);
    inner_data.extend_from_slice(data);
    let inner_hash = super::hash::sha256(&inner_data);

    // Outer hash: H((key XOR opad) || inner_hash)
    let mut opad = [0x5cu8; 64];
    for i in 0..64 {
        opad[i] ^= padded_key[i];
    }
    let mut outer_data = Vec::with_capacity(64 + 32);
    outer_data.extend_from_slice(&opad);
    outer_data.extend_from_slice(inner_hash.as_bytes());
    let outer_hash = super::hash::sha256(&outer_data);

    *outer_hash.as_bytes()
}

/// HMAC-BLAKE2s implementation (RFC 2104 construction)
fn hmac_blake2s(key: &[u8], data: &[u8]) -> [u8; 32] {
    use super::hash::Blake2s;

    let block_size = 64;
    let mut padded_key = [0u8; 64];

    if key.len() > block_size {
        let hash = super::hash::blake2s_hash(key, 32);
        padded_key[..32].copy_from_slice(&hash);
    } else {
        padded_key[..key.len()].copy_from_slice(key);
    }

    // Inner hash: H((key XOR ipad) || data)
    let mut ipad = [0x36u8; 64];
    for i in 0..64 {
        ipad[i] ^= padded_key[i];
    }
    let mut inner = Blake2s::new(32);
    inner.update(&ipad);
    inner.update(data);
    let inner_hash = inner.finalize();

    // Outer hash: H((key XOR opad) || inner_hash)
    let mut opad = [0x5cu8; 64];
    for i in 0..64 {
        opad[i] ^= padded_key[i];
    }
    let mut outer = Blake2s::new(32);
    outer.update(&opad);
    outer.update(&inner_hash);
    outer.finalize()
}

// ============================================================================
// Key Derivation Functions
// ============================================================================

/// KDF algorithm variants
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KdfAlgorithm {
    /// HKDF using BLAKE2s as the underlying HMAC
    HkdfBlake2s,
}

impl KdfAlgorithm {
    /// HKDF-Extract: derive a PRK from salt and input keying material
    pub fn extract(&self, salt: &[u8], ikm: &[u8]) -> [u8; 32] {
        match self {
            Self::HkdfBlake2s => HmacAlgorithm::HmacBlake2s.compute(salt, ikm),
        }
    }

    /// HKDF-Expand: derive output keying material from PRK and info
    ///
    /// Returns up to 32 bytes of output keying material.
    pub fn expand(&self, prk: &[u8], info: &[u8]) -> [u8; 32] {
        match self {
            Self::HkdfBlake2s => HmacAlgorithm::HmacBlake2s.compute(prk, info),
        }
    }

    /// Combined extract-and-expand producing two 32-byte outputs
    ///
    /// This is the pattern used by WireGuard's Noise handshake:
    /// PRK = HMAC(chaining_key, input), T1 = HMAC(PRK, 0x01),
    /// T2 = HMAC(PRK, T1 || 0x02)
    pub fn extract_expand2(&self, chaining_key: &[u8; 32], input: &[u8]) -> ([u8; 32], [u8; 32]) {
        match self {
            Self::HkdfBlake2s => {
                let prk = HmacAlgorithm::HmacBlake2s.compute(chaining_key, input);
                let t1 = HmacAlgorithm::HmacBlake2s.compute(&prk, &[0x01]);
                let mut t1_input = [0u8; 33];
                t1_input[..32].copy_from_slice(&t1);
                t1_input[32] = 0x02;
                let t2 = HmacAlgorithm::HmacBlake2s.compute(&prk, &t1_input);
                (t1, t2)
            }
        }
    }

    /// Combined extract-and-expand producing three 32-byte outputs
    ///
    /// Extended pattern for Noise handshake PSK mixing:
    /// PRK = HMAC(chaining_key, input), T1 = HMAC(PRK, 0x01),
    /// T2 = HMAC(PRK, T1 || 0x02), T3 = HMAC(PRK, T2 || 0x03)
    pub fn extract_expand3(
        &self,
        chaining_key: &[u8; 32],
        input: &[u8],
    ) -> ([u8; 32], [u8; 32], [u8; 32]) {
        match self {
            Self::HkdfBlake2s => {
                let prk = HmacAlgorithm::HmacBlake2s.compute(chaining_key, input);
                let t1 = HmacAlgorithm::HmacBlake2s.compute(&prk, &[0x01]);
                let mut t1_input = [0u8; 33];
                t1_input[..32].copy_from_slice(&t1);
                t1_input[32] = 0x02;
                let t2 = HmacAlgorithm::HmacBlake2s.compute(&prk, &t1_input);
                let mut t2_input = [0u8; 33];
                t2_input[..32].copy_from_slice(&t2);
                t2_input[32] = 0x03;
                let t3 = HmacAlgorithm::HmacBlake2s.compute(&prk, &t2_input);
                (t1, t2, t3)
            }
        }
    }
}
