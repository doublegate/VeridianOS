//! Cache Line Alignment Utilities
//!
//! Provides types and constants for eliminating false sharing in
//! concurrent data structures. False sharing occurs when independent
//! variables share a cache line, causing cache invalidation ping-pong
//! between CPU cores on writes.
//!
//! Modern x86_64 (Intel/AMD) and AArch64 (Cortex-A72) use 64-byte
//! cache lines. RISC-V implementations also commonly use 64 bytes.

use core::ops::{Deref, DerefMut};

/// Cache line size in bytes for the target architecture.
///
/// Intel/AMD x86_64: 64 bytes (since Pentium 4).
/// ARM Cortex-A72: 64 bytes (L1D and L2).
/// Common RISC-V implementations: 64 bytes.
pub const CACHE_LINE_SIZE: usize = 64;

/// Cache-line-aligned wrapper type for eliminating false sharing.
///
/// Wraps a value `T` and ensures it is aligned to a full cache line
/// boundary (64 bytes). When placed in arrays or adjacent to other
/// per-CPU data, this prevents false sharing between cores.
///
/// # Example
///
/// ```rust,ignore
/// use crate::mm::cache_aligned::CacheAligned;
/// use core::sync::atomic::{AtomicU64, Ordering};
///
/// // Each counter occupies its own cache line
/// static COUNTERS: [CacheAligned<AtomicU64>; 4] = [
///     CacheAligned::new(AtomicU64::new(0)),
///     CacheAligned::new(AtomicU64::new(0)),
///     CacheAligned::new(AtomicU64::new(0)),
///     CacheAligned::new(AtomicU64::new(0)),
/// ];
/// ```
#[repr(C, align(64))]
pub struct CacheAligned<T> {
    value: T,
}

impl<T> CacheAligned<T> {
    /// Create a new cache-line-aligned wrapper.
    pub const fn new(value: T) -> Self {
        Self { value }
    }

    /// Consume the wrapper and return the inner value.
    pub fn into_inner(self) -> T {
        self.value
    }
}

impl<T> Deref for CacheAligned<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.value
    }
}

impl<T> DerefMut for CacheAligned<T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.value
    }
}

// Safety: CacheAligned is transparent for Send/Sync -- it only adds alignment.
// If T: Send, CacheAligned<T>: Send. If T: Sync, CacheAligned<T>: Sync.
unsafe impl<T: Send> Send for CacheAligned<T> {}
unsafe impl<T: Sync> Sync for CacheAligned<T> {}

impl<T: Default> Default for CacheAligned<T> {
    fn default() -> Self {
        Self {
            value: T::default(),
        }
    }
}

impl<T: Clone> Clone for CacheAligned<T> {
    fn clone(&self) -> Self {
        Self {
            value: self.value.clone(),
        }
    }
}

impl<T: Copy> Copy for CacheAligned<T> {}

impl<T: core::fmt::Debug> core::fmt::Debug for CacheAligned<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CacheAligned")
            .field("value", &self.value)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use core::{
        mem,
        sync::atomic::{AtomicU64, Ordering},
    };

    use super::*;

    #[test]
    fn test_cache_line_alignment() {
        // CacheAligned<u64> should be 64 bytes (one full cache line)
        assert_eq!(mem::align_of::<CacheAligned<u64>>(), 64);
        assert!(mem::size_of::<CacheAligned<u64>>() >= 64);
    }

    #[test]
    fn test_deref() {
        let aligned = CacheAligned::new(42u64);
        assert_eq!(*aligned, 42);
    }

    #[test]
    fn test_deref_mut() {
        let mut aligned = CacheAligned::new(42u64);
        *aligned = 100;
        assert_eq!(*aligned, 100);
    }

    #[test]
    fn test_into_inner() {
        let aligned = CacheAligned::new(42u64);
        assert_eq!(aligned.into_inner(), 42);
    }

    #[test]
    fn test_atomic_usage() {
        let counter = CacheAligned::new(AtomicU64::new(0));
        counter.fetch_add(1, Ordering::Relaxed);
        assert_eq!(counter.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn test_array_no_false_sharing() {
        // Each element in the array should be on its own cache line
        let arr: [CacheAligned<AtomicU64>; 4] = [
            CacheAligned::new(AtomicU64::new(0)),
            CacheAligned::new(AtomicU64::new(0)),
            CacheAligned::new(AtomicU64::new(0)),
            CacheAligned::new(AtomicU64::new(0)),
        ];

        // Addresses should be at least 64 bytes apart
        let addr0 = &arr[0] as *const _ as usize;
        let addr1 = &arr[1] as *const _ as usize;
        assert!(addr1 - addr0 >= CACHE_LINE_SIZE);

        // Verify each element works independently
        arr[0].fetch_add(10, Ordering::Relaxed);
        arr[1].fetch_add(20, Ordering::Relaxed);
        arr[2].fetch_add(30, Ordering::Relaxed);
        arr[3].fetch_add(40, Ordering::Relaxed);
        assert_eq!(arr[0].load(Ordering::Relaxed), 10);
        assert_eq!(arr[1].load(Ordering::Relaxed), 20);
        assert_eq!(arr[2].load(Ordering::Relaxed), 30);
        assert_eq!(arr[3].load(Ordering::Relaxed), 40);
    }

    #[test]
    fn test_const_new() {
        // Verify const construction works (needed for static initialization)
        static COUNTER: CacheAligned<AtomicU64> = CacheAligned::new(AtomicU64::new(0));
        COUNTER.fetch_add(1, Ordering::Relaxed);
        assert_eq!(COUNTER.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn test_default() {
        let aligned: CacheAligned<u64> = CacheAligned::default();
        assert_eq!(*aligned, 0);
    }
}
