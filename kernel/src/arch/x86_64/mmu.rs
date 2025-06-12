//! x86_64 Memory Management Unit (MMU) support
//!
//! Handles x86_64-specific paging setup and management.

#![allow(dead_code)]

use crate::mm::{PhysicalAddress, VirtualAddress};

/// Enable paging and set up initial page tables
pub fn init() {
    println!("[x86_64 MMU] Initializing paging...");

    // The bootloader should have already set up paging for us
    // We just need to ensure our kernel is properly mapped

    let cr3 = read_cr3();
    println!("[x86_64 MMU] Current CR3: 0x{:x}", cr3.as_u64());

    // TODO: Set up kernel page tables properly
    // For now, we rely on bootloader's identity mapping
}

/// Read CR3 register (page table base)
pub fn read_cr3() -> PhysicalAddress {
    let cr3: u64;
    unsafe {
        core::arch::asm!("mov {}, cr3", out(reg) cr3);
    }
    PhysicalAddress::new(cr3 & 0x000FFFFF_FFFFF000)
}

/// Write CR3 register (page table base)
pub fn write_cr3(addr: PhysicalAddress) {
    unsafe {
        core::arch::asm!("mov cr3, {}", in(reg) addr.as_u64());
    }
}

/// Invalidate TLB entry for virtual address
pub fn invlpg(virt: VirtualAddress) {
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

    // TODO: Implement proper page fault handling
    // - Check if it's a valid stack growth
    // - Check if it's a valid heap access
    // - Handle copy-on-write
    // - Kill process if invalid access

    panic!("Unhandled page fault at 0x{:x}", faulting_address.as_u64());
}
