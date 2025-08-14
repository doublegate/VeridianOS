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
#![feature(naked_functions)]
#![test_runner(crate::test_runner)]
#![reexport_test_harness_main = "test_main"]
#![doc(
    html_logo_url = "https://raw.githubusercontent.com/doublegate/VeridianOS/main/docs/assets/logo.png",
    html_favicon_url = "https://raw.githubusercontent.com/doublegate/VeridianOS/main/images/veridian_os.ico",
    issue_tracker_base_url = "https://github.com/doublegate/VeridianOS/issues/"
)]

use core::panic::PanicInfo;

// Global allocator is defined in lib.rs

// Use the kernel library
use veridian_kernel::*;

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

#[no_mangle]
pub extern "C" fn kernel_main() -> ! {
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

    // Bootstrap should not return
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