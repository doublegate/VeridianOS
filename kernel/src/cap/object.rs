//! Capability object references
//!
//! Defines the kernel objects that capabilities can reference.

use crate::{
    ipc::Endpoint,
    process::{ProcessId, ThreadId},
};

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::sync::Arc;

/// Memory attributes for memory capabilities
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryAttributes {
    pub readable: bool,
    pub writable: bool,
    pub executable: bool,
    pub cacheable: bool,
    pub device: bool,
}

impl MemoryAttributes {
    pub const fn normal() -> Self {
        Self {
            readable: true,
            writable: true,
            executable: false,
            cacheable: true,
            device: false,
        }
    }

    pub const fn device() -> Self {
        Self {
            readable: true,
            writable: true,
            executable: false,
            cacheable: false,
            device: true,
        }
    }
}

/// References to kernel objects
#[derive(Clone)]
pub enum ObjectRef {
    /// Physical memory region
    Memory {
        base: usize, // Physical address
        size: usize,
        attributes: MemoryAttributes,
    },
    /// Process control
    Process { pid: ProcessId },
    /// Thread control
    Thread { tid: ThreadId },
    /// IPC endpoint
    #[cfg(feature = "alloc")]
    Endpoint { endpoint: Arc<Endpoint> },
    /// Hardware interrupt
    Interrupt { irq: u32 },
    /// I/O port range (x86_64 specific)
    IoPort { base: u16, size: u16 },
    /// Page table
    PageTable { root: usize, asid: u16 },
    /// Capability space itself (for meta-operations)
    CapabilitySpace { pid: ProcessId },
    /// Hardware device
    Device { device_id: u64 },
}

impl ObjectRef {
    /// Get the type code for this object
    pub fn type_code(&self) -> u8 {
        match self {
            ObjectRef::Memory { .. } => 0,
            ObjectRef::Process { .. } => 1,
            ObjectRef::Thread { .. } => 2,
            #[cfg(feature = "alloc")]
            ObjectRef::Endpoint { .. } => 3,
            ObjectRef::Interrupt { .. } => 4,
            ObjectRef::IoPort { .. } => 5,
            ObjectRef::PageTable { .. } => 6,
            ObjectRef::CapabilitySpace { .. } => 7,
            ObjectRef::Device { .. } => 8,
        }
    }

    /// Check if this object reference is valid
    pub fn is_valid(&self) -> bool {
        match self {
            ObjectRef::Memory { size, .. } => *size > 0,
            ObjectRef::Process { pid } => pid.0 > 0,
            ObjectRef::Thread { tid } => tid.0 > 0,
            #[cfg(feature = "alloc")]
            ObjectRef::Endpoint { .. } => true,
            ObjectRef::Interrupt { irq } => *irq < 256, // Reasonable IRQ limit
            ObjectRef::IoPort { size, .. } => *size > 0,
            ObjectRef::PageTable { .. } => true,
            ObjectRef::CapabilitySpace { pid } => pid.0 > 0,
            ObjectRef::Device { .. } => true,
        }
    }
}

/// Object access type for permission checking
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    Read,
    Write,
    Execute,
    Grant,
    Revoke,
}

impl Access {
    /// Convert to rights bit
    pub fn to_rights_bit(self) -> u32 {
        match self {
            Access::Read => 1 << 0,
            Access::Write => 1 << 1,
            Access::Execute => 1 << 2,
            Access::Grant => 1 << 3,
            Access::Revoke => 1 << 4,
        }
    }
}
