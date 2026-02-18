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
    let virt = super::phys_to_virt_addr(page_table_root);
    let l4_ptr = virt as *mut super::page_table::PageTable;
    // SAFETY: The caller guarantees that `page_table_root` is a valid
    // physical address of an L4 page table. phys_to_virt_addr converts
    // it to the corresponding virtual address in the bootloader's
    // physical memory mapping.
    unsafe { PageMapper::new(l4_ptr) }
}

/// Public wrapper around `create_mapper_from_root` for use by other kernel
/// modules (e.g., process creation for writing to user stack).
///
/// # Safety
///
/// Same requirements as [`create_mapper_from_root`].
pub unsafe fn create_mapper_from_root_pub(page_table_root: u64) -> PageMapper {
    unsafe { create_mapper_from_root(page_table_root) }
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

    /// Map kernel space into this address space.
    ///
    /// Copies the upper-half L4 entries (indices 256-511) from the current
    /// (boot) page tables into this VAS's L4. This shares the kernel's
    /// physical memory mapping, code, data, heap, and MMIO regions with
    /// the new process, so that the kernel remains accessible after a CR3
    /// switch.
    pub fn map_kernel_space(&mut self) -> Result<(), KernelError> {
        use super::page_table::{PageTable, PAGE_TABLE_ENTRIES};

        let new_root = self.page_table_root.load(Ordering::Acquire);
        if new_root == 0 {
            return Err(KernelError::NotInitialized {
                subsystem: "VAS page table",
            });
        }

        // Read the current (boot) CR3 to get the kernel's L4 entries
        let boot_cr3: u64;
        #[cfg(target_arch = "x86_64")]
        {
            // SAFETY: Reading CR3 is a read-only privileged operation.
            unsafe {
                core::arch::asm!("mov {}, cr3", out(reg) boot_cr3);
            }
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            boot_cr3 = 0;
        }

        let boot_l4_phys = boot_cr3 & 0x000F_FFFF_FFFF_F000;
        if boot_l4_phys == 0 {
            // On non-x86_64 or if CR3 is somehow 0, just record regions
            #[cfg(feature = "alloc")]
            {
                self.map_region(
                    VirtualAddress(0xFFFF_8000_0000_0000),
                    0x200000,
                    MappingType::Code,
                )?;
                self.map_region(
                    VirtualAddress(0xFFFF_8000_0020_0000),
                    0x200000,
                    MappingType::Data,
                )?;
                self.map_region(
                    VirtualAddress(0xFFFF_C000_0000_0000),
                    0x1000000,
                    MappingType::Heap,
                )?;
            }
            return Ok(());
        }

        // Copy kernel-space L4 entries (indices 256-511) from boot page
        // tables into the new process's L4. This shares the entire
        // kernel upper-half mapping.
        // SAFETY: Both boot_l4_phys and new_root are valid L4 page table
        // physical addresses. We convert via phys_to_virt_addr to get
        // kernel-accessible pointers. We copy only the upper half (kernel
        // space), leaving the lower half (user space) zeroed.
        unsafe {
            let boot_l4 = &*(super::phys_to_virt_addr(boot_l4_phys) as *const PageTable);
            let new_l4 = &mut *(super::phys_to_virt_addr(new_root) as *mut PageTable);

            let mut copied_count = 0;
            for i in 256..PAGE_TABLE_ENTRIES {
                if boot_l4[i].is_present() {
                    new_l4[i] = boot_l4[i];
                    copied_count += 1;
                }
            }

            // Diagnostic: Confirm kernel mapping is complete
            #[cfg(target_arch = "x86_64")]
            if copied_count > 0 {
                crate::println!(
                    "[VAS] Copied {} kernel L4 entries from boot tables to new process page tables",
                    copied_count
                );
            }
        }

        Ok(())
    }

    /// Clone from another address space (deep copy for fork).
    ///
    /// Allocates a new L4 page table for this VAS, copies kernel-space L4
    /// entries directly (shared kernel mapping), and for each user-space page
    /// in the parent, allocates a new physical frame, copies the 4KB content,
    /// and maps it into this VAS's page tables with the same flags.
    #[cfg(feature = "alloc")]
    pub fn clone_from(&mut self, other: &Self) -> Result<(), KernelError> {
        use super::page_table::{PageTable, PageTableHierarchy, PAGE_TABLE_ENTRIES};

        // Step 1: Allocate a new L4 page table for the child
        let new_hierarchy = PageTableHierarchy::new()?;
        let new_root = new_hierarchy.l4_addr().as_u64();
        self.page_table_root.store(new_root, Ordering::Release);

        let parent_root = other.page_table_root.load(Ordering::Acquire);

        if parent_root != 0 {
            // Step 2: Copy kernel-space L4 entries (indices 256-511) directly.
            // These are shared across all address spaces.
            let parent_l4 =
                unsafe { &*(super::phys_to_virt_addr(parent_root) as *const PageTable) };
            let child_l4 = unsafe { &mut *(super::phys_to_virt_addr(new_root) as *mut PageTable) };

            for i in 256..PAGE_TABLE_ENTRIES {
                child_l4[i] = parent_l4[i];
            }

            // Step 3: Deep-copy user-space pages.
            // Walk parent's mappings (which track user-space regions) and for
            // each mapped page, allocate a new frame, copy content, and map.
            let parent_mappings = other.mappings.lock();
            let mut child_mappings = self.mappings.lock();
            child_mappings.clear();

            // SAFETY: parent_root is a valid identity-mapped L4 page table.
            let parent_mapper = unsafe { create_mapper_from_root(parent_root) };
            // SAFETY: new_root was just allocated and kernel entries copied.
            let mut child_mapper = unsafe { create_mapper_from_root(new_root) };
            let mut alloc = VasFrameAllocator;

            const KERNEL_SPACE_START: u64 = 0xFFFF_8000_0000_0000;

            for (addr, mapping) in parent_mappings.iter() {
                // Only deep-copy user-space mappings
                if addr.0 >= KERNEL_SPACE_START {
                    // Kernel mappings are already shared via L4 entries above
                    child_mappings.insert(*addr, mapping.clone());
                    continue;
                }

                let num_pages = mapping.size / 4096;
                let mut child_frames = Vec::with_capacity(num_pages);

                for i in 0..num_pages {
                    let vaddr = VirtualAddress(mapping.start.0 + (i as u64) * 4096);

                    // Look up the parent's physical frame and flags
                    let (parent_frame, flags) = match parent_mapper.translate_page(vaddr) {
                        Ok(result) => result,
                        Err(_) => continue, // Page not actually mapped in HW
                    };

                    // Allocate a new frame for the child
                    let child_frame = {
                        FRAME_ALLOCATOR
                            .lock()
                            .allocate_frames(1, None)
                            .map_err(|_| KernelError::OutOfMemory {
                                requested: 4096,
                                available: 0,
                            })?
                    };

                    // Copy 4KB of content from parent frame to child frame.
                    // SAFETY: Both frame addresses are valid physical addresses
                    // in identity-mapped memory. Each points to a full 4KB page.
                    unsafe {
                        let src = (parent_frame.as_u64() << 12) as *const u8;
                        let dst = (child_frame.as_u64() << 12) as *mut u8;
                        core::ptr::copy_nonoverlapping(src, dst, 4096);
                    }

                    // Map the child's frame at the same virtual address
                    child_mapper
                        .map_page(vaddr, child_frame, flags, &mut alloc)
                        .ok(); // Ignore errors for already-mapped pages

                    child_frames.push(child_frame);
                }

                // Record the mapping with the child's physical frames
                let mut child_mapping = mapping.clone();
                child_mapping.physical_frames = child_frames;
                child_mappings.insert(*addr, child_mapping);
            }
        }

        // Copy metadata
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

    /// Clone from another address space (no-alloc stub).
    #[cfg(not(feature = "alloc"))]
    pub fn clone_from(&mut self, _other: &Self) -> Result<(), KernelError> {
        Err(KernelError::NotImplemented {
            feature: "clone_from (requires alloc)",
        })
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

    /// Extend or query heap (brk).
    ///
    /// When `new_break` is `Some`, attempts to move the program break:
    /// - **Grow** (new > current): allocates physical frames and maps pages for
    ///   the delta region.
    /// - **Shrink** (new < current but >= heap_start): unmaps pages and frees
    ///   frames for the delta region.
    /// - **Below heap_start** or **equal to current**: no-op.
    ///
    /// When `new_break` is `None`, returns the current break without changes.
    pub fn brk(&self, new_break: Option<VirtualAddress>) -> VirtualAddress {
        if let Some(addr) = new_break {
            let current = self.heap_break.load(Ordering::Acquire);
            let heap_start = self.heap_start.load(Ordering::Relaxed);

            if addr.0 < heap_start {
                // Below heap start: ignore
            } else if addr.0 > current {
                // Grow: allocate pages for [current, addr) range
                let old_page = (current + 4095) / 4096; // First page NOT yet allocated
                let new_page = (addr.0 + 4095) / 4096;

                if new_page > old_page {
                    #[cfg(feature = "alloc")]
                    {
                        // Map new heap pages
                        let start = VirtualAddress(old_page * 4096);
                        let size = ((new_page - old_page) * 4096) as usize;
                        if self.map_region(start, size, MappingType::Heap).is_ok() {
                            self.heap_break.store(addr.0, Ordering::Release);
                        }
                        // On failure, leave break unchanged
                    }
                    #[cfg(not(feature = "alloc"))]
                    {
                        // Without alloc, just move the pointer (legacy behavior)
                        self.heap_break.store(addr.0, Ordering::Release);
                    }
                } else {
                    // Within the same page, just update the pointer
                    self.heap_break.store(addr.0, Ordering::Release);
                }
            } else if addr.0 < current && addr.0 >= heap_start {
                // Shrink: unmap pages for [addr, current) range
                let new_page = (addr.0 + 4095) / 4096;
                let old_page = (current + 4095) / 4096;

                if old_page > new_page {
                    #[cfg(feature = "alloc")]
                    {
                        let start = VirtualAddress(new_page * 4096);
                        let _ = self.unmap_region(start);
                    }
                }
                self.heap_break.store(addr.0, Ordering::Release);
            }
        }

        VirtualAddress(self.heap_break.load(Ordering::Acquire))
    }

    /// Clone address space (for fork).
    ///
    /// Creates a new VAS with its own L4 page table and deep-copies all
    /// user-space pages from this VAS. Kernel-space entries are shared.
    #[cfg(feature = "alloc")]
    pub fn fork(&self) -> Result<Self, KernelError> {
        let mut new_vas = Self::new();
        new_vas.clone_from(self)?;
        Ok(new_vas)
    }

    /// Update hardware page table entry flags for a region.
    ///
    /// Walks the page table for each page in `[start, start+size)` and updates
    /// the PTE flags according to the POSIX `prot` bitmask. Flushes TLB for
    /// each modified page.
    #[cfg(feature = "alloc")]
    pub fn protect_region(
        &self,
        start: VirtualAddress,
        size: usize,
        prot: usize,
    ) -> Result<(), KernelError> {
        use super::PageFlags;

        let pt_root = self.page_table_root.load(Ordering::Acquire);
        if pt_root == 0 {
            return Ok(()); // No page tables, nothing to update
        }

        // Convert POSIX prot flags to hardware PageFlags
        let mut new_flags = PageFlags::PRESENT | PageFlags::USER;
        if prot & 0x2 != 0 {
            // PROT_WRITE
            new_flags |= PageFlags::WRITABLE;
        }
        if prot & 0x4 == 0 {
            // !PROT_EXEC -> NO_EXECUTE
            new_flags |= PageFlags::NO_EXECUTE;
        }

        // SAFETY: pt_root is a valid identity-mapped L4 page table. We hold the
        // mappings lock implicitly via the caller's &self borrow.
        let mut mapper = unsafe { create_mapper_from_root(pt_root) };

        let num_pages = (size + 4095) / 4096;
        for i in 0..num_pages {
            let vaddr = VirtualAddress(start.0 + (i as u64) * 4096);
            // Ignore errors for pages that aren't mapped in the hardware tables
            let _ = mapper.update_page_flags(vaddr, new_flags);
            crate::arch::tlb_flush_address(vaddr.0);
        }

        // Update the mapping metadata flags too
        let mut mappings = self.mappings.lock();
        if let Some(mapping) = mappings.get_mut(&start) {
            mapping.flags = new_flags;
        }

        Ok(())
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

    // Note: test_vas_clone_from removed -- clone_from() now allocates
    // real page tables via FRAME_ALLOCATOR, which is unavailable in the
    // host test environment. Verified via QEMU boot tests instead.

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
