//! Page table management for virtual memory
//!
//! Provides page table structures and operations for 4-level paging

#![allow(dead_code)]

use core::{
    marker::PhantomData,
    ops::{Index, IndexMut},
};

use super::{FrameNumber, PageFlags, PhysicalAddress, VirtualAddress, FRAME_ALLOCATOR};

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
        self.entry & PageFlags::PRESENT.0 != 0
    }

    /// Get the physical frame this entry points to
    pub fn frame(&self) -> Option<FrameNumber> {
        if self.is_present() {
            Some(FrameNumber::new((self.entry & 0x000FFFFF_FFFFF000) >> 12))
        } else {
            None
        }
    }

    /// Get the address this entry points to
    pub fn addr(&self) -> Option<PhysicalAddress> {
        self.frame().map(|f| PhysicalAddress::new(f.as_u64() << 12))
    }

    /// Get flags for this entry
    pub const fn flags(&self) -> PageFlags {
        PageFlags(self.entry & 0xFFF)
    }

    /// Set this entry to map to a frame with given flags
    pub fn set(&mut self, frame: FrameNumber, flags: PageFlags) {
        self.entry = (frame.as_u64() << 12) | flags.0;
    }

    /// Set this entry to map to an address with given flags
    pub fn set_addr(&mut self, addr: PhysicalAddress, flags: PageFlags) {
        self.set(FrameNumber::new(addr.as_u64() >> 12), flags);
    }

    /// Clear this entry
    pub fn clear(&mut self) {
        self.entry = 0;
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
    pub fn new() -> Result<Self, &'static str> {
        let frame = FRAME_ALLOCATOR
            .lock()
            .allocate_frames(1, None)
            .map_err(|_| "Failed to allocate L4 page table")?;

        let l4_addr = PhysicalAddress::new(frame.as_u64() << 12);

        // Zero the table (in real implementation, would map and clear)
        // For now, assume it's zeroed

        Ok(Self { l4_table: l4_addr })
    }

    /// Get the L4 table address
    pub const fn l4_addr(&self) -> PhysicalAddress {
        self.l4_table
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
            unsafe {
                core::arch::asm!("msr ttbr0_el1, {}", in(reg) self.l4_table.as_u64());
                core::arch::asm!("isb");
            }
        }

        #[cfg(target_arch = "riscv64")]
        {
            let satp = (8 << 60) | (self.l4_table.as_u64() >> 12); // Mode 8 = Sv48
            unsafe {
                core::arch::asm!("csrw satp, {}", in(reg) satp);
            }
        }
    }

    /// Get the physical address of the L4 table
    pub const fn l4_phys(&self) -> PhysicalAddress {
        self.l4_table
    }
}

/// Page mapper for modifying page tables
pub struct PageMapper {
    l4_table: *mut PageTable,
    /// Recursive mapping index (typically 510 on x86_64)
    recursive_index: Option<PageTableIndex>,
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
        }
    }

    /// Map a page to a frame
    pub fn map_page(
        &mut self,
        page: VirtualAddress,
        frame: FrameNumber,
        flags: PageFlags,
        allocator: &mut impl FrameAllocator,
    ) -> Result<(), &'static str> {
        let breakdown = VirtualAddressBreakdown::new(page);

        // Get L4 table
        let l4_table = unsafe { &mut *self.l4_table };
        let l4_entry = &mut l4_table[breakdown.l4_index];

        // Get or create L3 table
        if !l4_entry.is_present() {
            let frame = allocator
                .allocate_frames(1, None)
                .map_err(|_| "Failed to allocate L3 table")?;
            l4_entry.set(frame, PageFlags::PRESENT | PageFlags::WRITABLE);
        }
        let l3_phys = l4_entry.addr().unwrap();
        let l3_table = unsafe { &mut *(l3_phys.as_u64() as *mut PageTable) };
        let l3_entry = &mut l3_table[breakdown.l3_index];

        // Get or create L2 table
        if !l3_entry.is_present() {
            let frame = allocator
                .allocate_frames(1, None)
                .map_err(|_| "Failed to allocate L2 table")?;
            l3_entry.set(frame, PageFlags::PRESENT | PageFlags::WRITABLE);
        }
        let l2_phys = l3_entry.addr().unwrap();
        let l2_table = unsafe { &mut *(l2_phys.as_u64() as *mut PageTable) };
        let l2_entry = &mut l2_table[breakdown.l2_index];

        // Get or create L1 table
        if !l2_entry.is_present() {
            let frame = allocator
                .allocate_frames(1, None)
                .map_err(|_| "Failed to allocate L1 table")?;
            l2_entry.set(frame, PageFlags::PRESENT | PageFlags::WRITABLE);
        }
        let l1_phys = l2_entry.addr().unwrap();
        let l1_table = unsafe { &mut *(l1_phys.as_u64() as *mut PageTable) };

        // Map the page
        let entry = &mut l1_table[breakdown.l1_index];
        if entry.is_present() {
            return Err("Page already mapped");
        }
        entry.set(frame, flags | PageFlags::PRESENT);

        Ok(())
    }

    /// Unmap a page
    pub fn unmap_page(&mut self, page: VirtualAddress) -> Result<FrameNumber, &'static str> {
        let breakdown = VirtualAddressBreakdown::new(page);

        // Walk the page table hierarchy
        let l4_table = unsafe { &mut *self.l4_table };
        let l4_entry = &l4_table[breakdown.l4_index];
        if !l4_entry.is_present() {
            return Err("L4 entry not present");
        }

        let l3_phys = l4_entry.addr().unwrap();
        let l3_table = unsafe { &mut *(l3_phys.as_u64() as *mut PageTable) };
        let l3_entry = &l3_table[breakdown.l3_index];
        if !l3_entry.is_present() {
            return Err("L3 entry not present");
        }

        let l2_phys = l3_entry.addr().unwrap();
        let l2_table = unsafe { &mut *(l2_phys.as_u64() as *mut PageTable) };
        let l2_entry = &l2_table[breakdown.l2_index];
        if !l2_entry.is_present() {
            return Err("L2 entry not present");
        }

        let l1_phys = l2_entry.addr().unwrap();
        let l1_table = unsafe { &mut *(l1_phys.as_u64() as *mut PageTable) };

        // Unmap the page
        let entry = &mut l1_table[breakdown.l1_index];
        let frame = entry.frame().ok_or("Page not mapped")?;
        entry.clear();

        // TODO: TLB flush

        Ok(frame)
    }
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
