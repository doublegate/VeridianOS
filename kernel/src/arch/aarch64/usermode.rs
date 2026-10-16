//! User-mode entry point for AArch64
//!
//! Provides `try_enter_usermode()` which sets up user-accessible page tables,
//! copies the embedded init binary to a user page, and transitions the CPU
//! from EL1 to EL0 via `eret`.
//!
//! The EL1 → EL0 transition requires:
//! - SPSR_EL1 set to EL0t (0x0) with DAIF interrupts enabled
//! - ELR_EL1 set to the user-space entry point
//! - SP_EL0 set to the user-space stack pointer
//! - TTBR0_EL1 pointing to page tables with User-accessible mappings
//! - VBAR_EL1 configured for exception handling from EL0

use core::arch::asm;

/// Attempt to enter user mode with the embedded init binary.
///
/// On success, this function does not return (enters EL0).
/// On failure, returns a KernelError for the caller to log and fall
/// through to the interactive kernel shell.
pub fn try_enter_usermode() -> Result<(), crate::error::KernelError> {
    // AArch64 user-mode entry requires:
    // 1. MMU enabled with TTBR0_EL1 page tables containing User-accessible mappings
    // 2. Exception vectors (VBAR_EL1) configured to handle SVC from EL0
    // 3. Physical frame allocation for user code + stack pages
    //
    // The MMU is on with the kernel in TTBR1_EL1 (ADR 0015), but TTBR0_EL1
    // still holds the boot identity map, not a process: per-process address
    // spaces with PTE_USER (AP[1]=1) entries and the SVC path come next.
    //
    // The exception vector table must handle:
    //   - Synchronous exceptions from EL0 (SVC for syscalls)
    //   - IRQ/FIQ from EL0 (timer interrupts, device interrupts)
    //
    // Until the full page table and exception infrastructure is in place,
    // we verify prerequisites and return an appropriate error.

    // Check that we are running at EL1
    let current_el: u64;
    // SAFETY: Reading CurrentEL is always valid in kernel mode.
    unsafe {
        asm!("mrs {}, CurrentEL", out(reg) current_el);
    }
    let el = (current_el >> 2) & 0x3;
    if el != 1 {
        return Err(crate::error::KernelError::OperationNotSupported {
            operation: "usermode entry requires EL1",
        });
    }

    // Check that TTBR0_EL1 is configured (non-zero)
    let ttbr0: u64;
    // SAFETY: Reading TTBR0_EL1 is always valid at EL1.
    unsafe {
        asm!("mrs {}, TTBR0_EL1", out(reg) ttbr0);
    }

    if ttbr0 == 0 {
        return Err(crate::error::KernelError::NotInitialized {
            subsystem: "TTBR0_EL1 (user page tables)",
        });
    }

    // Check that VBAR_EL1 is configured for exception handling
    let vbar: u64;
    // SAFETY: Reading VBAR_EL1 is always valid at EL1.
    unsafe {
        asm!("mrs {}, VBAR_EL1", out(reg) vbar);
    }

    if vbar == 0 {
        return Err(crate::error::KernelError::NotInitialized {
            subsystem: "VBAR_EL1 (exception vectors)",
        });
    }

    // Prerequisites not yet met for full EL0 transition.
    // User-space page table management (per-process TTBR0_EL1 with AP[1]=1
    // user-accessible entries) and EL0 exception handling (SVC dispatch)
    // are required before eret can safely transfer to EL0.
    //
    // The embedded init binary (AArch64 SVC-based) is ready in
    // userspace::embedded and will be used once the MMU and exception
    // infrastructure supports EL0 execution.
    Err(crate::error::KernelError::OperationNotSupported {
        operation: "AArch64 EL0 entry (requires per-process TTBR0 + SVC handler)",
    })
}

/// Enter user mode via eret.
///
/// # Safety
/// - `entry_point` must be a valid user-space address with executable code
///   mapped
/// - `user_stack` must be a valid user-space stack address, 16-byte aligned
/// - TTBR0_EL1 must point to page tables with User-accessible mappings
/// - VBAR_EL1 must be configured for EL0 exception handling
#[allow(dead_code)] // User-space transition API -- used when user processes are launched
pub unsafe fn enter_usermode(entry_point: u64, user_stack: u64) -> ! {
    // Set SPSR_EL1: return to EL0t (EL0 using SP_EL0)
    // Bits: M[3:0]=0b0000 (EL0t), DAIF cleared (interrupts enabled)
    // SAFETY: forwarded from this function's contract: the entry point is
    // mapped executable for EL0, the stack is a valid 16-byte aligned user
    // stack, TTBR0_EL1 holds User-accessible tables and VBAR_EL1 handles EL0
    // exceptions, so eret lands in a well-formed EL0 context.
    unsafe {
        asm!(
            // Configure SPSR_EL1 for EL0t with interrupts enabled
            "msr SPSR_EL1, {spsr}",
            // Set return address to user entry point
            "msr ELR_EL1, {entry}",
            // Set user stack pointer
            "msr SP_EL0, {stack}",
            // Synchronize
            "isb",
            // Transition to EL0
            "eret",
            spsr = in(reg) 0u64,       // EL0t, all interrupts enabled
            entry = in(reg) entry_point,
            stack = in(reg) user_stack,
            options(noreturn)
        )
    };
}

// Boot-test programs for EL0 (position-independent, copied to a user page
// by `bootstrap`), as on RISC-V (ADR 0016). A: getpid, then exit_group
// with the result (a system call round trip). B: a load from address 0 (a
// user fault the kernel cannot resolve: SIGSEGV). C: reads SCTLR_EL1, an
// EL1 register (undefined at EL0: SIGILL, which shows the code ran at
// EL0). E: puts its pid in d0 and d31 (`fmov`, as `.inst` since the kernel
// is soft-float), yields 64 times (sched_yield 124) and exits 0 if both
// still hold it: two of these running together on one CPU see each other's
// values unless FP state is switched with the task. Numbers from Linux's
// generic table: getpid 172, exit_group 94.
core::arch::global_asm!(
    ".section .rodata",
    ".balign 4",
    ".global veridian_a64_user_getpid",
    "veridian_a64_user_getpid:",
    "    mov x8, #172",
    "    svc #0",
    "    mov x8, #94",
    "    svc #0",
    "1:  b 1b",
    ".global veridian_a64_user_getpid_end",
    "veridian_a64_user_getpid_end:",
    ".balign 4",
    ".global veridian_a64_user_fault",
    "veridian_a64_user_fault:",
    "    mov x1, #0",
    "    ldr x0, [x1]",
    "    mov x8, #94",
    "    svc #0",
    ".global veridian_a64_user_fault_end",
    "veridian_a64_user_fault_end:",
    ".balign 4",
    ".global veridian_a64_user_sysreg",
    "veridian_a64_user_sysreg:",
    "    mrs x0, sctlr_el1",
    "    mov x8, #94",
    "    svc #0",
    ".global veridian_a64_user_sysreg_end",
    "veridian_a64_user_sysreg_end:",
    ".balign 4",
    ".global veridian_a64_user_fp",
    "veridian_a64_user_fp:",
    "    mov x8, #172",
    "    svc #0",
    "    mov x19, x0",
    "    .inst 0x9e670000", // fmov d0, x0
    "    .inst 0x9e67001f", // fmov d31, x0
    "    mov x20, #64",
    "1:  mov x8, #124",
    "    svc #0",
    "    subs x20, x20, #1",
    "    b.ne 1b",
    "    .inst 0x9e660002", // fmov x2, d0
    "    .inst 0x9e6603e3", // fmov x3, d31
    "    eor x2, x2, x19",
    "    eor x3, x3, x19",
    "    orr x0, x2, x3",
    "    cbz x0, 2f",
    "    mov x0, #1",
    "2:  mov x8, #94",
    "    svc #0",
    ".global veridian_a64_user_fp_end",
    "veridian_a64_user_fp_end:",
    // F: installs a SIGUSR1 handler (rt_sigaction 134, SA_SIGINFO |
    // SA_RESTORER with its own rt_sigreturn stub, as musl does), sends
    // itself SIGUSR1 (kill 129). The handler checks si_signo and writes 42
    // into the saved x19 of its ucontext (uc_mcontext.regs[19], offset
    // 176 + 8 + 19 * 8 = 336). After rt_sigreturn, exit(x19 + x20) = 49
    // shows the frame, the handler's arguments, the restorer and the
    // restored (modified) context all worked.
    ".balign 4",
    ".global veridian_a64_user_sig",
    "veridian_a64_user_sig:",
    "    mov x20, #7",
    "    mov x19, #0",
    "    sub sp, sp, #32",
    "    adr x0, 3f",
    "    str x0, [sp]",
    "    mov x0, #0x4000000",
    "    orr x0, x0, #4",
    "    str x0, [sp, #8]",
    "    adr x0, 4f",
    "    str x0, [sp, #16]",
    "    str xzr, [sp, #24]",
    "    mov x0, #10",
    "    mov x1, sp",
    "    mov x2, #0",
    "    mov x3, #8",
    "    mov x8, #134",
    "    svc #0",
    "    cbnz x0, 2f",
    "    mov x8, #172",
    "    svc #0",
    "    mov x1, #10",
    "    mov x8, #129",
    "    svc #0",
    "    add x0, x19, x20",
    "    mov x8, #94",
    "    svc #0",
    "2:  mov x0, #1",
    "    mov x8, #94",
    "    svc #0",
    "3:  ldr w3, [x1]",
    "    mov x4, #42",
    "    cmp w3, #10",
    "    b.eq 5f",
    "    mov x4, #1",
    "5:  str x4, [x2, #336]",
    "    ret",
    "4:  mov x8, #139",
    "    svc #0",
    ".global veridian_a64_user_sig_end",
    "veridian_a64_user_sig_end:",
    // D: the body of an ELF test program: write(1, "A64-ELF-OK\n", 11),
    // then exit_group(argc), argc being the first word of the stack the
    // loader built.
    ".balign 4",
    ".global veridian_a64_user_elf",
    "veridian_a64_user_elf:",
    "    mov x0, #1",
    "    adr x1, 2f",
    "    mov x2, #11",
    "    mov x8, #64",
    "    svc #0",
    "    ldr x0, [sp]",
    "    mov x8, #94",
    "    svc #0",
    "2:  .ascii \"A64-ELF-OK\\n\"",
    ".global veridian_a64_user_elf_end",
    "veridian_a64_user_elf_end:",
);

extern "C" {
    static veridian_a64_user_getpid: u8;
    static veridian_a64_user_getpid_end: u8;
    static veridian_a64_user_fault: u8;
    static veridian_a64_user_fault_end: u8;
    static veridian_a64_user_sysreg: u8;
    static veridian_a64_user_sysreg_end: u8;
    static veridian_a64_user_fp: u8;
    static veridian_a64_user_fp_end: u8;
    static veridian_a64_user_sig: u8;
    static veridian_a64_user_sig_end: u8;
    static veridian_a64_user_elf: u8;
    static veridian_a64_user_elf_end: u8;
}

fn span(a: *const u8, b: *const u8) -> &'static [u8] {
    // SAFETY: two labels of one rodata block, `a` before `b`.
    unsafe { core::slice::from_raw_parts(a, b as usize - a as usize) }
}

/// A minimal static AArch64 ELF executable of the ELF test program: one
/// read+execute PT_LOAD at 0x10000 holding the headers and the code, with
/// the entry right after the headers.
pub fn test_elf() -> alloc::vec::Vec<u8> {
    // Only the addresses of the linker symbols are taken (safe).
    let code = span(
        core::ptr::addr_of!(veridian_a64_user_elf),
        core::ptr::addr_of!(veridian_a64_user_elf_end),
    );
    const BASE: u64 = 0x10000;
    const HDRS: u64 = 64 + 56;
    let size = HDRS + code.len() as u64;
    let mut e = alloc::vec::Vec::with_capacity(size as usize);
    e.extend_from_slice(&[0x7F, b'E', b'L', b'F', 2, 1, 1, 0]);
    e.extend_from_slice(&[0; 8]);
    e.extend_from_slice(&2u16.to_le_bytes()); // ET_EXEC
    e.extend_from_slice(&183u16.to_le_bytes()); // EM_AARCH64
    e.extend_from_slice(&1u32.to_le_bytes());
    e.extend_from_slice(&(BASE + HDRS).to_le_bytes()); // entry
    e.extend_from_slice(&64u64.to_le_bytes()); // phoff
    e.extend_from_slice(&0u64.to_le_bytes()); // shoff
    e.extend_from_slice(&0u32.to_le_bytes()); // flags
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
/// fault, EL1 system register, FP state kept across switches, signal
/// handler round trip.
pub fn test_programs() -> [&'static [u8]; 5] {
    // Only the addresses of the linker symbols are taken (safe).
    [
        span(
            core::ptr::addr_of!(veridian_a64_user_getpid),
            core::ptr::addr_of!(veridian_a64_user_getpid_end),
        ),
        span(
            core::ptr::addr_of!(veridian_a64_user_fault),
            core::ptr::addr_of!(veridian_a64_user_fault_end),
        ),
        span(
            core::ptr::addr_of!(veridian_a64_user_sysreg),
            core::ptr::addr_of!(veridian_a64_user_sysreg_end),
        ),
        span(
            core::ptr::addr_of!(veridian_a64_user_fp),
            core::ptr::addr_of!(veridian_a64_user_fp_end),
        ),
        span(
            core::ptr::addr_of!(veridian_a64_user_sig),
            core::ptr::addr_of!(veridian_a64_user_sig_end),
        ),
    ]
}
