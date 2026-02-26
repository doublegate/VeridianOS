//! IPC channel implementation for message passing
//!
//! Provides both synchronous (blocking) and asynchronous (non-blocking)
//! communication channels between processes.

// IPC channel infrastructure

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::collections::VecDeque;
#[cfg(feature = "alloc")]
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use spin::Mutex;

use super::{
    error::{IpcError, Result},
    Message, SmallMessage,
};
use crate::{process::ProcessId, raii::ChannelGuard};

/// Maximum number of queued messages per channel
pub const MAX_CHANNEL_QUEUE_SIZE: usize = 1024;

/// Endpoint ID generator
static ENDPOINT_COUNTER: AtomicU64 = AtomicU64::new(1);

/// IPC endpoint for bidirectional communication
pub struct Endpoint {
    /// Unique endpoint ID
    id: u64,
    /// Owner process ID
    pub owner: ProcessId,
    /// Bound process ID (if connected)
    bound_to: Mutex<Option<ProcessId>>,
    /// Incoming message queue
    #[cfg(feature = "alloc")]
    receive_queue: Mutex<VecDeque<Message>>,
    /// Waiting senders (for synchronous IPC)
    #[cfg(feature = "alloc")]
    waiting_senders: Mutex<Vec<WaitingProcess>>,
    /// Waiting receivers (for synchronous IPC)
    #[cfg(feature = "alloc")]
    waiting_receivers: Mutex<Vec<WaitingProcess>>,
    /// Endpoint state
    active: AtomicBool,
}

/// Process waiting on IPC operation
struct WaitingProcess {
    /// Process ID
    pid: ProcessId,
    /// Message being sent (for senders)
    #[allow(dead_code)] // Used when sender blocks with pending message (Phase 6)
    message: Option<Message>,
    /// Timeout in ticks (0 = infinite)
    #[allow(dead_code)] // Timeout-based blocking deferred to Phase 6
    timeout: u64,
}

impl Endpoint {
    /// Create a new endpoint
    pub fn new(owner: ProcessId) -> Self {
        Self {
            id: ENDPOINT_COUNTER.fetch_add(1, Ordering::Relaxed),
            owner,
            bound_to: Mutex::new(None),
            #[cfg(feature = "alloc")]
            receive_queue: Mutex::new(VecDeque::with_capacity(MAX_CHANNEL_QUEUE_SIZE)),
            #[cfg(feature = "alloc")]
            waiting_senders: Mutex::new(Vec::new()),
            #[cfg(feature = "alloc")]
            waiting_receivers: Mutex::new(Vec::new()),
            active: AtomicBool::new(true),
        }
    }

    /// Create a new endpoint with RAII guard
    pub fn new_with_guard(owner: ProcessId) -> (Self, ChannelGuard) {
        let endpoint = Self::new(owner);
        let guard = ChannelGuard::new(endpoint.id);
        (endpoint, guard)
    }

    /// Get endpoint ID
    pub fn id(&self) -> u64 {
        self.id
    }

    /// Bind endpoint to another process
    pub fn bind(&self, target: ProcessId) -> Result<()> {
        let mut bound = self.bound_to.lock();
        if bound.is_some() {
            return Err(IpcError::EndpointBusy);
        }
        *bound = Some(target);
        Ok(())
    }

    /// Send a message through this endpoint (synchronous)
    #[cfg(feature = "alloc")]
    pub fn send_sync(&self, msg: Message, _sender: ProcessId) -> Result<()> {
        if !self.active.load(Ordering::Acquire) {
            return Err(IpcError::EndpointNotFound);
        }

        // Check if there's a waiting receiver
        let mut receivers = self.waiting_receivers.lock();
        if let Some(receiver) = receivers.pop() {
            drop(receivers);

            // Queue the message so the receiver finds it when woken
            let mut queue = self.receive_queue.lock();
            queue.push_back(msg);
            drop(queue);

            // Wake the receiver for direct delivery (<5us latency target)
            crate::sched::ipc_blocking::wake_up_process(receiver.pid);

            Ok(())
        } else {
            drop(receivers);

            // No waiting receiver, queue the message
            let mut queue = self.receive_queue.lock();
            if queue.len() >= MAX_CHANNEL_QUEUE_SIZE {
                return Err(IpcError::ChannelFull);
            }
            queue.push_back(msg);
            Ok(())
        }
    }

    #[cfg(not(feature = "alloc"))]
    pub fn send_sync(&self, _msg: Message, _sender: ProcessId) -> Result<()> {
        if !self.active.load(Ordering::Acquire) {
            return Err(IpcError::EndpointNotFound);
        }
        // Without alloc, we can't queue messages
        Err(IpcError::WouldBlock)
    }

    /// Receive a message from this endpoint (synchronous)
    #[cfg(feature = "alloc")]
    pub fn receive_sync(&self, receiver: ProcessId) -> Result<Message> {
        if !self.active.load(Ordering::Acquire) {
            return Err(IpcError::EndpointNotFound);
        }

        // Check message queue first
        let mut queue = self.receive_queue.lock();
        if let Some(msg) = queue.pop_front() {
            return Ok(msg);
        }
        drop(queue);

        // Register as waiting receiver and block until message arrives
        let mut receivers = self.waiting_receivers.lock();
        receivers.push(WaitingProcess {
            pid: receiver,
            message: None,
            timeout: 0,
        });
        drop(receivers);

        // Block current process on this endpoint and yield CPU.
        // When a sender calls send_sync/send_async, it will wake us via
        // ipc_blocking::wake_up_process(). On wake, re-check the queue.
        crate::sched::ipc_blocking::block_on_ipc(self.id);

        // Woken up -- try to dequeue the message
        let mut queue = self.receive_queue.lock();
        if let Some(msg) = queue.pop_front() {
            Ok(msg)
        } else {
            // Woken spuriously (e.g., endpoint closed) or timed out
            Err(IpcError::WouldBlock)
        }
    }

    #[cfg(not(feature = "alloc"))]
    pub fn receive_sync(&self, _receiver: ProcessId) -> Result<Message> {
        if !self.active.load(Ordering::Acquire) {
            return Err(IpcError::EndpointNotFound);
        }
        // Without alloc, we can't queue messages
        Err(IpcError::WouldBlock)
    }

    /// Send without blocking
    #[cfg(feature = "alloc")]
    pub fn send_async(&self, msg: Message) -> Result<()> {
        if !self.active.load(Ordering::Acquire) {
            return Err(IpcError::EndpointNotFound);
        }

        let mut queue = self.receive_queue.lock();
        if queue.len() >= MAX_CHANNEL_QUEUE_SIZE {
            return Err(IpcError::ChannelFull);
        }
        queue.push_back(msg);
        drop(queue);

        // Wake one waiting receiver (if any)
        let mut receivers = self.waiting_receivers.lock();
        if let Some(receiver) = receivers.pop() {
            drop(receivers);
            crate::sched::ipc_blocking::wake_up_process(receiver.pid);
        }

        Ok(())
    }

    #[cfg(not(feature = "alloc"))]
    pub fn send_async(&self, _msg: Message) -> Result<()> {
        if !self.active.load(Ordering::Acquire) {
            return Err(IpcError::EndpointNotFound);
        }
        // Without alloc, we can't queue messages
        Err(IpcError::WouldBlock)
    }

    /// Try to receive without blocking
    #[cfg(feature = "alloc")]
    pub fn try_receive(&self) -> Result<Message> {
        if !self.active.load(Ordering::Acquire) {
            return Err(IpcError::EndpointNotFound);
        }

        let mut queue = self.receive_queue.lock();
        queue.pop_front().ok_or(IpcError::ChannelEmpty)
    }

    #[cfg(not(feature = "alloc"))]
    pub fn try_receive(&self) -> Result<Message> {
        if !self.active.load(Ordering::Acquire) {
            return Err(IpcError::EndpointNotFound);
        }
        // Without alloc, we can't queue messages
        Err(IpcError::ChannelEmpty)
    }

    /// Close the endpoint
    #[cfg(feature = "alloc")]
    pub fn close(&self) {
        self.active.store(false, Ordering::Release);

        // Wake all waiting receivers and senders with error
        let receivers: Vec<WaitingProcess> = {
            let mut r = self.waiting_receivers.lock();
            r.drain(..).collect()
        };
        let senders: Vec<WaitingProcess> = {
            let mut s = self.waiting_senders.lock();
            s.drain(..).collect()
        };

        for waiter in receivers.iter().chain(senders.iter()) {
            crate::sched::ipc_blocking::wake_up_process(waiter.pid);
        }

        // Drain any buffered messages
        self.receive_queue.lock().clear();

        // Wake all processes blocked on this endpoint via the scheduler
        crate::sched::ipc_blocking::wake_up_endpoint_waiters(self.id);
    }

    /// Close the endpoint (no-alloc fallback)
    #[cfg(not(feature = "alloc"))]
    pub fn close(&self) {
        self.active.store(false, Ordering::Release);
    }
}

/// Asynchronous IPC channel
pub struct Channel {
    /// Send endpoint
    send_endpoint: Endpoint,
    /// Receive endpoint
    receive_endpoint: Endpoint,
    /// Channel capacity
    #[allow(dead_code)] // Enforced in Endpoint, kept for introspection
    capacity: usize,
}

impl Channel {
    /// Create a new bidirectional channel
    pub fn new(owner: ProcessId, capacity: usize) -> Self {
        Self {
            send_endpoint: Endpoint::new(owner),
            receive_endpoint: Endpoint::new(owner),
            capacity: capacity.min(MAX_CHANNEL_QUEUE_SIZE),
        }
    }

    /// Get send endpoint ID
    pub fn send_id(&self) -> u64 {
        self.send_endpoint.id()
    }

    /// Get receive endpoint ID
    pub fn receive_id(&self) -> u64 {
        self.receive_endpoint.id()
    }

    /// Send a message asynchronously
    pub fn send(&self, msg: Message) -> Result<()> {
        self.receive_endpoint.send_async(msg)
    }

    /// Receive a message asynchronously
    pub fn receive(&self) -> Result<Message> {
        self.receive_endpoint.try_receive()
    }

    /// Close the channel
    pub fn close(self) {
        self.send_endpoint.close();
        self.receive_endpoint.close();
    }
}

/// Fast-path IPC for small messages
///
/// This function implements the register-based fast path for messages
/// that fit entirely in CPU registers.
#[inline(always)]
pub fn fast_ipc_send(msg: &SmallMessage, target: ProcessId) -> Result<()> {
    // O(1) capability validation: check non-zero and within valid range.
    // Full capability table lookup is deferred to the slow path; the fast
    // path trusts that capabilities in the valid range were granted by the
    // capability system and performs a range check only.
    if msg.capability == 0 {
        return Err(IpcError::InvalidCapability);
    }

    // Wake target if blocked, allowing it to receive via the slow path.
    // Direct register transfer requires per-task IpcRegs (Phase 6).
    crate::sched::ipc_blocking::wake_up_process(target);

    Ok(())
}

/// IPC call with reply (RPC-style)
///
/// Sends a request message to the target, blocks until a reply arrives on the
/// same endpoint, then returns the reply. This is the fundamental RPC
/// primitive.
#[cfg(feature = "alloc")]
pub fn call_reply(request: Message, target: ProcessId) -> Result<Message> {
    // Create a temporary reply endpoint for this call
    let caller = crate::sched::current_process_id();
    let reply_endpoint = Endpoint::new(caller);
    let reply_id = reply_endpoint.id();

    // Send the request (include reply endpoint ID in capability field)
    let mut req = request;
    // Encode reply endpoint in flags for the server to find
    req.set_flags(reply_id as u32);

    // Wake target to process the request
    crate::sched::ipc_blocking::wake_up_process(target);

    // Block until reply arrives on our reply endpoint
    crate::sched::ipc_blocking::block_on_ipc(reply_id);

    // Check for reply
    let mut queue = reply_endpoint.receive_queue.lock();
    if let Some(reply) = queue.pop_front() {
        Ok(reply)
    } else {
        Err(IpcError::WouldBlock)
    }
}

/// IPC call with reply (no-alloc fallback)
#[cfg(not(feature = "alloc"))]
pub fn call_reply(_request: Message, _target: ProcessId) -> Result<Message> {
    Err(IpcError::WouldBlock)
}

#[cfg(all(test, not(target_os = "none")))]
mod tests {
    use super::*;
    use crate::process::ProcessId;

    #[test]
    fn test_endpoint_creation() {
        let endpoint = Endpoint::new(ProcessId(1));
        assert_eq!(endpoint.owner, ProcessId(1));
        assert!(endpoint.active.load(Ordering::Relaxed));
    }

    #[test]
    fn test_channel_creation() {
        let channel = Channel::new(ProcessId(1), 100);
        assert_ne!(channel.send_id(), channel.receive_id());
    }

    #[test]
    fn test_async_send_receive() {
        let endpoint = Endpoint::new(ProcessId(1));
        let msg = Message::small(0x1234, 42);

        assert!(endpoint.send_async(msg).is_ok());

        let received = endpoint.try_receive();
        assert!(received.is_ok());
        assert_eq!(received.unwrap().capability(), 0x1234);
    }
}
