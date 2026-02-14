//! Ultra-simple allocator without locks for RISC-V and AArch64
//!
//! This is a lock-free bump allocator implementation to resolve the
//! heap initialization hangs caused by spin lock incompatibility
//! on RISC-V and AArch64 bare metal.

use core::{
    alloc::{GlobalAlloc, Layout},
    ptr::{self, NonNull},
    sync::atomic::{AtomicUsize, Ordering},
};

/// Simple bump allocator without locks
pub struct UnsafeBumpAllocator {
    pub start: AtomicUsize,
    pub size: AtomicUsize,
    pub next: AtomicUsize,
    pub allocations: AtomicUsize,
}

impl UnsafeBumpAllocator {
    /// Create a new uninitialized bump allocator
    #[allow(clippy::new_without_default)]
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
    /// The caller must ensure that the memory region from `start` to `start +
    /// size` is valid and available for allocation.
    #[inline(always)]
    pub unsafe fn init(&self, start: *mut u8, size: usize) {
        // Debug output to confirm init is called
        #[cfg(target_arch = "riscv64")]
        {
            let uart = 0x10000000 as *mut u8;
            let msg = b"[ALLOC] init called\n";
            for &byte in msg {
                core::ptr::write_volatile(uart, byte);
            }
        }
        #[cfg(target_arch = "aarch64")]
        {
            // Use index-based loop - iterators hang on AArch64 bare metal
            let uart = 0x09000000 as *mut u8;
            let msg: &[u8] = b"[ALLOC] init called\n";
            let mut i = 0;
            while i < msg.len() {
                core::ptr::write_volatile(uart, msg[i]);
                i += 1;
            }
        }

        let start_addr = start as usize;

        // Use SeqCst ordering for correctness
        self.start.store(start_addr, Ordering::SeqCst);
        self.size.store(size, Ordering::SeqCst);
        self.next.store(start_addr, Ordering::SeqCst);
        self.allocations.store(0, Ordering::SeqCst);

        // Add memory barrier to ensure atomic stores complete
        core::sync::atomic::fence(Ordering::SeqCst);

        // Debug output to confirm init completed
        #[cfg(target_arch = "riscv64")]
        {
            let uart = 0x10000000 as *mut u8;
            let msg = b"[ALLOC] init done\n";
            for &byte in msg {
                core::ptr::write_volatile(uart, byte);
            }
        }
        #[cfg(target_arch = "aarch64")]
        {
            let uart = 0x09000000 as *mut u8;
            let msg: &[u8] = b"[ALLOC] init done\n";
            let mut i = 0;
            while i < msg.len() {
                core::ptr::write_volatile(uart, msg[i]);
                i += 1;
            }
        }
    }

    /// Get statistics about the allocator
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
    #[inline(always)]
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let start = self.start.load(Ordering::SeqCst);
        let size = self.size.load(Ordering::SeqCst);

        // Debug output for first allocation attempt
        #[cfg(target_arch = "riscv64")]
        {
            use core::sync::atomic::AtomicBool;
            static FIRST_ALLOC: AtomicBool = AtomicBool::new(true);
            if FIRST_ALLOC
                .compare_exchange(true, false, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
            {
                let uart = 0x10000000 as *mut u8;
                let msg = b"[ALLOC] First allocation attempt\n";
                for &byte in msg {
                    core::ptr::write_volatile(uart, byte);
                }
            }
        }

        if start == 0 {
            // Not initialized
            #[cfg(target_arch = "riscv64")]
            {
                let uart = 0x10000000 as *mut u8;
                let msg = b"[ALLOC] ERROR: Allocator not initialized (start=0)\n";
                for &byte in msg {
                    core::ptr::write_volatile(uart, byte);
                }
            }
            return ptr::null_mut();
        }

        let alloc_size = layout.size();
        let alloc_align = layout.align();

        // AArch64: Simple load-store path (no CAS, no iterators)
        // Safe during single-core boot - no concurrent access possible.
        #[cfg(target_arch = "aarch64")]
        {
            let current_next = self.next.load(Ordering::Relaxed);
            let align = if alloc_align > 8 { 8 } else { alloc_align };
            let mask = align - 1;
            let aligned_next = (current_next + mask) & !mask;

            let alloc_end = aligned_next + alloc_size;
            if alloc_end > start + size {
                return ptr::null_mut();
            }

            self.next.store(alloc_end, Ordering::Relaxed);
            self.allocations.fetch_add(1, Ordering::Relaxed);

            let allocated_ptr = aligned_next as *mut u8;
            core::ptr::write_bytes(allocated_ptr, 0, alloc_size);
            allocated_ptr
        }

        // RISC-V: CAS-based path for correctness
        #[cfg(not(target_arch = "aarch64"))]
        {
            let max_retries = 100;
            let mut retry_count = 0;

            loop {
                let current_next = self.next.load(Ordering::SeqCst);
                let align = if alloc_align > 8 { 8 } else { alloc_align };
                let mask = align - 1;
                let aligned_next = (current_next + mask) & !mask;

                let alloc_end = match aligned_next.checked_add(alloc_size) {
                    Some(end) => {
                        if end > start + size {
                            return ptr::null_mut();
                        }
                        end
                    }
                    None => return ptr::null_mut(),
                };

                match self.next.compare_exchange_weak(
                    current_next,
                    alloc_end,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                ) {
                    Ok(_) => {
                        let allocated_ptr = aligned_next as *mut u8;
                        core::ptr::write_bytes(allocated_ptr, 0, alloc_size);
                        self.allocations.fetch_add(1, Ordering::Relaxed);
                        return allocated_ptr;
                    }
                    Err(_) => {
                        retry_count += 1;
                        if retry_count >= max_retries {
                            return ptr::null_mut();
                        }
                        for _ in 0..(retry_count * 10) {
                            core::hint::spin_loop();
                        }
                    }
                }
            }
        }
    }

    #[inline(always)]
    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {
        // Bump allocator doesn't support deallocation
    }
}

/// Simple locked-like interface for compatibility
pub struct LockedUnsafeBumpAllocator {
    pub inner: UnsafeBumpAllocator,
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
        UnsafeBumpAllocatorGuard { inner: &self.inner }
    }
}

/// Guard for unsafe bump allocator (no actual locking)
pub struct UnsafeBumpAllocatorGuard<'a> {
    pub inner: &'a UnsafeBumpAllocator,
}

impl UnsafeBumpAllocatorGuard<'_> {
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
        // SAFETY: self.inner.alloc returns either a valid heap pointer or null.
        // The null check ensures NonNull::new_unchecked is only called with a
        // non-null pointer. The allocator was initialized with a valid memory
        // region via the unsafe init() method.
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
