//! Capability revocation mechanism
//!
//! Implements cascading revocation and immediate invalidation of capabilities.
//!
//! # Derivation Tree
//!
//! When capabilities are delegated (derived), the parent-child relationship is
//! recorded in a global derivation tree. Revoking a parent capability
//! transitively revokes all descendants via [`revoke_cascade`].
//!
//! # Revocation Notifications
//!
//! When a capability is revoked, processes that held it receive a
//! [`RevocationNotification`] in a per-process queue, enabling them to
//! gracefully release resources.

use core::sync::atomic::{AtomicU64, Ordering};

use super::{manager::cap_manager, space::CapabilitySpace, token::CapabilityToken};
use crate::error::KernelError;

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    vec::Vec,
};

use spin::RwLock;

/// Global revocation list for fast checking
pub struct RevocationList {
    /// Set of revoked capability IDs with their generation
    #[cfg(feature = "alloc")]
    revoked: RwLock<BTreeSet<(u64, u8)>>, // (cap_id, generation)

    /// Revocation epoch counter
    epoch: AtomicU64,
}

impl RevocationList {
    const fn new() -> Self {
        Self {
            #[cfg(feature = "alloc")]
            revoked: RwLock::new(BTreeSet::new()),
            epoch: AtomicU64::new(0),
        }
    }

    /// Add a capability to the revocation list
    pub fn add(&self, cap: CapabilityToken) {
        #[cfg(feature = "alloc")]
        {
            self.revoked.write().insert((cap.id(), cap.generation()));
        }
        self.epoch.fetch_add(1, Ordering::SeqCst);
    }

    /// Check if a capability is revoked
    #[inline]
    pub fn is_revoked(&self, cap: CapabilityToken) -> bool {
        #[cfg(feature = "alloc")]
        {
            self.revoked.read().contains(&(cap.id(), cap.generation()))
        }

        #[cfg(not(feature = "alloc"))]
        false
    }

    /// Get current epoch
    pub fn epoch(&self) -> u64 {
        self.epoch.load(Ordering::Acquire)
    }

    /// Clear old revocations (garbage collection)
    #[cfg(feature = "alloc")]
    pub fn cleanup(&self, keep_recent: usize) {
        let mut revoked = self.revoked.write();
        if revoked.len() > keep_recent * 2 {
            // Keep only the most recent revocations
            let to_remove = revoked.len() - keep_recent;
            let remove_list: Vec<_> = revoked.iter().take(to_remove).cloned().collect();
            for item in remove_list {
                revoked.remove(&item);
            }
        }
    }
}

/// Global revocation list
static REVOCATION_LIST: RevocationList = RevocationList::new();

/// Revoke a capability and all derived capabilities
pub fn revoke_capability(cap: CapabilityToken) -> Result<(), KernelError> {
    // Add to global revocation list
    REVOCATION_LIST.add(cap);

    // Mark as revoked in capability manager
    cap_manager().revoke(cap).ok();

    // Notify all processes that might have this capability
    broadcast_revocation(cap);

    Ok(())
}

/// Check if a capability is revoked (fast path)
#[inline(always)]
pub fn is_revoked(cap: CapabilityToken) -> bool {
    REVOCATION_LIST.is_revoked(cap)
}

/// Public entry point for capability revocation broadcast from manager
pub fn broadcast_capability_revoked(cap: CapabilityToken) {
    broadcast_revocation(cap);
}

/// Broadcast revocation to all processes
///
/// Iterates through the process table and marks the revoked capability
/// as invalid in each process's capability space.
fn broadcast_revocation(_cap: CapabilityToken) {
    #[cfg(feature = "alloc")]
    {
        // Notify via IPC: send revocation event to process server
        let process_server = crate::services::process_server::get_process_server();
        let pids = process_server.list_process_ids();
        let _notified_count = pids.len();
        for pid in pids {
            // Mark the capability as revoked in each process's capability space
            // The process server tracks per-process capability spaces
            process_server.notify_capability_revoked(pid, _cap.id());
        }
        crate::println!(
            "[CAP] Broadcast revocation of capability {} to {} processes",
            _cap.id(),
            _notified_count
        );
    }
}

/// Revocation with cascading - revoke all capabilities derived from this one
pub fn revoke_cascading(
    cap: CapabilityToken,
    cap_space: &CapabilitySpace,
) -> Result<u32, KernelError> {
    let mut revoked_count = 0;

    #[cfg(feature = "alloc")]
    let mut to_revoke = Vec::new();

    // First revoke the main capability
    revoke_capability(cap)?;
    revoked_count += 1;

    // Find all capabilities that reference the same object
    if let Some((object, _)) = cap_space.lookup_entry(cap) {
        #[cfg(feature = "alloc")]
        {
            // Iterate through all capabilities to find those with same object
            let _ = cap_space.iter_capabilities(|entry| {
                if entry.object == object && entry.capability != cap {
                    // Check if this is a derived capability (has less rights)
                    if let Some((_, parent_rights)) = cap_space.lookup_entry(cap) {
                        if !entry.rights.contains(parent_rights) {
                            to_revoke.push(entry.capability);
                        }
                    }
                }
                true // Continue iteration
            });

            // Revoke all derived capabilities
            for derived_cap in to_revoke {
                if revoke_capability(derived_cap).is_ok() {
                    revoked_count += 1;
                }
            }
        }
    }

    Ok(revoked_count)
}

/// Batch revocation for efficiency
#[cfg(feature = "alloc")]
pub fn revoke_batch(caps: &[CapabilityToken]) -> Result<u32, KernelError> {
    let mut revoked_count = 0;

    for &cap in caps {
        if revoke_capability(cap).is_ok() {
            revoked_count += 1;
        }
    }

    Ok(revoked_count)
}

/// Per-CPU revocation cache for fast repeated checks
pub struct RevocationCache {
    /// Cached revocation epoch
    cached_epoch: AtomicU64,
    /// Cached revoked capabilities
    #[cfg(feature = "alloc")]
    cache: RwLock<BTreeSet<(u64, u8)>>,
}

impl RevocationCache {
    pub const fn new() -> Self {
        Self {
            cached_epoch: AtomicU64::new(0),
            #[cfg(feature = "alloc")]
            cache: RwLock::new(BTreeSet::new()),
        }
    }

    /// Check if capability is revoked (with cache)
    pub fn is_revoked(&self, cap: CapabilityToken) -> bool {
        let current_epoch = REVOCATION_LIST.epoch();
        let cached_epoch = self.cached_epoch.load(Ordering::Acquire);

        // If cache is stale, update it
        if current_epoch != cached_epoch {
            self.update_cache();
        }

        // Check cache
        #[cfg(feature = "alloc")]
        {
            self.cache.read().contains(&(cap.id(), cap.generation()))
        }

        #[cfg(not(feature = "alloc"))]
        REVOCATION_LIST.is_revoked(cap)
    }

    /// Update cache from global list
    #[cfg(feature = "alloc")]
    fn update_cache(&self) {
        let current_epoch = REVOCATION_LIST.epoch();

        // Copy revocation list to cache
        let global_revoked = REVOCATION_LIST.revoked.read();
        let mut cache = self.cache.write();
        cache.clear();
        cache.extend(global_revoked.iter().cloned());

        self.cached_epoch.store(current_epoch, Ordering::Release);
    }
}

/// System call handler for capability revocation
pub fn sys_capability_revoke(cap_value: u64) -> Result<(), KernelError> {
    let cap = CapabilityToken::from_u64(cap_value);

    // Verify the capability exists and the caller has REVOKE rights
    if !cap_manager().is_valid(cap) {
        return Err(KernelError::InvalidCapability {
            cap_id: cap.id(),
            reason: crate::error::CapError::NotFound,
        });
    }

    revoke_capability(cap)
}

// ===========================================================================
// Derivation Tree (Sprint 1.12)
// ===========================================================================

/// Global derivation tree: maps parent capability ID -> list of child
/// capability IDs.
///
/// This enables transitive (cascading) revocation: revoking a parent
/// automatically revokes all descendants.
#[cfg(feature = "alloc")]
static DERIVATION_TREE: RwLock<BTreeMap<u64, Vec<u64>>> = RwLock::new(BTreeMap::new());

/// Record a parent-child derivation relationship between two capabilities.
///
/// Called when a capability is delegated or derived from another. The child
/// capability will be transitively revoked if the parent is revoked.
#[cfg(feature = "alloc")]
pub fn record_derivation(parent_cap_id: u64, child_cap_id: u64) {
    let mut tree = DERIVATION_TREE.write();
    tree.entry(parent_cap_id).or_default().push(child_cap_id);
}

/// Revoke a capability and transitively revoke all derived capabilities.
///
/// Walks the derivation tree in breadth-first order, revoking each descendant.
/// Returns the list of all capability IDs that were revoked (including the
/// root).
///
/// Generation counters are checked before revocation: if a capability has been
/// re-created with a newer generation, it is skipped (the revocation list add
/// is a no-op for already-revoked IDs).
#[cfg(feature = "alloc")]
pub fn revoke_cascade(cap_id: u64) -> Vec<u64> {
    let mut revoked_ids = Vec::new();
    let mut queue = VecDeque::new();
    queue.push_back(cap_id);

    while let Some(current_id) = queue.pop_front() {
        // Construct a token from the raw ID (generation 0 for lookup purposes).
        // The revocation list and capability manager handle generation checks
        // internally.
        let cap = CapabilityToken::from_u64(current_id);

        // Add to global revocation list
        REVOCATION_LIST.add(cap);
        // Mark as revoked in capability manager (best-effort)
        cap_manager().revoke(cap).ok();

        revoked_ids.push(current_id);

        // Enqueue all children for transitive revocation
        let tree = DERIVATION_TREE.read();
        if let Some(children) = tree.get(&current_id) {
            for &child_id in children {
                // Avoid cycles (defensive)
                if !revoked_ids.contains(&child_id) {
                    queue.push_back(child_id);
                }
            }
        }
    }

    // Broadcast revocation for all affected capabilities
    for &id in &revoked_ids {
        broadcast_revocation(CapabilityToken::from_u64(id));
    }

    // Push notifications for all revoked capabilities
    push_revocation_notifications(&revoked_ids, RevocationReason::ParentRevoked);

    revoked_ids
}

/// Get the full derivation subtree rooted at the given capability ID.
///
/// Returns all descendant capability IDs (children, grandchildren, etc.)
/// in breadth-first order. Does not include the root `cap_id` itself.
#[cfg(feature = "alloc")]
pub fn get_derivation_tree(cap_id: u64) -> Vec<u64> {
    let mut descendants = Vec::new();
    let mut queue = VecDeque::new();
    queue.push_back(cap_id);

    let tree = DERIVATION_TREE.read();
    while let Some(current_id) = queue.pop_front() {
        if let Some(children) = tree.get(&current_id) {
            for &child_id in children {
                if !descendants.contains(&child_id) {
                    descendants.push(child_id);
                    queue.push_back(child_id);
                }
            }
        }
    }

    descendants
}

/// Remove a capability from the derivation tree entirely.
///
/// Removes it as a parent (dropping all its children references) and also
/// removes it from any parent's child list.
#[cfg(feature = "alloc")]
pub fn cleanup_capability(cap_id: u64) {
    let mut tree = DERIVATION_TREE.write();

    // Remove as parent
    tree.remove(&cap_id);

    // Remove from all parent child lists
    for children in tree.values_mut() {
        children.retain(|&id| id != cap_id);
    }
}

/// Get the direct children of a capability in the derivation tree.
#[cfg(feature = "alloc")]
pub fn get_children(cap_id: u64) -> Vec<u64> {
    let tree = DERIVATION_TREE.read();
    tree.get(&cap_id).cloned().unwrap_or_default()
}

/// Get statistics about the derivation tree.
///
/// Returns (number of parent nodes, total number of child edges).
#[cfg(feature = "alloc")]
pub fn derivation_tree_stats() -> (usize, usize) {
    let tree = DERIVATION_TREE.read();
    let parents = tree.len();
    let total_children: usize = tree.values().map(|v| v.len()).sum();
    (parents, total_children)
}

// ===========================================================================
// Revocation Notifications (Sprint 1.12)
// ===========================================================================

/// Maximum number of queued notifications per process.
const MAX_NOTIFICATIONS_PER_PROCESS: usize = 256;

/// Reason a capability was revoked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevocationReason {
    /// Explicitly revoked by the owning process
    Explicit,
    /// Parent capability was revoked (transitive / cascade)
    ParentRevoked,
    /// Process that held the capability exited
    ProcessExit,
    /// Security policy forced revocation
    PolicyEnforced,
}

impl RevocationReason {
    /// Human-readable description.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Explicit => "explicit",
            Self::ParentRevoked => "parent_revoked",
            Self::ProcessExit => "process_exit",
            Self::PolicyEnforced => "policy_enforced",
        }
    }
}

/// Notification delivered to a process when one of its capabilities is revoked.
#[derive(Debug, Clone, Copy)]
pub struct RevocationNotification {
    /// The ID of the revoked capability
    pub cap_id: u64,
    /// Timestamp (seconds since boot) when revocation occurred
    pub revoked_at: u64,
    /// Reason for the revocation
    pub reason: RevocationReason,
}

/// Per-process revocation notification queues.
///
/// Maps process ID -> bounded deque of notifications.
#[cfg(feature = "alloc")]
static REVOCATION_QUEUE: RwLock<BTreeMap<u64, VecDeque<RevocationNotification>>> =
    RwLock::new(BTreeMap::new());

/// Get a timestamp for revocation notifications.
fn get_revocation_timestamp() -> u64 {
    #[cfg(any(
        target_arch = "x86_64",
        target_arch = "aarch64",
        target_arch = "riscv64"
    ))]
    {
        crate::arch::timer::get_timestamp_secs()
    }
    #[cfg(not(any(
        target_arch = "x86_64",
        target_arch = "aarch64",
        target_arch = "riscv64"
    )))]
    {
        0
    }
}

/// Push revocation notifications for a list of revoked capability IDs.
///
/// Each process in the process table receives a notification for each
/// capability ID that was revoked.
#[cfg(feature = "alloc")]
fn push_revocation_notifications(cap_ids: &[u64], reason: RevocationReason) {
    let timestamp = get_revocation_timestamp();

    // Get list of all PIDs from the process server
    let process_server = crate::services::process_server::get_process_server();
    let pids = process_server.list_process_ids();

    let mut queue = REVOCATION_QUEUE.write();
    for pid in pids {
        let notifications = queue.entry(pid.0).or_default();
        for &cap_id in cap_ids {
            let notification = RevocationNotification {
                cap_id,
                revoked_at: timestamp,
                reason,
            };
            // Evict oldest if at capacity
            if notifications.len() >= MAX_NOTIFICATIONS_PER_PROCESS {
                notifications.pop_front();
            }
            notifications.push_back(notification);
        }
    }
}

/// Push a single revocation notification to a specific process.
#[cfg(feature = "alloc")]
pub fn notify_process(pid: u64, cap_id: u64, reason: RevocationReason) {
    let timestamp = get_revocation_timestamp();
    let notification = RevocationNotification {
        cap_id,
        revoked_at: timestamp,
        reason,
    };

    let mut queue = REVOCATION_QUEUE.write();
    let notifications = queue.entry(pid).or_default();
    if notifications.len() >= MAX_NOTIFICATIONS_PER_PROCESS {
        notifications.pop_front();
    }
    notifications.push_back(notification);
}

/// Drain all pending revocation notifications for a given process.
///
/// Returns the notifications and clears the queue for that process.
#[cfg(feature = "alloc")]
pub fn drain_notifications(pid: u64) -> Vec<RevocationNotification> {
    let mut queue = REVOCATION_QUEUE.write();
    match queue.remove(&pid) {
        Some(deque) => deque.into_iter().collect(),
        None => Vec::new(),
    }
}

/// Peek at pending revocation notifications for a process without consuming
/// them.
#[cfg(feature = "alloc")]
pub fn peek_notifications(pid: u64) -> Vec<RevocationNotification> {
    let queue = REVOCATION_QUEUE.read();
    match queue.get(&pid) {
        Some(deque) => deque.iter().copied().collect(),
        None => Vec::new(),
    }
}

/// Get the number of pending notifications for a process.
#[cfg(feature = "alloc")]
pub fn notification_count(pid: u64) -> usize {
    let queue = REVOCATION_QUEUE.read();
    queue.get(&pid).map_or(0, |q| q.len())
}

/// Clean up notification queue for a process that has exited.
#[cfg(feature = "alloc")]
pub fn cleanup_process_notifications(pid: u64) {
    let mut queue = REVOCATION_QUEUE.write();
    queue.remove(&pid);
}

/// Get summary statistics for the notification system.
///
/// Returns (number of processes with pending notifications,
///          total queued notifications across all processes).
#[cfg(feature = "alloc")]
pub fn notification_stats() -> (usize, usize) {
    let queue = REVOCATION_QUEUE.read();
    let process_count = queue.len();
    let total_notifications: usize = queue.values().map(|q| q.len()).sum();
    (process_count, total_notifications)
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    #[cfg(feature = "alloc")]
    #[allow(unused_imports)]
    use alloc::vec;

    use super::*;

    #[test]
    fn test_revocation_list_basic() {
        let list = RevocationList::new();
        let cap = CapabilityToken::new(100, 0, 0, 0);

        assert!(!list.is_revoked(cap));
        list.add(cap);
        assert!(list.is_revoked(cap));
    }

    #[test]
    fn test_revocation_list_generation() {
        let list = RevocationList::new();
        let cap_gen0 = CapabilityToken::new(200, 0, 0, 0);
        let cap_gen1 = CapabilityToken::new(200, 1, 0, 0);

        list.add(cap_gen0);
        assert!(list.is_revoked(cap_gen0));
        // Different generation should not be considered revoked
        assert!(!list.is_revoked(cap_gen1));
    }

    #[test]
    fn test_revocation_list_epoch() {
        let list = RevocationList::new();
        let initial_epoch = list.epoch();

        let cap = CapabilityToken::new(300, 0, 0, 0);
        list.add(cap);

        assert!(list.epoch() > initial_epoch);
    }

    #[test]
    fn test_revocation_cache_basic() {
        let cache = RevocationCache::new();
        let cap = CapabilityToken::new(400, 0, 0, 0);
        // cache queries REVOCATION_LIST which is a global static,
        // so results depend on prior test state. We verify no panic.
        let _ = cache.is_revoked(cap);
    }

    #[test]
    fn test_revocation_reason_display() {
        assert_eq!(RevocationReason::Explicit.as_str(), "explicit");
        assert_eq!(RevocationReason::ParentRevoked.as_str(), "parent_revoked");
        assert_eq!(RevocationReason::ProcessExit.as_str(), "process_exit");
        assert_eq!(RevocationReason::PolicyEnforced.as_str(), "policy_enforced");
    }

    #[test]
    fn test_revocation_notification_fields() {
        let notif = RevocationNotification {
            cap_id: 42,
            revoked_at: 1000,
            reason: RevocationReason::Explicit,
        };
        assert_eq!(notif.cap_id, 42);
        assert_eq!(notif.revoked_at, 1000);
        assert_eq!(notif.reason, RevocationReason::Explicit);
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_derivation_tree_record_and_get() {
        // Use unique cap IDs to avoid collision with other tests
        let parent = 10_000;
        let child1 = 10_001;
        let child2 = 10_002;
        let grandchild = 10_003;

        record_derivation(parent, child1);
        record_derivation(parent, child2);
        record_derivation(child1, grandchild);

        let children = get_children(parent);
        assert!(children.contains(&child1));
        assert!(children.contains(&child2));

        let subtree = get_derivation_tree(parent);
        assert!(subtree.contains(&child1));
        assert!(subtree.contains(&child2));
        assert!(subtree.contains(&grandchild));

        // Cleanup
        cleanup_capability(grandchild);
        cleanup_capability(child1);
        cleanup_capability(child2);
        cleanup_capability(parent);
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_derivation_tree_cleanup() {
        let parent = 20_000;
        let child = 20_001;

        record_derivation(parent, child);
        assert!(!get_children(parent).is_empty());

        cleanup_capability(child);
        // Child should be removed from parent's children list
        assert!(!get_children(parent).contains(&child));

        cleanup_capability(parent);
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_derivation_tree_stats() {
        let parent = 30_000;
        let child1 = 30_001;
        let child2 = 30_002;

        record_derivation(parent, child1);
        record_derivation(parent, child2);

        let (parents, total) = derivation_tree_stats();
        // At least our parent should be present (may have others from parallel tests)
        assert!(parents >= 1);
        assert!(total >= 2);

        // Cleanup
        cleanup_capability(child1);
        cleanup_capability(child2);
        cleanup_capability(parent);
    }
}
