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

impl PartialEq for ObjectRef {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (
                ObjectRef::Memory {
                    base: b1,
                    size: s1,
                    attributes: a1,
                },
                ObjectRef::Memory {
                    base: b2,
                    size: s2,
                    attributes: a2,
                },
            ) => b1 == b2 && s1 == s2 && a1 == a2,
            (ObjectRef::Process { pid: p1 }, ObjectRef::Process { pid: p2 }) => p1 == p2,
            (ObjectRef::Thread { tid: t1 }, ObjectRef::Thread { tid: t2 }) => t1 == t2,
            #[cfg(feature = "alloc")]
            (ObjectRef::Endpoint { endpoint: e1 }, ObjectRef::Endpoint { endpoint: e2 }) => {
                Arc::ptr_eq(e1, e2)
            }
            (ObjectRef::Interrupt { irq: i1 }, ObjectRef::Interrupt { irq: i2 }) => i1 == i2,
            (
                ObjectRef::IoPort { base: b1, size: s1 },
                ObjectRef::IoPort { base: b2, size: s2 },
            ) => b1 == b2 && s1 == s2,
            (
                ObjectRef::PageTable { root: r1, asid: a1 },
                ObjectRef::PageTable { root: r2, asid: a2 },
            ) => r1 == r2 && a1 == a2,
            (ObjectRef::CapabilitySpace { pid: p1 }, ObjectRef::CapabilitySpace { pid: p2 }) => {
                p1 == p2
            }
            (ObjectRef::Device { device_id: d1 }, ObjectRef::Device { device_id: d2 }) => d1 == d2,
            _ => false,
        }
    }
}

impl Eq for ObjectRef {}

impl core::fmt::Debug for ObjectRef {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ObjectRef::Memory {
                base,
                size,
                attributes,
            } => f
                .debug_struct("Memory")
                .field("base", base)
                .field("size", size)
                .field("attributes", attributes)
                .finish(),
            ObjectRef::Process { pid } => f.debug_struct("Process").field("pid", pid).finish(),
            ObjectRef::Thread { tid } => f.debug_struct("Thread").field("tid", tid).finish(),
            #[cfg(feature = "alloc")]
            ObjectRef::Endpoint { .. } => f.debug_struct("Endpoint").finish(),
            ObjectRef::Interrupt { irq } => f.debug_struct("Interrupt").field("irq", irq).finish(),
            ObjectRef::IoPort { base, size } => f
                .debug_struct("IoPort")
                .field("base", base)
                .field("size", size)
                .finish(),
            ObjectRef::PageTable { root, asid } => f
                .debug_struct("PageTable")
                .field("root", root)
                .field("asid", asid)
                .finish(),
            ObjectRef::CapabilitySpace { pid } => {
                f.debug_struct("CapabilitySpace").field("pid", pid).finish()
            }
            ObjectRef::Device { device_id } => f
                .debug_struct("Device")
                .field("device_id", device_id)
                .finish(),
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
