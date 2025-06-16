//! Ultra-simple allocator without locks for RISC-V
//!
//! This is a lock-free bump allocator implementation to resolve the
//! RISC-V heap initialization hang caused by spin lock incompatibility.

use core::alloc::{GlobalAlloc, Layout};
use core::ptr::{self, NonNull};
use core::sync::atomic::{AtomicUsize, Ordering};

/// Simple bump allocator without locks
pub struct UnsafeBumpAllocator {
    start: AtomicUsize,
    size: AtomicUsize,
    next: AtomicUsize,
    allocations: AtomicUsize,
}

impl UnsafeBumpAllocator {
    /// Create a new uninitialized bump allocator
    pub const fn new() -> Self {
        Self {
            start: AtomicUsize::new(0),
            size: AtomicUsize::new(0),
            next: AtomicUsize::new(0),
            allocations: AtomicUsize::new(0),
        }
    }

    /// Initialize the allocator with a memory region
    ///
    /// # Safety
    ///
    /// The caller must ensure that the memory region from `start` to `start + size`
    /// is valid and available for allocation.
    pub unsafe fn init(&self, start: *mut u8, size: usize) {
        let start_addr = start as usize;
        self.start.store(start_addr, Ordering::Relaxed);
        self.size.store(size, Ordering::Relaxed);
        self.next.store(start_addr, Ordering::Relaxed);
        self.allocations.store(0, Ordering::Relaxed);
    }

    /// Get statistics about the allocator
    #[allow(dead_code)]
    pub fn stats(&self) -> (usize, usize, usize) {
        let start = self.start.load(Ordering::Relaxed);
        let next = self.next.load(Ordering::Relaxed);
        let size = self.size.load(Ordering::Relaxed);
        let allocations = self.allocations.load(Ordering::Relaxed);
        let allocated = next - start;
        (allocated, size - allocated, allocations)
    }
}

unsafe impl GlobalAlloc for UnsafeBumpAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let start = self.start.load(Ordering::Relaxed);
        let size = self.size.load(Ordering::Relaxed);
        
        if start == 0 {
            // Not initialized
            return ptr::null_mut();
        }

        // Simple atomic increment for next pointer
        loop {
            let current_next = self.next.load(Ordering::Relaxed);
            
            // Align the allocation
            let align = layout.align();
            let aligned_next = (current_next + align - 1) & !(align - 1);
            
            let alloc_end = aligned_next + layout.size();
            
            if alloc_end > start + size {
                // Out of memory
                return ptr::null_mut();
            }
            
            // Try to update next pointer atomically
            if self.next.compare_exchange_weak(
                current_next, 
                alloc_end, 
                Ordering::Relaxed, 
                Ordering::Relaxed
            ).is_ok() {
                // Success, increment allocation count
                self.allocations.fetch_add(1, Ordering::Relaxed);
                return aligned_next as *mut u8;
            }
            // Retry if someone else updated next
        }
    }

    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {
        // Bump allocator doesn't support deallocation
    }
}

/// Simple locked-like interface for compatibility
pub struct LockedUnsafeBumpAllocator {
    inner: UnsafeBumpAllocator,
}

impl LockedUnsafeBumpAllocator {
    /// Create a new empty allocator
    pub const fn empty() -> Self {
        Self {
            inner: UnsafeBumpAllocator::new(),
        }
    }

    /// Get a "lock" (just returns a wrapper)
    pub fn lock(&self) -> UnsafeBumpAllocatorGuard<'_> {
        UnsafeBumpAllocatorGuard {
            inner: &self.inner,
        }
    }
}

/// Guard for unsafe bump allocator (no actual locking)
pub struct UnsafeBumpAllocatorGuard<'a> {
    inner: &'a UnsafeBumpAllocator,
}

impl<'a> UnsafeBumpAllocatorGuard<'a> {
    /// Initialize the allocator
    ///
    /// # Safety
    ///
    /// The caller must ensure the memory region is valid
    pub unsafe fn init(&mut self, start: *mut u8, size: usize) {
        self.inner.init(start, size);
    }

    /// Allocate memory using first fit (same as bump allocation)
    pub fn allocate_first_fit(&mut self, layout: Layout) -> Result<NonNull<u8>, ()> {
        unsafe {
            let ptr = self.inner.alloc(layout);
            if ptr.is_null() {
                Err(())
            } else {
                Ok(NonNull::new_unchecked(ptr))
            }
        }
    }

    /// Deallocate memory (no-op for bump allocator)
    ///
    /// # Safety
    ///
    /// The pointer must have been allocated by this allocator
    pub unsafe fn deallocate(&mut self, _ptr: NonNull<u8>, _layout: Layout) {
        // Bump allocator doesn't support deallocation
    }
}