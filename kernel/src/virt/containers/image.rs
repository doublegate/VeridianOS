//! Container Image Format - layers, overlay composition, manifest, SHA-256 IDs.

#[cfg(feature = "alloc")]
use alloc::{collections::BTreeMap, string::String, vec::Vec};

use super::simple_sha256;

/// Image layer digest (SHA-256).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayerDigest {
    pub bytes: [u8; 32],
}

impl LayerDigest {
    /// Compute a SHA-256 digest of the given data.
    pub fn compute(data: &[u8]) -> Self {
        Self {
            bytes: simple_sha256(data),
        }
    }

    /// Format as hex string.
    #[cfg(feature = "alloc")]
    pub fn to_hex(&self) -> String {
        let mut s = String::with_capacity(64);
        for b in &self.bytes {
            let hi = HEX_CHARS[(b >> 4) as usize];
            let lo = HEX_CHARS[(b & 0x0f) as usize];
            s.push(hi as char);
            s.push(lo as char);
        }
        s
    }
}

const HEX_CHARS: &[u8; 16] = b"0123456789abcdef";

/// A single layer in a container image.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct ImageLayer {
    /// SHA-256 digest of the layer content.
    pub digest: LayerDigest,
    /// Compressed size in bytes.
    pub compressed_size: u64,
    /// Uncompressed size in bytes.
    pub uncompressed_size: u64,
    /// Media type (e.g., "application/vnd.oci.image.layer.v1.tar+gzip").
    pub media_type: String,
}

/// Gzip detection: check for gzip magic bytes (0x1f, 0x8b).
pub fn is_gzip(data: &[u8]) -> bool {
    data.len() >= 2 && data[0] == 0x1f && data[1] == 0x8b
}

/// TAR header: first 100 bytes are the filename, bytes 124-135 are octal size.
#[cfg(feature = "alloc")]
pub fn parse_tar_filename(header: &[u8; 512]) -> String {
    let name_end = header[..100].iter().position(|&b| b == 0).unwrap_or(100);
    let mut name = String::new();
    for &b in &header[..name_end] {
        if b.is_ascii() && b != 0 {
            name.push(b as char);
        }
    }
    name
}

/// Parse octal size from TAR header bytes 124..135.
pub fn parse_tar_size(header: &[u8; 512]) -> u64 {
    let mut size: u64 = 0;
    for &b in &header[124..135] {
        if (b'0'..=b'7').contains(&b) {
            size = size.saturating_mul(8);
            size = size.saturating_add((b - b'0') as u64);
        }
    }
    size
}

/// Container image manifest.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct ImageManifest {
    /// Schema version (usually 2).
    pub schema_version: u32,
    /// Media type of the manifest.
    pub media_type: String,
    /// Config digest (SHA-256 of config JSON).
    pub config_digest: LayerDigest,
    /// Config size in bytes.
    pub config_size: u64,
    /// Ordered list of layer digests.
    pub layer_digests: Vec<LayerDigest>,
}

/// Container image: manifest + layers + config.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct ContainerImage {
    /// Image ID (SHA-256 of the config blob).
    pub image_id: LayerDigest,
    /// Human-readable name (e.g., "alpine:3.19").
    pub name: String,
    /// Image manifest.
    pub manifest: ImageManifest,
    /// Layers in order (bottom to top).
    pub layers: Vec<ImageLayer>,
}

/// Layer cache: stores extracted layers by their digest.
#[cfg(feature = "alloc")]
pub struct LayerCache {
    /// Maps digest hex -> layer entry.
    entries: BTreeMap<String, CachedLayer>,
    /// Maximum number of cached layers.
    max_entries: usize,
}

#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct CachedLayer {
    pub digest: LayerDigest,
    pub extracted_path: String,
    pub size_bytes: u64,
    pub reference_count: u32,
}

#[cfg(feature = "alloc")]
impl LayerCache {
    pub fn new(max_entries: usize) -> Self {
        Self {
            entries: BTreeMap::new(),
            max_entries,
        }
    }

    /// Get a cached layer by digest hex.
    pub fn get(&self, digest_hex: &str) -> Option<&CachedLayer> {
        self.entries.get(digest_hex)
    }

    /// Insert a layer into the cache. Returns false if cache is full.
    pub fn insert(&mut self, layer: CachedLayer) -> bool {
        if self.entries.len() >= self.max_entries {
            return false;
        }
        let hex = layer.digest.to_hex();
        self.entries.insert(hex, layer);
        true
    }

    /// Increment reference count for a layer.
    pub fn add_ref(&mut self, digest_hex: &str) -> bool {
        if let Some(entry) = self.entries.get_mut(digest_hex) {
            entry.reference_count = entry.reference_count.saturating_add(1);
            true
        } else {
            false
        }
    }

    /// Decrement reference count. Removes the entry if it reaches zero.
    pub fn release(&mut self, digest_hex: &str) -> bool {
        let should_remove = if let Some(entry) = self.entries.get_mut(digest_hex) {
            entry.reference_count = entry.reference_count.saturating_sub(1);
            entry.reference_count == 0
        } else {
            return false;
        };
        if should_remove {
            self.entries.remove(digest_hex);
        }
        true
    }

    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }

    pub fn is_full(&self) -> bool {
        self.entries.len() >= self.max_entries
    }
}

#[cfg(feature = "alloc")]
impl ContainerImage {
    /// Compose an image from config data and a list of layer data blobs.
    pub fn compose(name: &str, config_data: &[u8], layer_data: &[&[u8]]) -> Self {
        let config_digest = LayerDigest::compute(config_data);
        let image_id = config_digest.clone();

        let mut layers = Vec::new();
        let mut layer_digests = Vec::new();
        for data in layer_data {
            let digest = LayerDigest::compute(data);
            let compressed = is_gzip(data);
            layers.push(ImageLayer {
                digest: digest.clone(),
                compressed_size: if compressed { data.len() as u64 } else { 0 },
                uncompressed_size: data.len() as u64,
                media_type: if compressed {
                    String::from("application/vnd.oci.image.layer.v1.tar+gzip")
                } else {
                    String::from("application/vnd.oci.image.layer.v1.tar")
                },
            });
            layer_digests.push(digest);
        }

        let manifest = ImageManifest {
            schema_version: 2,
            media_type: String::from("application/vnd.oci.image.manifest.v1+json"),
            config_digest,
            config_size: config_data.len() as u64,
            layer_digests,
        };

        Self {
            image_id,
            name: String::from(name),
            manifest,
            layers,
        }
    }
}
