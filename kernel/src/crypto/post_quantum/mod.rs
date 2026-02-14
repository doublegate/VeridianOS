//! Post-Quantum Cryptography
//!
//! Implements ML-DSA (Dilithium) signatures and ML-KEM (Kyber) key
//! encapsulation.
//!
//! ## NIST Standards Compliance
//!
//! This module implements algorithms selected by NIST for post-quantum
//! cryptography:
//! - **ML-DSA (FIPS 204)**: Module-Lattice-Based Digital Signature Algorithm

// Phase 3 (security hardening) -- cryptographic structures are defined but
// not yet exercised by higher-level callers.
#![allow(dead_code)]
//!   - Replaces Dilithium after standardization
//!   - Provides quantum-resistant digital signatures
//!   - Security levels: 2, 3, 5 (128, 192, 256-bit equivalents)
//! - **ML-KEM (FIPS 203)**: Module-Lattice-Based Key Encapsulation Mechanism
//!   - Replaces Kyber after standardization
//!   - Provides quantum-resistant key exchange
//!   - Security levels: 512, 768, 1024 (128, 192, 256-bit equivalents)
//!
//! ## Implementation Status
//!
//! **Current**: Lattice-based implementations with NTT polynomial arithmetic
//! **Production Requirements**:
//! - Full NIST-compliant algorithm implementations
//! - Constant-time operations to prevent timing attacks
//! - Proper random number generation from hardware
//! - Side-channel attack mitigations
//! - FIPS 140-3 validation for cryptographic modules
//!
//! ## Integration with Classical Cryptography
//!
//! Hybrid key exchange combines classical (X25519) and post-quantum (Kyber) to
//! provide:
//! - Security against both classical and quantum attacks
//! - Backward compatibility during transition period
//! - Meet-in-the-middle security guarantees

mod dilithium;
mod hybrid;
pub mod kyber;

use alloc::vec::Vec;

pub use dilithium::{DilithiumSignature, DilithiumSigningKey, DilithiumVerifyingKey};
pub use hybrid::HybridKeyExchange;
pub use kyber::{KyberCiphertext, KyberPublicKey, KyberSecretKey, KyberSharedSecret};

// ============================================================================
// Shared Types
// ============================================================================

/// Dilithium security levels
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DilithiumLevel {
    Level2, // ~128 bits security
    Level3, // ~192 bits security
    Level5, // ~256 bits security
}

/// Kyber security levels
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KyberLevel {
    Kyber512,  // ~128 bits security
    Kyber768,  // ~192 bits security
    Kyber1024, // ~256 bits security
}

// ============================================================================
// Shared Constants and Utility Functions
// ============================================================================

/// Kyber modulus
const KYBER_Q: u32 = 3329;
/// Polynomial degree
const KYBER_N: usize = 256;

/// Primitive 256th root of unity mod q=3329
/// zeta = 17 (since 17^128 = -1 mod 3329)
const KYBER_ZETA: u32 = 17;

/// Bit-reverse a 7-bit number
fn bit_reverse_7(x: u8) -> u8 {
    let mut r = 0u8;
    let mut v = x;
    let mut i = 0;
    while i < 7 {
        r = (r << 1) | (v & 1);
        v >>= 1;
        i += 1;
    }
    r
}

/// Modular exponentiation: base^exp mod modulus
fn mod_pow(base: u32, exp: u32, modulus: u32) -> u32 {
    if modulus == 1 {
        return 0;
    }
    let mut result: u64 = 1;
    let mut base = (base % modulus) as u64;
    let modulus = modulus as u64;
    let mut exp = exp;
    while exp > 0 {
        if exp & 1 == 1 {
            result = (result * base) % modulus;
        }
        exp >>= 1;
        base = (base * base) % modulus;
    }
    result as u32
}

/// Barrett reduction for Kyber: reduce a to [0, q)
fn kyber_barrett_reduce(a: i32) -> i32 {
    /// Barrett reduction constant for Kyber
    const KYBER_BARRETT_V: u32 = 20159; // round(2^26 / q)

    let q = KYBER_Q as i32;
    // t = (a * v) >> 26 where v = round(2^26 / q)
    let t = ((a as i64 * KYBER_BARRETT_V as i64) >> 26) as i32;
    let r = a - t * q;
    // Result may still be >= q, one conditional subtraction
    if r >= q {
        r - q
    } else if r < 0 {
        r + q
    } else {
        r
    }
}

/// Expand a seed into pseudorandom bytes using SHA-256 in counter mode
fn expand_seed(seed: &[u8], len: usize) -> Vec<u8> {
    use super::hash::sha256;
    let mut result = Vec::with_capacity(len);
    let mut counter: u32 = 0;

    while result.len() < len {
        let mut input = Vec::with_capacity(seed.len() + 4);
        input.extend_from_slice(seed);
        input.extend_from_slice(&counter.to_le_bytes());
        let hash = sha256(&input);
        let remaining = len - result.len();
        let take = core::cmp::min(remaining, 32);
        result.extend_from_slice(&hash.as_bytes()[..take]);
        counter += 1;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test_case]
    fn test_dilithium_signing() {
        let signing_key = DilithiumSigningKey::generate(DilithiumLevel::Level2).unwrap();
        let verifying_key = signing_key.verifying_key();

        let message = b"Hello, Post-Quantum World!";
        let signature = signing_key.sign(message).unwrap();

        assert!(verifying_key.verify(message, &signature).unwrap());
    }

    #[test_case]
    fn test_kyber_kem() {
        let secret_key = KyberSecretKey::generate(KyberLevel::Kyber768).unwrap();
        let public_key = secret_key.public_key();

        let (ciphertext, shared_secret1) = public_key.encapsulate().unwrap();
        let shared_secret2 = secret_key.decapsulate(&ciphertext).unwrap();

        // Secrets should match
        assert_eq!(shared_secret1.as_bytes(), shared_secret2.as_bytes());
    }

    #[test_case]
    fn test_hybrid_exchange() {
        let alice = HybridKeyExchange::generate(KyberLevel::Kyber768).unwrap();
        let bob = HybridKeyExchange::generate(KyberLevel::Kyber768).unwrap();

        let (alice_classical_pub, alice_pq_pub) = alice.public_keys();
        let (bob_classical_pub, bob_pq_pub) = bob.public_keys();

        let (bob_pq_ct, _) = alice_pq_pub.encapsulate().unwrap();

        let alice_shared = alice.exchange(&bob_classical_pub, &bob_pq_ct).unwrap();

        // Just verify it completes without error
        assert_eq!(alice_shared.len(), 32);
    }
}
