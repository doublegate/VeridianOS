//! Persistent package database
//!
//! Stores package state to VFS-backed storage at `/var/pkg/db`.
//! Uses a simple binary format for serialization so the package
//! registry survives reboots.
//!
//! ## Binary Format
//!
//! The database file is a sequence of records. Each record is:
//!
//! ```text
//! name_len:    u16 (little-endian)
//! name:        [u8; name_len]
//! version_len: u16 (little-endian)
//! version:     [u8; version_len]
//! installed_at: u64 (little-endian)
//! files_count: u32 (little-endian)
//! size_bytes:  u64 (little-endian)
//! dep_count:   u16 (little-endian)
//! for each dep:
//!     dep_len: u16 (little-endian)
//!     dep:     [u8; dep_len]
//! ```

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::{collections::BTreeMap, string::String, vec, vec::Vec};

use crate::error::KernelError;

/// Record of a tracked configuration file
#[cfg(feature = "alloc")]
#[derive(Clone)]
pub struct ConfigRecord {
    /// Absolute path of the configuration file
    pub path: String,
    /// SHA-256 hash at install time
    pub original_hash: [u8; 32],
    /// Whether the user has modified this file since install
    pub is_user_modified: bool,
}

/// On-disk package database
#[cfg(feature = "alloc")]
pub struct PackageDatabase {
    /// Map of package name -> installed package record
    packages: BTreeMap<String, DbPackageRecord>,
    /// Database file path
    db_path: String,
    /// Whether database has unsaved changes
    dirty: bool,
    /// Tracked configuration files per package
    config_files: BTreeMap<String, Vec<ConfigRecord>>,
}

/// A single record in the package database
#[cfg(feature = "alloc")]
#[derive(Clone)]
pub struct DbPackageRecord {
    /// Package name
    pub name: String,
    /// Installed version string (e.g. "1.2.3")
    pub version: String,
    /// Timestamp when the package was installed (seconds since boot)
    pub installed_at: u64,
    /// Number of files installed by the package
    pub files_count: u32,
    /// Total size of installed files in bytes
    pub size_bytes: u64,
    /// Names of packages this package depends on
    pub dependencies: Vec<String>,
}

#[cfg(feature = "alloc")]
impl PackageDatabase {
    pub fn new(db_path: &str) -> Self {
        Self {
            packages: BTreeMap::new(),
            db_path: String::from(db_path),
            dirty: false,
            config_files: BTreeMap::new(),
        }
    }

    /// Load database from VFS.
    ///
    /// If the database file does not exist or the VFS is not available,
    /// the in-memory database remains empty -- this is not an error.
    pub fn load(&mut self) -> Result<(), KernelError> {
        let vfs_lock = match crate::fs::try_get_vfs() {
            Some(v) => v,
            None => return Ok(()), // VFS not yet initialised
        };

        let data = {
            let vfs = vfs_lock.read();
            match vfs.resolve_path(&self.db_path) {
                Ok(node) => {
                    let meta = node.metadata()?;
                    let mut buf = vec![0u8; meta.size];
                    let n = node.read(0, &mut buf)?;
                    buf.truncate(n);
                    buf
                }
                Err(_) => return Ok(()), // File does not exist yet
            }
        };

        self.packages = Self::deserialize(&data)?;
        self.dirty = false;
        Ok(())
    }

    /// Save database to VFS.
    ///
    /// Creates parent directories if needed. Silently succeeds if the
    /// VFS is not available (early boot).
    pub fn save(&self) -> Result<(), KernelError> {
        if crate::fs::try_get_vfs().is_none() {
            return Ok(());
        }

        let data = self.serialize();

        // Ensure parent directories exist (e.g. /var/pkg)
        if let Some(parent_end) = self.db_path.rfind('/') {
            if parent_end > 0 {
                let parent = &self.db_path[..parent_end];
                ensure_directories(parent)?;
            }
        }

        crate::fs::write_file(&self.db_path, &data)?;
        Ok(())
    }

    /// Record a package installation.
    pub fn record_install(&mut self, record: DbPackageRecord) {
        self.packages.insert(record.name.clone(), record);
        self.dirty = true;
    }

    /// Record a package removal.
    ///
    /// Returns the removed record so the caller can log or undo it.
    pub fn record_remove(&mut self, name: &str) -> Option<DbPackageRecord> {
        let removed = self.packages.remove(name);
        if removed.is_some() {
            self.dirty = true;
        }
        removed
    }

    /// Query all installed packages.
    pub fn query_installed(&self) -> Vec<&DbPackageRecord> {
        self.packages.values().collect()
    }

    /// Check if a package is installed.
    pub fn is_installed(&self, name: &str) -> bool {
        self.packages.contains_key(name)
    }

    /// Get a package record by name.
    pub fn get(&self, name: &str) -> Option<&DbPackageRecord> {
        self.packages.get(name)
    }

    /// Return whether the database has unsaved changes.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    // ------------------------------------------------------------------
    // Configuration tracking
    // ------------------------------------------------------------------

    /// Record a configuration file for a package.
    pub fn track_config_file(&mut self, package: &str, config: ConfigRecord) {
        let configs = self.config_files.entry(String::from(package)).or_default();
        // Replace existing entry for the same path
        if let Some(pos) = configs.iter().position(|c| c.path == config.path) {
            configs[pos] = config;
        } else {
            configs.push(config);
        }
        self.dirty = true;
    }

    /// Check whether a config file has been modified by the user.
    pub fn is_config_modified(&self, package: &str, path: &str) -> bool {
        self.config_files
            .get(package)
            .and_then(|configs| configs.iter().find(|c| c.path == path))
            .map(|c| c.is_user_modified)
            .unwrap_or(false)
    }

    /// List all tracked config files for a package.
    pub fn list_config_files(&self, package: &str) -> &[ConfigRecord] {
        self.config_files.get(package).map_or(&[], |v| v.as_slice())
    }

    /// Find orphan packages (packages with zero reverse dependencies).
    ///
    /// A package is an orphan if no other installed package depends on it.
    pub fn find_orphans(&self) -> Vec<String> {
        let mut orphans = Vec::new();
        for name in self.packages.keys() {
            let is_depended_on = self
                .packages
                .values()
                .any(|record| record.dependencies.iter().any(|dep| dep == name));
            if !is_depended_on {
                orphans.push(name.clone());
            }
        }
        orphans
    }

    // ------------------------------------------------------------------
    // Serialization helpers
    // ------------------------------------------------------------------

    fn serialize(&self) -> Vec<u8> {
        let mut buf = Vec::new();

        for record in self.packages.values() {
            Self::write_str(&mut buf, &record.name);
            Self::write_str(&mut buf, &record.version);
            buf.extend_from_slice(&record.installed_at.to_le_bytes());
            buf.extend_from_slice(&record.files_count.to_le_bytes());
            buf.extend_from_slice(&record.size_bytes.to_le_bytes());

            let dep_count = record.dependencies.len() as u16;
            buf.extend_from_slice(&dep_count.to_le_bytes());
            for dep in &record.dependencies {
                Self::write_str(&mut buf, dep);
            }
        }

        buf
    }

    fn deserialize(data: &[u8]) -> Result<BTreeMap<String, DbPackageRecord>, KernelError> {
        let mut map = BTreeMap::new();
        let mut pos = 0;

        while pos < data.len() {
            let name = Self::read_str(data, &mut pos)?;
            let version = Self::read_str(data, &mut pos)?;

            let installed_at = Self::read_u64(data, &mut pos)?;
            let files_count = Self::read_u32(data, &mut pos)?;
            let size_bytes = Self::read_u64(data, &mut pos)?;

            let dep_count = Self::read_u16(data, &mut pos)? as usize;
            let mut dependencies = Vec::with_capacity(dep_count);
            for _ in 0..dep_count {
                dependencies.push(Self::read_str(data, &mut pos)?);
            }

            let record = DbPackageRecord {
                name: name.clone(),
                version,
                installed_at,
                files_count,
                size_bytes,
                dependencies,
            };
            map.insert(name, record);
        }

        Ok(map)
    }

    /// Write a length-prefixed UTF-8 string (u16 length + bytes).
    fn write_str(buf: &mut Vec<u8>, s: &str) {
        let len = s.len() as u16;
        buf.extend_from_slice(&len.to_le_bytes());
        buf.extend_from_slice(s.as_bytes());
    }

    /// Read a length-prefixed UTF-8 string.
    fn read_str(data: &[u8], pos: &mut usize) -> Result<String, KernelError> {
        let len = Self::read_u16(data, pos)? as usize;
        if *pos + len > data.len() {
            return Err(KernelError::InvalidArgument {
                name: "db_record",
                value: "truncated_string",
            });
        }
        let s = core::str::from_utf8(&data[*pos..*pos + len]).map_err(|_| {
            KernelError::InvalidArgument {
                name: "db_record",
                value: "invalid_utf8",
            }
        })?;
        *pos += len;
        Ok(String::from(s))
    }

    fn read_u16(data: &[u8], pos: &mut usize) -> Result<u16, KernelError> {
        if *pos + 2 > data.len() {
            return Err(KernelError::InvalidArgument {
                name: "db_record",
                value: "truncated_u16",
            });
        }
        let val = u16::from_le_bytes([data[*pos], data[*pos + 1]]);
        *pos += 2;
        Ok(val)
    }

    fn read_u32(data: &[u8], pos: &mut usize) -> Result<u32, KernelError> {
        if *pos + 4 > data.len() {
            return Err(KernelError::InvalidArgument {
                name: "db_record",
                value: "truncated_u32",
            });
        }
        let val = u32::from_le_bytes([data[*pos], data[*pos + 1], data[*pos + 2], data[*pos + 3]]);
        *pos += 4;
        Ok(val)
    }

    fn read_u64(data: &[u8], pos: &mut usize) -> Result<u64, KernelError> {
        if *pos + 8 > data.len() {
            return Err(KernelError::InvalidArgument {
                name: "db_record",
                value: "truncated_u64",
            });
        }
        let val = u64::from_le_bytes([
            data[*pos],
            data[*pos + 1],
            data[*pos + 2],
            data[*pos + 3],
            data[*pos + 4],
            data[*pos + 5],
            data[*pos + 6],
            data[*pos + 7],
        ]);
        *pos += 8;
        Ok(val)
    }
}

#[cfg(feature = "alloc")]
impl Default for PackageDatabase {
    fn default() -> Self {
        Self::new("/var/pkg/db")
    }
}

/// Create directory hierarchy, ignoring errors for existing directories.
#[cfg(feature = "alloc")]
fn ensure_directories(path: &str) -> Result<(), KernelError> {
    use crate::fs::Permissions;

    let components: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();

    let mut current_path = String::new();
    for component in components {
        current_path.push('/');
        current_path.push_str(component);

        if let Some(vfs_lock) = crate::fs::try_get_vfs() {
            let perms = Permissions::from_mode(0o755);
            let _ = vfs_lock.write().mkdir(&current_path, perms);
        }
    }

    Ok(())
}
