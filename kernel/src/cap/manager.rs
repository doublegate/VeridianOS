//! Global capability manager
//!
//! Manages capability creation, delegation, and revocation across the system.

use core::sync::atomic::{AtomicU64, AtomicU8, Ordering};

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
}

/// ID allocator for capability IDs
struct IdAllocator {
    next_id: AtomicU64,
    #[cfg(feature = "alloc")]
    recycled: RwLock<BTreeSet<u64>>,
}

impl IdAllocator {
    fn new() -> Self {
        Self {
            next_id: AtomicU64::new(1),
            #[cfg(feature = "alloc")]
            recycled: RwLock::new(BTreeSet::new()),
        }
    }

    fn allocate(&self) -> Result<u64, CapError> {
        // Try to reuse a recycled ID first
        #[cfg(feature = "alloc")]
        {
            let mut recycled = self.recycled.write();
            if let Some(&id) = recycled.iter().next() {
                recycled.remove(&id);
                return Ok(id);
            }
        }

        // Allocate new ID
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        if id > 0xFFFF_FFFF_FFFF {
            // We've exhausted 48-bit IDs
            return Err(CapError::OutOfMemory);
        }
        Ok(id)
    }

    #[cfg(feature = "alloc")]
    fn recycle(&self, id: u64) {
        self.recycled.write().insert(id);
    }
}

/// Registry entry for a capability
struct RegistryEntry {
    object: ObjectRef,
    generation: AtomicU8,
    revoked: bool,
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
            id_allocator: IdAllocator {
                next_id: AtomicU64::new(1),
                #[cfg(feature = "alloc")]
                recycled: RwLock::new(BTreeSet::new()),
            },
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

        // Get object reference
        #[cfg(feature = "alloc")]
        let object = {
            let registry = self.registry.read();
            let entry = registry.get(&cap.id()).ok_or(CapError::InvalidCapability)?;

            if entry.revoked {
                return Err(CapError::CapabilityRevoked);
            }

            entry.object.clone()
        };

        #[cfg(not(feature = "alloc"))]
        return Err(CapError::OutOfMemory);

        // Ensure new rights are subset of source rights
        let derived_rights = source_rights.intersection(new_rights);

        // Create new capability with same ID but potentially different rights
        let new_cap = CapabilityToken::new(
            cap.id(),
            cap.generation(),
            cap.cap_type(),
            derived_rights.to_flags(),
        );

        // Insert into target space
        target
            .insert(new_cap, object, derived_rights)
            .map_err(|_| CapError::OutOfMemory)?;

        self.stats
            .capabilities_delegated
            .fetch_add(1, Ordering::Relaxed);

        Ok(new_cap)
    }

    /// Revoke a capability globally
    pub fn revoke(&self, cap: CapabilityToken) -> Result<(), CapError> {
        #[cfg(feature = "alloc")]
        {
            let mut registry = self.registry.write();
            let entry = registry
                .get_mut(&cap.id())
                .ok_or(CapError::InvalidCapability)?;

            if entry.revoked {
                return Ok(()); // Already revoked
            }

            entry.revoked = true;
            entry.generation.fetch_add(1, Ordering::SeqCst);
        }

        self.stats
            .capabilities_revoked
            .fetch_add(1, Ordering::Relaxed);

        // TODO: Notify all capability spaces of revocation

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
pub fn cap_manager() -> &'static CapabilityManager {
    &CAP_MANAGER
}

/// Fast inline capability check
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
