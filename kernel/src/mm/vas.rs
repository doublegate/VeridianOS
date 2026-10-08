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
///
/// A `MAP_SHARED` page (`shared_mapping`) is the exception: its frame is
/// shared on purpose, so a writable request makes it writable for every
/// sharer instead of copy-on-write (N-140).
fn cow_safe_flags(
    mapper: &PageMapper,
    page: VirtualAddress,
    requested: PageFlags,
    shared_mapping: bool,
) -> Option<PageFlags> {
    let (frame, old) = mapper.translate_page(page).ok()?;
    if shared_mapping {
        return Some(requested.without(PageFlags::COW));
    }
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
pub(crate) struct VasFrameAllocator;

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

/// How a write the kernel makes into one address space (ptrace POKE,
/// clone's SETTID stores) may reach a page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrivateWrite {
    /// The frame is this address space's to write (or meant for every
    /// sharer, for a writable shared mapping).
    InPlace,
    /// The frame is shared with another address space: copy it first.
    Copy,
    /// Not writable this way.
    Denied,
}

/// Whether `page` is a user page (kernel pages are mapped in every
/// address space and must never be reached through a user address).
fn is_user_page(page: u64) -> bool {
    use super::user_layout::{USER_SPACE_END, USER_SPACE_START};
    page >= USER_SPACE_START as u64 && page < USER_SPACE_END as u64
}

/// The rule for [`PrivateWrite`], as Linux's `check_vma_flags`: device
/// memory is never written; a shared page only while its mapping is
/// writable, even when forced (a FOLL_FORCE write to a shared mapping
/// without VM_WRITE fails, so a write-sealed or read-only memfd stays
/// unchanged); a private page needs write access or copy-on-write unless
/// forced, and is copied first while another address space shares it.
fn private_write_policy(
    kind: MappingType,
    may_write: bool,
    flags: PageFlags,
    shared_frame: bool,
    force: bool,
) -> PrivateWrite {
    match kind {
        MappingType::Device => PrivateWrite::Denied,
        MappingType::Shared | MappingType::SharedRegion => {
            if may_write && flags.contains(PageFlags::WRITABLE) {
                PrivateWrite::InPlace
            } else {
                PrivateWrite::Denied
            }
        }
        _ => {
            let writable = flags.contains(PageFlags::WRITABLE) || flags.contains(PageFlags::COW);
            if !force && !writable {
                PrivateWrite::Denied
            } else if shared_frame {
                PrivateWrite::Copy
            } else {
                PrivateWrite::InPlace
            }
        }
    }
}

/// Page flags for a user mapping with POSIX protection `prot` (N-132).
///
/// Readable pages are present and user-accessible; `PROT_WRITE` adds
/// writable, and everything not `PROT_EXEC` is no-execute. `PROT_NONE`
/// keeps the page present but supervisor-only, so any user access faults
/// while the frame and its contents stay in place for a later `mprotect`.
pub fn user_prot_flags(prot: usize) -> PageFlags {
    const PROT_WRITE: usize = 0x2;
    const PROT_EXEC: usize = 0x4;
    const PROT_RWX: usize = 0x7;
    let mut f = PageFlags::PRESENT;
    if prot & PROT_RWX != 0 {
        f |= PageFlags::USER;
    }
    if prot & PROT_WRITE != 0 {
        f |= PageFlags::WRITABLE;
    }
    if prot & PROT_EXEC == 0 {
        f |= PageFlags::NO_EXECUTE;
    }
    f
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
    /// Whether mprotect may make this mapping writable (Linux's
    /// VM_MAYWRITE). False for a shared mapping of a file it may never
    /// write -- opened read-only, or a memfd sealed against writes -- so a
    /// read-only mapping cannot be upgraded past the seal.
    pub may_write: bool,
    /// The file this mapping shows, if it is a file mapping: what mremap
    /// grows it with and MADV_DONTNEED restores it from.
    #[cfg(feature = "alloc")]
    pub backing: Option<FileBacking>,
    /// MADV_DONTFORK: a fork child does not get this mapping.
    pub dont_fork: bool,
    /// MADV_WIPEONFORK: a fork child gets zero pages here instead of the
    /// parent's (private anonymous memory only).
    pub wipe_on_fork: bool,
}

/// A file mapping's file, and the file offset of the mapping's first page.
#[cfg(feature = "alloc")]
#[derive(Clone)]
pub struct FileBacking {
    pub node: alloc::sync::Arc<dyn crate::fs::VfsNode>,
    pub offset: usize,
}

#[cfg(feature = "alloc")]
impl core::fmt::Debug for FileBacking {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FileBacking")
            .field("offset", &self.offset)
            .finish_non_exhaustive()
    }
}

impl VirtualMapping {
    /// Create a new virtual mapping
    pub fn new(start: VirtualAddress, size: usize, mapping_type: MappingType) -> Self {
        let flags = match mapping_type {
            MappingType::Code => PageFlags::PRESENT | PageFlags::USER,
            MappingType::Data => {
                PageFlags::PRESENT | PageFlags::WRITABLE | PageFlags::USER | PageFlags::NO_EXECUTE
            }
            MappingType::Stack => {
                PageFlags::PRESENT | PageFlags::WRITABLE | PageFlags::USER | PageFlags::NO_EXECUTE
            }
            MappingType::Heap => {
                PageFlags::PRESENT | PageFlags::WRITABLE | PageFlags::USER | PageFlags::NO_EXECUTE
            }
            MappingType::File => PageFlags::PRESENT | PageFlags::USER,
            MappingType::Shared => {
                PageFlags::PRESENT | PageFlags::WRITABLE | PageFlags::USER | PageFlags::NO_EXECUTE
            }
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
            may_write: true,
            #[cfg(feature = "alloc")]
            backing: None,
            dont_fork: false,
            wipe_on_fork: false,
        }
    }

    /// Pages `[from, to)` of this mapping as a mapping of their own, with
    /// everything else -- type, flags, may_write, file and offset -- kept.
    /// (Splitting rebuilt pieces from the type alone, so a partly unmapped
    /// read-only or write-sealed shared mapping came back writable-capable.)
    #[cfg(feature = "alloc")]
    pub fn piece(&self, from: usize, to: usize) -> Self {
        let mut piece = self.clone();
        piece.start = VirtualAddress(self.start.0 + (from as u64) * 4096);
        piece.size = (to - from) * 4096;
        piece.physical_frames = self
            .physical_frames
            .get(from..to.min(self.physical_frames.len()))
            .map_or_else(Vec::new, <[FrameNumber]>::to_vec);
        if let Some(backing) = piece.backing.as_mut() {
            backing.offset += from * 4096;
        }
        piece
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

    /// Whether RLIMIT_DATA counts this mapping (Linux's is_data_mapping):
    /// private, writable and not the stack.
    pub fn is_data(&self) -> bool {
        self.flags.contains(PageFlags::WRITABLE)
            && !matches!(
                self.mapping_type,
                MappingType::Stack
                    | MappingType::Shared
                    | MappingType::Device
                    | MappingType::SharedRegion
            )
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
    /// (boot) page tables into this VAS's L4; the lower half, user space,
    /// starts empty and is the process's own. This shares the
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
        let result = self.clone_from_inner(other, false);
        if result.is_err() {
            self.discard_partial_clone();
        }
        result
    }

    /// The address space of a CLONE_VM child that is not a thread -- vfork,
    /// posix_spawn (N-210): every page of `other` mapped to the same frame
    /// with the same access, so what the child writes the parent sees (the
    /// parent waits, under CLONE_VFORK, until the child execs or exits).
    /// Pages `other` still shares copy-on-write with a third process are
    /// first made its own, or the child's first write would copy them away
    /// from the parent. Huge pages are copied as by fork. Mappings either
    /// side adds or removes afterwards are its own.
    #[cfg(feature = "alloc")]
    pub fn share_from(&mut self, other: &Self) -> Result<(), KernelError> {
        other.resolve_all_cow()?;
        let result = self.clone_from_inner(other, true);
        if result.is_err() {
            self.discard_partial_clone();
        }
        result
    }

    /// The user pages this address space has frames for (its resident set:
    /// getrusage's ru_maxrss).
    #[cfg(feature = "alloc")]
    pub fn resident_pages(&self) -> usize {
        const KERNEL_SPACE_START: u64 = 0xFFFF_8000_0000_0000;
        self.mappings
            .lock()
            .values()
            .filter(|m| m.start.0 < KERNEL_SPACE_START)
            .map(|m| m.physical_frames.len())
            .sum()
    }

    /// The bytes of `[start, end)` that making writable would turn into
    /// data for RLIMIT_DATA (Linux's mprotect_fixup): in private mappings,
    /// not the stack, that are not writable now.
    #[cfg(feature = "alloc")]
    pub fn data_bytes_gained(&self, start: u64, end: u64) -> u64 {
        self.mappings
            .lock()
            .values()
            .filter(|m| {
                !m.flags.contains(PageFlags::WRITABLE)
                    && !matches!(
                        m.mapping_type,
                        MappingType::Stack
                            | MappingType::Shared
                            | MappingType::Device
                            | MappingType::SharedRegion
                    )
            })
            .map(|m| {
                (m.start.0 + m.size as u64)
                    .min(end)
                    .saturating_sub(m.start.0.max(start))
            })
            .sum()
    }

    /// The size of the user mappings, leaving out whatever of them lies in
    /// `[skip_start, skip_end)` (a MAP_FIXED request replaces that part):
    /// what RLIMIT_AS and RLIMIT_DATA are checked against.
    #[cfg(feature = "alloc")]
    pub fn vm_usage(&self, skip_start: u64, skip_end: u64) -> VmUsage {
        const KERNEL_SPACE_START: u64 = 0xFFFF_8000_0000_0000;
        let mut usage = VmUsage::default();
        for m in self.mappings.lock().values() {
            if m.start.0 >= KERNEL_SPACE_START {
                continue;
            }
            let end = m.start.0 + m.size as u64;
            let overlap = end.min(skip_end).saturating_sub(m.start.0.max(skip_start));
            let bytes = m.size as u64 - overlap;
            usage.total += bytes;
            if m.is_data() {
                usage.data += bytes;
            }
        }
        usage
    }

    /// Resolve every copy-on-write page of the user mappings now, as a
    /// write fault on each would.
    #[cfg(feature = "alloc")]
    fn resolve_all_cow(&self) -> Result<(), KernelError> {
        const KERNEL_SPACE_START: u64 = 0xFFFF_8000_0000_0000;
        let root = self.page_table_root.load(Ordering::Acquire);
        if root == 0 {
            return Ok(());
        }
        // Collected under the mappings lock, resolved after it is dropped:
        // resolve_cow_fault takes it.
        let pages: Vec<u64> = {
            // SAFETY: this address space's L4 table; only read here.
            let mapper = unsafe { create_mapper_from_root(root) };
            let mappings = self.mappings.lock();
            let mut pages = Vec::new();
            for mapping in mappings.values() {
                if mapping.start.0 >= KERNEL_SPACE_START || mapping.flags.contains(PageFlags::HUGE)
                {
                    continue;
                }
                for i in 0..mapping.physical_frames.len() as u64 {
                    let va = mapping.start.0 + i * 4096;
                    if let Ok((_, flags)) = mapper.translate_page(VirtualAddress(va)) {
                        if flags.contains(PageFlags::COW) {
                            pages.push(va);
                        }
                    }
                }
            }
            pages
        };
        for va in pages {
            self.resolve_cow_fault(va)?;
        }
        Ok(())
    }

    /// Undo a failed [`Self::clone_from`]: free the frames it deep-copied
    /// and the user page tables it built, and clear the root.
    ///
    /// Only frames clone_from allocated are freed: borrowed mappings
    /// (device memory, shared regions) and kernel-space entries belong to
    /// the parent. Every lower-half L4 entry is the child's own.
    #[cfg(feature = "alloc")]
    fn discard_partial_clone(&mut self) {
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

        free_user_page_table_frames(root);
    }

    #[cfg(feature = "alloc")]
    fn clone_from_inner(&mut self, other: &Self, share_all: bool) -> Result<(), KernelError> {
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

                // MADV_DONTFORK: not in the child at all. MADV_WIPEONFORK:
                // fresh zero pages. (A CLONE_VM child shares the whole
                // address space, as Linux, so neither applies to it.)
                if mapping.dont_fork && !share_all {
                    continue;
                }
                if mapping.wipe_on_fork && !share_all {
                    let mut child_mapping = mapping.clone();
                    child_mapping.physical_frames = Vec::new();
                    for i in 0..num_pages {
                        let frame = FRAME_ALLOCATOR
                            .lock()
                            .allocate_frames(1, None)
                            .map_err(|_| KernelError::OutOfMemory {
                                requested: 4096,
                                available: 0,
                            });
                        let frame = match frame {
                            Ok(frame) => frame,
                            Err(e) => {
                                // Recorded for teardown.
                                child_mappings.insert(*addr, child_mapping);
                                return Err(e);
                            }
                        };
                        let virt = super::phys_to_virt_addr(frame.as_u64() << 12) as *mut u8;
                        // SAFETY: `frame` was just allocated and is mapped
                        // nowhere; the physical map makes it writable here.
                        unsafe { core::ptr::write_bytes(virt, 0, 4096) };
                        child_mapping.physical_frames.push(frame);
                        let vaddr = VirtualAddress(mapping.start.0 + (i as u64) * 4096);
                        if let Err(e) =
                            child_mapper.map_page(vaddr, frame, mapping.flags, &mut alloc)
                        {
                            child_mappings.insert(*addr, child_mapping);
                            return Err(e);
                        }
                    }
                    child_mappings.insert(*addr, child_mapping);
                    continue;
                }

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

                // MAP_SHARED memory stays shared and writable in both
                // (N-140), and so does everything for a CLONE_VM child
                // (`share_from`): the frames gain an owner but not
                // copy-on-write.
                let keep_writable = share_all || mapping.mapping_type == MappingType::Shared;
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
                    let flags = if flags.contains(PageFlags::WRITABLE) && !keep_writable {
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

    /// Map a region of virtual memory with the default flags of its type.
    #[cfg(feature = "alloc")]
    pub fn map_region(
        &self,
        start: VirtualAddress,
        size: usize,
        mapping_type: MappingType,
    ) -> Result<(), KernelError> {
        self.map_region_flags(start, size, mapping_type, None)
    }

    /// Map a region of virtual memory: allocate zeroed frames and install
    /// them with `flags` (or the type's default flags).
    ///
    /// Every size is checked before anything is allocated, and a failure
    /// part-way through unwinds the page-table entries and frames installed
    /// so far (N-133, N-137): a huge length is an error, not a kernel panic.
    #[cfg(feature = "alloc")]
    pub fn map_region_flags(
        &self,
        start: VirtualAddress,
        size: usize,
        mapping_type: MappingType,
        flags: Option<PageFlags>,
    ) -> Result<(), KernelError> {
        self.map_region_inner(start, size, mapping_type, flags, false)
    }

    /// `MAP_FIXED`: like [`Self::map_region_flags`], but whatever is mapped
    /// in the range is replaced (unmapped under the same lock once the new
    /// frames are ready), as Linux does (N-141).
    #[cfg(feature = "alloc")]
    pub fn map_region_fixed(
        &self,
        start: VirtualAddress,
        size: usize,
        mapping_type: MappingType,
        flags: Option<PageFlags>,
    ) -> Result<(), KernelError> {
        self.map_region_inner(start, size, mapping_type, flags, true)
    }

    /// Map `frames` -- pages another object owns, such as a memfd's (N-230)
    /// -- as one MAP_SHARED mapping, at `at` (replacing what is there, as
    /// MAP_FIXED) or at an address the kernel chooses. The region is set up
    /// as an anonymous shared one would be, and its fresh frames are then
    /// exchanged for `frames`. The caller has given each of `frames` one
    /// more owner (`frame_refs::share`); unmapping releases it, and on
    /// failure the owners not handed to the mapping are released here.
    #[cfg(feature = "alloc")]
    pub fn map_shared_frames(
        &self,
        at: Option<VirtualAddress>,
        frames: &[FrameNumber],
        flags: PageFlags,
        may_write: bool,
    ) -> Result<VirtualAddress, KernelError> {
        let release_from = |i: usize| {
            for &frame in &frames[i..] {
                if super::frame_refs::release(frame) {
                    crate::mm::note_free_failure(
                        FRAME_ALLOCATOR.lock().free_frames(frame, 1),
                        frame,
                        "vas",
                    );
                }
            }
        };
        let size = frames.len() * 4096;
        let placed = match at {
            Some(start) => self
                .map_region_fixed(start, size, MappingType::Shared, Some(flags))
                .map(|_| start),
            None => self.mmap_flags(size, MappingType::Shared, Some(flags)),
        };
        let start = match placed {
            Ok(start) => start,
            Err(e) => {
                release_from(0);
                return Err(e);
            }
        };
        #[cfg(not(test))]
        {
            let root = self.page_table_root.load(Ordering::Acquire);
            // SAFETY: this address space's L4 table; the mappings lock taken
            // below serialises changes to it.
            let mut mapper = unsafe { create_mapper_from_root(root) };
            let mut mappings = self.mappings.lock();
            let Some(mapping) = mappings.get_mut(&start) else {
                release_from(0);
                return Err(KernelError::InvalidAddress {
                    addr: start.0 as usize,
                });
            };
            mapping.may_write = may_write;
            for (i, &frame) in frames.iter().enumerate() {
                let va = VirtualAddress(start.0 + (i as u64) * 4096);
                let page_flags = mapper.translate_page(va).map_or(flags, |(_, f)| f);
                let _ = mapper.unmap_page(va);
                super::tlb::flush_page(va.0);
                if let Err(e) = mapper.map_page(va, frame, page_flags, &mut VasFrameAllocator) {
                    // Frames from `i` on were not handed to the mapping;
                    // the fresh one at `i` stays recorded and is freed with
                    // the mapping.
                    release_from(i);
                    return Err(e);
                }
                if let Some(slot) = mapping.physical_frames.get_mut(i) {
                    let fresh = core::mem::replace(slot, frame);
                    if super::frame_refs::release(fresh) {
                        crate::mm::note_free_failure(
                            FRAME_ALLOCATOR.lock().free_frames(fresh, 1),
                            fresh,
                            "vas",
                        );
                    }
                }
            }
        }
        #[cfg(test)]
        let _ = (release_from, may_write);
        Ok(start)
    }

    /// Put file pages from a page cache (`mm::page_cache`, ADR 0010) into
    /// the mapping that starts at `start`: page `i` of it gets `frames[i]`
    /// in place of the fresh frame it was mapped with, read-only, or
    /// copy-on-write where the mapping may write, so the cached page itself
    /// never changes. `None` keeps the fresh (zero) page. Each frame comes
    /// with an owner for this mapping; on failure, the owners not handed
    /// over are released here.
    #[cfg(feature = "alloc")]
    pub fn install_file_pages(
        &self,
        start: VirtualAddress,
        frames: &[Option<FrameNumber>],
    ) -> Result<(), KernelError> {
        let release_from = |i: usize| {
            for frame in frames[i..].iter().flatten() {
                if super::frame_refs::release(*frame) {
                    crate::mm::note_free_failure(
                        FRAME_ALLOCATOR.lock().free_frames(*frame, 1),
                        *frame,
                        "vas",
                    );
                }
            }
        };
        #[cfg(not(test))]
        {
            let root = self.page_table_root.load(Ordering::Acquire);
            // SAFETY: this address space's L4 table; the mappings lock taken
            // below serialises changes to it.
            let mut mapper = unsafe { create_mapper_from_root(root) };
            let mut mappings = self.mappings.lock();
            // `start` may lie inside the mapping (MADV_DONTNEED restoring
            // part of one); the pages are indexed from its own start.
            let Some(mapping) = mappings
                .range_mut(..=start)
                .next_back()
                .map(|(_, m)| m)
                .filter(|m| m.contains(start))
            else {
                release_from(0);
                return Err(KernelError::InvalidAddress {
                    addr: start.0 as usize,
                });
            };
            // Only a private mapping takes cache pages: swapped into a
            // shared one, they would quietly stop it being shared.
            if matches!(
                mapping.mapping_type,
                MappingType::Shared | MappingType::Device | MappingType::SharedRegion
            ) {
                release_from(0);
                return Err(KernelError::InvalidAddress {
                    addr: start.0 as usize,
                });
            }
            let first = ((start.0 - mapping.start.0) / 4096) as usize;
            for (i, frame) in frames.iter().enumerate() {
                let Some(frame) = *frame else { continue };
                let va = VirtualAddress(start.0 + (i as u64) * 4096);
                let (Ok((_, current)), Some(slot)) = (
                    mapper.translate_page(va),
                    mapping.physical_frames.get_mut(first + i),
                ) else {
                    release_from(i);
                    return Err(KernelError::InvalidAddress {
                        addr: va.0 as usize,
                    });
                };
                // A read-only page keeps its flags: the cache holds an owner
                // of the frame, so it is shared for as long as it is cached,
                // and every later write path -- mprotect (`cow_safe_flags`),
                // the COW fault, kernel writes (`private_frame`) -- copies
                // a shared frame first. COW stays "logically writable".
                let flags = if current.contains(PageFlags::WRITABLE) {
                    current.without(PageFlags::WRITABLE) | PageFlags::COW
                } else {
                    current
                };
                let _ = mapper.unmap_page(va);
                super::tlb::flush_page(va.0);
                if let Err(e) = mapper.map_page(va, frame, flags, &mut VasFrameAllocator) {
                    // The fresh frame at `i` stays recorded and is freed
                    // with the mapping.
                    release_from(i);
                    return Err(e);
                }
                let fresh = core::mem::replace(slot, frame);
                if super::frame_refs::release(fresh) {
                    crate::mm::note_free_failure(
                        FRAME_ALLOCATOR.lock().free_frames(fresh, 1),
                        fresh,
                        "vas",
                    );
                }
            }
        }
        #[cfg(test)]
        {
            let _ = start;
            release_from(frames.len());
        }
        Ok(())
    }

    #[cfg(feature = "alloc")]
    fn map_region_inner(
        &self,
        start: VirtualAddress,
        size: usize,
        mapping_type: MappingType,
        flags: Option<PageFlags>,
        replace: bool,
    ) -> Result<(), KernelError> {
        let too_big = KernelError::OutOfMemory {
            requested: size,
            available: 0,
        };
        // Align to page boundary
        let aligned_start = VirtualAddress(start.0 & !(4096 - 1));
        let aligned_size = size.checked_add(4095).map(|s| s & !4095).ok_or(too_big)?;
        if aligned_size == 0 || aligned_start.0.checked_add(aligned_size as u64).is_none() {
            return Err(too_big);
        }

        let mut mapping = VirtualMapping::new(aligned_start, aligned_size, mapping_type);
        if let Some(f) = flags {
            mapping.flags = f;
        }

        let mut mappings = self.mappings.lock();

        let aligned_end = aligned_start.0 + aligned_size as u64;
        // A replacing map may not cut a huge page; refuse before anything
        // is allocated or unmapped.
        if replace
            && mappings
                .range(..VirtualAddress(aligned_end))
                .rev()
                .take_while(|(_, m)| m.end().0 > aligned_start.0)
                .any(|(_, m)| {
                    m.flags.contains(PageFlags::HUGE)
                        && (aligned_start.0 > m.start.0 || aligned_end < m.end().0)
                })
        {
            return Err(KernelError::InvalidArgument {
                name: "MAP_FIXED range",
                value: "part of a huge-page mapping",
            });
        }

        // [a_start, a_end) and [b_start, b_end) overlap iff
        // a_start < b_end && b_start < a_end.
        if !replace && Self::overlaps(&mappings, aligned_start.0, aligned_end) {
            return Err(KernelError::AlreadyExists {
                resource: "address range",
                id: aligned_start.0,
            });
        }

        let num_pages = aligned_size / 4096;
        // Fallible: a length the heap cannot index is OutOfMemory, not an
        // allocation-failure abort.
        let mut physical_frames = Vec::new();
        physical_frames
            .try_reserve_exact(num_pages)
            .map_err(|_| too_big)?;

        // Allocate all frames first (FRAME_ALLOCATOR held briefly); on
        // failure free the ones already taken.
        {
            let frame_allocator = FRAME_ALLOCATOR.lock();
            for _ in 0..num_pages {
                match frame_allocator.allocate_frames(1, None) {
                    Ok(frame) => physical_frames.push(frame),
                    Err(_) => {
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
        }

        // POSIX requires anonymous pages to be zero-filled.
        for &frame in &physical_frames {
            let virt = crate::mm::phys_to_virt_addr(frame.as_u64() << 12) as *mut u8;
            // SAFETY: each frame was just allocated and is not mapped
            // anywhere; the physical map makes it accessible to the kernel.
            unsafe { core::ptr::write_bytes(virt, 0, 4096) };
        }

        if replace {
            // Cannot fail: the huge-page case was refused above.
            if let Err(e) = self.unmap_range_locked(&mut mappings, aligned_start.0, aligned_end) {
                let frame_allocator = FRAME_ALLOCATOR.lock();
                for &f in &physical_frames {
                    frame_allocator.free_frames(f, 1).ok();
                }
                return Err(e);
            }
        }

        let pt_root = self.page_table_root.load(Ordering::Acquire);
        if pt_root != 0 {
            // SAFETY: `pt_root` is this address space's L4 table (set by
            // init or inherited), reached through the physical map; the
            // mappings lock serialises changes to it.
            let mut mapper = unsafe { create_mapper_from_root(pt_root) };
            let mut alloc = VasFrameAllocator;
            for (i, &frame) in physical_frames.iter().enumerate() {
                let vaddr = VirtualAddress(aligned_start.0 + (i as u64) * 4096);
                if let Err(e) = mapper.map_page(vaddr, frame, mapping.flags, &mut alloc) {
                    // Unwind: the pages mapped so far were never visible to
                    // any other CPU's TLB as anything else, so removing the
                    // entries and freeing every frame is enough.
                    for j in 0..i {
                        let _ =
                            mapper.unmap_page(VirtualAddress(aligned_start.0 + (j as u64) * 4096));
                    }
                    super::tlb::flush_all();
                    let frame_allocator = FRAME_ALLOCATOR.lock();
                    for &f in &physical_frames {
                        frame_allocator.free_frames(f, 1).ok();
                    }
                    return Err(e);
                }
            }
            // Not-present -> present needs no TLB flush on x86 (no negative
            // caching); RISC-V needs a local fence for the new entries
            // (N-143).
            #[cfg(target_arch = "riscv64")]
            super::tlb::flush_all();
        }

        mapping.physical_frames = physical_frames;
        mappings.insert(aligned_start, mapping);
        Ok(())
    }

    /// Whether any mapping overlaps `[start, end)`.
    #[cfg(feature = "alloc")]
    fn overlaps(mappings: &BTreeMap<VirtualAddress, VirtualMapping>, start: u64, end: u64) -> bool {
        // Mappings do not overlap each other, so only the last one starting
        // before `end` can reach into the range.
        mappings
            .range(..VirtualAddress(end))
            .next_back()
            .is_some_and(|(_, m)| m.start.0 + m.size as u64 > start)
    }

    /// Whether `[start, end)` is free of mappings.
    #[cfg(feature = "alloc")]
    pub fn range_is_free(&self, start: u64, end: u64) -> bool {
        !Self::overlaps(&self.mappings.lock(), start, end)
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
            // Borrowed: protect_region keeps it to its grant.
            may_write: true,
            backing: None,
            dont_fork: false,
            wipe_on_fork: false,
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
            None => VirtualAddress(self.reserve_mmap_range(size, 4096)?.0),
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
                // Borrowed: protect_region keeps it to its grant.
                may_write: true,
                backing: None,
                dont_fork: false,
                wipe_on_fork: false,
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

    /// Unmap `[start_addr, start_addr + size)` rounded out to pages, with
    /// Linux `munmap` semantics (N-141): the range may cover several
    /// mappings, parts of mappings and holes; only the pages actually mapped
    /// are removed, and a range with nothing mapped succeeds. A partly
    /// covered mapping is trimmed or split (GCC's ggc frees single pages of
    /// its pools this way). A 2 MiB page is only unmapped whole.
    #[cfg(feature = "alloc")]
    pub fn unmap(&self, start_addr: usize, size: usize) -> Result<(), KernelError> {
        let bad = KernelError::InvalidArgument {
            name: "munmap range",
            value: "overflows the address space",
        };
        let unmap_start = (start_addr & !(4096 - 1)) as u64;
        let unmap_end = (start_addr as u64)
            .checked_add(size as u64)
            .and_then(|e| e.checked_next_multiple_of(4096))
            .ok_or(bad)?;
        let mut mappings = self.mappings.lock();
        self.unmap_range_locked(&mut mappings, unmap_start, unmap_end)
    }

    /// Remove every mapped page of `[start, end)` (page aligned) from
    /// `mappings` and the page tables. Huge-page mappings must be covered
    /// whole; that is checked before anything changes, so an error leaves
    /// the address space untouched.
    #[cfg(feature = "alloc")]
    fn unmap_range_locked(
        &self,
        mappings: &mut BTreeMap<VirtualAddress, VirtualMapping>,
        start: u64,
        end: u64,
    ) -> Result<(), KernelError> {
        if start >= end {
            return Ok(());
        }
        // Mappings never overlap: walking back from the last one starting
        // below `end`, they overlap the range until one ends at or before
        // `start`.
        let keys: Vec<VirtualAddress> = mappings
            .range(..VirtualAddress(end))
            .rev()
            .take_while(|(_, m)| m.end().0 > start)
            .map(|(k, _)| *k)
            .collect();
        for k in &keys {
            let m = &mappings[k];
            if m.flags.contains(PageFlags::HUGE) && (start > m.start.0 || end < m.end().0) {
                return Err(KernelError::InvalidArgument {
                    name: "munmap range",
                    value: "part of a huge-page mapping",
                });
            }
        }
        for k in keys {
            let Some(mapping) = mappings.remove(&k) else {
                continue;
            };
            let part_start = start.max(k.0);
            let part_end = end.min(mapping.end().0);
            self.unmap_part(mappings, mapping, part_start, part_end);
        }
        Ok(())
    }

    /// Unmap `[unmap_start, unmap_end)` of `mapping` (already removed from
    /// `mappings`, and containing the range), then put back what is left
    /// before and after it.
    #[cfg(feature = "alloc")]
    fn unmap_part(
        &self,
        mappings: &mut BTreeMap<VirtualAddress, VirtualMapping>,
        mapping: VirtualMapping,
        unmap_start: u64,
        unmap_end: u64,
    ) {
        let m_start = mapping.start.0;

        // Calculate page indices within the mapping for the unmap range
        let unmap_page_start = ((unmap_start - m_start) / 4096) as usize;
        let unmap_page_end = ((unmap_end - m_start) / 4096) as usize;

        // Unmap the requested pages from the page table
        let pt_root = self.page_table_root.load(Ordering::Acquire);
        if pt_root != 0 {
            // SAFETY: pt_root is a valid L4 page table address set during
            // VAS::init(); the caller holds the mappings lock, which
            // serialises page-table changes for this space.
            let mut mapper = unsafe { create_mapper_from_root(pt_root) };
            if mapping.flags.contains(PageFlags::HUGE) {
                // Only whole huge mappings get here (checked by the caller).
                unmap_mapping_pages(&mut mapper, &mapping);
            } else {
                for i in unmap_page_start..unmap_page_end {
                    let vaddr = VirtualAddress(m_start + (i as u64) * 4096);
                    let _ = mapper.unmap_page(vaddr);
                }
            }

            // Flush TLB for unmapped pages using batched flushes (without
            // page tables nothing was ever translated).
            let mut tlb_batch = TlbFlushBatch::new();
            for i in unmap_page_start..unmap_page_end {
                let vaddr = m_start + (i as u64) * 4096;
                tlb_batch.add(vaddr);
            }
            tlb_batch.flush();
        }

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

        // What is left before and after the range, each keeping all of
        // the mapping's attributes.
        if unmap_page_start > 0 {
            mappings.insert(mapping.start, mapping.piece(0, unmap_page_start));
        }
        let total_pages = mapping.size / 4096;
        if unmap_page_end < total_pages {
            let back = mapping.piece(unmap_page_end, total_pages);
            mappings.insert(back.start, back);
        }
    }

    /// Split the mapping that contains `addr` (page aligned) strictly
    /// inside it into two at `addr`, so a change can apply to one side.
    /// A huge-page mapping is left whole.
    #[cfg(feature = "alloc")]
    fn split_at_locked(mappings: &mut BTreeMap<VirtualAddress, VirtualMapping>, addr: u64) {
        let Some((&key, m)) = mappings.range(..VirtualAddress(addr)).next_back() else {
            return;
        };
        if m.end().0 <= addr || m.flags.contains(PageFlags::HUGE) {
            return;
        }
        let Some(m) = mappings.remove(&key) else {
            return;
        };
        let at = ((addr - m.start.0) / 4096) as usize;
        let total = m.size / 4096;
        let back = m.piece(at, total);
        mappings.insert(key, m.piece(0, at));
        mappings.insert(back.start, back);
    }

    /// mremap's move (N-246): the pages of `[old, old + len)` -- each page
    /// table entry's frame and flags, copy-on-write state included -- and
    /// their record go to `[new, new + len)`, which must be free. The range
    /// must lie within one mapping, not a huge-page one. On failure nothing
    /// has moved.
    #[cfg(feature = "alloc")]
    pub fn move_range(&self, old: u64, len: u64, new: u64) -> Result<(), KernelError> {
        let refused = KernelError::InvalidArgument {
            name: "mremap range",
            value: "not one movable mapping",
        };
        let mut mappings = self.mappings.lock();
        if Self::overlaps(&mappings, new, new + len) {
            return Err(refused);
        }
        Self::split_at_locked(&mut mappings, old);
        Self::split_at_locked(&mut mappings, old + len);
        let Some(m) = mappings.remove(&VirtualAddress(old)) else {
            return Err(refused);
        };
        if m.size as u64 != len || m.flags.contains(PageFlags::HUGE) {
            mappings.insert(m.start, m);
            return Err(refused);
        }
        let root = self.page_table_root.load(Ordering::Acquire);
        if root != 0 {
            // SAFETY: root is this space's L4 table, reached through the
            // physical map; the mappings lock (held) serialises updates.
            let mut mapper = unsafe { create_mapper_from_root(root) };
            let mut alloc = VasFrameAllocator;
            let pages = len / 4096;
            let mut moved: Vec<(u64, FrameNumber, PageFlags)> = Vec::new();
            for i in 0..pages {
                let from = old + i * 4096;
                let Ok((frame, flags)) = mapper.translate_page(VirtualAddress(from)) else {
                    continue;
                };
                let _ = mapper.unmap_page(VirtualAddress(from));
                if let Err(e) =
                    mapper.map_page(VirtualAddress(new + i * 4096), frame, flags, &mut alloc)
                {
                    // Put back what moved, and this page.
                    let _ = mapper.map_page(VirtualAddress(from), frame, flags, &mut alloc);
                    for &(at, frame, flags) in &moved {
                        let _ = mapper.unmap_page(VirtualAddress(new + (at - old)));
                        let _ = mapper.map_page(VirtualAddress(at), frame, flags, &mut alloc);
                    }
                    crate::mm::tlb::flush_all();
                    mappings.insert(m.start, m);
                    return Err(e);
                }
                moved.push((from, frame, flags));
            }
            let mut batch = TlbFlushBatch::new();
            for &(at, _, _) in &moved {
                batch.add(at);
            }
            batch.flush();
        }
        let mut m = m;
        m.start = VirtualAddress(new);
        mappings.insert(m.start, m);
        Ok(())
    }

    /// Join the mapping starting at `start` with the one just after it when
    /// they differ only in being two records -- same type, flags and write
    /// permission, the same file at consecutive offsets (or none), every
    /// page's frame recorded -- as Linux merges adjacent VMAs. mremap grows
    /// a mapping this way, so the grown range is again one mapping.
    #[cfg(feature = "alloc")]
    pub fn merge_with_next(&self, start: VirtualAddress) {
        let mut mappings = self.mappings.lock();
        let Some(left) = mappings.get(&start) else {
            return;
        };
        let next = left.end();
        let Some(right) = mappings.get(&next) else {
            return;
        };
        let same_file = match (&left.backing, &right.backing) {
            (None, None) => true,
            (Some(a), Some(b)) => {
                core::ptr::eq(
                    alloc::sync::Arc::as_ptr(&a.node) as *const u8,
                    alloc::sync::Arc::as_ptr(&b.node) as *const u8,
                ) && b.offset == a.offset + left.size
            }
            _ => false,
        };
        let joinable = same_file
            && left.mapping_type == right.mapping_type
            && left.flags == right.flags
            && left.may_write == right.may_write
            && left.dont_fork == right.dont_fork
            && left.wipe_on_fork == right.wipe_on_fork
            && !left.flags.contains(PageFlags::HUGE)
            && left.physical_frames.len() == left.size / 4096
            && right.physical_frames.len() == right.size / 4096;
        if !joinable {
            return;
        }
        let Some(right) = mappings.remove(&next) else {
            return;
        };
        if let Some(left) = mappings.get_mut(&start) {
            left.size += right.size;
            left.physical_frames
                .extend_from_slice(&right.physical_frames);
        }
    }

    /// The mappings that overlap `[start, end)`, copied (madvise looks at
    /// them before locking the address space to read files).
    #[cfg(feature = "alloc")]
    pub fn mappings_in(&self, start: u64, end: u64) -> Vec<VirtualMapping> {
        let mappings = self.mappings.lock();
        let mut out: Vec<VirtualMapping> = mappings
            .range(..VirtualAddress(end))
            .rev()
            .take_while(|(_, m)| m.end().0 > start)
            .map(|(_, m)| m.clone())
            .collect();
        out.reverse();
        out
    }

    /// MADV_DONTNEED on private memory (N-243): every page of
    /// `[start, end)` in a private mapping gets a fresh zero frame, mapped
    /// with the mapping's protection, and gives up its old frame (freed if
    /// no copy-on-write sibling or cache still has it). Shared, device and
    /// huge-page mappings are left alone: their contents are not this
    /// mapping's to drop. A file mapping's caller then puts the file's
    /// pages back (`install_file_pages`).
    #[cfg(feature = "alloc")]
    pub fn discard_private_pages(&self, start: u64, end: u64) -> Result<(), KernelError> {
        let root = self.page_table_root.load(Ordering::Acquire);
        if root == 0 {
            return Ok(());
        }
        // SAFETY: root is this space's L4 table, reached through the
        // physical map; the mappings lock (held below) serialises updates.
        let mut mapper = unsafe { create_mapper_from_root(root) };
        let mut mappings = self.mappings.lock();
        let mut batch = TlbFlushBatch::new();
        for (_, m) in mappings.range_mut(..VirtualAddress(end)).rev() {
            if m.end().0 <= start {
                break;
            }
            if matches!(
                m.mapping_type,
                MappingType::Shared | MappingType::Device | MappingType::SharedRegion
            ) || m.flags.contains(PageFlags::HUGE)
            {
                continue;
            }
            let from = start.max(m.start.0);
            let to = end.min(m.end().0);
            for va in (from..to).step_by(4096) {
                let i = ((va - m.start.0) / 4096) as usize;
                let Some(slot) = m.physical_frames.get_mut(i) else {
                    continue;
                };
                let fresh = FRAME_ALLOCATOR
                    .lock()
                    .allocate_frames(1, None)
                    .map_err(|_| KernelError::OutOfMemory {
                        requested: 4096,
                        available: 0,
                    })?;
                let virt = crate::mm::phys_to_virt_addr(fresh.as_u64() << 12) as *mut u8;
                // SAFETY: `fresh` was just allocated and is mapped nowhere;
                // the physical map makes its 4096 bytes writable here.
                unsafe { core::ptr::write_bytes(virt, 0, 4096) };
                let _ = mapper.unmap_page(VirtualAddress(va));
                if let Err(e) =
                    mapper.map_page(VirtualAddress(va), fresh, m.flags, &mut VasFrameAllocator)
                {
                    let _ =
                        mapper.map_page(VirtualAddress(va), *slot, m.flags, &mut VasFrameAllocator);
                    crate::mm::note_free_failure(
                        FRAME_ALLOCATOR.lock().free_frames(fresh, 1),
                        fresh,
                        "vas",
                    );
                    batch.flush();
                    return Err(e);
                }
                batch.add(va);
                let old = core::mem::replace(slot, fresh);
                if super::frame_refs::release(old) {
                    crate::mm::note_free_failure(
                        FRAME_ALLOCATOR.lock().free_frames(old, 1),
                        old,
                        "vas",
                    );
                }
            }
        }
        batch.flush();
        Ok(())
    }

    /// Copy `data` into the pages from `start` (page aligned), each of which
    /// must be a frame this address space alone owns: recorded for it in a
    /// frame-owning mapping and mapped there, with no other owner. File
    /// contents read for fresh pages (mmap, mremap growth, MADV_DONTNEED)
    /// so never reach a page-cache frame, a memfd's page or a copy-on-write
    /// sibling, whatever another thread did to the mappings meanwhile.
    #[cfg(feature = "alloc")]
    pub fn fill_owned_pages(&self, start: u64, data: &[u8]) -> Result<(), KernelError> {
        let root = self.page_table_root.load(Ordering::Acquire);
        if root == 0 {
            return Ok(());
        }
        // SAFETY: root is this space's L4 table, reached through the
        // physical map; the mappings lock (held below) serialises updates.
        let mapper = unsafe { create_mapper_from_root(root) };
        let mappings = self.mappings.lock();
        for (n, chunk) in data.chunks(4096).enumerate() {
            let va = start + (n as u64) * 4096;
            let refused = KernelError::InvalidAddress { addr: va as usize };
            let (_, m) = mappings
                .range(..=VirtualAddress(va))
                .next_back()
                .filter(|(_, m)| m.contains(VirtualAddress(va)))
                .ok_or(refused)?;
            let recorded = m.physical_frames.get(((va - m.start.0) / 4096) as usize);
            let mapped = mapper
                .translate_page(VirtualAddress(va))
                .ok()
                .map(|(f, _)| f);
            let owned = m.owns_frames()
                && recorded.is_some()
                && recorded.copied() == mapped
                && recorded.is_some_and(|&f| !super::frame_refs::is_shared(f));
            if !owned {
                return Err(refused);
            }
            if let Some(&frame) = recorded {
                let virt = crate::mm::phys_to_virt_addr(frame.as_u64() << 12) as *mut u8;
                // SAFETY: `frame` is a RAM frame only this mapping owns
                // (checked above); the physical map makes its 4096 bytes
                // writable, and `chunk` is at most 4096 bytes.
                unsafe { core::ptr::copy_nonoverlapping(chunk.as_ptr(), virt, chunk.len()) };
            }
        }
        Ok(())
    }

    /// MADV_REMOVE (N-243): zero the pages of `[start, end)` that lie in
    /// shared mappings -- a hole punched in the memory every sharer sees.
    #[cfg(feature = "alloc")]
    pub fn zero_shared_pages(&self, start: u64, end: u64) {
        let mappings = self.mappings.lock();
        for (_, m) in mappings.range(..VirtualAddress(end)).rev() {
            if m.end().0 <= start {
                break;
            }
            if m.mapping_type != MappingType::Shared {
                continue;
            }
            let from = start.max(m.start.0);
            let to = end.min(m.end().0);
            for va in (from..to).step_by(4096) {
                if let Some(frame) = m.physical_frames.get(((va - m.start.0) / 4096) as usize) {
                    let virt = crate::mm::phys_to_virt_addr(frame.as_u64() << 12) as *mut u8;
                    // SAFETY: the frame backs a page of this shared
                    // mapping; the physical map makes it writable here.
                    unsafe { core::ptr::write_bytes(virt, 0, 4096) };
                }
            }
        }
    }

    /// MADV_DONTFORK/DOFORK and MADV_WIPEONFORK/KEEPONFORK (N-243): set the
    /// fork behaviour of every mapping in `[start, end)`, split at its ends
    /// as Linux splits VMAs. WIPEONFORK is for private anonymous memory and
    /// DONTFORK not for device memory: anything else in the range makes it
    /// `InvalidArgument` with nothing changed.
    #[cfg(feature = "alloc")]
    pub fn set_fork_behaviour(
        &self,
        start: u64,
        end: u64,
        dont_fork: Option<bool>,
        wipe_on_fork: Option<bool>,
    ) -> Result<(), KernelError> {
        let mut mappings = self.mappings.lock();
        let refused = KernelError::InvalidArgument {
            name: "madvise",
            value: "advice does not apply to this mapping",
        };
        for (_, m) in mappings.range(..VirtualAddress(end)).rev() {
            if m.end().0 <= start {
                break;
            }
            let anonymous_private = m.backing.is_none()
                && !matches!(
                    m.mapping_type,
                    MappingType::Shared | MappingType::Device | MappingType::SharedRegion
                );
            if wipe_on_fork == Some(true) && !anonymous_private
                || dont_fork.is_some() && m.mapping_type == MappingType::Device
            {
                return Err(refused);
            }
        }
        Self::split_at_locked(&mut mappings, start);
        Self::split_at_locked(&mut mappings, end);
        for (_, m) in mappings.range_mut(VirtualAddress(start)..VirtualAddress(end)) {
            if let Some(v) = dont_fork {
                m.dont_fork = v;
            }
            if let Some(v) = wipe_on_fork {
                m.wipe_on_fork = v;
            }
        }
        Ok(())
    }

    /// Whether mprotect may make the mapping starting at `start` writable
    /// (its VM_MAYWRITE), for a mapping made to continue another.
    #[cfg(feature = "alloc")]
    pub fn set_may_write(&self, start: VirtualAddress, may_write: bool) {
        if let Some(m) = self.mappings.lock().get_mut(&start) {
            m.may_write = may_write;
        }
    }

    /// Record the file a mapping starting at `start` shows (mmap of a
    /// file), from file offset `offset`.
    #[cfg(feature = "alloc")]
    pub fn set_backing(
        &self,
        start: VirtualAddress,
        node: alloc::sync::Arc<dyn crate::fs::VfsNode>,
        offset: usize,
    ) {
        if let Some(m) = self.mappings.lock().get_mut(&start) {
            m.backing = Some(FileBacking { node, offset });
        }
    }

    /// The physical address behind `addr` when it lies in a `MAP_SHARED`
    /// mapping (whose frames other processes may map too), else `None`.
    /// Futexes in shared memory are keyed by it (N-114).
    #[cfg(feature = "alloc")]
    pub fn shared_phys_addr(&self, addr: VirtualAddress) -> Option<u64> {
        let mappings = self.mappings.lock();
        let (_, m) = mappings
            .range(..=addr)
            .next_back()
            .filter(|(_, m)| m.contains(addr))?;
        if m.mapping_type != MappingType::Shared {
            return None;
        }
        let frame = m
            .physical_frames
            .get(((addr.0 - m.start.0) / 4096) as usize)?;
        Some((frame.as_u64() << 12) | (addr.0 & 0xfff))
    }

    /// Find mapping for address
    #[cfg(feature = "alloc")]
    pub fn find_mapping(&self, addr: VirtualAddress) -> Option<VirtualMapping> {
        // Mappings never overlap: the only candidate is the last one that
        // starts at or below `addr` (O(log n), was a linear scan; N-143).
        let mappings = self.mappings.lock();
        mappings
            .range(..=addr)
            .next_back()
            .filter(|(_, m)| m.contains(addr))
            .map(|(_, m)| m.clone())
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
            replace_with_copy(&mut mapper, mapping, index, page, frame, flags, writable)?;
            drop(mappings);
            release_after_copy(page, frame);
        } else {
            mapper.update_page_flags(VirtualAddress(page), writable)?;
            drop(mappings);
            super::tlb::flush_page(page);
        }
        Ok(true)
    }

    /// Make the user page at `vaddr` this address space's to write and
    /// return its frame, for writes the kernel makes into one address
    /// space only (ptrace POKE, clone's SETTID stores), which must not
    /// reach other address spaces: a private frame still shared after a
    /// fork -- copy-on-write or read-only -- is replaced by a private copy
    /// with the same access (a copy-on-write page becomes plainly
    /// writable). The page must belong to a user mapping of this address
    /// space; kernel addresses, device memory and shared pages the mapping
    /// may not write now are refused ([`private_write_policy`]). `force`
    /// (ptrace) also reaches read-only and PROT_NONE private pages.
    #[cfg(feature = "alloc")]
    pub fn private_frame(&self, vaddr: u64, force: bool) -> Result<FrameNumber, KernelError> {
        let page = vaddr & !0xFFF;
        let root = self.page_table_root.load(Ordering::Acquire);
        let bad = KernelError::InvalidAddress {
            addr: vaddr as usize,
        };
        if root == 0 || !is_user_page(page) {
            return Err(bad);
        }
        let mut mappings = self.mappings.lock();
        let mapping = mappings
            .values_mut()
            .find(|m| m.contains(VirtualAddress(page)))
            .ok_or(KernelError::InvalidAddress {
                addr: vaddr as usize,
            })?;
        // SAFETY: as in resolve_cow_fault: this address space's L4 table,
        // changed only under the mappings lock held here.
        let mut mapper = unsafe { create_mapper_from_root(root) };
        let (frame, flags) = mapper
            .translate_page(VirtualAddress(page))
            .map_err(|_| bad)?;
        let shared = super::frame_refs::is_shared(frame);
        match private_write_policy(
            mapping.mapping_type,
            mapping.may_write,
            flags,
            shared,
            force,
        ) {
            PrivateWrite::Denied => Err(KernelError::PermissionDenied {
                operation: "write to a page the mapping does not allow",
            }),
            PrivateWrite::InPlace => Ok(frame),
            PrivateWrite::Copy => {
                let index = ((page - mapping.start.0) / 4096) as usize;
                let private = if flags.contains(PageFlags::COW) {
                    flags.without(PageFlags::COW) | PageFlags::WRITABLE
                } else {
                    flags
                };
                let copy =
                    replace_with_copy(&mut mapper, mapping, index, page, frame, flags, private)?;
                drop(mappings);
                release_after_copy(page, frame);
                Ok(copy)
            }
        }
    }

    /// Write `data` at `vaddr` into this address space only (see
    /// [`Self::private_frame`]; `force` as there). Every page must be
    /// mapped and writable this way, or nothing past the failing page is
    /// written.
    #[cfg(feature = "alloc")]
    pub fn write_bytes_private(
        &self,
        vaddr: u64,
        data: &[u8],
        force: bool,
    ) -> Result<(), KernelError> {
        let mut done = 0usize;
        while done < data.len() {
            let at = vaddr
                .checked_add(done as u64)
                .ok_or(KernelError::InvalidAddress {
                    addr: vaddr as usize,
                })?;
            let offset = (at & 0xFFF) as usize;
            let n = (4096 - offset).min(data.len() - done);
            let frame = self.private_frame(at, force)?;
            // SAFETY: `frame` is RAM (device memory is refused) that this
            // address space alone maps, or a shared page its mapping may
            // write, reached through the physical map; `offset + n` stays
            // within the 4 KiB page.
            unsafe {
                core::ptr::copy_nonoverlapping(
                    data[done..].as_ptr(),
                    (super::phys_to_virt_addr(frame.as_u64() << 12) as *mut u8).add(offset),
                    n,
                );
            }
            done += n;
        }
        Ok(())
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
        self.mmap_flags(size, mapping_type, None)
    }

    /// Reserve `size` bytes (rounded up to `align`) at the mmap cursor.
    ///
    /// The cursor only moves forward within user space (N-133): a request
    /// that would leave it is refused without moving it.
    fn reserve_mmap_range(&self, size: usize, align: u64) -> Result<(u64, u64), KernelError> {
        let refused = KernelError::OutOfMemory {
            requested: size,
            available: 0,
        };
        let size = (size as u64)
            .checked_next_multiple_of(align)
            .filter(|&s| s != 0)
            .ok_or(refused)?;
        let mut cur = self.next_mmap_addr.load(Ordering::Acquire);
        loop {
            let base = cur.checked_next_multiple_of(align).ok_or(refused)?;
            let end = base.checked_add(size).ok_or(refused)?;
            if !crate::mm::user_layout::is_user_range(base as usize, size as usize) {
                return Err(refused);
            }
            match self.next_mmap_addr.compare_exchange_weak(
                cur,
                end,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Ok((base, size)),
                Err(now) => cur = now,
            }
        }
    }

    /// Take `size` bytes (page-rounded) of the mmap area without mapping
    /// anything: an address for the caller to map at with
    /// [`Self::map_region_fixed`], as exec does for the program
    /// interpreter (ADR 0010).
    pub fn reserve_mmap_area(&self, size: usize) -> Result<VirtualAddress, KernelError> {
        self.reserve_mmap_range(size, 4096)
            .map(|(base, _)| VirtualAddress(base))
    }

    /// Give back a reservation if nothing was reserved after it.
    fn unreserve_mmap_range(&self, base: u64, size: u64) {
        let _ = self.next_mmap_addr.compare_exchange(
            base + size,
            base,
            Ordering::AcqRel,
            Ordering::Relaxed,
        );
    }

    /// Like [`Self::mmap`], with explicit page flags (from `prot`).
    pub fn mmap_flags(
        &self,
        size: usize,
        mapping_type: MappingType,
        flags: Option<PageFlags>,
    ) -> Result<VirtualAddress, KernelError> {
        // A MAP_FIXED mapping may sit ahead of the cursor; a reservation
        // that lands on it is skipped (the cursor has moved past it) and
        // the next one tried, a bounded number of times.
        for _ in 0..64 {
            let (base, size) = self.reserve_mmap_range(size, 4096)?;
            // Skip physical page mapping in host tests (no frame allocator
            // available)
            #[cfg(all(feature = "alloc", not(test)))]
            match self.map_region_flags(VirtualAddress(base), size as usize, mapping_type, flags) {
                Ok(()) => {}
                Err(KernelError::AlreadyExists { .. }) => continue,
                Err(e) => {
                    self.unreserve_mmap_range(base, size);
                    return Err(e);
                }
            }
            #[cfg(any(not(feature = "alloc"), test))]
            let _ = (mapping_type, flags, size);
            return Ok(VirtualAddress(base));
        }
        Err(KernelError::OutOfMemory {
            requested: size,
            available: 0,
        })
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
            } else if addr.0 < current {
                // Shrink (N-242): the heap pages wholly above the new break
                // go back, as Linux's brk.
                #[cfg(all(feature = "alloc", not(test)))]
                self.brk_shrink_heap(addr.0, current);
                self.heap_break.store(addr.0, Ordering::Release);
            }
        }

        VirtualAddress(self.heap_break.load(Ordering::Acquire))
    }

    /// Lowering the break from `old_break` to `new_break`: unmap the heap
    /// pages in between (whole pages above the new break). Only the heap's
    /// own mapping is touched; a MAP_FIXED mapping placed over part of it
    /// is another mapping and stays.
    #[cfg(all(feature = "alloc", not(test)))]
    fn brk_shrink_heap(&self, new_break: u64, old_break: u64) {
        let new_end = new_break.div_ceil(4096) * 4096;
        let old_end = old_break.div_ceil(4096) * 4096;
        if new_end >= old_end {
            return;
        }
        let mut mappings = self.mappings.lock();
        let keys: Vec<VirtualAddress> = mappings
            .range(..VirtualAddress(old_end))
            .rev()
            .take_while(|(_, m)| m.end().0 > new_end)
            .filter(|(_, m)| m.mapping_type == MappingType::Heap)
            .map(|(k, _)| *k)
            .collect();
        for k in keys {
            if let Some(mapping) = mappings.remove(&k) {
                let start = new_end.max(k.0);
                let end = old_end.min(mapping.end().0);
                self.unmap_part(&mut mappings, mapping, start, end);
            }
        }
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
        let refused = KernelError::OutOfMemory {
            requested: delta_pages.saturating_mul(4096),
            available: 0,
        };
        // The new heap pages must be user space and must not run into
        // another mapping (a MAP_FIXED region above the break): mapping over
        // it failed half-way and leaked every frame (N-137).
        let end = new_page.checked_mul(4096).ok_or(refused)?;
        if !super::user_layout::is_user_range(start_addr.0 as usize, (end - start_addr.0) as usize)
            || !self.range_is_free(start_addr.0, end)
        {
            return Err(refused);
        }

        // Allocate physical frames
        let mut new_frames = Vec::new();
        new_frames
            .try_reserve_exact(delta_pages)
            .map_err(|_| refused)?;
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
            // The heap is data: never executable (N-132).
            let flags =
                PageFlags::PRESENT | PageFlags::WRITABLE | PageFlags::USER | PageFlags::NO_EXECUTE;

            for (i, &frame) in new_frames.iter().enumerate() {
                let vaddr = VirtualAddress(start_addr.0 + (i as u64) * 4096);
                if let Err(e) = mapper.map_page(vaddr, frame, flags, &mut alloc) {
                    for j in 0..i {
                        let _ = mapper.unmap_page(VirtualAddress(start_addr.0 + (j as u64) * 4096));
                    }
                    crate::mm::tlb::flush_all();
                    let frame_allocator = FRAME_ALLOCATOR.lock();
                    for &f in &new_frames {
                        frame_allocator.free_frames(f, 1).ok();
                    }
                    return Err(e);
                }
            }
            #[cfg(target_arch = "riscv64")]
            crate::mm::tlb::flush_all();
        }

        // The heap grows from its last piece -- the mapping ending at the old
        // break, if it is heap memory with the heap's protection and a frame
        // for every page -- or else the new pages are a heap mapping of
        // their own, as Linux merges or adds a brk VMA. (It used to extend
        // whatever was keyed at the heap start: once mprotect had split the
        // heap, that was its first piece, stretched over the others.)
        let rw = PageFlags::PRESENT | PageFlags::WRITABLE | PageFlags::USER | PageFlags::NO_EXECUTE;
        let mut mappings = self.mappings.lock();
        let last = mappings
            .range_mut(..start_addr)
            .next_back()
            .map(|(_, m)| m)
            .filter(|m| {
                m.end() == start_addr
                    && m.mapping_type == MappingType::Heap
                    && m.flags == rw
                    && m.physical_frames.len() == m.size / 4096
                    && !m.dont_fork
                    && !m.wipe_on_fork
            });
        if let Some(mapping) = last {
            mapping.size += delta_pages * 4096;
            mapping.physical_frames.extend_from_slice(&new_frames);
        } else {
            let mut mapping =
                VirtualMapping::new(start_addr, delta_pages * 4096, MappingType::Heap);
            mapping.physical_frames = new_frames;
            mapping.flags = rw;
            mappings.insert(start_addr, mapping);
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
        let unmapped = KernelError::UnmappedMemory {
            addr: start.0 as usize,
        };
        let size = size.checked_add(4095).map(|s| s & !4095).ok_or(unmapped)?;
        let end = start.0.checked_add(size as u64).ok_or(unmapped)?;
        let new_flags = user_prot_flags(prot);

        // The whole range must be mapped (Linux: ENOMEM), and frames this
        // space only borrows -- device memory, IPC shared regions -- may
        // not gain rights they were not granted (N-135).
        let mut mappings = self.mappings.lock();
        let mut addr = start.0;
        while addr < end {
            let Some((_, m)) = mappings
                .range(..=VirtualAddress(addr))
                .next_back()
                .filter(|(_, m)| m.contains(VirtualAddress(addr)))
            else {
                return Err(KernelError::UnmappedMemory {
                    addr: addr as usize,
                });
            };
            // A shared file mapping that may never write (a read-only fd, a
            // write-sealed memfd) stays read-only: mprotect would otherwise
            // write past the seal (Linux: VM_MAYWRITE, EACCES).
            if !m.may_write && new_flags.contains(PageFlags::WRITABLE) {
                return Err(KernelError::PermissionDenied {
                    operation: "mprotect: mapping may not be written",
                });
            }
            if !m.owns_frames() {
                let gains_write = new_flags.contains(PageFlags::WRITABLE)
                    && !m.flags.contains(PageFlags::WRITABLE);
                let gains_exec = !new_flags.contains(PageFlags::NO_EXECUTE);
                if gains_write || gains_exec {
                    return Err(KernelError::PermissionDenied {
                        operation: "mprotect beyond a borrowed mapping's grant",
                    });
                }
            }
            addr = m.end().0.min(end);
        }

        // SAFETY: pt_root is this space's L4 table reached through the
        // physical map; the mappings lock (held) serialises updates.
        let mut mapper = unsafe { create_mapper_from_root(pt_root) };
        let num_pages = size / 4096;
        for i in 0..num_pages {
            let vaddr = VirtualAddress(start.0 + (i as u64) * 4096);
            let shared_mapping = mappings
                .range(..=vaddr)
                .next_back()
                .is_some_and(|(_, m)| m.contains(vaddr) && m.mapping_type == MappingType::Shared);
            if let Some(flags) = cow_safe_flags(&mapper, vaddr, new_flags, shared_mapping) {
                let _ = mapper.update_page_flags(vaddr, flags);
            }
        }
        // One shootdown for the range rather than one per page.
        if num_pages <= 16 {
            let pages: alloc::vec::Vec<u64> =
                (0..num_pages as u64).map(|i| start.0 + i * 4096).collect();
            super::tlb::flush_pages(&pages);
        } else {
            super::tlb::flush_all();
        }

        // Record the protection on the mappings the range covers, split at
        // its ends as Linux splits VMAs, so each mapping's flags describe
        // all of its pages (RLIMIT_DATA counts by them). A huge-page mapping
        // only partly covered stays whole and keeps its old flags.
        Self::split_at_locked(&mut mappings, start.0);
        Self::split_at_locked(&mut mappings, end);
        for (_, m) in mappings.range_mut(..VirtualAddress(end)) {
            if m.start.0 >= start.0 && m.end().0 <= end {
                m.flags = new_flags;
            }
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
                    if let Some(flags) = cow_safe_flags(&mapper, vaddr_obj, flags, false) {
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
        let flags = VirtualMapping::new(VirtualAddress(0), 0, mapping_type).flags;
        self.mmap_huge_flags(size, mapping_type, flags)
    }

    /// Like [`Self::mmap_huge`], with explicit page flags (from `prot`).
    #[cfg(feature = "alloc")]
    pub fn mmap_huge_flags(
        &self,
        size: usize,
        mapping_type: MappingType,
        flags: PageFlags,
    ) -> Result<VirtualAddress, KernelError> {
        let (base, size) = self.reserve_mmap_range(size, HUGE_PAGE_SIZE)?;
        if let Err(e) = self.map_huge_region(base as usize, size as usize, flags, mapping_type) {
            self.unreserve_mmap_range(base, size);
            return Err(e);
        }
        Ok(VirtualAddress(base))
    }
}

/// The bytes of user mappings in an address space, as Linux counts them for
/// its total_vm and data_vm.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct VmUsage {
    /// Every user mapping (RLIMIT_AS).
    pub total: u64,
    /// Private writable mappings other than the stack, the heap included
    /// (RLIMIT_DATA).
    pub data: u64,
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
    let (base, reserved) = memory_space
        .reserve_mmap_range(size, 4096)
        .map_err(|_| crate::syscall::SyscallError::OutOfMemory)?;
    let aligned_size = reserved as usize;
    let vaddr = VirtualAddress(base);

    // Map the physical frames
    #[cfg(feature = "alloc")]
    memory_space
        .map_physical_region(phys_addr, aligned_size, vaddr)
        .map_err(|_| crate::syscall::SyscallError::OutOfMemory)?;

    Ok(vaddr.as_usize())
}

/// Replace the shared `frame` mapped at `page` (slot `index` of `mapping`)
/// with a private copy mapped with `new_flags`; on failure the shared page
/// is put back with `old_flags`. The caller drops the mappings lock and
/// then calls [`release_after_copy`].
#[cfg(feature = "alloc")]
fn replace_with_copy(
    mapper: &mut PageMapper,
    mapping: &mut VirtualMapping,
    index: usize,
    page: u64,
    frame: FrameNumber,
    old_flags: PageFlags,
    new_flags: PageFlags,
) -> Result<FrameNumber, KernelError> {
    let copy = FRAME_ALLOCATOR
        .lock()
        .allocate_frames(1, None)
        .map_err(|_| KernelError::OutOfMemory {
            requested: 4096,
            available: 0,
        })?;
    // SAFETY: both frames are RAM reached through the physical map; `copy`
    // was just allocated and is not mapped anywhere yet.
    unsafe {
        core::ptr::copy_nonoverlapping(
            super::phys_to_virt_addr(frame.as_u64() << 12) as *const u8,
            super::phys_to_virt_addr(copy.as_u64() << 12) as *mut u8,
            4096,
        );
    }
    let _ = mapper.unmap_page(VirtualAddress(page));
    if let Err(e) = mapper.map_page(
        VirtualAddress(page),
        copy,
        new_flags,
        &mut VasFrameAllocator,
    ) {
        // Put the shared page back so the process keeps a valid mapping,
        // and give up the copy.
        let _ = mapper.map_page(
            VirtualAddress(page),
            frame,
            old_flags,
            &mut VasFrameAllocator,
        );
        crate::mm::note_free_failure(FRAME_ALLOCATOR.lock().free_frames(copy, 1), copy, "vas");
        return Err(e);
    }
    if let Some(slot) = mapping.physical_frames.get_mut(index) {
        *slot = copy;
    }
    Ok(copy)
}

/// After [`replace_with_copy`]: flush the old translation, then give up
/// this address space's share of `frame` (freeing it if the other sharer
/// let go meanwhile).
#[cfg(feature = "alloc")]
fn release_after_copy(page: u64, frame: FrameNumber) {
    // Flush before the shared frame can lose its last owner.
    super::tlb::flush_page(page);
    if super::frame_refs::release(frame) {
        crate::mm::note_free_failure(FRAME_ALLOCATOR.lock().free_frames(frame, 1), frame, "vas");
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use super::*;

    /// A mapping record of `pages` pages at `start` with distinct frames.
    fn record(start: u64, pages: usize, kind: MappingType) -> VirtualMapping {
        let mut m = VirtualMapping::new(VirtualAddress(start), pages * 4096, kind);
        m.physical_frames = (0..pages as u64)
            .map(|i| FrameNumber::new(0x1000 + start / 4096 + i))
            .collect();
        m
    }

    /// Splitting keeps every attribute; a partly unmapped read-only shared
    /// mapping used to come back able to be made writable.
    #[test]
    fn pieces_keep_all_attributes() {
        let mut m = record(0x10_0000, 4, MappingType::Shared);
        m.may_write = false;
        m.dont_fork = true;
        m.flags = user_prot_flags(0x1);
        let p = m.piece(1, 3);
        assert_eq!((p.start.0, p.size), (0x10_1000, 0x2000));
        assert_eq!(p.physical_frames, m.physical_frames[1..3].to_vec());
        assert!(!p.may_write && p.dont_fork && p.flags == m.flags);
        assert_eq!(p.mapping_type, MappingType::Shared);

        let mut maps = BTreeMap::new();
        maps.insert(m.start, m.clone());
        VirtualAddressSpace::split_at_locked(&mut maps, 0x10_2000);
        let halves: Vec<_> = maps
            .values()
            .map(|m| (m.start.0, m.size, m.may_write))
            .collect();
        assert_eq!(
            halves,
            vec![(0x10_0000, 0x2000, false), (0x10_2000, 0x2000, false)]
        );
        // At an edge or outside: nothing to split.
        VirtualAddressSpace::split_at_locked(&mut maps, 0x10_2000);
        VirtualAddressSpace::split_at_locked(&mut maps, 0x20_0000);
        assert_eq!(maps.len(), 2);
    }

    /// mremap's record work: a range moves whole, and grown pages join the
    /// mapping again when nothing tells them apart.
    #[test]
    fn move_and_merge_records() {
        let vas = VirtualAddressSpace::new();
        let m = record(0x40_0000, 4, MappingType::Data);
        let frames = m.physical_frames.clone();
        vas.mappings.lock().insert(m.start, m);
        // The middle two pages move; the ends stay where they were.
        vas.move_range(0x40_1000, 0x2000, 0x80_0000).unwrap();
        let moved = vas.find_mapping(VirtualAddress(0x80_0000)).unwrap();
        assert_eq!(moved.size, 0x2000);
        assert_eq!(moved.physical_frames, frames[1..3].to_vec());
        assert!(vas.find_mapping(VirtualAddress(0x40_1000)).is_none());
        assert!(vas.find_mapping(VirtualAddress(0x40_3000)).is_some());
        // Onto a used range: refused, nothing moves.
        assert!(vas.move_range(0x40_0000, 0x1000, 0x80_1000).is_err());
        assert!(vas.find_mapping(VirtualAddress(0x40_0000)).is_some());

        // Adjacent and alike: one mapping again.
        let next = record(0x80_2000, 2, MappingType::Data);
        vas.mappings.lock().insert(next.start, next);
        vas.merge_with_next(VirtualAddress(0x80_0000));
        let merged = vas.find_mapping(VirtualAddress(0x80_3000)).unwrap();
        assert_eq!(
            (merged.start.0, merged.size, merged.physical_frames.len()),
            (0x80_0000, 0x4000, 4)
        );
        // Different protection: kept apart.
        let mut ro = record(0x80_4000, 1, MappingType::Data);
        ro.flags = user_prot_flags(0x1);
        vas.mappings.lock().insert(ro.start, ro);
        vas.merge_with_next(VirtualAddress(0x80_0000));
        assert_eq!(
            vas.find_mapping(VirtualAddress(0x80_0000)).unwrap().size,
            0x4000
        );
    }

    /// WIPEONFORK is for private anonymous memory; the setting splits the
    /// mapping at the range's ends.
    #[test]
    fn fork_behaviour_follows_linux_rules() {
        let vas = VirtualAddressSpace::new();
        let anon = record(0x40_0000, 4, MappingType::Data);
        let shared = record(0x50_0000, 1, MappingType::Shared);
        vas.mappings.lock().insert(anon.start, anon);
        vas.mappings.lock().insert(shared.start, shared);
        assert!(vas
            .set_fork_behaviour(0x50_0000, 0x50_1000, None, Some(true))
            .is_err());
        vas.set_fork_behaviour(0x40_1000, 0x40_2000, None, Some(true))
            .unwrap();
        let flags: Vec<_> = vas
            .mappings
            .lock()
            .values()
            .filter(|m| m.mapping_type == MappingType::Data)
            .map(|m| (m.start.0, m.wipe_on_fork))
            .collect();
        assert_eq!(
            flags,
            vec![(0x40_0000, false), (0x40_1000, true), (0x40_2000, false)]
        );
        vas.set_fork_behaviour(0x50_0000, 0x50_1000, Some(true), None)
            .unwrap();
        assert!(
            vas.find_mapping(VirtualAddress(0x50_0000))
                .unwrap()
                .dont_fork
        );
    }

    /// RLIMIT_DATA's view: write-enabling private memory gains data, shared
    /// memory and the stack never count.
    #[test]
    fn data_accounting_follows_protection() {
        let vas = VirtualAddressSpace::new();
        let mut ro = record(0x40_0000, 2, MappingType::Data);
        ro.flags = user_prot_flags(0x1);
        let rw = record(0x50_0000, 1, MappingType::Data);
        let stack = record(0x60_0000, 1, MappingType::Stack);
        let shared = record(0x70_0000, 1, MappingType::Shared);
        for m in [ro, rw, stack, shared] {
            vas.mappings.lock().insert(m.start, m);
        }
        let usage = vas.vm_usage(0, 0);
        assert_eq!((usage.total, usage.data), (0x5000, 0x1000));
        assert_eq!(vas.data_bytes_gained(0x40_0000, 0x80_0000), 0x2000);
        // A MAP_FIXED replacement does not count what it replaces.
        assert_eq!(vas.vm_usage(0x40_0000, 0x40_1000).total, 0x4000);
    }

    #[test]
    fn kernel_writes_respect_mapping_kind_and_protection() {
        use PrivateWrite::{Copy, Denied, InPlace};
        let rw = user_prot_flags(0x3);
        let ro = user_prot_flags(0x1);
        let none = user_prot_flags(0);
        let cow = ro | PageFlags::COW;
        let policy = |kind, may_write, flags, shared, force| {
            private_write_policy(kind, may_write, flags, shared, force)
        };

        // Device frames are never written through the physical map.
        assert_eq!(policy(MappingType::Device, true, rw, false, true), Denied);
        // Shared pages: only where the mapping is writable now, forced or
        // not (Linux refuses FOLL_FORCE on a shared mapping without
        // VM_WRITE); a write-sealed memfd stays sealed.
        for kind in [MappingType::Shared, MappingType::SharedRegion] {
            assert_eq!(policy(kind, true, rw, true, false), InPlace);
            assert_eq!(policy(kind, true, ro, true, true), Denied);
            assert_eq!(policy(kind, false, rw, true, true), Denied);
        }
        // Private pages: a plain write needs write access or copy-on-write.
        assert_eq!(policy(MappingType::Data, true, rw, false, false), InPlace);
        assert_eq!(policy(MappingType::Data, true, cow, true, false), Copy);
        assert_eq!(policy(MappingType::Code, true, ro, true, false), Denied);
        assert_eq!(policy(MappingType::Data, true, none, false, false), Denied);
        // A forced write (ptrace) reaches read-only and PROT_NONE private
        // pages, into this address space alone.
        assert_eq!(policy(MappingType::Code, true, ro, true, true), Copy);
        assert_eq!(policy(MappingType::Code, true, ro, false, true), InPlace);
        assert_eq!(policy(MappingType::Data, true, none, false, true), InPlace);
    }

    #[test]
    fn kernel_writes_stay_in_the_user_range() {
        assert!(!is_user_page(0));
        assert!(is_user_page(
            crate::mm::user_layout::USER_SPACE_START as u64
        ));
        assert!(!is_user_page(crate::mm::user_layout::USER_SPACE_END as u64));
        assert!(!is_user_page(0xFFFF_8000_0000_0000));
        assert!(!is_user_page(0xFFFF_FFFF_8010_0000));
    }

    #[test]
    fn prot_flags_follow_posix_protection() {
        let none = user_prot_flags(0);
        assert!(none.contains(PageFlags::PRESENT) && !none.contains(PageFlags::USER));
        let r = user_prot_flags(0x1);
        assert!(r.contains(PageFlags::USER) && r.contains(PageFlags::NO_EXECUTE));
        assert!(!r.contains(PageFlags::WRITABLE));
        let rw = user_prot_flags(0x3);
        assert!(rw.contains(PageFlags::WRITABLE) && rw.contains(PageFlags::NO_EXECUTE));
        let rx = user_prot_flags(0x5);
        assert!(!rx.contains(PageFlags::NO_EXECUTE) && !rx.contains(PageFlags::WRITABLE));
        // Default data/shared mappings are no longer executable.
        let data = VirtualMapping::new(VirtualAddress(0x1000), 0x1000, MappingType::Data);
        assert!(data.flags.contains(PageFlags::NO_EXECUTE));
        let shared = VirtualMapping::new(VirtualAddress(0x1000), 0x1000, MappingType::Shared);
        assert!(shared.flags.contains(PageFlags::NO_EXECUTE));
    }

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
    fn test_vas_brk_shrinks() {
        let vas = VirtualAddressSpace::new();

        // Extend the heap first
        let extended = VirtualAddress(0x2000_0001_0000);
        vas.brk(Some(extended));

        // Lowering the break takes effect, as Linux (N-242); it used to be
        // ignored. Not below the heap start, though.
        let lower = VirtualAddress(0x2000_0000_8000);
        assert_eq!(vas.brk(Some(lower)), lower);
        let start = VirtualAddress(0x2000_0000_0000);
        assert_eq!(vas.brk(Some(start)), start);
        assert_eq!(vas.brk(Some(VirtualAddress(0x1000_0000_0000))), start);
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

    /// A borrowed (frame-less to free) mapping for the munmap tests.
    fn device_mapping(start: u64, pages: usize) -> VirtualMapping {
        let mut m = VirtualMapping::new(VirtualAddress(start), pages * 4096, MappingType::Device);
        m.physical_frames = (0..pages as u64)
            .map(|i| FrameNumber::new(0x900 + i))
            .collect();
        m
    }

    fn ranges(vas: &VirtualAddressSpace) -> Vec<(u64, usize)> {
        vas.mappings
            .lock()
            .values()
            .map(|m| (m.start.0, m.size / 4096))
            .collect()
    }

    /// N-141: munmap spans mappings and holes, trims and splits, and a
    /// range with nothing mapped succeeds.
    #[test]
    fn munmap_spans_mappings_and_holes() {
        let vas = VirtualAddressSpace::new();
        {
            let mut m = vas.mappings.lock();
            m.insert(VirtualAddress(0x10000), device_mapping(0x10000, 4));
            m.insert(VirtualAddress(0x20000), device_mapping(0x20000, 2));
            m.insert(VirtualAddress(0x30000), device_mapping(0x30000, 3));
        }
        // From inside the first, across a hole, into the second.
        assert_eq!(vas.unmap(0x12000, 0x21000 - 0x12000), Ok(()));
        assert_eq!(ranges(&vas), vec![(0x10000, 2), (0x21000, 1), (0x30000, 3)]);
        let back = vas.find_mapping(VirtualAddress(0x21000)).unwrap();
        assert_eq!(back.physical_frames, vec![FrameNumber::new(0x901)]);
        // Nothing mapped: success, nothing changes.
        assert_eq!(vas.unmap(0x50000, 0x4000), Ok(()));
        // Hole punch in the middle of the third.
        assert_eq!(vas.unmap(0x31000, 0x1000), Ok(()));
        assert_eq!(
            ranges(&vas),
            vec![(0x10000, 2), (0x21000, 1), (0x30000, 1), (0x32000, 1)]
        );
        // Everything, with an unaligned length rounded up.
        assert_eq!(vas.unmap(0x10000, 0x22001), Ok(()));
        assert!(ranges(&vas).is_empty());
    }

    /// A huge-page mapping is only unmapped whole, and a refused request
    /// changes nothing.
    #[test]
    fn munmap_refuses_part_of_a_huge_page_atomically() {
        let vas = VirtualAddressSpace::new();
        {
            let mut m = vas.mappings.lock();
            m.insert(VirtualAddress(0x10000), device_mapping(0x10000, 1));
            let mut huge = device_mapping(0x20_0000, 512);
            huge.flags |= PageFlags::HUGE;
            m.insert(VirtualAddress(0x20_0000), huge);
        }
        assert!(vas.unmap(0x10000, 0x20_1000).is_err());
        assert_eq!(ranges(&vas), vec![(0x10000, 1), (0x20_0000, 512)]);
        assert_eq!(vas.unmap(0x10000, 0x40_0000 - 0x10000), Ok(()));
        assert!(ranges(&vas).is_empty());
    }
}
