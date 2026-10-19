//! Copies to and from user memory that survive a bad user pointer
//! (ARCH-MEM-03), RISC-V.
//!
//! As on x86_64 (`arch/x86_64/usercopy.rs`): the copy loop's two memory
//! instructions are listed in a fixup table. A page fault the kernel takes at
//! one of them is first offered to demand paging (`mm::page_fault`), and if
//! the page cannot be made present the trap handler resumes at the fixup
//! label, which returns failure (EFAULT for the system call) instead of the
//! kernel stopping.
//!
//! The kernel may touch user pages only while `sstatus.SUM` is set (the
//! RISC-V counterpart of x86's SMAP/STAC), and only these routines set it,
//! so a stray kernel access to user memory still faults.
//!
//! The loop copies a byte at a time: simple and correct for unaligned user
//! pointers. Faster word copies can come with measurements.

core::arch::global_asm!(
    ".section .text",
    ".balign 4",
    ".global veridian_riscv_copy_user",
    // a0 = dst, a1 = src, a2 = len; returns 0, or 1 after a fault.
    "veridian_riscv_copy_user:",
    "    li t1, 0x40000", // sstatus.SUM
    "    csrs sstatus, t1",
    "    beqz a2, 2f",
    "1:",
    ".global veridian_riscv_copy_user_load",
    "veridian_riscv_copy_user_load:",
    "    lbu t0, 0(a1)",
    ".global veridian_riscv_copy_user_store",
    "veridian_riscv_copy_user_store:",
    "    sb t0, 0(a0)",
    "    addi a0, a0, 1",
    "    addi a1, a1, 1",
    "    addi a2, a2, -1",
    "    bnez a2, 1b",
    "2:",
    "    csrc sstatus, t1",
    "    li a0, 0",
    "    ret",
    ".global veridian_riscv_copy_user_fault",
    "veridian_riscv_copy_user_fault:",
    "    li t1, 0x40000",
    "    csrc sstatus, t1",
    "    li a0, 1",
    "    ret",
    // Compare-and-exchange of an aligned 32-bit user word (futexes):
    // a0 = ptr, a1 = old, a2 = new, a3 = where to store the value found;
    // returns 0, or 1 after a fault. Both words are sign-extended (the
    // LP64 ABI passes a u32 that way, and lr.w loads that way). A trap
    // between lr and sc drops the reservation, so after a fault demand
    // paging resolved the sc fails and the loop reloads.
    ".balign 4",
    ".global veridian_riscv_cmpxchg_user32",
    "veridian_riscv_cmpxchg_user32:",
    "    li t1, 0x40000",
    "    csrs sstatus, t1",
    // The kernel target has the A extension, but release builds assemble
    // this without it unless it is named here.
    ".option push",
    ".option arch, +a",
    "1:",
    ".global veridian_riscv_cmpxchg_user32_load",
    "veridian_riscv_cmpxchg_user32_load:",
    "    lr.w.aqrl t0, (a0)",
    "    bne t0, a1, 2f",
    ".global veridian_riscv_cmpxchg_user32_store",
    "veridian_riscv_cmpxchg_user32_store:",
    "    sc.w.aqrl t2, a2, (a0)",
    "    bnez t2, 1b",
    ".option pop",
    "2:",
    "    csrc sstatus, t1",
    "    sw t0, 0(a3)",
    "    li a0, 0",
    "    ret",
    ".global veridian_riscv_cmpxchg_user32_fault",
    "veridian_riscv_cmpxchg_user32_fault:",
    "    li t1, 0x40000",
    "    csrc sstatus, t1",
    "    li a0, 1",
    "    ret",
);

extern "C" {
    fn veridian_riscv_copy_user(dst: *mut u8, src: *const u8, len: usize) -> usize;
    fn veridian_riscv_copy_user_load();
    fn veridian_riscv_copy_user_store();
    fn veridian_riscv_copy_user_fault();
    fn veridian_riscv_cmpxchg_user32(ptr: *mut u32, old: u32, new: u32, found: *mut u32) -> usize;
    fn veridian_riscv_cmpxchg_user32_load();
    fn veridian_riscv_cmpxchg_user32_store();
    fn veridian_riscv_cmpxchg_user32_fault();
}

/// Atomically store `new` in the aligned user word at `ptr` if it holds
/// `old`; returns the value found (equal to `old` iff the store happened).
/// `Err` if the page could not be made present and writable.
///
/// # Safety
/// `ptr` must be 4-byte aligned and lie in user space (`validate_user_ptr`).
pub(crate) unsafe fn cmpxchg_user_u32(ptr: *mut u32, old: u32, new: u32) -> Result<u32, ()> {
    let mut found = 0u32;
    // SAFETY: per the contract; `found` is a kernel local; a fault inside
    // resumes at the fixup.
    if unsafe { veridian_riscv_cmpxchg_user32(ptr, old, new, &mut found) } == 0 {
        Ok(found)
    } else {
        Err(())
    }
}

/// Copy `len` bytes where one side is user memory. `Err` if a user page
/// could not be made present.
///
/// # Safety
/// The kernel-side range must be valid for the access; the user-side range
/// must lie in user space (`validate_user_ptr`).
pub(crate) unsafe fn copy_user(dst: *mut u8, src: *const u8, len: usize) -> Result<(), ()> {
    // SAFETY: per the contract; a fault inside resumes at the fixup.
    if unsafe { veridian_riscv_copy_user(dst, src, len) } == 0 {
        Ok(())
    } else {
        Err(())
    }
}

/// The fixup for a kernel fault at `pc`, if `pc` is one of the user-copy
/// instructions.
pub fn fixup_for(pc: usize) -> Option<usize> {
    let at = |f: unsafe extern "C" fn()| f as *const () as usize;
    if pc == at(veridian_riscv_copy_user_load) || pc == at(veridian_riscv_copy_user_store) {
        Some(at(veridian_riscv_copy_user_fault))
    } else if pc == at(veridian_riscv_cmpxchg_user32_load)
        || pc == at(veridian_riscv_cmpxchg_user32_store)
    {
        Some(at(veridian_riscv_cmpxchg_user32_fault))
    } else {
        None
    }
}
