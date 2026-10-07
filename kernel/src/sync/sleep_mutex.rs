//! A mutex whose waiters sleep (blocking step of sprint D; N-138).
//!
//! The address-space lock was a spinlock that the page fault path could
//! only `try_lock`: a fault that found it held -- another thread of the
//! process in `mmap`, or on another CPU once those run tasks -- failed, and
//! a user thread got a spurious SIGSEGV. With [`SleepMutex`] a contended
//! `lock` sleeps on a wait queue until the holder releases it. Faults run on
//! the faulting thread's own kernel stack (ADR 0008), so they may sleep.
//!
//! The one case that must not wait is the holder faulting on its own lock:
//! a system call that copies user memory while holding it. The holder is
//! recorded, and [`SleepMutex::lock_unless_mine`] refuses instead of
//! deadlocking (the copy then fails with EFAULT, as before).
//!
//! Outside a dispatched task (early boot) `lock` spins. A holder must not
//! wait for anything while holding a spinlock that a waiter could need.

use core::{
    cell::UnsafeCell,
    ops::{Deref, DerefMut},
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};

/// A mutual-exclusion lock whose contended waiters sleep.
pub struct SleepMutex<T> {
    locked: AtomicBool,
    /// Task key of the holder (0: none, or not a dispatched task).
    owner: AtomicU64,
    waiters: crate::sched::dispatch::WaitQueue,
    data: UnsafeCell<T>,
}

// SAFETY: access to `data` is serialised by `locked` exactly as for a spin
// mutex; the value moves between threads only through the lock.
unsafe impl<T: Send> Send for SleepMutex<T> {}
// SAFETY: as above -- `&SleepMutex<T>` only yields `&T`/`&mut T` through a
// guard, and at most one guard exists at a time.
unsafe impl<T: Send> Sync for SleepMutex<T> {}

impl<T> SleepMutex<T> {
    pub const fn new(value: T) -> Self {
        Self {
            locked: AtomicBool::new(false),
            owner: AtomicU64::new(0),
            waiters: crate::sched::dispatch::WaitQueue::new(),
            data: UnsafeCell::new(value),
        }
    }

    fn acquire(&self) -> bool {
        self.locked
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
    }

    fn guard(&self) -> SleepMutexGuard<'_, T> {
        self.owner
            .store(crate::sched::dispatch::current_key(), Ordering::Relaxed);
        SleepMutexGuard { lock: self }
    }

    /// Take the lock if it is free.
    pub fn try_lock(&self) -> Option<SleepMutexGuard<'_, T>> {
        self.acquire().then(|| self.guard())
    }

    /// Take the lock, sleeping while another task holds it.
    pub fn lock(&self) -> SleepMutexGuard<'_, T> {
        if !self.acquire() {
            // Not interruptible: a lock wait ends only when the lock is ours.
            self.waiters.wait_until(|| self.acquire());
        }
        self.guard()
    }

    /// Like [`Self::lock`], but `None` when the calling task already holds
    /// the lock (waiting would deadlock).
    pub fn lock_unless_mine(&self) -> Option<SleepMutexGuard<'_, T>> {
        if self.acquire() {
            return Some(self.guard());
        }
        let me = crate::sched::dispatch::current_key();
        if me == 0 || self.owner.load(Ordering::Relaxed) == me {
            return None;
        }
        Some(self.lock())
    }

    /// The value, through exclusive access to the lock itself.
    pub fn get_mut(&mut self) -> &mut T {
        self.data.get_mut()
    }
}

/// Holds a [`SleepMutex`]; releases it (waking one waiter) when dropped.
pub struct SleepMutexGuard<'a, T> {
    lock: &'a SleepMutex<T>,
}

impl<T> Deref for SleepMutexGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        // SAFETY: the guard proves the lock is held, so no `&mut T` exists.
        unsafe { &*self.lock.data.get() }
    }
}

impl<T> DerefMut for SleepMutexGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: the guard proves exclusive ownership of the lock.
        unsafe { &mut *self.lock.data.get() }
    }
}

impl<T> Drop for SleepMutexGuard<'_, T> {
    fn drop(&mut self) {
        self.lock.owner.store(0, Ordering::Relaxed);
        self.lock.locked.store(false, Ordering::Release);
        self.lock.waiters.wake_one();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_try_lock_and_release() {
        let m = SleepMutex::new(5);
        {
            let mut g = m.lock();
            *g += 1;
            assert!(m.try_lock().is_none(), "held");
        }
        assert_eq!(*m.try_lock().expect("released"), 6);
    }

    #[test]
    fn get_mut_reaches_the_value() {
        let mut m = SleepMutex::new(alloc::vec![1, 2]);
        m.get_mut().push(3);
        assert_eq!(m.lock().len(), 3);
    }

    #[test]
    fn lock_unless_mine_outside_a_task_refuses_when_held() {
        // On the host there is no dispatched task (key 0): a held lock is
        // refused rather than waited for.
        let m = SleepMutex::new(());
        let _g = m.lock();
        assert!(m.lock_unless_mine().is_none());
    }
}
