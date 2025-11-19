//! Cryptographic Infrastructure
//!
//! Provides cryptographic primitives and services for secure operations.

pub mod hash;
pub mod symmetric;
pub mod asymmetric;
pub mod random;
pub mod keystore;
pub mod post_quantum;
pub mod constant_time;
pub mod pq_params;

pub use hash::{HashAlgorithm, Hash256, Hash512};
pub use symmetric::{SymmetricCipher, Aes256Gcm, ChaCha20Poly1305};
pub use asymmetric::{SigningKey, VerifyingKey, KeyPair};
pub use random::{SecureRandom, get_random};
pub use keystore::{KeyStore, KeyId, Key};
pub use post_quantum::{DilithiumSigningKey, KyberSecretKey, HybridKeyExchange};

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
