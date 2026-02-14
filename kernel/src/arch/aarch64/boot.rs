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
    uart_write_str("[BOOT] AArch64 Rust entry point reached\n");
    uart_write_str("[BOOT] Stack initialized and BSS cleared\n");
    uart_write_str("[BOOT] Preparing to enter kernel_main...\n");

    // Call kernel_main from main.rs
    extern "C" {
        fn kernel_main() -> !;
    }
    kernel_main()
}
