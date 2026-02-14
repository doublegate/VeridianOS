//! RISC-V kernel entry point and panic handler.
//!
//! Provides `arch_early_init` for early architecture-specific setup and
//! `arch_panic_handler` for kernel panic output via SBI console.

use core::panic::PanicInfo;

#[allow(unused_imports)]
use crate::println;

pub fn arch_early_init() {
    println!("VeridianOS Kernel v{}", env!("CARGO_PKG_VERSION"));
    println!("Architecture: riscv64");

    // SAFETY: sbi_putchar invokes the SBI legacy console putchar (ecall with
    // a7=0x01). This is the standard RISC-V mechanism for early boot console
    // output before the UART driver is initialized. Always safe to call.
    unsafe {
        // SBI console putchar
        sbi_putchar(b'I');
        sbi_putchar(b'N');
        sbi_putchar(b'I');
        sbi_putchar(b'T');
        sbi_putchar(b'\n');
    }
}

/// SBI console putchar using ecall
#[inline]
unsafe fn sbi_putchar(ch: u8) {
    core::arch::asm!(
        "ecall",
        in("a0") ch as usize,     // Character to print
        in("a7") 0x01usize,       // SBI function ID 0x01 = console_putchar (legacy)
        options(nostack, nomem)
    );
}

pub fn arch_panic_handler(_info: &PanicInfo) {
    println!("[KERNEL PANIC] {}", _info);
}
