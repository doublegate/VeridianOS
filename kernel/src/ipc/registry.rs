//! Global IPC registry for managing channels and endpoints
//!
//! This module provides O(1) lookup for IPC endpoints and channels,
//! managing the global namespace for IPC operations.

#![allow(dead_code)]

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

/// Global IPC registry
#[cfg(not(any(target_arch = "aarch64", target_arch = "riscv64")))]
static IPC_REGISTRY: Mutex<Option<IpcRegistry>> = Mutex::new(None);

/// Global IPC registry for AArch64/RISC-V (avoiding spin::Mutex issues)
#[cfg(target_arch = "riscv64")]
static mut IPC_REGISTRY_STATIC: Option<IpcRegistry> = None;

/// Global IPC registry for AArch64 (using lazy initialization)
#[cfg(target_arch = "aarch64")]
static mut IPC_REGISTRY_AARCH64: Option<IpcRegistry> = None;

/// Flag to indicate AArch64 registry is initialized
#[cfg(target_arch = "aarch64")]
static mut IPC_REGISTRY_INIT: bool = false;

/// Initialize the IPC registry
pub fn init() {
    #[cfg(target_arch = "aarch64")]
    unsafe {
        use crate::arch::aarch64::direct_uart::uart_write_str;
        uart_write_str("[IPC-REG] Marking registry as initialized for AArch64 (lazy init)...\n");
        
        // Just mark as initialized - actual registry created on first use
        IPC_REGISTRY_INIT = true;
        
        uart_write_str("[IPC-REG] Registry marked for lazy initialization on AArch64\n");
    }
    
    #[cfg(target_arch = "riscv64")]
    unsafe {
        IPC_REGISTRY_STATIC = Some(IpcRegistry::new());
    }
    
    #[cfg(all(not(target_arch = "aarch64"), not(target_arch = "riscv64")))]
    {
        let mut registry = IPC_REGISTRY.lock();
        *registry = Some(IpcRegistry::new());
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
        #[cfg(target_arch = "aarch64")]
        unsafe {
            use crate::arch::aarch64::direct_uart::uart_write_str;
            uart_write_str("[IPC-REG] Creating registry structure...\n");
        }
        
        // Create BTreeMaps with explicit feature check
        #[cfg(feature = "alloc")]
        let endpoints = {
            #[cfg(target_arch = "aarch64")]
            unsafe {
                use crate::arch::aarch64::direct_uart::uart_write_str;
                uart_write_str("[IPC-REG] Creating endpoints map...\n");
            }
            BTreeMap::new()
        };
        
        #[cfg(feature = "alloc")]
        let channels = {
            #[cfg(target_arch = "aarch64")]
            unsafe {
                use crate::arch::aarch64::direct_uart::uart_write_str;
                uart_write_str("[IPC-REG] Creating channels map...\n");
            }
            BTreeMap::new()
        };
        
        #[cfg(feature = "alloc")]
        let process_endpoints = {
            #[cfg(target_arch = "aarch64")]
            unsafe {
                use crate::arch::aarch64::direct_uart::uart_write_str;
                uart_write_str("[IPC-REG] Creating process_endpoints map...\n");
            }
            BTreeMap::new()
        };
        
        #[cfg(target_arch = "aarch64")]
        unsafe {
            use crate::arch::aarch64::direct_uart::uart_write_str;
            uart_write_str("[IPC-REG] Creating atomic counters...\n");
        }
        
        let registry = Self {
            #[cfg(feature = "alloc")]
            endpoints,
            #[cfg(feature = "alloc")]
            channels,
            #[cfg(feature = "alloc")]
            process_endpoints,
            next_endpoint_id: AtomicU64::new(1),
            stats: RegistryStats {
                endpoints_created: AtomicU64::new(0),
                endpoints_destroyed: AtomicU64::new(0),
                channels_created: AtomicU64::new(0),
                channels_destroyed: AtomicU64::new(0),
                capability_lookups: AtomicU64::new(0),
                capability_hits: AtomicU64::new(0),
            },
        };
        
        #[cfg(target_arch = "aarch64")]
        unsafe {
            use crate::arch::aarch64::direct_uart::uart_write_str;
            uart_write_str("[IPC-REG] Registry structure created successfully\n");
        }
        
        registry
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

    /// Validate a capability
    #[cfg(feature = "alloc")]
    pub fn validate_capability(
        &self,
        process: ProcessId,
        capability: &IpcCapability,
    ) -> Result<()> {
        self.stats
            .capability_lookups
            .fetch_add(1, Ordering::Relaxed);

        // Check if process owns this capability
        if let Some(process_caps) = self.process_endpoints.get(&process) {
            if let Some(stored_cap) = process_caps.get(&capability.target()) {
                // Verify generation matches
                if stored_cap.generation() == capability.generation() {
                    self.stats.capability_hits.fetch_add(1, Ordering::Relaxed);
                    return Ok(());
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
#[cfg(not(any(target_arch = "aarch64", target_arch = "riscv64")))]
fn with_registry_mut<T, F>(f: F) -> Result<T>
where
    F: FnOnce(&mut IpcRegistry) -> Result<T>,
{
    let mut registry_guard = IPC_REGISTRY.lock();
    let registry = registry_guard.as_mut().ok_or(IpcError::NotInitialized)?;
    f(registry)
}

#[cfg(target_arch = "riscv64")]
fn with_registry_mut<T, F>(f: F) -> Result<T>
where
    F: FnOnce(&mut IpcRegistry) -> Result<T>,
{
    unsafe {
        let registry = IPC_REGISTRY_STATIC.as_mut().ok_or(IpcError::NotInitialized)?;
        f(registry)
    }
}

#[cfg(target_arch = "aarch64")]
fn with_registry_mut<T, F>(f: F) -> Result<T>
where
    F: FnOnce(&mut IpcRegistry) -> Result<T>,
{
    unsafe {
        if !IPC_REGISTRY_INIT {
            return Err(IpcError::NotInitialized);
        }
        
        // Lazy initialize if needed
        if IPC_REGISTRY_AARCH64.is_none() {
            use crate::arch::aarch64::direct_uart::uart_write_str;
            uart_write_str("[IPC-REG] Lazy initializing registry on first use...\n");
            
            // Create registry inline without any intermediate variables
            IPC_REGISTRY_AARCH64 = Some(IpcRegistry {
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
            });
            
            uart_write_str("[IPC-REG] Registry lazy initialized successfully\n");
        }
        
        let registry = IPC_REGISTRY_AARCH64.as_mut().ok_or(IpcError::NotInitialized)?;
        f(registry)
    }
}

/// Get an immutable reference to the global registry
#[cfg(not(any(target_arch = "aarch64", target_arch = "riscv64")))]
fn with_registry<T, F>(f: F) -> Result<T>
where
    F: FnOnce(&IpcRegistry) -> Result<T>,
{
    let registry_guard = IPC_REGISTRY.lock();
    let registry = registry_guard.as_ref().ok_or(IpcError::NotInitialized)?;
    f(registry)
}

#[cfg(target_arch = "riscv64")]
fn with_registry<T, F>(f: F) -> Result<T>
where
    F: FnOnce(&IpcRegistry) -> Result<T>,
{
    unsafe {
        let registry = IPC_REGISTRY_STATIC.as_ref().ok_or(IpcError::NotInitialized)?;
        f(registry)
    }
}

#[cfg(target_arch = "aarch64")]
fn with_registry<T, F>(f: F) -> Result<T>
where
    F: FnOnce(&IpcRegistry) -> Result<T>,
{
    unsafe {
        if !IPC_REGISTRY_INIT {
            return Err(IpcError::NotInitialized);
        }
        
        // Lazy initialize if needed (same as with_registry_mut)
        if IPC_REGISTRY_AARCH64.is_none() {
            use crate::arch::aarch64::direct_uart::uart_write_str;
            uart_write_str("[IPC-REG] Lazy initializing registry on first use (read)...\n");
            
            IPC_REGISTRY_AARCH64 = Some(IpcRegistry {
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
            });
            
            uart_write_str("[IPC-REG] Registry lazy initialized successfully (read)\n");
        }
        
        let registry = IPC_REGISTRY_AARCH64.as_ref().ok_or(IpcError::NotInitialized)?;
        f(registry)
    }
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

/// Lookup an endpoint by ID
pub fn lookup_endpoint(id: EndpointId) -> Result<&'static Endpoint> {
    with_registry(|registry| {
        // SAFETY: We're returning a reference with 'static lifetime, but the registry
        // is a global static, so this is safe as long as endpoints aren't removed
        // while references exist. In production, we'd use Arc or similar.
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
