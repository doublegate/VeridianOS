//! Process memory management
//!
//! This module handles the integration between processes and the memory
//! management subsystem, including virtual address space management and memory
//! mapping.

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::vec::Vec;

use crate::{
    error::KernelError,
    mm::{PageFlags, PhysicalAddress, VirtualAddress, PAGE_SIZE},
};

/// Memory layout constants for user processes
pub mod layout {
    /// User space start
    pub const USER_SPACE_START: usize = 0x0000_0000_0001_0000;

    /// User space end
    pub const USER_SPACE_END: usize = 0x0000_7FFF_FFFF_0000;

    /// Default code segment start
    pub const CODE_START: usize = 0x0000_0000_0040_0000;

    /// Default data segment start
    pub const DATA_START: usize = 0x0000_0000_0080_0000;

    /// Default heap start
    pub const HEAP_START: usize = 0x0000_0000_1000_0000;

    /// Maximum heap size (8GB) -- supports rustc self-compilation (4-8GB peak)
    pub const MAX_HEAP_SIZE: usize = 8 * 1024 * 1024 * 1024;

    /// Stack end address (grows down from here)
    pub const STACK_END: usize = 0x0000_7FFF_0000_0000;

    /// Default stack size (8MB)
    pub const DEFAULT_STACK_SIZE: usize = 8 * 1024 * 1024;
}

/// Process memory region types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryRegionType {
    /// Code segment (executable)
    Code,
    /// Data segment (read/write)
    Data,
    /// Read-only data
    Rodata,
    /// Stack region
    Stack,
    /// Heap region
    Heap,
    /// Memory-mapped file
    MappedFile,
    /// Shared memory
    Shared,
    /// Device memory
    Device,
}

/// Memory region in a process's address space
#[derive(Debug)]
pub struct MemoryRegion {
    /// Starting virtual address
    pub start: VirtualAddress,
    /// Ending virtual address (exclusive)
    pub end: VirtualAddress,
    /// Region type
    pub region_type: MemoryRegionType,
    /// Access permissions
    pub flags: PageFlags,
    /// Physical pages backing this region (if any)
    #[cfg(feature = "alloc")]
    pub physical_pages: Option<Vec<PhysicalAddress>>,
    /// File mapping info (if mapped file)
    pub file_mapping: Option<FileMapping>,
}

/// File mapping information
#[derive(Debug)]
pub struct FileMapping {
    /// File descriptor
    pub fd: u32,
    /// Offset in file
    pub offset: u64,
    /// Mapping flags
    pub flags: u32,
}

impl MemoryRegion {
    /// Create a new memory region
    pub fn new(
        start: VirtualAddress,
        size: usize,
        region_type: MemoryRegionType,
        flags: PageFlags,
    ) -> Self {
        Self {
            start,
            end: VirtualAddress::new(start.as_u64() + size as u64),
            region_type,
            flags,
            physical_pages: None,
            file_mapping: None,
        }
    }

    /// Get region size
    pub fn size(&self) -> usize {
        self.end.as_usize() - self.start.as_usize()
    }

    /// Check if address is within this region
    pub fn contains(&self, addr: VirtualAddress) -> bool {
        addr >= self.start && addr < self.end
    }

    /// Check if region overlaps with another
    pub fn overlaps(&self, other: &MemoryRegion) -> bool {
        self.start < other.end && other.start < self.end
    }
}

/// Process memory operations
pub trait ProcessMemory {
    /// Allocate memory in process address space
    fn allocate(&mut self, size: usize, flags: PageFlags) -> Result<VirtualAddress, KernelError>;

    /// Free memory in process address space
    fn free(&mut self, addr: VirtualAddress, size: usize) -> Result<(), KernelError>;

    /// Map physical memory into process address space
    fn map_physical(
        &mut self,
        phys: PhysicalAddress,
        virt: VirtualAddress,
        size: usize,
        flags: PageFlags,
    ) -> Result<(), KernelError>;

    /// Unmap memory from process address space
    fn unmap(&mut self, virt: VirtualAddress, size: usize) -> Result<(), KernelError>;

    /// Change memory protection
    fn protect(
        &mut self,
        addr: VirtualAddress,
        size: usize,
        flags: PageFlags,
    ) -> Result<(), KernelError>;

    /// Grow the heap
    fn grow_heap(&mut self, increment: usize) -> Result<VirtualAddress, KernelError>;

    /// Grow the stack
    fn grow_stack(&mut self, increment: usize) -> Result<(), KernelError>;
}

/// Stack management for threads
pub struct ThreadStack {
    /// Stack bottom (lowest address)
    pub bottom: VirtualAddress,
    /// Stack top (highest address)
    pub top: VirtualAddress,
    /// Current stack pointer
    pub sp: VirtualAddress,
    /// Guard page size
    pub guard_size: usize,
}

impl ThreadStack {
    /// Create a new thread stack
    pub fn new(size: usize) -> Result<Self, KernelError> {
        if size < PAGE_SIZE * 2 {
            return Err(KernelError::InvalidArgument {
                name: "stack size",
                value: "stack too small (minimum 2 pages)",
            });
        }

        // Allocate virtual address range for stack
        let bottom = VirtualAddress::new((layout::STACK_END - size) as u64);
        let top = VirtualAddress::new(layout::STACK_END as u64);

        Ok(Self {
            bottom,
            top,
            sp: top,
            guard_size: PAGE_SIZE,
        })
    }

    /// Get usable stack size (excluding guard page)
    pub fn usable_size(&self) -> usize {
        self.top.as_usize() - self.bottom.as_usize() - self.guard_size
    }

    /// Check if address is within stack
    pub fn contains(&self, addr: VirtualAddress) -> bool {
        addr >= self.bottom && addr <= self.top
    }

    /// Check if address is in guard page
    pub fn in_guard_page(&self, addr: VirtualAddress) -> bool {
        addr >= self.bottom
            && addr < VirtualAddress::new(self.bottom.as_u64() + self.guard_size as u64)
    }
}

/// Heap management for processes
pub struct ProcessHeap {
    /// Current heap break
    pub brk: VirtualAddress,
    /// Heap start
    pub start: VirtualAddress,
    /// Maximum heap size
    pub max_size: usize,
}

impl Default for ProcessHeap {
    fn default() -> Self {
        Self {
            brk: VirtualAddress::new(layout::HEAP_START as u64),
            start: VirtualAddress::new(layout::HEAP_START as u64),
            max_size: layout::MAX_HEAP_SIZE,
        }
    }
}

impl ProcessHeap {
    /// Create a new process heap
    pub fn new() -> Self {
        Self::default()
    }

    /// Get current heap size
    pub fn size(&self) -> usize {
        self.brk.as_usize() - self.start.as_usize()
    }

    /// Set heap break (brk syscall)
    pub fn set_brk(&mut self, new_brk: VirtualAddress) -> Result<VirtualAddress, KernelError> {
        let new_size = new_brk.as_usize() - self.start.as_usize();

        if new_size > self.max_size {
            return Err(KernelError::ResourceExhausted {
                resource: "heap size limit",
            });
        }

        if new_brk < self.start {
            return Err(KernelError::InvalidArgument {
                name: "heap break",
                value: "below heap start",
            });
        }

        // Note: Actual page allocation for heap expansion is handled by
        // VAS::brk() + brk_extend_heap() in mm/vas.rs, which is invoked by
        // sys_brk().  This ProcessHeap struct tracks the logical break only.
        self.brk = new_brk;
        Ok(self.brk)
    }

    /// Grow heap by increment
    pub fn grow(&mut self, increment: usize) -> Result<VirtualAddress, KernelError> {
        let new_brk = VirtualAddress::new((self.brk.as_usize() + increment) as u64);
        self.set_brk(new_brk)
    }
}

/// Memory mapping operations
pub mod mmap {
    use super::*;

    /// Memory mapping flags
    pub mod flags {
        /// Pages may be executed
        pub const PROT_EXEC: u32 = 0x4;
        /// Pages may be written
        pub const PROT_WRITE: u32 = 0x2;
        /// Pages may be read
        pub const PROT_READ: u32 = 0x1;
        /// Pages may not be accessed
        pub const PROT_NONE: u32 = 0x0;

        /// Share changes
        pub const MAP_SHARED: u32 = 0x01;
        /// Changes are private
        pub const MAP_PRIVATE: u32 = 0x02;
        /// Place mapping at exact address
        pub const MAP_FIXED: u32 = 0x10;
        /// Anonymous mapping (no file)
        pub const MAP_ANONYMOUS: u32 = 0x20;
    }

    /// Convert mmap protection flags to page flags
    pub fn prot_to_page_flags(prot: u32) -> PageFlags {
        let mut flags = PageFlags::PRESENT | PageFlags::USER;

        if prot & flags::PROT_WRITE != 0 {
            flags |= PageFlags::WRITABLE;
        }

        if prot & flags::PROT_EXEC == 0 {
            flags |= PageFlags::NO_EXECUTE;
        }

        flags
    }
}

/// Copy-on-write (COW) support
pub struct CowMapping {
    /// Original physical page
    pub original_page: PhysicalAddress,
    /// Reference count
    pub ref_count: core::sync::atomic::AtomicUsize,
}

impl CowMapping {
    /// Create a new COW mapping
    pub fn new(page: PhysicalAddress) -> Self {
        Self {
            original_page: page,
            ref_count: core::sync::atomic::AtomicUsize::new(1),
        }
    }

    /// Increment reference count
    pub fn inc_ref(&self) {
        self.ref_count
            .fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    }

    /// Decrement reference count
    pub fn dec_ref(&self) -> usize {
        self.ref_count
            .fetch_sub(1, core::sync::atomic::Ordering::Relaxed)
            - 1
    }

    /// Get reference count
    pub fn ref_count(&self) -> usize {
        self.ref_count.load(core::sync::atomic::Ordering::Relaxed)
    }
}
