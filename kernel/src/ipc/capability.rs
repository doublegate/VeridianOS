//! IPC capability management
//! 
//! Capabilities are unforgeable tokens that grant specific permissions
//! for IPC operations. They are the foundation of VeridianOS security model.

use core::sync::atomic::{AtomicU64, Ordering};

/// Global capability ID generator
static CAPABILITY_COUNTER: AtomicU64 = AtomicU64::new(1);

/// Process ID type
pub type ProcessId = u64;

/// Endpoint ID type
pub type EndpointId = u64;

/// IPC capability structure
/// 
/// 64-bit capability token format:
/// - Bits 63-48: Generation counter (for revocation)
/// - Bits 47-32: Capability type
/// - Bits 31-0: Unique ID
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IpcCapability {
    /// Packed capability value
    value: u64,
    /// Target process or endpoint
    target: EndpointId,
    /// Permissions for this capability
    permissions: IpcPermissions,
    /// Usage limits and restrictions
    limits: IpcLimits,
}

impl IpcCapability {
    /// Create a new capability
    pub fn new(target: EndpointId, permissions: IpcPermissions) -> Self {
        let id = CAPABILITY_COUNTER.fetch_add(1, Ordering::Relaxed);
        let generation = 0u64; // Start with generation 0
        let cap_type = CapabilityType::Endpoint as u64;
        
        let value = (generation << 48) | (cap_type << 32) | (id & 0xFFFFFFFF);
        
        Self {
            value,
            target,
            permissions,
            limits: IpcLimits::default(),
        }
    }

    /// Get the capability ID
    pub fn id(&self) -> u64 {
        self.value & 0xFFFFFFFF
    }

    /// Get the generation counter
    pub fn generation(&self) -> u16 {
        ((self.value >> 48) & 0xFFFF) as u16
    }

    /// Get the capability type
    pub fn cap_type(&self) -> CapabilityType {
        let type_val = ((self.value >> 32) & 0xFFFF) as u16;
        CapabilityType::from_u16(type_val).unwrap_or(CapabilityType::Invalid)
    }

    /// Get the target endpoint/process
    pub fn target(&self) -> EndpointId {
        self.target
    }

    /// Check if capability has specific permission
    pub fn has_permission(&self, perm: Permission) -> bool {
        match perm {
            Permission::Send => self.permissions.can_send,
            Permission::Receive => self.permissions.can_receive,
            Permission::Share => self.permissions.can_share,
        }
    }

    /// Derive a new capability with reduced permissions
    pub fn derive(&self, new_perms: IpcPermissions) -> Option<Self> {
        // Can only reduce permissions, not increase them
        if new_perms.can_send && !self.permissions.can_send {
            return None;
        }
        if new_perms.can_receive && !self.permissions.can_receive {
            return None;
        }
        if new_perms.can_share && !self.permissions.can_share {
            return None;
        }
        if new_perms.max_message_size > self.permissions.max_message_size {
            return None;
        }

        let mut derived = *self;
        derived.permissions = new_perms;
        Some(derived)
    }

    /// Revoke this capability by incrementing generation
    pub fn revoke(&mut self) {
        let generation = self.generation().wrapping_add(1);
        let cap_type = self.cap_type() as u64;
        let id = self.id();
        self.value = ((generation as u64) << 48) | (cap_type << 32) | id;
    }
}

/// Capability types
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityType {
    /// Invalid capability
    Invalid = 0,
    /// Endpoint capability for message passing
    Endpoint = 1,
    /// Memory region capability
    Memory = 2,
    /// Process capability
    Process = 3,
    /// Interrupt capability
    Interrupt = 4,
    /// Channel capability for async IPC
    Channel = 5,
}

impl CapabilityType {
    fn from_u16(val: u16) -> Option<Self> {
        match val {
            0 => Some(Self::Invalid),
            1 => Some(Self::Endpoint),
            2 => Some(Self::Memory),
            3 => Some(Self::Process),
            4 => Some(Self::Interrupt),
            5 => Some(Self::Channel),
            _ => None,
        }
    }
}

/// Individual permissions
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permission {
    /// Can send messages
    Send,
    /// Can receive messages
    Receive,
    /// Can share capability with others
    Share,
}

/// IPC permissions structure
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IpcPermissions {
    /// Can send messages to target
    pub can_send: bool,
    /// Can receive messages from target
    pub can_receive: bool,
    /// Can share this capability with other processes
    pub can_share: bool,
    /// Maximum message size allowed (0 = unlimited)
    pub max_message_size: usize,
}

impl IpcPermissions {
    /// Create permissions with all rights
    pub const fn all() -> Self {
        Self {
            can_send: true,
            can_receive: true,
            can_share: true,
            max_message_size: 0, // Unlimited
        }
    }

    /// Create send-only permissions
    pub const fn send_only() -> Self {
        Self {
            can_send: true,
            can_receive: false,
            can_share: false,
            max_message_size: 0,
        }
    }

    /// Create receive-only permissions
    pub const fn receive_only() -> Self {
        Self {
            can_send: false,
            can_receive: true,
            can_share: false,
            max_message_size: 0,
        }
    }
}

/// IPC usage limits and restrictions
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct IpcLimits {
    /// Maximum messages per second (0 = unlimited)
    pub rate_limit: u32,
    /// Maximum bandwidth in bytes per second (0 = unlimited)
    pub bandwidth_limit: u64,
    /// Expiration time in seconds since epoch (0 = no expiration)
    pub expiration: u64,
}

impl IpcLimits {
    /// Create unlimited limits
    pub const fn unlimited() -> Self {
        Self {
            rate_limit: 0,
            bandwidth_limit: 0,
            expiration: 0,
        }
    }

    /// Create limits with rate limiting
    pub const fn with_rate_limit(messages_per_sec: u32) -> Self {
        Self {
            rate_limit: messages_per_sec,
            bandwidth_limit: 0,
            expiration: 0,
        }
    }
}

/// Capability lookup table for O(1) access
/// 
/// This will be expanded to use perfect hashing in production
pub struct CapabilityTable {
    // Placeholder - will implement proper lookup structure
}

impl CapabilityTable {
    /// Create a new capability table
    pub fn new() -> Self {
        Self {}
    }

    /// Insert a capability into the table
    pub fn insert(&mut self, _cap: IpcCapability) -> Result<(), ()> {
        // TODO: Implement insertion
        Ok(())
    }

    /// Lookup a capability by ID
    pub fn lookup(&self, _id: u64) -> Option<&IpcCapability> {
        // TODO: Implement lookup
        None
    }

    /// Remove a capability from the table
    pub fn remove(&mut self, _id: u64) -> Option<IpcCapability> {
        // TODO: Implement removal
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_capability_creation() {
        let cap = IpcCapability::new(42, IpcPermissions::all());
        assert_eq!(cap.target(), 42);
        assert_eq!(cap.generation(), 0);
        assert_eq!(cap.cap_type(), CapabilityType::Endpoint);
    }

    #[test]
    fn test_capability_permissions() {
        let cap = IpcCapability::new(1, IpcPermissions::send_only());
        assert!(cap.has_permission(Permission::Send));
        assert!(!cap.has_permission(Permission::Receive));
        assert!(!cap.has_permission(Permission::Share));
    }

    #[test]
    fn test_capability_derive() {
        let cap = IpcCapability::new(1, IpcPermissions::all());
        let derived = cap.derive(IpcPermissions::send_only());
        assert!(derived.is_some());
        
        let restricted = IpcCapability::new(1, IpcPermissions::send_only());
        let invalid_derive = restricted.derive(IpcPermissions::all());
        assert!(invalid_derive.is_none());
    }
}