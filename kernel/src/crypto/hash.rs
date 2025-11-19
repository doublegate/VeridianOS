//! Cryptographic Hash Functions
//!
//! Implements SHA-256, SHA-512, and BLAKE3 hash algorithms.

use alloc::vec::Vec;

use super::CryptoResult;

/// Hash algorithm types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HashAlgorithm {
    Sha256,
    Sha512,
    Blake3,
}

/// 256-bit hash output
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hash256(pub [u8; 32]);

/// 512-bit hash output
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hash512(pub [u8; 64]);

impl Hash256 {
    /// Create hash from bytes
    pub fn from_bytes(bytes: &[u8; 32]) -> Self {
        Self(*bytes)
    }

    /// Get hash as bytes
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Convert to hex string
    pub fn to_hex(&self) -> alloc::string::String {
        use alloc::format;
        let mut s = alloc::string::String::with_capacity(64);
        for byte in self.0 {
            s.push_str(&format!("{:02x}", byte));
        }
        s
    }
}

impl Hash512 {
    /// Create hash from bytes
    pub fn from_bytes(bytes: &[u8; 64]) -> Self {
        Self(*bytes)
    }

    /// Get hash as bytes
    pub fn as_bytes(&self) -> &[u8; 64] {
        &self.0
    }

    /// Convert to hex string
    pub fn to_hex(&self) -> alloc::string::String {
        use alloc::format;
        let mut s = alloc::string::String::with_capacity(128);
        for byte in self.0 {
            s.push_str(&format!("{:02x}", byte));
        }
        s
    }
}

/// Hash data with specified algorithm
pub fn hash(algorithm: HashAlgorithm, data: &[u8]) -> CryptoResult<Vec<u8>> {
    match algorithm {
        HashAlgorithm::Sha256 => {
            let hash = sha256(data);
            Ok(hash.0.to_vec())
        }
        HashAlgorithm::Sha512 => {
            let hash = sha512(data);
            Ok(hash.0.to_vec())
        }
        HashAlgorithm::Blake3 => {
            let hash = blake3(data);
            Ok(hash.0.to_vec())
        }
    }
}

/// SHA-256 hash
pub fn sha256(data: &[u8]) -> Hash256 {
    // Simplified SHA-256 implementation (stub)
    // TODO: Implement full SHA-256 algorithm
    let mut hash = [0u8; 32];

    // Simple hash for demo (NOT SECURE - just for testing)
    for (i, &byte) in data.iter().enumerate() {
        hash[i % 32] ^= byte.wrapping_add(i as u8);
    }

    Hash256(hash)
}

/// SHA-512 hash
pub fn sha512(data: &[u8]) -> Hash512 {
    // Simplified SHA-512 implementation (stub)
    // TODO: Implement full SHA-512 algorithm
    let mut hash = [0u8; 64];

    // Simple hash for demo (NOT SECURE - just for testing)
    for (i, &byte) in data.iter().enumerate() {
        hash[i % 64] ^= byte.wrapping_add(i as u8);
    }

    Hash512(hash)
}

/// BLAKE3 hash
pub fn blake3(data: &[u8]) -> Hash256 {
    // Simplified BLAKE3 implementation (stub)
    // TODO: Implement full BLAKE3 algorithm
    let mut hash = [0u8; 32];

    // Simple hash for demo (NOT SECURE - just for testing)
    for (i, &byte) in data.iter().enumerate() {
        hash[i % 32] ^= byte.wrapping_mul(3).wrapping_add(i as u8);
    }

    Hash256(hash)
}

/// Verify hash matches data
pub fn verify_hash(algorithm: HashAlgorithm, data: &[u8], expected: &[u8]) -> CryptoResult<bool> {
    let computed = hash(algorithm, data)?;
    Ok(computed == expected)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test_case]
    fn test_sha256() {
        let data = b"Hello, VeridianOS!";
        let hash = sha256(data);
        assert_eq!(hash.as_bytes().len(), 32);
    }

    #[test_case]
    fn test_hash_hex() {
        let hash = Hash256([0x12, 0x34, 0x56, 0x78] + [0; 28]);
        let hex = hash.to_hex();
        assert!(hex.starts_with("12345678"));
    }
}
