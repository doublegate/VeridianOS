//! Synchronous IPC implementation
//!
//! Provides blocking send/receive operations with direct handoff between
//! processes.

// Synchronous IPC -- exercised via syscall IPC paths
#![allow(dead_code)]

#[cfg(feature = "alloc")]
extern crate alloc;

use core::sync::atomic::{AtomicU64, Ordering};

use super::{
    error::{IpcError, Result},
    fast_path::fast_receive,
    message::Message,
};
use crate::{
    arch::entropy::read_timestamp,
    process::{ProcessId, ProcessState},
    sched::{current_process, find_process},
};

/// Statistics for synchronous IPC
pub struct SyncIpcStats {
    pub send_count: AtomicU64,
    pub receive_count: AtomicU64,
    pub fast_path_count: AtomicU64,
    pub slow_path_count: AtomicU64,
    pub avg_latency_cycles: AtomicU64,
}

static SYNC_STATS: SyncIpcStats = SyncIpcStats {
    send_count: AtomicU64::new(0),
    receive_count: AtomicU64::new(0),
    fast_path_count: AtomicU64::new(0),
    slow_path_count: AtomicU64::new(0),
    avg_latency_cycles: AtomicU64::new(0),
};

/// Simple send message function for tests
#[cfg(test)]
pub fn send_message(msg: Message, target_endpoint: u64) -> Result<()> {
    sync_send(msg, target_endpoint)
}

/// Synchronous message send
///
/// Blocks until message is delivered to receiver.
pub fn sync_send(msg: Message, target_endpoint: u64) -> Result<()> {
    let start = read_timestamp();
    SYNC_STATS.send_count.fetch_add(1, Ordering::Relaxed);

    match msg {
        Message::Small(small_msg) => {
            // Small messages also take the queued path for now. A direct
            // hand-off needs the receiver to be claimed atomically from a
            // receive-only wait list bound to this endpoint; the shared
            // IPC wait queue cannot do that (senders blocked on a full
            // endpoint park under the same key, and the task can change
            // state between lookup and delivery). The queued path checks
            // the capability's endpoint binding and wakes waiters
            // correctly. Restored with the IPC-SYNC-01/02 wait rework.
            SYNC_STATS.slow_path_count.fetch_add(1, Ordering::Relaxed);
            sync_send_slow_path(Message::Small(small_msg), target_endpoint)?;
            update_latency_stats(start);
            Ok(())
        }
        msg @ (Message::Large(_) | Message::Buffered(_)) => {
            // Large and buffered messages always use slow path
            SYNC_STATS.slow_path_count.fetch_add(1, Ordering::Relaxed);
            sync_send_slow_path(msg, target_endpoint)?;
            update_latency_stats(start);
            Ok(())
        }
    }
}

/// Synchronous message receive
///
/// Blocks until a message is available.
pub fn sync_receive(endpoint: u64) -> Result<Message> {
    let start = read_timestamp();
    SYNC_STATS.receive_count.fetch_add(1, Ordering::Relaxed);

    // Try fast path for small messages
    match fast_receive(endpoint, None) {
        Ok(msg) => {
            SYNC_STATS.fast_path_count.fetch_add(1, Ordering::Relaxed);
            update_latency_stats(start);
            Ok(msg)
        }
        Err(IpcError::WouldBlock) => {
            // Fall back to slow path
            SYNC_STATS.slow_path_count.fetch_add(1, Ordering::Relaxed);
            let msg = sync_receive_slow_path(endpoint)?;
            update_latency_stats(start);
            Ok(msg)
        }
        Err(e) => Err(e),
    }
}

/// Call operation (send and wait for reply)
pub fn sync_call(request: Message, target: u64) -> Result<Message> {
    // Send request
    sync_send(request, target)?;

    // Mark ourselves as waiting for reply
    let current = current_process();
    current.state = ProcessState::Blocked;

    // Wait for reply using process ID as endpoint
    sync_receive(current.pid.0)
}

/// Reply to a previous call
pub fn sync_reply(reply: Message, caller: u64) -> Result<()> {
    // Find caller process
    let caller_process = find_process(ProcessId(caller)).ok_or(IpcError::ProcessNotFound)?;

    // Verify caller is waiting for reply
    if caller_process.state != ProcessState::Blocked {
        return Err(IpcError::InvalidMessage);
    }

    // Send reply directly
    sync_send(reply, caller)?;

    // Wake the caller process after reply is sent
    crate::sched::ipc_blocking::wake_up_process(ProcessId(caller));
    Ok(())
}

/// Slow path for synchronous send
fn sync_send_slow_path(msg: Message, target_endpoint: u64) -> Result<()> {
    // Validate send capability
    validate_send_capability(&msg, target_endpoint)?;

    // Use message passing subsystem with retry-on-full blocking
    #[cfg(feature = "alloc")]
    {
        const MAX_RETRIES: u32 = 3;
        for _attempt in 0..MAX_RETRIES {
            match crate::ipc::message_passing::send_to_endpoint(msg.clone(), target_endpoint) {
                Ok(()) => {
                    // Wake any processes waiting on this endpoint
                    crate::sched::ipc_blocking::wake_up_endpoint_waiters(target_endpoint);
                    return Ok(());
                }
                Err(IpcError::ChannelFull) => {
                    // Block until space available
                    crate::sched::ipc_blocking::block_on_ipc(target_endpoint);
                }
                Err(e) => return Err(e),
            }
        }
        Err(IpcError::ChannelFull)
    }
    #[cfg(not(feature = "alloc"))]
    {
        Err(IpcError::OutOfMemory)
    }
}

/// Slow path for synchronous receive
fn sync_receive_slow_path(endpoint: u64) -> Result<Message> {
    // Use message passing subsystem with blocking
    #[cfg(feature = "alloc")]
    {
        crate::ipc::message_passing::receive_from_endpoint(endpoint, true)
    }
    #[cfg(not(feature = "alloc"))]
    {
        Err(IpcError::OutOfMemory)
    }
}

/// Validate send capability
fn validate_send_capability(msg: &Message, endpoint_id: u64) -> Result<()> {
    let cap_id = msg.capability();

    // Get current process's capability space
    let current_process = crate::process::current_process().ok_or(IpcError::ProcessNotFound)?;
    let cap_space = current_process.capability_space.lock();

    // Convert capability ID to token
    let cap_token = crate::cap::CapabilityToken::from_u64(cap_id);

    // Check if the capability grants send permission for this endpoint
    // Note: This checks the capability exists, is valid, and has SEND rights
    crate::cap::ipc_integration::check_send_permission(cap_token, &cap_space).map_err(
        |e| match e {
            IpcError::InvalidCapability => IpcError::InvalidCapability,
            IpcError::PermissionDenied => IpcError::PermissionDenied,
            _ => IpcError::InvalidCapability,
        },
    )?;

    // Verify the capability is associated with the target endpoint.
    // Look up the full capability entry to check the ObjectRef.
    // Non-endpoint capabilities are also valid for general IPC
    // (e.g., process capabilities can send on any endpoint they
    // have SEND rights for).
    #[cfg(feature = "alloc")]
    {
        if let Some((crate::cap::object::ObjectRef::Endpoint { endpoint }, _rights)) =
            cap_space.lookup_entry(cap_token)
        {
            if endpoint.id() != endpoint_id {
                return Err(IpcError::InvalidCapability);
            }
        }
    }
    #[cfg(not(feature = "alloc"))]
    {
        let _ = endpoint_id;
    }

    Ok(())
}

/// Update latency statistics
fn update_latency_stats(start_cycles: u64) {
    let elapsed = read_timestamp() - start_cycles;
    let count = SYNC_STATS.send_count.load(Ordering::Relaxed)
        + SYNC_STATS.receive_count.load(Ordering::Relaxed);
    let current_avg = SYNC_STATS.avg_latency_cycles.load(Ordering::Relaxed);

    // Calculate new average
    let new_avg = if count > 1 {
        (current_avg * (count - 1) + elapsed) / count
    } else {
        elapsed
    };

    SYNC_STATS
        .avg_latency_cycles
        .store(new_avg, Ordering::Relaxed);

    // Also record in global performance stats
    let is_fast_path = SYNC_STATS.fast_path_count.load(Ordering::Relaxed)
        > SYNC_STATS.slow_path_count.load(Ordering::Relaxed);
    crate::ipc::perf::IPC_PERF_STATS.record_operation(elapsed, is_fast_path);
}

/// Get synchronous IPC statistics
pub fn get_sync_stats() -> SyncStatsSummary {
    SyncStatsSummary {
        send_count: SYNC_STATS.send_count.load(Ordering::Relaxed),
        receive_count: SYNC_STATS.receive_count.load(Ordering::Relaxed),
        fast_path_count: SYNC_STATS.fast_path_count.load(Ordering::Relaxed),
        slow_path_count: SYNC_STATS.slow_path_count.load(Ordering::Relaxed),
        avg_latency_cycles: SYNC_STATS.avg_latency_cycles.load(Ordering::Relaxed),
        fast_path_percentage: {
            let fast = SYNC_STATS.fast_path_count.load(Ordering::Relaxed);
            let total = fast + SYNC_STATS.slow_path_count.load(Ordering::Relaxed);
            if total > 0 {
                (fast * 100) / total
            } else {
                0
            }
        },
    }
}

pub struct SyncStatsSummary {
    pub send_count: u64,
    pub receive_count: u64,
    pub fast_path_count: u64,
    pub slow_path_count: u64,
    pub avg_latency_cycles: u64,
    pub fast_path_percentage: u64,
}

#[cfg(all(test, not(target_os = "none")))]
mod tests {
    use super::*;

    #[test]
    fn test_sync_stats() {
        let stats = get_sync_stats();
        assert_eq!(stats.send_count, 0);
        assert_eq!(stats.receive_count, 0);
        assert_eq!(stats.fast_path_percentage, 0);
    }

    /// N-33 pin. Endpoints are created in `ipc::registry`, but the send and
    /// receive paths `sync_send`/`sync_receive` use (`message_passing::
    /// send_to_endpoint`/`receive_from_endpoint`) look them up in
    /// `message_passing::ENDPOINT_REGISTRY`, which nothing ever populates.
    /// So every send fails with `EndpointNotFound`. Un-ignore this when the
    /// IPC rework unifies the registries (review of the v0.26.0 stack,
    /// PR #8).
    #[cfg(feature = "alloc")]
    #[test]
    #[ignore = "N-33: native IPC unreachable; endpoint registries split"]
    fn endpoint_from_registry_round_trips_through_send_path() {
        crate::ipc::registry::init();
        let (id, _cap) = crate::ipc::registry::create_endpoint(ProcessId(1)).unwrap();
        assert!(
            crate::ipc::message_passing::ENDPOINT_REGISTRY
                .get(id)
                .is_some(),
            "endpoint {} is missing from the registry the send path uses",
            id
        );
        crate::ipc::message_passing::send_to_endpoint(Message::small(0, 7), id).unwrap();
        let received = crate::ipc::message_passing::receive_from_endpoint(id, false).unwrap();
        assert_eq!(received.opcode(), 7);
    }
}
