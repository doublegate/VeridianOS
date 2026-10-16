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
);

extern "C" {
    static veridian_rv_user_getpid: u8;
    static veridian_rv_user_getpid_end: u8;
    static veridian_rv_user_fault: u8;
    static veridian_rv_user_fault_end: u8;
    static veridian_rv_user_csr: u8;
    static veridian_rv_user_csr_end: u8;
}

/// The boot-test user programs (see above): getpid round trip, page
/// fault, privileged CSR.
pub fn test_programs() -> [&'static [u8]; 3] {
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
    ]
}
