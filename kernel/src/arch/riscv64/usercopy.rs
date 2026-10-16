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
);

extern "C" {
    fn veridian_riscv_copy_user(dst: *mut u8, src: *const u8, len: usize) -> usize;
    fn veridian_riscv_copy_user_load();
    fn veridian_riscv_copy_user_store();
    fn veridian_riscv_copy_user_fault();
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
    let load = veridian_riscv_copy_user_load as *const () as usize;
    let store = veridian_riscv_copy_user_store as *const () as usize;
    (pc == load || pc == store).then_some(veridian_riscv_copy_user_fault as *const () as usize)
}
