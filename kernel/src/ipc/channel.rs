//! IPC channel implementation for message passing
//!
//! Provides both synchronous (blocking) and asynchronous (non-blocking)
//! communication channels between processes.

// IPC channel infrastructure

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::collections::VecDeque;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use spin::Mutex;

use super::{
    error::{IpcError, Result},
    Message,
};
use crate::{process::ProcessId, raii::ChannelGuard};

/// Maximum number of queued messages per channel
pub const MAX_CHANNEL_QUEUE_SIZE: usize = 1024;

/// Endpoint ID generator
static ENDPOINT_COUNTER: AtomicU64 = AtomicU64::new(1);

/// IPC endpoint for bidirectional communication
/// Which call a queued request starts: the calling thread and the call's
/// number, so a server's reply can be matched to that call and no later
/// one (PR #24 review).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CallTag {
    pub caller: u64,
    pub id: u64,
}

pub struct Endpoint {
    /// Unique endpoint ID
    id: u64,
    /// Owner process ID
    pub owner: ProcessId,
    /// Bound process ID (if connected)
    bound_to: Mutex<Option<ProcessId>>,
    /// Incoming message queue, each with the call it starts if it is a
    /// `sync_call` request.
    #[cfg(feature = "alloc")]
    receive_queue: Mutex<VecDeque<(Message, Option<CallTag>)>>,
    /// Receivers waiting for a message and senders waiting for room, woken
    /// whenever the queue changes or the endpoint closes. They used to be
    /// pid lists for the old scheduler, whose blocking never blocked under
    /// the dispatcher (N-84, N-85).
    #[cfg(feature = "alloc")]
    waiters: crate::sched::dispatch::WaitQueue,
    /// Endpoint state
    active: AtomicBool,
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
            waiters: crate::sched::dispatch::WaitQueue::new(),
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

    /// Send a message through this endpoint (synchronous): queue it,
    /// waiting for room while the queue is full. A signal ends the wait
    /// (`Interrupted`); a closed endpoint is `EndpointNotFound`.
    #[cfg(feature = "alloc")]
    pub fn send_sync(&self, msg: Message, _sender: ProcessId) -> Result<()> {
        self.send_tagged(msg, None)
    }

    /// [`Self::send_sync`] for a call's request, tagged with the call.
    #[cfg(feature = "alloc")]
    pub fn send_tagged(&self, msg: Message, tag: Option<CallTag>) -> Result<()> {
        let mut msg = Some(msg);
        loop {
            if !self.active.load(Ordering::Acquire) {
                return Err(IpcError::EndpointNotFound);
            }
            {
                let mut queue = self.receive_queue.lock();
                if queue.len() < MAX_CHANNEL_QUEUE_SIZE {
                    if let Some(msg) = msg.take() {
                        queue.push_back((msg, tag));
                    }
                    drop(queue);
                    self.waiters.wake_all();
                    return Ok(());
                }
            }
            self.wait(|| self.receive_queue.lock().len() < MAX_CHANNEL_QUEUE_SIZE)?;
        }
    }

    /// Sleep until `ready` holds or the endpoint closes; a signal ends the
    /// wait.
    #[cfg(feature = "alloc")]
    fn wait(&self, ready: impl Fn() -> bool) -> Result<()> {
        crate::sched::dispatch::wait_event(&self.waiters, None, || {
            ready() || !self.active.load(Ordering::Acquire)
        })
        .map_err(|_| IpcError::Interrupted)
    }

    #[cfg(not(feature = "alloc"))]
    pub fn send_sync(&self, _msg: Message, _sender: ProcessId) -> Result<()> {
        if !self.active.load(Ordering::Acquire) {
            return Err(IpcError::EndpointNotFound);
        }
        // Without alloc, we can't queue messages
        Err(IpcError::WouldBlock)
    }

    /// Receive a message from this endpoint (synchronous), sleeping until
    /// one arrives. A signal ends the wait (`Interrupted`); a closed
    /// endpoint is `EndpointNotFound`.
    #[cfg(feature = "alloc")]
    pub fn receive_sync(&self, _receiver: ProcessId) -> Result<Message> {
        self.receive_tagged().map(|(msg, _)| msg)
    }

    /// [`Self::receive_sync`], with the call the message starts, if any.
    #[cfg(feature = "alloc")]
    pub fn receive_tagged(&self) -> Result<(Message, Option<CallTag>)> {
        loop {
            if !self.active.load(Ordering::Acquire) {
                return Err(IpcError::EndpointNotFound);
            }
            if let Some(entry) = self.receive_queue.lock().pop_front() {
                // A sender may be waiting for this room.
                self.waiters.wake_all();
                return Ok(entry);
            }
            self.wait(|| !self.receive_queue.lock().is_empty())?;
        }
    }

    /// Take back the request of call `tag` if no server has received it
    /// yet (its caller gave up). Returns whether it was still queued.
    #[cfg(feature = "alloc")]
    pub fn withdraw(&self, tag: CallTag) -> bool {
        let mut queue = self.receive_queue.lock();
        let Some(at) = queue.iter().position(|(_, t)| *t == Some(tag)) else {
            return false;
        };
        queue.remove(at);
        drop(queue);
        self.waiters.wake_all();
        true
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
        queue.push_back((msg, None));
        drop(queue);
        self.waiters.wake_all();
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

        let msg = self.receive_queue.lock().pop_front().map(|(msg, _)| msg);
        if msg.is_some() {
            self.waiters.wake_all(); // room for a waiting sender
        }
        msg.ok_or(IpcError::ChannelEmpty)
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
        // Drain any buffered messages; waiters see the endpoint closed.
        self.receive_queue.lock().clear();
        self.waiters.wake_all();
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

    /// A call's request carries the call to the server, and one whose
    /// caller gave up can be taken back while no server has it.
    #[test]
    fn call_requests_are_tagged_and_can_be_withdrawn() {
        let endpoint = Endpoint::new(ProcessId(1));
        let a = CallTag { caller: 40, id: 1 };
        let b = CallTag { caller: 40, id: 2 };
        endpoint.send_tagged(Message::small(1, 1), Some(a)).unwrap();
        endpoint.send_tagged(Message::small(2, 1), Some(b)).unwrap();
        assert!(endpoint.withdraw(a));
        assert!(!endpoint.withdraw(a));
        let (msg, tag) = endpoint.receive_tagged().unwrap();
        assert_eq!((msg.capability(), tag), (2, Some(b)));
    }

    #[test]
    fn receive_sync_takes_a_queued_message_without_waiting() {
        let endpoint = Endpoint::new(ProcessId(1));
        endpoint
            .send_sync(Message::small(7, 1), ProcessId(2))
            .unwrap();
        assert_eq!(endpoint.receive_sync(ProcessId(1)).unwrap().capability(), 7);
    }

    #[test]
    fn closed_endpoint_refuses_both_sides_and_drops_messages() {
        let endpoint = Endpoint::new(ProcessId(1));
        endpoint.send_async(Message::small(7, 1)).unwrap();
        endpoint.close();
        assert_eq!(
            endpoint.receive_sync(ProcessId(1)).err(),
            Some(IpcError::EndpointNotFound)
        );
        assert_eq!(
            endpoint.send_sync(Message::small(8, 1), ProcessId(2)).err(),
            Some(IpcError::EndpointNotFound)
        );
        assert_eq!(
            endpoint.try_receive().err(),
            Some(IpcError::EndpointNotFound)
        );
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
