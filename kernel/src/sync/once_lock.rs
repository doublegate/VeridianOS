//! Safe Global Initialization (Rust 2024 Compatible)
//!
//! Provides safe alternatives to `static mut` for global state management.
//! Uses atomic operations and proper synchronization for Rust 2024 edition.

use core::{
    cell::UnsafeCell,
    sync::atomic::{AtomicPtr, Ordering},
};

use spin::Mutex;

/// A cell that can be written to only once (Rust 2024 compatible)
///
/// Similar to std::sync::OnceLock but works in no_std environments.
pub struct OnceLock<T> {
    inner: AtomicPtr<T>,
}

impl<T> OnceLock<T> {
    /// Create a new empty OnceLock
    pub const fn new() -> Self {
        Self {
            inner: AtomicPtr::new(core::ptr::null_mut()),
        }
    }

    /// Get the value if initialized
    pub fn get(&self) -> Option<&'static T> {
        let ptr = self.inner.load(Ordering::Acquire);
        if ptr.is_null() {
            None
        } else {
            Some(unsafe { &*ptr })
        }
    }

    /// Get mutable reference if initialized (unsafe)
    ///
    /// # Safety
    /// Caller must ensure exclusive access
    pub unsafe fn get_mut(&self) -> Option<&'static mut T> {
        let ptr = self.inner.load(Ordering::Acquire);
        if ptr.is_null() {
            None
        } else {
            Some(&mut *ptr)
        }
    }

    /// Initialize the cell with a value
    ///
    /// Returns Ok(()) if initialization succeeds, Err(value) if already
    /// initialized
    pub fn set(&self, value: T) -> Result<(), T> {
        let boxed = alloc::boxed::Box::new(value);
        let ptr = alloc::boxed::Box::into_raw(boxed);

        match self.inner.compare_exchange(
            core::ptr::null_mut(),
            ptr,
            Ordering::Release,
            Ordering::Acquire,
        ) {
            Ok(_) => Ok(()),
            Err(_) => {
                // Already initialized, clean up our allocation
                let _ = unsafe { alloc::boxed::Box::from_raw(ptr) };
                Err(unsafe { core::ptr::read(ptr) })
            }
        }
    }

    /// Get or initialize the value
    pub fn get_or_init<F>(&self, f: F) -> &'static T
    where
        F: FnOnce() -> T,
    {
        if let Some(val) = self.get() {
            return val;
        }

        let value = f();
        match self.set(value) {
            Ok(()) => self.get().unwrap(),
            Err(_) => self.get().unwrap(), // Someone else initialized it
        }
    }
}

unsafe impl<T: Send> Send for OnceLock<T> {}
unsafe impl<T: Send + Sync> Sync for OnceLock<T> {}

impl<T> Drop for OnceLock<T> {
    fn drop(&mut self) {
        let ptr = self.inner.load(Ordering::Acquire);
        if !ptr.is_null() {
            unsafe {
                let _ = alloc::boxed::Box::from_raw(ptr);
            }
        }
    }
}

/// Lazy initialization with function (Rust 2024 compatible)
///
/// Similar to std::sync::LazyLock but for no_std
pub struct LazyLock<T, F = fn() -> T> {
    cell: OnceLock<T>,
    init: UnsafeCell<Option<F>>,
}

impl<T: 'static, F: FnOnce() -> T> LazyLock<T, F> {
    /// Create a new LazyLock with initialization function
    pub const fn new(init: F) -> Self {
        Self {
            cell: OnceLock::new(),
            init: UnsafeCell::new(Some(init)),
        }
    }

    /// Force initialization and get reference
    pub fn force(&self) -> &T {
        self.cell.get_or_init(|| {
            let init = unsafe { &mut *self.init.get() };
            match init.take() {
                Some(f) => f(),
                None => panic!("LazyLock initialization function called twice"),
            }
        })
    }
}

impl<T: 'static, F: FnOnce() -> T> core::ops::Deref for LazyLock<T, F> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.force()
    }
}

unsafe impl<T: Send, F: Send> Send for LazyLock<T, F> {}
unsafe impl<T: Sync, F: Send> Sync for LazyLock<T, F> {}

/// Safe global state with mutex (Rust 2024 compatible)
pub struct GlobalState<T> {
    inner: Mutex<Option<T>>,
}

impl<T> GlobalState<T> {
    /// Create new uninitialized global state
    pub const fn new() -> Self {
        Self {
            inner: Mutex::new(None),
        }
    }

    /// Initialize the global state
    pub fn init(&self, value: T) -> Result<(), T> {
        let mut lock = self.inner.lock();
        if lock.is_some() {
            Err(value)
        } else {
            *lock = Some(value);
            Ok(())
        }
    }

    /// Get reference with closure
    pub fn with<R, F: FnOnce(&T) -> R>(&self, f: F) -> Option<R> {
        let lock = self.inner.lock();
        lock.as_ref().map(f)
    }

    /// Get mutable reference with closure
    pub fn with_mut<R, F: FnOnce(&mut T) -> R>(&self, f: F) -> Option<R> {
        let mut lock = self.inner.lock();
        lock.as_mut().map(f)
    }

    /// Try to get a reference (may fail if not initialized)
    pub fn try_get(&self) -> Option<spin::MutexGuard<Option<T>>> {
        let lock = self.inner.lock();
        if lock.is_some() {
            Some(lock)
        } else {
            None
        }
    }
}

impl<T> Default for GlobalState<T> {
    fn default() -> Self {
        Self::new()
    }
}

unsafe impl<T: Send> Send for GlobalState<T> {}
unsafe impl<T: Send> Sync for GlobalState<T> {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test_case]
    fn test_once_lock() {
        let lock = OnceLock::new();
        assert!(lock.get().is_none());

        assert!(lock.set(42).is_ok());
        assert_eq!(*lock.get().unwrap(), 42);

        // Second set should fail
        assert!(lock.set(100).is_err());
    }

    #[test_case]
    fn test_lazy_lock() {
        let lazy = LazyLock::new(|| 42);
        assert_eq!(*lazy, 42);
    }

    #[test_case]
    fn test_global_state() {
        let state = GlobalState::new();
        assert!(state.init(String::from("hello")).is_ok());

        state.with(|s| {
            assert_eq!(s, "hello");
        });

        state.with_mut(|s| {
            s.push_str(" world");
        });

        state.with(|s| {
            assert_eq!(s, "hello world");
        });
    }
}
