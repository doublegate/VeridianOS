//! IPC channel implementation for message passing
//! 
//! Provides both synchronous (blocking) and asynchronous (non-blocking)
//! communication channels between processes.

use super::{Message, SmallMessage, IpcError};
use super::error::Result;
use core::sync::atomic::{AtomicU64, AtomicBool, Ordering};
use spin::Mutex;

#[cfg(feature = "alloc")]
use alloc::collections::VecDeque;
#[cfg(feature = "alloc")]
use alloc::vec::Vec;

// TODO: Import from sched module when available
pub type ProcessId = u64;

/// Maximum number of queued messages per channel
pub const MAX_CHANNEL_QUEUE_SIZE: usize = 1024;

/// Endpoint ID generator
static ENDPOINT_COUNTER: AtomicU64 = AtomicU64::new(1);

/// IPC endpoint for bidirectional communication
pub struct Endpoint {
    /// Unique endpoint ID
    id: u64,
    /// Owner process ID
    owner: ProcessId,
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
    message: Option<Message>,
    /// Timeout in ticks (0 = infinite)
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
        if let Some(_receiver) = receivers.pop() {
            // Direct handoff to waiting receiver
            drop(receivers); // Release lock before context switch
            
            // TODO: Perform context switch to receiver
            // TODO: Copy message to receiver's buffer
            // This is where we'd achieve < 5μs latency
            
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

        // No messages available, need to wait
        let mut receivers = self.waiting_receivers.lock();
        receivers.push(WaitingProcess {
            pid: receiver,
            message: None,
            timeout: 0, // Infinite wait for now
        });
        drop(receivers);

        // TODO: Block current process and yield CPU
        // TODO: Wake up when message arrives
        
        Err(IpcError::WouldBlock) // Placeholder
    }

    /// Send without blocking
    pub fn send_async(&self, msg: Message) -> Result<()> {
        if !self.active.load(Ordering::Acquire) {
            return Err(IpcError::EndpointNotFound);
        }

        let mut queue = self.receive_queue.lock();
        if queue.len() >= MAX_CHANNEL_QUEUE_SIZE {
            return Err(IpcError::ChannelFull);
        }
        queue.push_back(msg);
        
        // TODO: Wake up any waiting receivers
        
        Ok(())
    }

    /// Try to receive without blocking
    pub fn try_receive(&self) -> Result<Message> {
        if !self.active.load(Ordering::Acquire) {
            return Err(IpcError::EndpointNotFound);
        }

        let mut queue = self.receive_queue.lock();
        queue.pop_front().ok_or(IpcError::ChannelEmpty)
    }

    /// Close the endpoint
    pub fn close(&self) {
        self.active.store(false, Ordering::Release);
        
        // TODO: Wake up all waiting processes with error
        // TODO: Clean up resources
    }
}

/// Asynchronous IPC channel
pub struct Channel {
    /// Send endpoint
    send_endpoint: Endpoint,
    /// Receive endpoint
    receive_endpoint: Endpoint,
    /// Channel capacity
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
        self.send_endpoint.send_async(msg)
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
    // TODO: Implement fast path that:
    // 1. Validates capability in O(1) time
    // 2. Directly switches to target process
    // 3. Copies registers without touching memory
    // 4. Returns immediately
    
    // Placeholder implementation
    if msg.capability == 0 {
        return Err(IpcError::InvalidCapability);
    }
    
    Ok(())
}

/// IPC call with reply (RPC-style)
pub fn call_reply(request: Message, target: ProcessId) -> Result<Message> {
    // TODO: Implement call/reply semantics
    // 1. Send request
    // 2. Block waiting for reply
    // 3. Return reply message
    
    Err(IpcError::WouldBlock) // Placeholder
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::message::SmallMessage;

    #[test]
    fn test_endpoint_creation() {
        let endpoint = Endpoint::new(1);
        assert_eq!(endpoint.owner, 1);
        assert!(endpoint.active.load(Ordering::Relaxed));
    }

    #[test]
    fn test_channel_creation() {
        let channel = Channel::new(1, 100);
        assert_ne!(channel.send_id(), channel.receive_id());
    }

    #[test]
    fn test_async_send_receive() {
        let endpoint = Endpoint::new(1);
        let msg = Message::small(0x1234, 42);
        
        assert!(endpoint.send_async(msg).is_ok());
        
        let received = endpoint.try_receive();
        assert!(received.is_ok());
        assert_eq!(received.unwrap().capability(), 0x1234);
    }
}