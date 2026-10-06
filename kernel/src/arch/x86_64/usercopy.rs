//! Fault-tolerant copies between kernel and user memory (x86_64).
//!
//! A syscall handed an in-range but unmapped user pointer used to fault in
//! kernel mode, and the page-fault handler could only abandon the syscall
//! by unwinding to the boot context -- with whatever locks the handler held
//! (MEM-SEC-01, SYS-SEC-01, W-12).
//!
//! [`copy_user`] does the copy with a single `rep movsb` whose address is
//! exported. When the page-fault handler sees a kernel-mode fault on a user
//! address at exactly that instruction, it first tries to resolve the fault
//! (demand paging), and otherwise resumes at the fixup stub, which makes
//! `copy_user` return `Err` -- the syscall then fails with EFAULT and
//! unwinds normally, releasing its locks. This is the single-entry form of
//! the Linux `__ex_table` mechanism.

// rdi = dst, rsi = src, rdx = len. Returns 0 on success, -14 (EFAULT) if
// the copy faulted. `rep movsb` leaves rcx at the bytes remaining, which
// the fixup does not need.
core::arch::global_asm!(
    ".section .text",
    ".global veridian_copy_user",
    "veridian_copy_user:",
    "    mov rcx, rdx",
    ".global veridian_copy_user_insn",
    "veridian_copy_user_insn:",
    "    rep movsb",
    "    xor eax, eax",
    "    ret",
    ".global veridian_copy_user_fixup",
    "veridian_copy_user_fixup:",
    "    mov rax, -14",
    "    ret",
);

extern "C" {
    fn veridian_copy_user(dst: *mut u8, src: *const u8, len: usize) -> isize;
    fn veridian_copy_user_insn();
    fn veridian_copy_user_fixup();
}

/// Copy `len` bytes from `src` to `dst`, either of which may be a user
/// address. Returns `Err(())` if the copy faulted on an unmapped or
/// inaccessible user page.
///
/// # Safety
///
/// The caller must have validated that every user-side address in the
/// range lies in the user half (so a fault can only be a missing user
/// mapping, never kernel memory), and that the kernel-side range is valid.
pub(crate) unsafe fn copy_user(dst: *mut u8, src: *const u8, len: usize) -> Result<(), ()> {
    if len == 0 {
        return Ok(());
    }
    // SAFETY: forwarded from the caller's contract; the routine only reads
    // `src..src+len` and writes `dst..dst+len`.
    match unsafe { veridian_copy_user(dst, src, len) } {
        0 => Ok(()),
        _ => Err(()),
    }
}

/// Fixup address for a kernel-mode fault at `rip`, if `rip` is the
/// fault-tolerant copy instruction.
pub(crate) fn fixup_for(rip: u64) -> Option<u64> {
    let insn = veridian_copy_user_insn as unsafe extern "C" fn() as usize as u64;
    (rip == insn).then_some(veridian_copy_user_fixup as unsafe extern "C" fn() as usize as u64)
}
