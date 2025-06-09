//! Memory management subsystem
//!
//! This module handles physical and virtual memory management,
//! including page tables, allocators, and memory protection.

#![allow(dead_code)]

pub mod frame_allocator;

// Re-export PhysicalAddress from frame_allocator to avoid conflicts
pub use frame_allocator::{
    FrameAllocatorError, FrameNumber, PhysicalAddress, FRAME_ALLOCATOR, FRAME_SIZE,
};

/// Virtual memory address
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct VirtualAddress(pub u64);

impl VirtualAddress {
    pub fn new(addr: u64) -> Self {
        Self(addr)
    }

    pub fn as_u64(&self) -> u64 {
        self.0
    }

    pub fn add(&self, offset: usize) -> Self {
        Self(self.0 + offset as u64)
    }
}

/// Page size options
#[repr(usize)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageSize {
    /// 4 KiB pages
    Small = 4096,
    /// 2 MiB pages (x86_64) / 2 MiB (AArch64)
    Large = 2 * 1024 * 1024,
    /// 1 GiB pages (x86_64) / 1 GiB (AArch64)
    Huge = 1024 * 1024 * 1024,
}

/// Page table structure (placeholder)
pub struct PageTable {
    pub root_phys: PhysicalAddress,
}

/// Page flags
#[derive(Debug, Clone, Copy)]
pub struct PageFlags(u64);

impl PageFlags {
    pub const PRESENT: Self = Self(1 << 0);
    pub const WRITABLE: Self = Self(1 << 1);
    pub const USER: Self = Self(1 << 2);
    pub const WRITE_THROUGH: Self = Self(1 << 3);
    pub const NO_CACHE: Self = Self(1 << 4);
    pub const ACCESSED: Self = Self(1 << 5);
    pub const DIRTY: Self = Self(1 << 6);
    pub const HUGE: Self = Self(1 << 7);
    pub const GLOBAL: Self = Self(1 << 8);
    pub const NO_EXECUTE: Self = Self(1 << 63);
}

impl core::ops::BitOr for PageFlags {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

/// Memory region from bootloader/firmware
#[derive(Debug, Clone, Copy)]
pub struct MemoryRegion {
    pub start: u64,
    pub size: u64,
    pub usable: bool,
}

/// Initialize the memory management subsystem
#[cfg_attr(not(target_arch = "x86_64"), allow(unused_variables))]
pub fn init(memory_map: &[MemoryRegion]) {
    println!("[MM] Initializing memory management...");

    // Initialize frame allocator with available memory regions
    let mut allocator = FRAME_ALLOCATOR.lock();

    let mut total_memory = 0u64;
    let mut usable_memory = 0u64;

    for (idx, region) in memory_map.iter().enumerate() {
        total_memory += region.size;

        if region.usable {
            usable_memory += region.size;

            let start_frame = FrameNumber::new(region.start / FRAME_SIZE as u64);
            let frame_count = region.size as usize / FRAME_SIZE;

            // Use region index as NUMA node for now
            let numa_node = idx.min(7); // Max 8 NUMA nodes

            if let Err(_e) = allocator.init_numa_node(numa_node, start_frame, frame_count) {
                println!("[MM] Warning: Failed to initialize memory region {}", idx);
            } else {
                println!(
                    "[MM] Initialized {} MB at 0x{:x} (NUMA node {})",
                    region.size / (1024 * 1024),
                    region.start,
                    numa_node
                );
            }
        }
    }

    drop(allocator); // Release lock before getting stats

    let stats = FRAME_ALLOCATOR.lock().get_stats();
    println!(
        "[MM] Memory initialized: {} MB total, {} MB usable, {} MB available",
        total_memory / (1024 * 1024),
        usable_memory / (1024 * 1024),
        (stats.free_frames * FRAME_SIZE as u64) / (1024 * 1024)
    );
}

/// Initialize with default memory map for testing
pub fn init_default() {
    // Default memory map for testing (128MB starting at 1MB)
    let default_map = [MemoryRegion {
        start: 0x100000,         // 1MB
        size: 128 * 1024 * 1024, // 128MB
        usable: true,
    }];

    init(&default_map);
}

/// Placeholder types for IPC integration
pub type PagePermissions = PageFlags;
pub type PhysicalPage = FrameNumber;

/// Allocate physical pages
pub fn allocate_pages(
    count: usize,
    numa_node: Option<usize>,
) -> Result<Vec<PhysicalPage>, FrameAllocatorError> {
    let frame = FRAME_ALLOCATOR.lock().allocate_frames(count, numa_node)?;

    // Return a vector of consecutive frame numbers
    let mut pages = Vec::with_capacity(count);
    for i in 0..count {
        pages.push(FrameNumber::new(frame.as_u64() + i as u64));
    }

    Ok(pages)
}

/// Free physical pages
pub fn free_pages(pages: &[PhysicalPage]) -> Result<(), FrameAllocatorError> {
    if pages.is_empty() {
        return Ok(());
    }

    // Assume pages are contiguous for now
    let first_frame = pages[0];
    let count = pages.len();

    FRAME_ALLOCATOR.lock().free_frames(first_frame, count)
}

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::vec::Vec;
