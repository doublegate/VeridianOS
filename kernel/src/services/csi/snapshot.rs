//! CSI Snapshot Service
//!
//! Provides COW snapshot creation, deletion, and listing for volumes.

#![allow(dead_code)]

use alloc::{collections::BTreeMap, string::String, vec::Vec};
use core::sync::atomic::{AtomicU64, Ordering};

// ---------------------------------------------------------------------------
// Snapshot Types
// ---------------------------------------------------------------------------

/// A point-in-time snapshot of a volume.
#[derive(Debug, Clone)]
pub struct Snapshot {
    /// Unique snapshot identifier.
    pub id: u64,
    /// Source volume ID.
    pub source_volume_id: u64,
    /// Snapshot size in bytes.
    pub size_bytes: u64,
    /// Tick when the snapshot was created.
    pub created_tick: u64,
    /// Whether the snapshot is ready to use.
    pub ready: bool,
    /// Human-readable name.
    pub name: String,
}

/// Snapshot error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapshotError {
    /// Snapshot not found.
    NotFound(u64),
    /// Source volume not found.
    SourceNotFound(u64),
    /// Snapshot already exists with this name.
    AlreadyExists(String),
    /// Snapshot is not ready.
    NotReady(u64),
}

// ---------------------------------------------------------------------------
// Snapshot Service
// ---------------------------------------------------------------------------

/// Next snapshot ID generator.
static NEXT_SNAPSHOT_ID: AtomicU64 = AtomicU64::new(1);

fn alloc_snapshot_id() -> u64 {
    NEXT_SNAPSHOT_ID.fetch_add(1, Ordering::Relaxed)
}

/// CSI Snapshot Service implementation.
#[derive(Debug)]
pub struct SnapshotService {
    /// Snapshots keyed by ID.
    snapshots: BTreeMap<u64, Snapshot>,
    /// Name to ID index.
    name_index: BTreeMap<String, u64>,
    /// Known volume IDs (for validation).
    known_volumes: BTreeMap<u64, u64>, // volume_id -> capacity
}

impl Default for SnapshotService {
    fn default() -> Self {
        Self::new()
    }
}

impl SnapshotService {
    /// Create a new snapshot service.
    pub fn new() -> Self {
        SnapshotService {
            snapshots: BTreeMap::new(),
            name_index: BTreeMap::new(),
            known_volumes: BTreeMap::new(),
        }
    }

    /// Register a known volume (for snapshot source validation).
    pub fn register_volume(&mut self, volume_id: u64, capacity_bytes: u64) {
        self.known_volumes.insert(volume_id, capacity_bytes);
    }

    /// Unregister a volume.
    pub fn unregister_volume(&mut self, volume_id: u64) {
        self.known_volumes.remove(&volume_id);
    }

    /// Create a COW snapshot of a volume.
    pub fn create_snapshot(
        &mut self,
        name: String,
        source_volume_id: u64,
        current_tick: u64,
    ) -> Result<u64, SnapshotError> {
        // Check name uniqueness
        if self.name_index.contains_key(&name) {
            return Err(SnapshotError::AlreadyExists(name));
        }

        // Look up source volume capacity
        let capacity = self
            .known_volumes
            .get(&source_volume_id)
            .ok_or(SnapshotError::SourceNotFound(source_volume_id))?;

        let id = alloc_snapshot_id();
        let snapshot = Snapshot {
            id,
            source_volume_id,
            size_bytes: *capacity,
            created_tick: current_tick,
            ready: true, // COW snapshots are instant
            name: name.clone(),
        };

        self.name_index.insert(name, id);
        self.snapshots.insert(id, snapshot);
        Ok(id)
    }

    /// Delete a snapshot.
    pub fn delete_snapshot(&mut self, snapshot_id: u64) -> Result<(), SnapshotError> {
        let snapshot = self
            .snapshots
            .remove(&snapshot_id)
            .ok_or(SnapshotError::NotFound(snapshot_id))?;
        self.name_index.remove(&snapshot.name);
        Ok(())
    }

    /// List all snapshots, optionally filtered by source volume.
    pub fn list_snapshots(&self, source_filter: Option<u64>) -> Vec<&Snapshot> {
        self.snapshots
            .values()
            .filter(|s| source_filter.is_none() || Some(s.source_volume_id) == source_filter)
            .collect()
    }

    /// Get snapshot by ID.
    pub fn get_snapshot(&self, snapshot_id: u64) -> Option<&Snapshot> {
        self.snapshots.get(&snapshot_id)
    }

    /// Get snapshot by name.
    pub fn get_snapshot_by_name(&self, name: &str) -> Option<&Snapshot> {
        self.name_index
            .get(name)
            .and_then(|id| self.snapshots.get(id))
    }

    /// Get the total number of snapshots.
    pub fn snapshot_count(&self) -> usize {
        self.snapshots.len()
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

    fn make_service() -> SnapshotService {
        let mut svc = SnapshotService::new();
        svc.register_volume(1, 1024 * 1024 * 1024);
        svc.register_volume(2, 2048 * 1024 * 1024);
        svc
    }

    #[test]
    fn test_create_snapshot() {
        let mut svc = make_service();
        let id = svc.create_snapshot(String::from("snap-1"), 1, 100).unwrap();
        let snap = svc.get_snapshot(id).unwrap();
        assert_eq!(snap.source_volume_id, 1);
        assert!(snap.ready);
    }

    #[test]
    fn test_create_snapshot_unknown_volume() {
        let mut svc = make_service();
        assert_eq!(
            svc.create_snapshot(String::from("snap"), 999, 100),
            Err(SnapshotError::SourceNotFound(999))
        );
    }

    #[test]
    fn test_create_duplicate_name() {
        let mut svc = make_service();
        svc.create_snapshot(String::from("snap-1"), 1, 100).unwrap();
        assert!(svc.create_snapshot(String::from("snap-1"), 2, 200).is_err());
    }

    #[test]
    fn test_delete_snapshot() {
        let mut svc = make_service();
        let id = svc.create_snapshot(String::from("snap-1"), 1, 100).unwrap();
        svc.delete_snapshot(id).unwrap();
        assert_eq!(svc.snapshot_count(), 0);
    }

    #[test]
    fn test_list_snapshots_filter() {
        let mut svc = make_service();
        svc.create_snapshot(String::from("s1"), 1, 100).unwrap();
        svc.create_snapshot(String::from("s2"), 1, 200).unwrap();
        svc.create_snapshot(String::from("s3"), 2, 300).unwrap();

        let all = svc.list_snapshots(None);
        assert_eq!(all.len(), 3);

        let vol1 = svc.list_snapshots(Some(1));
        assert_eq!(vol1.len(), 2);
    }

    #[test]
    fn test_get_by_name() {
        let mut svc = make_service();
        svc.create_snapshot(String::from("my-snap"), 1, 100)
            .unwrap();
        assert!(svc.get_snapshot_by_name("my-snap").is_some());
        assert!(svc.get_snapshot_by_name("other").is_none());
    }
}
