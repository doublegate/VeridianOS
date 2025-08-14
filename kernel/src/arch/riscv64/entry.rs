// RISC-V kernel entry point and panic handler

use core::panic::PanicInfo;
use crate::println;

pub fn arch_early_init() {
    println!("VeridianOS Kernel v{}", env!("CARGO_PKG_VERSION"));
    println!("Architecture: riscv64");
}

pub fn arch_panic_handler(_info: &PanicInfo) {
    println!("[KERNEL PANIC] {}", _info);
}