//! x86_64 Memory Management Unit (MMU) support
//!
//! Handles x86_64-specific paging setup and management.

// x86_64 MMU support

use crate::mm::{PhysicalAddress, VirtualAddress};

/// Enable paging and set up initial page tables
pub fn init() {
    println!("[x86_64 MMU] Initializing paging...");

    // The bootloader should have already set up paging for us
    // We just need to ensure our kernel is properly mapped

    let cr3 = read_cr3();
    println!("[x86_64 MMU] Current CR3: 0x{:x}", cr3.as_u64());

    // The bootloader sets up initial page tables with kernel mapped at
    // L4[256-511].  Each process gets its own L4 with the kernel entries
    // copied from this root.  Dedicated kernel page tables are not needed
    // because the per-process page tables already include the kernel mapping.
    // KPTI shadow page tables for Meltdown mitigation are in kpti.rs.
}

/// Read CR3 register (page table base)
pub fn read_cr3() -> PhysicalAddress {
    let cr3: u64;
    // SAFETY: Reading CR3 is a privileged operation that returns the physical
    // address of the current page table root. Always accessible in kernel mode.
    unsafe {
        core::arch::asm!("mov {}, cr3", out(reg) cr3);
    }
    PhysicalAddress::new(cr3 & 0x000FFFFF_FFFFF000)
}

/// Write CR3 register (page table base)
pub fn write_cr3(addr: PhysicalAddress) {
    // SAFETY: Writing CR3 sets the page table root and flushes the TLB. The
    // caller must ensure `addr` points to a valid, properly aligned PML4 table.
    unsafe {
        core::arch::asm!("mov cr3, {}", in(reg) addr.as_u64());
    }
}

/// Invalidate TLB entry for virtual address
pub fn invlpg(virt: VirtualAddress) {
    // SAFETY: invlpg invalidates the TLB entry for the specified virtual address.
    // This is a privileged, non-destructive operation that only affects caching.
    unsafe {
        core::arch::asm!("invlpg [{}]", in(reg) virt.as_u64());
    }
}

/// Flush entire TLB by reloading CR3
pub fn flush_tlb() {
    let cr3 = read_cr3();
    write_cr3(cr3);
}

/// Flush TLB entry for a specific address
pub fn flush_tlb_address(addr: u64) {
    invlpg(VirtualAddress::new(addr));
}

/// Read CR2 register (page fault address)
pub fn read_cr2() -> VirtualAddress {
    let cr2: u64;
    // SAFETY: Reading CR2 returns the faulting virtual address from the last
    // page fault. Always accessible in kernel mode with no side effects.
    unsafe {
        core::arch::asm!("mov {}, cr2", out(reg) cr2);
    }
    VirtualAddress::new(cr2)
}

/// Page fault error code bits
#[derive(Debug, Clone, Copy)]
pub struct PageFaultErrorCode(u32);

impl PageFaultErrorCode {
    /// Was the fault caused by a page-level protection violation?
    pub fn protection_violation(&self) -> bool {
        self.0 & 0x1 != 0
    }

    /// Was the access a write?
    pub fn write(&self) -> bool {
        self.0 & 0x2 != 0
    }

    /// Was the access in user mode?
    pub fn user_mode(&self) -> bool {
        self.0 & 0x4 != 0
    }

    /// Was the fault caused by reserved bit violation?
    pub fn reserved_write(&self) -> bool {
        self.0 & 0x8 != 0
    }

    /// Was the fault caused by instruction fetch?
    pub fn instruction_fetch(&self) -> bool {
        self.0 & 0x10 != 0
    }
}

/// Handle page fault
pub fn handle_page_fault(error_code: u32, faulting_address: VirtualAddress) {
    let error = PageFaultErrorCode(error_code);

    println!(
        "[x86_64 MMU] Page fault at 0x{:x}",
        faulting_address.as_u64()
    );
    println!("  Protection violation: {}", error.protection_violation());
    println!("  Write access: {}", error.write());
    println!("  User mode: {}", error.user_mode());
    println!("  Reserved bit: {}", error.reserved_write());
    println!("  Instruction fetch: {}", error.instruction_fetch());

    // Delegate to the unified page fault handler which attempts demand paging,
    // copy-on-write, and stack growth before giving up.
    let info = crate::mm::page_fault::from_x86_64(
        error_code as u64,
        faulting_address.as_u64(),
        0, // RIP not available in this legacy path
    );
    if let Err(_e) = crate::mm::page_fault::handle_page_fault(info) {
        panic!(
            "Unhandled page fault at 0x{:x}: {}",
            faulting_address.as_u64(),
            _e
        );
    }
}
