//! Package file manifest tracking
//!
//! Maps packages to their installed files with checksums for
//! integrity verification. Each package records the files it installs
//! so they can be verified, queried, or cleaned up on removal.

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::{collections::BTreeMap, string::String, vec::Vec};

use crate::error::KernelError;

/// A record of files installed by a package
#[cfg(feature = "alloc")]
pub struct FileManifest {
    /// Map of package name -> list of installed file records
    entries: BTreeMap<String, Vec<FileRecord>>,
}

/// Record of a single installed file
#[cfg(feature = "alloc")]
#[derive(Clone)]
pub struct FileRecord {
    /// Absolute path of the installed file
    pub path: String,
    /// Size in bytes
    pub size: u64,
    /// Simple hash for integrity checking
    pub checksum: u64,
}

#[cfg(feature = "alloc")]
impl FileManifest {
    pub fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
        }
    }

    /// Record all files installed by a package.
    ///
    /// Replaces any previous manifest for this package.
    pub fn record_installation(&mut self, package: &str, files: Vec<FileRecord>) {
        self.entries.insert(String::from(package), files);
    }

    /// Verify installed files against the manifest.
    ///
    /// Checks that every recorded file still exists and matches its
    /// expected size. Returns `Ok(true)` if all files are intact,
    /// `Ok(false)` if any file is missing or has a different size.
    pub fn verify_installation(&self, package: &str) -> Result<bool, KernelError> {
        let records = self.entries.get(package).ok_or(KernelError::NotFound {
            resource: "package manifest",
            id: 0,
        })?;

        for record in records {
            if let Some(vfs_lock) = crate::fs::try_get_vfs() {
                let vfs = vfs_lock.read();
                match vfs.resolve_path(&record.path) {
                    Ok(node) => {
                        if let Ok(metadata) = node.metadata() {
                            if metadata.size as u64 != record.size {
                                return Ok(false);
                            }
                        }
                    }
                    Err(_) => return Ok(false),
                }
            }
        }

        Ok(true)
    }

    /// Get all files belonging to a package.
    pub fn get_package_files(&self, package: &str) -> Option<&[FileRecord]> {
        self.entries.get(package).map(|v| v.as_slice())
    }

    /// Find which package owns a given file path.
    ///
    /// Returns the package name if any manifest entry contains the path.
    pub fn find_file_owner(&self, path: &str) -> Option<String> {
        for (package, records) in &self.entries {
            if records.iter().any(|r| r.path == path) {
                return Some(package.clone());
            }
        }
        None
    }

    /// Remove manifest entries for a package.
    ///
    /// Returns the removed file records so the caller can clean up
    /// the actual files.
    pub fn remove_package(&mut self, package: &str) -> Option<Vec<FileRecord>> {
        self.entries.remove(package)
    }

    /// Return the total number of tracked packages.
    pub fn package_count(&self) -> usize {
        self.entries.len()
    }
}

#[cfg(feature = "alloc")]
impl Default for FileManifest {
    fn default() -> Self {
        Self::new()
    }
}

/// Compute a simple non-cryptographic hash of file data.
///
/// Uses FNV-1a (64-bit) for speed. This is not a security hash -- it is
/// only meant for detecting accidental corruption.
#[cfg(feature = "alloc")]
pub fn fnv1a_hash(data: &[u8]) -> u64 {
    const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME: u64 = 0x0100_0000_01b3;

    let mut hash = FNV_OFFSET_BASIS;
    for &byte in data {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}
