//! Memory management subsystem
//!
//! This module handles physical and virtual memory management,
//! including page tables, allocators, and memory protection.

#![allow(dead_code)]

pub mod bootloader;
pub mod frame_allocator;
pub mod heap;
pub mod page_table;
pub mod user_validation;
pub mod vas;
pub mod vmm;

#[cfg(feature = "alloc")]
extern crate alloc;
#[cfg(feature = "alloc")]
use alloc::vec::Vec;

// Re-export commonly used types
pub use frame_allocator::{
    FrameAllocatorError, FrameNumber, PhysicalAddress, PhysicalFrame, FRAME_ALLOCATOR, FRAME_SIZE,
};
#[allow(unused_imports)]
pub use heap::init as init_heap;
pub use user_validation::{is_user_addr_valid, translate_address as translate_user_address};
pub use vas::VirtualAddressSpace;

/// Page size constant (4KB)
pub const PAGE_SIZE: usize = 4096;

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

    pub fn as_usize(&self) -> usize {
        self.0 as usize
    }

    pub fn add(&self, offset: usize) -> Self {
        Self(self.0 + offset as u64)
    }
}

/// Get kernel page table base address
pub fn get_kernel_page_table() -> usize {
    // Return the kernel page table base address
    // This would be architecture-specific
    #[cfg(target_arch = "x86_64")]
    {
        // CR3 holds the page table base
        let cr3: u64;
        unsafe {
            core::arch::asm!("mov {}, cr3", out(reg) cr3);
        }
        cr3 as usize
    }

    #[cfg(target_arch = "aarch64")]
    {
        // TTBR0_EL1 holds the page table base
        let ttbr0: u64;
        unsafe {
            core::arch::asm!("mrs {}, TTBR0_EL1", out(reg) ttbr0);
        }
        ttbr0 as usize
    }

    #[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
    {
        // SATP holds the page table base
        let satp: usize;
        unsafe {
            core::arch::asm!("csrr {}, satp", out(reg) satp);
        }
        // Extract PPN field (bits 43:0 on RV64)
        (satp & 0xFFF_FFFFFFFF) << 12
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

    // Alias for NO_EXECUTE
    pub const EXECUTABLE: Self = Self(0); // No NO_EXECUTE bit set

    pub fn contains(&self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }
}

impl core::ops::BitOr for PageFlags {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

impl core::ops::BitOrAssign for PageFlags {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
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
    #[cfg(target_arch = "aarch64")]
    {
        unsafe {
            let uart = 0x0900_0000 as *mut u8;
            *uart = b'M';
            *uart = b'M';
            *uart = b'S';
            *uart = b'T';
            *uart = b'A';
            *uart = b'R';
            *uart = b'T';
            *uart = b'\n';
        }
    }
    #[cfg(not(target_arch = "aarch64"))]
    println!("[MM] Initializing memory management...");

    // Initialize frame allocator with available memory regions
    #[cfg(not(target_arch = "aarch64"))]
    println!("[MM] Getting frame allocator lock...");

    #[cfg(target_arch = "aarch64")]
    {
        unsafe {
            let uart = 0x0900_0000 as *mut u8;
            *uart = b'L';
            *uart = b'O';
            *uart = b'C';
            *uart = b'K';
            *uart = b'1';
            *uart = b'\n';
        }
    }

    // For AArch64, skip the frame allocator initialization due to mutex issues
    #[cfg(target_arch = "aarch64")]
    {
        unsafe {
            let uart = 0x0900_0000 as *mut u8;
            *uart = b'S';
            *uart = b'K';
            *uart = b'I';
            *uart = b'P';
            *uart = b'\n';
        }

        // Also output MMFIN before returning
        unsafe {
            let uart = 0x0900_0000 as *mut u8;
            *uart = b'M';
            *uart = b'M';
            *uart = b'F';
            *uart = b'I';
            *uart = b'N';
            *uart = b'\n';
        }

        unsafe {
            let uart = 0x0900_0000 as *mut u8;
            *uart = b'R';
            *uart = b'E';
            *uart = b'T';
            *uart = b'\n';
        }

        return;
    }

    #[cfg(not(target_arch = "aarch64"))]
    let mut allocator = FRAME_ALLOCATOR.lock();

    #[cfg(not(target_arch = "aarch64"))]
    #[allow(unreachable_code)] // Required due to AArch64 early return
    {
        unsafe {
            let uart = 0x0900_0000 as *mut u8;
            *uart = b'L';
            *uart = b'O';
            *uart = b'C';
            *uart = b'K';
            *uart = b'2';
            *uart = b'\n';
        }
    }

    #[cfg(not(target_arch = "aarch64"))]
    println!("[MM] Got frame allocator lock");

    #[cfg(not(target_arch = "aarch64"))]
    let mut total_memory = 0u64;
    #[cfg(not(target_arch = "aarch64"))]
    let mut usable_memory = 0u64;

    // Skip allocator usage for AArch64
    #[cfg(target_arch = "aarch64")]
    #[allow(unreachable_code)] // Required due to early return
    {
        // Just mark completion
        unsafe {
            let uart = 0x0900_0000 as *mut u8;
            *uart = b'A';
            *uart = b'A';
            *uart = b'6';
            *uart = b'4';
            *uart = b'S';
            *uart = b'K';
            *uart = b'I';
            *uart = b'P';
            *uart = b'\n';
        }
    }

    #[cfg(not(target_arch = "aarch64"))]
    #[allow(unreachable_code)] // Required due to AArch64 early return
    for (idx, region) in memory_map.iter().enumerate() {
        println!(
            "[MM] Processing region {}: start=0x{:x}, size={} MB, usable={}",
            idx,
            region.start,
            region.size / (1024 * 1024),
            region.usable
        );

        total_memory += region.size;

        if region.usable {
            usable_memory += region.size;

            let start_frame = FrameNumber::new(region.start / FRAME_SIZE as u64);
            let frame_count = region.size as usize / FRAME_SIZE;

            // Use region index as NUMA node for now
            let numa_node = idx.min(7); // Max 8 NUMA nodes

            println!("[MM] About to call init_numa_node for node {}", numa_node);
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

    #[cfg(not(target_arch = "aarch64"))]
    #[allow(unreachable_code)] // Required due to AArch64 early return
    drop(allocator); // Release lock before getting stats
    #[cfg(target_arch = "aarch64")]
    {
        unsafe {
            let uart = 0x0900_0000 as *mut u8;
            *uart = b'M';
            *uart = b'M';
            *uart = b'F';
            *uart = b'I';
            *uart = b'N';
            *uart = b'\n';
        }
    }
    #[cfg(not(target_arch = "aarch64"))]
    println!("[MM] Allocator lock dropped");

    // Skip stats for now to avoid deadlock
    #[cfg(not(target_arch = "aarch64"))]
    println!(
        "[MM] Memory initialized: {} MB total, {} MB usable",
        total_memory / (1024 * 1024),
        usable_memory / (1024 * 1024)
    );
}

/// Initialize with default memory map for testing
pub fn init_default() {
    #[cfg(target_arch = "aarch64")]
    {
        unsafe {
            let uart = 0x0900_0000 as *mut u8;
            *uart = b'M';
            *uart = b'M';
            *uart = b'D';
            *uart = b'E';
            *uart = b'F';
            *uart = b'\n';
        }
        // Early return for AArch64 to avoid any issues
        return;
    }
    #[cfg(not(target_arch = "aarch64"))]
    println!("[MM] init_default called");

    // Architecture-specific default memory maps
    #[cfg(target_arch = "x86_64")]
    let default_map = [MemoryRegion {
        start: 0x100000,         // 1MB
        size: 128 * 1024 * 1024, // 128MB
        usable: true,
    }];

    #[cfg(target_arch = "aarch64")]
    #[allow(unreachable_code)] // We return early, but need this for compilation
    let default_map = [MemoryRegion {
        start: 0x48000000, // 1.125GB (after kernel at 0x40080000)
        size: 134217728,   // 128MB pre-calculated
        usable: true,
    }];

    #[cfg(target_arch = "riscv64")]
    let default_map = [MemoryRegion {
        start: 0x88000000,       // 2.125GB (after kernel at 0x80200000)
        size: 128 * 1024 * 1024, // 128MB
        usable: true,
    }];

    #[cfg(target_arch = "aarch64")]
    #[allow(unreachable_code)] // Required due to early return
    {
        unsafe {
            let uart = 0x0900_0000 as *mut u8;
            *uart = b'M';
            *uart = b'M';
            *uart = b'I';
            *uart = b'N';
            *uart = b'I';
            *uart = b'T';
            *uart = b'\n';
        }
    }
    #[cfg(not(target_arch = "aarch64"))]
    println!("[MM] Calling init with default memory map");

    #[cfg(target_arch = "aarch64")]
    {
        unsafe {
            let uart = 0x0900_0000 as *mut u8;
            *uart = b'B';
            *uart = b'4';
            *uart = b'I';
            *uart = b'N';
            *uart = b'I';
            *uart = b'T';
            *uart = b'\n';
        }
    }

    init(&default_map);

    #[cfg(target_arch = "aarch64")]
    {
        unsafe {
            let uart = 0x0900_0000 as *mut u8;
            *uart = b'A';
            *uart = b'F';
            *uart = b'T';
            *uart = b'E';
            *uart = b'R';
            *uart = b'\n';
            *uart = b'M';
            *uart = b'M';
            *uart = b'O';
            *uart = b'K';
            *uart = b'\n';
        }
    }
    #[cfg(not(target_arch = "aarch64"))]
    println!("[MM] init returned successfully");

    #[cfg(target_arch = "aarch64")]
    {
        unsafe {
            let uart = 0x0900_0000 as *mut u8;
            *uart = b'E';
            *uart = b'N';
            *uart = b'D';
            *uart = b'D';
            *uart = b'E';
            *uart = b'F';
            *uart = b'\n';
        }
    }
}

/// Translate virtual address to physical address
pub fn translate_address(
    vas: &VirtualAddressSpace,
    vaddr: VirtualAddress,
) -> Option<PhysicalAddress> {
    // Find the mapping for this virtual address
    #[cfg(feature = "alloc")]
    {
        if let Some(mapping) = vas.find_mapping(vaddr) {
            // Calculate offset within the mapping
            let offset = vaddr.0 - mapping.start.0;
            let page_index = (offset / PAGE_SIZE as u64) as usize;

            // Check if we have physical frames allocated
            if page_index < mapping.physical_frames.len() {
                let frame = mapping.physical_frames[page_index];
                let page_offset = offset % PAGE_SIZE as u64;
                return Some(PhysicalAddress::new(frame.as_addr().as_u64() + page_offset));
            }
        }
    }

    None
}

/// Free a physical frame
pub fn free_frame(frame: PhysicalAddress) {
    let frame_num = FrameNumber::new(frame.as_u64() / FRAME_SIZE as u64);
    let _ = FRAME_ALLOCATOR.lock().free_frames(frame_num, 1);
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
