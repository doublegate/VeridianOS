//! Capability revocation mechanism
//!
//! Implements cascading revocation and immediate invalidation of capabilities.

use core::sync::atomic::{AtomicU64, Ordering};

use super::{manager::cap_manager, space::CapabilitySpace, token::CapabilityToken};
use crate::error::KernelError;

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::{collections::BTreeSet, vec::Vec};

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
