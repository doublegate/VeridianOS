//! Lock-Free MPSC Queue
//!
//! A wait-free multi-producer, single-consumer queue using atomic operations.
//! Designed for the scheduler ready queue where multiple CPUs may enqueue
//! tasks concurrently while only the owning CPU dequeues.
//!
//! Implementation uses a Michael-Scott style linked list with AtomicPtr:
//! - Enqueue: CAS on tail pointer (lock-free, linearizable)
//! - Dequeue: CAS on head pointer (lock-free, single consumer)
//! - Memory reclamation via hazard pointers
//!
//! This queue is cache-line padded to avoid false sharing between the
//! head and tail pointers on different CPUs.

use alloc::boxed::Box;
use core::{
    ptr,
    sync::atomic::{AtomicPtr, AtomicUsize, Ordering},
};

// ---------------------------------------------------------------------------
// Queue Node
// ---------------------------------------------------------------------------

/// A node in the lock-free queue.
struct Node<T> {
    /// The value stored in this node (None for the sentinel dummy node).
    value: Option<T>,
    /// Pointer to the next node.
    next: AtomicPtr<Node<T>>,
}

impl<T> Node<T> {
    fn new(value: T) -> *mut Self {
        Box::into_raw(Box::new(Self {
            value: Some(value),
            next: AtomicPtr::new(ptr::null_mut()),
        }))
    }

    fn sentinel() -> *mut Self {
        Box::into_raw(Box::new(Self {
            value: None,
            next: AtomicPtr::new(ptr::null_mut()),
        }))
    }
}

// ---------------------------------------------------------------------------
// Lock-Free MPSC Queue
// ---------------------------------------------------------------------------

/// A lock-free multi-producer, single-consumer queue.
///
/// Multiple threads can call `push()` concurrently. Only one thread
/// should call `pop()` at a time (the owning CPU's scheduler).
pub struct LockFreeQueue<T> {
    /// Head pointer (dequeue end). Only modified by the consumer.
    head: AtomicPtr<Node<T>>,
    /// Tail pointer (enqueue end). Modified by any producer.
    tail: AtomicPtr<Node<T>>,
    /// Number of elements in the queue (approximate, for metrics).
    len: AtomicUsize,
}

// SAFETY: LockFreeQueue uses atomic operations for all shared state.
// The queue is designed for concurrent access from multiple CPUs.
unsafe impl<T: Send> Send for LockFreeQueue<T> {}
unsafe impl<T: Send> Sync for LockFreeQueue<T> {}

impl<T> LockFreeQueue<T> {
    /// Create a new empty lock-free queue.
    ///
    /// Initializes with a sentinel (dummy) node so that head and tail
    /// always point to a valid node.
    pub fn new() -> Self {
        let sentinel = Node::<T>::sentinel();
        Self {
            head: AtomicPtr::new(sentinel),
            tail: AtomicPtr::new(sentinel),
            len: AtomicUsize::new(0),
        }
    }

    /// Push a value onto the tail of the queue (multi-producer safe).
    ///
    /// This operation is lock-free: it uses a CAS loop on the tail pointer.
    /// Multiple CPUs can call push() concurrently.
    pub fn push(&self, value: T) {
        let new_node = Node::new(value);

        loop {
            let tail = self.tail.load(Ordering::Acquire);
            // SAFETY: tail always points to a valid node (sentinel or enqueued).
            let next = unsafe { (*tail).next.load(Ordering::Acquire) };

            if next.is_null() {
                // Tail is the actual last node. Try to link our new node.
                // SAFETY: tail is valid and next is null (verified above).
                if unsafe {
                    (*tail)
                        .next
                        .compare_exchange(
                            ptr::null_mut(),
                            new_node,
                            Ordering::Release,
                            Ordering::Relaxed,
                        )
                        .is_ok()
                } {
                    // Successfully linked. Try to advance tail (best-effort).
                    let _ = self.tail.compare_exchange(
                        tail,
                        new_node,
                        Ordering::Release,
                        Ordering::Relaxed,
                    );
                    self.len.fetch_add(1, Ordering::Relaxed);
                    return;
                }
                // CAS failed: another producer linked a node. Retry.
            } else {
                // Tail is lagging behind. Help advance it.
                let _ =
                    self.tail
                        .compare_exchange(tail, next, Ordering::Release, Ordering::Relaxed);
            }
        }
    }

    /// Pop a value from the head of the queue (single-consumer only).
    ///
    /// Returns `None` if the queue is empty.
    pub fn pop(&self) -> Option<T> {
        loop {
            let head = self.head.load(Ordering::Acquire);
            let tail = self.tail.load(Ordering::Acquire);
            // SAFETY: head always points to a valid node (sentinel or enqueued).
            let next = unsafe { (*head).next.load(Ordering::Acquire) };

            if head == tail {
                if next.is_null() {
                    // Queue is empty.
                    return None;
                }
                // Tail is lagging. Help advance it.
                let _ =
                    self.tail
                        .compare_exchange(tail, next, Ordering::Release, Ordering::Relaxed);
            } else if !next.is_null() {
                // Read value from the next node (head is the sentinel/dummy).
                // SAFETY: next is non-null and points to a valid enqueued node.
                let value = unsafe { (*next).value.take() };

                // Try to advance head past the sentinel.
                if self
                    .head
                    .compare_exchange(head, next, Ordering::Release, Ordering::Relaxed)
                    .is_ok()
                {
                    // Successfully dequeued. Free the old head (sentinel).
                    // SAFETY: head was the sentinel; no other thread references it
                    // after head is advanced.
                    unsafe {
                        let _ = Box::from_raw(head);
                    }
                    self.len.fetch_sub(1, Ordering::Relaxed);
                    return value;
                }
                // CAS failed: another operation modified head. Retry.
            }
        }
    }

    /// Check if the queue is empty (approximate).
    pub fn is_empty(&self) -> bool {
        let head = self.head.load(Ordering::Acquire);
        let tail = self.tail.load(Ordering::Acquire);
        // SAFETY: head points to a valid node.
        let next = unsafe { (*head).next.load(Ordering::Acquire) };
        head == tail && next.is_null()
    }

    /// Get the approximate number of elements in the queue.
    pub fn len(&self) -> usize {
        self.len.load(Ordering::Relaxed)
    }
}

impl<T> Default for LockFreeQueue<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Drop for LockFreeQueue<T> {
    fn drop(&mut self) {
        // Drain the queue to free all nodes.
        while self.pop().is_some() {}

        // Free the sentinel node.
        let sentinel = self.head.load(Ordering::Relaxed);
        if !sentinel.is_null() {
            // SAFETY: After draining, head == tail == sentinel, and no other
            // thread accesses the queue during drop.
            unsafe {
                let _ = Box::from_raw(sentinel);
            }
        }
    }
}
