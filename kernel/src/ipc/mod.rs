//! Inter-Process Communication (IPC) subsystem for VeridianOS
//!
//! This module implements high-performance IPC mechanisms including:
//! - Synchronous message passing with < 5μs latency
//! - Asynchronous channels for streaming
//! - Zero-copy shared memory transfers
//! - Capability-based security integration

pub mod async_channel;
pub mod cap_transfer;
pub mod capability;
pub mod channel;
pub mod error;
pub mod fast_path;
pub mod message;
pub mod message_passing;
pub mod perf;
pub mod rate_limit;
pub mod registry;
pub mod rpc;
pub mod shared_memory;
pub mod sync;
pub mod zero_copy;

#[cfg(test)]
mod tests;

// Re-export core types
pub use async_channel::AsyncChannel;
pub use capability::{EndpointId, IpcCapability, IpcPermissions, Permission, ProcessId};
pub use channel::{Channel, Endpoint};
pub use error::{IpcError, Result};
pub use message::{LargeMessage, Message, SmallMessage};
pub use perf::{cycles_to_ns, measure_ipc_operation, IPC_PERF_STATS};
pub use rate_limit::{RateLimits, RATE_LIMITER};
// Re-export internal functions for tests
#[cfg(test)]
pub use registry::lookup_endpoint;
pub use registry::{
    create_channel, create_endpoint, get_registry_stats, remove_process_endpoints,
    validate_capability,
};
pub use shared_memory::{Permissions, SharedRegion, TransferMode};
#[cfg(test)]
pub use sync::send_message;
pub use sync::{sync_call, sync_receive, sync_reply, sync_send};

pub use crate::arch::entropy::read_timestamp;

/// IPC system initialization
#[allow(dead_code)]
pub fn init() {
    kprintln!("[IPC] Initializing IPC system...");

    // Initialize the global IPC registry
    registry::init();

    kprintln!("[IPC] IPC system initialized");
}
