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
/// address space, or `None` if it is not mapped, and the first address
/// past `addr` that may differ: the next page, or the end of the unmapped
/// or huge region `addr` is in, so checking a range costs one walk per
/// mapped page or missing entry rather than per 4 KiB of address space
/// (PR #33 review). Takes no lock (a system call may hold any), and reads
/// the root of the lower half: CR3 and satp hold the whole address space,
/// AArch64's TTBR0_EL1 the user half (ADR 0015). The walk is
/// `walk_lockless`: one atomic load per entry and
/// no reference to any table, against entry writes that are atomic stores,
/// so it reads each entry before or after a concurrent munmap or mprotect
/// (PR #33 review).
#[cfg(target_os = "none")]
pub fn active_user_page(addr: usize) -> (Option<PageFlags>, usize) {
    #[cfg(target_arch = "aarch64")]
    let root = {
        let ttbr0: u64;
        // SAFETY: reading TTBR0_EL1 has no side effects at EL1.
        unsafe { core::arch::asm!("mrs {}, TTBR0_EL1", out(reg) ttbr0) };
        ttbr0 & 0x0000_FFFF_FFFF_F000
    };
    // The low 12 bits are x86_64 CR3 flags (PCID, PWT, PCD); the RISC-V
    // root is already page-aligned.
    #[cfg(not(target_arch = "aarch64"))]
    let root = crate::mm::get_kernel_page_table() as u64 & 0x000F_FFFF_FFFF_F000;
    let root = crate::mm::phys_to_virt_addr(root) as *const crate::mm::page_table::PageTable;
    // SAFETY: the active root, mapped by the physical-memory map for the
    // kernel's lifetime. The user half's table frames are freed only at
    // exec (single-threaded by then) and teardown (no thread left in it),
    // so every table the walk reaches stays allocated.
    let found = unsafe {
        crate::mm::page_table::walk_lockless(
            root,
            crate::mm::page_table::paging_levels(),
            crate::mm::VirtualAddress(addr as u64 & !0xFFF),
        )
    };
    // The leaf's size comes from the level the walk stopped at, never from
    // bit 7 of the flags, which is the PAT bit in a 4 KiB entry on x86_64
    // (PR #33 review).
    let span = match found {
        Ok(leaf) => leaf.span as usize,
        Err(crate::mm::page_table::Hole::NotCanonical) => 0x1000,
        Err(crate::mm::page_table::Hole::Unmapped(span)) => span as usize,
    };
    let next = (addr & !(span - 1)).saturating_add(span);
    (found.ok().map(|leaf| leaf.flags), next)
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
