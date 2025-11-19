//! Package File Format
//!
//! Binary format specification for VeridianOS packages (.vpkg files).

use alloc::vec::Vec;

/// Package file magic number
pub const VPKG_MAGIC: [u8; 4] = *b"VPKG";

/// Package format version
pub const VPKG_VERSION: u32 = 1;

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
    /// Validate package header
    pub fn validate(&self) -> bool {
        self.magic == VPKG_MAGIC && self.version == VPKG_VERSION
    }
}
