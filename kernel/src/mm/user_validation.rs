//! User space memory validation utilities
//!
//! Provides functions to validate user space addresses and check page mappings.

use crate::mm::{page_table::PageTableEntry, PageFlags};

/// Check if a user address is valid (within user space range)
pub fn is_user_addr_valid(addr: usize) -> bool {
    addr < crate::mm::user_layout::user_space_end()
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
    // The kernel half is shared by every process's page table, so the
    // active root translates both user and kernel addresses.
    // The root's low 12 bits are flags (PCID, PWT, PCD on x86_64).
    let root = crate::mm::get_kernel_page_table() as u64 & 0x000F_FFFF_FFFF_F000;
    // The mapper reaches every table through the physical-memory map
    // (INC-MEM-03) and walks as deep as the paging mode (three levels under
    // RISC-V Sv39, ADR 0018).
    // SAFETY: get_kernel_page_table() returns the physical address of the
    // active page table root, mapped by the physical-memory map for the
    // kernel's lifetime; the mapper only reads here.
    let mapper = unsafe { crate::mm::vas::create_mapper_from_root_pub(root) };
    // The 4 KiB frame holding `addr`, also inside a 2 MiB or 1 GiB page.
    let (frame, flags) = mapper
        .translate_page(crate::mm::VirtualAddress(addr as u64 & !0xFFF))
        .ok()?;
    let mut entry = PageTableEntry::empty();
    entry.set(frame, flags.without(PageFlags::HUGE));
    Some(entry)
}

/// The flags of the page holding the user address `addr` in the active
/// address space, or `None` if it is not mapped. Takes no lock (a system
/// call may hold any), and reads the root of the lower half: CR3 and satp
/// hold the whole address space, AArch64's TTBR0_EL1 the user half (ADR
/// 0015).
#[cfg(target_os = "none")]
pub fn active_user_page_flags(addr: usize) -> Option<PageFlags> {
    #[cfg(target_arch = "aarch64")]
    let root = {
        let ttbr0: u64;
        // SAFETY: reading TTBR0_EL1 has no side effects at EL1.
        unsafe { core::arch::asm!("mrs {}, TTBR0_EL1", out(reg) ttbr0) };
        ttbr0 & 0x0000_FFFF_FFFF_F000
    };
    #[cfg(not(target_arch = "aarch64"))]
    let root = crate::mm::get_kernel_page_table() as u64 & 0x000F_FFFF_FFFF_F000;
    // SAFETY: the active root, mapped by the physical-memory map for the
    // kernel's lifetime; the mapper only reads here.
    let mapper = unsafe { crate::mm::vas::create_mapper_from_root_pub(root) };
    mapper
        .translate_page(crate::mm::VirtualAddress(addr as u64 & !0xFFF))
        .ok()
        .map(|(_, flags)| flags)
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
