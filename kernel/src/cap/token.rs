//! Capability token implementation
//!
//! Implements the 64-bit capability token format with packed fields
//! for efficient storage and fast validation.

use core::sync::atomic::Ordering;

/// 64-bit capability token with packed fields
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CapabilityToken(u64);

impl CapabilityToken {
    /// Create a new capability token
    pub fn new(id: u64, generation: u8, cap_type: u8, flags: u8) -> Self {
        debug_assert!(id <= 0xFFFF_FFFF_FFFF, "ID exceeds 48 bits");
        debug_assert!(cap_type <= 0xF, "Type exceeds 4 bits");
        debug_assert!(flags <= 0xF, "Flags exceed 4 bits");

        let value = (id & 0xFFFF_FFFF_FFFF)
            | ((generation as u64) << 48)
            | ((cap_type as u64) << 56)
            | ((flags as u64) << 60);

        Self(value)
    }

    /// Get the capability ID (48 bits)
    #[inline]
    pub fn id(&self) -> u64 {
        self.0 & 0xFFFF_FFFF_FFFF
    }

    /// Get the generation counter (8 bits)
    #[inline]
    pub fn generation(&self) -> u8 {
        ((self.0 >> 48) & 0xFF) as u8
    }

    /// Get the capability type (4 bits)
    #[inline]
    pub fn cap_type(&self) -> u8 {
        ((self.0 >> 56) & 0xF) as u8
    }

    /// Get the flags (4 bits)
    #[inline]
    pub fn flags(&self) -> u8 {
        ((self.0 >> 60) & 0xF) as u8
    }

    /// Create a token from ID and generation (for revocation)
    pub fn from_id_and_generation(id: super::types::CapabilityId, generation: u8) -> Self {
        Self::new(id.into(), generation, 0, 0)
    }

    /// Convert to raw u64 value
    #[inline]
    pub fn to_u64(self) -> u64 {
        self.0
    }

    /// Create from raw u64 value
    #[inline]
    pub fn from_u64(value: u64) -> Self {
        Self(value)
    }

    /// Check if this is a null capability
    #[inline]
    pub fn is_null(&self) -> bool {
        self.0 == 0
    }

    /// Create a null capability
    #[inline]
    pub const fn null() -> Self {
        Self(0)
    }

    /// Create capability token from parts
    pub fn from_parts(id: u64, _object_id: u64, generation: u8, _metadata: u8) -> Self {
        // For now, we'll use a simple format
        Self::new(id, generation, 0, 0)
    }
}

/// Capability permissions flags (4 bits)
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityFlags {
    /// Can read from resource
    Read = 0b0001,
    /// Can write to resource
    Write = 0b0010,
    /// Can execute (memory) or invoke (endpoint)
    Execute = 0b0100,
    /// Can delegate to other processes
    Grant = 0b1000,
}

impl CapabilityFlags {
    /// Check if flags contain a specific permission
    #[inline]
    pub fn has(flags: u8, perm: Self) -> bool {
        (flags & perm as u8) != 0
    }

    /// Combine multiple flags
    #[inline]
    pub fn combine(flags: &[Self]) -> u8 {
        flags.iter().fold(0, |acc, f| acc | (*f as u8))
    }
}

/// Rights structure for more detailed permissions
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rights(u32);

impl Rights {
    pub const READ: Self = Self(1 << 0);
    pub const WRITE: Self = Self(1 << 1);
    pub const EXECUTE: Self = Self(1 << 2);
    pub const GRANT: Self = Self(1 << 3);
    pub const REVOKE: Self = Self(1 << 4);
    pub const DELETE: Self = Self(1 << 5);
    pub const MODIFY: Self = Self(1 << 6);
    pub const CREATE: Self = Self(1 << 7);

    /// Create new rights
    #[inline]
    pub fn new(rights: u32) -> Self {
        Self(rights)
    }

    /// Check if rights contain specific permission
    #[inline]
    pub fn contains(&self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }

    /// Get intersection of rights
    #[inline]
    pub fn intersection(&self, other: Self) -> Self {
        Self(self.0 & other.0)
    }

    /// Get union of rights
    #[inline]
    pub fn union(&self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Remove rights
    #[inline]
    pub fn remove(&self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }

    /// Check if empty
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.0 == 0
    }

    /// Convert to capability flags (4-bit)
    #[inline]
    pub fn to_flags(self) -> u8 {
        let mut flags = 0;
        if self.contains(Self::READ) {
            flags |= CapabilityFlags::Read as u8;
        }
        if self.contains(Self::WRITE) {
            flags |= CapabilityFlags::Write as u8;
        }
        if self.contains(Self::EXECUTE) {
            flags |= CapabilityFlags::Execute as u8;
        }
        if self.contains(Self::GRANT) {
            flags |= CapabilityFlags::Grant as u8;
        }
        flags
    }
}

impl core::ops::BitOr for Rights {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

impl core::ops::BitAnd for Rights {
    type Output = Self;

    fn bitand(self, rhs: Self) -> Self::Output {
        Self(self.0 & rhs.0)
    }
}

impl core::ops::Not for Rights {
    type Output = Self;

    fn not(self) -> Self::Output {
        Self(!self.0)
    }
}

impl Rights {
    /// Get the difference of rights (self minus other)
    pub fn difference(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }
}

/// Global capability ID allocator
use core::sync::atomic::AtomicU64;

static GLOBAL_CAP_ID: AtomicU64 = AtomicU64::new(1);

/// Maximum capability ID (48 bits)
const MAX_CAP_ID: u64 = (1 << 48) - 1;

/// Capability allocation error
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapAllocError {
    /// Capability ID space exhausted
    IdExhausted,
}

/// Allocate a globally unique capability ID
pub fn alloc_cap_id() -> Result<u64, CapAllocError> {
    loop {
        let current = GLOBAL_CAP_ID.load(Ordering::Relaxed);

        // Check if we've exhausted the ID space
        if current > MAX_CAP_ID {
            return Err(CapAllocError::IdExhausted);
        }

        let next = current + 1;

        // Use compare_exchange_weak for better performance
        match GLOBAL_CAP_ID.compare_exchange_weak(
            current,
            next,
            Ordering::Release,
            Ordering::Relaxed,
        ) {
            Ok(_) => return Ok(current),
            Err(_) => {
                // Another thread updated the counter, retry
                continue;
            }
        }
    }
}
