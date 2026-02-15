//! Package File Format
//!
//! Binary format specification for VeridianOS packages (.vpkg files).
//!
//! This module is organized into submodules:
//! - [`compression`]: LZ4, Zstandard, and Brotli compression implementations
//! - [`signature`]: Cryptographic signature structures for package verification
//!
//! ## Package Structure
//!
//! ```text
//! +------------------+
//! | Header (64 bytes)|
//! +------------------+
//! | Metadata (JSON)  |
//! +------------------+
//! | Content (files)  |
//! +------------------+
//! | Signatures       |
//! | - Ed25519        |
//! | - Dilithium      |
//! +------------------+
//! ```

#![allow(clippy::no_effect, clippy::identity_op)]

mod compression;
mod signature;

// Re-export public types
pub use compression::{compress, decompress};
pub use signature::{PackageSignatures, SignaturePolicy, TrustLevel, TrustedKey, TrustedKeyRing};

/// Package file magic number
pub const VPKG_MAGIC: [u8; 4] = *b"VPKG";

/// Package format version
pub const VPKG_VERSION: u32 = 1;

/// Package types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PackageType {
    /// Binary executable package
    Binary = 0,
    /// Library package
    Library = 1,
    /// Kernel module/driver
    KernelModule = 2,
    /// Data-only package
    Data = 3,
    /// Meta-package (dependencies only)
    Meta = 4,
}

/// Compression algorithms
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Compression {
    /// No compression
    None = 0,
    /// Zstandard compression (recommended)
    Zstd = 1,
    /// LZ4 compression (fast)
    Lz4 = 2,
    /// Brotli compression (high ratio)
    Brotli = 3,
}

impl Compression {
    /// Create from byte value
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Compression::None),
            1 => Some(Compression::Zstd),
            2 => Some(Compression::Lz4),
            3 => Some(Compression::Brotli),
            _ => None,
        }
    }

    /// Get compression ratio estimate
    pub fn ratio_estimate(&self) -> f32 {
        match self {
            Compression::None => 1.0,
            Compression::Zstd => 0.3,    // ~70% reduction
            Compression::Lz4 => 0.5,     // ~50% reduction
            Compression::Brotli => 0.25, // ~75% reduction
        }
    }
}

/// Package file header (64 bytes)
#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct PackageHeader {
    /// Magic number "VPKG"
    pub magic: [u8; 4],
    /// Format version
    pub version: u32,
    /// Package type
    pub pkg_type: u8,
    /// Compression algorithm
    pub compression: u8,
    /// Reserved bytes
    pub _reserved: [u8; 6],
    /// Offset to metadata section
    pub metadata_offset: u64,
    /// Size of metadata section
    pub metadata_size: u64,
    /// Offset to content section
    pub content_offset: u64,
    /// Size of content section
    pub content_size: u64,
    /// Offset to signature section
    pub signature_offset: u64,
    /// Size of signature section
    pub signature_size: u64,
}

impl PackageHeader {
    /// Create new package header
    pub fn new(
        pkg_type: PackageType,
        compression: Compression,
        metadata_size: u64,
        content_size: u64,
        signature_size: u64,
    ) -> Self {
        let mut offset = 64u64; // Start after header

        let metadata_offset = offset;
        offset += metadata_size;

        let content_offset = offset;
        offset += content_size;

        let signature_offset = offset;

        Self {
            magic: VPKG_MAGIC,
            version: VPKG_VERSION,
            pkg_type: pkg_type as u8,
            compression: compression as u8,
            _reserved: [0; 6],
            metadata_offset,
            metadata_size,
            content_offset,
            content_size,
            signature_offset,
            signature_size,
        }
    }

    /// Validate package header
    pub fn validate(&self) -> bool {
        self.magic == VPKG_MAGIC && self.version == VPKG_VERSION
    }

    /// Get package type
    pub fn get_type(&self) -> Option<PackageType> {
        match self.pkg_type {
            0 => Some(PackageType::Binary),
            1 => Some(PackageType::Library),
            2 => Some(PackageType::KernelModule),
            3 => Some(PackageType::Data),
            4 => Some(PackageType::Meta),
            _ => None,
        }
    }

    /// Get compression algorithm
    pub fn get_compression(&self) -> Option<Compression> {
        Compression::from_u8(self.compression)
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use super::*;

    #[test_case]
    fn test_header_creation() {
        let header = PackageHeader::new(PackageType::Binary, Compression::Zstd, 1024, 4096, 128);

        assert!(header.validate());
        assert_eq!(header.get_type(), Some(PackageType::Binary));
        assert_eq!(header.get_compression(), Some(Compression::Zstd));
    }

    #[test_case]
    fn test_compression_enum() {
        assert_eq!(Compression::from_u8(0), Some(Compression::None));
        assert_eq!(Compression::from_u8(1), Some(Compression::Zstd));
        assert_eq!(Compression::from_u8(99), None);
    }

    #[test_case]
    fn test_signature_serialization() {
        let sigs = PackageSignatures {
            ed25519_sig: [0x42; 64],
            dilithium_sig: vec![0xAA; 100],
        };

        let bytes = sigs.to_bytes();
        let restored = PackageSignatures::from_bytes(&bytes).unwrap();

        assert_eq!(restored.ed25519_sig, sigs.ed25519_sig);
        assert_eq!(restored.dilithium_sig, sigs.dilithium_sig);
    }

    #[test_case]
    fn test_lz4_compression_roundtrip() {
        let original = b"Hello, World! This is a test of LZ4 compression. \
                        It should compress and decompress correctly.";

        let compressed = compress(original, Compression::Lz4).unwrap();
        let decompressed = decompress(&compressed, Compression::Lz4).unwrap();

        assert_eq!(decompressed, original);
    }

    #[test_case]
    fn test_zstd_compression_roundtrip() {
        let original = b"Test data for Zstd compression. \
                        AAAAAAAAAA BBBBBBBBBB CCCCCCCCCC repetitive data.";

        let compressed = compress(original, Compression::Zstd).unwrap();
        let decompressed = decompress(&compressed, Compression::Zstd).unwrap();

        assert_eq!(decompressed, original);
    }

    #[test_case]
    fn test_brotli_compression_roundtrip() {
        let original = b"Brotli compression test with some repeated patterns \
                        and varied content for better compression testing.";

        let compressed = compress(original, Compression::Brotli).unwrap();
        let decompressed = decompress(&compressed, Compression::Brotli).unwrap();

        assert_eq!(decompressed, original);
    }

    #[test_case]
    fn test_no_compression() {
        let original = b"Uncompressed data";

        let compressed = compress(original, Compression::None).unwrap();
        let decompressed = decompress(&compressed, Compression::None).unwrap();

        assert_eq!(compressed, original);
        assert_eq!(decompressed, original);
    }

    #[test_case]
    fn test_empty_input() {
        let empty: &[u8] = &[];

        // LZ4 empty
        let lz4_result = compress(empty, Compression::Lz4).unwrap();
        assert!(lz4_result.is_empty());

        // Brotli empty
        let brotli_result = compress(empty, Compression::Brotli).unwrap();
        assert!(!brotli_result.is_empty()); // Has empty stream marker

        // None empty
        let none_result = compress(empty, Compression::None).unwrap();
        assert!(none_result.is_empty());
    }

    #[test_case]
    fn test_highly_compressible_data() {
        // Create highly repetitive data
        let mut original = Vec::with_capacity(10000);
        for _ in 0..1000 {
            original.extend_from_slice(b"AAAAAAAAAA");
        }

        // LZ4 should compress this well
        let compressed = compress(&original, Compression::Lz4).unwrap();
        assert!(compressed.len() < original.len() / 2);

        let decompressed = decompress(&compressed, Compression::Lz4).unwrap();
        assert_eq!(decompressed, original);
    }
}
