//! CSI Node Service
//!
//! Provides node-local volume operations including staging (making a volume
//! available on the node) and publishing (bind-mounting into container paths).

#![allow(dead_code)]

use alloc::{collections::BTreeMap, string::String, vec::Vec};

// ---------------------------------------------------------------------------
// Node Volume Types
// ---------------------------------------------------------------------------

/// A volume staged on this node.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct StagedVolume {
    /// Volume ID.
    pub volume_id: u64,
    /// Staging target path (e.g., "/var/lib/csi/staging/vol-id").
    pub staging_target: String,
    /// Device path (e.g., "/dev/vdb").
    pub device_path: String,
    /// Filesystem type.
    pub fs_type: String,
    /// Mount options.
    pub mount_options: Vec<String>,
}

/// A volume published (bind-mounted) into a container.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct PublishedVolume {
    /// Volume ID.
    pub volume_id: u64,
    /// Target path inside the container (e.g., "/mnt/data").
    pub target_path: String,
    /// Whether mounted read-only.
    pub read_only: bool,
    /// Staging target this publish references.
    pub staging_target: String,
}

// ---------------------------------------------------------------------------
// Node Error
// ---------------------------------------------------------------------------

/// Node service error.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub enum NodeError {
    /// Volume not found.
    VolumeNotFound(u64),
    /// Volume not staged.
    NotStaged(u64),
    /// Volume already staged.
    AlreadyStaged(u64),
    /// Volume not published at target.
    NotPublished { volume_id: u64, target: String },
    /// Volume already published at target.
    AlreadyPublished { volume_id: u64, target: String },
    /// Mount operation failed.
    MountFailed(String),
}

// ---------------------------------------------------------------------------
// Node Service
// ---------------------------------------------------------------------------

/// CSI NodeService implementation.
#[derive(Debug)]
#[allow(dead_code)]
pub struct NodeService {
    /// Node identifier.
    node_id: String,
    /// Staged volumes keyed by volume ID.
    staged: BTreeMap<u64, StagedVolume>,
    /// Published volumes keyed by (volume_id, target_path).
    published: BTreeMap<(u64, String), PublishedVolume>,
    /// Maximum volumes this node can handle.
    max_volumes: usize,
}

impl NodeService {
    /// Default maximum volumes per node.
    pub const DEFAULT_MAX_VOLUMES: usize = 128;

    /// Create a new node service.
    pub fn new(node_id: String) -> Self {
        NodeService {
            node_id,
            staged: BTreeMap::new(),
            published: BTreeMap::new(),
            max_volumes: Self::DEFAULT_MAX_VOLUMES,
        }
    }

    /// Get the node ID.
    pub fn node_id(&self) -> &str {
        &self.node_id
    }

    /// Stage a volume on this node (make the device available).
    pub fn node_stage_volume(
        &mut self,
        volume_id: u64,
        staging_target: String,
        device_path: String,
        fs_type: String,
        mount_options: Vec<String>,
    ) -> Result<(), NodeError> {
        if self.staged.contains_key(&volume_id) {
            return Err(NodeError::AlreadyStaged(volume_id));
        }

        let staged = StagedVolume {
            volume_id,
            staging_target,
            device_path,
            fs_type,
            mount_options,
        };
        self.staged.insert(volume_id, staged);
        Ok(())
    }

    /// Unstage a volume (unmount from staging).
    pub fn node_unstage_volume(&mut self, volume_id: u64) -> Result<(), NodeError> {
        // Must not have any active publishes
        let has_publishes = self.published.keys().any(|(vid, _)| *vid == volume_id);
        if has_publishes {
            return Err(NodeError::MountFailed(String::from(
                "volume still has active publishes",
            )));
        }

        self.staged
            .remove(&volume_id)
            .map(|_| ())
            .ok_or(NodeError::NotStaged(volume_id))
    }

    /// Publish a volume into a container path (bind mount from staging).
    pub fn node_publish_volume(
        &mut self,
        volume_id: u64,
        target_path: String,
        read_only: bool,
    ) -> Result<(), NodeError> {
        // Must be staged first
        let staged = self
            .staged
            .get(&volume_id)
            .ok_or(NodeError::NotStaged(volume_id))?;

        let key = (volume_id, target_path.clone());
        if self.published.contains_key(&key) {
            return Err(NodeError::AlreadyPublished {
                volume_id,
                target: target_path,
            });
        }

        let published = PublishedVolume {
            volume_id,
            target_path,
            read_only,
            staging_target: staged.staging_target.clone(),
        };
        self.published.insert(key, published);
        Ok(())
    }

    /// Unpublish a volume from a container path.
    pub fn node_unpublish_volume(
        &mut self,
        volume_id: u64,
        target_path: String,
    ) -> Result<(), NodeError> {
        let key = (volume_id, target_path.clone());
        self.published
            .remove(&key)
            .map(|_| ())
            .ok_or(NodeError::NotPublished {
                volume_id,
                target: target_path,
            })
    }

    /// List staged volumes.
    pub fn list_staged(&self) -> Vec<&StagedVolume> {
        self.staged.values().collect()
    }

    /// List published volumes.
    pub fn list_published(&self) -> Vec<&PublishedVolume> {
        self.published.values().collect()
    }

    /// Get staged volume info.
    pub fn get_staged(&self, volume_id: u64) -> Option<&StagedVolume> {
        self.staged.get(&volume_id)
    }

    /// Get the number of staged volumes.
    pub fn staged_count(&self) -> usize {
        self.staged.len()
    }

    /// Get the number of published volumes.
    pub fn published_count(&self) -> usize {
        self.published.len()
    }

    /// Check if this node can accept more volumes.
    pub fn can_accept_volume(&self) -> bool {
        self.staged.len() < self.max_volumes
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use alloc::string::ToString;
    #[allow(unused_imports)]
    use alloc::vec;

    use super::*;

    fn make_service() -> NodeService {
        NodeService::new(String::from("node-1"))
    }

    #[test]
    fn test_stage_volume() {
        let mut svc = make_service();
        svc.node_stage_volume(
            1,
            String::from("/staging/vol-1"),
            String::from("/dev/vdb"),
            String::from("ext4"),
            Vec::new(),
        )
        .unwrap();
        assert_eq!(svc.staged_count(), 1);
        let staged = svc.get_staged(1).unwrap();
        assert_eq!(staged.device_path, "/dev/vdb");
    }

    #[test]
    fn test_double_stage() {
        let mut svc = make_service();
        svc.node_stage_volume(
            1,
            String::from("/s/v1"),
            String::from("/dev/vdb"),
            String::from("ext4"),
            Vec::new(),
        )
        .unwrap();
        assert_eq!(
            svc.node_stage_volume(
                1,
                String::from("/s/v1"),
                String::from("/dev/vdb"),
                String::from("ext4"),
                Vec::new()
            ),
            Err(NodeError::AlreadyStaged(1))
        );
    }

    #[test]
    fn test_unstage_volume() {
        let mut svc = make_service();
        svc.node_stage_volume(
            1,
            String::from("/s/v1"),
            String::from("/dev/vdb"),
            String::from("ext4"),
            Vec::new(),
        )
        .unwrap();
        svc.node_unstage_volume(1).unwrap();
        assert_eq!(svc.staged_count(), 0);
    }

    #[test]
    fn test_unstage_not_staged() {
        let mut svc = make_service();
        assert_eq!(svc.node_unstage_volume(999), Err(NodeError::NotStaged(999)));
    }

    #[test]
    fn test_publish_volume() {
        let mut svc = make_service();
        svc.node_stage_volume(
            1,
            String::from("/s/v1"),
            String::from("/dev/vdb"),
            String::from("ext4"),
            Vec::new(),
        )
        .unwrap();
        svc.node_publish_volume(1, String::from("/mnt/data"), false)
            .unwrap();
        assert_eq!(svc.published_count(), 1);
    }

    #[test]
    fn test_publish_not_staged() {
        let mut svc = make_service();
        assert_eq!(
            svc.node_publish_volume(999, String::from("/mnt"), false),
            Err(NodeError::NotStaged(999))
        );
    }

    #[test]
    fn test_unpublish_volume() {
        let mut svc = make_service();
        svc.node_stage_volume(
            1,
            String::from("/s/v1"),
            String::from("/dev/vdb"),
            String::from("ext4"),
            Vec::new(),
        )
        .unwrap();
        svc.node_publish_volume(1, String::from("/mnt/data"), false)
            .unwrap();
        svc.node_unpublish_volume(1, String::from("/mnt/data"))
            .unwrap();
        assert_eq!(svc.published_count(), 0);
    }

    #[test]
    fn test_unstage_with_active_publish() {
        let mut svc = make_service();
        svc.node_stage_volume(
            1,
            String::from("/s/v1"),
            String::from("/dev/vdb"),
            String::from("ext4"),
            Vec::new(),
        )
        .unwrap();
        svc.node_publish_volume(1, String::from("/mnt/data"), false)
            .unwrap();
        assert!(svc.node_unstage_volume(1).is_err());
    }
}
