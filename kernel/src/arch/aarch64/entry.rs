//! AArch64 kernel entry point and panic handler.
//!
//! Provides `arch_early_init` for early architecture setup and
//! `arch_panic_handler` for kernel panic output via direct UART.

use core::panic::PanicInfo;

pub fn arch_early_init() {
    use crate::arch::aarch64::direct_uart::uart_write_str;

    // SAFETY: uart_write_str performs raw MMIO writes to the PL011 UART at
    // 0x09000000 using assembly to bypass LLVM loop compilation issues on
    // AArch64. The UART is memory-mapped by QEMU's virt machine and writing
    // is non-destructive. Called during early single-threaded boot.
    unsafe {
        uart_write_str("[KERNEL] AArch64 kernel_main reached successfully\n");
        uart_write_str("[KERNEL] VeridianOS Kernel v");
        uart_write_str(env!("CARGO_PKG_VERSION"));
        uart_write_str("\n");
        uart_write_str("[KERNEL] Architecture: AArch64\n");
        uart_write_str("[KERNEL] Starting kernel initialization...\n");
    }
}

pub fn arch_panic_handler(_info: &PanicInfo) {
    use crate::arch::aarch64::direct_uart::uart_write_str;

    // SAFETY: uart_write_str performs raw MMIO writes to the PL011 UART at
    // 0x09000000. Safe during panic as only diagnostic output is produced.
    unsafe {
        uart_write_str("\n[PANIC] Kernel panic occurred!\n");

        // Try to extract panic message location if available
        if let Some(location) = _info.location() {
            uart_write_str("[PANIC] Location: ");
            uart_write_str(location.file());
            uart_write_str(":");
            // Can't easily print line number without loops, so skip for now
            uart_write_str("\n");
        }
    }
}
