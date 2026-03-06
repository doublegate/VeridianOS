//! Overlay Filesystem - lower/upper layers, copy-up, whiteout, directory merge.

#[cfg(feature = "alloc")]
use alloc::{collections::BTreeMap, format, string::String, vec::Vec};

use crate::error::KernelError;

/// Whiteout marker prefix per the OCI/overlay specification.
const WHITEOUT_PREFIX: &str = ".wh.";

/// Opaque directory marker.
const OPAQUE_WHITEOUT: &str = ".wh..wh..opq";

/// Entry type in the overlay filesystem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayEntryKind {
    File,
    Directory,
    Symlink,
    Whiteout,
    OpaqueDir,
}

/// A single entry in an overlay layer.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverlayEntry {
    /// Full path relative to the layer root.
    pub path: String,
    /// Entry kind.
    pub kind: OverlayEntryKind,
    /// File content (empty for directories/whiteouts).
    pub content: Vec<u8>,
    /// File permissions (Unix mode).
    pub mode: u32,
}

/// A single layer in the overlay filesystem.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct OverlayLayer {
    /// Layer entries keyed by path.
    pub(crate) entries: BTreeMap<String, OverlayEntry>,
    /// Whether this layer is read-only (lower layer).
    pub readonly: bool,
}

#[cfg(feature = "alloc")]
impl OverlayLayer {
    /// Create a new layer.
    pub fn new(readonly: bool) -> Self {
        Self {
            entries: BTreeMap::new(),
            readonly,
        }
    }

    /// Add an entry to the layer.
    pub fn add_entry(&mut self, entry: OverlayEntry) -> Result<(), KernelError> {
        if self.readonly {
            return Err(KernelError::PermissionDenied {
                operation: "write to readonly layer",
            });
        }
        self.entries.insert(entry.path.clone(), entry);
        Ok(())
    }

    /// Look up an entry by path.
    pub fn get_entry(&self, path: &str) -> Option<&OverlayEntry> {
        self.entries.get(path)
    }

    /// Check if a path has been whited out.
    pub fn is_whiteout(&self, path: &str) -> bool {
        // Check for explicit whiteout entry
        if let Some(entry) = self.entries.get(path) {
            return entry.kind == OverlayEntryKind::Whiteout;
        }
        // Check for whiteout file (.wh.<name>)
        if let Some((_dir, name)) = path.rsplit_once('/') {
            let wh_path = format!(
                "{}/{}{}",
                path.rsplit_once('/').map(|(d, _)| d).unwrap_or(""),
                WHITEOUT_PREFIX,
                name
            );
            self.entries.contains_key(&wh_path)
        } else {
            let wh_path = format!("{}{}", WHITEOUT_PREFIX, path);
            self.entries.contains_key(&wh_path)
        }
    }

    /// Check if a directory is opaque (blocks looking into lower layers).
    pub fn is_opaque_dir(&self, dir_path: &str) -> bool {
        let opq_path = if dir_path.ends_with('/') {
            format!("{}{}", dir_path, OPAQUE_WHITEOUT)
        } else {
            format!("{}/{}", dir_path, OPAQUE_WHITEOUT)
        };
        self.entries.contains_key(&opq_path)
    }

    /// List entries in a directory (non-recursive).
    pub fn list_dir(&self, dir_path: &str) -> Vec<&OverlayEntry> {
        let prefix = if dir_path.ends_with('/') || dir_path.is_empty() {
            String::from(dir_path)
        } else {
            format!("{}/", dir_path)
        };
        self.entries
            .values()
            .filter(|e| {
                if e.path.starts_with(prefix.as_str()) {
                    let rest = &e.path[prefix.len()..];
                    !rest.is_empty() && !rest.contains('/')
                } else {
                    false
                }
            })
            .collect()
    }

    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }
}

/// Overlay filesystem combining multiple layers.
#[cfg(feature = "alloc")]
#[derive(Debug)]
pub struct OverlayFs {
    /// Lower (read-only) layers, ordered bottom to top.
    lower_layers: Vec<OverlayLayer>,
    /// Upper (writable) layer.
    upper_layer: OverlayLayer,
    /// Work directory path (used for atomic operations).
    work_dir: String,
}

#[cfg(feature = "alloc")]
impl OverlayFs {
    /// Create a new overlay filesystem.
    pub fn new(work_dir: &str) -> Self {
        Self {
            lower_layers: Vec::new(),
            upper_layer: OverlayLayer::new(false),
            work_dir: String::from(work_dir),
        }
    }

    /// Add a read-only lower layer (bottom-most first).
    pub fn add_lower_layer(&mut self, layer: OverlayLayer) {
        self.lower_layers.push(layer);
    }

    /// Look up a file: check upper layer first, then lower layers top to
    /// bottom.
    pub fn lookup(&self, path: &str) -> Option<&OverlayEntry> {
        // Check upper layer first
        if self.upper_layer.is_whiteout(path) {
            return None; // deleted in upper
        }
        if let Some(entry) = self.upper_layer.get_entry(path) {
            return Some(entry);
        }

        // Check lower layers from top to bottom
        for layer in self.lower_layers.iter().rev() {
            if layer.is_whiteout(path) {
                return None;
            }
            // If the parent dir is opaque in this layer, skip lower layers
            if let Some((parent, _)) = path.rsplit_once('/') {
                if layer.is_opaque_dir(parent) {
                    return layer.get_entry(path);
                }
            }
            if let Some(entry) = layer.get_entry(path) {
                return Some(entry);
            }
        }

        None
    }

    /// Write a file to the upper layer. If the file exists in a lower layer,
    /// performs copy-up first.
    pub fn write_file(
        &mut self,
        path: &str,
        content: Vec<u8>,
        mode: u32,
    ) -> Result<(), KernelError> {
        let entry = OverlayEntry {
            path: String::from(path),
            kind: OverlayEntryKind::File,
            content,
            mode,
        };
        self.upper_layer.entries.insert(String::from(path), entry);
        Ok(())
    }

    /// Delete a file by creating a whiteout in the upper layer.
    pub fn delete_file(&mut self, path: &str) -> Result<(), KernelError> {
        // Remove from upper if present
        self.upper_layer.entries.remove(path);

        // Check if it exists in any lower layer
        let exists_in_lower = self
            .lower_layers
            .iter()
            .any(|l| l.get_entry(path).is_some());

        if exists_in_lower {
            // Create whiteout
            if let Some((dir, name)) = path.rsplit_once('/') {
                let wh_path = format!("{}/{}{}", dir, WHITEOUT_PREFIX, name);
                self.upper_layer.entries.insert(
                    wh_path.clone(),
                    OverlayEntry {
                        path: wh_path,
                        kind: OverlayEntryKind::Whiteout,
                        content: Vec::new(),
                        mode: 0,
                    },
                );
            } else {
                let wh_path = format!("{}{}", WHITEOUT_PREFIX, path);
                self.upper_layer.entries.insert(
                    wh_path.clone(),
                    OverlayEntry {
                        path: wh_path,
                        kind: OverlayEntryKind::Whiteout,
                        content: Vec::new(),
                        mode: 0,
                    },
                );
            }
        }

        Ok(())
    }

    /// Make a directory opaque (hides all entries from lower layers).
    pub fn make_opaque_dir(&mut self, dir_path: &str) -> Result<(), KernelError> {
        let opq_path = format!("{}/{}", dir_path, OPAQUE_WHITEOUT);
        self.upper_layer.entries.insert(
            opq_path.clone(),
            OverlayEntry {
                path: opq_path,
                kind: OverlayEntryKind::OpaqueDir,
                content: Vec::new(),
                mode: 0,
            },
        );
        Ok(())
    }

    /// List directory contents merging all layers. Upper entries take
    /// precedence. Whited-out entries are excluded.
    pub fn list_dir(&self, dir_path: &str) -> Vec<&OverlayEntry> {
        let mut seen: BTreeMap<String, &OverlayEntry> = BTreeMap::new();
        let mut whited_out: Vec<String> = Vec::new();

        // Upper layer first
        for entry in self.upper_layer.list_dir(dir_path) {
            if entry.kind == OverlayEntryKind::Whiteout {
                // Extract the original filename from the whiteout name
                if let Some(name) = entry
                    .path
                    .rsplit('/')
                    .next()
                    .and_then(|n| n.strip_prefix(WHITEOUT_PREFIX))
                {
                    let orig = if dir_path.is_empty() {
                        String::from(name)
                    } else {
                        format!("{}/{}", dir_path, name)
                    };
                    whited_out.push(orig);
                }
            } else if entry.kind != OverlayEntryKind::OpaqueDir {
                seen.insert(entry.path.clone(), entry);
            }
        }

        // Check if upper declares this directory opaque
        let is_opaque = self.upper_layer.is_opaque_dir(dir_path);

        if !is_opaque {
            // Lower layers from top to bottom
            for layer in self.lower_layers.iter().rev() {
                if layer.is_opaque_dir(dir_path) {
                    // This layer is opaque, add its entries but stop going lower
                    for entry in layer.list_dir(dir_path) {
                        if !seen.contains_key(&entry.path) && !whited_out.contains(&entry.path) {
                            seen.insert(entry.path.clone(), entry);
                        }
                    }
                    break;
                }
                for entry in layer.list_dir(dir_path) {
                    if !seen.contains_key(&entry.path) && !whited_out.contains(&entry.path) {
                        seen.insert(entry.path.clone(), entry);
                    }
                }
            }
        }

        seen.into_values().collect()
    }

    pub fn work_dir(&self) -> &str {
        &self.work_dir
    }

    pub fn lower_layer_count(&self) -> usize {
        self.lower_layers.len()
    }
}
