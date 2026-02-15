//! Virtual Address Space management
//!
//! Manages virtual memory for processes including page tables,
//! memory mappings, and address space operations.

#![allow(clippy::manual_div_ceil)]

use core::sync::atomic::{AtomicU64, Ordering};

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::{collections::BTreeMap, vec::Vec};

use spin::Mutex;

use super::{
    page_table::{FrameAllocator as PageFrameAllocator, PageMapper},
    FrameAllocatorError, FrameNumber, PageFlags, VirtualAddress, FRAME_ALLOCATOR,
};
use crate::error::KernelError;

/// Frame allocator wrapper implementing the page_table::FrameAllocator trait.
/// Delegates to the global FRAME_ALLOCATOR.
struct VasFrameAllocator;

impl PageFrameAllocator for VasFrameAllocator {
    fn allocate_frames(
        &mut self,
        count: usize,
        numa_node: Option<usize>,
    ) -> Result<FrameNumber, FrameAllocatorError> {
        FRAME_ALLOCATOR.lock().allocate_frames(count, numa_node)
    }
}

/// Create a PageMapper from a page table root physical address.
///
/// # Safety
///
/// The `page_table_root` must be a valid physical address of a properly
/// initialized L4 page table. The physical address must be identity-mapped
/// or accessible via the kernel's physical memory map so that it can be
/// dereferenced as a pointer. The caller must ensure exclusive access to
/// the page table hierarchy for the duration of the returned PageMapper's
/// use.
unsafe fn create_mapper_from_root(page_table_root: u64) -> PageMapper {
    let l4_ptr = page_table_root as *mut super::page_table::PageTable;
    // SAFETY: The caller guarantees that `page_table_root` is a valid,
    // identity-mapped physical address of an L4 page table. The pointer
    // is valid for the lifetime of this PageMapper usage.
    unsafe { PageMapper::new(l4_ptr) }
}

/// Memory mapping types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MappingType {
    /// Code segment (executable)
    Code,
    /// Data segment (read/write)
    Data,
    /// Stack segment
    Stack,
    /// Heap segment
    Heap,
    /// Memory-mapped file
    File,
    /// Shared memory
    Shared,
    /// Device memory (no caching)
    Device,
}

/// Virtual memory mapping
#[derive(Debug, Clone)]
pub struct VirtualMapping {
    /// Start address
    pub start: VirtualAddress,
    /// Size in bytes
    pub size: usize,
    /// Mapping type
    pub mapping_type: MappingType,
    /// Page flags
    pub flags: PageFlags,
    /// Backing physical frames (if mapped)
    #[cfg(feature = "alloc")]
    pub physical_frames: Vec<super::FrameNumber>,
}

impl VirtualMapping {
    /// Create a new virtual mapping
    pub fn new(start: VirtualAddress, size: usize, mapping_type: MappingType) -> Self {
        let flags = match mapping_type {
            MappingType::Code => PageFlags::PRESENT | PageFlags::USER,
            MappingType::Data => PageFlags::PRESENT | PageFlags::WRITABLE | PageFlags::USER,
            MappingType::Stack => {
                PageFlags::PRESENT | PageFlags::WRITABLE | PageFlags::USER | PageFlags::NO_EXECUTE
            }
            MappingType::Heap => {
                PageFlags::PRESENT | PageFlags::WRITABLE | PageFlags::USER | PageFlags::NO_EXECUTE
            }
            MappingType::File => PageFlags::PRESENT | PageFlags::USER,
            MappingType::Shared => PageFlags::PRESENT | PageFlags::WRITABLE | PageFlags::USER,
            MappingType::Device => PageFlags::PRESENT | PageFlags::WRITABLE | PageFlags::NO_CACHE,
        };

        Self {
            start,
            size,
            mapping_type,
            flags,
            #[cfg(feature = "alloc")]
            physical_frames: Vec::new(),
        }
    }

    /// Check if address is within this mapping
    pub fn contains(&self, addr: VirtualAddress) -> bool {
        addr.0 >= self.start.0 && addr.0 < self.start.0 + self.size as u64
    }

    /// Get end address
    pub fn end(&self) -> VirtualAddress {
        VirtualAddress(self.start.0 + self.size as u64)
    }
}

/// Virtual Address Space for a process
pub struct VirtualAddressSpace {
    /// Page table root (CR3 on x86_64)
    pub page_table_root: AtomicU64,

    /// Virtual memory mappings
    #[cfg(feature = "alloc")]
    mappings: Mutex<BTreeMap<VirtualAddress, VirtualMapping>>,

    /// Next free address for mmap
    next_mmap_addr: AtomicU64,

    /// Heap start and current break
    heap_start: AtomicU64,
    heap_break: AtomicU64,

    /// Stack top (grows down)
    stack_top: AtomicU64,
}

impl Default for VirtualAddressSpace {
    fn default() -> Self {
        Self {
            page_table_root: AtomicU64::new(0),
            #[cfg(feature = "alloc")]
            mappings: Mutex::new(BTreeMap::new()),
            // Start mmap region at 0x4000_0000_0000 (256GB)
            next_mmap_addr: AtomicU64::new(0x4000_0000_0000),
            // Heap starts at 0x2000_0000_0000 (128GB)
            heap_start: AtomicU64::new(0x2000_0000_0000),
            heap_break: AtomicU64::new(0x2000_0000_0000),
            // Stack starts at 0x7FFF_FFFF_0000 and grows down
            stack_top: AtomicU64::new(0x7FFF_FFFF_0000),
        }
    }
}

impl VirtualAddressSpace {
    /// Create a new virtual address space
    pub fn new() -> Self {
        Self::default()
    }

    /// Initialize virtual address space
    pub fn init(&mut self) -> Result<(), KernelError> {
        use super::page_table::PageTableHierarchy;

        // Allocate L4 page table
        let page_table = PageTableHierarchy::new()?;
        self.page_table_root
            .store(page_table.l4_addr().as_u64(), Ordering::Release);

        // Map kernel space
        self.map_kernel_space()?;

        Ok(())
    }

    /// Map kernel space into this address space
    pub fn map_kernel_space(&mut self) -> Result<(), KernelError> {
        // Kernel space is at 0xFFFF_8000_0000_0000 and above
        // This is typically shared across all address spaces
        // For now, we'll just record the mapping - actual page table updates
        // would happen through the PageMapper

        #[cfg(feature = "alloc")]
        {
            // Map kernel code region (read-only, executable)
            self.map_region(
                VirtualAddress(0xFFFF_8000_0000_0000),
                0x200000, // 2MB for kernel code
                MappingType::Code,
            )?;

            // Map kernel data region (read-write, no-execute)
            self.map_region(
                VirtualAddress(0xFFFF_8000_0020_0000),
                0x200000, // 2MB for kernel data
                MappingType::Data,
            )?;

            // Map kernel heap region (reduced from 256MB to 16MB)
            self.map_region(
                VirtualAddress(0xFFFF_C000_0000_0000),
                0x1000000, // 16MB for kernel heap (instead of 256MB)
                MappingType::Heap,
            )?;
        }

        Ok(())
    }

    /// Clone from another address space
    pub fn clone_from(&mut self, other: &Self) -> Result<(), KernelError> {
        // Copy page table root
        self.page_table_root.store(
            other.page_table_root.load(Ordering::Acquire),
            Ordering::Release,
        );

        // Clone mappings
        #[cfg(feature = "alloc")]
        {
            let other_mappings = other.mappings.lock();
            let mut self_mappings = self.mappings.lock();
            self_mappings.clear();
            for (k, v) in other_mappings.iter() {
                self_mappings.insert(*k, v.clone());
            }
        }

        // Copy other state
        self.heap_start
            .store(other.heap_start.load(Ordering::Relaxed), Ordering::Relaxed);
        self.heap_break
            .store(other.heap_break.load(Ordering::Relaxed), Ordering::Relaxed);
        self.stack_top
            .store(other.stack_top.load(Ordering::Relaxed), Ordering::Relaxed);
        self.next_mmap_addr.store(
            other.next_mmap_addr.load(Ordering::Relaxed),
            Ordering::Relaxed,
        );

        Ok(())
    }

    /// Destroy the address space
    pub fn destroy(&mut self) {
        #[cfg(feature = "alloc")]
        {
            let pt_root = self.page_table_root.load(Ordering::Acquire);

            // First unmap all regions from page tables and free physical frames
            let mut mappings = self.mappings.lock();

            // Unmap from architecture page tables if we have a valid root
            if pt_root != 0 {
                // SAFETY: `pt_root` is a non-zero physical address of an L4
                // page table set during VAS::init(). The address is identity-
                // mapped in the kernel's physical memory window. We have
                // `&mut self`, ensuring exclusive access.
                let mut mapper = unsafe { create_mapper_from_root(pt_root) };

                for (_, mapping) in mappings.iter() {
                    let num_pages = mapping.size / 4096;
                    for i in 0..num_pages {
                        let vaddr = VirtualAddress(mapping.start.0 + (i as u64) * 4096);
                        let _ = mapper.unmap_page(vaddr);
                    }
                }
            }

            // Free physical frames for each mapping
            for (_, mapping) in mappings.iter() {
                let allocator = FRAME_ALLOCATOR.lock();
                for &frame in &mapping.physical_frames {
                    let _ = allocator.free_frames(frame, 1);
                }
            }

            // Clear all mappings
            mappings.clear();

            // Flush entire TLB since we destroyed the whole address space
            crate::arch::tlb_flush_all();

            // TODO(future): Free page table structures themselves by walking
            // the hierarchy and freeing intermediate table pages.
        }
    }

    /// Set page table root
    pub fn set_page_table(&self, root_phys_addr: u64) {
        self.page_table_root
            .store(root_phys_addr, Ordering::Release);
    }

    /// Get page table root
    pub fn get_page_table(&self) -> u64 {
        self.page_table_root.load(Ordering::Acquire)
    }

    /// Map a region of virtual memory
    #[cfg(feature = "alloc")]
    pub fn map_region(
        &self,
        start: VirtualAddress,
        size: usize,
        mapping_type: MappingType,
    ) -> Result<(), KernelError> {
        // Align to page boundary
        let aligned_start = VirtualAddress(start.0 & !(4096 - 1));
        let aligned_size = ((size + 4095) / 4096) * 4096;

        let mapping = VirtualMapping::new(aligned_start, aligned_size, mapping_type);

        let mut mappings = self.mappings.lock();

        // Check for overlaps
        for (_, existing) in mappings.iter() {
            if existing.contains(aligned_start) || existing.contains(mapping.end()) {
                return Err(KernelError::AlreadyExists {
                    resource: "address range",
                    id: aligned_start.0,
                });
            }
        }

        // Allocate physical frames for the mapping
        let num_pages = aligned_size / 4096;
        let mut physical_frames = Vec::with_capacity(num_pages);

        // Allocate all frames first (hold FRAME_ALLOCATOR lock briefly)
        {
            let frame_allocator = FRAME_ALLOCATOR.lock();
            for _ in 0..num_pages {
                let frame = frame_allocator.allocate_frames(1, None).map_err(|_| {
                    KernelError::OutOfMemory {
                        requested: 4096,
                        available: 0,
                    }
                })?;
                physical_frames.push(frame);
            }
        } // Drop frame allocator lock before page table operations

        // Wire mappings into the architecture page table
        let pt_root = self.page_table_root.load(Ordering::Acquire);
        if pt_root != 0 {
            // SAFETY: `pt_root` is a non-zero physical address of an L4 page
            // table that was set during VAS::init() or inherited from a valid
            // parent address space. The address is identity-mapped in the
            // kernel's physical memory window. We hold the mappings lock,
            // ensuring exclusive page table modification for this VAS.
            let mut mapper = unsafe { create_mapper_from_root(pt_root) };
            let mut alloc = VasFrameAllocator;

            for (i, &frame) in physical_frames.iter().enumerate() {
                let vaddr = VirtualAddress(aligned_start.0 + (i as u64) * 4096);
                // Intermediate page tables may need frame allocation, which
                // VasFrameAllocator provides by locking FRAME_ALLOCATOR
                // internally. This is safe because we already dropped our
                // earlier lock on FRAME_ALLOCATOR above.
                mapper.map_page(vaddr, frame, mapping.flags, &mut alloc)?;
            }

            // Flush TLB for the entire mapped range
            for i in 0..num_pages {
                let vaddr = aligned_start.0 + (i as u64) * 4096;
                crate::arch::tlb_flush_address(vaddr);
            }
        }

        // Record the mapping in our tracking structure
        let mut mapping = mapping;
        mapping.physical_frames = physical_frames;

        mappings.insert(aligned_start, mapping);
        Ok(())
    }

    /// Map a region of virtual memory with RAII guard
    #[cfg(feature = "alloc")]
    pub fn map_region_raii(
        &self,
        start: VirtualAddress,
        size: usize,
        mapping_type: MappingType,
        process_id: crate::process::ProcessId,
    ) -> Result<crate::raii::MappedRegion, KernelError> {
        // First map the region normally
        self.map_region(start, size, mapping_type)?;

        // Create RAII guard for automatic unmapping
        let aligned_start = VirtualAddress(start.0 & !(4096 - 1));
        let aligned_size = ((size + 4095) / 4096) * 4096;

        Ok(crate::raii::MappedRegion::new(
            aligned_start.as_usize(),
            aligned_size,
            process_id,
        ))
    }

    /// Unmap a region
    #[cfg(feature = "alloc")]
    pub fn unmap_region(&self, start: VirtualAddress) -> Result<(), KernelError> {
        let mut mappings = self.mappings.lock();
        let mapping = mappings.remove(&start).ok_or(KernelError::NotFound {
            resource: "memory region",
            id: start.0,
        })?;

        let num_pages = mapping.size / 4096;

        // Unmap each page from the architecture page table
        let pt_root = self.page_table_root.load(Ordering::Acquire);
        if pt_root != 0 {
            // SAFETY: `pt_root` is a non-zero physical address of an L4 page
            // table set during VAS::init(). The address is identity-mapped in
            // the kernel's physical memory window. We hold the mappings lock,
            // ensuring exclusive page table modification for this VAS.
            let mut mapper = unsafe { create_mapper_from_root(pt_root) };

            for i in 0..num_pages {
                let vaddr = VirtualAddress(mapping.start.0 + (i as u64) * 4096);
                // Ignore errors from unmap_page -- the page may not have been
                // installed in the hardware table (e.g., if map_region was
                // called before the page table root was set).
                let _ = mapper.unmap_page(vaddr);
            }
        }

        // Flush TLB for each page in the unmapped range
        for i in 0..num_pages {
            let vaddr = mapping.start.0 + (i as u64) * 4096;
            crate::arch::tlb_flush_address(vaddr);
        }

        // Free the physical frames
        let frame_allocator = FRAME_ALLOCATOR.lock();
        for frame in mapping.physical_frames {
            let _ = frame_allocator.free_frames(frame, 1);
        }

        Ok(())
    }

    /// Unmap a region by address
    #[cfg(feature = "alloc")]
    pub fn unmap(&self, start_addr: usize, _size: usize) -> Result<(), KernelError> {
        self.unmap_region(VirtualAddress(start_addr as u64))
    }

    /// Find mapping for address
    #[cfg(feature = "alloc")]
    pub fn find_mapping(&self, addr: VirtualAddress) -> Option<VirtualMapping> {
        let mappings = self.mappings.lock();
        for (_, mapping) in mappings.iter() {
            if mapping.contains(addr) {
                return Some(mapping.clone());
            }
        }
        None
    }

    /// Allocate memory-mapped region
    pub fn mmap(
        &self,
        size: usize,
        mapping_type: MappingType,
    ) -> Result<VirtualAddress, KernelError> {
        let aligned_size = ((size + 4095) / 4096) * 4096;
        let addr = VirtualAddress(
            self.next_mmap_addr
                .fetch_add(aligned_size as u64, Ordering::Relaxed),
        );

        #[cfg(feature = "alloc")]
        self.map_region(addr, aligned_size, mapping_type)?;

        Ok(addr)
    }

    /// Extend heap (brk)
    pub fn brk(&self, new_break: Option<VirtualAddress>) -> VirtualAddress {
        if let Some(addr) = new_break {
            // Try to set new break
            let current = self.heap_break.load(Ordering::Acquire);
            if addr.0 >= self.heap_start.load(Ordering::Relaxed) && addr.0 > current {
                self.heap_break.store(addr.0, Ordering::Release);
            }
        }

        VirtualAddress(self.heap_break.load(Ordering::Acquire))
    }

    /// Clone address space (for fork)
    #[cfg(feature = "alloc")]
    pub fn fork(&self) -> Result<Self, KernelError> {
        let new_vas = Self::new();

        // Clone all mappings
        {
            let mappings = self.mappings.lock();
            let mut new_mappings = new_vas.mappings.lock();

            for (addr, mapping) in mappings.iter() {
                new_mappings.insert(*addr, mapping.clone());
            }
        } // Drop locks here

        // Copy metadata
        new_vas
            .heap_start
            .store(self.heap_start.load(Ordering::Relaxed), Ordering::Relaxed);
        new_vas
            .heap_break
            .store(self.heap_break.load(Ordering::Relaxed), Ordering::Relaxed);
        new_vas
            .stack_top
            .store(self.stack_top.load(Ordering::Relaxed), Ordering::Relaxed);
        new_vas.next_mmap_addr.store(
            self.next_mmap_addr.load(Ordering::Relaxed),
            Ordering::Relaxed,
        );

        Ok(new_vas)
    }

    /// Handle page fault
    pub fn handle_page_fault(
        &self,
        fault_addr: VirtualAddress,
        write: bool,
        user: bool,
    ) -> Result<(), KernelError> {
        #[cfg(feature = "alloc")]
        {
            // Find the mapping for this address
            let mapping = self
                .find_mapping(fault_addr)
                .ok_or(KernelError::UnmappedMemory {
                    addr: fault_addr.0 as usize,
                })?;

            // Check permissions
            if write && !mapping.flags.contains(PageFlags::WRITABLE) {
                return Err(KernelError::PermissionDenied {
                    operation: "write to read-only page",
                });
            }

            if user && !mapping.flags.contains(PageFlags::USER) {
                return Err(KernelError::PermissionDenied {
                    operation: "user access to kernel page",
                });
            }

            // Check if this is a valid fault (e.g., COW, demand paging)
            // For now, we'll just return an error as we don't support these features yet
            Err(KernelError::NotImplemented {
                feature: "page fault handling (COW/demand paging)",
            })
        }

        #[cfg(not(feature = "alloc"))]
        Err(KernelError::NotImplemented {
            feature: "page fault handling (requires alloc)",
        })
    }

    /// Get memory statistics
    #[cfg(feature = "alloc")]
    pub fn get_stats(&self) -> VasStats {
        let mappings = self.mappings.lock();
        let mut total_size = 0;
        let mut code_size = 0;
        let mut data_size = 0;
        let mut stack_size = 0;
        let mut heap_size = 0;

        for (_, mapping) in mappings.iter() {
            total_size += mapping.size;
            match mapping.mapping_type {
                MappingType::Code => code_size += mapping.size,
                MappingType::Data => data_size += mapping.size,
                MappingType::Stack => stack_size += mapping.size,
                MappingType::Heap => heap_size += mapping.size,
                _ => {}
            }
        }

        VasStats {
            total_size,
            code_size,
            data_size,
            stack_size,
            heap_size,
            mapping_count: mappings.len(),
        }
    }

    /// Clear all mappings and free resources
    pub fn clear(&mut self) {
        #[cfg(feature = "alloc")]
        {
            let pt_root = self.page_table_root.load(Ordering::Acquire);

            // Get all mappings to free their frames
            let mappings = self.mappings.get_mut();

            // Unmap from architecture page tables if we have a valid root
            if pt_root != 0 {
                // SAFETY: `pt_root` is a non-zero physical address of an L4
                // page table set during VAS::init(). The address is identity-
                // mapped in the kernel's physical memory window. We have
                // `&mut self`, ensuring exclusive access.
                let mut mapper = unsafe { create_mapper_from_root(pt_root) };

                for (_, mapping) in mappings.iter() {
                    let num_pages = mapping.size / 4096;
                    for i in 0..num_pages {
                        let vaddr = VirtualAddress(mapping.start.0 + (i as u64) * 4096);
                        let _ = mapper.unmap_page(vaddr);
                    }
                }
            }

            // Free physical frames for each mapping
            for (_, mapping) in mappings.iter() {
                let frame_allocator = FRAME_ALLOCATOR.lock();
                for frame in &mapping.physical_frames {
                    frame_allocator.free_frames(*frame, 1).ok();
                }
            }

            // Clear all mappings
            mappings.clear();

            // Flush entire TLB since we cleared all mappings
            crate::arch::tlb_flush_all();
        }

        // Reset metadata
        self.heap_break
            .store(self.heap_start.load(Ordering::Relaxed), Ordering::Release);
        self.next_mmap_addr
            .store(0x4000_0000_0000, Ordering::Release);

        // TODO(future): Free page table structures
    }

    /// Clear user-space mappings only (for exec)
    pub fn clear_user_space(&mut self) -> Result<(), KernelError> {
        #[cfg(feature = "alloc")]
        {
            let pt_root = self.page_table_root.load(Ordering::Acquire);
            let mappings = self.mappings.get_mut();
            let mut to_remove = Vec::new();

            // Find all user-space mappings (below kernel space)
            const KERNEL_SPACE_START: u64 = 0xFFFF_8000_0000_0000;

            for (addr, _mapping) in mappings.iter() {
                if addr.0 < KERNEL_SPACE_START {
                    to_remove.push(*addr);
                }
            }

            // Unmap user-space pages from architecture page tables
            if pt_root != 0 {
                // SAFETY: `pt_root` is a non-zero physical address of an L4
                // page table set during VAS::init(). The address is identity-
                // mapped in the kernel's physical memory window. We have
                // `&mut self`, ensuring exclusive access.
                let mut mapper = unsafe { create_mapper_from_root(pt_root) };

                for addr in &to_remove {
                    if let Some(mapping) = mappings.get(addr) {
                        let num_pages = mapping.size / 4096;
                        for i in 0..num_pages {
                            let vaddr = VirtualAddress(mapping.start.0 + (i as u64) * 4096);
                            let _ = mapper.unmap_page(vaddr);
                        }
                    }
                }
            }

            // Free physical frames and remove mappings
            for addr in &to_remove {
                if let Some(mapping) = mappings.get(addr) {
                    let frame_allocator = FRAME_ALLOCATOR.lock();
                    for frame in &mapping.physical_frames {
                        frame_allocator.free_frames(*frame, 1).ok();
                    }
                }
            }

            for addr in to_remove {
                mappings.remove(&addr);
            }

            // Flush TLB for user-space changes
            crate::arch::tlb_flush_all();
        }

        // Reset user-space metadata
        self.heap_break
            .store(self.heap_start.load(Ordering::Relaxed), Ordering::Release);
        self.next_mmap_addr
            .store(0x4000_0000_0000, Ordering::Release);

        Ok(())
    }

    /// Get user stack base address
    pub fn user_stack_base(&self) -> usize {
        // User stack starts below stack_top and grows downward
        // Reserve 8MB for stack
        const DEFAULT_STACK_SIZE: u64 = 8 * 1024 * 1024;
        (self.stack_top.load(Ordering::Acquire) - DEFAULT_STACK_SIZE) as usize
    }

    /// Get user stack size
    pub fn user_stack_size(&self) -> usize {
        // Default stack size is 8MB
        8 * 1024 * 1024
    }

    /// Get stack top address
    pub fn stack_top(&self) -> usize {
        self.stack_top.load(Ordering::Acquire) as usize
    }

    /// Set stack top address
    pub fn set_stack_top(&self, addr: usize) {
        self.stack_top.store(addr as u64, Ordering::Release);
    }

    /// Map a single page at a virtual address
    pub fn map_page(&mut self, vaddr: usize, flags: PageFlags) -> Result<(), KernelError> {
        use super::PAGE_SIZE;

        // Allocate a physical frame (drop lock before page table operations)
        let frame = {
            FRAME_ALLOCATOR
                .lock()
                .allocate_frames(1, None)
                .map_err(|_| KernelError::OutOfMemory {
                    requested: 4096,
                    available: 0,
                })?
        };

        let vaddr_obj = VirtualAddress(vaddr as u64);

        // Install the mapping in the architecture page table
        let pt_root = self.page_table_root.load(Ordering::Acquire);
        if pt_root != 0 {
            // SAFETY: `pt_root` is a non-zero physical address of an L4 page
            // table set during VAS::init(). The address is identity-mapped in
            // the kernel's physical memory window. We have `&mut self`,
            // ensuring exclusive access to this VAS and its page tables.
            let mut mapper = unsafe { create_mapper_from_root(pt_root) };
            let mut alloc = VasFrameAllocator;
            mapper.map_page(vaddr_obj, frame, flags, &mut alloc)?;
            crate::arch::tlb_flush_address(vaddr as u64);
        }

        // Record the mapping
        #[cfg(feature = "alloc")]
        {
            let mut mappings = self.mappings.lock();

            if let Some(mapping) = mappings.get_mut(&vaddr_obj) {
                mapping.physical_frames.push(frame);
            } else {
                let mut new_mapping = VirtualMapping::new(vaddr_obj, PAGE_SIZE, MappingType::Data);
                new_mapping.physical_frames.push(frame);
                new_mapping.flags = flags;
                mappings.insert(vaddr_obj, new_mapping);
            }
        }

        Ok(())
    }
}

/// Virtual address space statistics
#[derive(Debug, Default)]
pub struct VasStats {
    pub total_size: usize,
    pub code_size: usize,
    pub data_size: usize,
    pub stack_size: usize,
    pub heap_size: usize,
    pub mapping_count: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- MappingType tests ---

    #[test]
    fn test_mapping_type_equality() {
        assert_eq!(MappingType::Code, MappingType::Code);
        assert_ne!(MappingType::Code, MappingType::Data);
        assert_ne!(MappingType::Stack, MappingType::Heap);
    }

    // --- VirtualMapping tests ---

    #[test]
    fn test_virtual_mapping_new_code() {
        let start = VirtualAddress(0x1000);
        let mapping = VirtualMapping::new(start, 0x4000, MappingType::Code);

        assert_eq!(mapping.start, start);
        assert_eq!(mapping.size, 0x4000);
        assert_eq!(mapping.mapping_type, MappingType::Code);
        // Code should be PRESENT and USER, but not WRITABLE
        assert!(mapping.flags.contains(PageFlags::PRESENT));
        assert!(mapping.flags.contains(PageFlags::USER));
        assert!(!mapping.flags.contains(PageFlags::WRITABLE));
    }

    #[test]
    fn test_virtual_mapping_new_data() {
        let mapping = VirtualMapping::new(VirtualAddress(0x2000), 0x1000, MappingType::Data);

        assert!(mapping.flags.contains(PageFlags::PRESENT));
        assert!(mapping.flags.contains(PageFlags::WRITABLE));
        assert!(mapping.flags.contains(PageFlags::USER));
    }

    #[test]
    fn test_virtual_mapping_new_stack() {
        let mapping = VirtualMapping::new(VirtualAddress(0x3000), 0x2000, MappingType::Stack);

        assert!(mapping.flags.contains(PageFlags::PRESENT));
        assert!(mapping.flags.contains(PageFlags::WRITABLE));
        assert!(mapping.flags.contains(PageFlags::USER));
        assert!(mapping.flags.contains(PageFlags::NO_EXECUTE));
    }

    #[test]
    fn test_virtual_mapping_new_heap() {
        let mapping = VirtualMapping::new(VirtualAddress(0x4000), 0x10000, MappingType::Heap);

        assert!(mapping.flags.contains(PageFlags::PRESENT));
        assert!(mapping.flags.contains(PageFlags::WRITABLE));
        assert!(mapping.flags.contains(PageFlags::USER));
        assert!(mapping.flags.contains(PageFlags::NO_EXECUTE));
    }

    #[test]
    fn test_virtual_mapping_new_device() {
        let mapping = VirtualMapping::new(VirtualAddress(0xF000), 0x1000, MappingType::Device);

        assert!(mapping.flags.contains(PageFlags::PRESENT));
        assert!(mapping.flags.contains(PageFlags::WRITABLE));
        assert!(mapping.flags.contains(PageFlags::NO_CACHE));
        // Device memory should NOT have USER flag
        assert!(!mapping.flags.contains(PageFlags::USER));
    }

    #[test]
    fn test_virtual_mapping_contains() {
        let mapping = VirtualMapping::new(VirtualAddress(0x1000), 0x3000, MappingType::Data);

        // Start address - contained
        assert!(mapping.contains(VirtualAddress(0x1000)));
        // Middle address - contained
        assert!(mapping.contains(VirtualAddress(0x2000)));
        // Last byte before end - contained
        assert!(mapping.contains(VirtualAddress(0x3FFF)));
        // End address - NOT contained (exclusive)
        assert!(!mapping.contains(VirtualAddress(0x4000)));
        // Before start - NOT contained
        assert!(!mapping.contains(VirtualAddress(0x0FFF)));
        // Well past end - NOT contained
        assert!(!mapping.contains(VirtualAddress(0x5000)));
    }

    #[test]
    fn test_virtual_mapping_end() {
        let mapping = VirtualMapping::new(VirtualAddress(0x1000), 0x3000, MappingType::Data);
        assert_eq!(mapping.end(), VirtualAddress(0x4000));
    }

    #[test]
    fn test_virtual_mapping_zero_size() {
        let mapping = VirtualMapping::new(VirtualAddress(0x1000), 0, MappingType::File);
        assert_eq!(mapping.end(), VirtualAddress(0x1000));
        // A zero-sized mapping should not contain its start address
        assert!(!mapping.contains(VirtualAddress(0x1000)));
    }

    // --- VirtualAddressSpace tests ---

    #[test]
    fn test_vas_default_values() {
        let vas = VirtualAddressSpace::new();

        // Check default page table root
        assert_eq!(vas.get_page_table(), 0);

        // Check default heap settings
        let heap_break = vas.brk(None);
        assert_eq!(heap_break, VirtualAddress(0x2000_0000_0000));

        // Check default stack settings
        assert_eq!(vas.stack_top(), 0x7FFF_FFFF_0000);
    }

    #[test]
    fn test_vas_set_page_table() {
        let vas = VirtualAddressSpace::new();
        vas.set_page_table(0xDEAD_BEEF_0000);
        assert_eq!(vas.get_page_table(), 0xDEAD_BEEF_0000);
    }

    #[test]
    fn test_vas_brk_extend_heap() {
        let vas = VirtualAddressSpace::new();

        // Initial break
        let initial = vas.brk(None);
        assert_eq!(initial, VirtualAddress(0x2000_0000_0000));

        // Extend the heap
        let new_addr = VirtualAddress(0x2000_0001_0000);
        let result = vas.brk(Some(new_addr));
        assert_eq!(result, new_addr);

        // Verify it persisted
        let current = vas.brk(None);
        assert_eq!(current, new_addr);
    }

    #[test]
    fn test_vas_brk_refuses_shrink() {
        let vas = VirtualAddressSpace::new();

        // Extend the heap first
        let extended = VirtualAddress(0x2000_0001_0000);
        vas.brk(Some(extended));

        // Try to shrink (should be ignored -- brk only grows)
        let shrink_addr = VirtualAddress(0x2000_0000_0000);
        let result = vas.brk(Some(shrink_addr));
        // The break should remain at the extended address
        assert_eq!(result, extended);
    }

    #[test]
    fn test_vas_brk_refuses_below_heap_start() {
        let vas = VirtualAddressSpace::new();

        // Try to set break below heap start
        let below_start = VirtualAddress(0x1000_0000_0000);
        let result = vas.brk(Some(below_start));
        // Should remain at initial break
        assert_eq!(result, VirtualAddress(0x2000_0000_0000));
    }

    #[test]
    fn test_vas_stack_top_get_set() {
        let vas = VirtualAddressSpace::new();

        let default_top = vas.stack_top();
        assert_eq!(default_top, 0x7FFF_FFFF_0000);

        vas.set_stack_top(0x7000_0000_0000);
        assert_eq!(vas.stack_top(), 0x7000_0000_0000);
    }

    #[test]
    fn test_vas_user_stack_base_and_size() {
        let vas = VirtualAddressSpace::new();

        let stack_size = vas.user_stack_size();
        assert_eq!(stack_size, 8 * 1024 * 1024); // 8MB

        let stack_base = vas.user_stack_base();
        let expected_base = 0x7FFF_FFFF_0000 - 8 * 1024 * 1024;
        assert_eq!(stack_base, expected_base);
    }

    #[test]
    fn test_vas_clone_from() {
        let source = VirtualAddressSpace::new();
        source.set_page_table(0xCAFE_0000);
        source.brk(Some(VirtualAddress(0x2000_0002_0000)));
        source.set_stack_top(0x6000_0000_0000);

        let mut target = VirtualAddressSpace::new();
        target
            .clone_from(&source)
            .expect("clone_from should succeed");

        assert_eq!(target.get_page_table(), 0xCAFE_0000);
        assert_eq!(target.brk(None), VirtualAddress(0x2000_0002_0000));
        assert_eq!(target.stack_top(), 0x6000_0000_0000);
    }

    #[test]
    fn test_vas_mmap_advances_address() {
        let vas = VirtualAddressSpace::new();

        // First mmap should return the initial mmap address
        let addr1 = vas.mmap(0x1000, MappingType::Data);
        assert!(addr1.is_ok());
        let addr1 = addr1.unwrap();
        assert_eq!(addr1, VirtualAddress(0x4000_0000_0000));

        // Second mmap should advance past the first (page-aligned)
        let addr2 = vas.mmap(0x2000, MappingType::Data);
        assert!(addr2.is_ok());
        let addr2 = addr2.unwrap();
        assert_eq!(addr2, VirtualAddress(0x4000_0000_1000));
    }

    #[test]
    fn test_vas_mmap_page_alignment() {
        let vas = VirtualAddressSpace::new();

        // Request a non-page-aligned size
        let addr = vas.mmap(100, MappingType::Code);
        assert!(addr.is_ok());

        // Next mmap should be at page-aligned offset
        let addr2 = vas.mmap(100, MappingType::Code);
        assert!(addr2.is_ok());
        let diff = addr2.unwrap().as_u64() - addr.unwrap().as_u64();
        assert_eq!(diff, 4096, "mmap allocations should be page-aligned");
    }

    // --- VasStats tests ---

    #[test]
    fn test_vas_stats_default() {
        let stats = VasStats::default();
        assert_eq!(stats.total_size, 0);
        assert_eq!(stats.code_size, 0);
        assert_eq!(stats.data_size, 0);
        assert_eq!(stats.stack_size, 0);
        assert_eq!(stats.heap_size, 0);
        assert_eq!(stats.mapping_count, 0);
    }
}
