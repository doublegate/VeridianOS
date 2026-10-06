//! Asynchronous IPC channels with lock-free implementation
//!
//! This module provides high-performance async channels using lock-free
//! ring buffers and event notification for efficient message passing.

// Async IPC channels

#[cfg(feature = "alloc")]
extern crate alloc;

use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use super::{
    capability::ProcessId,
    error::{IpcError, Result},
    message::Message,
};
use crate::bench::read_timestamp;

/// Maximum messages in async channel
pub const ASYNC_CHANNEL_SIZE: usize = 256;

/// Lock-free ring buffer for async messages
pub struct AsyncChannel {
    /// Channel ID
    id: u64,
    /// Owner process
    #[allow(dead_code)] // Needed for ownership checks in Phase 6
    owner: ProcessId,
    /// Ring buffer for messages
    buffer: RingBuffer<Message>,
    /// Subscribers waiting for messages
    #[cfg(feature = "alloc")]
    subscribers: spin::Mutex<alloc::vec::Vec<ProcessId>>,
    /// Channel statistics
    stats: ChannelStats,
    /// Channel active flag
    active: AtomicBool,
}

/// Channel statistics
struct ChannelStats {
    messages_sent: AtomicU64,
    messages_received: AtomicU64,
    messages_dropped: AtomicU64,
    max_queue_depth: AtomicUsize,
}

impl AsyncChannel {
    /// Create a new async channel
    pub fn new(id: u64, owner: ProcessId, capacity: usize) -> Self {
        Self {
            id,
            owner,
            buffer: RingBuffer::new(capacity),
            #[cfg(feature = "alloc")]
            subscribers: spin::Mutex::new(alloc::vec::Vec::new()),
            stats: ChannelStats {
                messages_sent: AtomicU64::new(0),
                messages_received: AtomicU64::new(0),
                messages_dropped: AtomicU64::new(0),
                max_queue_depth: AtomicUsize::new(0),
            },
            active: AtomicBool::new(true),
        }
    }

    /// Send a message without blocking
    pub fn send_async(&self, msg: Message) -> Result<()> {
        if !self.active.load(Ordering::Acquire) {
            return Err(IpcError::EndpointNotFound);
        }

        // Validate capability if provided
        let cap_id = msg.capability();
        if cap_id != 0 {
            // Get current process's capability space
            if let Some(current_process) = crate::process::current_process() {
                if let Some(real_process) = crate::process::table::get_process(current_process.pid)
                {
                    let cap_space = real_process.capability_space.lock();
                    let cap_token = crate::cap::CapabilityToken::from_u64(cap_id);

                    // Check send permission
                    crate::cap::ipc_integration::check_send_permission(cap_token, &cap_space)?;
                }
            }
        }

        // Try to enqueue message
        match self.buffer.push(msg) {
            Ok(()) => {
                self.stats.messages_sent.fetch_add(1, Ordering::Relaxed);

                // Update max queue depth
                let current_size = self.buffer.size();
                let mut max_depth = self.stats.max_queue_depth.load(Ordering::Relaxed);
                while current_size > max_depth {
                    match self.stats.max_queue_depth.compare_exchange_weak(
                        max_depth,
                        current_size,
                        Ordering::Relaxed,
                        Ordering::Relaxed,
                    ) {
                        Ok(_) => break,
                        Err(old) => max_depth = old,
                    }
                }

                // Wake up subscribers
                #[cfg(feature = "alloc")]
                {
                    let subscribers = self.subscribers.lock();
                    for &pid in subscribers.iter() {
                        wake_process(pid);
                    }
                }

                // Also wake any processes blocked on this channel's endpoint
                crate::sched::ipc_blocking::wake_up_endpoint_waiters(self.id);

                Ok(())
            }
            Err(_) => {
                self.stats.messages_dropped.fetch_add(1, Ordering::Relaxed);
                Err(IpcError::ChannelFull)
            }
        }
    }

    /// Receive a message without blocking
    pub fn receive_async(&self) -> Result<Message> {
        if !self.active.load(Ordering::Acquire) {
            return Err(IpcError::EndpointNotFound);
        }

        // For receiving, we check if the caller has receive permission
        // This would typically be done at channel subscription time
        // For now, we allow receives if the process has access to the channel

        match self.buffer.pop() {
            Some(msg) => {
                self.stats.messages_received.fetch_add(1, Ordering::Relaxed);
                Ok(msg)
            }
            None => Err(IpcError::ChannelEmpty),
        }
    }

    /// Poll for messages with timeout
    pub fn poll(&self, timeout_ns: u64) -> Result<Option<Message>> {
        let start = read_timestamp();

        loop {
            // Try to receive
            match self.receive_async() {
                Ok(msg) => return Ok(Some(msg)),
                Err(IpcError::ChannelEmpty) => {
                    // Check timeout
                    if timeout_ns > 0 {
                        let elapsed = timestamp_to_ns(read_timestamp() - start);
                        if elapsed >= timeout_ns {
                            return Ok(None);
                        }
                    }

                    // Yield CPU and retry
                    core::hint::spin_loop();
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// Subscribe to channel notifications
    #[cfg(feature = "alloc")]
    pub fn subscribe(&self, pid: ProcessId) -> Result<()> {
        if !self.active.load(Ordering::Acquire) {
            return Err(IpcError::EndpointNotFound);
        }

        let mut subscribers = self.subscribers.lock();
        if !subscribers.contains(&pid) {
            subscribers.push(pid);
        }
        Ok(())
    }

    #[cfg(not(feature = "alloc"))]
    pub fn subscribe(&self, _pid: ProcessId) -> Result<()> {
        Ok(())
    }

    /// Get channel statistics
    pub fn get_stats(&self) -> AsyncChannelStats {
        AsyncChannelStats {
            messages_sent: self.stats.messages_sent.load(Ordering::Relaxed),
            messages_received: self.stats.messages_received.load(Ordering::Relaxed),
            messages_dropped: self.stats.messages_dropped.load(Ordering::Relaxed),
            max_queue_depth: self.stats.max_queue_depth.load(Ordering::Relaxed),
            current_size: self.buffer.size(),
            capacity: self.buffer.capacity(),
        }
    }

    /// Close the channel
    pub fn close(&self) {
        self.active.store(false, Ordering::Release);

        // Wake all subscribers
        #[cfg(feature = "alloc")]
        {
            let subscribers = self.subscribers.lock();
            for &pid in subscribers.iter() {
                wake_process(pid);
            }
        }
    }
}

/// Bounded FIFO shared by producers and consumers.
///
/// This was a "lock-free" ring whose producers could both pass the size
/// check and overwrite a live slot, and whose consumers could read a slot
/// before it was written (IPC-SYNC-03); `capacity == 0` divided by zero.
/// A spinlocked `VecDeque`, as `Endpoint` uses, is correct for any number of
/// producers and consumers and needs no `unsafe`.
struct RingBuffer<T> {
    items: spin::Mutex<alloc::collections::VecDeque<T>>,
    capacity: usize,
}

impl<T> RingBuffer<T> {
    /// Create a buffer holding at most `capacity` items (0 = always full).
    fn new(capacity: usize) -> Self {
        Self {
            items: spin::Mutex::new(alloc::collections::VecDeque::with_capacity(capacity)),
            capacity,
        }
    }

    /// Push an item, or give it back if the buffer is full.
    fn push(&self, item: T) -> core::result::Result<(), T> {
        let mut items = self.items.lock();
        if items.len() >= self.capacity {
            return Err(item);
        }
        items.push_back(item);
        Ok(())
    }

    /// Pop the oldest item.
    fn pop(&self) -> Option<T> {
        self.items.lock().pop_front()
    }

    /// Get current size
    fn size(&self) -> usize {
        self.items.lock().len()
    }

    /// Get capacity
    fn capacity(&self) -> usize {
        self.capacity
    }
}

/// Async channel statistics
pub struct AsyncChannelStats {
    pub messages_sent: u64,
    pub messages_received: u64,
    pub messages_dropped: u64,
    pub max_queue_depth: usize,
    pub current_size: usize,
    pub capacity: usize,
}

/// Batch message processing for efficiency
pub struct MessageBatch {
    messages: [Option<Message>; 16],
    count: usize,
}

impl MessageBatch {
    /// Create a new batch
    pub fn new() -> Self {
        Self {
            messages: core::array::from_fn(|_| None),
            count: 0,
        }
    }

    /// Add a message to the batch
    pub fn add(&mut self, msg: Message) -> bool {
        if self.count < 16 {
            self.messages[self.count] = Some(msg);
            self.count += 1;
            true
        } else {
            false
        }
    }

    /// Process the batch
    pub fn process<F>(self, mut f: F)
    where
        F: FnMut(Message),
    {
        for msg in self.messages.into_iter().take(self.count).flatten() {
            f(msg);
        }
    }
}

impl Default for MessageBatch {
    fn default() -> Self {
        Self::new()
    }
}

// Process wakeup via scheduler
fn wake_process(pid: ProcessId) {
    crate::sched::ipc_blocking::wake_up_process(pid);
}

fn timestamp_to_ns(ticks: u64) -> u64 {
    crate::bench::cycles_to_ns(ticks)
}

#[cfg(all(test, not(target_os = "none")))]
mod tests {
    use super::*;
    use crate::process::ProcessId;

    #[test]
    fn test_ring_buffer() {
        let buffer = RingBuffer::<u64>::new(4);

        // Test push/pop
        assert!(buffer.push(1).is_ok());
        assert!(buffer.push(2).is_ok());
        assert_eq!(buffer.pop(), Some(1));
        assert_eq!(buffer.pop(), Some(2));
        assert_eq!(buffer.pop(), None);
    }

    #[test]
    fn test_ring_buffer_zero_capacity_is_full() {
        let buffer = RingBuffer::<u64>::new(0);
        assert_eq!(buffer.push(1), Err(1));
        assert_eq!(buffer.pop(), None);
    }

    /// IPC-SYNC-03: with several producers and consumers, every item is
    /// delivered exactly once and none is lost or duplicated.
    #[test]
    fn test_ring_buffer_mpmc_delivers_each_item_once() {
        extern crate std;
        use std::{sync::Arc, thread, vec::Vec};

        const PER_PRODUCER: u64 = 2_000;
        let buffer = Arc::new(RingBuffer::<u64>::new(64));
        let producers: Vec<_> = (0..4u64)
            .map(|p| {
                let b = Arc::clone(&buffer);
                thread::spawn(move || {
                    for i in 0..PER_PRODUCER {
                        let mut item = p * PER_PRODUCER + i;
                        while let Err(back) = b.push(item) {
                            item = back;
                            std::thread::yield_now();
                        }
                    }
                })
            })
            .collect();
        let consumers: Vec<_> = (0..4)
            .map(|_| {
                let b = Arc::clone(&buffer);
                thread::spawn(move || {
                    let mut got = Vec::new();
                    while got.len() < PER_PRODUCER as usize {
                        match b.pop() {
                            Some(v) => got.push(v),
                            None => std::thread::yield_now(),
                        }
                    }
                    got
                })
            })
            .collect();
        for p in producers {
            p.join().unwrap();
        }
        let mut all: Vec<u64> = consumers
            .into_iter()
            .flat_map(|c| c.join().unwrap())
            .collect();
        all.sort_unstable();
        assert_eq!(all, (0..4 * PER_PRODUCER).collect::<Vec<_>>());
        assert_eq!(buffer.size(), 0);
    }

    #[test]
    fn test_async_channel() {
        let channel = AsyncChannel::new(1, ProcessId(1), 10);
        let msg = Message::small(0x1234, 42);

        // Test send/receive
        assert!(channel.send_async(msg).is_ok());
        let received = channel.receive_async();
        assert!(received.is_ok());
        assert_eq!(received.unwrap().capability(), 0x1234);
    }

    #[test]
    fn test_channel_full() {
        let channel = AsyncChannel::new(1, ProcessId(1), 2);
        let msg = Message::small(0x1234, 42);

        // Fill channel
        assert!(channel.send_async(msg.clone()).is_ok());
        assert!(channel.send_async(msg.clone()).is_ok());

        // Should be full
        assert_eq!(channel.send_async(msg), Err(IpcError::ChannelFull));
    }
}
