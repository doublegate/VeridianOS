//! Memory management subsystem
//!
//! This module handles physical and virtual memory management,
//! including page tables, allocators, and memory protection.

// Memory management core -- many APIs exercised at boot and during allocation
#![allow(dead_code)]

pub mod bootloader;
pub mod cache_aligned;
pub mod cache_topology;
pub mod demand_paging;
pub mod frame_allocator;
pub mod heap;
pub mod ksm;
pub mod page_fault;
pub mod page_table;
pub mod user_layout;
pub mod user_validation;
pub mod vas;

#[cfg(feature = "alloc")]
extern crate alloc;
#[cfg(feature = "alloc")]
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};

pub(crate) use frame_allocator::FRAME_ALLOCATOR;
// Re-export commonly used types
pub use frame_allocator::{
    note_free_failure, FrameAllocatorError, FrameNumber, PhysicalAddress, PhysicalFrame, FRAME_SIZE,
};
pub use heap::init as init_heap;
pub use user_validation::{is_user_addr_valid, translate_address as translate_user_address};
pub use vas::VirtualAddressSpace;

/// Page size constant (4KB)
pub const PAGE_SIZE: usize = 4096;

/// Physical memory offset: virtual = physical + PHYS_MEM_OFFSET.
///
/// On x86_64 with bootloader 0.11, physical memory is mapped at a dynamic
/// offset provided by the bootloader. On AArch64 and RISC-V, physical
/// memory is identity-mapped (offset = 0).
pub(crate) static PHYS_MEM_OFFSET: AtomicU64 = AtomicU64::new(0);

/// Set the physical memory offset (called once during early boot).
pub fn set_phys_mem_offset(offset: u64) {
    PHYS_MEM_OFFSET.store(offset, Ordering::Release);
}

/// Convert a physical address to a virtual pointer.
///
/// On x86_64, adds the bootloader's physical memory mapping offset.
/// On AArch64/RISC-V, returns the address unchanged (identity-mapped).
#[inline]
pub fn phys_to_virt_addr(phys: u64) -> u64 {
    phys + PHYS_MEM_OFFSET.load(Ordering::Acquire)
}

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
        // SAFETY: Reading CR3 is a privileged, read-only operation that returns the
        // current page table root physical address. It has no side effects and is
        // always valid in ring 0 (kernel mode).
        unsafe {
            core::arch::asm!("mov {}, cr3", out(reg) cr3);
        }
        cr3 as usize
    }

    #[cfg(target_arch = "aarch64")]
    {
        // TTBR0_EL1 holds the page table base
        let ttbr0: u64;
        // SAFETY: Reading TTBR0_EL1 is a read-only system register access that
        // returns the EL0/EL1 page table base address. It has no side effects and
        // is always accessible at EL1 (kernel mode).
        unsafe {
            core::arch::asm!("mrs {}, TTBR0_EL1", out(reg) ttbr0);
        }
        ttbr0 as usize
    }

    #[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
    {
        // SATP holds the page table base
        let satp: usize;
        // SAFETY: Reading the SATP CSR is a read-only operation that returns the
        // page table configuration (mode + ASID + PPN). It has no side effects and
        // is always accessible in S-mode (supervisor/kernel mode).
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
#[allow(unused_variables, unused_assignments)]
pub fn init(memory_map: &[MemoryRegion]) {
    kprintln!("[MM] Initializing memory management...");

    // Initialize frame allocator with available memory regions
    kprintln!("[MM] Getting frame allocator lock...");

    // Get frame allocator lock and initialize memory regions
    {
        let mut allocator = FRAME_ALLOCATOR.lock();

        kprintln!("[MM] Frame allocator locked successfully");

        #[allow(unused_assignments)]
        let mut total_memory = 0u64;
        #[allow(unused_assignments)]
        let mut usable_memory = 0u64;

        for (idx, region) in memory_map.iter().enumerate() {
            kprintln!("[MM] Processing memory region");

            total_memory += region.size;

            if region.usable {
                usable_memory += region.size;

                let start_frame = FrameNumber::new(region.start / FRAME_SIZE as u64);
                let frame_count = region.size as usize / FRAME_SIZE;

                // Use region index as NUMA node for now
                let numa_node = idx.min(7); // Max 8 NUMA nodes

                kprintln!("[MM] Initializing NUMA node");

                if let Err(_e) = allocator.init_numa_node(numa_node, start_frame, frame_count) {
                    kprintln!("[MM] Warning: Failed to initialize memory region");
                } else {
                    kprintln!("[MM] Memory region initialized");
                }
            }
        }

        drop(allocator); // Release lock before getting stats

        kprintln!("[MM] Memory initialization complete");
    } // End of allocator block
}

/// Translate a kernel virtual address to its physical address by walking
/// the boot page tables (CR3). Returns the physical address, or 0 on failure.
///
/// This is needed to find the kernel's actual physical extent, since UEFI
/// may load the kernel at any physical address.
#[cfg(target_arch = "x86_64")]
fn translate_kernel_vaddr(vaddr: u64) -> u64 {
    let cr3: u64;
    // SAFETY: Reading CR3 is a read-only privileged operation with no side effects;
    // always valid in ring 0.
    unsafe {
        core::arch::asm!("mov {}, cr3", out(reg) cr3, options(nomem, nostack));
    }
    let phys_offset = PHYS_MEM_OFFSET.load(core::sync::atomic::Ordering::Acquire);
    let l4_phys = cr3 & !0xFFF;

    // L4 index
    let l4_idx = ((vaddr >> 39) & 0x1FF) as usize;
    let l4_virt = (l4_phys + phys_offset) as *const u64;
    // SAFETY: l4_virt points into the L4 page table via the identity-mapped
    // physical memory region; index is bounded to 0..511.
    let l4_entry = unsafe { core::ptr::read_volatile(l4_virt.add(l4_idx)) };
    if l4_entry & 1 == 0 {
        return 0;
    }

    // L3 index
    let l3_phys = l4_entry & 0x000F_FFFF_FFFF_F000;
    let l3_idx = ((vaddr >> 30) & 0x1FF) as usize;
    let l3_virt = (l3_phys + phys_offset) as *const u64;
    // SAFETY: l3_virt points into the L3 page table via identity-mapped region;
    // index is bounded to 0..511.
    let l3_entry = unsafe { core::ptr::read_volatile(l3_virt.add(l3_idx)) };
    if l3_entry & 1 == 0 {
        return 0;
    }
    if l3_entry & (1 << 7) != 0 {
        // 1GB huge page
        return (l3_entry & 0x000F_FFFF_C000_0000) | (vaddr & 0x3FFF_FFFF);
    }

    // L2 index
    let l2_phys = l3_entry & 0x000F_FFFF_FFFF_F000;
    let l2_idx = ((vaddr >> 21) & 0x1FF) as usize;
    let l2_virt = (l2_phys + phys_offset) as *const u64;
    // SAFETY: l2_virt points into the L2 page table via identity-mapped region;
    // index is bounded to 0..511.
    let l2_entry = unsafe { core::ptr::read_volatile(l2_virt.add(l2_idx)) };
    if l2_entry & 1 == 0 {
        return 0;
    }
    if l2_entry & (1 << 7) != 0 {
        // 2MB huge page
        return (l2_entry & 0x000F_FFFF_FFE0_0000) | (vaddr & 0x1F_FFFF);
    }

    // L1 index
    let l1_phys = l2_entry & 0x000F_FFFF_FFFF_F000;
    let l1_idx = ((vaddr >> 12) & 0x1FF) as usize;
    let l1_virt = (l1_phys + phys_offset) as *const u64;
    // SAFETY: l1_virt points into the L1 page table via identity-mapped region;
    // index is bounded to 0..511.
    let l1_entry = unsafe { core::ptr::read_volatile(l1_virt.add(l1_idx)) };
    if l1_entry & 1 == 0 {
        return 0;
    }

    (l1_entry & 0x000F_FFFF_FFFF_F000) | (vaddr & 0xFFF)
}

/// Most memory regions handed to `init` (each becomes a NUMA node).
const MAX_BOOT_REGIONS: usize = 8;

/// Guard between the end of the kernel image and the first frame the frame
/// allocator may hand out.
const FRAME_POOL_GUARD: u64 = 0x20_0000;

/// First physical address the frame pool may use after a kernel image that
/// ends at `kernel_end`: rounded up to 2 MiB, plus a 2 MiB guard.
///
/// Deriving this from the linker's `__kernel_end` -- rather than a constant
/// -- is what keeps the pool from overlapping the kernel as it grows. A
/// hardcoded 0x80E00000 on riscv64 ended up 3.3 MiB inside a kernel ending
/// at 0x81148000, so frames aliased the kernel heap and boot stack (N-13).
pub(crate) const fn pool_start_after(kernel_end: u64) -> u64 {
    let aligned = (kernel_end + 0x1F_FFFF) & !0x1F_FFFF;
    aligned + FRAME_POOL_GUARD
}

/// The single region `[pool_start_after(kernel_end), ram_end)`, or `None`
/// if the kernel leaves no room in RAM.
pub(crate) fn region_after_kernel(kernel_end: u64, ram_end: u64) -> Option<MemoryRegion> {
    let start = pool_start_after(kernel_end);
    (ram_end > start).then(|| MemoryRegion {
        start,
        size: ram_end - start,
        usable: true,
    })
}

/// Clip usable `[start, end)` ranges to begin at or above `floor`, align
/// them to whole frames, and keep the largest `out.len()` of them (each
/// becomes a NUMA node, and there are only so many). Returns how many were
/// written; the written regions are sorted by start address.
pub(crate) fn clip_usable_regions(
    ranges: impl IntoIterator<Item = (u64, u64)>,
    floor: u64,
    out: &mut [MemoryRegion],
) -> usize {
    let page = FRAME_SIZE as u64;
    let mut count = 0;
    for (start, end) in ranges {
        let start = (start.max(floor) + page - 1) & !(page - 1);
        let end = end & !(page - 1);
        if end <= start {
            continue;
        }
        let region = MemoryRegion {
            start,
            size: end - start,
            usable: true,
        };
        if count < out.len() {
            out[count] = region;
            count += 1;
        } else if let Some(smallest) = out.iter_mut().min_by_key(|r| r.size) {
            if region.size > smallest.size {
                *smallest = region;
            }
        }
    }
    out[..count].sort_unstable_by_key(|r| r.start);
    count
}

/// Initialize with default memory map for testing
pub fn init_default() {
    kprintln!("[MM] Using default memory map for initialization");

    // Architecture-specific default memory maps
    #[cfg(target_arch = "x86_64")]
    let default_map = {
        // Determine the kernel's physical end address by translating
        // __kernel_end through the boot page tables. UEFI may load the
        // kernel at any physical address, so we cannot hard-code the
        // frame allocator start.
        extern "C" {
            static __kernel_end: u8;
        }
        // SAFETY: __kernel_end is a linker-defined symbol; we only take its address,
        // not dereference it.
        let kernel_end_virt = unsafe { &__kernel_end as *const u8 as u64 };
        let kernel_end_phys = translate_kernel_vaddr(kernel_end_virt);

        // If __kernel_end translation failed, try translating the last byte
        // of HEAP_MEMORY instead. The heap is the largest object in BSS
        // (~512MB) and its end address is a tight lower bound on the kernel's
        // physical extent. __kernel_end may fail translation if the
        // bootloader's page table walk hits an unmapped intermediate entry
        // for the very last page of the BSS.
        let kernel_end_phys = if kernel_end_phys != 0 {
            kernel_end_phys
        } else {
            let heap_end_virt = heap::heap_end_vaddr();
            let heap_end_phys = translate_kernel_vaddr(heap_end_virt);
            if heap_end_phys != 0 {
                kprintln!(
                    "[MM] __kernel_end translation failed, using heap end at phys {:#x}",
                    heap_end_phys
                );
            }
            heap_end_phys
        };

        // Round up to next 2MB boundary for safety margin, then add 2MB
        let alloc_start = if kernel_end_phys != 0 {
            let aligned = (kernel_end_phys + 0x1FFFFF) & !0x1FFFFF; // Round up to 2MB
            let start = aligned + 0x200000; // 2MB safety margin
            kprintln!(
                "[MM] Kernel ends at phys {:#x}, allocator starts at {:#x} ({} MB)",
                kernel_end_phys,
                start,
                start / (1024 * 1024)
            );
            start
        } else {
            // Fallback: compute a safe start from HEAP_SIZE. The kernel's
            // physical footprint is dominated by the BSS (which contains
            // HEAP_MEMORY). Add 64MB for code, rodata, data, stacks, and
            // the bootloader's own allocations.
            let safe_start = (heap::HEAP_SIZE as u64 + 64 * 1024 * 1024 + 0x1FFFFF) & !0x1FFFFF;
            kprintln!(
                "[MM] WARNING: Could not find kernel physical end, using {} MB start (heap={}MB + \
                 64MB margin)",
                safe_start / (1024 * 1024),
                heap::HEAP_SIZE / (1024 * 1024)
            );
            safe_start
        };

        // Usable RAM comes from the bootloader's memory map (UEFI), clipped
        // to start above the kernel. This replaces a hardcoded 2 GiB end
        // that had to match QEMU's `-m` exactly: over-estimating hands out
        // frames that do not exist.
        let mut regions = [MemoryRegion {
            start: 0,
            size: 0,
            usable: false,
        }; MAX_BOOT_REGIONS];
        // SAFETY: BOOT_INFO is written once by the boot entry before
        // kernel_main and is read-only afterwards; this runs during
        // single-threaded early init.
        #[allow(static_mut_refs)]
        let count = unsafe {
            crate::arch::x86_64::boot::BOOT_INFO
                .as_ref()
                .map_or(0, |bi| {
                    clip_usable_regions(
                        bi.memory_regions
                            .iter()
                            .filter(|r| r.kind == bootloader_api::info::MemoryRegionKind::Usable)
                            .map(|r| (r.start, r.end)),
                        alloc_start,
                        &mut regions,
                    )
                })
        };
        if count == 0 {
            panic!("[MM] bootloader reported no usable RAM above the kernel");
        }
        kprintln!(
            "[MM] {} usable region(s) above {:#x} from the bootloader memory map",
            count,
            alloc_start
        );
        (regions, count)
    };

    // AArch64 and RISC-V boot with the MMU off or identity-mapped, so the
    // linker's __kernel_end is the kernel's physical end. The pool starts
    // after it and ends at 128 MiB of RAM -- QEMU `virt`'s default when no
    // `-m` is given. Under-estimating only leaves memory unused; over-
    // estimating would hand out frames that do not exist.
    // TODO(v0.27): read the RAM size from the device tree instead.
    #[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
    let default_map = {
        extern "C" {
            static __kernel_end: u8;
        }
        // Only the linker symbol's address is taken; it is never read.
        let kernel_end = core::ptr::addr_of!(__kernel_end) as u64;
        #[cfg(target_arch = "aarch64")]
        let ram_end: u64 = 0x4000_0000 + 128 * 1024 * 1024;
        #[cfg(target_arch = "riscv64")]
        let ram_end: u64 = 0x8000_0000 + 128 * 1024 * 1024;
        let Some(region) = region_after_kernel(kernel_end, ram_end) else {
            panic!("[MM] kernel image leaves no RAM for the frame allocator");
        };
        let mut regions = [region; MAX_BOOT_REGIONS];
        regions[0] = region;
        (regions, 1)
    };

    kprintln!("[MM] Calling init with default memory map");

    let (regions, count) = default_map;
    init(&regions[..count]);

    kprintln!("[MM] init returned successfully");

    // Initialize heap allocator after frame allocator is ready
    init_heap().expect("Heap initialization failed");
}

/// Walk the boot page tables (CR3) and mark all intermediate table frames
/// as reserved in the frame allocator. This prevents the allocator from
/// handing out frames that the bootloader used for page tables, which would
/// corrupt kernel address space mappings when those frames are overwritten.
///
/// Reserves page table frames for both:
/// - Kernel-space L4 entries (256..512): kernel code, heap, stacks, MMIO
/// - Physical memory mapping L4 entry (lower half): used by phys_to_virt_addr()
///
/// Must be called AFTER init_default() (frame allocator is ready).
#[cfg(target_arch = "x86_64")]
pub fn reserve_boot_page_table_frames() {
    let cr3: u64;
    // SAFETY: Reading CR3 is a read-only privileged operation with no side effects;
    // always valid in ring 0.
    unsafe {
        core::arch::asm!("mov {}, cr3", out(reg) cr3, options(nomem, nostack));
    }
    let l4_phys = cr3 & !0xFFF; // Mask flags

    let phys_offset = PHYS_MEM_OFFSET.load(core::sync::atomic::Ordering::Acquire);
    let mut reserved_count = 0u32;

    // Reserve the L4 table frame itself
    let l4_frame = FrameNumber::new(l4_phys / FRAME_SIZE as u64);
    let _ = FRAME_ALLOCATOR.lock().mark_frame_used(l4_frame);
    reserved_count += 1;

    let l4_virt = (l4_phys + phys_offset) as *const u64;

    // Helper: walk one L4 entry's subtree (L3 → L2 → L1) and reserve all
    // intermediate page table frames.
    let mut reserve_l4_subtree = |l4_idx: usize| {
        // SAFETY: l4_virt points into the L4 page table via identity-mapped region;
        // l4_idx is bounded by callers to valid L4 indices.
        let l4_entry = unsafe { core::ptr::read_volatile(l4_virt.add(l4_idx)) };
        if l4_entry & 1 == 0 {
            return; // Not present
        }
        let l3_phys = l4_entry & 0x000F_FFFF_FFFF_F000;
        let l3_frame = FrameNumber::new(l3_phys / FRAME_SIZE as u64);
        let _ = FRAME_ALLOCATOR.lock().mark_frame_used(l3_frame);
        reserved_count += 1;

        // Walk L3 entries
        let l3_virt = (l3_phys + phys_offset) as *const u64;
        for l3_idx in 0..512 {
            // SAFETY: l3_virt points into the L3 page table via identity-mapped region;
            // l3_idx is bounded to 0..511.
            let l3_entry = unsafe { core::ptr::read_volatile(l3_virt.add(l3_idx)) };
            if l3_entry & 1 == 0 {
                continue;
            }
            if l3_entry & (1 << 7) != 0 {
                continue; // 1GB huge page, no L2 table
            }
            let l2_phys = l3_entry & 0x000F_FFFF_FFFF_F000;
            let l2_frame = FrameNumber::new(l2_phys / FRAME_SIZE as u64);
            let _ = FRAME_ALLOCATOR.lock().mark_frame_used(l2_frame);
            reserved_count += 1;

            // Walk L2 entries
            let l2_virt = (l2_phys + phys_offset) as *const u64;
            for l2_idx in 0..512 {
                // SAFETY: l2_virt points into the L2 page table via identity-mapped region;
                // l2_idx is bounded to 0..511.
                let l2_entry = unsafe { core::ptr::read_volatile(l2_virt.add(l2_idx)) };
                if l2_entry & 1 == 0 {
                    continue;
                }
                if l2_entry & (1 << 7) != 0 {
                    continue; // 2MB huge page, no L1 table
                }
                let l1_phys = l2_entry & 0x000F_FFFF_FFFF_F000;
                let l1_frame = FrameNumber::new(l1_phys / FRAME_SIZE as u64);
                let _ = FRAME_ALLOCATOR.lock().mark_frame_used(l1_frame);
                reserved_count += 1;
            }
        }
    };

    // Reserve kernel-half page table frames (L4 entries 256..512)
    for l4_idx in 256..512 {
        reserve_l4_subtree(l4_idx);
    }

    // Reserve physical memory mapping page table frames.
    // The bootloader maps all physical memory at PHYS_MEM_OFFSET, which
    // occupies one or more L4 entries in the lower half. Without reserving
    // these, the frame allocator can hand out page table frames used by
    // the physical memory mapping, corrupting it when those frames are
    // overwritten (e.g., during fork's clone_from deep copy).
    if phys_offset != 0 {
        let phys_l4_idx = ((phys_offset >> 39) & 0x1FF) as usize;
        if phys_l4_idx < 256 {
            reserve_l4_subtree(phys_l4_idx);
        }
    }

    kprintln!(
        "[MM] Reserved {} boot page table frames from frame allocator",
        reserved_count
    );
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
    if let Err(_e) = FRAME_ALLOCATOR.lock().free_frames(frame_num, 1) {
        kprintln!(
            "[MM] Warning: Failed to free frame at {:#x}: {:?}",
            frame.as_u64(),
            _e
        );
    }
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

/// Page cache frame counter: tracks the number of frames currently held in
/// the page cache (file-backed pages kept in memory for faster re-access).
static PAGE_CACHE_FRAMES: AtomicU64 = AtomicU64::new(0);

/// Increment the page cache frame count when a page is added to the cache.
pub fn page_cache_add(count: u64) {
    PAGE_CACHE_FRAMES.fetch_add(count, Ordering::Relaxed);
}

/// Decrement the page cache frame count when a page is evicted from the cache.
pub fn page_cache_remove(count: u64) {
    PAGE_CACHE_FRAMES.fetch_sub(count, Ordering::Relaxed);
}

/// Memory statistics structure
pub struct MemoryStats {
    pub total_frames: usize,
    pub free_frames: usize,
    pub cached_frames: usize,
}

/// Get memory statistics
pub fn get_memory_stats() -> MemoryStats {
    let allocator = FRAME_ALLOCATOR.lock();
    let stats = allocator.get_stats();

    MemoryStats {
        total_frames: stats.total_frames as usize,
        free_frames: stats.free_frames as usize,
        cached_frames: PAGE_CACHE_FRAMES.load(Ordering::Relaxed) as usize,
    }
}

#[cfg(all(test, not(target_os = "none")))]
mod layout_tests {
    use super::*;

    /// N-13: the riscv64 kernel that ends at 0x81148000 gets a pool that
    /// starts past it, not the old hardcoded 0x80E00000.
    #[test]
    fn pool_starts_after_the_kernel() {
        assert_eq!(pool_start_after(0x8114_8000), 0x8140_0000);
        assert_eq!(pool_start_after(0x8120_0000), 0x8140_0000);
        assert!(pool_start_after(0x8114_8000) > 0x8114_8000);
        let region = region_after_kernel(0x8114_8000, 0x8800_0000).unwrap();
        assert_eq!(region.start + region.size, 0x8800_0000);
        assert!(region_after_kernel(0x87F0_0000, 0x8800_0000).is_none());
    }

    #[test]
    fn clip_drops_low_and_tiny_regions_and_aligns() {
        let mut out = [MemoryRegion {
            start: 0,
            size: 0,
            usable: false,
        }; 4];
        let n = clip_usable_regions(
            [
                (0x1000, 0x9F000),
                (0x10_0000, 0x40_0000),
                (0x40_0800, 0x80_0FFF),
            ],
            0x20_0000,
            &mut out,
        );
        assert_eq!(n, 2);
        assert_eq!(out[0].start, 0x20_0000);
        assert_eq!(out[0].size, 0x20_0000);
        assert_eq!(out[1].start, 0x40_1000);
        assert_eq!(out[1].start + out[1].size, 0x80_0000);
    }

    #[test]
    fn clip_keeps_the_largest_regions() {
        let mut out = [MemoryRegion {
            start: 0,
            size: 0,
            usable: false,
        }; 2];
        let ranges = [
            (0x1_0000_0000u64, 0x1_0010_0000u64),
            (0x2000_0000, 0x4000_0000),
            (0x5000_0000, 0x5001_0000),
            (0x6000_0000, 0x7000_0000),
        ];
        let n = clip_usable_regions(ranges, 0, &mut out);
        assert_eq!(n, 2);
        assert_eq!((out[0].start, out[1].start), (0x2000_0000, 0x6000_0000));
    }
}
