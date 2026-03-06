//! CSI Controller Service
//!
//! Provides volume lifecycle management including creation, deletion,
//! publishing, and capacity tracking.

#![allow(dead_code)]

use alloc::{collections::BTreeMap, string::String, vec::Vec};
use core::sync::atomic::{AtomicU64, Ordering};

// ---------------------------------------------------------------------------
// Volume Types
// ---------------------------------------------------------------------------

/// Volume access type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessType {
    /// Block device access.
    Block,
    /// Mounted filesystem access.
    Mount,
}

/// Volume access mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessMode {
    /// Single node writer.
    SingleNodeWriter,
    /// Single node read-only.
    SingleNodeReadOnly,
    /// Multi-node read-only.
    MultiNodeReadOnly,
    /// Multi-node multi-writer.
    MultiNodeMultiWriter,
}

/// Volume state in its lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VolumeState {
    /// Volume is being created.
    #[default]
    Creating,
    /// Volume is available for use.
    Available,
    /// Volume is currently in use.
    InUse,
    /// Volume is being deleted.
    Deleting,
}

/// A storage volume.
#[derive(Debug, Clone)]
pub struct Volume {
    /// Unique volume identifier.
    pub id: u64,
    /// Human-readable name.
    pub name: String,
    /// Capacity in bytes.
    pub capacity_bytes: u64,
    /// Access type (block or mount).
    pub access_type: AccessType,
    /// Filesystem type (e.g., "ext4", "xfs"). Empty for block.
    pub fs_type: String,
    /// Node this volume is published to (if any).
    pub node_id: Option<String>,
    /// Current state.
    pub state: VolumeState,
    /// Access mode.
    pub access_mode: AccessMode,
    /// Volume attributes.
    pub attributes: BTreeMap<String, String>,
    /// Tick when created.
    pub created_tick: u64,
}

// ---------------------------------------------------------------------------
// Controller Error
// ---------------------------------------------------------------------------

/// CSI controller error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControllerError {
    /// Volume not found.
    VolumeNotFound(u64),
    /// Volume already exists.
    VolumeAlreadyExists(String),
    /// Volume is in use and cannot be deleted.
    VolumeInUse(u64),
    /// Insufficient capacity.
    InsufficientCapacity,
    /// Invalid state for operation.
    InvalidState { volume_id: u64, state: VolumeState },
    /// Volume already published to a node.
    AlreadyPublished(u64),
    /// Volume not published.
    NotPublished(u64),
}

// ---------------------------------------------------------------------------
// Controller Service
// ---------------------------------------------------------------------------

/// Next volume ID generator.
static NEXT_VOLUME_ID: AtomicU64 = AtomicU64::new(1);

fn alloc_volume_id() -> u64 {
    NEXT_VOLUME_ID.fetch_add(1, Ordering::Relaxed)
}

/// CSI ControllerService implementation.
#[derive(Debug)]
pub struct ControllerService {
    /// Volumes keyed by ID.
    volumes: BTreeMap<u64, Volume>,
    /// Name to ID index.
    name_index: BTreeMap<String, u64>,
    /// Total capacity available (bytes).
    total_capacity: u64,
    /// Capacity currently used (bytes).
    used_capacity: u64,
}

impl Default for ControllerService {
    fn default() -> Self {
        Self::new()
    }
}

impl ControllerService {
    /// Default total capacity: 100 GB.
    pub const DEFAULT_CAPACITY: u64 = 100 * 1024 * 1024 * 1024;

    /// Create a new controller service.
    pub fn new() -> Self {
        ControllerService {
            volumes: BTreeMap::new(),
            name_index: BTreeMap::new(),
            total_capacity: Self::DEFAULT_CAPACITY,
            used_capacity: 0,
        }
    }

    /// Create with custom capacity.
    pub fn with_capacity(total_capacity: u64) -> Self {
        ControllerService {
            volumes: BTreeMap::new(),
            name_index: BTreeMap::new(),
            total_capacity,
            used_capacity: 0,
        }
    }

    /// Create a new volume.
    pub fn create_volume(
        &mut self,
        name: String,
        capacity_bytes: u64,
        access_type: AccessType,
        fs_type: String,
        access_mode: AccessMode,
        current_tick: u64,
    ) -> Result<u64, ControllerError> {
        // Check for duplicate name
        if self.name_index.contains_key(&name) {
            return Err(ControllerError::VolumeAlreadyExists(name));
        }

        // Check capacity
        if self.used_capacity.saturating_add(capacity_bytes) > self.total_capacity {
            return Err(ControllerError::InsufficientCapacity);
        }

        let id = alloc_volume_id();
        let volume = Volume {
            id,
            name: name.clone(),
            capacity_bytes,
            access_type,
            fs_type,
            node_id: None,
            state: VolumeState::Available,
            access_mode,
            attributes: BTreeMap::new(),
            created_tick: current_tick,
        };

        self.used_capacity = self.used_capacity.saturating_add(capacity_bytes);
        self.name_index.insert(name, id);
        self.volumes.insert(id, volume);
        Ok(id)
    }

    /// Delete a volume.
    pub fn delete_volume(&mut self, volume_id: u64) -> Result<(), ControllerError> {
        let volume = self
            .volumes
            .get(&volume_id)
            .ok_or(ControllerError::VolumeNotFound(volume_id))?;

        if volume.state == VolumeState::InUse {
            return Err(ControllerError::VolumeInUse(volume_id));
        }

        let capacity = volume.capacity_bytes;
        let name = volume.name.clone();

        self.volumes.remove(&volume_id);
        self.name_index.remove(&name);
        self.used_capacity = self.used_capacity.saturating_sub(capacity);
        Ok(())
    }

    /// Get remaining capacity.
    pub fn get_capacity(&self) -> u64 {
        self.total_capacity.saturating_sub(self.used_capacity)
    }

    /// Publish a volume to a node (make it available for node operations).
    pub fn controller_publish(
        &mut self,
        volume_id: u64,
        node_id: String,
    ) -> Result<(), ControllerError> {
        let volume = self
            .volumes
            .get_mut(&volume_id)
            .ok_or(ControllerError::VolumeNotFound(volume_id))?;

        if volume.state != VolumeState::Available {
            return Err(ControllerError::InvalidState {
                volume_id,
                state: volume.state,
            });
        }

        if volume.node_id.is_some() {
            return Err(ControllerError::AlreadyPublished(volume_id));
        }

        volume.node_id = Some(node_id);
        volume.state = VolumeState::InUse;
        Ok(())
    }

    /// Unpublish a volume from a node.
    pub fn controller_unpublish(&mut self, volume_id: u64) -> Result<(), ControllerError> {
        let volume = self
            .volumes
            .get_mut(&volume_id)
            .ok_or(ControllerError::VolumeNotFound(volume_id))?;

        if volume.node_id.is_none() {
            return Err(ControllerError::NotPublished(volume_id));
        }

        volume.node_id = None;
        volume.state = VolumeState::Available;
        Ok(())
    }

    /// List all volumes.
    pub fn list_volumes(&self) -> Vec<&Volume> {
        self.volumes.values().collect()
    }

    /// Get volume by ID.
    pub fn get_volume(&self, volume_id: u64) -> Option<&Volume> {
        self.volumes.get(&volume_id)
    }

    /// Get volume by name.
    pub fn get_volume_by_name(&self, name: &str) -> Option<&Volume> {
        self.name_index
            .get(name)
            .and_then(|id| self.volumes.get(id))
    }

    /// Validate volume capabilities.
    pub fn validate_capabilities(
        &self,
        volume_id: u64,
        access_mode: AccessMode,
    ) -> Result<bool, ControllerError> {
        let volume = self
            .volumes
            .get(&volume_id)
            .ok_or(ControllerError::VolumeNotFound(volume_id))?;
        Ok(volume.access_mode == access_mode)
    }

    /// Get the total number of volumes.
    pub fn volume_count(&self) -> usize {
        self.volumes.len()
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

    fn make_service() -> ControllerService {
        ControllerService::new()
    }

    #[test]
    fn test_create_volume() {
        let mut svc = make_service();
        let id = svc
            .create_volume(
                String::from("data-vol"),
                1024 * 1024 * 1024,
                AccessType::Mount,
                String::from("ext4"),
                AccessMode::SingleNodeWriter,
                100,
            )
            .unwrap();
        assert!(id > 0);
        let vol = svc.get_volume(id).unwrap();
        assert_eq!(vol.name, "data-vol");
        assert_eq!(vol.state, VolumeState::Available);
    }

    #[test]
    fn test_create_duplicate_name() {
        let mut svc = make_service();
        svc.create_volume(
            String::from("vol1"),
            1024,
            AccessType::Mount,
            String::from("ext4"),
            AccessMode::SingleNodeWriter,
            100,
        )
        .unwrap();
        let result = svc.create_volume(
            String::from("vol1"),
            1024,
            AccessType::Mount,
            String::from("ext4"),
            AccessMode::SingleNodeWriter,
            200,
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_delete_volume() {
        let mut svc = make_service();
        let id = svc
            .create_volume(
                String::from("vol1"),
                1024,
                AccessType::Block,
                String::new(),
                AccessMode::SingleNodeWriter,
                100,
            )
            .unwrap();
        svc.delete_volume(id).unwrap();
        assert_eq!(svc.volume_count(), 0);
    }

    #[test]
    fn test_delete_in_use_volume() {
        let mut svc = make_service();
        let id = svc
            .create_volume(
                String::from("vol1"),
                1024,
                AccessType::Mount,
                String::from("ext4"),
                AccessMode::SingleNodeWriter,
                100,
            )
            .unwrap();
        svc.controller_publish(id, String::from("node-1")).unwrap();
        assert_eq!(svc.delete_volume(id), Err(ControllerError::VolumeInUse(id)));
    }

    #[test]
    fn test_publish_unpublish() {
        let mut svc = make_service();
        let id = svc
            .create_volume(
                String::from("vol1"),
                1024,
                AccessType::Mount,
                String::from("ext4"),
                AccessMode::SingleNodeWriter,
                100,
            )
            .unwrap();

        svc.controller_publish(id, String::from("node-1")).unwrap();
        let vol = svc.get_volume(id).unwrap();
        assert_eq!(vol.state, VolumeState::InUse);

        svc.controller_unpublish(id).unwrap();
        let vol = svc.get_volume(id).unwrap();
        assert_eq!(vol.state, VolumeState::Available);
    }

    #[test]
    fn test_capacity_tracking() {
        let mut svc = ControllerService::with_capacity(2048);
        svc.create_volume(
            String::from("v1"),
            1024,
            AccessType::Block,
            String::new(),
            AccessMode::SingleNodeWriter,
            100,
        )
        .unwrap();
        assert_eq!(svc.get_capacity(), 1024);

        // Should fail: not enough capacity
        let result = svc.create_volume(
            String::from("v2"),
            2048,
            AccessType::Block,
            String::new(),
            AccessMode::SingleNodeWriter,
            200,
        );
        assert_eq!(result, Err(ControllerError::InsufficientCapacity));
    }

    #[test]
    fn test_list_volumes() {
        let mut svc = make_service();
        svc.create_volume(
            String::from("v1"),
            1024,
            AccessType::Block,
            String::new(),
            AccessMode::SingleNodeWriter,
            100,
        )
        .unwrap();
        svc.create_volume(
            String::from("v2"),
            2048,
            AccessType::Mount,
            String::from("xfs"),
            AccessMode::MultiNodeReadOnly,
            200,
        )
        .unwrap();
        assert_eq!(svc.list_volumes().len(), 2);
    }

    #[test]
    fn test_validate_capabilities() {
        let mut svc = make_service();
        let id = svc
            .create_volume(
                String::from("v1"),
                1024,
                AccessType::Block,
                String::new(),
                AccessMode::SingleNodeWriter,
                100,
            )
            .unwrap();
        assert!(svc
            .validate_capabilities(id, AccessMode::SingleNodeWriter)
            .unwrap());
        assert!(!svc
            .validate_capabilities(id, AccessMode::MultiNodeReadOnly)
            .unwrap());
    }

    #[test]
    fn test_get_volume_by_name() {
        let mut svc = make_service();
        svc.create_volume(
            String::from("my-vol"),
            1024,
            AccessType::Mount,
            String::from("ext4"),
            AccessMode::SingleNodeWriter,
            100,
        )
        .unwrap();
        assert!(svc.get_volume_by_name("my-vol").is_some());
        assert!(svc.get_volume_by_name("other").is_none());
    }

    #[test]
    fn test_unpublish_not_published() {
        let mut svc = make_service();
        let id = svc
            .create_volume(
                String::from("v1"),
                1024,
                AccessType::Block,
                String::new(),
                AccessMode::SingleNodeWriter,
                100,
            )
            .unwrap();
        assert_eq!(
            svc.controller_unpublish(id),
            Err(ControllerError::NotPublished(id))
        );
    }
}
