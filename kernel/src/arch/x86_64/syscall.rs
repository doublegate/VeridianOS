//! x86_64 system call entry point

#![allow(function_casts_as_integer)]

use crate::syscall::syscall_handler;

/// x86_64 SYSCALL instruction entry point
///
/// This function handles the transition from user mode to kernel mode
/// when a SYSCALL instruction is executed. It saves the user context,
/// switches to the kernel stack, and calls the system call handler.
///
/// # Safety
/// This function must only be called by the CPU's SYSCALL instruction.
/// It expects specific register states as defined by the x86_64 ABI.
#[no_mangle]
#[unsafe(naked)]
pub unsafe extern "C" fn syscall_entry() {
    core::arch::naked_asm!(
        // Save user context on kernel stack
        "swapgs",                    // Switch to kernel GS
        "mov gs:[0x8], rsp",        // Save user RSP in per-CPU data
        "mov rsp, gs:[0x0]",        // Load kernel RSP from per-CPU data

        // Save registers
        "push rcx",                  // User RIP
        "push r11",                  // User RFLAGS
        "push rbp",
        "push rbx",
        "push r12",
        "push r13",
        "push r14",
        "push r15",

        // Call syscall handler with proper arguments
        // rax = syscall number
        // rdi = arg1, rsi = arg2, rdx = arg3
        // r10 = arg4, r8 = arg5, r9 = arg6
        "mov rcx, r10",              // Move arg4 to rcx (ABI mismatch fix)
        "call {handler}",

        // Restore registers
        "pop r15",
        "pop r14",
        "pop r13",
        "pop r12",
        "pop rbx",
        "pop rbp",
        "pop r11",                   // User RFLAGS
        "pop rcx",                   // User RIP

        // Restore user stack and return
        "mov rsp, gs:[0x8]",        // Restore user RSP
        "swapgs",                    // Switch back to user GS
        "sysretq",

        handler = sym syscall_handler
    );
}

/// Initialize SYSCALL/SYSRET support
#[allow(dead_code)]
pub fn init_syscall() {
    use x86_64::registers::{
        model_specific::{Efer, EferFlags, LStar, Star},
        segmentation::SegmentSelector,
    };

    unsafe {
        // Enable SYSCALL/SYSRET
        Efer::update(|flags| {
            flags.insert(EferFlags::SYSTEM_CALL_EXTENSIONS);
        });

        // Set up SYSCALL entry point
        LStar::write(x86_64::VirtAddr::new(syscall_entry as usize as u64));

        // Set up segment selectors
        // Star::write takes 4 arguments:
        // 1. User CS (for SYSRET)
        // 2. User SS (for SYSRET)
        // 3. Kernel CS (for SYSCALL)
        // 4. Kernel SS (for SYSCALL)
        Star::write(
            SegmentSelector(0x18), // User CS (ring 3)
            SegmentSelector(0x20), // User SS (ring 3)
            SegmentSelector(0x08), // Kernel CS (ring 0)
            SegmentSelector(0x10), // Kernel SS (ring 0)
        )
        .unwrap();
    }
}
