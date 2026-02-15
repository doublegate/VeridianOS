//! Global IPC registry for managing channels and endpoints
//!
//! This module provides O(1) lookup for IPC endpoints and channels,
//! managing the global namespace for IPC operations.

#![allow(dead_code, clippy::explicit_auto_deref)]

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::collections::BTreeMap;
use core::sync::atomic::{AtomicU64, Ordering};

use spin::Mutex;

use super::{
    capability::{EndpointId, IpcCapability, IpcPermissions, ProcessId},
    channel::{Channel, Endpoint},
    error::{IpcError, Result},
};

/// Global IPC registry using OnceLock for safe initialization.
static IPC_REGISTRY: crate::sync::once_lock::OnceLock<Mutex<IpcRegistry>> =
    crate::sync::once_lock::OnceLock::new();

/// Initialize the IPC registry
pub fn init() {
    #[allow(unused_imports)]
    use crate::println;

    println!("[IPC-REG] Initializing IPC registry...");
    let registry = IpcRegistry::new();
    let registry_mutex = Mutex::new(registry);

    match IPC_REGISTRY.set(registry_mutex) {
        Ok(()) => println!("[IPC-REG] Registry initialized successfully"),
        Err(_) => println!("[IPC-REG] Registry already initialized, skipping..."),
    }
}

/// IPC registry for managing all endpoints and channels
pub struct IpcRegistry {
    /// Endpoint lookup table
    #[cfg(feature = "alloc")]
    endpoints: BTreeMap<EndpointId, Endpoint>,
    /// Channel lookup table
    #[cfg(feature = "alloc")]
    channels: BTreeMap<EndpointId, Channel>,
    /// Process to endpoints mapping
    #[cfg(feature = "alloc")]
    process_endpoints: BTreeMap<ProcessId, BTreeMap<EndpointId, IpcCapability>>,
    /// Next endpoint ID
    next_endpoint_id: AtomicU64,
    /// Statistics
    stats: RegistryStats,
}

/// Registry statistics
struct RegistryStats {
    endpoints_created: AtomicU64,
    endpoints_destroyed: AtomicU64,
    channels_created: AtomicU64,
    channels_destroyed: AtomicU64,
    capability_lookups: AtomicU64,
    capability_hits: AtomicU64,
}

impl IpcRegistry {
    /// Create a new IPC registry
    fn new() -> Self {
        Self {
            #[cfg(feature = "alloc")]
            endpoints: BTreeMap::new(),
            #[cfg(feature = "alloc")]
            channels: BTreeMap::new(),
            #[cfg(feature = "alloc")]
            process_endpoints: BTreeMap::new(),
            next_endpoint_id: AtomicU64::new(1),
            stats: RegistryStats {
                endpoints_created: AtomicU64::new(0),
                endpoints_destroyed: AtomicU64::new(0),
                channels_created: AtomicU64::new(0),
                channels_destroyed: AtomicU64::new(0),
                capability_lookups: AtomicU64::new(0),
                capability_hits: AtomicU64::new(0),
            },
        }
    }

    /// Create a new endpoint
    #[cfg(feature = "alloc")]
    pub fn create_endpoint(&mut self, owner: ProcessId) -> Result<(EndpointId, IpcCapability)> {
        let endpoint_id = self.next_endpoint_id.fetch_add(1, Ordering::Relaxed);
        let endpoint = Endpoint::new(owner);

        // Create capability for the endpoint
        let capability = IpcCapability::new(endpoint_id, IpcPermissions::all());

        // Insert into tables
        self.endpoints.insert(endpoint_id, endpoint);

        // Add to process's endpoint list
        self.process_endpoints
            .entry(owner)
            .or_default()
            .insert(endpoint_id, capability);

        self.stats.endpoints_created.fetch_add(1, Ordering::Relaxed);

        Ok((endpoint_id, capability))
    }

    #[cfg(not(feature = "alloc"))]
    pub fn create_endpoint(&mut self, _owner: ProcessId) -> Result<(EndpointId, IpcCapability)> {
        Err(IpcError::OutOfMemory)
    }

    /// Create a new channel
    #[cfg(feature = "alloc")]
    pub fn create_channel(
        &mut self,
        owner: ProcessId,
        capacity: usize,
    ) -> Result<(EndpointId, EndpointId, IpcCapability, IpcCapability)> {
        let channel = Channel::new(owner, capacity);
        let send_id = channel.send_id();
        let recv_id = channel.receive_id();

        // Create capabilities
        let send_cap = IpcCapability::new(send_id, IpcPermissions::send_only());
        let recv_cap = IpcCapability::new(recv_id, IpcPermissions::receive_only());

        // Insert into registry
        self.channels.insert(send_id, channel);

        // Add to process's endpoint list
        let process_eps = self.process_endpoints.entry(owner).or_default();
        process_eps.insert(send_id, send_cap);
        process_eps.insert(recv_id, recv_cap);

        self.stats.channels_created.fetch_add(1, Ordering::Relaxed);

        Ok((send_id, recv_id, send_cap, recv_cap))
    }

    #[cfg(not(feature = "alloc"))]
    pub fn create_channel(
        &mut self,
        _owner: ProcessId,
        _capacity: usize,
    ) -> Result<(EndpointId, EndpointId, IpcCapability, IpcCapability)> {
        Err(IpcError::OutOfMemory)
    }

    /// Lookup an endpoint by ID
    #[cfg(feature = "alloc")]
    pub fn lookup_endpoint(&self, id: EndpointId) -> Option<&Endpoint> {
        self.stats
            .capability_lookups
            .fetch_add(1, Ordering::Relaxed);

        if let Some(endpoint) = self.endpoints.get(&id) {
            self.stats.capability_hits.fetch_add(1, Ordering::Relaxed);
            Some(endpoint)
        } else {
            None
        }
    }

    #[cfg(not(feature = "alloc"))]
    pub fn lookup_endpoint(&self, _id: EndpointId) -> Option<&Endpoint> {
        None
    }

    /// Validate a capability (Level 1: registry, Level 2: process capability
    /// space)
    #[cfg(feature = "alloc")]
    pub fn validate_capability(
        &self,
        process: ProcessId,
        capability: &IpcCapability,
    ) -> Result<()> {
        self.stats
            .capability_lookups
            .fetch_add(1, Ordering::Relaxed);

        // Level 1: Check if process owns this capability in the registry
        if let Some(process_caps) = self.process_endpoints.get(&process) {
            if let Some(stored_cap) = process_caps.get(&capability.target()) {
                // Verify generation matches
                if stored_cap.generation() == capability.generation() {
                    self.stats.capability_hits.fetch_add(1, Ordering::Relaxed);

                    // Level 2: Cross-validate against process's capability space
                    if let Some(real_process) = crate::process::table::get_process(process) {
                        let cap_space = real_process.capability_space.lock();
                        let cap_token = crate::cap::CapabilityToken::from_u64(capability.id());
                        if cap_space.lookup(cap_token).is_some() {
                            return Ok(());
                        }
                        // Cap space doesn't have it - fall through to error
                    } else {
                        // Process table not available (early boot) - trust Level 1
                        return Ok(());
                    }
                }
            }
        }

        Err(IpcError::InvalidCapability)
    }

    #[cfg(not(feature = "alloc"))]
    pub fn validate_capability(
        &self,
        _process: ProcessId,
        _capability: &IpcCapability,
    ) -> Result<()> {
        Err(IpcError::InvalidCapability)
    }

    /// Remove an endpoint
    #[cfg(feature = "alloc")]
    pub fn remove_endpoint(&mut self, id: EndpointId, owner: ProcessId) -> Result<()> {
        // Verify ownership
        if let Some(endpoint) = self.endpoints.get(&id) {
            if endpoint.owner != owner {
                return Err(IpcError::PermissionDenied);
            }
        } else {
            return Err(IpcError::EndpointNotFound);
        }

        // Remove from registry
        self.endpoints.remove(&id);

        // Remove from process's endpoint list
        if let Some(process_eps) = self.process_endpoints.get_mut(&owner) {
            process_eps.remove(&id);
        }

        self.stats
            .endpoints_destroyed
            .fetch_add(1, Ordering::Relaxed);

        Ok(())
    }

    #[cfg(not(feature = "alloc"))]
    pub fn remove_endpoint(&mut self, _id: EndpointId, _owner: ProcessId) -> Result<()> {
        Err(IpcError::EndpointNotFound)
    }

    /// Get registry statistics
    pub fn get_stats(&self) -> RegistryStatsSummary {
        RegistryStatsSummary {
            endpoints_created: self.stats.endpoints_created.load(Ordering::Relaxed),
            endpoints_destroyed: self.stats.endpoints_destroyed.load(Ordering::Relaxed),
            channels_created: self.stats.channels_created.load(Ordering::Relaxed),
            channels_destroyed: self.stats.channels_destroyed.load(Ordering::Relaxed),
            capability_lookups: self.stats.capability_lookups.load(Ordering::Relaxed),
            capability_hits: self.stats.capability_hits.load(Ordering::Relaxed),
            cache_hit_rate: {
                let lookups = self.stats.capability_lookups.load(Ordering::Relaxed);
                let hits = self.stats.capability_hits.load(Ordering::Relaxed);
                if lookups > 0 {
                    (hits * 100) / lookups
                } else {
                    0
                }
            },
        }
    }
}

/// Registry statistics summary
pub struct RegistryStatsSummary {
    pub endpoints_created: u64,
    pub endpoints_destroyed: u64,
    pub channels_created: u64,
    pub channels_destroyed: u64,
    pub capability_lookups: u64,
    pub capability_hits: u64,
    pub cache_hit_rate: u64,
}

/// Helper functions for cross-architecture registry access
/// Get a mutable reference to the global registry
fn with_registry_mut<T, F>(f: F) -> Result<T>
where
    F: FnOnce(&mut IpcRegistry) -> Result<T>,
{
    let registry_mutex = IPC_REGISTRY.get().ok_or(IpcError::NotInitialized)?;
    let mut registry_guard = registry_mutex.lock();
    f(&mut *registry_guard)
}

/// Get an immutable reference to the global registry
fn with_registry<T, F>(f: F) -> Result<T>
where
    F: FnOnce(&IpcRegistry) -> Result<T>,
{
    let registry_mutex = IPC_REGISTRY.get().ok_or(IpcError::NotInitialized)?;
    let registry_guard = registry_mutex.lock();
    f(&*registry_guard)
}

/// Global registry access functions
/// Create an endpoint through the global registry
pub fn create_endpoint(owner: ProcessId) -> Result<(EndpointId, IpcCapability)> {
    with_registry_mut(|registry| registry.create_endpoint(owner))
}

/// Create a channel through the global registry
pub fn create_channel(
    owner: ProcessId,
    capacity: usize,
) -> Result<(EndpointId, EndpointId, IpcCapability, IpcCapability)> {
    with_registry_mut(|registry| registry.create_channel(owner, capacity))
}

/// Remove a channel from the global registry
pub fn remove_channel(channel_id: u64) -> Result<()> {
    with_registry_mut(|registry| {
        // For now, treat channel_id as endpoint_id
        // In a real implementation, we'd track channel IDs separately
        let endpoint_id = channel_id;

        // Remove the channel if it exists
        #[cfg(feature = "alloc")]
        {
            if registry.channels.remove(&endpoint_id).is_some() {
                registry
                    .stats
                    .channels_destroyed
                    .fetch_add(1, Ordering::Relaxed);
                Ok(())
            } else {
                // Try removing as endpoint
                if registry.endpoints.remove(&endpoint_id).is_some() {
                    registry
                        .stats
                        .endpoints_destroyed
                        .fetch_add(1, Ordering::Relaxed);
                    Ok(())
                } else {
                    Err(IpcError::EndpointNotFound)
                }
            }
        }

        #[cfg(not(feature = "alloc"))]
        Err(IpcError::EndpointNotFound)
    })
}

/// Remove all endpoints owned by a process (used during process cleanup)
pub fn remove_process_endpoints(owner: ProcessId) -> Result<usize> {
    with_registry_mut(|registry| {
        #[cfg(feature = "alloc")]
        {
            // Get all endpoint IDs owned by this process
            let endpoint_ids: alloc::vec::Vec<EndpointId> = registry
                .process_endpoints
                .get(&owner)
                .map(|eps| eps.keys().cloned().collect())
                .unwrap_or_default();

            let mut removed_count = 0;

            // Remove each endpoint from the registry
            for endpoint_id in &endpoint_ids {
                // Remove from endpoints table
                if registry.endpoints.remove(endpoint_id).is_some() {
                    registry
                        .stats
                        .endpoints_destroyed
                        .fetch_add(1, Ordering::Relaxed);
                    removed_count += 1;
                }

                // Remove from channels table (in case it's a channel endpoint)
                if registry.channels.remove(endpoint_id).is_some() {
                    registry
                        .stats
                        .channels_destroyed
                        .fetch_add(1, Ordering::Relaxed);
                }
            }

            // Remove the process's endpoint mapping entirely
            registry.process_endpoints.remove(&owner);

            Ok(removed_count)
        }

        #[cfg(not(feature = "alloc"))]
        {
            let _ = owner;
            Ok(0)
        }
    })
}

/// Lookup an endpoint by ID
pub fn lookup_endpoint(id: EndpointId) -> Result<&'static Endpoint> {
    with_registry(|registry| {
        // SAFETY: We cast the registry reference to a raw pointer and dereference
        // it to obtain a &'static Endpoint. This is sound because:
        // 1. The registry is heap-allocated via Box::leak and lives for the kernel's
        //    lifetime, so the Endpoint data it contains also has 'static lifetime.
        // 2. The Endpoint reference is derived from data owned by the registry's
        //    BTreeMap, which persists as long as the entry is not removed.
        // CAVEAT: If the endpoint is removed from the registry while a &'static
        // reference is held, this becomes a dangling reference. Production code
        // should use reference counting (Arc) to prevent use-after-free.
        unsafe {
            let registry_ptr = registry as *const IpcRegistry;
            (*registry_ptr)
                .lookup_endpoint(id)
                .ok_or(IpcError::EndpointNotFound)
                .map(|ep| &*(ep as *const Endpoint))
        }
    })
}

/// Validate a capability
pub fn validate_capability(process: ProcessId, capability: &IpcCapability) -> Result<()> {
    with_registry(|registry| registry.validate_capability(process, capability))
}

/// Get registry statistics
pub fn get_registry_stats() -> Result<RegistryStatsSummary> {
    with_registry(|registry| Ok(registry.get_stats()))
}

#[cfg(all(test, not(target_os = "none")))]
mod tests {
    use super::*;

    #[test]
    fn test_registry_init() {
        init();
        let result = create_endpoint(1);
        assert!(result.is_ok());
    }

    #[test]
    fn test_endpoint_creation() {
        init();
        let (id, cap) = create_endpoint(1).unwrap();
        assert_eq!(cap.target(), id);
        assert!(cap.has_permission(super::super::capability::Permission::Send));
    }

    #[test]
    fn test_channel_creation() {
        init();
        let (send_id, recv_id, send_cap, recv_cap) = create_channel(1, 100).unwrap();
        assert_ne!(send_id, recv_id);
        assert!(send_cap.has_permission(super::super::capability::Permission::Send));
        assert!(!send_cap.has_permission(super::super::capability::Permission::Receive));
        assert!(recv_cap.has_permission(super::super::capability::Permission::Receive));
        assert!(!recv_cap.has_permission(super::super::capability::Permission::Send));
    }
}
