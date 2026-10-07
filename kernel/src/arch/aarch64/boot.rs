//! AArch64 boot entry point.
//!
//! Includes the assembly startup code (`boot.S`) and the Rust `_start_rust`
//! entry that prints an early banner via direct UART and calls `kernel_main`.

use core::arch::global_asm;

// Include the assembly boot code
global_asm!(include_str!("boot.S"));

/// Entry point from assembly code
///
/// # Safety
///
/// This function is called from assembly with:
/// - Stack properly initialized
/// - BSS section cleared
/// - Running in EL1 with MMU disabled
#[no_mangle]
#[link_section = ".text.boot"]
pub unsafe extern "C" fn _start_rust() -> ! {
    // Use direct_uart for proper string output
    use crate::arch::aarch64::direct_uart::uart_write_str;

    // Write startup messages
    // SAFETY: with the MMU disabled (this function's contract) physical
    // address 0x09000000 is directly accessible, and it is the PL011 UART on
    // the QEMU virt machine this boot path targets.
    unsafe {
        uart_write_str("[BOOT] AArch64 Rust entry point reached\n");
        uart_write_str("[BOOT] Stack initialized and BSS cleared\n");
        uart_write_str("[BOOT] Preparing to enter kernel_main...\n");
    }

    // The boot CPU is logical CPU 0; TPIDR_EL1 now points at its per-CPU
    // block. Its hardware id is the MPIDR affinity (Aff0..Aff2).
    let mpidr: u64;
    // SAFETY: reading MPIDR_EL1 at EL1 has no side effects.
    unsafe { core::arch::asm!("mrs {}, mpidr_el1", out(reg) mpidr, options(nomem, nostack)) };
    // SAFETY: first Rust code on the boot CPU, before anything reads
    // TPIDR_EL1.
    unsafe { crate::arch::percpu::install(0, (mpidr & 0x00FF_FFFF) as u32) };

    // Arm boot-stack overflow detection before anything deep runs.
    crate::arch::stack_canary::install();

    // Call kernel_main from main.rs
    extern "C" {
        fn kernel_main() -> !;
    }
    // SAFETY: kernel_main is the kernel's `extern "C" fn() -> !` entry point
    // defined in main.rs; it takes no arguments and expects exactly the
    // state this function's contract provides (stack set up, BSS cleared,
    // EL1).
    unsafe { kernel_main() }
}
