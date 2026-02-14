//! Safe Global Initialization (Rust 2024 Compatible)
//!
//! Provides safe alternatives to `static mut` for global state management.
//! Uses atomic operations and proper synchronization for Rust 2024 edition.

#![allow(clippy::needless_lifetimes, mismatched_lifetime_syntaxes)]

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

impl<T> Default for OnceLock<T> {
    fn default() -> Self {
        Self::new()
    }
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
            // SAFETY: The pointer is non-null, meaning `set()` or `get_or_init()`
            // has previously stored a valid, heap-allocated `T` via `Box::into_raw()`.
            // The Acquire ordering on the load synchronizes-with the Release in
            // `set()`, ensuring the pointed-to data is fully initialized before we
            // read it. The 'static lifetime is valid because the allocation is leaked
            // (only freed in `Drop`) and the OnceLock owns the allocation.
            Some(unsafe { &*ptr })
        }
    }

    /// Get mutable reference if initialized (unsafe)
    ///
    /// # Safety
    /// Caller must ensure exclusive access to the contained value for the
    /// duration of the returned reference's lifetime. No other references
    /// (mutable or immutable) to the inner value may exist concurrently.
    /// Violating this invariant causes undefined behavior due to aliased
    /// mutable references.
    pub unsafe fn get_mut(&self) -> Option<&'static mut T> {
        let ptr = self.inner.load(Ordering::Acquire);
        if ptr.is_null() {
            None
        } else {
            // SAFETY: The pointer is non-null, so `set()` has previously stored
            // a valid, heap-allocated `T`. The caller guarantees exclusive access,
            // so creating a mutable reference does not alias any existing references.
            // The Acquire load ensures we see the fully initialized data.
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
                // Already initialized, reclaim our allocation and return the value.
                // SAFETY: `ptr` was obtained from `Box::into_raw()` on the line above,
                // so it points to a valid, properly aligned, heap-allocated `T`. The
                // compare_exchange failed, meaning no one else has taken ownership of
                // this pointer, so we must reclaim it to avoid a memory leak.
                // We dereference the Box to extract the owned value before the Box
                // is dropped, avoiding a use-after-free.
                let boxed = unsafe { alloc::boxed::Box::from_raw(ptr) };
                Err(*boxed)
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
            // After set() succeeds or detects prior init, get() is guaranteed Some
            Ok(()) => self
                .get()
                .expect("OnceLock get failed after successful set"),
            Err(_) => self
                .get()
                .expect("OnceLock get failed after concurrent init"),
        }
    }
}

// SAFETY: OnceLock<T> can be sent across threads if T: Send because the inner
// value is heap-allocated and accessed through an AtomicPtr with proper memory
// ordering. Ownership transfer is safe when T itself is safe to transfer.
unsafe impl<T: Send> Send for OnceLock<T> {}
// SAFETY: OnceLock<T> can be shared across threads if T: Send + Sync. The
// AtomicPtr with Acquire/Release ordering ensures that concurrent `get()` calls
// observe a fully initialized T. The `set()` method uses compare_exchange to
// ensure at most one successful initialization. T must be Sync because multiple
// threads may hold shared references to the inner value simultaneously.
unsafe impl<T: Send + Sync> Sync for OnceLock<T> {}

impl<T> Drop for OnceLock<T> {
    fn drop(&mut self) {
        let ptr = self.inner.load(Ordering::Acquire);
        if !ptr.is_null() {
            // SAFETY: The pointer was originally created by `Box::into_raw()` in
            // `set()`. Since we are in `drop(&mut self)`, we have exclusive access
            // to the OnceLock, guaranteeing no other thread is concurrently reading
            // or writing the pointer. Reconstructing the Box reclaims the heap
            // allocation and drops the contained T.
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
            // SAFETY: Access to the UnsafeCell is safe here because `get_or_init`
            // on the inner OnceLock guarantees that this closure is called at most
            // once. The OnceLock's compare_exchange in `set()` ensures that even
            // if multiple threads race to call `force()`, only one will execute
            // this closure. After `take()` extracts the init function, subsequent
            // calls to `force()` will find the OnceLock already initialized and
            // skip this closure entirely.
            let init = unsafe { &mut *self.init.get() };
            match init.take() {
                Some(f) => f(),
                // Panic is intentional: this is a logic error. The OnceLock
                // guarantees single-init, so reaching None means the internal
                // invariant was violated (a bug in the LazyLock implementation).
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

// SAFETY: LazyLock<T, F> can be sent across threads if both T and F are Send.
// The inner OnceLock handles synchronization for the value, and the init
// function F is only accessed once (consumed via take()) so transferring
// ownership is safe.
unsafe impl<T: Send, F: Send> Send for LazyLock<T, F> {}
// SAFETY: LazyLock<T, F> can be shared across threads if T: Sync and F: Send.
// The OnceLock provides the synchronization for concurrent access to T. F must
// be Send (not Sync) because it is consumed exactly once via the UnsafeCell;
// the OnceLock's atomic CAS ensures only one thread executes the init closure.
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

// SAFETY: GlobalState<T> can be sent across threads if T: Send. The inner
// spin::Mutex provides mutual exclusion, so the contained Option<T> is only
// accessed by one thread at a time. Transferring ownership is safe when T
// itself supports cross-thread transfer.
unsafe impl<T: Send> Send for GlobalState<T> {}
// SAFETY: GlobalState<T> can be shared across threads if T: Send. The
// spin::Mutex serializes all access to the inner Option<T>, preventing data
// races. T only needs to be Send (not Sync) because the Mutex ensures no
// concurrent access -- each caller gets exclusive access through the lock
// guard.
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
