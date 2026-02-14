//! Hybrid Key Exchange
//!
//! Combines classical (X25519) and post-quantum (Kyber) key exchange to
//! provide security against both classical and quantum attacks during
//! the transition period.

use alloc::vec::Vec;

use super::{
    kyber::{KyberCiphertext, KyberPublicKey, KyberSecretKey},
    KyberLevel,
};
use crate::crypto::CryptoResult;

/// Hybrid key exchange (combining classical and post-quantum)
pub struct HybridKeyExchange {
    /// X25519 classical key
    classical_secret: crate::crypto::asymmetric::key_exchange::SecretKey,
    /// Kyber post-quantum key
    pq_secret: KyberSecretKey,
}

impl HybridKeyExchange {
    /// Generate new hybrid key pair
    pub fn generate(kyber_level: KyberLevel) -> CryptoResult<Self> {
        let classical_secret = crate::crypto::asymmetric::key_exchange::SecretKey::generate()?;
        let pq_secret = KyberSecretKey::generate(kyber_level)?;

        Ok(Self {
            classical_secret,
            pq_secret,
        })
    }

    /// Get public keys
    pub fn public_keys(
        &self,
    ) -> (
        crate::crypto::asymmetric::key_exchange::PublicKey,
        KyberPublicKey,
    ) {
        (
            self.classical_secret.public_key(),
            self.pq_secret.public_key(),
        )
    }

    /// Perform hybrid key exchange
    pub fn exchange(
        &self,
        their_classical: &crate::crypto::asymmetric::key_exchange::PublicKey,
        their_pq_ct: &KyberCiphertext,
    ) -> CryptoResult<[u8; 32]> {
        // Classical key exchange
        let classical_shared = self.classical_secret.exchange(their_classical)?;

        // Post-quantum key decapsulation
        let pq_shared = self.pq_secret.decapsulate(their_pq_ct)?;

        // Combine both secrets (XOR for simplicity, KDF in production)
        use crate::crypto::hash::sha256;

        let mut combined = Vec::new();
        combined.extend_from_slice(classical_shared.as_bytes());
        combined.extend_from_slice(pq_shared.as_bytes());

        let final_secret = sha256(&combined);

        Ok(*final_secret.as_bytes())
    }
}
