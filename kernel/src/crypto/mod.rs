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
