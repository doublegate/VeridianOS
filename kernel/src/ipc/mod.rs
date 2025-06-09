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
#[allow(unused_imports)]
pub use async_channel::AsyncChannel;
#[allow(unused_imports)]
pub use capability::{EndpointId, IpcCapability, IpcPermissions, Permission, ProcessId};
#[allow(unused_imports)]
pub use channel::{Channel, Endpoint};
#[allow(unused_imports)]
pub use error::{IpcError, Result};
#[allow(unused_imports)]
pub use message::{LargeMessage, Message, SmallMessage};
#[allow(unused_imports)]
pub use perf::{cycles_to_ns, measure_ipc_operation, read_timestamp, IPC_PERF_STATS};
#[allow(unused_imports)]
pub use rate_limit::{RateLimits, RATE_LIMITER};
// Re-export internal functions for tests
#[cfg(test)]
pub use registry::lookup_endpoint;
#[allow(unused_imports)]
pub use registry::{create_channel, create_endpoint, get_registry_stats, validate_capability};
#[allow(unused_imports)]
pub use shared_memory::{Permissions, SharedRegion, TransferMode};
#[cfg(test)]
pub use sync::send_message;

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
