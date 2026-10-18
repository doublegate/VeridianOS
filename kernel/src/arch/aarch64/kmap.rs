//! W^X for the kernel image (AArch64).
//!
//! boot.S maps the image with writable, executable 2 MiB blocks (ADR 0015).
//! [`protect_image`] maps it in 4 KiB pages with its sections' permissions
//! (`mm::kimage`). Turning a live block into a table of pages would need
//! break-before-make, which the code doing it cannot survive, so, as Linux's
//! `cpu_replace_ttbr1` does, it builds a whole new TTBR1 root and switches to
//! it from the identity map, with the TLB invalidated in between. The
//! physical map was never executable (boot.S).

use core::ptr::addr_of;

use crate::mm::{page_table::PageTable, FrameNumber, FRAME_ALLOCATOR};

extern "C" {
    static _start: u8;
    static __text_end: u8;
    static __rodata_end: u8;
    static __kernel_end: u8;
    /// MAIR, TCR, TTBR0 (boot.S's identity root, executable at EL1, where
    /// the switch runs), TTBR1 and the empty root, which secondary CPUs
    /// load with their MMU off.
    static mut veridian_mmu_regs: [u64; 5];
    fn veridian_replace_ttbr1(root: u64, idroot: u64, step_phys: u64, high: u64);
    static veridian_replace_ttbr1_step: u8;
    static veridian_replace_ttbr1_high: u8;
}

// Switch TTBR1 to x0 while running from the identity map (x1, loaded into
// TTBR0): branch to the physical address of the step (x2), where no TTBR1
// translation is used, replace TTBR1 and invalidate the TLB, then return to
// the upper half (x3) and restore TTBR0. Interrupts are masked throughout;
// between the two branches only registers are touched.
core::arch::global_asm!(
    ".section .text",
    ".balign 4",
    ".global veridian_replace_ttbr1",
    "veridian_replace_ttbr1:",
    "    mrs x5, daif",
    "    msr daifset, #0xf",
    "    dsb ish",
    "    mrs x4, ttbr0_el1",
    "    msr ttbr0_el1, x1",
    "    isb",
    "    tlbi vmalle1",
    "    dsb nsh",
    "    isb",
    "    br x2",
    ".global veridian_replace_ttbr1_step",
    "veridian_replace_ttbr1_step:",
    "    msr ttbr1_el1, x0",
    "    isb",
    "    tlbi vmalle1",
    "    dsb nsh",
    "    isb",
    "    br x3",
    ".global veridian_replace_ttbr1_high",
    "veridian_replace_ttbr1_high:",
    "    msr ttbr0_el1, x4",
    "    isb",
    "    tlbi vmalle1",
    "    dsb nsh",
    "    isb",
    "    msr daif, x5",
    "    ret",
);

/// A zeroed page-table frame, reached through the physical map.
fn new_table() -> Option<(FrameNumber, &'static mut PageTable)> {
    let frame = FRAME_ALLOCATOR.lock().allocate_frames(1, None).ok()?;
    let virt = crate::mm::phys_to_virt_addr(frame.as_u64() << 12);
    // SAFETY: a frame just allocated for this table, owned by nothing else,
    // reached through the physical map.
    let table = unsafe { &mut *(virt as *mut PageTable) };
    table.zero();
    Some((frame, table))
}

/// A table that a present entry points to, through the physical map.
fn table_at(phys: u64) -> &'static mut PageTable {
    // SAFETY: callers pass the address of a live translation table.
    unsafe { &mut *(crate::mm::phys_to_virt_addr(phys) as *mut PageTable) }
}

/// Map the image page by page with its sections' permissions in a new
/// TTBR1 root and switch to it. Called once, after the frame allocator is
/// up and before another CPU starts (they load the new root from
/// `veridian_mmu_regs`).
pub fn protect_image() {
    let start = addr_of!(_start) as u64;
    let text_end = addr_of!(__text_end) as u64;
    let rodata_end = addr_of!(__rodata_end) as u64;
    let end = addr_of!(__kernel_end) as u64;
    const GIB: u64 = 1 << 30;
    let gib = start & !(GIB - 1);
    if end > gib + GIB {
        crate::println!("[AARCH64] W^X: the image crosses a GiB boundary; left as it is");
        return;
    }
    let fail = || crate::println!("[AARCH64] W^X: no frame for the image's page tables");
    let old_root = table_at(crate::mm::get_kernel_page_table() as u64);
    let Some(old_l1) = old_root[511].addr() else {
        return;
    };
    let old_l1 = table_at(old_l1.as_u64());
    let (Some((root_frame, root)), Some((l1_frame, l1)), Some((l2_frame, l2))) =
        (new_table(), new_table(), new_table())
    else {
        fail();
        return;
    };
    // The rest of the kernel half (the physical map) stays as it is.
    for i in 0..512 {
        root[i] = old_root[i];
        l1[i] = old_l1[i];
    }
    let mut va = start;
    while va < end {
        let entry = &mut l2[((va >> 21) & 0x1FF) as usize];
        if !entry.is_present() {
            let Some((frame, _)) = new_table() else {
                fail();
                return;
            };
            entry.set_table(frame, false);
        }
        let Some(l3) = entry.addr() else {
            return;
        };
        table_at(l3.as_u64())[((va >> 12) & 0x1FF) as usize].set(
            FrameNumber::new(super::boot::kernel_phys(va) >> 12),
            crate::mm::kimage::page_flags(va, text_end, rodata_end),
        );
        va += 4096;
    }
    l1[((gib >> 30) & 0x1FF) as usize].set_table(l2_frame, false);
    root[511].set_table(l1_frame, false);

    let root_phys = root_frame.as_u64() << 12;
    let kphys = |p: *const u8| super::boot::kernel_phys(p as u64);
    // SAFETY: boot.S wrote the block before Rust ran; only read.
    let idroot = unsafe { addr_of!(veridian_mmu_regs[2]).read_volatile() };
    // SAFETY: `root_phys` maps the kernel half as the old root does, with
    // the image's pages at the same frames; the identity root maps the
    // switch code at its physical address; no other CPU runs yet.
    unsafe {
        veridian_replace_ttbr1(
            root_phys,
            idroot,
            kphys(addr_of!(veridian_replace_ttbr1_step)),
            addr_of!(veridian_replace_ttbr1_high) as u64,
        );
    }
    // Secondary CPUs read their TTBR1 with the caches off: write it back
    // to the point of coherency.
    // SAFETY: boot.S's block, written only here after boot; no other CPU
    // reads it until it is started.
    unsafe {
        let slot = core::ptr::addr_of_mut!(veridian_mmu_regs[3]);
        slot.write_volatile(root_phys);
        core::arch::asm!("dc civac, {0}", "dsb sy", in(reg) slot, options(nostack));
    }
    crate::println!(
        "[AARCH64] W^X: kernel code {:#x}..{:#x} read-execute, rodata to {:#x} read-only, data to \
         {:#x}",
        start,
        text_end,
        rodata_end,
        end
    );
}
