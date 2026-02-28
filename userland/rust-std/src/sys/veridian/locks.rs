//! Futex-based synchronization primitives for VeridianOS.
//!
//! Provides `Mutex`, `RwLock`, `Condvar`, and `Once` -- all built on top
//! of the VeridianOS futex syscalls (`SYS_FUTEX_WAIT` / `SYS_FUTEX_WAKE`).
//!
//! These are designed to be used in user-space Rust programs running on
//! VeridianOS.  They mirror the semantics of `std::sync` primitives.

use core::{
    cell::UnsafeCell,
    ops::{Deref, DerefMut},
    sync::atomic::{AtomicI32, AtomicU32, Ordering},
};

use super::thread::{futex_wait, futex_wake};

// ============================================================================
// Mutex
// ============================================================================

/// Futex state values for the mutex.
const MUTEX_UNLOCKED: i32 = 0;
const MUTEX_LOCKED: i32 = 1;
const MUTEX_LOCKED_CONTENDED: i32 = 2;

/// A mutual exclusion lock backed by futex.
///
/// This is a simple, non-recursive mutex.  The implementation uses a
/// three-state futex word:
/// - 0 = unlocked
/// - 1 = locked, no waiters
/// - 2 = locked, waiters present
pub struct Mutex<T: ?Sized> {
    /// Futex word.
    state: AtomicI32,
    /// Protected data.
    data: UnsafeCell<T>,
}

// SAFETY: Mutex provides synchronized access to the inner data.
unsafe impl<T: ?Sized + Send> Send for Mutex<T> {}
unsafe impl<T: ?Sized + Send> Sync for Mutex<T> {}

impl<T> Mutex<T> {
    /// Create a new unlocked mutex.
    pub const fn new(val: T) -> Self {
        Mutex {
            state: AtomicI32::new(MUTEX_UNLOCKED),
            data: UnsafeCell::new(val),
        }
    }

    /// Consume the mutex and return the inner value.
    pub fn into_inner(self) -> T {
        self.data.into_inner()
    }
}

impl<T: ?Sized> Mutex<T> {
    /// Acquire the lock, blocking the current thread until it is available.
    pub fn lock(&self) -> MutexGuard<'_, T> {
        // Fast path: try to go from unlocked to locked.
        if self
            .state
            .compare_exchange(
                MUTEX_UNLOCKED,
                MUTEX_LOCKED,
                Ordering::Acquire,
                Ordering::Relaxed,
            )
            .is_ok()
        {
            return MutexGuard { lock: self };
        }
        self.lock_slow();
        MutexGuard { lock: self }
    }

    #[cold]
    fn lock_slow(&self) {
        loop {
            // Set to contended (or it already is).
            let prev = self.state.swap(MUTEX_LOCKED_CONTENDED, Ordering::Acquire);
            if prev == MUTEX_UNLOCKED {
                // We acquired it.
                return;
            }
            // Wait until the state changes from LOCKED_CONTENDED.
            let _ = futex_wait(
                self.state_ptr(),
                MUTEX_LOCKED_CONTENDED,
                0, // no timeout
            );
        }
    }

    /// Try to acquire the lock without blocking.
    pub fn try_lock(&self) -> Option<MutexGuard<'_, T>> {
        if self
            .state
            .compare_exchange(
                MUTEX_UNLOCKED,
                MUTEX_LOCKED,
                Ordering::Acquire,
                Ordering::Relaxed,
            )
            .is_ok()
        {
            Some(MutexGuard { lock: self })
        } else {
            None
        }
    }

    /// Release the lock.
    fn unlock(&self) {
        let prev = self.state.swap(MUTEX_UNLOCKED, Ordering::Release);
        if prev == MUTEX_LOCKED_CONTENDED {
            // There are waiters -- wake one.
            let _ = futex_wake(self.state_ptr(), 1);
        }
    }

    /// Get a mutable reference to the inner data (when we have exclusive
    /// access to the mutex itself, e.g. `&mut Mutex`).
    pub fn get_mut(&mut self) -> &mut T {
        self.data.get_mut()
    }

    #[inline]
    fn state_ptr(&self) -> *const i32 {
        &self.state as *const AtomicI32 as *const i32
    }
}

/// RAII guard for `Mutex`.
pub struct MutexGuard<'a, T: ?Sized> {
    lock: &'a Mutex<T>,
}

impl<T: ?Sized> Deref for MutexGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        // SAFETY: We hold the lock.
        unsafe { &*self.lock.data.get() }
    }
}

impl<T: ?Sized> DerefMut for MutexGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: We hold the lock exclusively.
        unsafe { &mut *self.lock.data.get() }
    }
}

impl<T: ?Sized> Drop for MutexGuard<'_, T> {
    fn drop(&mut self) {
        self.lock.unlock();
    }
}

impl<T: ?Sized + core::fmt::Debug> core::fmt::Debug for MutexGuard<'_, T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        (**self).fmt(f)
    }
}

// ============================================================================
// RwLock
// ============================================================================

/// RwLock state encoding:
/// - Bits 0-29: reader count (up to ~1 billion)
/// - Bit 30: writer waiting flag
/// - Bit 31 (sign bit): writer active
const RWLOCK_WRITER_ACTIVE: i32 = i32::MIN; // 0x8000_0000
const RWLOCK_WRITER_WAITING: i32 = 0x4000_0000;
const RWLOCK_READER_MASK: i32 = 0x3FFF_FFFF;

/// A reader-writer lock backed by futex.
pub struct RwLock<T: ?Sized> {
    state: AtomicI32,
    data: UnsafeCell<T>,
}

unsafe impl<T: ?Sized + Send> Send for RwLock<T> {}
unsafe impl<T: ?Sized + Send + Sync> Sync for RwLock<T> {}

impl<T> RwLock<T> {
    /// Create a new unlocked RwLock.
    pub const fn new(val: T) -> Self {
        RwLock {
            state: AtomicI32::new(0),
            data: UnsafeCell::new(val),
        }
    }

    /// Consume and return inner value.
    pub fn into_inner(self) -> T {
        self.data.into_inner()
    }
}

impl<T: ?Sized> RwLock<T> {
    /// Acquire a shared (read) lock.
    pub fn read(&self) -> RwLockReadGuard<'_, T> {
        loop {
            let val = self.state.load(Ordering::Relaxed);
            // Cannot read if writer is active or waiting.
            if val & (RWLOCK_WRITER_ACTIVE | RWLOCK_WRITER_WAITING) != 0 {
                let _ = futex_wait(self.state_ptr(), val, 0);
                continue;
            }
            // Try to increment reader count.
            if self
                .state
                .compare_exchange_weak(val, val + 1, Ordering::Acquire, Ordering::Relaxed)
                .is_ok()
            {
                return RwLockReadGuard { lock: self };
            }
        }
    }

    /// Try to acquire a shared lock without blocking.
    pub fn try_read(&self) -> Option<RwLockReadGuard<'_, T>> {
        let val = self.state.load(Ordering::Relaxed);
        if val & (RWLOCK_WRITER_ACTIVE | RWLOCK_WRITER_WAITING) != 0 {
            return None;
        }
        if self
            .state
            .compare_exchange(val, val + 1, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
        {
            Some(RwLockReadGuard { lock: self })
        } else {
            None
        }
    }

    /// Acquire an exclusive (write) lock.
    pub fn write(&self) -> RwLockWriteGuard<'_, T> {
        loop {
            let val = self.state.load(Ordering::Relaxed);
            if val == 0 {
                // No readers, no writer -- try to acquire.
                if self
                    .state
                    .compare_exchange(
                        0,
                        RWLOCK_WRITER_ACTIVE,
                        Ordering::Acquire,
                        Ordering::Relaxed,
                    )
                    .is_ok()
                {
                    return RwLockWriteGuard { lock: self };
                }
                continue;
            }
            // Set writer-waiting flag so new readers back off.
            if val & RWLOCK_WRITER_WAITING == 0 {
                let _ = self.state.compare_exchange(
                    val,
                    val | RWLOCK_WRITER_WAITING,
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                );
            }
            let _ = futex_wait(self.state_ptr(), self.state.load(Ordering::Relaxed), 0);
        }
    }

    /// Try to acquire an exclusive lock without blocking.
    pub fn try_write(&self) -> Option<RwLockWriteGuard<'_, T>> {
        if self
            .state
            .compare_exchange(
                0,
                RWLOCK_WRITER_ACTIVE,
                Ordering::Acquire,
                Ordering::Relaxed,
            )
            .is_ok()
        {
            Some(RwLockWriteGuard { lock: self })
        } else {
            None
        }
    }

    /// Release a read lock.
    fn read_unlock(&self) {
        let prev = self.state.fetch_sub(1, Ordering::Release);
        // If we were the last reader and a writer is waiting, wake it.
        if (prev & RWLOCK_READER_MASK) == 1 && (prev & RWLOCK_WRITER_WAITING) != 0 {
            let _ = futex_wake(self.state_ptr(), 1);
        }
    }

    /// Release a write lock.
    fn write_unlock(&self) {
        self.state.store(0, Ordering::Release);
        // Wake all waiters (readers and writers).
        let _ = futex_wake(self.state_ptr(), i32::MAX);
    }

    #[inline]
    fn state_ptr(&self) -> *const i32 {
        &self.state as *const AtomicI32 as *const i32
    }
}

/// RAII guard for shared (read) access.
pub struct RwLockReadGuard<'a, T: ?Sized> {
    lock: &'a RwLock<T>,
}

impl<T: ?Sized> Deref for RwLockReadGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        unsafe { &*self.lock.data.get() }
    }
}

impl<T: ?Sized> Drop for RwLockReadGuard<'_, T> {
    fn drop(&mut self) {
        self.lock.read_unlock();
    }
}

/// RAII guard for exclusive (write) access.
pub struct RwLockWriteGuard<'a, T: ?Sized> {
    lock: &'a RwLock<T>,
}

impl<T: ?Sized> Deref for RwLockWriteGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        unsafe { &*self.lock.data.get() }
    }
}

impl<T: ?Sized> DerefMut for RwLockWriteGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.lock.data.get() }
    }
}

impl<T: ?Sized> Drop for RwLockWriteGuard<'_, T> {
    fn drop(&mut self) {
        self.lock.write_unlock();
    }
}

// ============================================================================
// Condvar
// ============================================================================

/// A condition variable backed by futex.
///
/// Used with `Mutex` to wait for a condition to become true.
pub struct Condvar {
    /// Sequence counter -- incremented on each `notify_*` call.
    seq: AtomicI32,
}

impl Condvar {
    /// Create a new condition variable.
    pub const fn new() -> Self {
        Condvar {
            seq: AtomicI32::new(0),
        }
    }

    /// Block the current thread until notified.
    ///
    /// The mutex guard is released while waiting and re-acquired before
    /// returning.  Spurious wake-ups are possible.
    pub fn wait<'a, T: ?Sized>(&self, guard: MutexGuard<'a, T>) -> MutexGuard<'a, T> {
        let seq = self.seq.load(Ordering::Relaxed);
        let lock = guard.lock;
        // Release the mutex.
        drop(guard);
        // Wait for the sequence number to change.
        let _ = futex_wait(self.seq_ptr(), seq, 0);
        // Re-acquire the mutex.
        lock.lock()
    }

    /// Block until notified or timeout (in milliseconds).
    ///
    /// Returns `true` if woken by notification, `false` on timeout.
    pub fn wait_timeout<'a, T: ?Sized>(
        &self,
        guard: MutexGuard<'a, T>,
        timeout_ms: u64,
    ) -> (MutexGuard<'a, T>, bool) {
        let seq = self.seq.load(Ordering::Relaxed);
        let lock = guard.lock;
        drop(guard);
        let result = futex_wait(self.seq_ptr(), seq, timeout_ms);
        let timed_out = matches!(result, Err(SyscallError::WouldBlock));
        (lock.lock(), !timed_out)
    }

    /// Wake one waiting thread.
    pub fn notify_one(&self) {
        self.seq.fetch_add(1, Ordering::Release);
        let _ = futex_wake(self.seq_ptr(), 1);
    }

    /// Wake all waiting threads.
    pub fn notify_all(&self) {
        self.seq.fetch_add(1, Ordering::Release);
        let _ = futex_wake(self.seq_ptr(), i32::MAX);
    }

    #[inline]
    fn seq_ptr(&self) -> *const i32 {
        &self.seq as *const AtomicI32 as *const i32
    }
}

use super::SyscallError;

// ============================================================================
// Once
// ============================================================================

/// State machine for `Once`.
const ONCE_INCOMPLETE: u32 = 0;
const ONCE_RUNNING: u32 = 1;
const ONCE_COMPLETE: u32 = 2;
const ONCE_POISONED: u32 = 3;

/// A synchronization primitive for one-time initialization.
pub struct Once {
    state: AtomicU32,
}

impl Once {
    /// Create a new `Once` in the incomplete state.
    pub const fn new() -> Self {
        Once {
            state: AtomicU32::new(ONCE_INCOMPLETE),
        }
    }

    /// Run the given closure, ensuring it is executed exactly once.
    ///
    /// If another thread is currently running the closure, this blocks
    /// until it completes.
    pub fn call_once<F: FnOnce()>(&self, f: F) {
        if self.state.load(Ordering::Acquire) == ONCE_COMPLETE {
            return;
        }
        self.call_once_slow(f);
    }

    #[cold]
    fn call_once_slow<F: FnOnce()>(&self, f: F) {
        loop {
            match self.state.compare_exchange(
                ONCE_INCOMPLETE,
                ONCE_RUNNING,
                Ordering::Acquire,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    // We won the race -- run the closure.
                    f();
                    self.state.store(ONCE_COMPLETE, Ordering::Release);
                    // Wake all waiters.
                    let _ = futex_wake(self.state_ptr(), i32::MAX);
                    return;
                }
                Err(ONCE_RUNNING) => {
                    // Another thread is running -- wait.
                    let _ = futex_wait(self.state_ptr(), ONCE_RUNNING as i32, 0);
                    // After waking, check if it's complete now.
                    if self.state.load(Ordering::Acquire) == ONCE_COMPLETE {
                        return;
                    }
                }
                Err(ONCE_COMPLETE) => return,
                Err(ONCE_POISONED) => {
                    // Previous init panicked.  In no_std we cannot unwind,
                    // so just loop trying to become the runner.
                    let _ = self.state.compare_exchange(
                        ONCE_POISONED,
                        ONCE_RUNNING,
                        Ordering::Acquire,
                        Ordering::Relaxed,
                    );
                }
                Err(_) => {
                    core::hint::spin_loop();
                }
            }
        }
    }

    /// Check if `call_once` has completed.
    pub fn is_completed(&self) -> bool {
        self.state.load(Ordering::Acquire) == ONCE_COMPLETE
    }

    #[inline]
    fn state_ptr(&self) -> *const i32 {
        // AtomicU32 and AtomicI32 have the same layout; the futex syscall
        // operates on raw 32-bit words.
        &self.state as *const AtomicU32 as *const i32
    }
}
