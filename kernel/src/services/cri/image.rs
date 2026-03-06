//! CRI Image Service
//!
//! Provides container image lifecycle management including pull, list,
//! status, and removal operations.

#![allow(dead_code)]

use alloc::{collections::BTreeMap, string::String, vec::Vec};
use core::sync::atomic::{AtomicU64, Ordering};

// ---------------------------------------------------------------------------
// Image Types
// ---------------------------------------------------------------------------

/// Image specification (reference to pull).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageSpec {
    /// Full image name (e.g., "docker.io/library/nginx").
    pub image_name: String,
    /// Image tag (e.g., "latest", "1.25").
    pub tag: String,
    /// Content-addressable digest (e.g., "sha256:abc...").
    pub digest: String,
}

impl ImageSpec {
    /// Create a new image spec.
    pub fn new(image_name: String, tag: String) -> Self {
        ImageSpec {
            image_name,
            tag,
            digest: String::new(),
        }
    }

    /// Create with digest.
    pub fn with_digest(image_name: String, tag: String, digest: String) -> Self {
        ImageSpec {
            image_name,
            tag,
            digest,
        }
    }

    /// Get the full reference string (name:tag).
    pub fn reference(&self) -> String {
        let mut r = self.image_name.clone();
        r.push(':');
        r.push_str(&self.tag);
        r
    }
}

/// Status of a pulled image.
#[derive(Debug, Clone)]
pub struct ImageStatus {
    /// Unique image identifier.
    pub id: u64,
    /// Size in bytes.
    pub size: u64,
    /// Repository tags (e.g., ["nginx:latest", "nginx:1.25"]).
    pub repo_tags: Vec<String>,
    /// Repository digests.
    pub repo_digests: Vec<String>,
    /// Image spec that was pulled.
    pub spec: ImageSpec,
    /// Tick when the image was pulled.
    pub pulled_tick: u64,
}

/// Authentication configuration for pulling images from private registries.
#[derive(Debug, Clone, Default)]
pub struct AuthConfig {
    /// Username.
    pub username: String,
    /// Password or token.
    pub password: String,
    /// Registry server address.
    pub server_address: String,
}

/// Image pull progress.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PullStatus {
    /// Pull has not started.
    Pending,
    /// Downloading layers.
    Downloading,
    /// Extracting layers.
    Extracting,
    /// Pull complete.
    Complete,
    /// Pull failed.
    Failed,
}

impl PullStatus {
    /// Check if pull is terminal.
    pub fn is_terminal(self) -> bool {
        matches!(self, PullStatus::Complete | PullStatus::Failed)
    }
}

/// Image service error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageError {
    /// Image not found.
    NotFound(String),
    /// Image already exists.
    AlreadyExists(String),
    /// Registry authentication failed.
    AuthenticationFailed,
    /// Registry unreachable.
    RegistryUnavailable,
    /// Image pull failed.
    PullFailed(String),
}

// ---------------------------------------------------------------------------
// Image Service
// ---------------------------------------------------------------------------

/// Next image ID generator.
static NEXT_IMAGE_ID: AtomicU64 = AtomicU64::new(1);

fn alloc_image_id() -> u64 {
    NEXT_IMAGE_ID.fetch_add(1, Ordering::Relaxed)
}

/// CRI ImageService implementation.
#[derive(Debug)]
pub struct ImageService {
    /// Stored images keyed by ID.
    images: BTreeMap<u64, ImageStatus>,
    /// Index: image reference string -> image ID.
    ref_index: BTreeMap<String, u64>,
    /// Maximum total image storage (bytes).
    max_storage: u64,
    /// Current total storage used (bytes).
    used_storage: u64,
}

impl Default for ImageService {
    fn default() -> Self {
        Self::new()
    }
}

impl ImageService {
    /// Default maximum image storage: 10 GB.
    pub const DEFAULT_MAX_STORAGE: u64 = 10 * 1024 * 1024 * 1024;

    /// Create a new image service.
    pub fn new() -> Self {
        ImageService {
            images: BTreeMap::new(),
            ref_index: BTreeMap::new(),
            max_storage: Self::DEFAULT_MAX_STORAGE,
            used_storage: 0,
        }
    }

    /// Create with a custom storage limit.
    pub fn with_max_storage(max_storage: u64) -> Self {
        ImageService {
            images: BTreeMap::new(),
            ref_index: BTreeMap::new(),
            max_storage,
            used_storage: 0,
        }
    }

    /// Pull an image from a registry.
    ///
    /// In the kernel environment this is a conceptual operation that
    /// registers the image metadata. Actual layer download would go
    /// through the network stack and filesystem.
    pub fn pull_image(
        &mut self,
        spec: ImageSpec,
        _auth: Option<AuthConfig>,
        current_tick: u64,
    ) -> Result<u64, ImageError> {
        let reference = spec.reference();

        // Check if already pulled
        if let Some(&existing_id) = self.ref_index.get(&reference) {
            return Ok(existing_id);
        }

        // Simulate a pulled image with estimated size
        let estimated_size = self.estimate_image_size(&spec);

        if self.used_storage.saturating_add(estimated_size) > self.max_storage {
            return Err(ImageError::PullFailed(String::from(
                "storage limit exceeded",
            )));
        }

        let id = alloc_image_id();
        let digest = self.compute_digest(&spec);

        let status = ImageStatus {
            id,
            size: estimated_size,
            repo_tags: alloc::vec![reference.clone()],
            repo_digests: alloc::vec![digest],
            spec,
            pulled_tick: current_tick,
        };

        self.used_storage = self.used_storage.saturating_add(estimated_size);
        self.ref_index.insert(reference, id);
        self.images.insert(id, status);

        Ok(id)
    }

    /// List all images.
    pub fn list_images(&self) -> Vec<&ImageStatus> {
        self.images.values().collect()
    }

    /// Get image status by ID.
    pub fn image_status(&self, image_id: u64) -> Option<&ImageStatus> {
        self.images.get(&image_id)
    }

    /// Get image status by reference string.
    pub fn image_status_by_ref(&self, reference: &str) -> Option<&ImageStatus> {
        self.ref_index
            .get(reference)
            .and_then(|id| self.images.get(id))
    }

    /// Remove an image.
    pub fn remove_image(&mut self, image_id: u64) -> Result<(), ImageError> {
        let image = self
            .images
            .remove(&image_id)
            .ok_or_else(|| ImageError::NotFound(alloc::format!("id={}", image_id)))?;

        self.used_storage = self.used_storage.saturating_sub(image.size);

        // Remove all ref index entries pointing to this image
        self.ref_index.retain(|_, id| *id != image_id);

        Ok(())
    }

    /// Remove an image by reference string.
    pub fn remove_image_by_ref(&mut self, reference: &str) -> Result<(), ImageError> {
        let image_id = *self
            .ref_index
            .get(reference)
            .ok_or_else(|| ImageError::NotFound(String::from(reference)))?;
        self.remove_image(image_id)
    }

    /// Get the total number of images.
    pub fn image_count(&self) -> usize {
        self.images.len()
    }

    /// Get total storage used.
    pub fn used_storage(&self) -> u64 {
        self.used_storage
    }

    /// Get available storage.
    pub fn available_storage(&self) -> u64 {
        self.max_storage.saturating_sub(self.used_storage)
    }

    /// Estimate image size from spec (deterministic pseudo-hash).
    fn estimate_image_size(&self, spec: &ImageSpec) -> u64 {
        // Deterministic size based on name length
        let base = 50 * 1024 * 1024; // 50 MB base
        let name_factor = spec.image_name.len() as u64 * 1024 * 1024;
        base + name_factor
    }

    /// Compute a deterministic digest from spec.
    fn compute_digest(&self, spec: &ImageSpec) -> String {
        // Simple deterministic digest for testing
        let mut hash: u64 = 0x5381;
        for b in spec.image_name.bytes() {
            hash = hash.wrapping_mul(33).wrapping_add(b as u64);
        }
        for b in spec.tag.bytes() {
            hash = hash.wrapping_mul(33).wrapping_add(b as u64);
        }
        alloc::format!(
            "sha256:{:016x}{:016x}{:016x}{:016x}",
            hash,
            hash ^ 0xFF,
            hash.wrapping_mul(7),
            hash.wrapping_add(42)
        )
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use alloc::string::ToString;

    use super::*;

    fn make_service() -> ImageService {
        ImageService::new()
    }

    fn test_spec() -> ImageSpec {
        ImageSpec::new(String::from("nginx"), String::from("latest"))
    }

    #[test]
    fn test_pull_image() {
        let mut svc = make_service();
        let id = svc.pull_image(test_spec(), None, 100).unwrap();
        assert!(id > 0);
        assert_eq!(svc.image_count(), 1);
    }

    #[test]
    fn test_pull_duplicate_returns_existing() {
        let mut svc = make_service();
        let id1 = svc.pull_image(test_spec(), None, 100).unwrap();
        let id2 = svc.pull_image(test_spec(), None, 200).unwrap();
        assert_eq!(id1, id2);
        assert_eq!(svc.image_count(), 1);
    }

    #[test]
    fn test_image_status() {
        let mut svc = make_service();
        let id = svc.pull_image(test_spec(), None, 100).unwrap();
        let status = svc.image_status(id).unwrap();
        assert_eq!(status.spec.image_name, "nginx");
        assert_eq!(status.pulled_tick, 100);
    }

    #[test]
    fn test_image_status_by_ref() {
        let mut svc = make_service();
        svc.pull_image(test_spec(), None, 100).unwrap();
        let status = svc.image_status_by_ref("nginx:latest").unwrap();
        assert_eq!(status.spec.tag, "latest");
    }

    #[test]
    fn test_list_images() {
        let mut svc = make_service();
        svc.pull_image(test_spec(), None, 100).unwrap();
        svc.pull_image(
            ImageSpec::new(String::from("redis"), String::from("7")),
            None,
            200,
        )
        .unwrap();
        assert_eq!(svc.list_images().len(), 2);
    }

    #[test]
    fn test_remove_image() {
        let mut svc = make_service();
        let id = svc.pull_image(test_spec(), None, 100).unwrap();
        let storage_before = svc.used_storage();
        svc.remove_image(id).unwrap();
        assert_eq!(svc.image_count(), 0);
        assert!(svc.used_storage() < storage_before);
    }

    #[test]
    fn test_remove_image_by_ref() {
        let mut svc = make_service();
        svc.pull_image(test_spec(), None, 100).unwrap();
        svc.remove_image_by_ref("nginx:latest").unwrap();
        assert_eq!(svc.image_count(), 0);
    }

    #[test]
    fn test_storage_limit() {
        let mut svc = ImageService::with_max_storage(1024); // Very small
        let result = svc.pull_image(test_spec(), None, 100);
        assert!(result.is_err());
    }
}
