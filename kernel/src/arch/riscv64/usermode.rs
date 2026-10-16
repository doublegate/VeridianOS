//! User-mode entry point for RISC-V 64
//!
//! Provides `try_enter_usermode()` which sets up user-accessible page tables,
//! copies the embedded init binary to a user page, and transitions the CPU
//! from S-mode to U-mode via `sret`.
//!
//! The S-mode → U-mode transition requires:
//! - sstatus.SPP = 0 (return to U-mode)
//! - sepc set to the user-space entry point
//! - sscratch set to the kernel stack pointer (for ecall trap handler)
//! - satp configured with Sv39/Sv48 page tables containing User-accessible
//!   pages
//! - stvec configured for ecall trap handling from U-mode

use core::arch::asm;

/// Attempt to enter user mode with the embedded init binary.
///
/// On success, this function does not return (enters U-mode).
/// On failure, returns a KernelError for the caller to log and fall
/// through to the interactive kernel shell.
pub fn try_enter_usermode() -> Result<(), crate::error::KernelError> {
    // RISC-V user-mode entry requires:
    // 1. satp configured with Sv39 or Sv48 page tables containing U-bit entries
    // 2. stvec configured to handle ecall traps from U-mode
    // 3. Physical frame allocation for user code + stack pages
    //
    // Currently, the RISC-V kernel runs with the MMU disabled (satp = 0,
    // Bare mode). All memory accesses use physical addresses directly.
    // Enabling the MMU requires:
    //   - Building Sv39 page tables with proper PTE flags (V, R, W, X, U)
    //   - Setting satp with MODE=8 (Sv39) and PPN of root page table
    //   - Configuring sfence.vma for TLB management
    //
    // The trap vector (stvec) must handle:
    //   - Environment call from U-mode (ecall → syscall dispatch)
    //   - Timer interrupts (mtime/mtimecmp via SBI)
    //   - Page faults (for demand paging)

    // Check that we are running in S-mode by reading sstatus
    // (if we can read it without trapping, we're in S-mode or M-mode)
    let sstatus: u64;
    // SAFETY: Reading sstatus is valid in S-mode. If we're in M-mode,
    // this reads mstatus instead (which is fine for the check).
    unsafe {
        asm!("csrr {}, sstatus", out(reg) sstatus);
    }

    // Check if satp is configured (non-zero means MMU is enabled)
    let satp: u64;
    // SAFETY: Reading satp is always valid in S-mode.
    unsafe {
        asm!("csrr {}, satp", out(reg) satp);
    }

    if satp == 0 {
        return Err(crate::error::KernelError::NotInitialized {
            subsystem: "satp (Sv39/Sv48 page tables not configured)",
        });
    }

    // Check if stvec is configured for trap handling
    let stvec: u64;
    // SAFETY: Reading stvec is always valid in S-mode.
    unsafe {
        asm!("csrr {}, stvec", out(reg) stvec);
    }

    if stvec == 0 {
        return Err(crate::error::KernelError::NotInitialized {
            subsystem: "stvec (trap vector not configured)",
        });
    }

    // Prerequisites not yet met for full U-mode transition.
    // The MMU must be enabled with Sv39 page tables containing
    // U-bit entries before sret can safely transfer to U-mode.
    // The embedded init binary (RISC-V ecall-based) is ready in
    // userspace::embedded and will be used once the MMU and trap
    // infrastructure supports U-mode execution.
    let _ = sstatus; // Used for prerequisite check above
    Err(crate::error::KernelError::OperationNotSupported {
        operation: "RISC-V U-mode entry (requires Sv39 page tables + ecall handler)",
    })
}

/// Enter user mode via sret.
///
/// # Safety
/// - `entry_point` must be a valid user-space address with executable code
/// - `user_stack` must be a valid user-space stack address
/// - satp must be configured with page tables containing User-accessible
///   mappings
/// - stvec must be configured for U-mode ecall handling
/// - sscratch must contain the kernel stack pointer
#[allow(dead_code)] // User-space transition API -- used when user processes are launched
pub unsafe fn enter_usermode(entry_point: u64, user_stack: u64, kernel_sp: u64) -> ! {
    // SAFETY: forwarded from this function's contract: the entry point and
    // stack are valid user addresses, satp holds User-accessible tables,
    // stvec handles U-mode ecalls and `kernel_sp` is a valid kernel stack, so
    // sret lands in a well-formed U-mode context.
    unsafe {
        asm!(
            // Save kernel stack pointer in sscratch for ecall handler
            "csrw sscratch, {ksp}",
            // Set sepc to user entry point
            "csrw sepc, {entry}",
            // Clear sstatus.SPP (bit 8) to return to U-mode
            "csrc sstatus, {spp_mask}",
            // Set sstatus.SPIE (bit 5) to enable interrupts after sret
            "csrs sstatus, {spie_mask}",
            // Set user stack pointer
            "mv sp, {stack}",
            // Fence for safety
            "sfence.vma",
            // Transition to U-mode
            "sret",
            entry = in(reg) entry_point,
            stack = in(reg) user_stack,
            ksp = in(reg) kernel_sp,
            spp_mask = in(reg) (1u64 << 8),
            spie_mask = in(reg) (1u64 << 5),
            options(noreturn)
        )
    };
}

// Boot-test programs for user mode (position-independent, copied to a user
// page by `bootstrap`). A: getpid, then exit_group with the result (a
// system call round trip). B: a load from address 0 (a user page fault the
// kernel cannot resolve: SIGSEGV). C: reads sstatus, a supervisor CSR
// (illegal in U-mode: SIGILL, which shows the code ran in U-mode).
// Numbers from Linux's generic table: getpid 172, exit_group 94.
core::arch::global_asm!(
    ".section .rodata",
    ".balign 4",
    ".global veridian_rv_user_getpid",
    "veridian_rv_user_getpid:",
    "    li a7, 172",
    "    ecall",
    "    li a7, 94",
    "    ecall",
    "1:  j 1b",
    ".global veridian_rv_user_getpid_end",
    "veridian_rv_user_getpid_end:",
    ".balign 4",
    ".global veridian_rv_user_fault",
    "veridian_rv_user_fault:",
    "    ld a0, 0(zero)",
    "    li a7, 94",
    "    ecall",
    ".global veridian_rv_user_fault_end",
    "veridian_rv_user_fault_end:",
    ".balign 4",
    ".global veridian_rv_user_csr",
    "veridian_rv_user_csr:",
    "    csrr a0, sstatus",
    "    li a7, 94",
    "    ecall",
    ".global veridian_rv_user_csr_end",
    "veridian_rv_user_csr_end:",
    // E: puts its pid in f0 and f31, yields 64 times (sched_yield 124) and
    // exits 0 if both still hold it: two of these running together on one
    // hart see each other's values unless FP state is switched with the
    // task. The kernel is soft-float, so the FP instructions are enabled
    // for this program only.
    ".balign 4",
    ".global veridian_rv_user_fp",
    "veridian_rv_user_fp:",
    ".option push",
    ".option arch, +d",
    "    li a7, 172",
    "    ecall",
    "    mv s1, a0",
    "    fmv.d.x f0, a0",
    "    fmv.d.x f31, a0",
    "    li s2, 64",
    "1:  li a7, 124",
    "    ecall",
    "    addi s2, s2, -1",
    "    bnez s2, 1b",
    "    fmv.x.d t0, f0",
    "    fmv.x.d t1, f31",
    "    xor t0, t0, s1",
    "    xor t1, t1, s1",
    "    or a0, t0, t1",
    "    snez a0, a0",
    "    li a7, 94",
    "    ecall",
    ".option pop",
    ".global veridian_rv_user_fp_end",
    "veridian_rv_user_fp_end:",
    // F: installs a SIGUSR1 handler (rt_sigaction 134, SA_SIGINFO; RISC-V
    // has no restorer: the kernel's trampoline returns), sends itself
    // SIGUSR1 (kill 129). The handler checks si_signo and writes 42 into
    // the saved s1 of its ucontext (uc_mcontext.regs[9], offset 176 + 9 *
    // 8 = 248). After rt_sigreturn, exit(s1 + s2) = 49.
    ".balign 4",
    ".global veridian_rv_user_sig",
    "veridian_rv_user_sig:",
    "    li s2, 7",
    "    li s1, 0",
    "    addi sp, sp, -32",
    "    lla t0, 3f",
    "    sd t0, 0(sp)",
    "    li t0, 4",
    "    sd t0, 8(sp)",
    "    sd zero, 16(sp)",
    "    li a0, 10",
    "    mv a1, sp",
    "    li a2, 0",
    "    li a3, 8",
    "    li a7, 134",
    "    ecall",
    "    bnez a0, 2f",
    "    li a7, 172",
    "    ecall",
    "    li a1, 10",
    "    li a7, 129",
    "    ecall",
    "    add a0, s1, s2",
    "    li a7, 94",
    "    ecall",
    "2:  li a0, 1",
    "    li a7, 94",
    "    ecall",
    "3:  lw t0, 0(a1)",
    "    li t1, 42",
    "    li t2, 10",
    "    beq t0, t2, 5f",
    "    li t1, 1",
    "5:  sd t1, 248(a2)",
    "    ret",
    ".global veridian_rv_user_sig_end",
    "veridian_rv_user_sig_end:",
    // D: the body of an ELF test program: write(1, "RV-ELF-OK\n", 10),
    // then exit_group(argc), argc being the first word of the stack the
    // loader built.
    ".balign 4",
    ".global veridian_rv_user_elf",
    "veridian_rv_user_elf:",
    "    li a0, 1",
    "    lla a1, 2f",
    "    li a2, 10",
    "    li a7, 64",
    "    ecall",
    "    ld a0, 0(sp)",
    "    li a7, 94",
    "    ecall",
    "2:  .ascii \"RV-ELF-OK\\n\"",
    ".global veridian_rv_user_elf_end",
    "veridian_rv_user_elf_end:",
);

extern "C" {
    static veridian_rv_user_getpid: u8;
    static veridian_rv_user_getpid_end: u8;
    static veridian_rv_user_fault: u8;
    static veridian_rv_user_fault_end: u8;
    static veridian_rv_user_csr: u8;
    static veridian_rv_user_csr_end: u8;
    static veridian_rv_user_fp: u8;
    static veridian_rv_user_fp_end: u8;
    static veridian_rv_user_sig: u8;
    static veridian_rv_user_sig_end: u8;
    static veridian_rv_user_elf: u8;
    static veridian_rv_user_elf_end: u8;
}

/// A minimal static RISC-V ELF executable of the ELF test program: one
/// read+execute PT_LOAD at 0x10000 holding the headers and the code, with
/// the entry right after the headers.
pub fn test_elf() -> alloc::vec::Vec<u8> {
    // Only the addresses of the linker symbols are taken (safe).
    let a = core::ptr::addr_of!(veridian_rv_user_elf);
    let b = core::ptr::addr_of!(veridian_rv_user_elf_end);
    // SAFETY: two labels of one rodata block, `a` before `b`.
    let code = unsafe { core::slice::from_raw_parts(a, b as usize - a as usize) };
    const BASE: u64 = 0x10000;
    const HDRS: u64 = 64 + 56;
    let size = HDRS + code.len() as u64;
    let mut e = alloc::vec::Vec::with_capacity(size as usize);
    e.extend_from_slice(&[0x7F, b'E', b'L', b'F', 2, 1, 1, 0]);
    e.extend_from_slice(&[0; 8]);
    e.extend_from_slice(&2u16.to_le_bytes()); // ET_EXEC
    e.extend_from_slice(&243u16.to_le_bytes()); // EM_RISCV
    e.extend_from_slice(&1u32.to_le_bytes());
    e.extend_from_slice(&(BASE + HDRS).to_le_bytes()); // entry
    e.extend_from_slice(&64u64.to_le_bytes()); // phoff
    e.extend_from_slice(&0u64.to_le_bytes()); // shoff
    e.extend_from_slice(&1u32.to_le_bytes()); // flags: RVC, soft float
    e.extend_from_slice(&64u16.to_le_bytes());
    e.extend_from_slice(&56u16.to_le_bytes());
    e.extend_from_slice(&1u16.to_le_bytes()); // phnum
    e.extend_from_slice(&64u16.to_le_bytes());
    e.extend_from_slice(&0u16.to_le_bytes());
    e.extend_from_slice(&0u16.to_le_bytes());
    e.extend_from_slice(&1u32.to_le_bytes()); // PT_LOAD
    e.extend_from_slice(&5u32.to_le_bytes()); // R | X
    e.extend_from_slice(&0u64.to_le_bytes()); // offset
    e.extend_from_slice(&BASE.to_le_bytes()); // vaddr
    e.extend_from_slice(&BASE.to_le_bytes()); // paddr
    e.extend_from_slice(&size.to_le_bytes()); // filesz
    e.extend_from_slice(&size.to_le_bytes()); // memsz
    e.extend_from_slice(&0x1000u64.to_le_bytes()); // align
    e.extend_from_slice(code);
    e
}

/// The boot-test user programs (see above): getpid round trip, page
/// fault, privileged CSR, FP state kept across switches, signal handler
/// round trip.
pub fn test_programs() -> [&'static [u8]; 5] {
    let span = |a: *const u8, b: *const u8| {
        // SAFETY: two labels of one rodata block, `a` before `b`.
        unsafe { core::slice::from_raw_parts(a, b as usize - a as usize) }
    };
    // Only the addresses of the linker symbols are taken (safe).
    [
        span(
            core::ptr::addr_of!(veridian_rv_user_getpid),
            core::ptr::addr_of!(veridian_rv_user_getpid_end),
        ),
        span(
            core::ptr::addr_of!(veridian_rv_user_fault),
            core::ptr::addr_of!(veridian_rv_user_fault_end),
        ),
        span(
            core::ptr::addr_of!(veridian_rv_user_csr),
            core::ptr::addr_of!(veridian_rv_user_csr_end),
        ),
        span(
            core::ptr::addr_of!(veridian_rv_user_fp),
            core::ptr::addr_of!(veridian_rv_user_fp_end),
        ),
        span(
            core::ptr::addr_of!(veridian_rv_user_sig),
            core::ptr::addr_of!(veridian_rv_user_sig_end),
        ),
    ]
}
