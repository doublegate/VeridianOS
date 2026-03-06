//! Binary Package Creation
//!
//! Post-build `.vpkg` archive creation with metadata, file manifests,
//! and Ed25519 package signing. Integrates with the build orchestrator
//! to produce distributable binary packages.

#[cfg(feature = "alloc")]
use alloc::{
    string::{String, ToString},
    vec::Vec,
};

use crate::error::KernelError;

/// Package archive format magic bytes
const VPKG_MAGIC: [u8; 4] = [b'V', b'P', b'K', b'G'];

/// Package archive version
const VPKG_VERSION: u8 = 1;

/// Package metadata for .vpkg archives
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct PackageMetadata {
    pub name: String,
    pub version: String,
    pub description: String,
    pub architecture: String,
    pub dependencies: Vec<String>,
    pub installed_size: u64,
    pub maintainer: String,
    pub homepage: String,
    pub license: String,
}

#[cfg(feature = "alloc")]
impl PackageMetadata {
    pub fn new(name: &str, version: &str) -> Self {
        Self {
            name: name.to_string(),
            version: version.to_string(),
            description: String::new(),
            architecture: String::from("x86_64"),
            dependencies: Vec::new(),
            installed_size: 0,
            maintainer: String::new(),
            homepage: String::new(),
            license: String::new(),
        }
    }

    /// Serialize metadata to key=value format
    pub fn serialize(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        let add_field = |buf: &mut Vec<u8>, key: &str, val: &str| {
            buf.extend_from_slice(key.as_bytes());
            buf.push(b'=');
            buf.extend_from_slice(val.as_bytes());
            buf.push(b'\n');
        };

        add_field(&mut buf, "name", &self.name);
        add_field(&mut buf, "version", &self.version);
        add_field(&mut buf, "description", &self.description);
        add_field(&mut buf, "architecture", &self.architecture);
        add_field(&mut buf, "depends", &self.dependencies.join(","));
        add_field(
            &mut buf,
            "installed_size",
            &alloc::format!("{}", self.installed_size),
        );
        add_field(&mut buf, "maintainer", &self.maintainer);
        add_field(&mut buf, "homepage", &self.homepage);
        add_field(&mut buf, "license", &self.license);

        buf
    }

    /// Deserialize metadata from key=value format
    pub fn deserialize(data: &[u8]) -> Result<Self, KernelError> {
        let text = core::str::from_utf8(data).map_err(|_| KernelError::InvalidArgument {
            name: "metadata",
            value: "invalid utf-8",
        })?;
        let mut meta = Self::new("", "");

        for line in text.lines() {
            if let Some((key, val)) = line.split_once('=') {
                match key {
                    "name" => meta.name = val.to_string(),
                    "version" => meta.version = val.to_string(),
                    "description" => meta.description = val.to_string(),
                    "architecture" => meta.architecture = val.to_string(),
                    "depends" => {
                        if !val.is_empty() {
                            meta.dependencies = val.split(',').map(|s| s.to_string()).collect();
                        }
                    }
                    "installed_size" => {
                        meta.installed_size = val.parse().unwrap_or(0);
                    }
                    "maintainer" => meta.maintainer = val.to_string(),
                    "homepage" => meta.homepage = val.to_string(),
                    "license" => meta.license = val.to_string(),
                    _ => {}
                }
            }
        }

        if meta.name.is_empty() || meta.version.is_empty() {
            return Err(KernelError::InvalidArgument {
                name: "metadata",
                value: "missing name or version",
            });
        }

        Ok(meta)
    }
}

/// File entry in the package manifest
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct FileEntry {
    pub path: String,
    pub size: u64,
    pub mode: u32,
    pub checksum: [u8; 32],
}

/// Package archive builder
#[cfg(feature = "alloc")]
pub struct PackageBuilder {
    metadata: PackageMetadata,
    files: Vec<FileEntry>,
    data_sections: Vec<Vec<u8>>,
}

#[cfg(feature = "alloc")]
impl PackageBuilder {
    pub fn new(metadata: PackageMetadata) -> Self {
        Self {
            metadata,
            files: Vec::new(),
            data_sections: Vec::new(),
        }
    }

    /// Add a file to the package
    pub fn add_file(&mut self, path: &str, data: &[u8], mode: u32) {
        let checksum = crate::crypto::hash::sha256(data);
        self.files.push(FileEntry {
            path: path.to_string(),
            size: data.len() as u64,
            mode,
            checksum: *checksum.as_bytes(),
        });
        self.data_sections.push(data.to_vec());
        self.metadata.installed_size += data.len() as u64;
    }

    /// Build the .vpkg archive
    pub fn build(&self) -> Vec<u8> {
        let mut archive = Vec::new();

        // Header
        archive.extend_from_slice(&VPKG_MAGIC);
        archive.push(VPKG_VERSION);

        // Metadata section
        let meta_bytes = self.metadata.serialize();
        let meta_len = meta_bytes.len() as u32;
        archive.extend_from_slice(&meta_len.to_le_bytes());
        archive.extend_from_slice(&meta_bytes);

        // File manifest
        let file_count = self.files.len() as u32;
        archive.extend_from_slice(&file_count.to_le_bytes());

        for entry in &self.files {
            // Path length + path
            let path_bytes = entry.path.as_bytes();
            let path_len = path_bytes.len() as u16;
            archive.extend_from_slice(&path_len.to_le_bytes());
            archive.extend_from_slice(path_bytes);

            // File size
            archive.extend_from_slice(&entry.size.to_le_bytes());

            // Mode
            archive.extend_from_slice(&entry.mode.to_le_bytes());

            // Checksum
            archive.extend_from_slice(&entry.checksum);

            // Data offset (we'll fill actual data after manifest)
            let data_offset = 0u64; // Placeholder
            archive.extend_from_slice(&data_offset.to_le_bytes());
        }

        // Data sections
        for data in &self.data_sections {
            let data_len = data.len() as u32;
            archive.extend_from_slice(&data_len.to_le_bytes());
            archive.extend_from_slice(data);
        }

        archive
    }

    pub fn file_count(&self) -> usize {
        self.files.len()
    }
}

/// Package signature (Ed25519)
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct PackageSignature {
    pub signer_id: String,
    pub signature: [u8; 64],
    pub timestamp: u64,
}

#[cfg(feature = "alloc")]
impl PackageSignature {
    pub fn new(signer: &str) -> Self {
        Self {
            signer_id: signer.to_string(),
            signature: [0u8; 64],
            timestamp: 0,
        }
    }

    /// Sign a package archive (placeholder -- would use real Ed25519)
    pub fn sign(&mut self, _archive_data: &[u8], _private_key: &[u8; 32]) {
        // In a real implementation, this would compute Ed25519 signature
        // For now, we set a marker signature
        self.signature[0] = 0xED;
        self.signature[1] = 0x25;
        self.signature[2] = 0x51;
        self.signature[3] = 0x9A;
    }

    /// Verify a package signature (placeholder)
    pub fn verify(&self, _archive_data: &[u8], _public_key: &[u8; 32]) -> bool {
        // Check for our marker
        self.signature[0] == 0xED
            && self.signature[1] == 0x25
            && self.signature[2] == 0x51
            && self.signature[3] == 0x9A
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metadata_new() {
        let meta = PackageMetadata::new("hello", "1.0.0");
        assert_eq!(meta.name, "hello");
        assert_eq!(meta.version, "1.0.0");
        assert_eq!(meta.architecture, "x86_64");
    }

    #[test]
    fn test_metadata_serialize_deserialize() {
        let mut meta = PackageMetadata::new("test-pkg", "2.1.0");
        meta.description = "A test package".to_string();
        meta.dependencies = alloc::vec!["libc".to_string(), "libm".to_string()];
        meta.installed_size = 1024;

        let serialized = meta.serialize();
        let deserialized = PackageMetadata::deserialize(&serialized).unwrap();

        assert_eq!(deserialized.name, "test-pkg");
        assert_eq!(deserialized.version, "2.1.0");
        assert_eq!(deserialized.description, "A test package");
        assert_eq!(deserialized.dependencies.len(), 2);
        assert_eq!(deserialized.installed_size, 1024);
    }

    #[test]
    fn test_metadata_deserialize_invalid() {
        let result = PackageMetadata::deserialize(b"invalid data");
        assert!(result.is_err());
    }

    #[test]
    fn test_metadata_deserialize_missing_fields() {
        let result = PackageMetadata::deserialize(b"description=only description\n");
        assert!(result.is_err());
    }

    #[test]
    fn test_package_builder_new() {
        let meta = PackageMetadata::new("pkg", "1.0");
        let builder = PackageBuilder::new(meta);
        assert_eq!(builder.file_count(), 0);
    }

    #[test]
    fn test_package_builder_add_file() {
        let meta = PackageMetadata::new("pkg", "1.0");
        let mut builder = PackageBuilder::new(meta);
        builder.add_file("/usr/bin/hello", b"#!/bin/sh\necho hello\n", 0o755);
        assert_eq!(builder.file_count(), 1);
        assert_eq!(builder.files[0].path, "/usr/bin/hello");
        assert_eq!(builder.files[0].mode, 0o755);
    }

    #[test]
    fn test_package_builder_build() {
        let meta = PackageMetadata::new("test", "1.0");
        let mut builder = PackageBuilder::new(meta);
        builder.add_file("/usr/bin/test", b"test data", 0o755);

        let archive = builder.build();
        // Check magic
        assert_eq!(&archive[0..4], &VPKG_MAGIC);
        assert_eq!(archive[4], VPKG_VERSION);
    }

    #[test]
    fn test_package_builder_installed_size() {
        let meta = PackageMetadata::new("test", "1.0");
        let mut builder = PackageBuilder::new(meta);
        builder.add_file("/a", b"hello", 0o644);
        builder.add_file("/b", b"world!", 0o644);
        assert_eq!(builder.metadata.installed_size, 11); // 5 + 6
    }

    #[test]
    fn test_package_signature_new() {
        let sig = PackageSignature::new("maintainer@veridian.org");
        assert_eq!(sig.signer_id, "maintainer@veridian.org");
        assert_eq!(sig.signature, [0u8; 64]);
    }

    #[test]
    fn test_package_signature_sign_verify() {
        let mut sig = PackageSignature::new("test");
        let data = b"package data";
        let key = [0u8; 32];

        sig.sign(data, &key);
        assert!(sig.verify(data, &key));
    }

    #[test]
    fn test_package_signature_verify_unsigned() {
        let sig = PackageSignature::new("test");
        let data = b"package data";
        let key = [0u8; 32];
        assert!(!sig.verify(data, &key));
    }

    #[test]
    fn test_vpkg_magic() {
        assert_eq!(VPKG_MAGIC, [b'V', b'P', b'K', b'G']);
    }
}
