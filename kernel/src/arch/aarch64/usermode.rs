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
    // Currently, the kernel runs with a basic identity mapping set up during
    // early boot. Full user-space page table management (TTBR0_EL1 for
    // user-space, TTBR1_EL1 for kernel-space) requires the VMM to create
    // per-process address spaces with proper PTE_USER (AP[1]=1) bits.
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
    );
}
