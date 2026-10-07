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
    page_table::{FrameAllocator as PageFrameAllocator, PageMapper, PageTable, PAGE_TABLE_ENTRIES},
    FrameAllocatorError, FrameNumber, PageFlags, VirtualAddress, FRAME_ALLOCATOR, FRAME_SIZE,
};
use crate::error::KernelError;

/// The flags to install on a present user page when changing its protection
/// to `requested`, keeping copy-on-write intact: a frame shared by a fork is
/// never mapped writable. A writable request on a shared (or still-COW) page
/// becomes read-only + COW, so the first write copies; a read-only request
/// drops COW (the frame stays shared and read-only, so a later writable
/// request sees it shared again). `None` when the page is not mapped.
fn cow_safe_flags(
    mapper: &PageMapper,
    page: VirtualAddress,
    requested: PageFlags,
) -> Option<PageFlags> {
    let (frame, old) = mapper.translate_page(page).ok()?;
    Some(cow_flags_for(
        old,
        requested,
        super::frame_refs::is_shared(frame),
    ))
}

/// Pure rule behind [`cow_safe_flags`].
fn cow_flags_for(old: PageFlags, requested: PageFlags, shared: bool) -> PageFlags {
    let requested = requested.without(PageFlags::COW);
    if requested.contains(PageFlags::WRITABLE) && (shared || old.contains(PageFlags::COW)) {
        requested.without(PageFlags::WRITABLE) | PageFlags::COW
    } else {
        requested
    }
}

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
    // SAFETY: Caller guarantees page_table_root is a valid, identity-mapped L4 page
    // table address.
    unsafe { create_mapper_from_root(page_table_root) }
}

/// Size of a huge page.
const HUGE_PAGE_SIZE: u64 = 2 * 1024 * 1024;

/// Remove the translations of `mapping`: one L2 leaf per 2 MiB page for a
/// huge mapping (`PageFlags::HUGE`), each 4 KiB page otherwise. Pages that
/// were never installed are skipped, as the callers always did.
fn unmap_mapping_pages(mapper: &mut PageMapper, mapping: &VirtualMapping) {
    if mapping.flags.contains(PageFlags::HUGE) {
        let mut addr = mapping.start.0;
        while addr < mapping.start.0 + mapping.size as u64 {
            let _ = mapper.unmap_huge_2m(VirtualAddress(addr));
            addr += HUGE_PAGE_SIZE;
        }
    } else {
        for i in 0..mapping.size / 4096 {
            let _ = mapper.unmap_page(VirtualAddress(mapping.start.0 + (i as u64) * 4096));
        }
    }
}

/// Whether `virt` is mapped in the hierarchy rooted at `root`, counting a
/// 1 GiB or 2 MiB leaf as mapped. (`PageMapper` does not understand huge
/// pages, which the bootloader's direct map uses.)
#[cfg(target_arch = "x86_64")]
fn is_mapped_any_size(root: u64, virt: u64) -> bool {
    const PRESENT: u64 = 1;
    const HUGE: u64 = 1 << 7;
    const ADDR: u64 = 0x000F_FFFF_FFFF_F000;
    let mut table = root & ADDR;
    for (level, shift) in [(4, 39), (3, 30), (2, 21), (1, 12)] {
        let index = ((virt >> shift) & 0x1FF) as usize;
        // SAFETY: `table` is the physical address of a present page table
        // (the root, or taken from a present non-leaf entry), reachable
        // through the direct map; index < 512.
        let entry = unsafe {
            core::ptr::read_volatile((super::phys_to_virt_addr(table) as *const u64).add(index))
        };
        if entry & PRESENT == 0 {
            return false;
        }
        if level == 1 || ((level == 3 || level == 2) && entry & HUGE != 0) {
            return true;
        }
        table = entry & ADDR;
    }
    true
}

/// Make the physical range `[phys, phys + size)` -- device registers --
/// reachable through the kernel direct map, uncached, and return the virtual
/// address of `phys`.
///
/// The bootloader's direct map covers RAM and low MMIO but not 64-bit BARs
/// placed far above RAM. Pages that are already mapped are left alone.
/// Must run while the kernel page table is active and before other CPUs
/// edit it (boot-time driver probing).
#[cfg(target_arch = "x86_64")]
pub fn map_mmio(phys: u64, size: usize) -> Result<usize, KernelError> {
    let start = phys & !0xFFF;
    let end = phys
        .checked_add(size as u64)
        .and_then(|e| e.checked_add(0xFFF))
        .ok_or(KernelError::InvalidArgument {
            name: "mmio range",
            value: "overflows",
        })?
        & !0xFFF;
    let root = (super::get_kernel_page_table() as u64) & !0xFFF;
    // SAFETY: `root` is the active page table (CR3), reachable through the
    // direct map; boot-time callers have exclusive use of it.
    let mut mapper = unsafe { create_mapper_from_root(root) };
    let mut alloc = VasFrameAllocator;
    let flags =
        PageFlags::PRESENT | PageFlags::WRITABLE | PageFlags::NO_CACHE | PageFlags::WRITE_THROUGH;
    let mut pa = start;
    while pa < end {
        let va = super::phys_to_virt_addr(pa);
        if !is_mapped_any_size(root, va) {
            mapper.map_page(
                VirtualAddress(va),
                FrameNumber::new(pa >> 12),
                flags,
                &mut alloc,
            )?;
            crate::arch::x86_64::tlb_flush_address(va);
        }
        pa += 4096;
    }
    Ok(super::phys_to_virt_addr(phys) as usize)
}

/// Free all user-space page table frames in a page table hierarchy.
///
/// Walks the L4 table and for each **user-space** L4 entry (indices 0..256),
/// recursively frees L3, L2, and L1 table frames. Kernel-space entries
/// (indices 256..512) are left untouched because they are shared across all
/// address spaces (copied from the boot page tables).
///
/// Finally, frees the L4 frame itself.
///
/// Returns the number of frames freed.
pub fn free_user_page_table_frames(l4_phys: u64) -> usize {
    if l4_phys == 0 {
        return 0;
    }

    // No CPU may still cache a translation through these tables (or the
    // tables themselves, in its paging-structure caches) once their frames
    // are reused (MEM-SEC-02).
    crate::mm::tlb::flush_all();

    let phys_offset_val = super::PHYS_MEM_OFFSET.load(core::sync::atomic::Ordering::Acquire);
    let mut freed = 0usize;

    // SAFETY: l4_phys is a valid physical address of an L4 page table.
    // phys_to_virt_addr maps it to the kernel's identity-mapped region.
    let l4_table = unsafe { &*(super::phys_to_virt_addr(l4_phys) as *const PageTable) };

    // Only walk user-space entries (0..256). Kernel entries (256..512) are
    // shared references to the boot page tables and must NOT be freed.
    for l4_idx in 0..256 {
        let l4_entry = &l4_table[l4_idx];
        if !l4_entry.is_present() {
            continue;
        }

        // Also skip the physical memory mapping L4 entry (bootloader puts
        // the identity map in a lower-half L4 slot).
        if phys_offset_val != 0 {
            let phys_l4_idx = ((phys_offset_val >> 39) & 0x1FF) as usize;
            if l4_idx == phys_l4_idx {
                continue;
            }
        }

        let l3_phys = match l4_entry.addr() {
            Some(a) => a.as_u64(),
            None => continue,
        };

        // Walk L3 table
        // SAFETY: l3_phys is from a present L4 entry; phys_to_virt_addr maps it into
        // the kernel's identity-mapped region.
        let l3_table = unsafe { &*(super::phys_to_virt_addr(l3_phys) as *const PageTable) };
        for l3_idx in 0..PAGE_TABLE_ENTRIES {
            let l3_entry = &l3_table[l3_idx];
            if !l3_entry.is_present() {
                continue;
            }
            // Skip huge pages (1GB) -- they have no L2 subtable
            if l3_entry.flags().0 & PageFlags::HUGE.0 != 0 {
                continue;
            }

            let l2_phys = match l3_entry.addr() {
                Some(a) => a.as_u64(),
                None => continue,
            };

            // Walk L2 table
            // SAFETY: l2_phys is from a present L3 entry; phys_to_virt_addr maps it into
            // the kernel's identity-mapped region.
            let l2_table = unsafe { &*(super::phys_to_virt_addr(l2_phys) as *const PageTable) };
            for l2_idx in 0..PAGE_TABLE_ENTRIES {
                let l2_entry = &l2_table[l2_idx];
                if !l2_entry.is_present() {
                    continue;
                }
                // Skip huge pages (2MB) -- they have no L1 subtable
                if l2_entry.flags().0 & PageFlags::HUGE.0 != 0 {
                    continue;
                }

                let l1_phys = match l2_entry.addr() {
                    Some(a) => a.as_u64(),
                    None => continue,
                };

                // Free the L1 table frame
                let l1_frame = FrameNumber::new(l1_phys / FRAME_SIZE as u64);
                FRAME_ALLOCATOR.lock().free_frames(l1_frame, 1).ok();
                freed += 1;
            }

            // Free the L2 table frame
            let l2_frame = FrameNumber::new(l2_phys / FRAME_SIZE as u64);
            FRAME_ALLOCATOR.lock().free_frames(l2_frame, 1).ok();
            freed += 1;
        }

        // Free the L3 table frame
        let l3_frame = FrameNumber::new(l3_phys / FRAME_SIZE as u64);
        FRAME_ALLOCATOR.lock().free_frames(l3_frame, 1).ok();
        freed += 1;
    }

    // Free the L4 table frame itself
    let l4_frame = FrameNumber::new(l4_phys / FRAME_SIZE as u64);
    FRAME_ALLOCATOR.lock().free_frames(l4_frame, 1).ok();
    freed += 1;

    freed
}

/// Free user-space page table subtrees (L3/L2/L1) but keep the L4 frame.
///
/// Used during exec to reclaim intermediate page table frames from the old
/// executable's mappings while keeping the L4 root for reuse. After this
/// call, user-space L4 entries (0..256) are cleared so fresh intermediate
/// tables will be allocated by subsequent `map_page` calls.
fn free_user_page_table_subtrees(l4_phys: u64) {
    let phys_offset_val = super::PHYS_MEM_OFFSET.load(core::sync::atomic::Ordering::Acquire);

    // SAFETY: l4_phys is a valid physical address of an L4 page table.
    let l4_table = unsafe { &mut *(super::phys_to_virt_addr(l4_phys) as *mut PageTable) };

    for l4_idx in 0..256 {
        let l4_entry = &l4_table[l4_idx];
        if !l4_entry.is_present() {
            continue;
        }

        // Skip the physical memory mapping L4 entry
        if phys_offset_val != 0 {
            let phys_l4_idx = ((phys_offset_val >> 39) & 0x1FF) as usize;
            if l4_idx == phys_l4_idx {
                continue;
            }
        }

        let l3_phys = match l4_entry.addr() {
            Some(a) => a.as_u64(),
            None => continue,
        };

        // Walk and free L3 subtree
        // SAFETY: l3_phys is from a present L4 entry; phys_to_virt_addr maps it into
        // the kernel's identity-mapped region.
        let l3_table = unsafe { &*(super::phys_to_virt_addr(l3_phys) as *const PageTable) };
        for l3_idx in 0..PAGE_TABLE_ENTRIES {
            let l3_entry = &l3_table[l3_idx];
            if !l3_entry.is_present() || l3_entry.flags().0 & PageFlags::HUGE.0 != 0 {
                continue;
            }

            let l2_phys = match l3_entry.addr() {
                Some(a) => a.as_u64(),
                None => continue,
            };

            // SAFETY: l2_phys is from a present L3 entry; phys_to_virt_addr maps it into
            // the kernel's identity-mapped region.
            let l2_table = unsafe { &*(super::phys_to_virt_addr(l2_phys) as *const PageTable) };
            for l2_idx in 0..PAGE_TABLE_ENTRIES {
                let l2_entry = &l2_table[l2_idx];
                if !l2_entry.is_present() || l2_entry.flags().0 & PageFlags::HUGE.0 != 0 {
                    continue;
                }

                let l1_phys = match l2_entry.addr() {
                    Some(a) => a.as_u64(),
                    None => continue,
                };

                // Free L1 frame
                let l1_frame = FrameNumber::new(l1_phys / FRAME_SIZE as u64);
                FRAME_ALLOCATOR.lock().free_frames(l1_frame, 1).ok();
            }

            // Free L2 frame
            let l2_frame = FrameNumber::new(l2_phys / FRAME_SIZE as u64);
            FRAME_ALLOCATOR.lock().free_frames(l2_frame, 1).ok();
        }

        // Free L3 frame
        let l3_frame = FrameNumber::new(l3_phys / FRAME_SIZE as u64);
        FRAME_ALLOCATOR.lock().free_frames(l3_frame, 1).ok();

        // Clear the L4 entry so new mappings get fresh page tables
        l4_table[l4_idx].clear();
    }
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
    /// Device memory (no caching). Frames belong to the device.
    Device,
    /// Frames of an IPC shared region, which owns and frees them.
    SharedRegion,
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
            MappingType::SharedRegion => {
                PageFlags::PRESENT | PageFlags::WRITABLE | PageFlags::USER | PageFlags::NO_EXECUTE
            }
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

    /// Whether this mapping's frames came from the frame allocator for it
    /// and must be freed with it. Device memory and IPC shared-region frames
    /// are only borrowed: freeing them handed MMIO or framebuffer frames, or
    /// a region another process still maps, to the allocator (N-32).
    pub fn owns_frames(&self) -> bool {
        !matches!(
            self.mapping_type,
            MappingType::Device | MappingType::SharedRegion
        )
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
    /// Stack size (bytes)
    stack_size: AtomicU64,

    /// TLB generation counter. Incremented on every page table modification.
    /// The scheduler compares this against the last-seen generation at switch
    /// time to determine whether a TLB flush is needed.
    pub tlb_generation: AtomicU64,
}

/// Batched TLB flush accumulator.
///
/// Collects up to `MAX_BATCH` virtual addresses for individual flushes.
/// If more than `MAX_BATCH` addresses are accumulated, the entire TLB is
/// flushed on commit. This reduces the overhead of multiple individual
/// `invlpg` instructions in loops (e.g., munmap of many pages).
pub struct TlbFlushBatch {
    addresses: [u64; Self::MAX_BATCH],
    count: usize,
}

impl Default for TlbFlushBatch {
    fn default() -> Self {
        Self::new()
    }
}

impl TlbFlushBatch {
    const MAX_BATCH: usize = 16;

    /// Create a new empty batch.
    pub const fn new() -> Self {
        Self {
            addresses: [0; Self::MAX_BATCH],
            count: 0,
        }
    }

    /// Add an address to the batch. Does not flush yet.
    #[inline]
    pub fn add(&mut self, vaddr: u64) {
        if self.count < Self::MAX_BATCH {
            self.addresses[self.count] = vaddr;
        }
        self.count += 1; // Allow overflow past MAX_BATCH to trigger full flush
    }

    /// Flush all accumulated addresses. If > MAX_BATCH, do a full TLB flush.
    pub fn flush(self) {
        if self.count == 0 {
            return;
        }
        if self.count > Self::MAX_BATCH {
            // Too many addresses -- full TLB flush is cheaper
            crate::mm::tlb::flush_all();
        } else {
            // Individual flushes for small batches (one remote request).
            crate::mm::tlb::flush_pages(&self.addresses[..self.count]);
        }
    }

    /// Number of addresses accumulated
    pub fn len(&self) -> usize {
        self.count
    }

    /// Is the batch empty?
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }
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
            stack_size: AtomicU64::new(8 * 1024 * 1024),
            tlb_generation: AtomicU64::new(0),
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
    /// (boot) page tables into this VAS's L4, plus the bootloader's physical
    /// memory mapping entry (which may be in the lower half). This shares the
    /// kernel's code, data, heap, MMIO, and physical memory access with the
    /// new process, so that the kernel remains accessible during syscalls
    /// (which run with the user's CR3).
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

            for i in 256..PAGE_TABLE_ENTRIES {
                if boot_l4[i].is_present() {
                    new_l4[i] = boot_l4[i];
                }
            }

            // Also copy the bootloader's physical memory mapping L4 entry.
            // On x86_64, PHYS_MEM_OFFSET is typically in the lower half
            // (e.g. 0x180_0000_0000 = L4 index 3). Without this, syscalls
            // running with the user's CR3 cannot access physical memory via
            // phys_to_virt_addr(), causing page faults in kernel code.
            let phys_offset = super::PHYS_MEM_OFFSET.load(core::sync::atomic::Ordering::Acquire);
            if phys_offset != 0 {
                let phys_l4_idx = ((phys_offset >> 39) & 0x1FF) as usize;
                if phys_l4_idx < 256 && boot_l4[phys_l4_idx].is_present() {
                    new_l4[phys_l4_idx] = boot_l4[phys_l4_idx];
                }
            }

            // Copy the kernel heap L4 entry. The bootloader maps the kernel
            // heap at HEAP_START (0x444444440000, L4 index 136). Without this,
            // kernel code running on the process's CR3 (interrupt handlers,
            // syscalls) cannot access heap-allocated data structures (alloc,
            // BTreeMap, Vec, etc.), causing page faults that escalate to
            // double faults.
            #[cfg(all(target_arch = "x86_64", target_os = "none"))]
            {
                let heap_start = crate::arch::x86_64::HEAP_START as u64;
                let heap_l4_idx = ((heap_start >> 39) & 0x1FF) as usize;
                if heap_l4_idx < 256 && boot_l4[heap_l4_idx].is_present() {
                    new_l4[heap_l4_idx] = boot_l4[heap_l4_idx];
                }
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
    ///
    /// On error the half-built copy is torn down (its private frames and user
    /// page tables freed) and this VAS is left without a page table, so the
    /// caller can simply drop the child.
    #[cfg(feature = "alloc")]
    pub fn clone_from(&mut self, other: &Self) -> Result<(), KernelError> {
        let result = self.clone_from_inner(other);
        if result.is_err() {
            self.discard_partial_clone();
        }
        result
    }

    /// Undo a failed [`Self::clone_from`]: free the frames it deep-copied
    /// and the user page tables it built, and clear the root.
    ///
    /// Only frames clone_from allocated are freed: borrowed mappings
    /// (device memory, shared regions) and kernel-space entries belong to
    /// the parent, as do the lower-half L4 entries copied from it.
    #[cfg(feature = "alloc")]
    fn discard_partial_clone(&mut self) {
        use super::page_table::PageTable;

        const KERNEL_SPACE_START: u64 = 0xFFFF_8000_0000_0000;

        let root = self.page_table_root.swap(0, Ordering::AcqRel);
        {
            let mut mappings = self.mappings.lock();
            let allocator = FRAME_ALLOCATOR.lock();
            for (addr, mapping) in mappings.iter() {
                if addr.0 < KERNEL_SPACE_START && mapping.owns_frames() {
                    for &frame in &mapping.physical_frames {
                        if super::frame_refs::release(frame) {
                            crate::mm::note_free_failure(
                                allocator.free_frames(frame, 1),
                                frame,
                                "vas",
                            );
                        }
                    }
                }
            }
            mappings.clear();
        }
        if root == 0 {
            return;
        }

        // SAFETY: `root` is the child's own L4 table, allocated by
        // clone_from and never loaded into CR3, so nothing else uses it.
        let l4 = unsafe { &mut *(super::phys_to_virt_addr(root) as *mut PageTable) };
        // Unshare the lower-half entries clone_from copied from the parent
        // so freeing the user tables below cannot free the parent's.
        let phys_offset = super::PHYS_MEM_OFFSET.load(Ordering::Acquire);
        if phys_offset != 0 {
            let idx = ((phys_offset >> 39) & 0x1FF) as usize;
            if idx < 256 {
                l4[idx].clear();
            }
        }
        #[cfg(all(target_arch = "x86_64", target_os = "none"))]
        {
            let idx = ((crate::arch::x86_64::HEAP_START as u64 >> 39) & 0x1FF) as usize;
            if idx < 256 {
                l4[idx].clear();
            }
        }
        free_user_page_table_frames(root);
    }

    #[cfg(feature = "alloc")]
    fn clone_from_inner(&mut self, other: &Self) -> Result<(), KernelError> {
        use super::page_table::{PageTable, PageTableHierarchy, PAGE_TABLE_ENTRIES};

        // Step 1: Allocate a new L4 page table for the child
        let new_hierarchy = PageTableHierarchy::new()?;
        let new_root = new_hierarchy.l4_addr().as_u64();
        self.page_table_root.store(new_root, Ordering::Release);

        let parent_root = other.page_table_root.load(Ordering::Acquire);

        if parent_root != 0 {
            // Step 2: Copy kernel-space L4 entries (indices 256-511) directly.
            // These are shared across all address spaces.
            // SAFETY: parent_root is a valid L4 page table physical address;
            // phys_to_virt_addr maps it into the kernel's identity-mapped region.
            let parent_l4 =
                unsafe { &*(super::phys_to_virt_addr(parent_root) as *const PageTable) };
            // SAFETY: new_root was just allocated by PageTableHierarchy::new() and is a
            // valid, zeroed L4 page table.
            let child_l4 = unsafe { &mut *(super::phys_to_virt_addr(new_root) as *mut PageTable) };

            for i in 256..PAGE_TABLE_ENTRIES {
                child_l4[i] = parent_l4[i];
            }

            // Also copy the bootloader's physical memory mapping L4 entry
            // (may be in the lower half, e.g. L4 index 3 for 0x180_0000_0000).
            let phys_offset = super::PHYS_MEM_OFFSET.load(core::sync::atomic::Ordering::Acquire);
            if phys_offset != 0 {
                let phys_l4_idx = ((phys_offset >> 39) & 0x1FF) as usize;
                if phys_l4_idx < 256 {
                    child_l4[phys_l4_idx] = parent_l4[phys_l4_idx];
                }
            }

            // Copy the kernel heap L4 entry (HEAP_START, lower half).
            #[cfg(all(target_arch = "x86_64", target_os = "none"))]
            {
                let heap_start = crate::arch::x86_64::HEAP_START as u64;
                let heap_l4_idx = ((heap_start >> 39) & 0x1FF) as usize;
                if heap_l4_idx < 256 {
                    child_l4[heap_l4_idx] = parent_l4[heap_l4_idx];
                }
            }

            // Step 3: Deep-copy user-space pages.
            // Walk parent's mappings (which track user-space regions) and for
            // each mapped page, allocate a new frame, copy content, and map.
            let parent_mappings = other.mappings.lock();
            let mut child_mappings = self.mappings.lock();
            child_mappings.clear();

            // SAFETY: parent_root is a valid identity-mapped L4 page table.
            let mut parent_mapper = unsafe { create_mapper_from_root(parent_root) };
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

                // Device memory and shared regions are mapped to the same
                // frames in the child: copying a framebuffer or a region
                // into private frames would silently stop sharing it.
                //
                // A map_page failure (page-table frame exhaustion; the tables
                // are fresh, so "already mapped" cannot happen) fails the
                // fork instead of leaving metadata for unmapped pages (review
                // of the v0.26.0 stack, PR #13).
                if !mapping.owns_frames() {
                    for (i, &frame) in mapping.physical_frames.iter().enumerate() {
                        let vaddr = VirtualAddress(mapping.start.0 + (i as u64) * 4096);
                        child_mapper
                            .map_page(vaddr, frame, mapping.flags, &mut alloc)
                            .map_err(|_| KernelError::OutOfMemory {
                                requested: 4096,
                                available: 0,
                            })?;
                    }
                    child_mappings.insert(*addr, mapping.clone());
                    continue;
                }

                // Copy-on-write (MEM-PERF-03, PROC-ARCH-01): the child maps
                // the parent's frames instead of copies. Writable pages
                // become read-only + COW in both address spaces, and the
                // first write by either side takes a private copy
                // (`resolve_cow_fault`); read-only pages are simply shared.
                // Every frame gains an owner here, so whichever side releases
                // it last frees it.
                let _ = num_pages;

                // Huge pages are not shared copy-on-write (ADR 0005): the
                // child gets its own copy of every 2 MiB chunk. A mapping may
                // span several chunks, each a separate 2 MiB block, so each
                // is copied and mapped on its own (copying the whole size
                // from the first block read past it; review of f3f2c1a).
                if mapping.flags.contains(PageFlags::HUGE) {
                    const CHUNK_FRAMES: usize = (HUGE_PAGE_SIZE / 4096) as usize;
                    let mut child_mapping = mapping.clone();
                    child_mapping.physical_frames.clear();
                    for (c, src_chunk) in mapping.physical_frames.chunks(CHUNK_FRAMES).enumerate() {
                        let src = src_chunk[0];
                        let copy = match FRAME_ALLOCATOR.lock().allocate_frames(CHUNK_FRAMES, None)
                        {
                            Ok(f) => f,
                            Err(_) => {
                                // Record what was copied so far for teardown.
                                child_mappings.insert(*addr, child_mapping);
                                return Err(KernelError::OutOfMemory {
                                    requested: HUGE_PAGE_SIZE as usize,
                                    available: 0,
                                });
                            }
                        };
                        child_mapping.physical_frames.extend(
                            (0..CHUNK_FRAMES as u64).map(|i| FrameNumber::new(copy.as_u64() + i)),
                        );
                        if src_chunk.len() != CHUNK_FRAMES
                            || copy.as_u64() % CHUNK_FRAMES as u64 != 0
                        {
                            child_mappings.insert(*addr, child_mapping);
                            return Err(KernelError::OutOfMemory {
                                requested: HUGE_PAGE_SIZE as usize,
                                available: 0,
                            });
                        }
                        // SAFETY: `src` starts one 2 MiB block of the parent
                        // (contiguous, 2 MiB aligned); `copy` is a fresh,
                        // unmapped 2 MiB block. Both are RAM in the physical
                        // map, and exactly 2 MiB is copied.
                        unsafe {
                            core::ptr::copy_nonoverlapping(
                                super::phys_to_virt_addr(src.as_u64() << 12) as *const u8,
                                super::phys_to_virt_addr(copy.as_u64() << 12) as *mut u8,
                                HUGE_PAGE_SIZE as usize,
                            );
                        }
                        let chunk_va = VirtualAddress(mapping.start.0 + c as u64 * HUGE_PAGE_SIZE);
                        if let Err(e) =
                            child_mapper.map_huge_2m(chunk_va, copy, mapping.flags, &mut alloc)
                        {
                            child_mappings.insert(*addr, child_mapping);
                            return Err(e);
                        }
                    }
                    child_mappings.insert(*addr, child_mapping);
                    continue;
                }

                let mut child_mapping = mapping.clone();
                child_mapping.physical_frames = mapping.physical_frames.clone();
                for &frame in &child_mapping.physical_frames {
                    super::frame_refs::share(frame);
                }
                // Recorded before mapping, so on failure clone_from's
                // teardown releases exactly the owners added above.
                child_mappings.insert(*addr, child_mapping);

                for i in 0..mapping.physical_frames.len() {
                    let vaddr = VirtualAddress(mapping.start.0 + (i as u64) * 4096);
                    let (frame, flags) = match parent_mapper.translate_page(vaddr) {
                        Ok(result) => result,
                        Err(_) => continue, // Page not actually mapped in HW
                    };
                    let flags = if flags.contains(PageFlags::WRITABLE) {
                        let cow = flags.without(PageFlags::WRITABLE) | PageFlags::COW;
                        parent_mapper.update_page_flags(vaddr, cow)?;
                        cow
                    } else {
                        flags
                    };
                    child_mapper
                        .map_page(vaddr, frame, flags, &mut alloc)
                        .map_err(|_| KernelError::OutOfMemory {
                            requested: 4096,
                            available: 0,
                        })?;
                }
            }

            // The parent's writable pages just became read-only: drop any
            // writable translation every CPU may still hold.
            super::tlb::flush_all();
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
                    unmap_mapping_pages(&mut mapper, mapping);
                }
            }

            // Invalidate the unmapped pages on every CPU BEFORE their frames
            // go back to the allocator, or a CPU holding a stale translation
            // could write into a frame that already belongs to someone else
            // (MEM-SEC-02).
            crate::mm::tlb::flush_all();

            // Free physical frames for each mapping that owns them
            for (_, mapping) in mappings.iter().filter(|(_, m)| m.owns_frames()) {
                let allocator = FRAME_ALLOCATOR.lock();
                for &frame in &mapping.physical_frames {
                    if super::frame_refs::release(frame) {
                        crate::mm::note_free_failure(allocator.free_frames(frame, 1), frame, "vas");
                    }
                }
            }

            // Clear all mappings
            mappings.clear();

            // NOTE: Page table frames are NOT freed here -- see clear()
            // comment. The caller must free them after switching to
            // a different CR3.
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

        // Check for overlaps using standard interval overlap test:
        // [a_start, a_end) and [b_start, b_end) overlap iff
        // a_start < b_end AND b_start < a_end.
        // The previous check missed containment (new fully contains existing)
        // and falsely rejected adjacent mappings (end == start).
        let b_start = aligned_start.0;
        let b_end = aligned_start.0 + aligned_size as u64;
        for (_, existing) in mappings.iter() {
            let a_start = existing.start.0;
            let a_end = existing.start.0 + existing.size as u64;
            if a_start < b_end && b_start < a_end {
                return Err(KernelError::AlreadyExists {
                    resource: "address range",
                    id: aligned_start.0,
                });
            }
        }

        // Allocate physical frames for the mapping
        let num_pages = aligned_size / 4096;
        let mut physical_frames = Vec::with_capacity(num_pages);

        // Allocate all frames first (hold FRAME_ALLOCATOR lock briefly).
        // On partial failure, free any already-allocated frames before
        // returning the error. Without this cleanup, OOM during a large
        // mmap would permanently leak every frame allocated before the
        // failing one.
        {
            let frame_allocator = FRAME_ALLOCATOR.lock();
            for _ in 0..num_pages {
                match frame_allocator.allocate_frames(1, None) {
                    Ok(frame) => physical_frames.push(frame),
                    Err(_) => {
                        // Free all frames allocated so far
                        for &f in &physical_frames {
                            frame_allocator.free_frames(f, 1).ok();
                        }
                        return Err(KernelError::OutOfMemory {
                            requested: 4096,
                            available: 0,
                        });
                    }
                }
            }
        } // Drop frame allocator lock before page table operations

        // Zero all allocated frames through the kernel physical memory window.
        // POSIX requires brk/mmap(MAP_ANONYMOUS) pages to be zero-filled.
        for &frame in &physical_frames {
            let phys_addr = frame.as_u64() << 12;
            let virt = crate::mm::phys_to_virt_addr(phys_addr) as *mut u8;
            // SAFETY: Each frame is a valid physical address returned by the
            // frame allocator and not yet mapped anywhere else.
            // phys_to_virt_addr maps it into the kernel's physical memory
            // window, which is always accessible in kernel context.
            unsafe {
                core::ptr::write_bytes(virt, 0, 4096);
            }
        }

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

            // Flush TLB for the entire mapped range using batched flushes.
            // TlbFlushBatch accumulates up to 16 addresses and issues a full
            // TLB flush if more are needed, reducing individual invlpg overhead.
            let mut tlb_batch = TlbFlushBatch::new();
            for i in 0..num_pages {
                let vaddr = aligned_start.0 + (i as u64) * 4096;
                tlb_batch.add(vaddr);
            }
            tlb_batch.flush();
        }

        // Record the mapping in our tracking structure
        let mut mapping = mapping;
        mapping.physical_frames = physical_frames;

        mappings.insert(aligned_start, mapping);
        Ok(())
    }

    /// Map specific physical frames into user space at a chosen virtual
    /// address.
    ///
    /// Used for framebuffer mmap: the physical frames already exist (MMIO) and
    /// must be mapped read/write into the process address space.
    #[cfg(feature = "alloc")]
    pub fn map_physical_region(
        &self,
        phys_addr: u64,
        size: usize,
        vaddr: VirtualAddress,
    ) -> Result<(), KernelError> {
        let aligned_size = ((size + 4095) / 4096) * 4096;
        let num_pages = aligned_size / 4096;
        let aligned_phys = phys_addr & !(4096 - 1);

        let flags = PageFlags::PRESENT | PageFlags::WRITABLE | PageFlags::USER;

        let pt_root = self.page_table_root.load(Ordering::Acquire);
        if pt_root == 0 {
            return Err(KernelError::InvalidState {
                expected: "initialized page table",
                actual: "null page table root",
            });
        }

        // SAFETY: pt_root is a valid L4 page table.
        let mut mapper = unsafe { create_mapper_from_root(pt_root) };
        let mut alloc = VasFrameAllocator;

        // Build frame list from the physical region
        let mut physical_frames = Vec::with_capacity(num_pages);
        for i in 0..num_pages {
            let frame = FrameNumber::new((aligned_phys >> 12) + i as u64);
            physical_frames.push(frame);
            let page_vaddr = VirtualAddress(vaddr.0 + (i as u64) * 4096);
            mapper.map_page(page_vaddr, frame, flags, &mut alloc)?;
        }

        // Flush TLB
        let mut tlb_batch = TlbFlushBatch::new();
        for i in 0..num_pages {
            tlb_batch.add(vaddr.0 + (i as u64) * 4096);
        }
        tlb_batch.flush();

        // Record mapping (no frame ownership -- these are MMIO, not allocator frames)
        let mapping = VirtualMapping {
            start: vaddr,
            size: aligned_size,
            mapping_type: MappingType::Device,
            flags,
            physical_frames,
        };
        self.mappings.lock().insert(vaddr, mapping);

        Ok(())
    }

    /// Map frames this address space borrows -- an IPC shared region's --
    /// at `at`, or at a fresh mmap address when `at` is `None`, and return
    /// the start. The mapping (`MappingType::SharedRegion`) does not own the
    /// frames: unmapping it or destroying the address space leaves them to
    /// their owner. `at` must be page-aligned user space that overlaps no
    /// existing mapping.
    #[cfg(feature = "alloc")]
    pub fn map_borrowed_frames(
        &self,
        at: Option<VirtualAddress>,
        frames: &[FrameNumber],
        flags: PageFlags,
    ) -> Result<VirtualAddress, KernelError> {
        if frames.is_empty() {
            return Err(KernelError::InvalidArgument {
                name: "frames",
                value: "empty",
            });
        }
        let size = frames.len() * 4096;
        let start = match at {
            Some(addr) => addr,
            None => VirtualAddress(
                self.next_mmap_addr
                    .fetch_add(size as u64, Ordering::Relaxed),
            ),
        };
        if start.0 % 4096 != 0 || !super::user_layout::is_user_range(start.0 as usize, size) {
            return Err(KernelError::InvalidArgument {
                name: "addr",
                value: "not page-aligned user space",
            });
        }

        let mut mappings = self.mappings.lock();
        let end = start.0 + size as u64;
        if mappings
            .values()
            .any(|m| m.start.0 < end && start.0 < m.start.0 + m.size as u64)
        {
            return Err(KernelError::AlreadyExists {
                resource: "address range",
                id: start.0,
            });
        }

        let pt_root = self.page_table_root.load(Ordering::Acquire);
        if pt_root != 0 {
            // SAFETY: pt_root is a valid L4 page table set during
            // VAS::init(); the mappings lock serializes changes to it.
            let mut mapper = unsafe { create_mapper_from_root(pt_root) };
            let mut alloc = VasFrameAllocator;
            for (i, &frame) in frames.iter().enumerate() {
                let page = VirtualAddress(start.0 + (i as u64) * 4096);
                if let Err(e) = mapper.map_page(page, frame, flags, &mut alloc) {
                    for j in 0..i {
                        let page = start.0 + (j as u64) * 4096;
                        let _ = mapper.unmap_page(VirtualAddress(page));
                        crate::mm::tlb::flush_page(page);
                    }
                    return Err(e);
                }
            }
            let mut tlb_batch = TlbFlushBatch::new();
            for i in 0..frames.len() {
                tlb_batch.add(start.0 + (i as u64) * 4096);
            }
            tlb_batch.flush();
        }

        mappings.insert(
            start,
            VirtualMapping {
                start,
                size,
                mapping_type: MappingType::SharedRegion,
                flags,
                physical_frames: frames.to_vec(),
            },
        );
        Ok(start)
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

            // Errors are ignored: a page may not have been installed in the
            // hardware table (e.g., if map_region was called before the page
            // table root was set).
            unmap_mapping_pages(&mut mapper, &mapping);
        }

        // Flush TLB for the unmapped range using batched flushes
        let mut tlb_batch = TlbFlushBatch::new();
        for i in 0..num_pages {
            let vaddr = mapping.start.0 + (i as u64) * 4096;
            tlb_batch.add(vaddr);
        }
        tlb_batch.flush();

        // Free the physical frames (only if the mapping owns them)
        if mapping.owns_frames() {
            let frame_allocator = FRAME_ALLOCATOR.lock();
            for frame in mapping.physical_frames {
                if super::frame_refs::release(frame) {
                    crate::mm::note_free_failure(
                        frame_allocator.free_frames(frame, 1),
                        frame,
                        "vas",
                    );
                }
            }
        }

        Ok(())
    }

    /// Unmap a region by address and size (POSIX-compliant partial munmap).
    ///
    /// Supports three cases:
    /// 1. **Exact match**: `addr` and `size` match a BTreeMap entry → remove
    ///    it.
    /// 2. **Front trim**: `addr` matches the start of a larger mapping → shrink
    ///    the mapping and free the leading pages.
    /// 3. **Back trim**: `addr+size` matches the end of a mapping → shrink from
    ///    the back.
    /// 4. **Hole punch**: Range is in the middle of a mapping → split into two.
    /// 5. **Sub-range not at start**: `addr` is inside a mapping → find the
    ///    containing mapping and trim/punch accordingly.
    ///
    /// GCC's ggc garbage collector relies on partial munmap to free individual
    /// pages within larger mmap pools. Without this, munmap(pool_start, 4KB)
    /// would destroy the entire multi-MB pool.
    #[cfg(feature = "alloc")]
    pub fn unmap(&self, start_addr: usize, size: usize) -> Result<(), KernelError> {
        let unmap_start = (start_addr & !(4096 - 1)) as u64;
        let unmap_size = ((size + 4095) / 4096) * 4096;
        let unmap_end = unmap_start + unmap_size as u64;

        // First try exact-key match (fast path, most common for our small mmaps)
        let addr = VirtualAddress(unmap_start);
        let mut mappings = self.mappings.lock();

        if let Some(existing) = mappings.get(&addr) {
            if existing.size == unmap_size {
                // Exact match: remove entire mapping
                drop(mappings);
                return self.unmap_region(addr);
            }
        }

        // Find the mapping that CONTAINS the requested unmap range.
        // This handles partial munmap within a larger mmap.
        let mut containing_key = None;
        for (key, mapping) in mappings.iter() {
            let m_start = key.0;
            let m_end = m_start + mapping.size as u64;
            if m_start <= unmap_start && m_end >= unmap_end {
                containing_key = Some(*key);
                break;
            }
        }

        let containing_key = match containing_key {
            Some(k) => k,
            None => {
                // No containing mapping found. If the exact key exists but with
                // a different size, fall back to removing the entire mapping
                // (original behavior, for backwards compat with code that passes
                // size=0 or incorrect size).
                if mappings.contains_key(&addr) {
                    drop(mappings);
                    return self.unmap_region(addr);
                }
                return Err(KernelError::NotFound {
                    resource: "memory region",
                    id: unmap_start,
                });
            }
        };

        // A 2 MiB page is unmapped whole; splitting one is not supported.
        if mappings
            .get(&containing_key)
            .is_some_and(|m| m.flags.contains(PageFlags::HUGE))
            && (unmap_start != containing_key.0
                || unmap_size as u64 != mappings[&containing_key].size as u64)
        {
            return Err(KernelError::InvalidArgument {
                name: "munmap range",
                value: "part of a huge-page mapping",
            });
        }

        // Remove the containing mapping from BTreeMap
        let mapping = mappings
            .remove(&containing_key)
            .ok_or(KernelError::NotFound {
                resource: "vas_mapping",
                id: containing_key.0 as u64,
            })?;
        let m_start = containing_key.0;

        // Calculate page indices within the mapping for the unmap range
        let unmap_page_start = ((unmap_start - m_start) / 4096) as usize;
        let unmap_page_count = unmap_size / 4096;
        let unmap_page_end = unmap_page_start + unmap_page_count;

        // Unmap the requested pages from the page table
        let pt_root = self.page_table_root.load(Ordering::Acquire);
        if pt_root != 0 {
            // SAFETY: pt_root is a valid L4 page table address set during VAS::init().
            let mut mapper = unsafe { create_mapper_from_root(pt_root) };
            if mapping.flags.contains(PageFlags::HUGE) {
                // Only whole huge mappings get here (checked above).
                unmap_mapping_pages(&mut mapper, &mapping);
            } else {
                for i in unmap_page_start..unmap_page_end {
                    let vaddr = VirtualAddress(m_start + (i as u64) * 4096);
                    let _ = mapper.unmap_page(vaddr);
                }
            }
        }

        // Flush TLB for unmapped pages using batched flushes
        let mut tlb_batch = TlbFlushBatch::new();
        for i in unmap_page_start..unmap_page_end {
            let vaddr = m_start + (i as u64) * 4096;
            tlb_batch.add(vaddr);
        }
        tlb_batch.flush();

        // Free the physical frames for the unmapped range
        if mapping.owns_frames() {
            let frame_allocator = FRAME_ALLOCATOR.lock();
            for i in unmap_page_start..unmap_page_end.min(mapping.physical_frames.len()) {
                if !super::frame_refs::release(mapping.physical_frames[i]) {
                    continue; // still mapped by a copy-on-write sibling
                }
                crate::mm::note_free_failure(
                    frame_allocator.free_frames(mapping.physical_frames[i], 1),
                    mapping.physical_frames[i],
                    "vas",
                );
            }
        }

        // Re-insert the remaining parts of the mapping

        // Front portion: pages [0..unmap_page_start)
        if unmap_page_start > 0 {
            let front_size = unmap_page_start * 4096;
            let mut front = VirtualMapping::new(containing_key, front_size, mapping.mapping_type);
            front.flags = mapping.flags;
            if unmap_page_start <= mapping.physical_frames.len() {
                front.physical_frames = mapping.physical_frames[..unmap_page_start].to_vec();
            }
            mappings.insert(containing_key, front);
        }

        // Back portion: pages [unmap_page_end..total_pages)
        let total_pages = mapping.size / 4096;
        if unmap_page_end < total_pages {
            let back_start_addr = m_start + (unmap_page_end as u64) * 4096;
            let back_size = (total_pages - unmap_page_end) * 4096;
            let mut back = VirtualMapping::new(
                VirtualAddress(back_start_addr),
                back_size,
                mapping.mapping_type,
            );
            back.flags = mapping.flags;
            if unmap_page_end < mapping.physical_frames.len() {
                back.physical_frames = mapping.physical_frames[unmap_page_end..].to_vec();
            }
            mappings.insert(VirtualAddress(back_start_addr), back);
        }

        Ok(())
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

    /// Resolve a write fault at `vaddr` on a copy-on-write page.
    ///
    /// Returns `Ok(false)` if the page is not a COW page (some other fault),
    /// `Ok(true)` once the writer has a writable page. If another address
    /// space still shares the frame, the page is copied into a new private
    /// frame; if this was the last sharer, the page is simply made writable
    /// again. The mapping itself must be writable: COW never grants a write
    /// the process could not otherwise make.
    #[cfg(feature = "alloc")]
    pub fn resolve_cow_fault(&self, vaddr: u64) -> Result<bool, KernelError> {
        let page = vaddr & !0xFFF;
        let root = self.page_table_root.load(Ordering::Acquire);
        if root == 0 {
            return Ok(false);
        }
        // SAFETY: `root` is this address space's L4 table, reached through
        // the physical map; mutation is serialised by the mappings lock
        // taken below (the fault path holds the memory-space lock too).
        let mut mapper = unsafe { create_mapper_from_root(root) };
        let (frame, flags) = match mapper.translate_page(VirtualAddress(page)) {
            Ok(r) => r,
            Err(_) => return Ok(false),
        };
        if !flags.contains(PageFlags::COW) {
            return Ok(false);
        }

        let mut mappings = self.mappings.lock();
        let mapping = mappings
            .values_mut()
            .find(|m| m.contains(VirtualAddress(page)))
            .ok_or(KernelError::InvalidAddress {
                addr: vaddr as usize,
            })?;
        // COW in a PTE means "logically writable, frame shared": every path
        // that changes protection goes through `cow_safe_flags`, so the bit
        // is never left on a page the process may not write (the mapping's
        // own flags can be stale after an mprotect of part of it).
        let index = ((page - mapping.start.0) / 4096) as usize;
        let writable = flags.without(PageFlags::COW) | PageFlags::WRITABLE;

        if super::frame_refs::is_shared(frame) {
            let copy = FRAME_ALLOCATOR
                .lock()
                .allocate_frames(1, None)
                .map_err(|_| KernelError::OutOfMemory {
                    requested: 4096,
                    available: 0,
                })?;
            // SAFETY: both frames are RAM reached through the physical map;
            // `copy` was just allocated and is not mapped anywhere yet.
            unsafe {
                core::ptr::copy_nonoverlapping(
                    super::phys_to_virt_addr(frame.as_u64() << 12) as *const u8,
                    super::phys_to_virt_addr(copy.as_u64() << 12) as *mut u8,
                    4096,
                );
            }
            let _ = mapper.unmap_page(VirtualAddress(page));
            if let Err(e) =
                mapper.map_page(VirtualAddress(page), copy, writable, &mut VasFrameAllocator)
            {
                // Put the shared page back so the process keeps a valid
                // (read-only) mapping, and give up the copy.
                let _ = mapper.map_page(VirtualAddress(page), frame, flags, &mut VasFrameAllocator);
                crate::mm::note_free_failure(
                    FRAME_ALLOCATOR.lock().free_frames(copy, 1),
                    copy,
                    "vas",
                );
                return Err(e);
            }
            if let Some(slot) = mapping.physical_frames.get_mut(index) {
                *slot = copy;
            }
            drop(mappings);
            // Flush before the shared frame can lose its last owner.
            super::tlb::flush_page(page);
            if super::frame_refs::release(frame) {
                // The other sharer let go meanwhile.
                crate::mm::note_free_failure(
                    FRAME_ALLOCATOR.lock().free_frames(frame, 1),
                    frame,
                    "vas",
                );
            }
        } else {
            mapper.update_page_flags(VirtualAddress(page), writable)?;
            drop(mappings);
            super::tlb::flush_page(page);
        }
        Ok(true)
    }

    /// Get a reference to the underlying mappings BTreeMap.
    ///
    /// Used by COW fork to iterate user-space pages and by diagnostics.
    /// The caller must lock the returned Mutex before accessing entries.
    pub fn mappings_ref(
        &self,
    ) -> &spin::Mutex<alloc::collections::BTreeMap<VirtualAddress, VirtualMapping>> {
        &self.mappings
    }

    /// Map a specific virtual address using a pre-allocated physical frame.
    ///
    /// Unlike `map_page` (which allocates its own frame), this takes an
    /// existing frame -- used by demand paging and COW fault handlers.
    #[cfg(feature = "alloc")]
    pub fn map_page_with_frame(
        &mut self,
        vaddr: usize,
        frame: super::FrameNumber,
        flags: PageFlags,
    ) -> Result<(), KernelError> {
        let vaddr_obj = VirtualAddress(vaddr as u64);
        let pt_root = self.page_table_root.load(Ordering::Acquire);
        if pt_root != 0 {
            // SAFETY: pt_root is a valid L4 page table address set during
            // VAS::init(). We have &mut self for exclusive access.
            let mut mapper = unsafe { create_mapper_from_root(pt_root) };
            let mut alloc = VasFrameAllocator;
            mapper.map_page(vaddr_obj, frame, flags, &mut alloc)?;
            crate::mm::tlb::flush_page(vaddr as u64);
        }
        Ok(())
    }

    /// Re-map a virtual address to a different physical frame (for COW).
    ///
    /// Unmaps the old mapping and installs the new frame with the given flags.
    #[cfg(feature = "alloc")]
    pub fn remap_page(
        &mut self,
        vaddr: usize,
        new_frame: super::FrameNumber,
        flags: PageFlags,
    ) -> Result<(), KernelError> {
        let vaddr_obj = VirtualAddress(vaddr as u64);
        let pt_root = self.page_table_root.load(Ordering::Acquire);
        if pt_root != 0 {
            // SAFETY: Same as map_page_with_frame.
            let mut mapper = unsafe { create_mapper_from_root(pt_root) };
            let mut alloc = VasFrameAllocator;
            // Unmap old entry (ignore error if not currently mapped)
            let _ = mapper.unmap_page(vaddr_obj);
            mapper.map_page(vaddr_obj, new_frame, flags, &mut alloc)?;
            crate::mm::tlb::flush_page(vaddr as u64);
        }
        Ok(())
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

        // Skip physical page mapping in host tests (no frame allocator available)
        #[cfg(all(feature = "alloc", not(test)))]
        self.map_region(addr, aligned_size, mapping_type)?;

        Ok(addr)
    }

    /// Return the base address of the user heap region.
    pub fn heap_start_addr(&self) -> u64 {
        self.heap_start.load(Ordering::Relaxed)
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
    ///
    /// All heap pages are tracked in a SINGLE consolidated BTreeMap entry
    /// keyed at the heap start page. This avoids creating one entry per brk()
    /// call, which previously caused 50,000+ entries and O(n^2) slowdown.
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
                    // In bare-metal alloc builds, map the physical pages.
                    // In host test builds, skip physical mapping (no frame allocator).
                    #[cfg(all(feature = "alloc", not(test)))]
                    {
                        if self.brk_extend_heap(old_page, new_page).is_ok() {
                            self.heap_break.store(addr.0, Ordering::Release);
                        }
                        // On failure, leave break unchanged
                    }
                    #[cfg(any(not(feature = "alloc"), test))]
                    {
                        // Without alloc or in tests: just move the pointer
                        self.heap_break.store(addr.0, Ordering::Release);
                    }
                } else {
                    // Within the same page, just update the pointer
                    self.heap_break.store(addr.0, Ordering::Release);
                }
            } else if addr.0 < current && addr.0 >= heap_start {
                // Shrink attempt: brk only grows, so ignore requests to
                // decrease the break. Return current break
                // unchanged.
            }
        }

        VirtualAddress(self.heap_break.load(Ordering::Acquire))
    }

    /// Extend the heap by mapping pages [old_page..new_page).
    ///
    /// Instead of calling `map_region()` (which creates a new BTreeMap entry
    /// each time), this method maintains a SINGLE consolidated heap mapping.
    /// The first call creates the entry; subsequent calls extend it in-place.
    /// This reduces the mapping count from O(brk_calls) to O(1) and avoids
    /// the O(n) overlap check in `map_region()`.
    #[cfg(all(feature = "alloc", not(test)))]
    fn brk_extend_heap(&self, old_page: u64, new_page: u64) -> Result<(), KernelError> {
        let delta_pages = (new_page - old_page) as usize;
        let start_addr = VirtualAddress(old_page * 4096);

        // Allocate physical frames
        let mut new_frames = Vec::with_capacity(delta_pages);
        {
            let frame_allocator = FRAME_ALLOCATOR.lock();
            for _ in 0..delta_pages {
                match frame_allocator.allocate_frames(1, None) {
                    Ok(frame) => new_frames.push(frame),
                    Err(_) => {
                        for &f in &new_frames {
                            frame_allocator.free_frames(f, 1).ok();
                        }
                        return Err(KernelError::OutOfMemory {
                            requested: 4096,
                            available: 0,
                        });
                    }
                }
            }
        }

        // Zero the frames (POSIX requires zero-filled pages)
        for &frame in &new_frames {
            let phys_addr = frame.as_u64() << 12;
            let virt = crate::mm::phys_to_virt_addr(phys_addr) as *mut u8;
            // SAFETY: Frame is freshly allocated; phys_to_virt_addr maps it into the
            // kernel's identity-mapped region.
            unsafe {
                core::ptr::write_bytes(virt, 0, 4096);
            }
        }

        // Map into page tables
        let pt_root = self.page_table_root.load(Ordering::Acquire);
        if pt_root != 0 {
            // SAFETY: pt_root is a valid L4 page table address set during VAS::init().
            let mut mapper = unsafe { create_mapper_from_root(pt_root) };
            let mut alloc = VasFrameAllocator;
            let flags = PageFlags::PRESENT | PageFlags::WRITABLE | PageFlags::USER;

            for (i, &frame) in new_frames.iter().enumerate() {
                let vaddr = VirtualAddress(start_addr.0 + (i as u64) * 4096);
                mapper.map_page(vaddr, frame, flags, &mut alloc)?;
                crate::mm::tlb::flush_page(vaddr.0);
            }
        }

        // Extend existing heap mapping or create initial one
        let heap_start_page = (self.heap_start.load(Ordering::Relaxed) + 4095) / 4096;
        let heap_key = VirtualAddress(heap_start_page * 4096);

        let mut mappings = self.mappings.lock();
        if let Some(mapping) = mappings.get_mut(&heap_key) {
            // Extend existing consolidated heap mapping
            mapping.size += delta_pages * 4096;
            mapping.physical_frames.extend_from_slice(&new_frames);
        } else {
            // First heap allocation: create consolidated mapping
            let total_size = ((new_page - heap_start_page) as usize) * 4096;
            let mut mapping = VirtualMapping::new(heap_key, total_size, MappingType::Heap);
            mapping.physical_frames = new_frames;
            mapping.flags = PageFlags::PRESENT | PageFlags::WRITABLE | PageFlags::USER;
            mappings.insert(heap_key, mapping);
        }

        Ok(())
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
            if let Some(flags) = cow_safe_flags(&mapper, vaddr, new_flags) {
                let _ = mapper.update_page_flags(vaddr, flags);
            }
            crate::mm::tlb::flush_page(vaddr.0);
        }

        // Update the mapping metadata flags too
        let mut mappings = self.mappings.lock();
        if let Some(mapping) = mappings.get_mut(&start) {
            mapping.flags = new_flags;
        }

        Ok(())
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
                    unmap_mapping_pages(&mut mapper, mapping);
                }
            }

            // Invalidate the unmapped pages on every CPU BEFORE their frames
            // go back to the allocator, or a CPU holding a stale translation
            // could write into a frame that already belongs to someone else
            // (MEM-SEC-02).
            crate::mm::tlb::flush_all();

            // Free physical frames for each mapping that owns them
            for (_, mapping) in mappings.iter().filter(|(_, m)| m.owns_frames()) {
                let frame_allocator = FRAME_ALLOCATOR.lock();
                for frame in &mapping.physical_frames {
                    if super::frame_refs::release(*frame) {
                        frame_allocator.free_frames(*frame, 1).ok();
                    }
                }
            }

            // Clear all mappings
            mappings.clear();

            // The TLB was flushed above, before the data frames were freed,
            // which also covers the page table subtrees freed below.

            // Free user-space page table subtree frames (L3/L2/L1) now that
            // all user PTEs have been cleared and the TLB flushed. The L4
            // frame itself is NOT freed because it may be the active CR3;
            // freeing it would cause a triple fault on the next TLB miss.
            // The L4 frame is freed later by the boot wrapper (e.g.,
            // run_user_process_scheduled) after the boot CR3 is restored.
            //
            // Freeing subtrees here (rather than deferring to the boot
            // wrapper) is critical for the exec path: exec calls clear()
            // then init(), which allocates a NEW L4 and overwrites
            // page_table_root. Without freeing the old subtrees here, they
            // would be leaked because the old L4 address is overwritten and
            // the boot wrapper only frees the pre-exec L4 (saved before
            // entering user mode).
            if pt_root != 0 {
                free_user_page_table_subtrees(pt_root);
            }
        }

        // Reset metadata
        self.heap_break
            .store(self.heap_start.load(Ordering::Relaxed), Ordering::Release);
        self.next_mmap_addr
            .store(0x4000_0000_0000, Ordering::Release);
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
                        unmap_mapping_pages(&mut mapper, mapping);
                    }
                }
            }

            // Invalidate the unmapped pages on every CPU BEFORE their frames
            // go back to the allocator, or a CPU holding a stale translation
            // could write into a frame that already belongs to someone else
            // (MEM-SEC-02).
            crate::mm::tlb::flush_all();

            // Free physical frames and remove mappings
            for addr in &to_remove {
                if let Some(mapping) = mappings.get(addr).filter(|m| m.owns_frames()) {
                    let frame_allocator = FRAME_ALLOCATOR.lock();
                    for frame in &mapping.physical_frames {
                        if super::frame_refs::release(*frame) {
                            frame_allocator.free_frames(*frame, 1).ok();
                        }
                    }
                }
            }

            for addr in to_remove {
                mappings.remove(&addr);
            }

            // NOTE: Page table subtree frames (L3/L2/L1) are NOT freed here
            // because clear_user_space() runs during exec while the process's
            // CR3 is still active. Freeing intermediate table frames would
            // corrupt the active page table hierarchy. The old page table
            // frames are reused by subsequent map_region calls since their L1
            // entries were already unmapped above (all slots are non-present).
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
        let size = self.stack_size.load(Ordering::Acquire);
        (self.stack_top.load(Ordering::Acquire) - size) as usize
    }

    /// Get user stack size
    pub fn user_stack_size(&self) -> usize {
        self.stack_size.load(Ordering::Acquire) as usize
    }

    /// Get stack top address
    pub fn stack_top(&self) -> usize {
        self.stack_top.load(Ordering::Acquire) as usize
    }

    /// Set stack top address
    pub fn set_stack_top(&self, addr: usize) {
        self.stack_top.store(addr as u64, Ordering::Release);
    }

    /// Set stack size in bytes
    pub fn set_stack_size(&self, size: usize) {
        self.stack_size.store(size as u64, Ordering::Release);
    }

    /// Map a single page at a virtual address
    pub fn map_page(&mut self, vaddr: usize, flags: PageFlags) -> Result<(), KernelError> {
        use super::PAGE_SIZE;

        // Allocate a physical frame via per-CPU cache (avoids global lock)
        let frame = crate::mm::frame_allocator::per_cpu_alloc_frame().map_err(|_| {
            KernelError::OutOfMemory {
                requested: 4096,
                available: 0,
            }
        })?;

        // Zero the frame before mapping. POSIX requires freshly mapped pages
        // to be zero-filled, and the ELF loader relies on this for BSS.
        let phys_addr = frame.as_u64() << 12;
        let virt = crate::mm::phys_to_virt_addr(phys_addr) as *mut u8;
        // SAFETY: frame is a valid physical address just allocated by the
        // frame allocator. phys_to_virt_addr maps it into the kernel's
        // physical memory window, so the 4 KiB page is writable and unshared.
        unsafe {
            core::ptr::write_bytes(virt, 0, 4096);
        }

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
            match mapper.map_page(vaddr_obj, frame, flags, &mut alloc) {
                Ok(()) => {}
                Err(KernelError::AlreadyExists { .. }) => {
                    // Page already mapped by a previous segment (e.g.,
                    // overlapping LOAD segments sharing a boundary page).
                    // Update flags to the union of old and new, then free
                    // the unused frame we just allocated.
                    if let Some(flags) = cow_safe_flags(&mapper, vaddr_obj, flags) {
                        let _ = mapper.update_page_flags(vaddr_obj, flags);
                    }
                    crate::mm::note_free_failure(
                        FRAME_ALLOCATOR.lock().free_frames(frame, 1),
                        frame,
                        "vas",
                    );
                    crate::mm::tlb::flush_page(vaddr as u64);
                    return Ok(());
                }
                Err(e) => return Err(e),
            }
            crate::mm::tlb::flush_page(vaddr as u64);
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

    /// Map a 2MB huge page at the given virtual address.
    ///
    /// Allocates 512 contiguous 4KB frames (= 2MB) and installs a single
    /// L2 page table entry with the HUGE flag set. This reduces TLB pressure
    /// for large contiguous allocations (heap, framebuffer, DMA).
    ///
    /// The virtual address must be 2MB-aligned.
    pub fn map_huge_page(&self, vaddr: usize, flags: PageFlags) -> Result<(), KernelError> {
        self.map_huge_region(vaddr, HUGE_PAGE_SIZE as usize, flags, MappingType::Data)
    }

    /// Map `size` bytes (a multiple of 2 MiB) at the 2 MiB-aligned `vaddr`
    /// as huge pages (MEM-ARCH-01): each 2 MiB is an L2 leaf over 512
    /// contiguous, 2 MiB-aligned, zeroed frames. One mapping records every
    /// frame (so the per-frame free paths release all of them) and carries
    /// `PageFlags::HUGE` (so unmapping removes the leaves).
    pub fn map_huge_region(
        &self,
        vaddr: usize,
        size: usize,
        flags: PageFlags,
        mapping_type: MappingType,
    ) -> Result<(), KernelError> {
        const HUGE_PAGE_FRAMES: usize = (HUGE_PAGE_SIZE / 4096) as usize; // 512

        if vaddr as u64 & (HUGE_PAGE_SIZE - 1) != 0
            || size == 0
            || size as u64 & (HUGE_PAGE_SIZE - 1) != 0
        {
            return Err(KernelError::InvalidArgument {
                name: "huge page region",
                value: "address or size not a multiple of 2 MiB",
            });
        }
        let pt_root = self.page_table_root.load(Ordering::Acquire);
        // SAFETY: Same as map_page -- pt_root is this address space's L4.
        let mut mapper = (pt_root != 0).then(|| unsafe { create_mapper_from_root(pt_root) });

        let release = |frame: FrameNumber| {
            let allocator = FRAME_ALLOCATOR.lock();
            for i in 0..HUGE_PAGE_FRAMES as u64 {
                let f = FrameNumber::new(frame.as_u64() + i);
                crate::mm::note_free_failure(allocator.free_frames(f, 1), f, "vas");
            }
        };

        let mut chunks: Vec<FrameNumber> = Vec::with_capacity(size / HUGE_PAGE_SIZE as usize);
        let mut failure = None;
        for c in 0..size / HUGE_PAGE_SIZE as usize {
            let chunk_va = VirtualAddress(vaddr as u64 + c as u64 * HUGE_PAGE_SIZE);
            // 512 frames from the buddy allocator, whose blocks of that size
            // are 2 MiB aligned (its region starts on a 2 MiB boundary);
            // checked anyway, since an L2 leaf requires it.
            let frame = match FRAME_ALLOCATOR
                .lock()
                .allocate_frames(HUGE_PAGE_FRAMES, None)
            {
                Ok(f) if f.as_u64() % HUGE_PAGE_FRAMES as u64 == 0 => f,
                Ok(f) => {
                    release(f);
                    failure = Some(KernelError::OutOfMemory {
                        requested: size,
                        available: 0,
                    });
                    break;
                }
                Err(_) => {
                    failure = Some(KernelError::OutOfMemory {
                        requested: size,
                        available: 0,
                    });
                    break;
                }
            };
            // SAFETY: freshly allocated contiguous frames in the physical map.
            unsafe {
                core::ptr::write_bytes(
                    super::phys_to_virt_addr(frame.as_u64() * 4096) as *mut u8,
                    0,
                    HUGE_PAGE_SIZE as usize,
                );
            }
            if let Some(m) = mapper.as_mut() {
                if let Err(e) = m.map_huge_2m(chunk_va, frame, flags, &mut VasFrameAllocator) {
                    release(frame);
                    failure = Some(e);
                    break;
                }
            }
            chunks.push(frame);
        }

        if let Some(e) = failure {
            // Undo the chunks already mapped.
            for (c, &frame) in chunks.iter().enumerate() {
                if let Some(m) = mapper.as_mut() {
                    let _ =
                        m.unmap_huge_2m(VirtualAddress(vaddr as u64 + c as u64 * HUGE_PAGE_SIZE));
                }
                crate::mm::tlb::flush_all();
                release(frame);
            }
            return Err(e);
        }
        crate::mm::tlb::flush_all();

        #[cfg(feature = "alloc")]
        {
            let mut new_mapping =
                VirtualMapping::new(VirtualAddress(vaddr as u64), size, mapping_type);
            new_mapping.physical_frames = chunks
                .iter()
                .flat_map(|f| {
                    (0..HUGE_PAGE_FRAMES as u64).map(move |i| FrameNumber::new(f.as_u64() + i))
                })
                .collect();
            new_mapping.flags = flags | PageFlags::HUGE;
            self.mappings
                .lock()
                .insert(VirtualAddress(vaddr as u64), new_mapping);
        }

        Ok(())
    }

    /// Like [`Self::mmap`], but backed by 2 MiB pages: `size` is rounded up
    /// to 2 MiB and the address is 2 MiB aligned (`mmap(MAP_HUGETLB)`).
    #[cfg(feature = "alloc")]
    pub fn mmap_huge(
        &self,
        size: usize,
        mapping_type: MappingType,
    ) -> Result<VirtualAddress, KernelError> {
        let size = (size as u64).div_ceil(HUGE_PAGE_SIZE) * HUGE_PAGE_SIZE;
        let base = self
            .next_mmap_addr
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |next| {
                Some(next.next_multiple_of(HUGE_PAGE_SIZE) + size)
            })
            .map(|prev| prev.next_multiple_of(HUGE_PAGE_SIZE))
            .unwrap_or(0);
        let flags = VirtualMapping::new(VirtualAddress(base), 0, mapping_type).flags;
        self.map_huge_region(base as usize, size as usize, flags, mapping_type)?;
        Ok(VirtualAddress(base))
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

/// Map a physical memory region into the current process's user-space address
/// space.
///
/// Allocates a virtual address range via the process's VAS mmap region and
/// maps the given physical frames into it. Used for framebuffer mmap.
///
/// Returns the user-space virtual address of the mapping.
pub fn map_physical_region_user(
    phys_addr: u64,
    size: usize,
) -> Result<usize, crate::syscall::SyscallError> {
    let proc =
        crate::process::current_process().ok_or(crate::syscall::SyscallError::InvalidState)?;

    let memory_space = proc.memory_space.lock();

    // Allocate a virtual address range from the mmap region
    let aligned_size = ((size + 4095) / 4096) * 4096;
    let vaddr = VirtualAddress(
        memory_space
            .next_mmap_addr
            .fetch_add(aligned_size as u64, Ordering::Relaxed),
    );

    // Map the physical frames
    #[cfg(feature = "alloc")]
    memory_space
        .map_physical_region(phys_addr, aligned_size, vaddr)
        .map_err(|_| crate::syscall::SyscallError::OutOfMemory)?;

    Ok(vaddr.as_usize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protection_change_keeps_shared_frames_copy_on_write() {
        let rw = PageFlags::PRESENT | PageFlags::USER | PageFlags::WRITABLE;
        let ro = PageFlags::PRESENT | PageFlags::USER;
        let cow = ro | PageFlags::COW;
        // mprotect(PROT_WRITE) on a page a fork still shares: no direct write.
        assert_eq!(cow_flags_for(cow, rw, true), cow);
        // A shared frame made read-only earlier (COW dropped) stays protected.
        assert_eq!(cow_flags_for(ro, rw, true), cow);
        // Still marked COW but no longer shared: the fault path finishes it.
        assert_eq!(cow_flags_for(cow, rw, false), cow);
        // Read-only request drops COW; the PTE stays read-only.
        assert_eq!(cow_flags_for(cow, ro, true), ro);
        // A caller cannot set COW itself; private frames become writable.
        assert_eq!(cow_flags_for(ro, rw | PageFlags::COW, false), rw);
    }

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
    fn test_map_borrowed_frames_validates_and_records() {
        let vas = VirtualAddressSpace::new();
        let frames = [FrameNumber::new(0x100), FrameNumber::new(0x101)];
        let flags = PageFlags::PRESENT | PageFlags::USER | PageFlags::WRITABLE;
        let at = VirtualAddress(0x4000_0000);
        assert_eq!(vas.map_borrowed_frames(Some(at), &frames, flags), Ok(at));
        let m = vas.find_mapping(VirtualAddress(0x4000_1000)).unwrap();
        assert_eq!(m.mapping_type, MappingType::SharedRegion);
        assert_eq!(m.physical_frames, frames);
        assert!(!m.owns_frames());
        // Overlap, misalignment, kernel half and the reserved top page.
        assert!(vas
            .map_borrowed_frames(Some(VirtualAddress(0x4000_1000)), &frames, flags)
            .is_err());
        assert!(vas
            .map_borrowed_frames(Some(VirtualAddress(0x5000_0010)), &frames, flags)
            .is_err());
        assert!(vas
            .map_borrowed_frames(Some(VirtualAddress(0xFFFF_8000_0000_0000)), &frames, flags)
            .is_err());
        let top = super::super::user_layout::USER_SPACE_END as u64 - 0x1000;
        assert!(vas
            .map_borrowed_frames(Some(VirtualAddress(top)), &frames, flags)
            .is_err());
        // Kernel-chosen address.
        let chosen = vas.map_borrowed_frames(None, &frames, flags).unwrap();
        assert_ne!(chosen, at);
        assert!(vas.find_mapping(chosen).is_some());
        // (unmap_region flushes the TLB, a privileged instruction, so it is
        // exercised by the in-kernel runs, not here.)
    }

    #[test]
    fn test_only_allocator_backed_mappings_own_their_frames() {
        // N-32: unmapping a device or shared-region mapping must not hand
        // its frames to the allocator.
        let at = VirtualAddress(0x4000_0000);
        for (ty, owns) in [
            (MappingType::Code, true),
            (MappingType::Data, true),
            (MappingType::Stack, true),
            (MappingType::Heap, true),
            (MappingType::File, true),
            (MappingType::Shared, true),
            (MappingType::Device, false),
            (MappingType::SharedRegion, false),
        ] {
            assert_eq!(
                VirtualMapping::new(at, 4096, ty).owns_frames(),
                owns,
                "{:?}",
                ty
            );
        }
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
