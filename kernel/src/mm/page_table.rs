//! Page table management for virtual memory
//!
//! Provides page table structures and operations for 4-level paging, and
//! for 3-level paging (RISC-V Sv39, where the root is the L3 table; ADR
//! 0018).

// Page table management
#![allow(dead_code)]

use core::{
    marker::PhantomData,
    ops::{Index, IndexMut},
    sync::atomic::{AtomicU64, Ordering},
};

use super::{pte, FrameNumber, PageFlags, PhysicalAddress, VirtualAddress, FRAME_ALLOCATOR};
use crate::error::KernelError;

/// Number of entries in a page table
pub const PAGE_TABLE_ENTRIES: usize = 512;

/// Page table entry
#[derive(Debug, Clone, Copy)]
#[repr(transparent)]
pub struct PageTableEntry {
    entry: u64,
}

impl PageTableEntry {
    /// Create an empty/unused entry
    pub const fn empty() -> Self {
        Self { entry: 0 }
    }

    /// Check if the entry is unused
    pub const fn is_unused(&self) -> bool {
        self.entry == 0
    }

    /// Check if the entry is present
    pub const fn is_present(&self) -> bool {
        pte::native::is_present(self.entry)
    }

    /// Whether this entry, above the last level, is a large-page leaf
    /// rather than a pointer to the next table.
    pub const fn is_huge(&self) -> bool {
        pte::native::is_huge(self.entry)
    }

    /// Get the physical frame this entry points to
    pub fn frame(&self) -> Option<FrameNumber> {
        pte::native::frame(self.entry)
    }

    /// Get the address this entry points to
    pub fn addr(&self) -> Option<PhysicalAddress> {
        self.frame().map(|f| PhysicalAddress::new(f.as_u64() << 12))
    }

    /// Get flags for this entry. Never test `HUGE` in them to tell a leaf
    /// from a table: RISC-V has no such bit (the level is the size), so the
    /// test is always false there. Use [`Self::is_huge`].
    pub const fn flags(&self) -> PageFlags {
        pte::native::flags(self.entry)
    }

    /// Set this entry to map to a frame with given flags (a leaf)
    pub fn set(&mut self, frame: FrameNumber, flags: PageFlags) {
        self.store(pte::native::leaf(frame, flags));
    }

    /// Point this entry at the next-level table in `frame`; `user` when it
    /// is on the path to a user page.
    pub fn set_table(&mut self, frame: FrameNumber, user: bool) {
        self.store(pte::native::table(frame, user));
    }

    /// Let user pages below this table entry be reached (x86_64 checks
    /// USER at every level; RISC-V has nothing to add).
    pub fn grant_user_table(&mut self) {
        self.store(pte::native::grant_user_table(self.entry));
    }

    /// Set this entry to map to an address with given flags
    pub fn set_addr(&mut self, addr: PhysicalAddress, flags: PageFlags) {
        self.set(FrameNumber::new(addr.as_u64() >> 12), flags);
    }

    /// Clear this entry
    pub fn clear(&mut self) {
        self.store(0);
    }

    /// Write the entry with one atomic store, so a lock-free walk
    /// ([`translate_lockless`]) racing it reads the old or the new value and
    /// the two accesses are not a data race (Linux's WRITE_ONCE in set_pte).
    /// Release, paired with the walk's Acquire load: a table zeroed before
    /// it is installed here reads as zeroed to a walk that finds it (Linux's
    /// smp_wmb in pmd_install). Both are plain moves on x86_64.
    fn store(&mut self, value: u64) {
        // SAFETY: `self.entry` is a valid, 8-byte aligned u64 (the entry is
        // repr(transparent) over it), borrowed mutably for the call.
        unsafe { AtomicU64::from_ptr(&mut self.entry) }.store(value, Ordering::Release);
    }

    /// Entry `index` of the table at `table`, read with one atomic load and
    /// no reference to the table (Linux's READ_ONCE in a lockless walk).
    ///
    /// # Safety
    ///
    /// `table` points to a live page table and `index` is below
    /// [`PAGE_TABLE_ENTRIES`].
    unsafe fn load(table: *const PageTable, index: usize) -> Self {
        // SAFETY: per the caller, the word is inside a live table; entries
        // are 8-byte aligned (the table is 4096-aligned), and every write
        // to an entry is atomic (`store`).
        let word = unsafe { AtomicU64::from_ptr(table.cast::<u64>().add(index).cast_mut()) };
        Self {
            entry: word.load(Ordering::Acquire),
        }
    }
}

/// A page table with 512 entries
#[repr(C, align(4096))]
pub struct PageTable {
    entries: [PageTableEntry; PAGE_TABLE_ENTRIES],
}

impl PageTable {
    /// Create a new empty page table
    pub const fn new() -> Self {
        Self {
            entries: [PageTableEntry::empty(); PAGE_TABLE_ENTRIES],
        }
    }

    /// Clear all entries
    pub fn zero(&mut self) {
        for entry in &mut self.entries {
            entry.clear();
        }
    }

    /// Get an iterator over all entries
    pub fn iter(&self) -> impl Iterator<Item = &PageTableEntry> {
        self.entries.iter()
    }

    /// Get a mutable iterator over all entries
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut PageTableEntry> {
        self.entries.iter_mut()
    }
}

impl Default for PageTable {
    fn default() -> Self {
        Self::new()
    }
}

impl Index<usize> for PageTable {
    type Output = PageTableEntry;

    fn index(&self, index: usize) -> &Self::Output {
        &self.entries[index]
    }
}

impl IndexMut<usize> for PageTable {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        &mut self.entries[index]
    }
}

impl Index<PageTableIndex> for PageTable {
    type Output = PageTableEntry;

    fn index(&self, index: PageTableIndex) -> &Self::Output {
        &self.entries[usize::from(index)]
    }
}

impl IndexMut<PageTableIndex> for PageTable {
    fn index_mut(&mut self, index: PageTableIndex) -> &mut Self::Output {
        &mut self.entries[usize::from(index)]
    }
}

/// An index into a page table
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageTableIndex(u16);

impl PageTableIndex {
    /// Create a new index, panics if >= 512
    pub fn new(index: u16) -> Self {
        assert!(index < 512, "page table index out of bounds");
        Self(index)
    }

    /// Create a new index, truncates if >= 512
    pub const fn new_truncate(index: u16) -> Self {
        Self(index & 0x1FF)
    }
}

impl From<PageTableIndex> for usize {
    fn from(index: PageTableIndex) -> Self {
        index.0 as usize
    }
}

impl From<u16> for PageTableIndex {
    fn from(index: u16) -> Self {
        Self::new(index)
    }
}

impl From<usize> for PageTableIndex {
    fn from(index: usize) -> Self {
        assert!(index < 512);
        Self(index as u16)
    }
}

/// A 4-level page table hierarchy
pub struct PageTableHierarchy {
    /// Level 4 (PML4/PGD) table physical address
    pub l4_table: PhysicalAddress,
}

impl PageTableHierarchy {
    /// Create a new page table hierarchy
    pub fn new() -> Result<Self, KernelError> {
        let frame = FRAME_ALLOCATOR
            .lock()
            .allocate_frames(1, None)
            .map_err(|_| KernelError::OutOfMemory {
                requested: 1,
                available: 0,
            })?;
        let l4_addr = PhysicalAddress::new(frame.as_u64() << 12);

        // Zero the L4 table so all entries start as "not present".
        // Without this, garbage data in the frame looks like valid page
        // table entries, causing the mapper to follow bogus pointers.
        // SAFETY: The virtual address is computed via phys_to_virt_addr
        // which adds the bootloader's physical memory mapping offset.
        unsafe {
            let virt = super::phys_to_virt_addr(l4_addr.as_u64());
            crate::mm::page_ops::clear_page(virt as *mut u8);
        }

        Ok(Self { l4_table: l4_addr })
    }

    /// Get the L4 table address
    pub const fn l4_addr(&self) -> PhysicalAddress {
        self.l4_table
    }

    /// Create an empty page table hierarchy for unit tests.
    ///
    /// This avoids calling the frame allocator, which is unavailable
    /// in the host test environment.
    #[cfg(test)]
    pub fn empty_for_test() -> Self {
        Self {
            l4_table: PhysicalAddress::new(0),
        }
    }
}

/// Virtual address breakdown for 4-level paging
#[derive(Debug, Clone, Copy)]
pub struct VirtualAddressBreakdown {
    pub l4_index: PageTableIndex,
    pub l3_index: PageTableIndex,
    pub l2_index: PageTableIndex,
    pub l1_index: PageTableIndex,
    pub page_offset: u16,
}

impl VirtualAddressBreakdown {
    /// Break down a virtual address into page table indices
    pub fn new(addr: VirtualAddress) -> Self {
        let addr = addr.as_u64();
        Self {
            l4_index: PageTableIndex::new_truncate((addr >> 39) as u16),
            l3_index: PageTableIndex::new_truncate((addr >> 30) as u16),
            l2_index: PageTableIndex::new_truncate((addr >> 21) as u16),
            l1_index: PageTableIndex::new_truncate((addr >> 12) as u16),
            page_offset: (addr & 0xFFF) as u16,
        }
    }
}

/// Active page table (architecture-specific)
pub struct ActivePageTable {
    l4_table: PhysicalAddress,
    _phantom: PhantomData<PageTable>,
}

impl ActivePageTable {
    /// Create from the current active page table
    #[cfg(target_arch = "x86_64")]
    pub fn current() -> Self {
        use crate::arch::x86_64::mmu;
        Self {
            l4_table: mmu::read_cr3(),
            _phantom: PhantomData,
        }
    }

    #[cfg(target_arch = "aarch64")]
    pub fn current() -> Self {
        let ttbr0: u64;
        // SAFETY: Reading TTBR0_EL1 (Translation Table Base Register 0) is a
        // read-only operation on a system register that is always accessible at
        // EL1 (kernel mode). It has no side effects beyond returning the current
        // page table base address. The kernel always runs at EL1 when this is
        // called, so the register access is valid.
        unsafe {
            core::arch::asm!("mrs {}, ttbr0_el1", out(reg) ttbr0);
        }
        Self {
            l4_table: PhysicalAddress::new(ttbr0 & 0x0000_FFFF_FFFF_F000),
            _phantom: PhantomData,
        }
    }

    #[cfg(target_arch = "riscv64")]
    pub fn current() -> Self {
        let satp: u64;
        // SAFETY: Reading the SATP (Supervisor Address Translation and Protection)
        // CSR is a read-only operation on a control/status register that is always
        // accessible in supervisor mode. It returns the current page table
        // configuration with no side effects. The kernel runs in S-mode when this
        // is called.
        unsafe {
            core::arch::asm!("csrr {}, satp", out(reg) satp);
        }
        let ppn = satp & 0x0FFF_FFFF_FFFF;
        Self {
            l4_table: PhysicalAddress::new(ppn << 12),
            _phantom: PhantomData,
        }
    }

    /// Switch to this page table
    pub fn make_active(&self) {
        #[cfg(target_arch = "x86_64")]
        {
            use crate::arch::x86_64::mmu;
            mmu::write_cr3(self.l4_table);
        }

        #[cfg(target_arch = "aarch64")]
        {
            // SAFETY: Writing TTBR0_EL1 switches the active page table for EL0/EL1
            // translations. `self.l4_table` must contain a valid physical address of
            // a properly constructed page table hierarchy. The ISB ensures the
            // pipeline is flushed so subsequent instructions use the new translation
            // tables. This is only called from kernel context (EL1) where TTBR0_EL1
            // is writable. The caller is responsible for ensuring the new page table
            // maps all memory the kernel needs to continue executing.
            unsafe {
                core::arch::asm!("msr ttbr0_el1, {}", in(reg) self.l4_table.as_u64());
                core::arch::asm!("isb");
            }
        }

        #[cfg(target_arch = "riscv64")]
        {
            // The mode boot found (Sv39 or Sv48), which every table was
            // built for (ADR 0018).
            let satp =
                (crate::arch::riscv64::boot::satp_mode() << 60) | (self.l4_table.as_u64() >> 12);
            // SAFETY: Writing satp switches the active page table in S-mode.
            // `self.l4_table` must be the physical
            // address of a root table that maps everything the kernel needs to
            // keep executing; the caller guarantees that.
            // sfence.vma: the privileged spec requires it after a satp
            // write so no stale translation from the old table is used.
            unsafe {
                core::arch::asm!("csrw satp, {}", "sfence.vma", in(reg) satp);
            }
        }
    }

    /// Get the physical address of the L4 table
    pub const fn l4_phys(&self) -> PhysicalAddress {
        self.l4_table
    }
}

/// How many levels the hardware walks: 4 (x86_64 4-level paging, AArch64
/// with 48-bit addresses, RISC-V Sv48) or 3 (RISC-V Sv39, ADR 0018).
pub fn paging_levels() -> u8 {
    #[cfg(all(target_arch = "riscv64", target_os = "none"))]
    {
        crate::arch::riscv64::boot::paging_levels()
    }
    #[cfg(not(all(target_arch = "riscv64", target_os = "none")))]
    {
        4
    }
}

/// Page mapper for modifying page tables
pub struct PageMapper {
    /// The root table: an L4 table under 4-level paging, an L3 table (1 GiB
    /// entries) under 3-level.
    l4_table: *mut PageTable,
    /// Recursive mapping index (typically 510 on x86_64)
    recursive_index: Option<PageTableIndex>,
    /// Levels below and including the root: 3 or 4 ([`paging_levels`]).
    levels: u8,
}

impl PageMapper {
    /// Create a new page mapper (unsafe: requires valid mapped L4 table)
    ///
    /// # Safety
    ///
    /// The l4_table pointer must:
    /// - Point to a valid, mapped page table
    /// - Remain valid for the lifetime of the PageMapper
    /// - Not be accessed through any other means while this exists
    pub unsafe fn new(l4_table: *mut PageTable) -> Self {
        Self {
            l4_table,
            recursive_index: None,
            levels: paging_levels(),
        }
    }

    /// A mapper for a root walked with `levels` levels (3 or 4) rather than
    /// the active mode's.
    ///
    /// # Safety
    ///
    /// As for [`Self::new`].
    pub unsafe fn with_levels(l4_table: *mut PageTable, levels: u8) -> Self {
        Self {
            l4_table,
            recursive_index: None,
            levels,
        }
    }

    /// Create a new page mapper with recursive mapping
    ///
    /// # Safety
    ///
    /// Same requirements as `new`, plus:
    /// - The recursive_index must be set up for recursive mapping
    pub unsafe fn new_with_recursive(
        l4_table: *mut PageTable,
        recursive_index: PageTableIndex,
    ) -> Self {
        Self {
            l4_table,
            recursive_index: Some(recursive_index),
            levels: paging_levels(),
        }
    }

    /// The root's entry index for `va`. Entries 256 and up are the kernel
    /// half in every mode (ADR 0018).
    fn root_index(&self, va: u64) -> usize {
        ((va >> (12 + 9 * (u32::from(self.levels) - 1))) & 0x1FF) as usize
    }

    /// Refuse an address the walk cannot translate: the bits above the top
    /// translated one (47 with four levels, 38 with three) must all copy
    /// it, or the root index would alias another range.
    fn check_canonical(&self, page: VirtualAddress) -> Result<(), KernelError> {
        let shift = 64 - (12 + 9 * u32::from(self.levels));
        let va = page.as_u64();
        if (((va << shift) as i64) >> shift) as u64 == va {
            Ok(())
        } else {
            Err(KernelError::InvalidAddress { addr: va as usize })
        }
    }

    /// The table of 1 GiB entries (the L3 table) covering `page`: the root
    /// itself under three levels, else the table under the root's entry.
    /// `grant_user` marks that entry user-accessible. Unmapped if absent.
    fn l3_table(
        &self,
        page: VirtualAddress,
        grant_user: bool,
    ) -> Result<*mut PageTable, KernelError> {
        self.check_canonical(page)?;
        if self.levels == 3 {
            return Ok(self.l4_table);
        }
        // SAFETY: `self.l4_table` was provided by the caller of
        // `PageMapper::new`, who guaranteed it is a valid, mapped page table
        // this mapper has exclusive access to.
        let root = unsafe { &mut *self.l4_table };
        let entry = &mut root[self.root_index(page.as_u64())];
        if !entry.is_present() || entry.is_huge() {
            return Err(KernelError::UnmappedMemory {
                addr: page.as_u64() as usize,
            });
        }
        if grant_user {
            entry.grant_user_table();
        }
        Ok(super::phys_to_virt_addr(l3_phys_from_entry(entry, page)?) as *mut PageTable)
    }

    /// Walk to the L2 entry for `page`, creating the L3 and L2 tables if
    /// needed (with USER on the intermediate entries for a user mapping).
    /// Refuses to descend through a 1 GiB leaf.
    fn l2_entry_for(
        &mut self,
        page: VirtualAddress,
        flags: PageFlags,
        allocator: &mut impl FrameAllocator,
    ) -> Result<*mut PageTableEntry, KernelError> {
        self.check_canonical(page)?;
        let breakdown = VirtualAddressBreakdown::new(page);

        // The upper half belongs to the kernel and its tables are shared by
        // every address space: a user mapping there would mark them USER for
        // all processes (N-133).
        let user = flags.contains(PageFlags::USER);
        if user && self.root_index(page.as_u64()) >= 256 {
            return Err(KernelError::PermissionDenied {
                operation: "user mapping in the kernel half",
            });
        }

        // The path to a user page must allow user access (x86_64 checks
        // USER at every level; on RISC-V table entries carry none).
        let l3_table = if self.levels == 3 {
            self.l4_table
        } else {
            // SAFETY: `self.l4_table` was provided by the caller of
            // `PageMapper::new`, who guaranteed it points to a valid, mapped,
            // 4096-byte-aligned page table that remains valid for the
            // lifetime of this PageMapper, with exclusive access.
            let root = unsafe { &mut *self.l4_table };
            next_table_or_create(&mut root[breakdown.l4_index], user, allocator)?
        };
        // SAFETY: a live table: the root, or one reached through the
        // physical map from a present entry (or freshly allocated, zeroed).
        let l3_entry = unsafe { &mut (&mut *l3_table)[breakdown.l3_index] };
        if l3_entry.is_huge() {
            return Err(KernelError::AlreadyExists {
                resource: "page mapping (1 GiB page)",
                id: page.as_u64(),
            });
        }
        let l2_table = next_table_or_create(l3_entry, user, allocator)?;
        // SAFETY: as above.
        Ok(unsafe { &mut (&mut *l2_table)[breakdown.l2_index] } as *mut PageTableEntry)
    }

    /// Map a 2 MiB page: an L2 leaf (HUGE) for the 512 frames starting at
    /// `frame`. `page` and `frame` must be 2 MiB aligned (MEM-ARCH-01).
    pub fn map_huge_2m(
        &mut self,
        page: VirtualAddress,
        frame: FrameNumber,
        flags: PageFlags,
        allocator: &mut impl FrameAllocator,
    ) -> Result<(), KernelError> {
        if page.as_u64() & 0x1F_FFFF != 0 || frame.as_u64() & 0x1FF != 0 {
            return Err(KernelError::InvalidArgument {
                name: "huge page",
                value: "address or frame not 2 MiB aligned",
            });
        }
        // SAFETY: the pointer is to an entry of a live page table reached
        // through the physical map; this mapper has exclusive access.
        let l2_entry = unsafe { &mut *self.l2_entry_for(page, flags, allocator)? };
        if l2_entry.is_present() {
            return Err(KernelError::AlreadyExists {
                resource: "page mapping",
                id: page.as_u64(),
            });
        }
        l2_entry.set(frame, flags | PageFlags::HUGE | PageFlags::PRESENT);
        Ok(())
    }

    /// Unmap a 2 MiB page installed by [`Self::map_huge_2m`]; returns its
    /// first frame. The caller flushes the TLB on every CPU.
    pub fn unmap_huge_2m(&mut self, page: VirtualAddress) -> Result<FrameNumber, KernelError> {
        let breakdown = VirtualAddressBreakdown::new(page);
        // SAFETY: the root or a present entry's table, through the physical
        // map; exclusive access per PageMapper::new.
        let l3_table = unsafe { &*self.l3_table(page, false)? };
        if l3_table[breakdown.l3_index].is_huge() {
            return Err(KernelError::UnmappedMemory {
                addr: page.as_u64() as usize,
            });
        }
        let l2_phys = l3_phys_from_entry(&l3_table[breakdown.l3_index], page)?;
        // SAFETY: a present L3 entry's table, through the physical map.
        let l2_table = unsafe { &mut *(super::phys_to_virt_addr(l2_phys) as *mut PageTable) };
        let entry = &mut l2_table[breakdown.l2_index];
        if !entry.is_huge() {
            return Err(KernelError::UnmappedMemory {
                addr: page.as_u64() as usize,
            });
        }
        let frame = entry.frame().ok_or(KernelError::UnmappedMemory {
            addr: page.as_u64() as usize,
        })?;
        entry.clear();
        Ok(frame)
    }

    /// Map a page to a frame
    pub fn map_page(
        &mut self,
        page: VirtualAddress,
        frame: FrameNumber,
        flags: PageFlags,
        allocator: &mut impl FrameAllocator,
    ) -> Result<(), KernelError> {
        let breakdown = VirtualAddressBreakdown::new(page);
        let user = flags.contains(PageFlags::USER);
        // SAFETY: the pointer is to an entry of a live page table reached
        // through the physical map; this mapper has exclusive access.
        let l2_entry = unsafe { &mut *self.l2_entry_for(page, flags, allocator)? };
        if l2_entry.is_huge() {
            return Err(KernelError::AlreadyExists {
                resource: "page mapping (2 MiB page)",
                id: page.as_u64(),
            });
        }

        // SAFETY: a table reached through the physical map from a present
        // entry, or freshly allocated and zeroed.
        let l1_table = unsafe { &mut *next_table_or_create(l2_entry, user, allocator)? };

        // Map the page
        let entry = &mut l1_table[breakdown.l1_index];
        if entry.is_present() {
            return Err(KernelError::AlreadyExists {
                resource: "page mapping",
                id: page.as_u64(),
            });
        }
        entry.set(frame, flags | PageFlags::PRESENT);

        Ok(())
    }

    /// Look up the physical frame mapped at a virtual address.
    ///
    /// Walks the page table hierarchy and returns the frame number and flags
    /// of the leaf entry if the page is present at every level. The walk is
    /// [`translate_lockless`]: it makes no reference to any table.
    pub fn translate_page(
        &self,
        page: VirtualAddress,
    ) -> Result<(FrameNumber, PageFlags), KernelError> {
        // SAFETY: `self.l4_table` is a live root (PageMapper::new), and the
        // tables below it stay allocated while this mapper exists.
        unsafe { translate_lockless(self.l4_table, self.levels, page) }
    }

    /// Update the flags on an existing page table entry without changing the
    /// mapped frame.
    ///
    /// Returns the old flags on success.
    pub fn update_page_flags(
        &mut self,
        page: VirtualAddress,
        new_flags: PageFlags,
    ) -> Result<PageFlags, KernelError> {
        let breakdown = VirtualAddressBreakdown::new(page);
        // A leaf that becomes user-accessible needs USER on every level
        // above it too (x86_64 checks all four). The tables may have been
        // created for a kernel-only leaf -- a PROT_NONE mapping -- so mprotect
        // to PROT_READ|PROT_WRITE left user code faulting on a present page
        // (musl's thread stacks: mmap PROT_NONE, then mprotect). Never in
        // the kernel half.
        let grant_user =
            new_flags.contains(PageFlags::USER) && self.root_index(page.as_u64()) < 256;
        let add_user = |entry: &mut PageTableEntry| {
            if grant_user {
                entry.grant_user_table();
            }
        };

        // SAFETY: the root or a present entry's table, through the physical
        // map (same invariants as the other PageMapper methods).
        let l3_table = unsafe { &mut *self.l3_table(page, grant_user)? };
        let l3_entry = &mut l3_table[breakdown.l3_index];
        if !l3_entry.is_present() || l3_entry.is_huge() {
            return Err(KernelError::UnmappedMemory {
                addr: page.as_u64() as usize,
            });
        }

        add_user(l3_entry);
        let l2_phys = l3_phys_from_entry(l3_entry, page)?;
        // SAFETY: Physical address from a present L3 entry, converted to virtual via
        // identity-mapped region.
        let l2_table =
            unsafe { &mut *(super::phys_to_virt_addr(l2_phys as u64) as *mut PageTable) };
        let l2_entry = &mut l2_table[breakdown.l2_index];
        if !l2_entry.is_present() {
            return Err(KernelError::UnmappedMemory {
                addr: page.as_u64() as usize,
            });
        }
        if l2_entry.is_huge() {
            // A 2 MiB leaf has no L1 table below it; treating the data page
            // as one would rewrite user memory.
            return Err(KernelError::InvalidArgument {
                name: "page",
                value: "inside a 2 MiB page",
            });
        }

        add_user(l2_entry);
        let l1_phys = l3_phys_from_entry(l2_entry, page)?;
        // SAFETY: Physical address from a present L2 entry, converted to virtual via
        // identity-mapped region.
        let l1_table =
            unsafe { &mut *(super::phys_to_virt_addr(l1_phys as u64) as *mut PageTable) };
        let entry = &mut l1_table[breakdown.l1_index];

        let frame = entry.frame().ok_or(KernelError::UnmappedMemory {
            addr: page.as_u64() as usize,
        })?;

        let old_flags = entry.flags();
        entry.set(frame, new_flags | PageFlags::PRESENT);
        Ok(old_flags)
    }

    /// Unmap a page
    pub fn unmap_page(&mut self, page: VirtualAddress) -> Result<FrameNumber, KernelError> {
        let breakdown = VirtualAddressBreakdown::new(page);

        // Walk the page table hierarchy
        // SAFETY: the root or a present entry's table, through the physical
        // map; exclusive access per the PageMapper ownership contract.
        let l3_table = unsafe { &mut *self.l3_table(page, false)? };
        let l3_entry = &l3_table[breakdown.l3_index];
        if !l3_entry.is_present() || l3_entry.is_huge() {
            return Err(KernelError::UnmappedMemory {
                addr: page.as_u64() as usize,
            });
        }

        let l2_phys = l3_entry.addr().ok_or(KernelError::InvalidState {
            expected: "L3 entry has address",
            actual: "no address",
        })?;
        // SAFETY: Physical address from a present L3 entry, converted to virtual via
        // identity-mapped region.
        let l2_table =
            unsafe { &mut *(super::phys_to_virt_addr(l2_phys.as_u64()) as *mut PageTable) };
        let l2_entry = &l2_table[breakdown.l2_index];
        if !l2_entry.is_present() {
            return Err(KernelError::UnmappedMemory {
                addr: page.as_u64() as usize,
            });
        }
        if l2_entry.is_huge() {
            // Part of a 2 MiB page: unmapped whole, by unmap_huge_2m.
            return Err(KernelError::InvalidArgument {
                name: "page",
                value: "inside a 2 MiB page",
            });
        }

        let l1_phys = l2_entry.addr().ok_or(KernelError::InvalidState {
            expected: "L2 entry has address",
            actual: "no address",
        })?;
        // SAFETY: Physical address from a present L2 entry, converted to virtual via
        // identity-mapped region.
        let l1_table =
            unsafe { &mut *(super::phys_to_virt_addr(l1_phys.as_u64()) as *mut PageTable) };

        // Unmap the page
        let entry = &mut l1_table[breakdown.l1_index];
        let frame = entry.frame().ok_or(KernelError::UnmappedMemory {
            addr: page.as_u64() as usize,
        })?;
        entry.clear();

        // Flush the TLB entry for the unmapped page to ensure stale translations
        // are not used. Each architecture has its own invalidation instruction
        // (not on the host, where it would fault).
        #[cfg(all(target_arch = "x86_64", target_os = "none"))]
        crate::arch::x86_64::mmu::flush_tlb_address(page.as_u64());

        #[cfg(target_arch = "aarch64")]
        {
            // SAFETY: TLBI invalidates a single TLB entry for the given virtual
            // address. This is a non-destructive, privileged operation.
            unsafe {
                core::arch::asm!(
                    "tlbi vaae1is, {0}",
                    "dsb ish",
                    "isb",
                    in(reg) (page.as_u64() >> 12),
                );
            }
        }

        #[cfg(target_arch = "riscv64")]
        {
            // SAFETY: sfence.vma invalidates TLB entries for the given virtual
            // address. This is a non-destructive, privileged operation.
            unsafe {
                core::arch::asm!(
                    "sfence.vma {0}, zero",
                    in(reg) page.as_u64(),
                );
            }
        }

        Ok(frame)
    }
}

/// The table under `entry`, reached through the physical map. An empty
/// entry first gets a freshly allocated, zeroed table; `user` marks the path
/// user-accessible.
fn next_table_or_create(
    entry: &mut PageTableEntry,
    user: bool,
    allocator: &mut impl FrameAllocator,
) -> Result<*mut PageTable, KernelError> {
    if !entry.is_present() {
        let frame = allocator
            .allocate_frames(1, None)
            .map_err(|_| KernelError::OutOfMemory {
                requested: 1,
                available: 0,
            })?;
        // SAFETY: the frame is freshly allocated and exclusively owned,
        // reached through the physical map.
        unsafe {
            let virt = super::phys_to_virt_addr(frame.as_u64() << 12);
            crate::mm::page_ops::clear_page(virt as *mut u8);
        }
        entry.set_table(frame, user);
    } else if user {
        // Existing entry needs USER bit added for user-space child mapping
        entry.grant_user_table();
    }
    let phys = entry.addr().ok_or(KernelError::InvalidState {
        expected: "page table entry present",
        actual: "not present",
    })?;
    Ok(super::phys_to_virt_addr(phys.as_u64()) as *mut PageTable)
}

/// The frame and flags `page` maps to under the root table `root`, walked
/// with `levels` levels (3 or 4), as [`PageMapper::translate_page`]. Takes
/// no lock and makes no reference to any table: each entry is one atomic
/// load ([`PageTableEntry::load`]), and entries are only written by atomic
/// stores, so a walk racing an unmap or an mprotect of the same address
/// space reads each entry before or after the change, as the hardware
/// walker does (PR #33 review).
///
/// # Safety
///
/// `root` points to a live page table, and every table below it stays
/// allocated for the call. That holds for the active user half, whose
/// table frames are freed only at exec (single-threaded by then) and at
/// teardown (no thread left in it); unmap and mprotect only rewrite
/// entries.
pub unsafe fn translate_lockless(
    root: *const PageTable,
    levels: u8,
    page: VirtualAddress,
) -> Result<(FrameNumber, PageFlags), KernelError> {
    let unmapped = || KernelError::UnmappedMemory {
        addr: page.as_u64() as usize,
    };
    let va = page.as_u64();
    let shift = 64 - (12 + 9 * u32::from(levels));
    if (((va << shift) as i64) >> shift) as u64 != va {
        return Err(KernelError::InvalidAddress { addr: va as usize });
    }
    let table_at = |entry: PageTableEntry| -> Result<*const PageTable, KernelError> {
        let phys = entry.addr().ok_or_else(unmapped)?;
        Ok(super::phys_to_virt_addr(phys.as_u64()) as *const PageTable)
    };
    let breakdown = VirtualAddressBreakdown::new(page);

    let l3_table = if levels == 3 {
        root
    } else {
        let index = ((va >> (12 + 9 * (u32::from(levels) - 1))) & 0x1FF) as usize;
        // SAFETY: `root` is live (the caller) and the index is masked to 9
        // bits.
        let entry = unsafe { PageTableEntry::load(root, index) };
        if !entry.is_present() || entry.is_huge() {
            return Err(unmapped());
        }
        table_at(entry)?
    };
    // SAFETY: a table below the live root, through the physical map; the
    // breakdown indices are 9 bits.
    let l3_entry = unsafe { PageTableEntry::load(l3_table, usize::from(breakdown.l3_index)) };
    if !l3_entry.is_present() {
        return Err(unmapped());
    }
    // A 1 GiB leaf (the kernel's physical map): the 4 KiB frame in it.
    if l3_entry.is_huge() {
        let base = l3_entry.frame().ok_or_else(unmapped)?;
        return Ok((
            FrameNumber::new(base.as_u64() + (va >> 12 & 0x3FFFF)),
            l3_entry.flags() | PageFlags::HUGE,
        ));
    }

    // SAFETY: as above, from a present L3 entry.
    let l2_entry =
        unsafe { PageTableEntry::load(table_at(l3_entry)?, usize::from(breakdown.l2_index)) };
    if !l2_entry.is_present() {
        return Err(unmapped());
    }
    // A 2 MiB leaf: the 4 KiB frame within it (flags keep HUGE).
    if l2_entry.is_huge() {
        let base = l2_entry.frame().ok_or_else(unmapped)?;
        return Ok((
            FrameNumber::new(base.as_u64() + (va >> 12 & 0x1FF)),
            // RISC-V leaves have no HUGE bit: the level is the size.
            l2_entry.flags() | PageFlags::HUGE,
        ));
    }

    // SAFETY: as above, from a present L2 entry.
    let entry =
        unsafe { PageTableEntry::load(table_at(l2_entry)?, usize::from(breakdown.l1_index)) };
    let frame = entry.frame().ok_or_else(unmapped)?;
    Ok((frame, entry.flags()))
}

/// Extract a physical address from a present page table entry.
/// Returns the address as a raw u64 suitable for casting to a pointer.
fn l3_phys_from_entry(entry: &PageTableEntry, page: VirtualAddress) -> Result<u64, KernelError> {
    entry
        .addr()
        .map(|a| a.as_u64())
        .ok_or(KernelError::UnmappedMemory {
            addr: page.as_u64() as usize,
        })
}

/// Frame allocator trait for page mapper
pub trait FrameAllocator {
    /// Allocate frames
    fn allocate_frames(
        &mut self,
        count: usize,
        numa_node: Option<usize>,
    ) -> Result<FrameNumber, super::FrameAllocatorError>;
}

#[cfg(test)]
/// Hands out zeroed, page-aligned tables from the heap. On the host the
/// physical map's offset is 0, so a table's address is its "physical"
/// one and the walker reaches it directly.
pub(crate) struct HeapTables(pub(crate) alloc::vec::Vec<*mut PageTable>);

#[cfg(test)]
impl HeapTables {
    pub(crate) fn table(&mut self) -> *mut PageTable {
        let t = alloc::boxed::Box::into_raw(alloc::boxed::Box::new(PageTable::new()));
        self.0.push(t);
        t
    }
}

#[cfg(test)]
impl FrameAllocator for HeapTables {
    fn allocate_frames(
        &mut self,
        count: usize,
        _numa_node: Option<usize>,
    ) -> Result<FrameNumber, crate::mm::FrameAllocatorError> {
        assert_eq!(count, 1);
        Ok(FrameNumber::new(self.table() as u64 >> 12))
    }
}

#[cfg(test)]
impl Drop for HeapTables {
    fn drop(&mut self) {
        for &t in &self.0 {
            // SAFETY: each pointer came from alloc::boxed::Box::into_raw above and is
            // dropped once.
            drop(unsafe { alloc::boxed::Box::from_raw(t) });
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use super::*;

    fn present_entries(t: *const PageTable) -> Vec<usize> {
        // SAFETY: a live table from HeapTables.
        let t = unsafe { &*t };
        (0..PAGE_TABLE_ENTRIES)
            .filter(|&i| t[i].is_present())
            .collect()
    }

    const USER: PageFlags = PageFlags::PRESENT
        .union(PageFlags::USER)
        .union(PageFlags::WRITABLE);

    /// Sv39 (ADR 0018): the root is indexed by VA[38:30], the walk is three
    /// levels deep, and the kernel/user split is still root entry 256.
    #[test]
    fn three_level_walk_starts_at_the_root() {
        let mut tables = HeapTables(Vec::new());
        let root = tables.table();
        // SAFETY: `root` is a live, zeroed table owned by `tables`.
        let mut m = unsafe { PageMapper::with_levels(root, 3) };
        // The last user page under Sv39: root entry 255.
        let va = VirtualAddress(0x3F_FFFF_E000);
        m.map_page(va, FrameNumber::new(0x1234), USER, &mut tables)
            .unwrap();
        assert_eq!(present_entries(root), [255]);
        // Root, L2, L1: two tables below the root.
        assert_eq!(tables.0.len(), 3);
        let (frame, flags) = m.translate_page(va).unwrap();
        assert_eq!(frame, FrameNumber::new(0x1234));
        assert!(flags.contains(PageFlags::USER));
        m.update_page_flags(va, PageFlags::PRESENT | PageFlags::USER)
            .unwrap();
        assert!(!m
            .translate_page(va)
            .unwrap()
            .1
            .contains(PageFlags::WRITABLE));
        assert_eq!(m.unmap_page(va).unwrap(), FrameNumber::new(0x1234));
        assert!(m.translate_page(va).is_err());

        // The kernel half is root entries 256 and up: no user mapping there.
        let kernel = VirtualAddress(0xFFFF_FFC0_0000_0000);
        assert!(m
            .map_page(kernel, FrameNumber::new(1), USER, &mut tables)
            .is_err());
        // A kernel mapping goes to root entry 256.
        m.map_page(
            kernel,
            FrameNumber::new(1),
            PageFlags::PRESENT | PageFlags::WRITABLE,
            &mut tables,
        )
        .unwrap();
        assert_eq!(present_entries(root), [255, 256]);
        // 256 GiB is not an Sv39 address (bits 63..39 must copy bit 38): it
        // would alias root entry 256 or 0.
        for bad in [0x40_0000_0000u64, 0x80_0000_0000, 0xFFFF_FF80_0000_0000] {
            let bad = VirtualAddress(bad);
            assert!(m
                .map_page(bad, FrameNumber::new(2), USER, &mut tables)
                .is_err());
            assert!(m.translate_page(bad).is_err());
            assert!(m.unmap_page(bad).is_err());
        }
        assert_eq!(present_entries(root), [255, 256]);
    }

    /// The lock-free walk reads what the mapper wrote, through the root
    /// alone, and sees each later change to an entry (PR #33 review).
    #[test]
    fn lockless_walk_reads_the_entries_the_mapper_writes() {
        let mut tables = HeapTables(Vec::new());
        let root = tables.table();
        // SAFETY: `root` is a live, zeroed table owned by `tables`.
        let mut m = unsafe { PageMapper::with_levels(root, 4) };
        let va = VirtualAddress(0x7FFF_0000_0000);
        // SAFETY (each call below): `root` and the tables under it are live
        // in `tables` for the whole test.
        assert!(unsafe { translate_lockless(root, 4, va) }.is_err());
        m.map_page(va, FrameNumber::new(0x42), USER, &mut tables)
            .unwrap();
        let (frame, flags) = unsafe { translate_lockless(root, 4, va) }.unwrap();
        assert_eq!(frame, FrameNumber::new(0x42));
        assert!(flags.contains(PageFlags::WRITABLE));
        m.update_page_flags(va, PageFlags::PRESENT | PageFlags::USER)
            .unwrap();
        let (_, flags) = unsafe { translate_lockless(root, 4, va) }.unwrap();
        assert!(!flags.contains(PageFlags::WRITABLE));
        m.unmap_page(va).unwrap();
        assert!(unsafe { translate_lockless(root, 4, va) }.is_err());
        // Not canonical: refused, as by the mapper.
        assert!(unsafe { translate_lockless(root, 4, VirtualAddress(0x8000_0000_0000)) }.is_err());
    }

    /// The same page under four levels goes through root entry 0, and
    /// 256 GiB is an ordinary user address there.
    #[test]
    fn four_level_walk_goes_through_root_entry_0() {
        let mut tables = HeapTables(Vec::new());
        let root = tables.table();
        // SAFETY: `root` is a live, zeroed table owned by `tables`.
        let mut m = unsafe { PageMapper::with_levels(root, 4) };
        let va = VirtualAddress(0x3F_FFFF_E000);
        m.map_page(va, FrameNumber::new(0x1234), USER, &mut tables)
            .unwrap();
        assert_eq!(present_entries(root), [0]);
        assert_eq!(tables.0.len(), 4);
        assert_eq!(m.translate_page(va).unwrap().0, FrameNumber::new(0x1234));
        let high = VirtualAddress(0x40_0000_0000);
        m.map_page(high, FrameNumber::new(5), USER, &mut tables)
            .unwrap();
        assert_eq!(m.translate_page(high).unwrap().0, FrameNumber::new(5));
        // Not canonical under 4 levels (bits 63..48 must copy bit 47).
        assert!(m
            .map_page(
                VirtualAddress(0x8000_0000_0000),
                FrameNumber::new(6),
                USER,
                &mut tables
            )
            .is_err());
        assert_eq!(present_entries(root), [0]);
    }

    #[test]
    fn entry_flags_keep_no_execute_and_cow() {
        let mut e = PageTableEntry::empty();
        let flags = PageFlags::PRESENT | PageFlags::USER | PageFlags::COW | PageFlags::NO_EXECUTE;
        e.set(FrameNumber::new(0x1234), flags);
        let got = e.flags();
        assert!(
            got.contains(PageFlags::NO_EXECUTE),
            "NX must survive a read-back"
        );
        assert!(got.contains(PageFlags::COW));
        assert!(!got.contains(PageFlags::WRITABLE));
        assert_eq!(e.frame(), Some(FrameNumber::new(0x1234)));
        assert!(!got.without(PageFlags::COW).contains(PageFlags::COW));
    }
}
