//! Kernel heap allocator
//!
//! Implements a slab allocator for the kernel heap with size classes
//! and per-CPU caches for performance.

#![allow(dead_code, clippy::unwrap_or_default)]

#[cfg(target_arch = "x86_64")]
use core::{alloc::Layout, ptr::NonNull};

#[cfg(target_arch = "x86_64")]
use linked_list_allocator::LockedHeap;
#[cfg(target_arch = "x86_64")]
use spin::Mutex;

#[cfg(target_arch = "x86_64")]
use super::VirtualAddress;

// Static heap storage - kept in BSS for layout stability across all
// architectures. x86_64 uses this directly; AArch64/RISC-V use fixed physical
// addresses instead to avoid BSS/heap overlap issues.
static mut HEAP_MEMORY: [u8; 2 * 1024 * 1024] = [0; 2 * 1024 * 1024];

/// Kernel heap size (16 MB initially)
pub const HEAP_SIZE: usize = 16 * 1024 * 1024;

/// Kernel heap start address
#[cfg(target_arch = "x86_64")]
pub const HEAP_START: usize = 0x444444440000; // Address mapped by bootloader 0.9

#[cfg(target_arch = "aarch64")]
pub const HEAP_START: usize = 0x41000000; // 16MB into QEMU virt RAM (starts at 0x40000000)

#[cfg(target_arch = "riscv64")]
pub const HEAP_START: usize = 0x81000000; // 16MB into QEMU virt RAM (starts at 0x80000000)

/// Slab allocator for efficient small allocations (x86_64 only, uses
/// LockedHeap)
#[cfg(target_arch = "x86_64")]
pub struct SlabAllocator {
    /// Size classes for slab allocation
    slabs: [Option<Slab>; 10],
    /// Fallback allocator for large allocations
    fallback: LockedHeap,
    /// Statistics
    stats: Mutex<HeapStats>,
}

#[cfg(target_arch = "x86_64")]
/// A slab for a specific size class
struct Slab {
    /// Object size for this slab
    object_size: usize,
    /// Free list head
    free_list: Option<NonNull<FreeObject>>,
    /// Number of free objects
    free_count: usize,
    /// Total objects in slab
    total_objects: usize,
    /// Base address of slab
    base: VirtualAddress,
}

#[cfg(target_arch = "x86_64")]
/// Free object in slab free list
struct FreeObject {
    next: Option<NonNull<FreeObject>>,
}

#[cfg(target_arch = "x86_64")]
/// Heap statistics
#[derive(Debug, Default, Clone)]
pub struct HeapStats {
    /// Total bytes allocated
    pub allocated_bytes: usize,
    /// Total bytes freed
    pub freed_bytes: usize,
    /// Current bytes in use
    pub used_bytes: usize,
    /// Peak bytes used
    pub peak_bytes: usize,
    /// Number of allocations
    pub allocation_count: u64,
    /// Number of frees
    pub free_count: u64,
}

#[cfg(target_arch = "x86_64")]
/// Size classes for slab allocator (in bytes)
const SIZE_CLASSES: [usize; 10] = [8, 16, 32, 64, 128, 256, 512, 1024, 2048, 4096];

#[cfg(target_arch = "x86_64")]
impl Default for SlabAllocator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(target_arch = "x86_64")]
impl SlabAllocator {
    /// Create a new slab allocator
    pub const fn new() -> Self {
        Self {
            slabs: [const { None }; 10],
            fallback: LockedHeap::empty(),
            stats: Mutex::new(HeapStats {
                allocated_bytes: 0,
                freed_bytes: 0,
                used_bytes: 0,
                peak_bytes: 0,
                allocation_count: 0,
                free_count: 0,
            }),
        }
    }

    /// Initialize the slab allocator with heap memory
    ///
    /// # Safety
    ///
    /// The caller must ensure that:
    /// - The heap_start address is valid and properly aligned
    /// - The heap_size bytes starting at heap_start are available for use
    /// - This function is called only once
    pub unsafe fn init(&self, heap_start: usize, heap_size: usize) {
        // Reserve some memory for slab metadata
        let metadata_size = heap_size / 16; // 1/16 of heap for metadata
        let slab_area_size = heap_size - metadata_size;

        // Initialize fallback allocator with metadata area
        self.fallback
            .lock()
            .init(heap_start as *mut u8, metadata_size);

        // Initialize slabs
        let mut current_addr = heap_start + metadata_size;
        let slab_size = slab_area_size / SIZE_CLASSES.len();

        for &size in SIZE_CLASSES.iter() {
            if current_addr + slab_size > heap_start + heap_size {
                break;
            }

            // Create slab for this size class
            let _slab = self.init_slab(VirtualAddress::new(current_addr as u64), slab_size, size);

            // Store in array (this is a bit tricky without mut self)
            // In real implementation, would need interior mutability
            // For now, this is a placeholder

            current_addr += slab_size;
        }
    }

    /// Initialize a single slab
    fn init_slab(&self, base: VirtualAddress, size: usize, object_size: usize) -> Slab {
        let objects_per_slab = size / object_size;
        let mut free_list = None;

        // Build free list
        for i in (0..objects_per_slab).rev() {
            let obj_addr = base.as_u64() + (i * object_size) as u64;
            let obj_ptr = obj_addr as *mut FreeObject;

            // SAFETY: `obj_ptr` is derived from `base` (the slab's base virtual address)
            // plus a bounded offset `i * object_size` where `i < objects_per_slab` and
            // `objects_per_slab = size / object_size`, so the pointer stays within the
            // slab's allocated memory region [base, base + size). The slab memory was
            // reserved during `init()` and is exclusively owned by this allocator.
            // `obj_ptr` is non-null because `base` is a valid non-zero virtual address
            // and the offset is bounded, so `NonNull::new_unchecked` is sound.
            unsafe {
                (*obj_ptr).next = free_list;
                free_list = Some(NonNull::new_unchecked(obj_ptr));
            }
        }

        Slab {
            object_size,
            free_list,
            free_count: objects_per_slab,
            total_objects: objects_per_slab,
            base,
        }
    }

    /// Allocate from appropriate slab
    fn allocate(&self, layout: Layout) -> *mut u8 {
        let size = layout.size().max(layout.align());

        // Find appropriate size class
        for &class_size in SIZE_CLASSES.iter() {
            if size <= class_size {
                // Try to allocate from this slab
                // In real implementation, would need proper synchronization
                // This is a placeholder
                return self
                    .fallback
                    .lock()
                    .allocate_first_fit(layout)
                    .map(|ptr| ptr.as_ptr())
                    .unwrap_or(core::ptr::null_mut());
            }
        }

        // Large allocation - use fallback
        self.fallback
            .lock()
            .allocate_first_fit(layout)
            .map(|ptr| ptr.as_ptr())
            .unwrap_or(core::ptr::null_mut())
    }

    /// Free to appropriate slab
    fn deallocate(&self, ptr: *mut u8, layout: Layout) {
        let size = layout.size().max(layout.align());

        // Find appropriate size class
        for &class_size in SIZE_CLASSES.iter() {
            if size <= class_size {
                // Return to slab
                // In real implementation, would need proper synchronization
                // SAFETY: `ptr` was returned by a prior `allocate_first_fit` call on the
                // same `fallback` allocator, so it is a valid, non-null, properly aligned
                // pointer to an allocation of the given `layout`. The caller guarantees
                // the pointer is no longer in use (standard dealloc contract).
                // `NonNull::new_unchecked` is sound because `ptr` originated from
                // `allocate_first_fit` which only returns non-null pointers.
                unsafe {
                    self.fallback
                        .lock()
                        .deallocate(NonNull::new_unchecked(ptr), layout);
                }
                return;
            }
        }

        // Large allocation - use fallback
        // SAFETY: Same invariants as above -- `ptr` was returned by a prior
        // `allocate_first_fit` call for this `layout`, so it is valid, non-null,
        // and properly aligned. The caller guarantees exclusive ownership has been
        // relinquished.
        unsafe {
            self.fallback
                .lock()
                .deallocate(NonNull::new_unchecked(ptr), layout);
        }
    }

    /// Get heap statistics
    pub fn stats(&self) -> HeapStats {
        self.stats.lock().clone()
    }
}

// Global slab allocator instance (not used for now)
// static SLAB_ALLOCATOR: SlabAllocator = SlabAllocator::new();

/// Initialize the kernel heap
pub fn init() -> Result<(), &'static str> {
    // SAFETY: `uart_write_str` writes to the PL011 UART at a fixed MMIO address
    // (0x09000000) that is always mapped on the QEMU virt machine. This is a
    // simple register write with no memory safety implications beyond the MMIO
    // access, which is valid at any point during kernel execution on AArch64.
    #[cfg(target_arch = "aarch64")]
    unsafe {
        use crate::arch::aarch64::direct_uart::uart_write_str;
        uart_write_str("[HEAP] Initializing kernel heap\n");
    }
    #[cfg(not(target_arch = "aarch64"))]
    println!("[HEAP] Initializing kernel heap at 0x{:x}", HEAP_START);

    // SAFETY: We access `HEAP_MEMORY`, a static mut byte array in the kernel's BSS
    // section. This function is called exactly once during kernel initialization
    // (single-threaded boot context), so there are no concurrent accesses. The
    // resulting `heap_start` pointer is valid for `heap_size` bytes and the memory
    // does not overlap with any other allocation because it is a dedicated static
    // array in the kernel binary.
    #[allow(unused_unsafe)]
    unsafe {
        let heap_start = core::ptr::addr_of_mut!(HEAP_MEMORY) as *mut u8;
        let heap_size = 512 * 1024; // Size of HEAP_MEMORY (512KB)

        // RISC-V: Use lock-free UnsafeBumpAllocator
        #[cfg(target_arch = "riscv64")]
        {
            println!("[HEAP] Initializing RISC-V UnsafeBumpAllocator");
            println!(
                "[HEAP] Heap start: {:p}, size: {} bytes",
                heap_start, heap_size
            );

            // SAFETY: `ALLOCATOR` is the global bump allocator. `heap_start` points to
            // valid memory of at least `heap_size` bytes (the static HEAP_MEMORY array).
            // This is called once during single-threaded boot, so no concurrent access.
            // The `alloc` call uses a layout with size=8, align=8 which are valid (both
            // powers of two, size > 0).
            unsafe {
                use core::alloc::GlobalAlloc;

                use crate::ALLOCATOR;

                println!("[HEAP] Calling ALLOCATOR.init()...");
                ALLOCATOR.init(heap_start, heap_size);
                println!("[HEAP] ALLOCATOR.init() completed");

                let test_layout = core::alloc::Layout::from_size_align(8, 8)
                    .expect("Layout(8, 8) is always valid: size and align are both powers of two");
                let test_ptr = ALLOCATOR.alloc(test_layout);
                if !test_ptr.is_null() {
                    println!("[HEAP] Test allocation successful at {:p}", test_ptr);
                } else {
                    println!("[HEAP] WARNING: Test allocation failed!");
                }
            }

            println!("[HEAP] Initializing locked allocator...");
            let mut allocator = crate::get_allocator().lock();
            // SAFETY: `heap_start` and `heap_size` describe valid, owned memory
            // (the static HEAP_MEMORY array). The locked allocator's `init` requires
            // the memory region to be unused and exclusively available, which is
            // guaranteed because this runs once during single-threaded boot.
            unsafe {
                allocator.init(heap_start, heap_size);
            }
            #[allow(clippy::drop_non_drop)]
            drop(allocator);
            println!("[HEAP] RISC-V heap initialization complete");
        }

        // AArch64: Use lock-free UnsafeBumpAllocator (LockedHeap deadlocks on AArch64)
        // Initialize fields directly to avoid function call issues on AArch64
        #[cfg(target_arch = "aarch64")]
        {
            use core::sync::atomic::Ordering;

            use crate::arch::aarch64::direct_uart::uart_write_str;

            uart_write_str("[HEAP] Initializing AArch64 UnsafeBumpAllocator\n");

            let start_addr = heap_start as usize;

            // Initialize ALLOCATOR atomics directly (bypasses function call)
            crate::ALLOCATOR.start.store(start_addr, Ordering::SeqCst);
            crate::ALLOCATOR.size.store(heap_size, Ordering::SeqCst);
            crate::ALLOCATOR.next.store(start_addr, Ordering::SeqCst);
            crate::ALLOCATOR.allocations.store(0, Ordering::SeqCst);
            core::sync::atomic::fence(Ordering::SeqCst);

            // AArch64 memory barriers
            // SAFETY: DSB SY (Data Synchronization Barrier) and ISB (Instruction
            // Synchronization Barrier) are architectural barrier instructions that
            // are always safe to execute at any exception level. They ensure all
            // preceding memory operations complete before subsequent ones begin,
            // which is required after writing to the allocator's atomic fields so
            // that the allocator state is visible before any allocation attempts.
            unsafe {
                core::arch::asm!("dsb sy", "isb", options(nomem, nostack));
            }

            uart_write_str("[HEAP] ALLOCATOR initialized\n");

            // Verify allocator state
            let next_val = crate::ALLOCATOR.next.load(Ordering::SeqCst);
            if next_val != 0 {
                uart_write_str("[HEAP] Allocator state verified OK\n");
            } else {
                uart_write_str("[HEAP] WARNING: Allocator next=0\n");
            }

            // Initialize locked allocator too (direct field access)
            crate::LOCKED_ALLOCATOR
                .inner
                .start
                .store(start_addr, Ordering::SeqCst);
            crate::LOCKED_ALLOCATOR
                .inner
                .size
                .store(heap_size, Ordering::SeqCst);
            crate::LOCKED_ALLOCATOR
                .inner
                .next
                .store(start_addr, Ordering::SeqCst);
            crate::LOCKED_ALLOCATOR
                .inner
                .allocations
                .store(0, Ordering::SeqCst);
            core::sync::atomic::fence(Ordering::SeqCst);

            uart_write_str("[HEAP] AArch64 heap initialization complete\n");
        }

        // x86_64: Use LockedHeap
        // Note: The `init` call on LockedHeap is unsafe because it trusts the
        // caller to provide valid memory. The outer unsafe block already
        // establishes that `heap_start`/`heap_size` describe valid, exclusive
        // memory from the static HEAP_MEMORY array.
        #[cfg(target_arch = "x86_64")]
        {
            let mut allocator = crate::get_allocator().lock();
            allocator.init(heap_start, heap_size);
            drop(allocator);
        }

        println!(
            "[HEAP] Heap initialized: {} KB at {:p}",
            heap_size / 1024,
            core::ptr::addr_of!(HEAP_MEMORY)
        );
    }

    Ok(())
}

#[cfg(all(test, not(target_os = "none")))]
mod tests {
    use alloc::{boxed::Box, vec::Vec};

    use super::*;

    #[test]
    fn test_heap_allocation() {
        // Test various allocations
        let x = Box::new(42);
        assert_eq!(*x, 42);

        let mut v = Vec::new();
        for i in 0..100 {
            v.push(i);
        }
        assert_eq!(v.len(), 100);
    }

    #[test]
    fn test_size_classes() {
        // Test that size classes are powers of 2 or nice round numbers
        for &size in &SIZE_CLASSES {
            assert!(size >= 8);
            assert!(size <= 4096);
        }
    }
}
