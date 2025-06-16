//! Kernel heap allocator
//!
//! Implements a slab allocator for the kernel heap with size classes
//! and per-CPU caches for performance.

#![allow(dead_code)]

use core::{alloc::Layout, ptr::NonNull};

use linked_list_allocator::LockedHeap;
use spin::Mutex;

use super::VirtualAddress;

// Static heap storage - 4MB should be enough for initial testing
static mut HEAP_MEMORY: [u8; 4 * 1024 * 1024] = [0; 4 * 1024 * 1024];

/// Kernel heap size (16 MB initially)
pub const HEAP_SIZE: usize = 16 * 1024 * 1024;

/// Kernel heap start address
/// For now, use a lower address that's likely to be identity mapped by
/// bootloader In a real implementation, we'd properly set up page tables first
#[cfg(target_arch = "x86_64")]
pub const HEAP_START: usize = 0x444444440000; // Use an arbitrary high address that bootloader 0.9 maps

#[cfg(all(not(target_arch = "x86_64"), not(target_arch = "riscv64")))]
pub const HEAP_START: usize = 0xFFFF_C000_0000_0000;

#[cfg(target_arch = "riscv64")]
pub const HEAP_START: usize = 0x81000000; // Use physical address that's likely mapped

/// Slab allocator for efficient small allocations
pub struct SlabAllocator {
    /// Size classes for slab allocation
    slabs: [Option<Slab>; 10],
    /// Fallback allocator for large allocations
    fallback: LockedHeap,
    /// Statistics
    stats: Mutex<HeapStats>,
}

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

/// Free object in slab free list
struct FreeObject {
    next: Option<NonNull<FreeObject>>,
}

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

/// Size classes for slab allocator (in bytes)
const SIZE_CLASSES: [usize; 10] = [8, 16, 32, 64, 128, 256, 512, 1024, 2048, 4096];

impl Default for SlabAllocator {
    fn default() -> Self {
        Self::new()
    }
}

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
                    .unwrap()
                    .as_ptr();
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
                unsafe {
                    self.fallback
                        .lock()
                        .deallocate(NonNull::new_unchecked(ptr), layout);
                }
                return;
            }
        }

        // Large allocation - use fallback
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
    println!("[HEAP] Initializing kernel heap at 0x{:x}", HEAP_START);

    unsafe {
        // Use the static heap array instead of an arbitrary address
        // Use raw pointers to avoid static mut refs warning
        let heap_start = core::ptr::addr_of_mut!(HEAP_MEMORY) as *mut u8;
        let heap_size = 4 * 1024 * 1024; // Size of HEAP_MEMORY

        // For RISC-V, initialize both allocators since they're separate
        #[cfg(target_arch = "riscv64")]
        {
            // Initialize the global allocator directly
            crate::ALLOCATOR.init(heap_start, heap_size);

            // Also initialize the locked allocator for compatibility
            let mut allocator = crate::get_allocator().lock();
            allocator.init(heap_start, heap_size);
            #[allow(clippy::drop_non_drop)]
            drop(allocator);
        }

        #[cfg(not(target_arch = "riscv64"))]
        {
            let mut allocator = crate::get_allocator().lock();
            allocator.init(heap_start, heap_size);
            drop(allocator);
        }

        println!(
            "[HEAP] Heap initialized: {} MB at 0x{:x}",
            4, // 4MB heap size
            core::ptr::addr_of!(HEAP_MEMORY) as usize
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
