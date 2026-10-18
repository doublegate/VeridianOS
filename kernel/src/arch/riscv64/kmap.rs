//! W^X for the kernel image.
//!
//! boot.S maps the image with writable, executable 1 GiB pages, which is all
//! it can do before there is a frame allocator (ADR 0013). [`protect_image`]
//! then maps it in 4 KiB pages with its sections' permissions -- code read
//! and execute, read-only data read, data, BSS and the boot stack read and
//! write -- and swaps the new tables in with one store. The physical map,
//! which also reaches the image, is never executable (boot.S).

use core::ptr::addr_of;

use crate::mm::{
    page_table::{paging_levels, PageTable},
    FrameNumber, FRAME_ALLOCATOR,
};

extern "C" {
    static _start: u8;
    static __text_end: u8;
    static __rodata_end: u8;
    static __kernel_end: u8;
    /// The trampoline root under Sv39 holds a copy of the kernel half.
    static mut veridian_early_l3_phys: PageTable;
}

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

/// Map the image page by page with its sections' permissions, replacing
/// boot.S's writable, executable 1 GiB pages. Called once, after the frame
/// allocator is up and before another hart runs or an address space copies
/// the kernel half (they would keep the old entry).
pub fn protect_image() {
    let start = addr_of!(_start) as u64;
    let text_end = addr_of!(__text_end) as u64;
    let rodata_end = addr_of!(__rodata_end) as u64;
    let end = addr_of!(__kernel_end) as u64;
    const GIB: u64 = 1 << 30;
    let gib = start & !(GIB - 1);
    if end > gib + GIB {
        crate::println!("[RISCV64] W^X: the image crosses a GiB boundary; left as it is");
        return;
    }
    let Some((l2_frame, l2)) = new_table() else {
        crate::println!("[RISCV64] W^X: no frame for the image's page tables");
        return;
    };
    let mut va = start;
    while va < end {
        let entry = &mut l2[((va >> 21) & 0x1FF) as usize];
        if !entry.is_present() {
            let Some((frame, _)) = new_table() else {
                crate::println!("[RISCV64] W^X: no frame for the image's page tables");
                return;
            };
            entry.set_table(frame, false);
        }
        let Some(l1_phys) = entry.addr() else {
            return;
        };
        // SAFETY: a table allocated above, through the physical map.
        let l1 =
            unsafe { &mut *(crate::mm::phys_to_virt_addr(l1_phys.as_u64()) as *mut PageTable) };
        l1[((va >> 12) & 0x1FF) as usize].set(
            FrameNumber::new(super::boot::kernel_phys(va) >> 12),
            crate::mm::kimage::page_flags(va, text_end, rodata_end),
        );
        va += 4096;
    }

    // The table of 1 GiB entries holding the kernel half: the root under
    // Sv39, L4[511]'s table under Sv48 (ADR 0018).
    let root = crate::mm::get_kernel_page_table() as u64;
    // SAFETY: the active root, through the physical map; no other hart
    // runs yet and nothing else changes these entries now.
    let root = unsafe { &mut *(crate::mm::phys_to_virt_addr(root) as *mut PageTable) };
    let half = if paging_levels() == 3 {
        root
    } else {
        let Some(l3) = root[511].addr() else {
            return;
        };
        // SAFETY: as above.
        unsafe { &mut *(crate::mm::phys_to_virt_addr(l3.as_u64()) as *mut PageTable) }
    };
    let index = ((gib >> 30) & 0x1FF) as usize;
    // One store: a walk sees the old 1 GiB page or the new tables, which
    // map the same frames. The next GiB was only ever mapped for symmetry.
    half[index].set_table(l2_frame, false);
    half[index + 1].clear();
    if paging_levels() == 3 {
        // SAFETY: the trampoline root, a boot.S table; secondary harts use
        // it only once they are started, after this.
        let tramp = unsafe { &mut *core::ptr::addr_of_mut!(veridian_early_l3_phys) };
        tramp[index] = half[index];
        tramp[index + 1].clear();
    }
    // SAFETY: drops this hart's translations of the old 1 GiB page.
    unsafe { core::arch::asm!("sfence.vma", options(nostack)) };
    crate::println!(
        "[RISCV64] W^X: kernel code {:#x}..{:#x} read-execute, rodata to {:#x} read-only, data to \
         {:#x}",
        start,
        text_end,
        rodata_end,
        end
    );
}
