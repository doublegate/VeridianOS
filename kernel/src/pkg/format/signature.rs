//! Package signature structures
//!
//! Handles serialization and deserialization of cryptographic signatures
//! used for package verification. Supports dual-signature scheme with
//! Ed25519 (classical) and Dilithium/ML-DSA (post-quantum) signatures.
//!
//! Also defines the signature policy, trust levels, and trusted key ring
//! used by the package manager during installation to decide whether a
//! package's cryptographic signatures are acceptable.

use alloc::vec::Vec;

// ============================================================================
// Trust Levels
// ============================================================================

/// Trust level assigned to a signing key.
///
/// Levels are ordered: `Untrusted < Community < Developer < Core`.
/// The package manager's [`SignaturePolicy`] specifies a minimum trust level
/// that the signing key must meet for installation to proceed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum TrustLevel {
    /// Key is not trusted (default for unknown keys)
    #[default]
    Untrusted = 0,
    /// Community-contributed key (third-party packages)
    Community = 1,
    /// Verified developer key
    Developer = 2,
    /// Core OS key (kernel, base system)
    Core = 3,
}

// ============================================================================
// Signature Policy
// ============================================================================

/// Policy controlling how the package manager enforces signature verification.
///
/// # Examples
///
/// ```ignore
/// // Production: require both Ed25519 and Dilithium, minimum Developer trust
/// let policy = SignaturePolicy::production();
///
/// // Development: skip all signature checks
/// let policy = SignaturePolicy::development();
/// ```
#[derive(Debug, Clone)]
pub struct SignaturePolicy {
    /// When `true`, packages must have a valid Ed25519 signature from a
    /// trusted key. When `false`, signature verification is skipped entirely.
    pub require_signatures: bool,
    /// Minimum trust level the signing key must have.
    pub minimum_trust_level: TrustLevel,
    /// When `true`, require a valid post-quantum (Dilithium/ML-DSA) signature
    /// in addition to the Ed25519 signature.
    pub require_post_quantum: bool,
}

impl SignaturePolicy {
    /// Production policy: require Ed25519 signatures at Developer trust level.
    /// Post-quantum (Dilithium) is optional until ecosystem matures.
    pub fn production() -> Self {
        Self {
            require_signatures: true,
            minimum_trust_level: TrustLevel::Developer,
            require_post_quantum: false,
        }
    }

    /// Development policy: no signature requirements.
    pub fn development() -> Self {
        Self {
            require_signatures: false,
            minimum_trust_level: TrustLevel::Untrusted,
            require_post_quantum: false,
        }
    }
}

impl Default for SignaturePolicy {
    fn default() -> Self {
        Self::production()
    }
}

// ============================================================================
// Trusted Key Ring
// ============================================================================

/// A trusted Ed25519 public key with its SHA-256 fingerprint and trust level.
#[derive(Debug, Clone)]
pub struct TrustedKey {
    /// Ed25519 public key (32 bytes)
    pub public_key: [u8; 32],
    /// SHA-256 fingerprint of `public_key` for human-readable identification
    pub fingerprint: [u8; 32],
    /// Trust level assigned to this key
    pub trust_level: TrustLevel,
}

/// Collection of trusted Ed25519 signing keys.
///
/// The package manager iterates over these keys when verifying a package
/// signature, accepting the first key whose Ed25519 verification succeeds.
#[derive(Debug, Clone)]
pub struct TrustedKeyRing {
    keys: Vec<TrustedKey>,
}

impl TrustedKeyRing {
    /// Create an empty key ring.
    pub fn new() -> Self {
        Self { keys: Vec::new() }
    }

    /// Create a key ring pre-populated with the built-in test key.
    ///
    /// The built-in key is a deterministic test key for development.
    /// Production builds MUST replace this with keys from a real key ceremony.
    pub fn with_builtin_keys() -> Self {
        let mut ring = Self::new();

        // Deterministic test key derived from "VeridianOS-PKG-KEY"
        let mut pk = [0u8; 32];
        let seed: &[u8; 18] = b"VeridianOS-PKG-KEY";
        for (i, byte) in pk.iter_mut().enumerate() {
            *byte = seed[i % seed.len()].wrapping_add(i as u8);
        }

        let fp = crate::crypto::hash::sha256(&pk);
        ring.add_key(TrustedKey {
            public_key: pk,
            fingerprint: *fp.as_bytes(),
            trust_level: TrustLevel::Core,
        });

        ring
    }

    /// Add a trusted key to the ring.
    pub fn add_key(&mut self, key: TrustedKey) {
        self.keys.push(key);
    }

    /// Find the first key whose public key bytes match.
    pub fn find_key(&self, public_key: &[u8; 32]) -> Option<&TrustedKey> {
        self.keys.iter().find(|k| &k.public_key == public_key)
    }

    /// Find a key by its SHA-256 fingerprint.
    pub fn find_by_fingerprint(&self, fingerprint: &[u8; 32]) -> Option<&TrustedKey> {
        self.keys.iter().find(|k| &k.fingerprint == fingerprint)
    }

    /// Remove keys matching the given fingerprint.
    pub fn remove_key(&mut self, fingerprint: &[u8; 32]) {
        self.keys.retain(|k| &k.fingerprint != fingerprint);
    }

    /// Iterate over all trusted keys.
    pub fn keys(&self) -> &[TrustedKey] {
        &self.keys
    }
}

impl Default for TrustedKeyRing {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Package Signatures (serialization)
// ============================================================================

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
