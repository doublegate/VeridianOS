//! Global capability manager
//!
//! Manages capability creation, delegation, and revocation across the system.

use core::sync::atomic::{AtomicU32, AtomicU64, AtomicU8, AtomicUsize, Ordering};

use super::{
    object::ObjectRef,
    space::CapabilitySpace,
    token::{CapabilityToken, Rights},
};

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::collections::{BTreeMap, BTreeSet};

use spin::RwLock;

/// Error types for capability operations
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapError {
    InvalidCapability,
    InsufficientRights,
    CapabilityRevoked,
    OutOfMemory,
    InvalidObject,
    PermissionDenied,
    AlreadyExists,
    NotFound,
    IdExhausted,
    QuotaExceeded,
}

/// ID allocator for capability IDs (CAP-PERF-02).
///
/// Fresh IDs come from an atomic counter. Recycled IDs sit in a set whose
/// size is mirrored in `recycled_len`, so the common case (nothing to reuse)
/// never takes the lock; it used to take the set's write lock on every
/// allocation.
struct IdAllocator {
    next_id: AtomicU64,
    #[cfg(feature = "alloc")]
    recycled: RwLock<BTreeSet<u64>>,
    #[cfg(feature = "alloc")]
    recycled_len: AtomicUsize,
}

/// IDs occupy the low 48 bits of a capability token.
const MAX_CAP_ID: u64 = (1 << 48) - 1;

impl IdAllocator {
    const fn new() -> Self {
        Self {
            next_id: AtomicU64::new(1),
            #[cfg(feature = "alloc")]
            recycled: RwLock::new(BTreeSet::new()),
            #[cfg(feature = "alloc")]
            recycled_len: AtomicUsize::new(0),
        }
    }

    fn allocate(&self) -> Result<u64, CapError> {
        #[cfg(feature = "alloc")]
        if self.recycled_len.load(Ordering::Acquire) > 0 {
            let mut recycled = self.recycled.write();
            if let Some(id) = recycled.pop_first() {
                self.recycled_len.store(recycled.len(), Ordering::Release);
                return Ok(id);
            }
        }

        self.next_id
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| {
                (id <= MAX_CAP_ID).then_some(id + 1)
            })
            .map_err(|_| CapError::IdExhausted)
    }

    #[cfg(feature = "alloc")]
    fn recycle(&self, id: u64) {
        let mut recycled = self.recycled.write();
        recycled.insert(id);
        self.recycled_len.store(recycled.len(), Ordering::Release);
    }
}

/// Most capabilities that may be delegated from one capability. Revoked
/// delegations still count until the parent is deleted.
#[cfg(feature = "alloc")]
const MAX_DELEGATIONS: usize = 1024;

/// Registry entry for a capability
struct RegistryEntry {
    object: ObjectRef,
    generation: AtomicU8,
    revoked: bool,
    /// Capability spaces holding this capability (see `hold`).
    holders: AtomicU32,
}

/// Global capability manager
pub struct CapabilityManager {
    /// Global capability registry
    #[cfg(feature = "alloc")]
    registry: RwLock<BTreeMap<u64, RegistryEntry>>,

    /// ID allocator
    id_allocator: IdAllocator,

    /// Global generation counter
    global_generation: AtomicU8,

    /// Statistics
    stats: CapManagerStats,
}

/// Statistics for capability manager
pub struct CapManagerStats {
    pub capabilities_created: AtomicU64,
    pub capabilities_delegated: AtomicU64,
    pub capabilities_revoked: AtomicU64,
    pub capabilities_deleted: AtomicU64,
}

impl Default for CapManagerStats {
    fn default() -> Self {
        Self {
            capabilities_created: AtomicU64::new(0),
            capabilities_delegated: AtomicU64::new(0),
            capabilities_revoked: AtomicU64::new(0),
            capabilities_deleted: AtomicU64::new(0),
        }
    }
}

/// Global capability manager instance
static CAP_MANAGER: CapabilityManager = CapabilityManager::new();

impl CapabilityManager {
    const fn new() -> Self {
        Self {
            #[cfg(feature = "alloc")]
            registry: RwLock::new(BTreeMap::new()),
            id_allocator: IdAllocator::new(),
            global_generation: AtomicU8::new(0),
            stats: CapManagerStats {
                capabilities_created: AtomicU64::new(0),
                capabilities_delegated: AtomicU64::new(0),
                capabilities_revoked: AtomicU64::new(0),
                capabilities_deleted: AtomicU64::new(0),
            },
        }
    }

    /// Create a new capability
    pub fn create_capability(
        &self,
        object: ObjectRef,
        rights: Rights,
        cap_space: &CapabilitySpace,
    ) -> Result<CapabilityToken, CapError> {
        // Validate object
        if !object.is_valid() {
            return Err(CapError::InvalidObject);
        }

        // Allocate ID
        let id = self.id_allocator.allocate()?;

        // Create capability token
        let cap = CapabilityToken::new(
            id,
            self.global_generation.load(Ordering::Relaxed),
            object.type_code(),
            rights.to_flags(),
        );

        // Register globally
        #[cfg(feature = "alloc")]
        {
            let entry = RegistryEntry {
                object: object.clone(),
                generation: AtomicU8::new(0),
                revoked: false,
                holders: AtomicU32::new(0),
            };
            self.registry.write().insert(id, entry);
        }

        // Insert into capability space
        cap_space
            .insert(cap, object, rights)
            .map_err(|_| CapError::OutOfMemory)?;

        self.stats
            .capabilities_created
            .fetch_add(1, Ordering::Relaxed);

        // Audit log: capability creation
        crate::security::audit::log_capability_op(0, id, 0);

        Ok(cap)
    }

    /// Delegate capability to another capability space
    pub fn delegate(
        &self,
        cap: CapabilityToken,
        source: &CapabilitySpace,
        target: &CapabilitySpace,
        new_rights: Rights,
    ) -> Result<CapabilityToken, CapError> {
        // Verify source has the capability
        let source_rights = source.lookup(cap).ok_or(CapError::InvalidCapability)?;

        // Check grant permission
        if !source_rights.contains(Rights::GRANT) {
            return Err(CapError::PermissionDenied);
        }

        #[cfg(not(feature = "alloc"))]
        return Err(CapError::OutOfMemory);

        // Ensure new rights are subset of source rights
        let derived_rights = source_rights.intersection(new_rights);

        // A target that already holds a live delegation of this capability
        // with these rights gets that one back, and a capability has at
        // most MAX_DELEGATIONS children, so repeated transfers cannot grow
        // the registry without bound.
        #[cfg(feature = "alloc")]
        {
            let children = super::revocation::get_children(cap.id());
            for &child in &children {
                let generation = match self.registry.read().get(&child) {
                    Some(entry) if !entry.revoked => entry.generation.load(Ordering::Acquire),
                    _ => continue,
                };
                let token = CapabilityToken::new(
                    child,
                    generation,
                    cap.cap_type(),
                    derived_rights.to_flags(),
                );
                if target.lookup(token) == Some(derived_rights) {
                    return Ok(token);
                }
            }
            if children.len() >= MAX_DELEGATIONS
                && self.collect_unheld_delegations(cap.id()) >= MAX_DELEGATIONS
            {
                return Err(CapError::QuotaExceeded);
            }
        }

        // A delegation is a capability of its own, recorded under its
        // parent. It used to reuse the parent's ID, so revoking any one
        // holder's copy revoked them all, and the derivation tree that
        // cascading revocation walks was never filled (CAP-INC-02).
        //
        // The parent check, the registration and the derivation record are
        // one critical section under the registry lock, and `revoke` reads
        // the subtree under the same lock, so a delegation either completes
        // before a revocation (and is revoked with its parent) or sees the
        // parent revoked and fails. Lock order: registry, then tree.
        #[cfg(feature = "alloc")]
        let (new_cap, object) = {
            let id = self.id_allocator.allocate()?;
            let mut registry = self.registry.write();
            let parent = match registry.get(&cap.id()) {
                Some(entry)
                    if !entry.revoked
                        && entry.generation.load(Ordering::Acquire) == cap.generation() =>
                {
                    entry
                }
                Some(_) => {
                    drop(registry);
                    self.id_allocator.recycle(id);
                    return Err(CapError::CapabilityRevoked);
                }
                None => {
                    drop(registry);
                    self.id_allocator.recycle(id);
                    return Err(CapError::InvalidCapability);
                }
            };
            let object = parent.object.clone();
            registry.insert(
                id,
                RegistryEntry {
                    object: object.clone(),
                    generation: AtomicU8::new(0),
                    revoked: false,
                    // In flight: held by this call until the target space
                    // holds it, so a concurrent collection cannot take it.
                    holders: AtomicU32::new(1),
                },
            );
            super::revocation::record_derivation(cap.id(), id);
            (
                CapabilityToken::new(id, 0, cap.cap_type(), derived_rights.to_flags()),
                object,
            )
        };

        // Outside the registry lock (the space has its own locks). A
        // revocation that lands now finds the child in the tree and revokes
        // it, so the token inserted here is already invalid.
        #[cfg(feature = "alloc")]
        if target.insert(new_cap, object, derived_rights).is_err() {
            super::revocation::cleanup_capability(new_cap.id());
            self.registry.write().remove(&new_cap.id());
            self.id_allocator.recycle(new_cap.id());
            return Err(CapError::OutOfMemory);
        }
        // The target holds it now; drop the in-flight reference.
        #[cfg(feature = "alloc")]
        self.release(new_cap.id());

        self.stats
            .capabilities_delegated
            .fetch_add(1, Ordering::Relaxed);

        // Audit log: capability delegation
        crate::security::audit::log_capability_op(0, cap.id(), 0);

        Ok(new_cap)
    }

    /// A capability space now holds capability `id`. Called by
    /// `CapabilitySpace` for every entry it gains (insert, fork copy);
    /// tokens the registry does not know are ignored.
    pub(crate) fn hold(&self, id: u64) {
        #[cfg(feature = "alloc")]
        if let Some(entry) = self.registry.read().get(&id) {
            entry.holders.fetch_add(1, Ordering::AcqRel);
        }
        #[cfg(not(feature = "alloc"))]
        let _ = id;
    }

    /// A capability space no longer holds capability `id` (remove, clear,
    /// drop).
    pub(crate) fn release(&self, id: u64) {
        #[cfg(feature = "alloc")]
        if let Some(entry) = self.registry.read().get(&id) {
            let _ = entry
                .holders
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |h| {
                    Some(h.saturating_sub(1))
                });
        }
        #[cfg(not(feature = "alloc"))]
        let _ = id;
    }

    /// Forget delegations of `parent` that no space holds any more and that
    /// have no delegations of their own (a delegation with children stays,
    /// so revoking `parent` still reaches them). Returns how many
    /// delegations `parent` has left. This is what keeps MAX_DELEGATIONS a
    /// limit on delegations in use: without it, those held by processes
    /// that have exited would use it up for good. IDs are not recycled, so
    /// no new capability can match a revocation-list entry of an old one.
    #[cfg(feature = "alloc")]
    fn collect_unheld_delegations(&self, parent: u64) -> usize {
        let mut registry = self.registry.write();
        let children = super::revocation::get_children(parent);
        let mut left = children.len();
        for child in children {
            let unheld = registry
                .get(&child)
                .is_none_or(|e| e.holders.load(Ordering::Acquire) == 0);
            if unheld && super::revocation::get_children(child).is_empty() {
                registry.remove(&child);
                super::revocation::cleanup_capability(child);
                left -= 1;
            }
        }
        left
    }

    /// Revoke a capability globally, together with every capability
    /// delegated from it, directly or transitively (CAP-INC-02).
    pub fn revoke(&self, cap: CapabilityToken) -> Result<(), CapError> {
        #[cfg(feature = "alloc")]
        {
            let mut revoked = alloc::vec::Vec::new();
            {
                // The subtree is read under the registry lock (order:
                // registry, then tree), so it includes every delegation
                // registered before this point and none can be added under
                // a revoked parent afterwards (see `delegate`).
                let mut registry = self.registry.write();
                let descendants = super::revocation::get_derivation_tree(cap.id());
                let root = registry
                    .get_mut(&cap.id())
                    .ok_or(CapError::InvalidCapability)?;
                if root.revoked {
                    return Ok(()); // Already revoked (and so is its subtree)
                }
                for id in core::iter::once(cap.id()).chain(descendants) {
                    if let Some(entry) = registry.get_mut(&id) {
                        if !entry.revoked {
                            entry.revoked = true;
                            // The generation the holders' tokens carry.
                            let generation = entry.generation.fetch_add(1, Ordering::SeqCst);
                            revoked.push((id, generation));
                        }
                    }
                }
            }

            self.stats
                .capabilities_revoked
                .fetch_add(revoked.len() as u64, Ordering::Relaxed);
            for (i, &(id, generation)) in revoked.iter().enumerate() {
                crate::security::audit::log_capability_op(0, id, 0);
                super::revocation::record_revoked(id, generation, i > 0);
            }
        }

        Ok(())
    }

    /// Delete a capability completely
    pub fn delete(&self, cap: CapabilityToken) -> Result<(), CapError> {
        #[cfg(feature = "alloc")]
        {
            self.registry
                .write()
                .remove(&cap.id())
                .ok_or(CapError::NotFound)?;

            // Recycle the ID
            self.id_allocator.recycle(cap.id());
            super::revocation::cleanup_capability(cap.id());
        }

        self.stats
            .capabilities_deleted
            .fetch_add(1, Ordering::Relaxed);

        Ok(())
    }

    /// Check if a capability is valid (not revoked)
    pub fn is_valid(&self, cap: CapabilityToken) -> bool {
        #[cfg(feature = "alloc")]
        {
            let registry = self.registry.read();
            if let Some(entry) = registry.get(&cap.id()) {
                !entry.revoked && entry.generation.load(Ordering::Relaxed) == cap.generation()
            } else {
                false
            }
        }

        #[cfg(not(feature = "alloc"))]
        true // Without alloc, we can't track revocation
    }

    /// Get statistics
    pub fn stats(&self) -> &CapManagerStats {
        &self.stats
    }
}

/// Get the global capability manager
impl CapabilityManager {
    /// Allocate a capability id from the one system-wide allocator. Every
    /// path that mints a capability id must come here: separate counters
    /// all started at 1 and handed out colliding ids (N-05).
    pub(crate) fn allocate_id(&self) -> Result<u64, CapError> {
        self.id_allocator.allocate()
    }
}

pub fn cap_manager() -> &'static CapabilityManager {
    &CAP_MANAGER
}

/// Fast inline capability check.
///
/// NOTE: There is a theoretical TOCTOU window between the rights check and
/// the revocation check -- a capability could be revoked between the two
/// lookups. In practice this window is extremely narrow: the capability
/// space is RwLock-protected and revocation is a rare administrative
/// operation. A single atomic check would eliminate this entirely but
/// requires restructuring the capability space lookup. Documented as a
/// known limitation.
#[inline(always)]
pub fn check_capability(
    cap: CapabilityToken,
    required_rights: Rights,
    cap_space: &CapabilitySpace,
) -> Result<(), CapError> {
    // Check if capability exists and has required rights
    if !cap_space.check_rights(cap, required_rights) {
        return Err(CapError::InsufficientRights);
    }

    // Check if not revoked
    if !cap_manager().is_valid(cap) {
        return Err(CapError::CapabilityRevoked);
    }

    Ok(())
}

/// Capability check macro for system calls
#[macro_export]
macro_rules! require_capability {
    ($cap:expr, $rights:expr, $cap_space:expr) => {
        $crate::cap::manager::check_capability($cap, $rights, $cap_space)?
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_allocator_reuses_recycled_ids_then_counts_on() {
        let ids = IdAllocator::new();
        assert_eq!(ids.allocate(), Ok(1));
        assert_eq!(ids.allocate(), Ok(2));
        ids.recycle(1);
        assert_eq!(ids.recycled_len.load(Ordering::Relaxed), 1);
        assert_eq!(ids.allocate(), Ok(1));
        assert_eq!(ids.recycled_len.load(Ordering::Relaxed), 0);
        assert_eq!(ids.allocate(), Ok(3));
    }

    #[test]
    fn id_allocator_stops_at_48_bits() {
        let ids = IdAllocator::new();
        ids.next_id.store(MAX_CAP_ID, Ordering::Relaxed);
        assert_eq!(ids.allocate(), Ok(MAX_CAP_ID));
        assert_eq!(ids.allocate(), Err(CapError::IdExhausted));
    }
}
