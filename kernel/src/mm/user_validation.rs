//! User space memory validation utilities
//!
//! Provides functions to validate user space addresses and check page mappings.

use crate::mm::{
    page_table::{PageTable, PageTableEntry},
    PageFlags,
};

/// Check if a user address is valid (within user space range)
pub fn is_user_addr_valid(addr: usize) -> bool {
    addr < crate::mm::user_layout::USER_SPACE_END
}

/// Translate a virtual address to its page table entry
///
/// Returns None if the address is not mapped
pub fn translate_address(addr: usize) -> Option<PageTableEntry> {
    // Get current process's page table
    let _current_process = crate::process::current_process()?;

    // Get the active page table base address (CR3 on x86_64).
    // In VeridianOS, syscalls run with the process's page tables loaded
    // (CR3 switching was removed from syscalls in v0.4.9 for performance).
    // The kernel is mapped via L4[256-511] in every process's page table,
    // so CR3 already points to the correct page table for translating both
    // user and kernel addresses.
    // Every table is reached through the physical-memory map (INC-MEM-03):
    // the walk used to dereference the physical addresses themselves, which
    // since the map moved into the kernel half (N-134) are low user
    // addresses of the current process -- it could plant fake tables there.
    let table_at = |phys: u64| crate::mm::phys_to_virt_addr(phys) as *const PageTable;
    // The root's low 12 bits are flags (PCID, PWT, PCD on x86_64).
    let root = crate::mm::get_kernel_page_table() as u64 & 0x000F_FFFF_FFFF_F000;
    // SAFETY: get_kernel_page_table() returns the physical address of the
    // active page table root, mapped by the physical-memory map for the
    // kernel's lifetime.
    let page_table = unsafe { &*table_at(root) };

    // Walk the page tables to find the entry
    let vpn = addr >> 12; // Virtual page number

    // 4-level page table walk (x86_64 style)
    let l4_index = (vpn >> 27) & 0x1FF;
    let l3_index = (vpn >> 18) & 0x1FF;
    let l2_index = (vpn >> 9) & 0x1FF;
    let l1_index = vpn & 0x1FF;

    // Walk L4
    let l4_entry = page_table[l4_index];
    if !l4_entry.is_present() {
        return None;
    }

    // Get L3 table
    // SAFETY: l4_entry.addr() returns the physical address of the next-level
    // page table. This address was set by the kernel's page table setup code
    // and points to a valid PageTable, reached through the physical-memory map.
    let l3_table = unsafe { &*table_at(l4_entry.addr()?.as_u64()) };

    let l3_entry = l3_table[l3_index];
    if !l3_entry.is_present() {
        return None;
    }

    // Check for huge page (1GB)
    if l3_entry.flags().contains(PageFlags::HUGE) {
        return Some(l3_entry);
    }

    // Get L2 table
    // SAFETY: l3_entry.addr() returns the physical address of the next-level
    // page table, set by kernel page table initialization. The address points
    // to a valid PageTable, reached through the physical-memory map.
    let l2_table = unsafe { &*table_at(l3_entry.addr()?.as_u64()) };

    let l2_entry = l2_table[l2_index];
    if !l2_entry.is_present() {
        return None;
    }

    // Check for large page (2MB)
    if l2_entry.flags().contains(PageFlags::HUGE) {
        return Some(l2_entry);
    }

    // Get L1 table
    // SAFETY: l2_entry.addr() returns the physical address of the final-level
    // page table, set by kernel page table initialization. The address points
    // to a valid PageTable, reached through the physical-memory map.
    let l1_table = unsafe { &*table_at(l2_entry.addr()?.as_u64()) };

    let l1_entry = l1_table[l1_index];
    if !l1_entry.is_present() {
        return None;
    }

    Some(l1_entry)
}

/// Extension trait for PageTableEntry to check user accessibility
pub trait PageTableEntryExt {
    fn is_user_accessible(&self) -> bool;
}

impl PageTableEntryExt for PageTableEntry {
    fn is_user_accessible(&self) -> bool {
        // Check user bit (bit 2) in flags
        self.flags().contains(PageFlags::USER)
    }
}
