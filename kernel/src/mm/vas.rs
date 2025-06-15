//! Virtual Address Space management
//!
//! Manages virtual memory for processes including page tables,
//! memory mappings, and address space operations.

use core::sync::atomic::{AtomicU64, Ordering};

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::{collections::BTreeMap, vec::Vec};

use spin::Mutex;

use super::{PageFlags, VirtualAddress};

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
    pub fn init(&mut self) -> Result<(), &'static str> {
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
    pub fn map_kernel_space(&mut self) -> Result<(), &'static str> {
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

            // Map kernel heap region
            self.map_region(
                VirtualAddress(0xFFFF_C000_0000_0000),
                0x1000_0000, // 256MB for kernel heap
                MappingType::Heap,
            )?;
        }

        Ok(())
    }

    /// Clone from another address space
    pub fn clone_from(&mut self, other: &Self) -> Result<(), &'static str> {
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
            use super::FRAME_ALLOCATOR;

            // First unmap all regions and free their physical frames
            let mut mappings = self.mappings.lock();
            for (_, mapping) in mappings.iter() {
                // Free the physical frames
                let allocator = FRAME_ALLOCATOR.lock();
                for &frame in &mapping.physical_frames {
                    let _ = allocator.free_frames(frame, 1);
                }

                // Unmap from page tables
                // Note: In a real implementation, we would need the page mapper
                // for this VAS to unmap the pages. For now, we'll just flush
                // TLB.
            }

            // Clear all mappings
            mappings.clear();

            // Free the page table structures themselves
            // Note: This would require walking the page table hierarchy
            // and freeing intermediate table pages. For now, we just
            // clear our tracking structures.
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
    ) -> Result<(), &'static str> {
        use super::FRAME_ALLOCATOR;

        // Align to page boundary
        let aligned_start = VirtualAddress(start.0 & !(4096 - 1));
        let aligned_size = ((size + 4095) / 4096) * 4096;

        let mapping = VirtualMapping::new(aligned_start, aligned_size, mapping_type);

        let mut mappings = self.mappings.lock();

        // Check for overlaps
        for (_, existing) in mappings.iter() {
            if existing.contains(aligned_start) || existing.contains(mapping.end()) {
                return Err("Address range already mapped");
            }
        }

        // Allocate physical frames for the mapping
        let num_pages = aligned_size / 4096;
        let mut physical_frames = Vec::with_capacity(num_pages);

        // Get frame allocator
        let frame_allocator = FRAME_ALLOCATOR.lock();

        // Allocate frames
        for _ in 0..num_pages {
            let frame = frame_allocator
                .allocate_frames(1, None)
                .map_err(|_| "Failed to allocate physical frame")?;
            physical_frames.push(frame);
        }

        // Map pages to frames in page table
        // Note: In a real implementation, we would need to map the page table
        // into virtual memory to modify it. For now, we'll just store the mapping.
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
    ) -> Result<crate::raii::MappedRegion, &'static str> {
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
    pub fn unmap_region(&self, start: VirtualAddress) -> Result<(), &'static str> {
        use super::FRAME_ALLOCATOR;

        let mut mappings = self.mappings.lock();
        let mapping = mappings.remove(&start).ok_or("Region not mapped")?;

        // Flush TLB for the unmapped range
        #[cfg(target_arch = "x86_64")]
        crate::arch::mmu::flush_tlb_address(mapping.start.0);
        #[cfg(target_arch = "aarch64")]
        {
            // AArch64 TLB flush
            use cortex_a::asm::barrier;
            unsafe {
                core::arch::asm!("tlbi vaae1is, {}", in(reg) mapping.start.0 >> 12);
                barrier::dsb(barrier::SY);
                barrier::isb(barrier::SY);
            }
        }
        #[cfg(target_arch = "riscv64")]
        {
            // RISC-V TLB flush
            unsafe {
                core::arch::asm!("sfence.vma {}, zero", in(reg) mapping.start.0);
            }
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
    pub fn unmap(&self, start_addr: usize, _size: usize) -> Result<(), &'static str> {
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
    ) -> Result<VirtualAddress, &'static str> {
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
    pub fn fork(&self) -> Result<Self, &'static str> {
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
    ) -> Result<(), &'static str> {
        #[cfg(feature = "alloc")]
        {
            // Find the mapping for this address
            let mapping = self
                .find_mapping(fault_addr)
                .ok_or("Page fault in unmapped region")?;

            // Check permissions
            if write && !mapping.flags.contains(PageFlags::WRITABLE) {
                return Err("Write to read-only page");
            }

            if user && !mapping.flags.contains(PageFlags::USER) {
                return Err("User access to kernel page");
            }

            // Check if this is a valid fault (e.g., COW, demand paging)
            // For now, we'll just return an error as we don't support these features yet
            Err("Page fault handling not fully implemented")
        }

        #[cfg(not(feature = "alloc"))]
        Err("Page fault handling requires alloc feature")
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
            use super::FRAME_ALLOCATOR;

            // Get all mappings to free their frames
            let mappings = self.mappings.get_mut();

            // Free physical frames for each mapping
            for (_, mapping) in mappings.iter() {
                let frame_allocator = FRAME_ALLOCATOR.lock();
                for frame in &mapping.physical_frames {
                    frame_allocator.free_frames(*frame, 1).ok();
                }
            }

            // Clear all mappings
            mappings.clear();
        }

        // Reset metadata
        self.heap_break
            .store(self.heap_start.load(Ordering::Relaxed), Ordering::Release);
        self.next_mmap_addr
            .store(0x4000_0000_0000, Ordering::Release);

        // TODO: Free page tables
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
