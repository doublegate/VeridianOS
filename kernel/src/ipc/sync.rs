//! Synchronous IPC implementation
//!
//! Provides blocking send/receive operations with direct handoff between
//! processes.

#![allow(dead_code)]

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};

#[cfg(feature = "alloc")]
use spin::Mutex;

use super::{
    error::{IpcError, Result},
    fast_path::{fast_receive, fast_send},
    message::Message,
};
use crate::{
    process::pcb::ProcessState,
    sched::{block_on_ipc, current_process, find_process, wake_up_process},
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

    // Wait for reply
    sync_receive(current.pid)
}

/// Reply to a previous call
pub fn sync_reply(reply: Message, caller: u64) -> Result<()> {
    // Find caller process
    let caller_process = find_process(caller).ok_or(IpcError::ProcessNotFound)?;

    // Verify caller is waiting for reply
    if caller_process.state != ProcessState::Blocked {
        return Err(IpcError::InvalidMessage);
    }

    // Send reply directly
    sync_send(reply, caller)
}

/// Slow path for synchronous send
fn sync_send_slow_path(msg: Message, target_endpoint: u64) -> Result<()> {
    // Find target endpoint and validate capability
    let endpoint = find_endpoint(target_endpoint)?;
    validate_send_capability(&msg, endpoint)?;

    // Check if receiver is waiting
    if let Some(receiver_pid) = endpoint.get_waiting_receiver() {
        // Direct handoff
        deliver_message_to_process(msg, receiver_pid)?;
        wake_up_process(receiver_pid);
        Ok(())
    } else {
        // Queue message
        endpoint.queue_message(msg)?;
        Ok(())
    }
}

/// Slow path for synchronous receive
fn sync_receive_slow_path(endpoint: u64) -> Result<Message> {
    let ep = find_endpoint(endpoint)?;

    // Check for queued messages
    if let Some(msg) = ep.dequeue_message() {
        return Ok(msg);
    }

    // No messages, block current process
    let current = current_process();
    ep.add_waiting_receiver(current.pid);
    block_on_ipc(endpoint);

    // When we wake up, message should be available
    ep.dequeue_message().ok_or(IpcError::InvalidMessage)
}

/// Deliver message directly to a process
fn deliver_message_to_process(_msg: Message, pid: u64) -> Result<()> {
    let _process = find_process(pid).ok_or(IpcError::ProcessNotFound)?;

    // TODO: Copy message to process's address space
    // For now, this is a placeholder

    Ok(())
}

/// Validate send capability
fn validate_send_capability(msg: &Message, _endpoint: &Endpoint) -> Result<()> {
    let cap_id = msg.capability();

    // TODO: Lookup capability in capability table
    // For now, basic validation
    if cap_id == 0 || cap_id > 0x10000 {
        return Err(IpcError::InvalidCapability);
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

// Placeholder endpoint structure
struct Endpoint {
    id: u64,
    #[cfg(feature = "alloc")]
    waiting_receivers: Mutex<Vec<u64>>,
    #[cfg(feature = "alloc")]
    message_queue: Mutex<Vec<Message>>,
}

impl Endpoint {
    #[cfg(feature = "alloc")]
    fn get_waiting_receiver(&self) -> Option<u64> {
        self.waiting_receivers.lock().pop()
    }

    #[cfg(not(feature = "alloc"))]
    fn get_waiting_receiver(&self) -> Option<u64> {
        None
    }

    #[cfg(feature = "alloc")]
    fn add_waiting_receiver(&self, pid: u64) {
        self.waiting_receivers.lock().push(pid);
    }

    #[cfg(not(feature = "alloc"))]
    fn add_waiting_receiver(&self, _pid: u64) {
        // Without alloc, we can't queue receivers
    }

    #[cfg(feature = "alloc")]
    fn queue_message(&self, msg: Message) -> Result<()> {
        let mut queue = self.message_queue.lock();
        if queue.len() >= 1024 {
            return Err(IpcError::ChannelFull);
        }
        queue.push(msg);
        Ok(())
    }

    #[cfg(not(feature = "alloc"))]
    fn queue_message(&self, _msg: Message) -> Result<()> {
        Err(IpcError::OutOfMemory)
    }

    #[cfg(feature = "alloc")]
    fn dequeue_message(&self) -> Option<Message> {
        self.message_queue.lock().pop()
    }

    #[cfg(not(feature = "alloc"))]
    fn dequeue_message(&self) -> Option<Message> {
        None
    }
}

fn find_endpoint(_id: u64) -> Result<&'static Endpoint> {
    // TODO: Implement endpoint lookup
    Err(IpcError::EndpointNotFound)
}

#[cfg(target_arch = "x86_64")]
fn read_timestamp() -> u64 {
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
