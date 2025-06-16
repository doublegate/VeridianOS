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
        
        #[cfg(target_arch = "riscv64")]
        {
            let uart = 0x10000000 as *mut u8;
            let msg = b"[ALLOC] Initializing allocator\n";
            for &byte in msg {
                core::ptr::write_volatile(uart, byte);
            }
        }
        
        self.start.store(start_addr, Ordering::Relaxed);
        self.size.store(size, Ordering::Relaxed);
        self.next.store(start_addr, Ordering::Relaxed);
        self.allocations.store(0, Ordering::Relaxed);
        
        #[cfg(target_arch = "riscv64")]
        {
            let uart = 0x10000000 as *mut u8;
            let msg = b"[ALLOC] Allocator initialized\n";
            for &byte in msg {
                core::ptr::write_volatile(uart, byte);
            }
        }
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
        let start = self.start.load(Ordering::Acquire);
        let size = self.size.load(Ordering::Acquire);
        
        if start == 0 {
            // Not initialized
            #[cfg(target_arch = "riscv64")]
            {
                // Use RISC-V UART for debug output during early allocation
                let uart = 0x10000000 as *mut u8;
                let msg = b"[ALLOC] ERROR: Allocator not initialized\n";
                for &byte in msg {
                    core::ptr::write_volatile(uart, byte);
                }
            }
            return ptr::null_mut();
        }

        // Store layout values early to avoid repeated method calls
        let alloc_size = layout.size();
        let alloc_align = layout.align();
        
        // Debug large allocations
        if alloc_size > 1024 {
            #[cfg(target_arch = "riscv64")]
            {
                let uart = 0x10000000 as *mut u8;
                let msg = b"[ALLOC] Large allocation requested: ";
                for &byte in msg {
                    core::ptr::write_volatile(uart, byte);
                }
                // Simple number output
                let size_kb = alloc_size / 1024;
                let digit = (size_kb % 10) as u8 + b'0';
                core::ptr::write_volatile(uart, digit);
                core::ptr::write_volatile(uart, b'K');
                core::ptr::write_volatile(uart, b'\n');
            }
        }

        // Enhanced atomic allocation with detailed debugging
        let max_retries = 100;
        let mut retry_count = 0;
        
        #[cfg(target_arch = "riscv64")]
        {
            let uart = 0x10000000 as *mut u8;
            let msg = b"[ALLOC] Starting allocation loop\n";
            for &byte in msg {
                core::ptr::write_volatile(uart, byte);
            }
        }
        
        loop {
            let current_next = self.next.load(Ordering::Acquire);
            
            #[cfg(target_arch = "riscv64")]
            {
                let uart = 0x10000000 as *mut u8;
                let msg = b"[ALLOC] Got current_next value\n";
                for &byte in msg {
                    core::ptr::write_volatile(uart, byte);
                }
            }
            
            // Use the stored alignment value (but simplified to 8-byte minimum)
            let align = if alloc_align > 8 { 8 } else { alloc_align }; // Cap at 8 bytes for simplicity
            
            #[cfg(target_arch = "riscv64")]
            {
                let uart = 0x10000000 as *mut u8;
                let msg = b"[ALLOC] About to align\n";
                for &byte in msg {
                    core::ptr::write_volatile(uart, byte);
                }
            }
            
            // Simplified alignment calculation that's guaranteed to work
            let mask = align - 1;
            let aligned_next = (current_next + mask) & !mask;
            
            #[cfg(target_arch = "riscv64")]
            {
                let uart = 0x10000000 as *mut u8;
                let msg = b"[ALLOC] Alignment done\n";
                for &byte in msg {
                    core::ptr::write_volatile(uart, byte);
                }
            }
            
            // Check for arithmetic overflow (using stored alloc_size)
            if aligned_next > usize::MAX - alloc_size {
                #[cfg(target_arch = "riscv64")]
                {
                    let uart = 0x10000000 as *mut u8;
                    let msg = b"[ALLOC] ERROR: Arithmetic overflow\n";
                    for &byte in msg {
                        core::ptr::write_volatile(uart, byte);
                    }
                }
                return ptr::null_mut();
            }
            
            let alloc_end = aligned_next + alloc_size;
            
            // Check bounds
            if alloc_end > start + size {
                #[cfg(target_arch = "riscv64")]
                {
                    let uart = 0x10000000 as *mut u8;
                    let msg = b"[ALLOC] ERROR: Out of memory\n";
                    for &byte in msg {
                        core::ptr::write_volatile(uart, byte);
                    }
                }
                return ptr::null_mut();
            }
            
            #[cfg(target_arch = "riscv64")]
            {
                let uart = 0x10000000 as *mut u8;
                let msg = b"[ALLOC] About to CAS\n";
                for &byte in msg {
                    core::ptr::write_volatile(uart, byte);
                }
            }
            
            // Try to update next pointer atomically with stronger ordering
            match self.next.compare_exchange_weak(
                current_next,
                alloc_end,
                Ordering::Release,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    #[cfg(target_arch = "riscv64")]
                    {
                        let uart = 0x10000000 as *mut u8;
                        let msg = b"[ALLOC] CAS succeeded\n";
                        for &byte in msg {
                            core::ptr::write_volatile(uart, byte);
                        }
                    }
                    
                    // Success! Zero the allocated memory for safety
                    let allocated_ptr = aligned_next as *mut u8;
                    
                    #[cfg(target_arch = "riscv64")]
                    {
                        let uart = 0x10000000 as *mut u8;
                        let msg = b"[ALLOC] About to zero memory\n";
                        for &byte in msg {
                            core::ptr::write_volatile(uart, byte);
                        }
                    }
                    
                    core::ptr::write_bytes(allocated_ptr, 0, alloc_size);
                    
                    #[cfg(target_arch = "riscv64")]
                    {
                        let uart = 0x10000000 as *mut u8;
                        let msg = b"[ALLOC] Memory zeroed\n";
                        for &byte in msg {
                            core::ptr::write_volatile(uart, byte);
                        }
                    }
                    
                    // Update statistics
                    self.allocations.fetch_add(1, Ordering::Relaxed);
                    
                    // Debug successful large allocations
                    if alloc_size > 1024 {
                        #[cfg(target_arch = "riscv64")]
                        {
                            let uart = 0x10000000 as *mut u8;
                            let msg = b"[ALLOC] Large allocation successful\n";
                            for &byte in msg {
                                core::ptr::write_volatile(uart, byte);
                            }
                        }
                    }
                    
                    return allocated_ptr;
                }
                Err(_) => {
                    #[cfg(target_arch = "riscv64")]
                    {
                        let uart = 0x10000000 as *mut u8;
                        let msg = b"[ALLOC] CAS failed, retrying\n";
                        for &byte in msg {
                            core::ptr::write_volatile(uart, byte);
                        }
                    }
                    
                    // Retry with exponential backoff
                    retry_count += 1;
                    if retry_count >= max_retries {
                        #[cfg(target_arch = "riscv64")]
                        {
                            let uart = 0x10000000 as *mut u8;
                            let msg = b"[ALLOC] ERROR: Too many retries\n";
                            for &byte in msg {
                                core::ptr::write_volatile(uart, byte);
                            }
                        }
                        return ptr::null_mut();
                    }
                    
                    // Simple busy wait for backoff
                    for _ in 0..(retry_count * 10) {
                        core::hint::spin_loop();
                    }
                }
            }
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