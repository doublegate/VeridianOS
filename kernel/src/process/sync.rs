//! Process synchronization primitives
//!
//! This module provides synchronization mechanisms for processes and threads,
//! including mutexes, semaphores, and condition variables.

use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};

use crate::error::KernelError;

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::{collections::VecDeque, vec::Vec};

use spin::Mutex as SpinMutex;

use super::{ProcessId, ThreadId};

/// Wait queue for blocking threads
#[cfg(feature = "alloc")]
pub struct WaitQueue {
    /// Queue of waiting threads
    waiters: SpinMutex<VecDeque<(ProcessId, ThreadId)>>,
}

#[cfg(feature = "alloc")]
impl Default for WaitQueue {
    fn default() -> Self {
        Self {
            waiters: SpinMutex::new(VecDeque::new()),
        }
    }
}

#[cfg(feature = "alloc")]
impl WaitQueue {
    /// Create a new wait queue
    pub const fn new() -> Self {
        Self {
            waiters: SpinMutex::new(VecDeque::new()),
        }
    }

    /// Add current thread to wait queue
    pub fn wait(&self) {
        if let (Some(process), Some(thread)) = (super::current_process(), super::current_thread()) {
            self.waiters.lock().push_back((process.pid, thread.tid));

            // Block thread
            thread.set_state(super::thread::ThreadState::Blocked);

            // Yield to scheduler
            crate::sched::yield_cpu();
        }
    }

    /// Wake up one thread
    pub fn wake_one(&self) -> bool {
        if let Some((pid, tid)) = self.waiters.lock().pop_front() {
            // Wake up the thread
            if let Some(process) = super::table::get_process(pid) {
                if let Some(thread) = process.get_thread(tid) {
                    thread.set_state(super::thread::ThreadState::Ready);
                    // TODO(phase5): Add thread to scheduler run queue
                    return true;
                }
            }
        }
        false
    }

    /// Wake up all threads
    pub fn wake_all(&self) -> usize {
        let mut count = 0;
        let waiters = self.waiters.lock().drain(..).collect::<Vec<_>>();

        for (pid, tid) in waiters {
            if let Some(process) = super::table::get_process(pid) {
                if let Some(thread) = process.get_thread(tid) {
                    thread.set_state(super::thread::ThreadState::Ready);
                    // TODO(phase5): Add thread to scheduler run queue
                    count += 1;
                }
            }
        }

        count
    }

    /// Check if queue is empty
    pub fn is_empty(&self) -> bool {
        self.waiters.lock().is_empty()
    }
}

/// Mutex implementation
pub struct Mutex {
    /// Lock state (0 = unlocked, 1 = locked)
    locked: AtomicBool,
    /// Owner thread
    owner: AtomicU64,
    /// Wait queue for blocked threads
    #[cfg(feature = "alloc")]
    waiters: WaitQueue,
}

impl Default for Mutex {
    fn default() -> Self {
        Self {
            locked: AtomicBool::new(false),
            owner: AtomicU64::new(0),
            #[cfg(feature = "alloc")]
            waiters: WaitQueue::new(),
        }
    }
}

impl Mutex {
    /// Create a new mutex
    pub const fn new() -> Self {
        Self {
            locked: AtomicBool::new(false),
            owner: AtomicU64::new(0),
            #[cfg(feature = "alloc")]
            waiters: WaitQueue::new(),
        }
    }

    /// Try to acquire the mutex
    pub fn try_lock(&self) -> bool {
        if self
            .locked
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
        {
            if let Some(thread) = super::current_thread() {
                self.owner.store(thread.tid.0, Ordering::Relaxed);
            }
            true
        } else {
            false
        }
    }

    /// Acquire the mutex, blocking if necessary
    pub fn lock(&self) {
        while !self.try_lock() {
            #[cfg(feature = "alloc")]
            {
                // Add to wait queue and block
                self.waiters.wait();
            }

            #[cfg(not(feature = "alloc"))]
            {
                // Spin or yield
                crate::sched::yield_cpu();
            }
        }
    }

    /// Release the mutex.
    ///
    /// Returns `Err` if the calling thread does not own the lock. Callers
    /// should handle this as a programming error rather than letting the
    /// kernel panic inside a critical section.
    pub fn unlock(&self) -> Result<(), KernelError> {
        // Verify we own the lock
        if let Some(thread) = super::current_thread() {
            if self.owner.load(Ordering::Relaxed) != thread.tid.0 {
                return Err(KernelError::PermissionDenied {
                    operation: "mutex_unlock",
                });
            }
        }

        self.owner.store(0, Ordering::Relaxed);
        self.locked.store(false, Ordering::Release);

        // Wake up one waiter
        #[cfg(feature = "alloc")]
        self.waiters.wake_one();

        Ok(())
    }

    /// Check if mutex is locked
    pub fn is_locked(&self) -> bool {
        self.locked.load(Ordering::Relaxed)
    }
}

/// Semaphore implementation
pub struct Semaphore {
    /// Current count
    count: AtomicU32,
    /// Maximum count
    max_count: u32,
    /// Wait queue
    #[cfg(feature = "alloc")]
    waiters: WaitQueue,
}

impl Semaphore {
    /// Create a new semaphore
    pub const fn new(initial: u32, max: u32) -> Self {
        Self {
            count: AtomicU32::new(initial),
            max_count: max,
            #[cfg(feature = "alloc")]
            waiters: WaitQueue::new(),
        }
    }

    /// Wait on semaphore (P operation)
    pub fn wait(&self) {
        loop {
            let count = self.count.load(Ordering::Relaxed);
            if count > 0 {
                if self
                    .count
                    .compare_exchange(count, count - 1, Ordering::Acquire, Ordering::Relaxed)
                    .is_ok()
                {
                    return;
                }
            } else {
                #[cfg(feature = "alloc")]
                {
                    // Block on wait queue
                    self.waiters.wait();
                }

                #[cfg(not(feature = "alloc"))]
                {
                    // Yield
                    crate::sched::yield_cpu();
                }
            }
        }
    }

    /// Try to wait on semaphore without blocking
    pub fn try_wait(&self) -> bool {
        loop {
            let count = self.count.load(Ordering::Relaxed);
            if count > 0 {
                if self
                    .count
                    .compare_exchange(count, count - 1, Ordering::Acquire, Ordering::Relaxed)
                    .is_ok()
                {
                    return true;
                }
            } else {
                return false;
            }
        }
    }

    /// Signal semaphore (V operation).
    ///
    /// Returns `Err` if signalling would exceed the maximum count, indicating
    /// a caller bug (more signals than waits). This avoids a kernel panic
    /// inside what may be a lock-holding context.
    pub fn signal(&self) -> Result<(), KernelError> {
        loop {
            let count = self.count.load(Ordering::Relaxed);
            if count >= self.max_count {
                return Err(KernelError::InvalidState {
                    expected: "count < max_count",
                    actual: "semaphore overflow",
                });
            }

            if self
                .count
                .compare_exchange(count, count + 1, Ordering::Release, Ordering::Relaxed)
                .is_ok()
            {
                // Wake up one waiter
                #[cfg(feature = "alloc")]
                self.waiters.wake_one();

                return Ok(());
            }
        }
    }

    /// Get current count
    pub fn count(&self) -> u32 {
        self.count.load(Ordering::Relaxed)
    }
}

/// Condition variable implementation
#[cfg(feature = "alloc")]
pub struct CondVar {
    /// Wait queue
    waiters: WaitQueue,
}

#[cfg(feature = "alloc")]
impl Default for CondVar {
    fn default() -> Self {
        Self {
            waiters: WaitQueue::new(),
        }
    }
}

#[cfg(feature = "alloc")]
impl CondVar {
    /// Create a new condition variable
    pub const fn new() -> Self {
        Self {
            waiters: WaitQueue::new(),
        }
    }

    /// Wait on condition variable.
    ///
    /// Returns `Err` if the mutex is not held by the caller. The caller
    /// must hold the mutex before calling `wait`, as required by the
    /// standard condition variable protocol.
    pub fn wait(&self, mutex: &Mutex) -> Result<(), KernelError> {
        // Must hold the mutex
        if !mutex.is_locked() {
            return Err(KernelError::InvalidState {
                expected: "mutex locked",
                actual: "mutex unlocked",
            });
        }

        // Add to wait queue
        self.waiters.wait();

        // Release mutex before blocking
        // Ignore the unlock result here: we verified the mutex was locked
        // above and we are the only thread that should be releasing it in
        // this protocol. If unlock fails, we still need to re-acquire.
        let _ = mutex.unlock();

        // We've been woken up, re-acquire mutex
        mutex.lock();

        Ok(())
    }

    /// Signal one waiting thread
    pub fn signal(&self) {
        self.waiters.wake_one();
    }

    /// Signal all waiting threads
    pub fn broadcast(&self) {
        self.waiters.wake_all();
    }
}

/// Read-write lock implementation
pub struct RwLock {
    /// Number of readers (0 = unlocked, >0 = read locked, -1 = write locked)
    state: AtomicUsize,
    /// Wait queues
    #[cfg(feature = "alloc")]
    read_waiters: WaitQueue,
    #[cfg(feature = "alloc")]
    write_waiters: WaitQueue,
}

impl Default for RwLock {
    fn default() -> Self {
        Self {
            state: AtomicUsize::new(0),
            #[cfg(feature = "alloc")]
            read_waiters: WaitQueue::new(),
            #[cfg(feature = "alloc")]
            write_waiters: WaitQueue::new(),
        }
    }
}

impl RwLock {
    /// Create a new read-write lock
    pub const fn new() -> Self {
        Self {
            state: AtomicUsize::new(0),
            #[cfg(feature = "alloc")]
            read_waiters: WaitQueue::new(),
            #[cfg(feature = "alloc")]
            write_waiters: WaitQueue::new(),
        }
    }

    /// Acquire read lock
    pub fn read_lock(&self) {
        loop {
            let state = self.state.load(Ordering::Relaxed);

            // Can't read if write locked
            if state == usize::MAX {
                #[cfg(feature = "alloc")]
                self.read_waiters.wait();

                #[cfg(not(feature = "alloc"))]
                crate::sched::yield_cpu();

                continue;
            }

            // Try to increment reader count
            if self
                .state
                .compare_exchange(state, state + 1, Ordering::Acquire, Ordering::Relaxed)
                .is_ok()
            {
                return;
            }
        }
    }

    /// Try to acquire read lock
    pub fn try_read_lock(&self) -> bool {
        let state = self.state.load(Ordering::Relaxed);

        if state != usize::MAX {
            self.state
                .compare_exchange(state, state + 1, Ordering::Acquire, Ordering::Relaxed)
                .is_ok()
        } else {
            false
        }
    }

    /// Release read lock
    pub fn read_unlock(&self) {
        let prev = self.state.fetch_sub(1, Ordering::Release);

        // If we were the last reader, wake up writers
        #[cfg(feature = "alloc")]
        if prev == 1 {
            self.write_waiters.wake_one();
        }
    }

    /// Acquire write lock
    pub fn write_lock(&self) {
        loop {
            if self
                .state
                .compare_exchange(0, usize::MAX, Ordering::Acquire, Ordering::Relaxed)
                .is_ok()
            {
                return;
            }

            #[cfg(feature = "alloc")]
            self.write_waiters.wait();

            #[cfg(not(feature = "alloc"))]
            crate::sched::yield_cpu();
        }
    }

    /// Try to acquire write lock
    pub fn try_write_lock(&self) -> bool {
        self.state
            .compare_exchange(0, usize::MAX, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
    }

    /// Release write lock
    pub fn write_unlock(&self) {
        self.state.store(0, Ordering::Release);

        // Wake up all readers and one writer
        #[cfg(feature = "alloc")]
        {
            self.read_waiters.wake_all();
            self.write_waiters.wake_one();
        }
    }
}

/// Barrier synchronization
pub struct Barrier {
    /// Number of threads to wait for
    threshold: usize,
    /// Current count
    count: AtomicUsize,
    /// Generation counter
    generation: AtomicUsize,
    /// Wait queue
    #[cfg(feature = "alloc")]
    waiters: WaitQueue,
}

impl Barrier {
    /// Create a new barrier
    pub const fn new(n: usize) -> Self {
        Self {
            threshold: n,
            count: AtomicUsize::new(0),
            generation: AtomicUsize::new(0),
            #[cfg(feature = "alloc")]
            waiters: WaitQueue::new(),
        }
    }

    /// Wait at barrier
    pub fn wait(&self) {
        let gen = self.generation.load(Ordering::Relaxed);
        let count = self.count.fetch_add(1, Ordering::Relaxed) + 1;

        if count == self.threshold {
            // We're the last thread, reset and wake everyone
            self.count.store(0, Ordering::Relaxed);
            self.generation.fetch_add(1, Ordering::Relaxed);

            #[cfg(feature = "alloc")]
            self.waiters.wake_all();
        } else {
            // Wait for others
            while self.generation.load(Ordering::Relaxed) == gen {
                #[cfg(feature = "alloc")]
                self.waiters.wait();

                #[cfg(not(feature = "alloc"))]
                crate::sched::yield_cpu();
            }
        }
    }
}
