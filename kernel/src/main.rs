//! # VeridianOS Microkernel
//!
//! A next-generation microkernel operating system written in Rust.
//!
//! ## Architecture Support
//!
//! - x86_64 - Full support with UEFI/BIOS boot
//! - AArch64 - Full support with device tree
//! - RISC-V - Full support with OpenSBI
//!
//! ## Key Components
//!
//! - [`mm`] - Memory management subsystem
//! - [`sched`] - Process scheduling
//! - [`ipc`] - Inter-process communication
//! - [`cap`] - Capability-based security
//!
//! ## Safety
//!
//! This is kernel code - most functions are `unsafe` and require careful
//! handling. See individual module documentation for specific requirements.

#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![feature(abi_x86_interrupt)]
// naked_functions is stable since Rust 1.88.0, no feature flag needed
#![test_runner(crate::test_runner)]
#![reexport_test_harness_main = "test_main"]
#![doc(
    html_logo_url = "https://raw.githubusercontent.com/doublegate/VeridianOS/main/docs/assets/logo.png",
    html_favicon_url = "https://raw.githubusercontent.com/doublegate/VeridianOS/main/images/veridian_os.ico",
    issue_tracker_base_url = "https://github.com/doublegate/VeridianOS/issues/"
)]

use core::panic::PanicInfo;

// For x86_64, use bootloader 0.9 for working boot
#[cfg(target_arch = "x86_64")]
use bootloader::{entry_point, BootInfo};
// Global allocator is defined in lib.rs

// Use the kernel library
use veridian_kernel::*;

#[cfg(target_arch = "x86_64")]
entry_point!(x86_64_kernel_entry);

#[cfg(target_arch = "x86_64")]
fn x86_64_kernel_entry(boot_info: &'static BootInfo) -> ! {
    // Write 'E' to VGA to show we reached entry point
    unsafe {
        let vga = 0xb8000 as *mut u16;
        vga.write_volatile(0x0F45); // 'E' in white on black
        vga.offset(1).write_volatile(0x0F42); // 'B' to show boot.rs was called

        // Also try direct serial output here
        let port: u16 = 0x3F8;
        // Simple serial byte output
        core::arch::asm!(
            "out dx, al",
            in("dx") port,
            in("al") b'X',
            options(nomem, nostack, preserves_flags)
        );
    }

    // Run early boot initialization (serial port, etc.)
    arch::x86_64::boot::early_boot_init();

    // Store boot info for later use
    unsafe {
        arch::x86_64::boot::BOOT_INFO = Some(boot_info);
    }

    // Call the main kernel implementation
    kernel_main_impl()
}

#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    // Use architecture-specific panic handler
    #[cfg(target_arch = "x86_64")]
    arch::x86_64::entry::arch_panic_handler(_info);

    #[cfg(target_arch = "aarch64")]
    arch::aarch64::entry::arch_panic_handler(_info);

    #[cfg(target_arch = "riscv64")]
    arch::riscv64::entry::arch_panic_handler(_info);

    arch::halt();
}

#[cfg(test)]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    test_framework::test_panic_handler(info)
}

// For non-x86_64 architectures, kernel_main is called from assembly boot code
#[cfg(not(target_arch = "x86_64"))]
#[no_mangle]
pub extern "C" fn kernel_main() -> ! {
    // Very early RISC-V debug output
    #[cfg(target_arch = "riscv64")]
    unsafe {
        // Direct write to UART at 0x10000000
        let uart_base = 0x1000_0000 as *mut u8;
        // Write 'V' for VeridianOS
        uart_base.write_volatile(b'V');
        uart_base.write_volatile(b'E');
        uart_base.write_volatile(b'R');
        uart_base.write_volatile(b'I');
        uart_base.write_volatile(b'\n');
    }

    kernel_main_impl()
}

// For x86_64, kernel_main_impl is called directly from x86_64_kernel_entry

#[allow(unreachable_code)]
fn kernel_main_impl() -> ! {
    // Architecture-specific early initialization
    #[cfg(target_arch = "x86_64")]
    arch::x86_64::entry::arch_early_init();

    #[cfg(target_arch = "aarch64")]
    arch::aarch64::entry::arch_early_init();

    #[cfg(target_arch = "riscv64")]
    arch::riscv64::entry::arch_early_init();

    // Use unified bootstrap initialization
    #[cfg(target_arch = "x86_64")]
    early_println!("[EARLY] Starting bootstrap initialization...");

    #[cfg(not(target_arch = "x86_64"))]
    boot_println!("[EARLY] Starting bootstrap initialization...");

    // Run bootstrap
    bootstrap::run();

    // Bootstrap should not return (unreachable but kept for safety)
    panic!("Bootstrap returned unexpectedly!");
}

// Test runner for kernel tests
#[cfg(test)]
fn test_runner(tests: &[&dyn Fn()]) {
    println!("Running {} tests", tests.len());
    for test in tests {
        test();
    }
}

// Export key functions for tests
#[cfg(test)]
pub use test_framework::*;
