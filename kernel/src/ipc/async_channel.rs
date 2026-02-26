//! Asynchronous IPC channels with lock-free implementation
//!
//! This module provides high-performance async channels using lock-free
//! ring buffers and event notification for efficient message passing.

// Async IPC channels

#[cfg(feature = "alloc")]
extern crate alloc;

use core::{
    ptr,
    sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};

use super::{
    capability::ProcessId,
    error::{IpcError, Result},
    message::Message,
};
use crate::arch::entropy::read_timestamp;

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

/// Lock-free ring buffer implementation
struct RingBuffer<T> {
    /// Buffer storage
    buffer: *mut T,
    /// Buffer capacity
    capacity: usize,
    /// Write position
    write_pos: AtomicUsize,
    /// Read position
    read_pos: AtomicUsize,
    /// Number of items in buffer
    size: AtomicUsize,
}

impl<T> RingBuffer<T> {
    /// Create a new ring buffer
    fn new(capacity: usize) -> Self {
        let layout =
            core::alloc::Layout::array::<T>(capacity).expect("ring buffer capacity overflow");
        // SAFETY: The layout is computed from Layout::array::<T>(capacity) which
        // ensures proper size and alignment for an array of T elements. The alloc()
        // call returns a pointer to uninitialized memory of the requested layout.
        // If allocation fails (returns null), subsequent push/pop operations will
        // cause undefined behavior -- in a production kernel, this should be checked.
        // The returned pointer is cast to *mut T, which is valid because the layout
        // guarantees correct alignment for T.
        let buffer = unsafe { alloc::alloc::alloc(layout) as *mut T };

        Self {
            buffer,
            capacity,
            write_pos: AtomicUsize::new(0),
            read_pos: AtomicUsize::new(0),
            size: AtomicUsize::new(0),
        }
    }

    /// Push an item into the buffer
    fn push(&self, item: T) -> core::result::Result<(), T> {
        let current_size = self.size.load(Ordering::Acquire);
        if current_size >= self.capacity {
            return Err(item);
        }

        // Reserve a slot
        let write_pos = self.write_pos.fetch_add(1, Ordering::Relaxed) % self.capacity;

        // SAFETY: `write_pos` is computed modulo `self.capacity`, so it is always
        // within bounds of the allocated buffer (0..capacity-1). The size check above
        // ensures we are not writing past the buffer's logical capacity. `ptr::write`
        // is used instead of assignment because the slot may contain uninitialized
        // memory (never written) or previously-read memory (already consumed by pop).
        // In either case, we must not run Drop on the old value, which ptr::write
        // avoids. The buffer pointer is valid because it was allocated in `new()`.
        unsafe {
            ptr::write(self.buffer.add(write_pos), item);
        }

        // Update size
        self.size.fetch_add(1, Ordering::Release);

        Ok(())
    }

    /// Pop an item from the buffer
    fn pop(&self) -> Option<T> {
        loop {
            let current_size = self.size.load(Ordering::Acquire);
            if current_size == 0 {
                return None;
            }

            // Try to decrement size
            match self.size.compare_exchange_weak(
                current_size,
                current_size - 1,
                Ordering::Acquire,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    // Successfully reserved an item
                    let read_pos = self.read_pos.fetch_add(1, Ordering::Relaxed) % self.capacity;

                    // SAFETY: `read_pos` is computed modulo `self.capacity`, so it is
                    // within bounds. The compare_exchange above successfully decremented
                    // the size, guaranteeing a valid item exists at this slot (placed by
                    // a prior push()). `ptr::read` is used to move the value out of the
                    // buffer without dropping it in place -- ownership transfers to the
                    // caller. The buffer pointer is valid from the allocation in `new()`.
                    let item = unsafe { ptr::read(self.buffer.add(read_pos)) };

                    return Some(item);
                }
                Err(_) => {
                    // Retry
                    core::hint::spin_loop();
                }
            }
        }
    }

    /// Get current size
    fn size(&self) -> usize {
        self.size.load(Ordering::Relaxed)
    }

    /// Get capacity
    fn capacity(&self) -> usize {
        self.capacity
    }
}

impl<T> Drop for RingBuffer<T> {
    fn drop(&mut self) {
        // Clean up remaining items
        while self.pop().is_some() {}

        // Deallocate buffer
        let layout = core::alloc::Layout::array::<T>(self.capacity)
            .expect("ring buffer layout error in drop");
        // SAFETY: The buffer was allocated in `new()` using `alloc::alloc::alloc`
        // with the same layout (same capacity and type T). All remaining items have
        // been drained by the pop() loop above, so no live T values remain in the
        // buffer. The pointer has not been deallocated elsewhere. We have exclusive
        // access via `&mut self` in the Drop impl.
        unsafe {
            alloc::alloc::dealloc(self.buffer as *mut u8, layout);
        }
    }
}

// SAFETY: RingBuffer<T> can be sent across threads if T: Send. The buffer is a
// raw heap allocation owned entirely by the RingBuffer, and all T values stored
// in it are owned by the buffer. Transferring the RingBuffer transfers
// ownership of the contained T values.
unsafe impl<T: Send> Send for RingBuffer<T> {}
// SAFETY: RingBuffer<T> can be shared across threads if T: Send. Thread safety
// is provided by atomic operations on write_pos, read_pos, and size, which
// coordinate concurrent push/pop access. The size atomic with Acquire/Release
// ordering ensures that a consumer sees fully written data from a producer.
// NOTE: This implementation has a subtle race between concurrent pushers (or
// concurrent poppers) since fetch_add on position does not coordinate with the
// size check. In practice, this is used with single-producer/single-consumer
// patterns.
unsafe impl<T: Send> Sync for RingBuffer<T> {}

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
            messages: [None; 16],
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
        for i in 0..self.count {
            if let Some(msg) = self.messages[i] {
                f(msg);
            }
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

fn timestamp_to_ns(cycles: u64) -> u64 {
    // Assume 2GHz CPU for now
    cycles / 2
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
        assert!(channel.send_async(msg).is_ok());
        assert!(channel.send_async(msg).is_ok());

        // Should be full
        assert_eq!(channel.send_async(msg), Err(IpcError::ChannelFull));
    }
}
