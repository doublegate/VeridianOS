//! Ultra-simple allocator without locks for RISC-V and AArch64
//!
//! This is a lock-free bump allocator implementation to resolve the
//! heap initialization hangs caused by spin lock incompatibility
//! on RISC-V and AArch64 bare metal.

use core::{
    alloc::{GlobalAlloc, Layout},
    ptr,
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
        // Layout guarantees a non-zero power-of-two alignment; honour all of
        // it (it used to be capped at 8, misaligning page-aligned requests).
        let mask = layout.align() - 1;
        let end_of_heap = start + size;

        // Claim [aligned, aligned + size) with a CAS so two CPUs (or an
        // interrupt handler) cannot be handed the same bytes (MEM-SEC-03).
        let claimed = self
            .next
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current| {
                let aligned = current.checked_add(mask)? & !mask;
                let end = aligned.checked_add(alloc_size)?;
                (end <= end_of_heap).then_some(end)
            });
        let Ok(previous) = claimed else {
            return ptr::null_mut();
        };
        let aligned = (previous + mask) & !mask;
        self.allocations.fetch_add(1, Ordering::Relaxed);

        let allocated_ptr = aligned as *mut u8;
        // SAFETY: [aligned, aligned + alloc_size) lies inside the heap
        // region given to init() and was claimed exclusively by the CAS.
        unsafe { core::ptr::write_bytes(allocated_ptr, 0, alloc_size) };
        allocated_ptr
    }

    #[inline(always)]
    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {
        // Bump allocator doesn't support deallocation
    }
}

#[cfg(test)]
mod tests {
    use alloc::{vec, vec::Vec};

    use super::*;

    fn heap(bytes: usize) -> (UnsafeBumpAllocator, Vec<u8>) {
        let mut backing = vec![0u8; bytes];
        let alloc = UnsafeBumpAllocator::new();
        // SAFETY: `backing` outlives every allocation made in the test.
        unsafe { alloc.init(backing.as_mut_ptr(), bytes) };
        (alloc, backing)
    }

    #[test]
    fn honours_large_alignment() {
        let (a, _backing) = heap(64 * 1024);
        let layout = Layout::from_size_align(16, 4096).unwrap();
        // SAFETY: valid non-zero layout.
        let p = unsafe { a.alloc(layout) };
        assert!(!p.is_null());
        assert_eq!(p as usize % 4096, 0);
    }

    #[test]
    fn returns_null_when_exhausted() {
        let (a, _backing) = heap(256);
        let layout = Layout::from_size_align(512, 8).unwrap();
        // SAFETY: valid non-zero layout.
        assert!(unsafe { a.alloc(layout) }.is_null());
    }

    #[test]
    fn allocations_never_overlap() {
        let (a, _backing) = heap(64 * 1024);
        let layout = Layout::from_size_align(24, 8).unwrap();
        let mut ptrs: Vec<usize> = (0..100)
            // SAFETY: valid non-zero layout.
            .map(|_| unsafe { a.alloc(layout) } as usize)
            .collect();
        ptrs.sort_unstable();
        assert!(ptrs.windows(2).all(|w| w[1] - w[0] >= 24));
    }
}
