//! Capability system module
//!
//! Implements capability-based security for VeridianOS

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
    println!("[CAP] Initializing capability system...");

    // The global capability manager is already initialized as a static

    // TODO: Create root capability for kernel
    // TODO: Set up initial capability spaces for core processes

    println!("[CAP] Capability system initialized");
}
