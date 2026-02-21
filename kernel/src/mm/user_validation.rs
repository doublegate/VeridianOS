//! User space memory validation utilities
//!
//! Provides functions to validate user space addresses and check page mappings.

use crate::mm::{
    page_table::{PageTable, PageTableEntry},
    PageFlags,
};

/// Check if a user address is valid (within user space range)
pub fn is_user_addr_valid(addr: usize) -> bool {
    // User space is 0x0 - 0x7FFF_FFFF_FFFF (128TB)
    addr < 0x0000_8000_0000_0000
}

/// Translate a virtual address to its page table entry
///
/// Returns None if the address is not mapped
pub fn translate_address(addr: usize) -> Option<PageTableEntry> {
    // Get current process's page table
    let _current_process = crate::process::current_process()?;

    // Get the page table base address
    // This would normally come from the process's memory space
    // For now, we'll use the kernel page table as a placeholder
    // SAFETY: get_kernel_page_table() returns a usize representing the physical
    // address of the active kernel page table (CR3 on x86_64). Casting to
    // *const PageTable and dereferencing is valid because the page table is
    // identity-mapped in kernel space and has 'static lifetime.
    let page_table = unsafe {
        // TODO(phase5): Get page table from process memory space
        &*(crate::mm::get_kernel_page_table() as *const PageTable)
    };

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
    // and points to a valid PageTable in identity-mapped kernel memory.
    let l3_table = unsafe { &*(l4_entry.addr()?.as_u64() as *const PageTable) };

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
    // to a valid PageTable in identity-mapped kernel memory.
    let l2_table = unsafe { &*(l3_entry.addr()?.as_u64() as *const PageTable) };

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
    // to a valid PageTable in identity-mapped kernel memory.
    let l1_table = unsafe { &*(l2_entry.addr()?.as_u64() as *const PageTable) };

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
