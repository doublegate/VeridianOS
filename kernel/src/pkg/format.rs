//! Package File Format
//!
//! Binary format specification for VeridianOS packages (.vpkg files).
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

use alloc::vec::Vec;
use alloc::string::String;

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
            Compression::Zstd => 0.3,   // ~70% reduction
            Compression::Lz4 => 0.5,    // ~50% reduction
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
    pub fn from_bytes(data: &[u8]) -> Result<Self, &'static str> {
        if data.len() < 68 {
            // 64 (Ed25519) + 4 (length)
            return Err("Invalid signature data");
        }

        // Extract Ed25519 signature
        let mut ed25519_sig = [0u8; 64];
        ed25519_sig.copy_from_slice(&data[0..64]);

        // Extract Dilithium signature length
        let dil_len = u32::from_le_bytes([data[64], data[65], data[66], data[67]]) as usize;

        if data.len() < 68 + dil_len {
            return Err("Invalid Dilithium signature length");
        }

        // Extract Dilithium signature
        let dilithium_sig = data[68..68 + dil_len].to_vec();

        Ok(Self {
            ed25519_sig,
            dilithium_sig,
        })
    }
}

/// Decompress data based on compression algorithm
pub fn decompress(data: &[u8], compression: Compression) -> Result<Vec<u8>, String> {
    match compression {
        Compression::None => {
            // No decompression needed
            Ok(data.to_vec())
        }
        Compression::Zstd => {
            // TODO: Implement Zstd decompression
            // For now, just return error
            Err(String::from("Zstd decompression not yet implemented"))
        }
        Compression::Lz4 => {
            // TODO: Implement LZ4 decompression
            Err(String::from("LZ4 decompression not yet implemented"))
        }
        Compression::Brotli => {
            // TODO: Implement Brotli decompression
            Err(String::from("Brotli decompression not yet implemented"))
        }
    }
}

/// Compress data using specified algorithm
pub fn compress(data: &[u8], compression: Compression) -> Result<Vec<u8>, String> {
    match compression {
        Compression::None => {
            // No compression needed
            Ok(data.to_vec())
        }
        Compression::Zstd => {
            // TODO: Implement Zstd compression
            Err(String::from("Zstd compression not yet implemented"))
        }
        Compression::Lz4 => {
            // TODO: Implement LZ4 compression
            Err(String::from("LZ4 compression not yet implemented"))
        }
        Compression::Brotli => {
            // TODO: Implement Brotli compression
            Err(String::from("Brotli compression not yet implemented"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test_case]
    fn test_header_creation() {
        let header = PackageHeader::new(
            PackageType::Binary,
            Compression::Zstd,
            1024,
            4096,
            128,
        );

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
}
