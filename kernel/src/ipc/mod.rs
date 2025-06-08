//! Inter-Process Communication (IPC) subsystem for VeridianOS
//! 
//! This module implements high-performance IPC mechanisms including:
//! - Synchronous message passing with < 5μs latency
//! - Asynchronous channels for streaming
//! - Zero-copy shared memory transfers
//! - Capability-based security integration

pub mod message;
pub mod channel;
pub mod shared_memory;
pub mod capability;
pub mod error;

#[cfg(test)]
mod tests;

// Re-export core types
pub use message::{SmallMessage, LargeMessage, Message};
pub use channel::{Channel, Endpoint};
pub use shared_memory::{SharedRegion, MemoryRegion};
pub use capability::{IpcCapability, IpcPermissions, CapabilityType};
pub use error::{IpcError, Result};

/// IPC system initialization
#[allow(dead_code)]
pub fn init() {
    println!("[IPC] Initializing IPC system...");
    // TODO: Initialize message queues
    // TODO: Set up shared memory regions
    // TODO: Initialize synchronization primitives
    println!("[IPC] IPC system initialized");
}
