//! Capability system module
//!
//! Implements capability-based security for VeridianOS. Every resource
//! access in the microkernel requires an unforgeable capability token.
//!
//! Key components:
//! - 64-bit packed capability tokens with generation counters
//! - Two-level capability space with O(1) lookup
//! - Hierarchical inheritance and cascading revocation
//! - Per-CPU capability cache for performance
//! - Integration with IPC, memory, and process subsystems

// Capability types and operations are fully implemented but not all paths
// are exercised yet. Will be fully active once user-space capability
// enforcement is enabled.
#![allow(dead_code)]

pub mod inheritance;
pub mod ipc_integration;
pub mod manager;
pub mod memory_integration;
pub mod object;
pub mod revocation;
pub mod space;
pub mod token;
pub mod types;

#[cfg(all(test, not(target_os = "none")))]
mod tests;

// Re-export common types
pub use manager::CapError;
// Re-export ObjectRef for RAII tests and examples
pub use object::ObjectRef;
pub use space::CapabilitySpace;
pub use token::{CapabilityToken, Rights};
pub use types::CapabilityId;

#[cfg(feature = "alloc")]
extern crate alloc;

use spin::RwLock;

/// Kernel's capability space - holds the root capability and kernel-level caps.
/// Uses a large quota since the kernel may need to create caps for all
/// processes.
static KERNEL_CAP_SPACE: RwLock<Option<CapabilitySpace>> = RwLock::new(None);

/// Get a reference to the kernel's capability space
pub fn kernel_cap_space() -> &'static RwLock<Option<CapabilitySpace>> {
    &KERNEL_CAP_SPACE
}

/// The root capability token (Memory, ALL rights, generation 0, slot 1)
static ROOT_CAP: spin::Once<CapabilityToken> = spin::Once::new();

/// Get the root capability token
pub fn root_capability() -> Option<CapabilityToken> {
    ROOT_CAP.get().copied()
}

pub fn init() {
    kprintln!("[CAP] Initializing capability system...");

    // The global capability manager is already initialized as a static

    // Create kernel capability space with unlimited quota
    let kernel_space = CapabilitySpace::with_quota(4096);

    // Create root capability: Memory type, ALL rights, generation 0
    let root_cap = CapabilityToken::new(
        1, // ID 1 (ID 0 is null)
        0, // generation 0
        0, // type: Memory
        Rights::ALL.to_flags(),
    );

    // Insert root capability into kernel space
    let root_object = object::ObjectRef::Memory {
        base: 0,
        size: usize::MAX,
        attributes: object::MemoryAttributes::normal(),
    };

    if let Err(_e) = kernel_space.insert(root_cap, root_object, Rights::ALL) {
        kprint_rt!("[CAP] WARNING: Failed to create root capability");
        kprintln!();
    } else {
        ROOT_CAP.call_once(|| root_cap);
        kprintln!("[CAP] Root capability created (id=1, rights=ALL)");
    }

    // Store kernel capability space
    *KERNEL_CAP_SPACE.write() = Some(kernel_space);

    kprintln!("[CAP] Capability system initialized");
}
