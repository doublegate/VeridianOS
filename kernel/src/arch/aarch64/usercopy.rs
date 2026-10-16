//! Copies to and from user memory that survive a bad user pointer
//! (ARCH-MEM-03), AArch64.
//!
//! As on x86_64 and RISC-V (`arch/riscv64/usercopy.rs`): the copy loop's
//! two memory instructions are listed, a data abort the kernel takes at one
//! of them is first offered to demand paging (`mm::page_fault`), and if the
//! page cannot be made present the exception handler resumes at the fixup
//! label, which returns failure (EFAULT for the system call) instead of the
//! kernel stopping (`exceptions::aarch64_kernel_sync`).
//!
//! The kernel can reach user pages directly (their AP[1] grants EL0 access
//! and EL1 keeps it; ARMv8.0 has no PAN), so nothing is switched around the
//! copy. The loop copies a byte at a time: simple and correct for unaligned
//! user pointers. Faster copies can come with measurements.

core::arch::global_asm!(
    ".section .text",
    ".balign 4",
    ".global veridian_aarch64_copy_user",
    // x0 = dst, x1 = src, x2 = len; returns 0, or 1 after a fault.
    "veridian_aarch64_copy_user:",
    "    cbz x2, 2f",
    "1:",
    ".global veridian_aarch64_copy_user_load",
    "veridian_aarch64_copy_user_load:",
    "    ldrb w3, [x1], #1",
    ".global veridian_aarch64_copy_user_store",
    "veridian_aarch64_copy_user_store:",
    "    strb w3, [x0], #1",
    "    subs x2, x2, #1",
    "    b.ne 1b",
    "2:",
    "    mov x0, #0",
    "    ret",
    ".global veridian_aarch64_copy_user_fault",
    "veridian_aarch64_copy_user_fault:",
    "    mov x0, #1",
    "    ret",
);

extern "C" {
    fn veridian_aarch64_copy_user(dst: *mut u8, src: *const u8, len: usize) -> usize;
    fn veridian_aarch64_copy_user_load();
    fn veridian_aarch64_copy_user_store();
    fn veridian_aarch64_copy_user_fault();
}

/// Copy `len` bytes where one side is user memory. `Err` if a user page
/// could not be made present.
///
/// # Safety
/// The kernel-side range must be valid for the access; the user-side range
/// must lie in user space (`validate_user_ptr`).
pub(crate) unsafe fn copy_user(dst: *mut u8, src: *const u8, len: usize) -> Result<(), ()> {
    // SAFETY: per the contract; a fault inside resumes at the fixup.
    if unsafe { veridian_aarch64_copy_user(dst, src, len) } == 0 {
        Ok(())
    } else {
        Err(())
    }
}

/// The fixup for a kernel fault at `pc`, if `pc` is one of the user-copy
/// instructions.
pub fn fixup_for(pc: usize) -> Option<usize> {
    let load = veridian_aarch64_copy_user_load as *const () as usize;
    let store = veridian_aarch64_copy_user_store as *const () as usize;
    (pc == load || pc == store).then_some(veridian_aarch64_copy_user_fault as *const () as usize)
}
