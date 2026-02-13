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

unsafe impl<T: ?Sized + Send> Send for RwLock<T> {}
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
            data: unsafe { &*self.data.get() },
        }
    }

    pub fn write(&self) -> RwLockWriteGuard<'_, T> {
        RwLockWriteGuard {
            data: unsafe { &mut *self.data.get() },
        }
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
