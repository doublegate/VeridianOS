//! Inter-Process Communication (IPC) subsystem for VeridianOS
//!
//! This module implements high-performance IPC mechanisms including:
//! - Synchronous message passing with < 5μs latency
//! - Asynchronous channels for streaming
//! - Zero-copy shared memory transfers
//! - Capability-based security integration

pub mod async_channel;
pub mod capability;
pub mod channel;
pub mod error;
pub mod fast_path;
pub mod message;
pub mod perf;
pub mod rate_limit;
pub mod registry;
pub mod shared_memory;
pub mod sync;
pub mod zero_copy;

#[cfg(test)]
mod tests;

// Re-export core types
pub use error::IpcError;
pub use message::{Message, SmallMessage};

/// IPC system initialization
#[allow(dead_code)]
pub fn init() {
    println!("[IPC] Initializing IPC system...");

    // Initialize the global IPC registry
    registry::init();

    // TODO: Initialize message queues
    // TODO: Set up shared memory regions
    // TODO: Initialize synchronization primitives

    println!("[IPC] IPC system initialized");
}
