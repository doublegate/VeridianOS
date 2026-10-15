//! Message passing implementation for IPC
//!
//! This module provides the core message passing functionality between
//! processes, including message queues, delivery, and process synchronization.

// Core message passing
#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::{collections::BTreeMap, sync::Arc, vec::Vec};
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use spin::Mutex;

use super::{
    capability::EndpointId,
    error::{IpcError, Result},
    message::Message,
};
use crate::{process::ProcessId, sched};

/// Message queue for each endpoint
#[cfg(feature = "alloc")]
pub struct MessageQueue {
    /// Queued messages
    messages: Vec<Message>,
    /// Maximum queue size
    max_size: usize,
    /// Processes waiting to receive
    waiting_receivers: Vec<ProcessId>,
    /// Processes waiting to send (when queue is full)
    waiting_senders: Vec<(ProcessId, Message)>,
}

#[cfg(feature = "alloc")]
impl MessageQueue {
    /// Create a new message queue
    pub fn new(max_size: usize) -> Self {
        Self {
            messages: Vec::new(),
            max_size,
            waiting_receivers: Vec::new(),
            waiting_senders: Vec::new(),
        }
    }

    /// Enqueue a message
    pub fn enqueue(&mut self, msg: Message) -> Result<()> {
        if self.messages.len() >= self.max_size {
            return Err(IpcError::ChannelFull);
        }
        self.messages.push(msg);
        Ok(())
    }

    /// Dequeue a message
    pub fn dequeue(&mut self) -> Option<Message> {
        if self.messages.is_empty() {
            None
        } else {
            Some(self.messages.remove(0))
        }
    }

    /// Add a waiting receiver
    pub fn add_receiver(&mut self, pid: ProcessId) {
        self.waiting_receivers.push(pid);
    }

    /// Get a waiting receiver
    pub fn get_receiver(&mut self) -> Option<ProcessId> {
        if self.waiting_receivers.is_empty() {
            None
        } else {
            Some(self.waiting_receivers.remove(0))
        }
    }

    /// Add a waiting sender
    pub fn add_sender(&mut self, pid: ProcessId, msg: Message) {
        self.waiting_senders.push((pid, msg));
    }

    /// Process waiting senders (when space becomes available)
    pub fn process_waiting_senders(&mut self) -> Vec<ProcessId> {
        let mut woken = Vec::new();

        while self.messages.len() < self.max_size && !self.waiting_senders.is_empty() {
            let (pid, msg) = self.waiting_senders.remove(0);
            self.messages.push(msg);
            woken.push(pid);
        }

        woken
    }

    /// Check if queue has messages
    pub fn has_messages(&self) -> bool {
        !self.messages.is_empty()
    }

    /// Check if there are waiting receivers
    pub fn has_waiting_receivers(&self) -> bool {
        !self.waiting_receivers.is_empty()
    }
}

/// Endpoint structure with message queue
pub struct Endpoint {
    /// Endpoint ID
    pub id: EndpointId,
    /// Owner process
    pub owner: ProcessId,
    /// Message queue
    #[cfg(feature = "alloc")]
    pub queue: Mutex<MessageQueue>,
    /// Active flag
    pub active: AtomicBool,
    /// Message counter
    pub message_count: AtomicU64,
}

impl Endpoint {
    /// Create a new endpoint
    #[cfg(feature = "alloc")]
    pub fn new(id: EndpointId, owner: ProcessId) -> Self {
        Self {
            id,
            owner,
            queue: Mutex::new(MessageQueue::new(1024)), // Default queue size
            active: AtomicBool::new(true),
            message_count: AtomicU64::new(0),
        }
    }

    /// Close the endpoint
    pub fn close(&self) {
        self.active.store(false, Ordering::Release);
    }

    /// Check if endpoint is active
    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::Acquire)
    }
}

/// Global endpoint registry
#[cfg(feature = "alloc")]
pub struct EndpointRegistry {
    /// Endpoints indexed by ID. Shared (`Arc`), so a sender or receiver
    /// keeps its endpoint alive while it uses it (N-83): the map used to
    /// hold endpoints by value and hand out `&'static` references, which
    /// dangled as soon as the map moved its entries (any insertion can) or
    /// the endpoint was unregistered.
    endpoints: Mutex<BTreeMap<EndpointId, Arc<Endpoint>>>,
}

#[cfg(feature = "alloc")]
impl EndpointRegistry {
    /// Create a new registry
    pub const fn new() -> Self {
        Self {
            endpoints: Mutex::new(BTreeMap::new()),
        }
    }

    /// Register an endpoint
    pub fn register(&self, endpoint: Endpoint) -> Result<()> {
        let mut endpoints = self.endpoints.lock();

        if endpoints.contains_key(&endpoint.id) {
            return Err(IpcError::EndpointBusy);
        }

        endpoints.insert(endpoint.id, Arc::new(endpoint));
        Ok(())
    }

    /// Unregister an endpoint
    pub fn unregister(&self, id: EndpointId) -> Result<()> {
        let mut endpoints = self.endpoints.lock();

        if let Some(endpoint) = endpoints.remove(&id) {
            endpoint.close();

            // Wake up any waiting processes
            let queue = endpoint.queue.lock();
            let waiting = queue.waiting_receivers.clone();
            drop(queue);

            for pid in waiting {
                sched::wake_up_process(pid);
            }

            Ok(())
        } else {
            Err(IpcError::EndpointNotFound)
        }
    }

    /// Get an endpoint. It stays valid after it is unregistered (closed
    /// then, so sends and receives on it fail).
    pub fn get(&self, id: EndpointId) -> Option<Arc<Endpoint>> {
        self.endpoints.lock().get(&id).cloned()
    }
}

#[cfg(feature = "alloc")]
impl Default for EndpointRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Global endpoint registry instance
#[cfg(feature = "alloc")]
pub(crate) static ENDPOINT_REGISTRY: EndpointRegistry = EndpointRegistry::new();

/// Send a message to an endpoint
#[cfg(feature = "alloc")]
pub fn send_to_endpoint(mut msg: Message, endpoint_id: EndpointId) -> Result<()> {
    let endpoint = ENDPOINT_REGISTRY
        .get(endpoint_id)
        .ok_or(IpcError::EndpointNotFound)?;

    if !endpoint.is_active() {
        return Err(IpcError::EndpointNotFound);
    }

    let mut queue = endpoint.queue.lock();

    // Check if there's a waiting receiver
    if let Some(receiver_pid) = queue.get_receiver() {
        // Direct delivery
        drop(queue);

        // Transfer a capability only when the sender asks for it. The field
        // otherwise holds the sender's own (validated) endpoint capability,
        // which used to be delegated to the receiver on every message
        // (N-61).
        if msg.capability() != 0 && msg.flags() & crate::ipc::message::flags::HAS_CAPABILITY != 0 {
            if let Some(sender) = crate::process::current_process() {
                if let Some(real_sender) = crate::process::table::get_process(sender.pid) {
                    let sender_cap_space = real_sender.capability_space.lock();
                    // Transfer capability to receiver. A delegation is a
                    // capability of its own (CAP-INC-02), so the receiver
                    // gets its token, not the sender's.
                    match crate::ipc::cap_transfer::transfer_capability(
                        &msg,
                        &sender_cap_space,
                        receiver_pid,
                    ) {
                        Ok(Some(delegated)) => msg.set_capability(delegated.to_u64()),
                        Ok(None) => {}
                        Err(e) => {
                            // Log capability transfer failure but don't fail the message send
                            #[cfg(target_arch = "x86_64")]
                            println!("[IPC] Capability transfer failed: {:?}", e);
                            #[cfg(not(target_arch = "x86_64"))]
                            let _ = e;
                        }
                    }
                }
            }
        }

        deliver_to_process(msg, receiver_pid)?;
        sched::wake_up_process(receiver_pid);
        endpoint.message_count.fetch_add(1, Ordering::Relaxed);
        Ok(())
    } else {
        // Queue the message
        match queue.enqueue(msg) {
            Ok(()) => {
                endpoint.message_count.fetch_add(1, Ordering::Relaxed);
                // Note: Capability transfer happens when message is dequeued
                Ok(())
            }
            Err(e) => Err(e),
        }
    }
}

/// Deliver an authorized reply to `caller`'s reply endpoint, the endpoint
/// numbered by its pid. The endpoint must belong to `caller`: endpoint
/// numbers and pids share one namespace, so another process's endpoint
/// with that number must never receive it.
#[cfg(feature = "alloc")]
pub fn send_reply(msg: Message, caller: u64) -> Result<()> {
    let endpoint = ENDPOINT_REGISTRY
        .get(caller)
        .ok_or(IpcError::EndpointNotFound)?;
    if endpoint.owner != ProcessId(caller) {
        return Err(IpcError::PermissionDenied);
    }
    send_to_endpoint(msg, caller)
}

/// Receive a message from an endpoint
#[cfg(feature = "alloc")]
pub fn receive_from_endpoint(endpoint_id: EndpointId, blocking: bool) -> Result<Message> {
    let endpoint = ENDPOINT_REGISTRY
        .get(endpoint_id)
        .ok_or(IpcError::EndpointNotFound)?;

    if !endpoint.is_active() {
        return Err(IpcError::EndpointNotFound);
    }

    let mut queue = endpoint.queue.lock();

    // Try to get a message
    if let Some(msg) = queue.dequeue() {
        // Process any waiting senders
        let woken_senders = queue.process_waiting_senders();
        drop(queue);

        // Wake up senders that were waiting for space
        for pid in woken_senders {
            sched::wake_up_process(pid);
        }

        Ok(msg)
    } else if blocking {
        // No message, block if requested
        let current_pid = sched::current_process().pid;
        queue.add_receiver(current_pid);
        drop(queue);

        // Block the current process, unless a message arrived while we were
        // registering (IPC-SYNC-01: the queue lock is already released).
        crate::sched::ipc_blocking::block_on_ipc_unless(endpoint_id, || {
            endpoint.queue.lock().has_messages()
        });

        // When we wake up, try again
        receive_from_endpoint(endpoint_id, false)
    } else {
        Err(IpcError::WouldBlock)
    }
}

/// Deliver a message directly to a process
#[cfg(feature = "alloc")]
fn deliver_to_process(msg: Message, pid: ProcessId) -> Result<()> {
    // Store message in per-process message buffer
    static PROCESS_MESSAGES: Mutex<BTreeMap<ProcessId, Message>> = Mutex::new(BTreeMap::new());

    let mut messages = PROCESS_MESSAGES.lock();
    messages.insert(pid, msg);

    Ok(())
}

/// Retrieve a delivered message for the current process
#[cfg(feature = "alloc")]
pub fn retrieve_delivered_message() -> Option<Message> {
    static PROCESS_MESSAGES: Mutex<BTreeMap<ProcessId, Message>> = Mutex::new(BTreeMap::new());

    let current_pid = sched::current_process().pid;
    let mut messages = PROCESS_MESSAGES.lock();

    if let Some(msg) = messages.remove(&current_pid) {
        // Validate any capability in the message
        if msg.capability() != 0 {
            if let Some(current) = crate::process::current_process() {
                if let Some(real_process) = crate::process::table::get_process(current.pid) {
                    let cap_space = real_process.capability_space.lock();
                    let cap_token = crate::cap::CapabilityToken::from_u64(msg.capability());

                    // Verify the capability exists in receiver's space
                    if cap_space.lookup(cap_token).is_none() {
                        println!(
                            "[IPC] Warning: Capability {} not found in receiver's space",
                            msg.capability()
                        );
                        // Still deliver the message but without the capability
                    }
                }
            }
        }
        Some(msg)
    } else {
        None
    }
}

/// Initialize message passing subsystem
pub fn init() {
    println!("[IPC] Message passing subsystem initialized");
}

#[cfg(all(test, feature = "alloc"))]
mod tests {
    use super::*;

    /// N-83: an endpoint obtained from the registry stays valid while the
    /// registry grows (a by-value map moves its entries) and after it is
    /// unregistered, when it reads as closed.
    #[test]
    fn registry_endpoints_outlive_map_changes_and_removal() {
        let registry = EndpointRegistry::new();
        registry.register(Endpoint::new(1, ProcessId(5))).unwrap();
        let first = registry.get(1).unwrap();
        for id in 2..500 {
            registry.register(Endpoint::new(id, ProcessId(5))).unwrap();
        }
        assert_eq!((first.id, first.owner), (1, ProcessId(5)));
        assert!(first.is_active());
        registry.unregister(1).unwrap();
        assert_eq!(first.id, 1, "still readable after removal");
        assert!(!first.is_active(), "and closed");
        assert!(registry.get(1).is_none());
        assert!(matches!(
            registry.register(Endpoint::new(7, ProcessId(5))),
            Err(IpcError::EndpointBusy)
        ));
    }

    /// A reply for pid 9 goes only to an endpoint 9 that pid 9 owns.
    #[test]
    fn replies_go_only_to_the_callers_own_endpoint() {
        ENDPOINT_REGISTRY
            .register(Endpoint::new(0x9_0009, ProcessId(0x9_0001)))
            .unwrap();
        let msg = Message::Small(crate::ipc::SmallMessage::new(0, 0));
        assert!(matches!(
            send_reply(msg, 0x9_0009),
            Err(IpcError::PermissionDenied)
        ));
        let msg = Message::Small(crate::ipc::SmallMessage::new(0, 0));
        assert!(matches!(
            send_reply(msg, 0x9_000A),
            Err(IpcError::EndpointNotFound)
        ));
        ENDPOINT_REGISTRY.unregister(0x9_0009).unwrap();
    }
}
