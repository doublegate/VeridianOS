//! Message passing implementation for IPC
//!
//! This module provides the core message passing functionality between
//! processes, including message queues, delivery, and process synchronization.

// Core message passing -- exercised via syscall IPC paths
#![allow(dead_code)]

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::{collections::BTreeMap, vec::Vec};
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
    /// Endpoints indexed by ID
    endpoints: Mutex<BTreeMap<EndpointId, Endpoint>>,
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

        endpoints.insert(endpoint.id, endpoint);
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

    /// Get an endpoint reference
    pub fn get(&self, id: EndpointId) -> Option<&'static Endpoint> {
        let endpoints = self.endpoints.lock();

        endpoints.get(&id).map(|ep| {
            // SAFETY: The Endpoint is stored in a BTreeMap behind a Mutex, so it has
            // a stable address as long as it is not removed. The returned 'static
            // reference is valid as long as the endpoint remains in the registry.
            // Callers must not hold this reference across endpoint removal.
            unsafe { &*(ep as *const Endpoint) }
        })
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
pub static ENDPOINT_REGISTRY: EndpointRegistry = EndpointRegistry::new();

/// Send a message to an endpoint
#[cfg(feature = "alloc")]
pub fn send_to_endpoint(msg: Message, endpoint_id: EndpointId) -> Result<()> {
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

        // Handle capability transfer if present
        if msg.capability() != 0 {
            if let Some(sender) = crate::process::current_process() {
                if let Some(real_sender) = crate::process::table::get_process(sender.pid) {
                    let sender_cap_space = real_sender.capability_space.lock();
                    // Transfer capability to receiver
                    if let Err(e) = crate::ipc::cap_transfer::transfer_capability(
                        &msg,
                        &sender_cap_space,
                        receiver_pid,
                    ) {
                        // Log capability transfer failure but don't fail the message send
                        #[cfg(target_arch = "x86_64")]
                        println!("[IPC] Capability transfer failed: {:?}", e);
                        #[cfg(not(target_arch = "x86_64"))]
                        let _ = e;
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

        // Block the current process
        sched::block_on_ipc(endpoint_id);

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
