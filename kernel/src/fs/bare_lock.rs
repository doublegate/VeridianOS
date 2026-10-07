//! Single-threaded RwLock replacement for AArch64 bare metal.
//!
//! On AArch64 without proper exclusive monitor configuration, `spin::RwLock`
//! hangs because its atomic CAS instructions (ldaxr/stlxr) spin forever.
//! Since we run single-threaded in kernel-mode init, a simple UnsafeCell
//! wrapper provides the same API without atomics.

use core::{
    cell::UnsafeCell,
    ops::{Deref, DerefMut},
};

pub struct RwLock<T: ?Sized> {
    data: UnsafeCell<T>,
}

// SAFETY: RwLock owns its `T`, so sending the lock sends the `T`, which is
// sound for `T: Send`.
unsafe impl<T: ?Sized + Send> Send for RwLock<T> {}
// SAFETY: NOT enforced by the type: read()/write() hand out references
// without any locking. This is sound only under the module's usage rule
// that the lock is used single-threaded during AArch64 kernel-mode init and
// that no write guard coexists with any other guard.
unsafe impl<T: ?Sized + Send + Sync> Sync for RwLock<T> {}

pub struct RwLockReadGuard<'a, T: ?Sized> {
    data: &'a T,
}

pub struct RwLockWriteGuard<'a, T: ?Sized> {
    data: &'a mut T,
}

impl<T> RwLock<T> {
    pub const fn new(val: T) -> Self {
        Self {
            data: UnsafeCell::new(val),
        }
    }
}

impl<T: ?Sized> RwLock<T> {
    pub fn read(&self) -> RwLockReadGuard<'_, T> {
        RwLockReadGuard {
            // SAFETY: This single-threaded RwLock replacement is only
            // used on AArch64 during kernel-mode init (no concurrent
            // access). UnsafeCell::get() returns a valid pointer to
            // the contained data. We create a shared reference since
            // this is a read lock.
            data: unsafe { &*self.data.get() },
        }
    }

    pub fn write(&self) -> RwLockWriteGuard<'_, T> {
        RwLockWriteGuard {
            // SAFETY: Same as read() - single-threaded init context.
            // We create an exclusive reference since this is a write
            // lock. No other references can exist simultaneously in
            // the single-threaded init context.
            data: unsafe { &mut *self.data.get() },
        }
    }

    /// Exclusive access through `&mut self`, which already proves no other
    /// reference exists.
    pub fn get_mut(&mut self) -> &mut T {
        self.data.get_mut()
    }
}

impl<T: ?Sized> Deref for RwLockReadGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.data
    }
}

impl<T: ?Sized> Deref for RwLockWriteGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.data
    }
}

impl<T: ?Sized> DerefMut for RwLockWriteGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        self.data
    }
}
