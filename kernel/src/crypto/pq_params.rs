//! NIST Post-Quantum Cryptography Parameter Sets
//!
//! Official parameter sets from NIST FIPS 203 (ML-KEM) and FIPS 204 (ML-DSA).
//!
//! ## References
//!
//! - FIPS 203: Module-Lattice-Based Key-Encapsulation Mechanism Standard
//! - FIPS 204: Module-Lattice-Based Digital Signature Standard
//! - NIST PQC Standardization Process: <https://csrc.nist.gov/projects/post-quantum-cryptography>

/// ML-DSA (Dilithium) Parameter Sets
pub mod dilithium {
    /// ML-DSA-44 (Security Level 2 - equivalent to AES-128)
    pub mod level2 {
        pub const PUBLIC_KEY_SIZE: usize = 1312;
        pub const SECRET_KEY_SIZE: usize = 2528;
        pub const SIGNATURE_SIZE: usize = 2420;

        // Lattice parameters
        pub const Q: u32 = 8380417; // Prime modulus
        pub const D: u32 = 13; // Dropped bits from t
        pub const TAU: usize = 39; // Number of ±1's in c
        pub const CHALLENGE_ENTROPY: usize = 192; // Bits of entropy in c
        pub const GAMMA1: u32 = 1 << 17; // y coefficient range
        pub const GAMMA2: u32 = (Q - 1) / 88; // Low-order rounding range
        pub const K: usize = 4; // Rows in A
        pub const L: usize = 4; // Columns in A
        pub const ETA: u32 = 2; // Secret key range
        pub const BETA: u32 = TAU as u32 * ETA; // Max coefficient of ct0
        pub const OMEGA: usize = 80; // Max number of 1s in hint
    }

    /// ML-DSA-65 (Security Level 3 - equivalent to AES-192)
    pub mod level3 {
        pub const PUBLIC_KEY_SIZE: usize = 1952;
        pub const SECRET_KEY_SIZE: usize = 4000;
        pub const SIGNATURE_SIZE: usize = 3293;

        pub const Q: u32 = 8380417;
        pub const D: u32 = 13;
        pub const TAU: usize = 49;
        pub const CHALLENGE_ENTROPY: usize = 225;
        pub const GAMMA1: u32 = 1 << 19;
        pub const GAMMA2: u32 = (Q - 1) / 32;
        pub const K: usize = 6;
        pub const L: usize = 5;
        pub const ETA: u32 = 4;
        pub const BETA: u32 = TAU as u32 * ETA;
        pub const OMEGA: usize = 55;
    }

    /// ML-DSA-87 (Security Level 5 - equivalent to AES-256)
    pub mod level5 {
        pub const PUBLIC_KEY_SIZE: usize = 2592;
        pub const SECRET_KEY_SIZE: usize = 4864;
        pub const SIGNATURE_SIZE: usize = 4595;

        pub const Q: u32 = 8380417;
        pub const D: u32 = 13;
        pub const TAU: usize = 60;
        pub const CHALLENGE_ENTROPY: usize = 257;
        pub const GAMMA1: u32 = 1 << 19;
        pub const GAMMA2: u32 = (Q - 1) / 32;
        pub const K: usize = 8;
        pub const L: usize = 7;
        pub const ETA: u32 = 2;
        pub const BETA: u32 = TAU as u32 * ETA;
        pub const OMEGA: usize = 75;
    }
}

/// ML-KEM (Kyber) Parameter Sets
pub mod kyber {
    /// ML-KEM-512 (Security Level 1 - equivalent to AES-128)
    pub mod kyber512 {
        pub const PUBLIC_KEY_SIZE: usize = 800;
        pub const SECRET_KEY_SIZE: usize = 1632;
        pub const CIPHERTEXT_SIZE: usize = 768;
        pub const SHARED_SECRET_SIZE: usize = 32;

        // Lattice parameters
        pub const Q: u32 = 3329; // Prime modulus
        pub const N: usize = 256; // Polynomial degree
        pub const K: usize = 2; // Module rank
        pub const ETA1: u32 = 3; // Secret key noise
        pub const ETA2: u32 = 2; // Encryption noise
        pub const DU: usize = 10; // Ciphertext compression
        pub const DV: usize = 4; // Ciphertext compression
    }

    /// ML-KEM-768 (Security Level 3 - equivalent to AES-192)
    pub mod kyber768 {
        pub const PUBLIC_KEY_SIZE: usize = 1184;
        pub const SECRET_KEY_SIZE: usize = 2400;
        pub const CIPHERTEXT_SIZE: usize = 1088;
        pub const SHARED_SECRET_SIZE: usize = 32;

        pub const Q: u32 = 3329;
        pub const N: usize = 256;
        pub const K: usize = 3;
        pub const ETA1: u32 = 2;
        pub const ETA2: u32 = 2;
        pub const DU: usize = 10;
        pub const DV: usize = 4;
    }

    /// ML-KEM-1024 (Security Level 5 - equivalent to AES-256)
    pub mod kyber1024 {
        pub const PUBLIC_KEY_SIZE: usize = 1568;
        pub const SECRET_KEY_SIZE: usize = 3168;
        pub const CIPHERTEXT_SIZE: usize = 1568;
        pub const SHARED_SECRET_SIZE: usize = 32;

        pub const Q: u32 = 3329;
        pub const N: usize = 256;
        pub const K: usize = 4;
        pub const ETA1: u32 = 2;
        pub const ETA2: u32 = 2;
        pub const DU: usize = 11;
        pub const DV: usize = 5;
    }
}

/// Security level mapping
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecurityLevel {
    /// NIST Level 1 (equivalent to AES-128, ~128 bits quantum security)
    Level1,
    /// NIST Level 2 (equivalent to SHA-256 collision resistance, ~128 bits)
    Level2,
    /// NIST Level 3 (equivalent to AES-192, ~192 bits quantum security)
    Level3,
    /// NIST Level 4 (equivalent to SHA-384 collision resistance, ~192 bits)
    Level4,
    /// NIST Level 5 (equivalent to AES-256, ~256 bits quantum security)
    Level5,
}

impl SecurityLevel {
    /// Get classical security bits
    pub fn classical_bits(&self) -> u32 {
        match self {
            SecurityLevel::Level1 | SecurityLevel::Level2 => 128,
            SecurityLevel::Level3 | SecurityLevel::Level4 => 192,
            SecurityLevel::Level5 => 256,
        }
    }

    /// Get quantum security bits
    pub fn quantum_bits(&self) -> u32 {
        match self {
            SecurityLevel::Level1 | SecurityLevel::Level2 => 128,
            SecurityLevel::Level3 | SecurityLevel::Level4 => 192,
            SecurityLevel::Level5 => 256,
        }
    }

    /// Get recommended use cases
    pub fn use_cases(&self) -> &'static str {
        match self {
            SecurityLevel::Level1 | SecurityLevel::Level2 => {
                "Standard security applications, IoT devices, embedded systems"
            }
            SecurityLevel::Level3 | SecurityLevel::Level4 => {
                "High-value data, long-term confidentiality, government applications"
            }
            SecurityLevel::Level5 => {
                "Top Secret information, critical infrastructure, military use"
            }
        }
    }
}

/// Recommended parameter selections
pub mod recommendations {
    use super::SecurityLevel;

    /// Get recommended Dilithium level for security requirement
    pub fn dilithium_level(required_security: SecurityLevel) -> &'static str {
        match required_security {
            SecurityLevel::Level1 | SecurityLevel::Level2 => "ML-DSA-44 (Level 2)",
            SecurityLevel::Level3 | SecurityLevel::Level4 => "ML-DSA-65 (Level 3)",
            SecurityLevel::Level5 => "ML-DSA-87 (Level 5)",
        }
    }

    /// Get recommended Kyber level for security requirement
    pub fn kyber_level(required_security: SecurityLevel) -> &'static str {
        match required_security {
            SecurityLevel::Level1 | SecurityLevel::Level2 => "ML-KEM-512",
            SecurityLevel::Level3 | SecurityLevel::Level4 => "ML-KEM-768",
            SecurityLevel::Level5 => "ML-KEM-1024",
        }
    }

    /// Default security level for general use
    pub const DEFAULT_SECURITY: SecurityLevel = SecurityLevel::Level3;

    /// Performance vs Security trade-offs
    pub const PERFORMANCE_NOTES: &str = "
    Level 2 (ML-DSA-44, ML-KEM-512):
      - Fastest performance
      - Smallest key/signature sizes
      - Suitable for most applications
      - ~2-3x faster than Level 3

    Level 3 (ML-DSA-65, ML-KEM-768): [RECOMMENDED]
      - Balanced performance/security
      - Recommended default
      - Long-term security margin
      - Moderate size increase

    Level 5 (ML-DSA-87, ML-KEM-1024):
      - Maximum security
      - Largest keys/signatures
      - ~2x slower than Level 3
      - Use only when required
    ";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parameter_consistency() {
        // Verify Dilithium parameters match NIST spec
        assert_eq!(dilithium::level2::PUBLIC_KEY_SIZE, 1312);
        assert_eq!(dilithium::level3::PUBLIC_KEY_SIZE, 1952);
        assert_eq!(dilithium::level5::PUBLIC_KEY_SIZE, 2592);

        // Verify Kyber parameters match NIST spec
        assert_eq!(kyber::kyber512::PUBLIC_KEY_SIZE, 800);
        assert_eq!(kyber::kyber768::PUBLIC_KEY_SIZE, 1184);
        assert_eq!(kyber::kyber1024::PUBLIC_KEY_SIZE, 1568);
    }

    #[test]
    fn test_security_levels() {
        assert_eq!(SecurityLevel::Level2.classical_bits(), 128);
        assert_eq!(SecurityLevel::Level3.classical_bits(), 192);
        assert_eq!(SecurityLevel::Level5.classical_bits(), 256);
    }
}
