// x86_64 kernel entry point and panic handler

use core::panic::PanicInfo;
use crate::{early_println, println};

pub fn arch_early_init() {
    // Disable interrupts immediately
    unsafe {
        core::arch::asm!("cli", options(nomem, nostack));
    }
    
    // Initialize early serial before any println! usage
    crate::arch::x86_64::early_serial::init();
    early_println!("[EARLY] x86_64 kernel_main reached!");
    early_println!("[EARLY] VeridianOS Kernel v{}", env!("CARGO_PKG_VERSION"));
    early_println!("[EARLY] Architecture: x86_64");
}

pub fn arch_panic_handler(info: &PanicInfo) {
    println!("[KERNEL PANIC] {}", info);
}