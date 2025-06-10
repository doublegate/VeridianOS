//! Capability system types and structures

use core::sync::atomic::{AtomicU64, Ordering};

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::{collections::BTreeMap, vec::Vec};

use spin::Mutex;

/// Capability ID - unique identifier for a capability
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CapabilityId(pub u64);

impl core::fmt::Display for CapabilityId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "Cap#{}", self.0)
    }
}

/// Capability types
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityType {
    /// Memory capability - access to physical memory region
    Memory = 0,
    /// IPC endpoint capability - send/receive messages
    IpcEndpoint = 1,
    /// Process capability - control another process
    Process = 2,
    /// Thread capability - control a thread
    Thread = 3,
    /// Device capability - access to hardware device
    Device = 4,
    /// File capability - access to file/directory
    File = 5,
    /// Network capability - network socket access
    Network = 6,
    /// Time capability - access to timers/clocks
    Time = 7,
}

/// Capability permissions
#[derive(Debug, Clone, Copy)]
pub struct CapabilityPermissions(u32);

impl CapabilityPermissions {
    pub const READ: Self = Self(1 << 0);
    pub const WRITE: Self = Self(1 << 1);
    pub const EXECUTE: Self = Self(1 << 2);
    pub const GRANT: Self = Self(1 << 3);
    pub const REVOKE: Self = Self(1 << 4);
    
    pub fn new(perms: u32) -> Self {
        Self(perms)
    }
    
    pub fn has(&self, perm: Self) -> bool {
        (self.0 & perm.0) != 0
    }
}

impl core::ops::BitOr for CapabilityPermissions {
    type Output = Self;
    
    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

/// Capability structure
pub struct Capability {
    /// Unique ID
    pub id: CapabilityId,
    
    /// Capability type
    pub cap_type: CapabilityType,
    
    /// Permissions
    pub permissions: CapabilityPermissions,
    
    /// Resource-specific data
    pub resource_id: u64,
    
    /// Parent capability (for hierarchical caps)
    pub parent: Option<CapabilityId>,
    
    /// Generation counter for revocation
    pub generation: AtomicU64,
}

impl Capability {
    /// Create a new capability
    pub fn new(
        id: CapabilityId,
        cap_type: CapabilityType,
        permissions: CapabilityPermissions,
        resource_id: u64,
    ) -> Self {
        Self {
            id,
            cap_type,
            permissions,
            resource_id,
            parent: None,
            generation: AtomicU64::new(0),
        }
    }
    
    /// Check if capability is valid
    pub fn is_valid(&self) -> bool {
        // In future, check generation against revocation list
        true
    }
    
    /// Revoke capability
    pub fn revoke(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
    }
}

/// Capability space for a process
pub struct CapabilitySpace {
    /// Capabilities owned by this process
    #[cfg(feature = "alloc")]
    capabilities: Mutex<BTreeMap<CapabilityId, Capability>>,
    
    /// Next capability ID
    next_id: AtomicU64,
}

impl CapabilitySpace {
    /// Create a new capability space
    pub fn new() -> Self {
        Self {
            #[cfg(feature = "alloc")]
            capabilities: Mutex::new(BTreeMap::new()),
            next_id: AtomicU64::new(1),
        }
    }
    
    /// Clone from another capability space
    #[cfg(feature = "alloc")]
    pub fn clone_from(&mut self, other: &Self) -> Result<(), &'static str> {
        let other_caps = other.capabilities.lock();
        let mut self_caps = self.capabilities.lock();
        
        // Clear existing capabilities
        self_caps.clear();
        
        // Clone all capabilities
        for (id, cap) in other_caps.iter() {
            self_caps.insert(*id, cap.clone());
        }
        
        // Copy next ID
        self.next_id.store(other.next_id.load(Ordering::Relaxed), Ordering::Relaxed);
        
        Ok(())
    }
    
    /// Destroy the capability space
    #[cfg(feature = "alloc")]
    pub fn destroy(&mut self) {
        self.capabilities.lock().clear();
    }
    
    /// Insert a capability
    #[cfg(feature = "alloc")]
    pub fn insert(&self, cap: Capability) -> Result<(), &'static str> {
        let mut caps = self.capabilities.lock();
        if caps.contains_key(&cap.id) {
            return Err("Capability ID already exists");
        }
        caps.insert(cap.id, cap);
        Ok(())
    }
    
    /// Get a capability by ID
    #[cfg(feature = "alloc")]
    pub fn get(&self, id: CapabilityId) -> Option<Capability> {
        self.capabilities.lock().get(&id).cloned()
    }
    
    /// Remove a capability
    #[cfg(feature = "alloc")]
    pub fn remove(&self, id: CapabilityId) -> Option<Capability> {
        self.capabilities.lock().remove(&id)
    }
    
    /// Generate a new capability ID
    pub fn next_cap_id(&self) -> CapabilityId {
        CapabilityId(self.next_id.fetch_add(1, Ordering::Relaxed))
    }
    
    /// Check if process has capability with permissions
    #[cfg(feature = "alloc")]
    pub fn has_capability(&self, id: CapabilityId, perms: CapabilityPermissions) -> bool {
        if let Some(cap) = self.get(id) {
            cap.is_valid() && cap.permissions.has(perms)
        } else {
            false
        }
    }
}

impl Clone for Capability {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            cap_type: self.cap_type,
            permissions: self.permissions,
            resource_id: self.resource_id,
            parent: self.parent,
            generation: AtomicU64::new(self.generation.load(Ordering::Relaxed)),
        }
    }
}

/// Global capability table ID allocator
static GLOBAL_CAP_ID: AtomicU64 = AtomicU64::new(1);

/// Allocate a globally unique capability ID
pub fn alloc_cap_id() -> CapabilityId {
    CapabilityId(GLOBAL_CAP_ID.fetch_add(1, Ordering::Relaxed))
}