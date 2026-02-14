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
#[allow(unused_imports)]
pub use object::ObjectRef;
pub use space::CapabilitySpace;
pub use token::{CapabilityToken, Rights};
pub use types::CapabilityId;

pub fn init() {
    #[cfg(target_arch = "aarch64")]
    {
        // SAFETY: uart_write_str performs a raw MMIO write to the PL011 UART at
        // 0x09000000. This is safe during kernel init as the UART is memory-mapped
        // by QEMU's virt machine and the write is non-destructive.
        unsafe {
            use crate::arch::aarch64::direct_uart::uart_write_str;
            uart_write_str("[CAP] Initializing capability system...\n");
        }
    }
    #[cfg(not(target_arch = "aarch64"))]
    println!("[CAP] Initializing capability system...");

    // The global capability manager is already initialized as a static

    // TODO(phase3): Create root capability for kernel and initial capability spaces
    // for core processes

    #[cfg(target_arch = "aarch64")]
    {
        // SAFETY: uart_write_str performs a raw MMIO write to the PL011 UART at
        // 0x09000000. This is safe during kernel init as the UART is memory-mapped
        // by QEMU's virt machine and the write is non-destructive.
        unsafe {
            use crate::arch::aarch64::direct_uart::uart_write_str;
            uart_write_str("[CAP] Capability system initialized\n");
        }
    }
    #[cfg(not(target_arch = "aarch64"))]
    println!("[CAP] Capability system initialized");
}
