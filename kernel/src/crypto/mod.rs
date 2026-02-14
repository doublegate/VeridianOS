//! Cryptographic Infrastructure
//!
//! Provides cryptographic primitives and services for secure operations.

pub mod asymmetric;
pub mod constant_time;
pub mod hash;
pub mod keystore;
pub mod post_quantum;
pub mod pq_params;
pub mod random;
pub mod symmetric;

pub use asymmetric::{KeyPair, SigningKey, VerifyingKey};
pub use hash::{Hash256, Hash512, HashAlgorithm};
pub use keystore::{Key, KeyId, KeyStore};
pub use post_quantum::{DilithiumSigningKey, HybridKeyExchange, KyberSecretKey};
pub use random::{get_random, SecureRandom};
pub use symmetric::{Aes256Gcm, ChaCha20Poly1305, SymmetricCipher};

use crate::error::KernelError;

/// Initialize cryptographic subsystem
pub fn init() -> Result<(), KernelError> {
    crate::println!("[CRYPTO] Initializing cryptographic subsystem...");

    // Initialize secure random number generator
    random::init().map_err(|_| KernelError::InvalidState {
        expected: "initialized",
        actual: "failed_to_init_random",
    })?;

    // Initialize key store
    keystore::init().map_err(|_| KernelError::InvalidState {
        expected: "initialized",
        actual: "failed_to_init_keystore",
    })?;

    crate::println!("[CRYPTO] Cryptographic subsystem initialized");
    Ok(())
}

/// Crypto operation result
pub type CryptoResult<T> = Result<T, CryptoError>;

/// Cryptographic errors
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CryptoError {
    InvalidKeySize,
    InvalidNonceSize,
    InvalidTagSize,
    EncryptionFailed,
    DecryptionFailed,
    SignatureFailed,
    VerificationFailed,
    KeyGenerationFailed,
    InvalidKey,
    InsufficientEntropy,
}

/// Validate crypto primitives against known test vectors (NIST FIPS 180-4).
///
/// Returns true if all test vectors pass, false otherwise.
pub fn validate() -> bool {
    // NIST FIPS 180-4 SHA-256 test vector: SHA-256("abc")
    // Expected: ba7816bf 8f01cfea 414140de 5dae2223 b00361a3 96177a9c b410ff61
    // f20015ad
    let expected: [u8; 32] = [
        0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea, 0x41, 0x41, 0x40, 0xde, 0x5d, 0xae, 0x22,
        0x23, 0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c, 0xb4, 0x10, 0xff, 0x61, 0xf2, 0x00,
        0x15, 0xad,
    ];

    let result = hash::sha256(b"abc");
    result.as_bytes() == &expected
}

impl core::fmt::Display for CryptoError {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        match self {
            CryptoError::InvalidKeySize => write!(f, "Invalid key size"),
            CryptoError::InvalidNonceSize => write!(f, "Invalid nonce size"),
            CryptoError::InvalidTagSize => write!(f, "Invalid authentication tag size"),
            CryptoError::EncryptionFailed => write!(f, "Encryption failed"),
            CryptoError::DecryptionFailed => write!(f, "Decryption failed"),
            CryptoError::SignatureFailed => write!(f, "Signature generation failed"),
            CryptoError::VerificationFailed => write!(f, "Signature verification failed"),
            CryptoError::KeyGenerationFailed => write!(f, "Key generation failed"),
            CryptoError::InvalidKey => write!(f, "Invalid cryptographic key"),
            CryptoError::InsufficientEntropy => write!(f, "Insufficient entropy"),
        }
    }
}
