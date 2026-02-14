//! Package signature structures
//!
//! Handles serialization and deserialization of cryptographic signatures
//! used for package verification. Supports dual-signature scheme with
//! Ed25519 (classical) and Dilithium/ML-DSA (post-quantum) signatures.

use alloc::vec::Vec;

/// Package signature section
#[derive(Debug)]
pub struct PackageSignatures {
    /// Ed25519 signature (64 bytes)
    pub ed25519_sig: [u8; 64],
    /// Dilithium signature (variable size)
    pub dilithium_sig: Vec<u8>,
}

impl PackageSignatures {
    /// Serialize signatures to bytes
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();

        // Ed25519 signature
        bytes.extend_from_slice(&self.ed25519_sig);

        // Dilithium signature length (u32)
        let dil_len = self.dilithium_sig.len() as u32;
        bytes.extend_from_slice(&dil_len.to_le_bytes());

        // Dilithium signature
        bytes.extend_from_slice(&self.dilithium_sig);

        bytes
    }

    /// Deserialize signatures from bytes
    pub fn from_bytes(data: &[u8]) -> Result<Self, crate::error::KernelError> {
        if data.len() < 68 {
            // 64 (Ed25519) + 4 (length)
            return Err(crate::error::KernelError::InvalidArgument {
                name: "signature_data",
                value: "data too short (need >= 68 bytes)",
            });
        }

        // Extract Ed25519 signature
        let mut ed25519_sig = [0u8; 64];
        ed25519_sig.copy_from_slice(&data[0..64]);

        // Extract Dilithium signature length
        let dil_len = u32::from_le_bytes([data[64], data[65], data[66], data[67]]) as usize;

        if data.len() < 68 + dil_len {
            return Err(crate::error::KernelError::InvalidArgument {
                name: "signature_data",
                value: "invalid Dilithium signature length",
            });
        }

        // Extract Dilithium signature
        let dilithium_sig = data[68..68 + dil_len].to_vec();

        Ok(Self {
            ed25519_sig,
            dilithium_sig,
        })
    }
}
