//! Page Attribute Table (PAT) initialization for write-combining memory.
//!
//! Reprograms PAT entry 1 from WT (write-through) to WC (write-combining),
//! then provides `apply_write_combining()` to set framebuffer page table
//! entries to use the WC memory type. This yields 5-150x faster MMIO
//! writes for the fbcon flush path.
//!
//! # PAT index mapping after `init()`
//!
//! | Index | PWT | PCD | PAT | Type |
//! |-------|-----|-----|-----|------|
//! | 0     | 0   | 0   | 0   | WB   |
//! | 1     | 1   | 0   | 0   | **WC** (was WT) |
//! | 2     | 0   | 1   | 0   | UC-  |
//! | 3     | 1   | 1   | 0   | UC   |
//! | 4-7   |     |     | 1   | (mirrors 0-3 by default) |
//!
//! Framebuffer PTEs use index 1 (PWT=1, PCD=0, PAT=0) for write-combining.

use super::msr::{phys_to_virt, rdmsr, wrmsr};

/// IA32_PAT MSR address.
const IA32_PAT: u32 = 0x277;

/// PAT memory type: Write-Combining.
const PAT_WC: u64 = 0x01;

/// PTE flag: Page Write-Through (bit 3).
const PTE_PWT: u64 = 1 << 3;
/// PTE flag: Page Cache Disable (bit 4).
const PTE_PCD: u64 = 1 << 4;
/// PTE flag: PAT bit in leaf PTE (bit 7).
const PTE_PAT: u64 = 1 << 7;
/// PTE flag: Present (bit 0).
const PTE_PRESENT: u64 = 1 << 0;

/// Check CPUID for PAT support (leaf 1, EDX bit 16).
fn cpu_has_pat() -> bool {
    let edx: u32;
    // SAFETY: CPUID with EAX=1 is a read-only, side-effect-free instruction.
    // RBX is saved/restored because LLVM reserves it as a frame pointer.
    unsafe {
        core::arch::asm!(
            "push rbx",
            "mov eax, 1",
            "cpuid",
            "pop rbx",
            out("edx") edx,
            out("eax") _,
            out("ecx") _,
            options(nomem, preserves_flags),
        );
    }
    (edx & (1 << 16)) != 0
}

/// Reprogram PAT entry 1 from WT to WC.
///
/// Must be called early in boot, before any memory is mapped with PAT
/// index 1. No-op if the CPU does not support PAT.
pub fn init() {
    if !cpu_has_pat() {
        return;
    }
    let mut pat = rdmsr(IA32_PAT);
    // Clear PAT entry 1 (bits [15:8]) and set to WC (0x01)
    pat = (pat & !0xFF00) | (PAT_WC << 8);
    wrmsr(IA32_PAT, pat);
}

/// Apply write-combining attributes to a virtual address range.
///
/// Walks the active page table (CR3), finds PTEs for each 4KB page in
/// the range, sets PWT=1 PCD=0 PAT=0 (PAT index 1 = WC after `init()`),
/// and flushes the TLB entry.
///
/// # Safety
///
/// - `vaddr` must be page-aligned and mapped with 4KB pages.
/// - `size` must be a multiple of 4096.
/// - PAT entry 1 must have been reprogrammed to WC via `init()`.
pub unsafe fn apply_write_combining(vaddr: usize, size: usize) {
    if !cpu_has_pat() {
        return;
    }

    // Read CR3 for PML4 physical address
    let cr3: u64;
    core::arch::asm!("mov {}, cr3", out(reg) cr3);
    let pml4_phys = (cr3 & 0x000F_FFFF_FFFF_F000) as usize;

    let num_pages = size / 4096;
    for i in 0..num_pages {
        let addr = vaddr + i * 4096;
        set_page_wc(pml4_phys, addr);
    }
}

/// Set a single 4KB page's PTE to use PAT index 1 (WC).
///
/// Walks PML4 -> PDPT -> PD -> PT, reads the leaf PTE, sets PWT=1,
/// clears PCD and PAT, writes back, and flushes the TLB for that address.
unsafe fn set_page_wc(pml4_phys: usize, vaddr: usize) {
    // Extract page table indices from the virtual address
    let pml4_idx = (vaddr >> 39) & 0x1FF;
    let pdpt_idx = (vaddr >> 30) & 0x1FF;
    let pd_idx = (vaddr >> 21) & 0x1FF;
    let pt_idx = (vaddr >> 12) & 0x1FF;

    // Walk PML4 -> PDPT
    let pml4_virt = match phys_to_virt(pml4_phys) {
        Some(v) => v as *const u64,
        None => return,
    };
    let pml4_entry = pml4_virt.add(pml4_idx).read_volatile();
    if (pml4_entry & PTE_PRESENT) == 0 {
        return;
    }
    let pdpt_phys = (pml4_entry & 0x000F_FFFF_FFFF_F000) as usize;

    // Walk PDPT -> PD
    let pdpt_virt = match phys_to_virt(pdpt_phys) {
        Some(v) => v as *const u64,
        None => return,
    };
    let pdpt_entry = pdpt_virt.add(pdpt_idx).read_volatile();
    if (pdpt_entry & PTE_PRESENT) == 0 {
        return;
    }
    // Check for 1GiB huge page (bit 7 = PS)
    if (pdpt_entry & (1 << 7)) != 0 {
        return; // Cannot set WC on huge pages via this path
    }
    let pd_phys = (pdpt_entry & 0x000F_FFFF_FFFF_F000) as usize;

    // Walk PD -> PT
    let pd_virt = match phys_to_virt(pd_phys) {
        Some(v) => v as *const u64,
        None => return,
    };
    let pd_entry = pd_virt.add(pd_idx).read_volatile();
    if (pd_entry & PTE_PRESENT) == 0 {
        return;
    }
    // Check for 2MiB huge page (bit 7 = PS)
    if (pd_entry & (1 << 7)) != 0 {
        return; // Cannot set WC on huge pages via this path
    }
    let pt_phys = (pd_entry & 0x000F_FFFF_FFFF_F000) as usize;

    // Read and modify leaf PTE
    let pt_virt = match phys_to_virt(pt_phys) {
        Some(v) => v as *mut u64,
        None => return,
    };
    let pt_entry_ptr = pt_virt.add(pt_idx);
    let mut pte = pt_entry_ptr.read_volatile();
    if (pte & PTE_PRESENT) == 0 {
        return;
    }

    // Set PAT index 1: PWT=1, PCD=0, PAT(bit7)=0
    pte |= PTE_PWT;
    pte &= !PTE_PCD;
    pte &= !PTE_PAT;
    pt_entry_ptr.write_volatile(pte);

    // Flush TLB for this address
    super::tlb_flush_address(vaddr as u64);
}
