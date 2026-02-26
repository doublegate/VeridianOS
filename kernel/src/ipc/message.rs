//! IPC message format definitions
//!
//! This module defines the message structures used for IPC communication.
//! Small messages (≤64 bytes) are passed via registers for optimal performance,
//! while large messages use shared memory for zero-copy transfers.

// Core IPC message types

use core::mem::size_of;

/// Maximum size for register-based small messages
pub const SMALL_MESSAGE_MAX_SIZE: usize = 64;

/// Number of data registers available for small messages
pub const DATA_REGISTERS: usize = 4;

/// Small message for register-based transfers (≤64 bytes)
///
/// This structure is optimized for fast register-based IPC where the entire
/// message can be passed in CPU registers without memory access.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SmallMessage {
    /// Capability token for the operation
    pub capability: u64,
    /// Message type/operation code
    pub opcode: u32,
    /// Message flags
    pub flags: u32,
    /// Payload (up to 4 registers)
    pub data: [u64; DATA_REGISTERS],
}

impl SmallMessage {
    /// Create a new small message
    pub const fn new(capability: u64, opcode: u32) -> Self {
        Self {
            capability,
            opcode,
            flags: 0,
            data: [0; DATA_REGISTERS],
        }
    }

    /// Set message flags
    pub fn with_flags(mut self, flags: u32) -> Self {
        self.flags = flags;
        self
    }

    /// Set data at specific index
    pub fn with_data(mut self, index: usize, value: u64) -> Self {
        if index < DATA_REGISTERS {
            self.data[index] = value;
        }
        self
    }

    /// Get the total size of the message
    pub const fn size() -> usize {
        size_of::<Self>()
    }
}

/// Message header for large messages
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageHeader {
    /// Capability token
    pub capability: u64,
    /// Operation code
    pub opcode: u32,
    /// Message flags
    pub flags: u32,
    /// Total size of the message including payload
    pub total_size: u64,
    /// Optional checksum for integrity
    pub checksum: u32,
    /// Reserved for alignment
    _reserved: u32,
}

impl MessageHeader {
    /// Create a new message header
    pub const fn new(capability: u64, opcode: u32, total_size: u64) -> Self {
        Self {
            capability,
            opcode,
            flags: 0,
            total_size,
            checksum: 0,
            _reserved: 0,
        }
    }
}

/// Memory region descriptor for shared memory transfers
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryRegion {
    /// Base virtual address
    pub base_addr: u64,
    /// Size of the region in bytes
    pub size: u64,
    /// Memory permissions (read/write/execute)
    pub permissions: u32,
    /// Cache policy (write-back, write-through, uncached)
    pub cache_policy: u32,
}

impl MemoryRegion {
    /// Create a new memory region descriptor
    pub const fn new(base_addr: u64, size: u64) -> Self {
        Self {
            base_addr,
            size,
            permissions: 0,
            cache_policy: 0,
        }
    }

    /// Set permissions for the region
    pub fn with_permissions(mut self, permissions: u32) -> Self {
        self.permissions = permissions;
        self
    }

    /// Set cache policy for the region
    pub fn with_cache_policy(mut self, policy: u32) -> Self {
        self.cache_policy = policy;
        self
    }
}

/// Large message for memory-based transfers
///
/// Used when message size exceeds register capacity or when zero-copy
/// semantics are required for performance.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct LargeMessage {
    /// Message header with metadata
    pub header: MessageHeader,
    /// Shared memory region descriptor
    pub memory_region: MemoryRegion,
    /// Optional inline data for hybrid transfers
    pub inline_data: [u8; SMALL_MESSAGE_MAX_SIZE],
}

impl LargeMessage {
    /// Create a new large message
    pub fn new(capability: u64, opcode: u32, region: MemoryRegion) -> Self {
        let total_size = region.size + size_of::<Self>() as u64;
        Self {
            header: MessageHeader::new(capability, opcode, total_size),
            memory_region: region,
            inline_data: [0; SMALL_MESSAGE_MAX_SIZE],
        }
    }

    /// Set inline data for hybrid transfers
    pub fn with_inline_data(mut self, data: &[u8]) -> Self {
        let len = data.len().min(SMALL_MESSAGE_MAX_SIZE);
        self.inline_data[..len].copy_from_slice(&data[..len]);
        self
    }
}

/// Unified message type that can represent both small and large messages
#[derive(Debug, Clone, Copy)]
pub enum Message {
    /// Small register-based message
    Small(SmallMessage),
    /// Large memory-based message
    Large(LargeMessage),
}

impl Message {
    /// Create a small message
    pub const fn small(capability: u64, opcode: u32) -> Self {
        Message::Small(SmallMessage::new(capability, opcode))
    }

    /// Create a large message
    pub fn large(capability: u64, opcode: u32, region: MemoryRegion) -> Self {
        Message::Large(LargeMessage::new(capability, opcode, region))
    }

    /// Get the capability token from the message
    pub fn capability(&self) -> u64 {
        match self {
            Message::Small(msg) => msg.capability,
            Message::Large(msg) => msg.header.capability,
        }
    }

    /// Get the operation code from the message
    pub fn opcode(&self) -> u32 {
        match self {
            Message::Small(msg) => msg.opcode,
            Message::Large(msg) => msg.header.opcode,
        }
    }

    /// Get message flags
    pub fn flags(&self) -> u32 {
        match self {
            Message::Small(msg) => msg.flags,
            Message::Large(msg) => msg.header.flags,
        }
    }

    /// Set message flags
    pub fn set_flags(&mut self, flags: u32) {
        match self {
            Message::Small(msg) => msg.flags = flags,
            Message::Large(msg) => msg.header.flags = flags,
        }
    }
}

/// Message flags
pub mod flags {
    /// Message requires immediate delivery
    pub const URGENT: u32 = 1 << 0;
    /// Message can be delivered out of order
    pub const UNORDERED: u32 = 1 << 1;
    /// Message requires acknowledgment
    pub const NEEDS_ACK: u32 = 1 << 2;
    /// Message is a reply to a previous message
    pub const IS_REPLY: u32 = 1 << 3;
    /// Message contains capability transfer
    pub const HAS_CAPABILITY: u32 = 1 << 4;
}

/// Memory permissions
pub mod permissions {
    /// Region is readable
    pub const READ: u32 = 1 << 0;
    /// Region is writable
    pub const WRITE: u32 = 1 << 1;
    /// Region is executable
    pub const EXECUTE: u32 = 1 << 2;
    /// Region is shared between processes
    pub const SHARED: u32 = 1 << 3;
}

/// Cache policies
pub mod cache_policy {
    /// Write-back caching (default)
    pub const WRITE_BACK: u32 = 0;
    /// Write-through caching
    pub const WRITE_THROUGH: u32 = 1;
    /// Uncached (device memory)
    pub const UNCACHED: u32 = 2;
    /// Write-combining (for framebuffers)
    pub const WRITE_COMBINING: u32 = 3;
}

#[cfg(all(test, not(target_os = "none")))]
mod tests {
    use super::*;

    #[test]
    fn test_small_message_size() {
        assert_eq!(SmallMessage::size(), 48); // 8 + 4 + 4 + (8 * 4)
    }

    #[test]
    fn test_message_creation() {
        let small = Message::small(0x1234, 42);
        assert_eq!(small.capability(), 0x1234);
        assert_eq!(small.opcode(), 42);

        let region = MemoryRegion::new(0x1000, 4096);
        let large = Message::large(0x5678, 84, region);
        assert_eq!(large.capability(), 0x5678);
        assert_eq!(large.opcode(), 84);
    }
}
