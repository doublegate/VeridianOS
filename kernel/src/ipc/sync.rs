//! Synchronous IPC implementation
//!
//! Provides blocking send/receive operations with direct handoff between
//! processes.

#![allow(dead_code)]

#[cfg(feature = "alloc")]
extern crate alloc;

use core::sync::atomic::{AtomicU64, Ordering};

use super::{
    error::{IpcError, Result},
    fast_path::{fast_receive, fast_send},
    message::Message,
};
use crate::{
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
            // Try fast path first
            match fast_send(&small_msg, target_endpoint) {
                Ok(()) => {
                    SYNC_STATS.fast_path_count.fetch_add(1, Ordering::Relaxed);
                    update_latency_stats(start);
                    Ok(())
                }
                Err(IpcError::WouldBlock) => {
                    // Fall back to slow path
                    SYNC_STATS.slow_path_count.fetch_add(1, Ordering::Relaxed);
                    sync_send_slow_path(Message::Small(small_msg), target_endpoint)?;
                    update_latency_stats(start);
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
        Message::Large(large_msg) => {
            // Large messages always use slow path
            SYNC_STATS.slow_path_count.fetch_add(1, Ordering::Relaxed);
            sync_send_slow_path(Message::Large(large_msg), target_endpoint)?;
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
        Ok(small_msg) => {
            SYNC_STATS.fast_path_count.fetch_add(1, Ordering::Relaxed);
            update_latency_stats(start);
            Ok(Message::Small(small_msg))
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
    sync_send(reply, caller)
}

/// Slow path for synchronous send
fn sync_send_slow_path(msg: Message, target_endpoint: u64) -> Result<()> {
    // Validate send capability
    validate_send_capability(&msg, target_endpoint)?;

    // Use message passing subsystem
    #[cfg(feature = "alloc")]
    {
        crate::ipc::message_passing::send_to_endpoint(msg, target_endpoint)
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

    // TODO(phase3): Verify capability is for the specific endpoint_id
    let _ = endpoint_id;

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

#[cfg(target_arch = "x86_64")]
fn read_timestamp() -> u64 {
    // SAFETY: _rdtsc() reads the x86_64 Time Stamp Counter via the RDTSC
    // instruction. This is a read-only, side-effect-free operation that is always
    // available in kernel mode and requires no special setup or preconditions.
    unsafe { core::arch::x86_64::_rdtsc() }
}

#[cfg(not(target_arch = "x86_64"))]
fn read_timestamp() -> u64 {
    0
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
}
