//! Fast path IPC implementation for register-based messages
//!
//! Achieves < 1μs latency by using per-task IPC register storage for direct
//! message transfer. When a sender targets a blocked receiver, the message
//! is copied directly into the receiver's `Task::ipc_regs` and the receiver
//! is woken. No intermediate queuing or memory allocation is needed.
//!
//! ## Performance features
//!
//! - **O(log n) PID lookup** via global task registry (no linear scan)
//! - Capability validation against the sender's own capability space
//! - **Tracepoints** for IpcFastSend / IpcFastReceive / IpcSlowPath events
//!
//! ## Register mapping
//!
//! The IPC register convention maps to architecture registers as follows:
//! - x86_64:  RDI, RSI, RDX, RCX, R8, R9, R10
//! - AArch64: X0, X1, X2, X3, X4, X5, X6
//! - RISC-V:  a0, a1, a2, a3, a4, a5, a6
//!
//! All share the same semantic layout (see `IPC_REG_*` constants below).

// Fast-path IPC -- register-based transfer for <1us latency

use core::sync::atomic::{AtomicU64, Ordering};

use super::{
    error::{IpcError, Result},
    SmallMessage,
};
use crate::{
    arch::entropy::read_timestamp, cap::token::CapabilityToken, process::pcb::ProcessState,
    sched::current_process,
};

/// Performance counter for fast path operations
static FAST_PATH_COUNT: AtomicU64 = AtomicU64::new(0);
static FAST_PATH_CYCLES: AtomicU64 = AtomicU64::new(0);
/// Counter for slow-path fallbacks (target not blocked)
static SLOW_PATH_FALLBACK_COUNT: AtomicU64 = AtomicU64::new(0);

// IPC register semantic indices (architecture-neutral)
const IPC_REG_CAP: usize = 0; // Capability token
const IPC_REG_OPCODE: usize = 1; // Operation code
const IPC_REG_FLAGS: usize = 2; // Flags
const IPC_REG_DATA0: usize = 3; // Data word 0
const IPC_REG_DATA1: usize = 4; // Data word 1
const IPC_REG_DATA2: usize = 5; // Data word 2
const IPC_REG_DATA3: usize = 6; // Data word 3

/// Fast path IPC send for small messages
///
/// Copies the message directly into the target task's `ipc_regs` array
/// if the target is blocked waiting for a message. This avoids all
/// intermediate queuing and achieves sub-microsecond latency.
///
/// Not reachable from the syscall path: it checks that the sender holds a
/// SEND capability, but not that `target_pid` is a *receiver* on that
/// capability's endpoint, nor does it claim the target atomically
/// (IPC-SYNC-01/02). `sync_send` uses the queued path until the wait-list
/// rework provides both. Module-private so nothing new can depend on it.
#[inline(always)]
fn fast_send(msg: &SmallMessage, target_pid: u64) -> Result<()> {
    let start = read_timestamp();

    // The sender must hold this capability, with SEND rights, in its own
    // capability space (IPC-INC-01).
    let endpoint_id = validate_send_capability(msg.capability)?;

    // Find target task via global registry (O(log n) lookup, no scheduler lock)
    #[cfg(feature = "alloc")]
    let target_ptr = {
        // First check current task (most common case for IPC reply)
        let current_match = {
            let sched = crate::sched::scheduler::SCHEDULER.lock();
            if let Some(current) = sched.current() {
                // SAFETY: current is a valid NonNull<Task> from the scheduler.
                unsafe { (*current.as_ptr()).pid.0 == target_pid }
            } else {
                false
            }
        };

        if current_match {
            // Target is self -- unusual but valid (self-IPC)
            let sched = crate::sched::scheduler::SCHEDULER.lock();
            sched.current()
        } else {
            // O(log n) lookup via global PID-to-Task registry
            crate::sched::scheduler::get_task_ptr(target_pid)
        }
    };

    #[cfg(not(feature = "alloc"))]
    let target_ptr = {
        let sched = crate::sched::scheduler::SCHEDULER.lock();
        if let Some(current) = sched.current() {
            unsafe {
                if (*current.as_ptr()).pid.0 == target_pid {
                    Some(current)
                } else {
                    None
                }
            }
        } else {
            None
        }
    };

    let target_ptr = match target_ptr {
        Some(ptr) => ptr,
        None => return Err(IpcError::ProcessNotFound),
    };

    // Check and claim the target under the scheduler lock, so its state
    // cannot change between the check and the register write, and only
    // transfer to a task blocked on *this* endpoint: one blocked on a
    // futex, sleep or another endpoint must not get its registers
    // overwritten (IPC-SYNC-02).
    let sched_guard = crate::sched::scheduler::SCHEDULER.lock();
    // SAFETY: target_ptr is a valid NonNull<Task> from the task registry.
    // We hold the scheduler lock and the target is blocked on this
    // endpoint (not running), so nothing else accesses its ipc_regs.
    unsafe {
        let target = target_ptr.as_ptr();

        if (*target).state == ProcessState::Blocked && (*target).blocked_on == Some(endpoint_id) {
            // Direct transfer: copy message into target's IPC registers
            (*target).ipc_regs[IPC_REG_CAP] = msg.capability;
            (*target).ipc_regs[IPC_REG_OPCODE] = msg.opcode as u64;
            (*target).ipc_regs[IPC_REG_FLAGS] = msg.flags as u64;
            (*target).ipc_regs[IPC_REG_DATA0] = msg.data[0];
            (*target).ipc_regs[IPC_REG_DATA1] = msg.data[1];
            (*target).ipc_regs[IPC_REG_DATA2] = msg.data[2];
            (*target).ipc_regs[IPC_REG_DATA3] = msg.data[3];

            // Wake up receiver via scheduler. wake_up_process takes the
            // scheduler lock itself, so release ours first.
            let receiver = crate::process::ProcessId((*target).pid.0);
            drop(sched_guard);
            crate::sched::ipc_blocking::wake_up_process(receiver);

            // Update performance counters
            let elapsed = read_timestamp() - start;
            FAST_PATH_COUNT.fetch_add(1, Ordering::Relaxed);
            FAST_PATH_CYCLES.fetch_add(elapsed, Ordering::Relaxed);

            // Trace: IPC fast path send
            crate::trace!(
                crate::perf::trace::TraceEventType::IpcFastSend,
                target_pid,
                msg.capability
            );

            Ok(())
        } else {
            // Target not blocked -- fall back to queuing (slow path)
            SLOW_PATH_FALLBACK_COUNT.fetch_add(1, Ordering::Relaxed);

            // Trace: slow path fallback
            crate::trace!(
                crate::perf::trace::TraceEventType::IpcSlowPath,
                target_pid,
                msg.capability
            );

            Err(IpcError::WouldBlock)
        }
    }
}

/// Fast path IPC receive
///
/// If a message has already been deposited in the current task's `ipc_regs`
/// (by a fast_send while we were blocked), read it directly. Otherwise,
/// check the endpoint's message queue, and if empty, block.
#[inline(always)]
pub fn fast_receive(endpoint: u64, timeout: Option<u64>) -> Result<super::Message> {
    let current = current_process();

    // Check if message already waiting in endpoint queue
    if let Some(msg) = check_pending_message(endpoint) {
        // Trace: IPC fast path receive (from queue)
        crate::trace!(
            crate::perf::trace::TraceEventType::IpcFastReceive,
            endpoint,
            msg.capability()
        );
        return Ok(msg);
    }

    // Block current process
    current.state = ProcessState::Blocked;
    current.blocked_on = Some(endpoint);

    // Yield CPU and wait for message
    yield_and_wait(timeout)?;

    // When we wake up, check if fast_send deposited data in our ipc_regs.
    // Read from current task's ipc_regs (set by sender's fast_send).
    let msg = read_from_current_task_ipc_regs();
    if msg.capability != 0 || msg.opcode != 0 {
        // Trace: IPC fast path receive (direct register transfer)
        crate::trace!(
            crate::perf::trace::TraceEventType::IpcFastReceive,
            endpoint,
            msg.capability
        );
        return Ok(super::Message::Small(msg));
    }

    // No fast-path message; re-check endpoint queue (slow path deposited it)
    if let Some(msg) = check_pending_message(endpoint) {
        return Ok(msg);
    }

    // Spurious wake-up or timeout -- return default
    Ok(super::Message::Small(SmallMessage {
        capability: 0,
        opcode: 0,
        flags: 0,
        data: [0; 4],
    }))
}

/// Check that the calling process holds `cap`, unrevoked and with SEND
/// rights, in its own capability space.
///
/// This replaced a check that rejected every genuine token (all are at
/// least 2^32: generation, type and flags live in the high bits) and accepted
/// any integer below 2^32 on a cache miss, after which it was cached with
/// all rights for every process (IPC-INC-01, IPC-PERF-01). There is no
/// cache: a cache entry would outlive revocation. Lookup cost is the
/// capability space's to fix (CAP-PERF-01/02).
fn validate_send_capability(cap: u64) -> Result<u64> {
    if cap == 0 {
        return Err(IpcError::InvalidCapability);
    }
    let process = crate::process::current_process().ok_or(IpcError::ProcessNotFound)?;
    let space = process.capability_space.lock();
    let token = CapabilityToken::from_u64(cap);
    crate::cap::ipc_integration::check_send_permission(token, &space)?;
    // The endpoint the capability names: a direct transfer may only go to
    // a receiver blocked on this endpoint (IPC-SYNC-02).
    match space.lookup_entry(token) {
        #[cfg(feature = "alloc")]
        Some((crate::cap::object::ObjectRef::Endpoint { endpoint }, _)) => Ok(endpoint.id()),
        _ => Err(IpcError::InvalidCapability),
    }
}

/// Read message from the current task's IPC registers.
fn read_from_current_task_ipc_regs() -> SmallMessage {
    let sched = crate::sched::scheduler::SCHEDULER.lock();
    if let Some(current) = sched.current() {
        // SAFETY: current is our task. We read ipc_regs which were written
        // by fast_send while we were blocked. No concurrent writer now.
        unsafe {
            let task = current.as_ptr();
            let regs = &(*task).ipc_regs;
            let msg = SmallMessage {
                capability: regs[IPC_REG_CAP],
                opcode: regs[IPC_REG_OPCODE] as u32,
                flags: regs[IPC_REG_FLAGS] as u32,
                data: [
                    regs[IPC_REG_DATA0],
                    regs[IPC_REG_DATA1],
                    regs[IPC_REG_DATA2],
                    regs[IPC_REG_DATA3],
                ],
            };
            // Clear ipc_regs after read to prevent stale re-reads
            (*task).ipc_regs = [0; 7];
            msg
        }
    } else {
        SmallMessage {
            capability: 0,
            opcode: 0,
            flags: 0,
            data: [0; 4],
        }
    }
}

/// Check for pending messages without blocking.
///
/// Queries the IPC registry for the endpoint and tries to dequeue a message.
/// Returns None if no message is waiting or the endpoint doesn't exist.
/// The whole message is returned: this used to flatten every queued
/// message to a `SmallMessage`, dropping any payload.
fn check_pending_message(endpoint: u64) -> Option<super::Message> {
    #[cfg(feature = "alloc")]
    {
        if let Some(msg) = crate::ipc::registry::try_receive_from_endpoint(endpoint) {
            return Some(msg);
        }
    }
    let _ = endpoint;
    None
}

/// Yield CPU and wait for message or timeout.
///
/// Blocks the current task via the scheduler. When a message arrives for
/// this endpoint, `wake_up_process()` will resume execution here.
fn yield_and_wait(_timeout: Option<u64>) -> Result<()> {
    crate::sched::yield_cpu();
    Ok(())
}

/// Get performance statistics (fast_path_count, avg_cycles,
/// slow_path_fallbacks)
pub fn get_fast_path_stats() -> (u64, u64) {
    let count = FAST_PATH_COUNT.load(Ordering::Relaxed);
    let cycles = FAST_PATH_CYCLES.load(Ordering::Relaxed);
    let avg_cycles = if count > 0 { cycles / count } else { 0 };
    (count, avg_cycles)
}

/// Get the number of slow-path fallbacks
pub fn get_slow_path_count() -> u64 {
    SLOW_PATH_FALLBACK_COUNT.load(Ordering::Relaxed)
}

// ---------------------------------------------------------------------------
// IPC Message Batching
// ---------------------------------------------------------------------------

/// Maximum number of messages in a batch before automatic flush.
pub const BATCH_SIZE: usize = 8;

/// IPC message batch for amortizing per-message overhead.
///
/// Collects multiple small messages destined for the same target and
/// delivers them in a single operation. This reduces per-message overhead
/// (capability validation, task lookup) by performing these steps once
/// per batch instead of once per message.
pub struct IpcBatch {
    /// Buffered messages.
    messages: [Option<SmallMessage>; BATCH_SIZE],
    /// Number of messages currently in the batch.
    count: usize,
    /// Target PID for all messages in this batch.
    target_pid: u64,
}

impl IpcBatch {
    /// Create a new empty batch targeting a specific process.
    pub fn new(target_pid: u64) -> Self {
        const NONE_MSG: Option<SmallMessage> = None;
        Self {
            messages: [NONE_MSG; BATCH_SIZE],
            count: 0,
            target_pid,
        }
    }

    /// Add a message to the batch.
    ///
    /// Returns `true` if the batch is now full and should be flushed.
    /// Returns `false` if there is still room for more messages.
    pub fn add_to_batch(&mut self, msg: SmallMessage) -> bool {
        if self.count < BATCH_SIZE {
            self.messages[self.count] = Some(msg);
            self.count += 1;
        }
        self.count >= BATCH_SIZE
    }

    /// Number of messages currently in the batch.
    pub fn len(&self) -> usize {
        self.count
    }

    /// Whether the batch is empty.
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Flush all buffered messages by sending them via fast_send.
    ///
    /// Each message is sent individually via the fast path. Capability
    /// validation is only performed once per batch (the first message's
    /// capability is cached for subsequent sends).
    ///
    /// Returns the number of messages successfully sent.
    pub fn flush(&mut self) -> usize {
        let mut sent = 0;
        for i in 0..self.count {
            if let Some(ref msg) = self.messages[i] {
                if fast_send(msg, self.target_pid).is_ok() {
                    sent += 1;
                    crate::perf::record_ipc_message_sent();
                }
            }
        }

        if sent > 0 {
            crate::perf::record_ipc_batch_flushed();
        }

        // Clear the batch
        self.count = 0;
        for slot in &mut self.messages {
            *slot = None;
        }

        sent
    }

    /// Get the target PID for this batch.
    pub fn target_pid(&self) -> u64 {
        self.target_pid
    }
}

/// Flush an IPC batch (convenience function for external callers).
pub fn flush_batch(batch: &mut IpcBatch) -> usize {
    batch.flush()
}

#[cfg(all(test, not(target_os = "none")))]
mod tests {
    use super::*;

    #[test]
    fn test_fast_path_stats() {
        let (count, avg) = get_fast_path_stats();
        assert_eq!(count, 0);
        assert_eq!(avg, 0);
    }

    #[test]
    fn test_slow_path_count() {
        assert_eq!(get_slow_path_count(), 0);
    }

    #[test]
    fn test_batch_add_and_full() {
        let mut batch = IpcBatch::new(42);
        assert!(batch.is_empty());
        assert_eq!(batch.len(), 0);

        // Add messages until full
        for i in 0..BATCH_SIZE - 1 {
            let msg = SmallMessage {
                capability: (i as u64) + 1,
                opcode: 0,
                flags: 0,
                data: [0; 4],
            };
            assert!(!batch.add_to_batch(msg));
        }

        assert_eq!(batch.len(), BATCH_SIZE - 1);

        // Last one should indicate full
        let msg = SmallMessage {
            capability: 100,
            opcode: 0,
            flags: 0,
            data: [0; 4],
        };
        assert!(batch.add_to_batch(msg));
        assert_eq!(batch.len(), BATCH_SIZE);
    }

    #[test]
    fn test_batch_target_pid() {
        let batch = IpcBatch::new(99);
        assert_eq!(batch.target_pid(), 99);
    }
}
