// AArch64 kernel entry point and panic handler

use core::panic::PanicInfo;

pub fn arch_early_init() {
    use crate::arch::aarch64::direct_uart::uart_write_str;

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
