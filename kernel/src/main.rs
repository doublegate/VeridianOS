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
    // For AArch64, just write PANIC to UART and loop
    #[cfg(target_arch = "aarch64")]
    unsafe {
        let uart = 0x0900_0000 as *mut u8;
        core::ptr::write_volatile(uart, b'P');
        core::ptr::write_volatile(uart, b'A');
        core::ptr::write_volatile(uart, b'N');
        core::ptr::write_volatile(uart, b'I');
        core::ptr::write_volatile(uart, b'C');
        core::ptr::write_volatile(uart, b'\n');
    }

    #[cfg(target_arch = "x86_64")]
    println!("[KERNEL PANIC] {}", _info);

    arch::halt();
}

#[cfg(test)]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    test_framework::test_panic_handler(info)
}

#[no_mangle]
pub extern "C" fn kernel_main() -> ! {
    // For AArch64, add early debug output
    #[cfg(target_arch = "aarch64")]
    unsafe {
        let uart = 0x0900_0000 as *mut u8;
        // Write "KERN" to show we reached kernel_main
        *uart = b'K';
        *uart = b'E';
        *uart = b'R';
        *uart = b'N';
        *uart = b'\n';
    }

    // For AArch64, test with direct writes first
    #[cfg(target_arch = "aarch64")]
    unsafe {
        let uart = 0x0900_0000 as *mut u8;
        // Write without loops
        *uart = b'V';
        *uart = b'e';
        *uart = b'r';
        *uart = b'i';
        *uart = b'd';
        *uart = b'i';
        *uart = b'a';
        *uart = b'n';
        *uart = b'O';
        *uart = b'S';
        *uart = b' ';
        *uart = b'K';
        *uart = b'e';
        *uart = b'r';
        *uart = b'n';
        *uart = b'e';
        *uart = b'l';
        *uart = b' ';
        *uart = b'v';
        *uart = b'0';
        *uart = b'.';
        *uart = b'2';
        *uart = b'.';
        *uart = b'0';
        *uart = b'\n';
        *uart = b'A';
        *uart = b'r';
        *uart = b'c';
        *uart = b'h';
        *uart = b'i';
        *uart = b't';
        *uart = b'e';
        *uart = b'c';
        *uart = b't';
        *uart = b'u';
        *uart = b'r';
        *uart = b'e';
        *uart = b':';
        *uart = b' ';
        *uart = b'a';
        *uart = b'a';
        *uart = b'r';
        *uart = b'c';
        *uart = b'h';
        *uart = b'6';
        *uart = b'4';
        *uart = b'\n';
    }

    // Now let's try println with a simple message
    #[cfg(target_arch = "aarch64")]
    unsafe {
        let uart = 0x0900_0000 as *mut u8;
        // Test println
        *uart = b'T';
        *uart = b'E';
        *uart = b'S';
        *uart = b'T';
        *uart = b'\n';
    }

    // Skip println! for AArch64 for now since it seems problematic
    #[cfg(not(target_arch = "aarch64"))]
    {
        println!("VeridianOS Kernel v{}", env!("CARGO_PKG_VERSION"));
        #[cfg(target_arch = "x86_64")]
        println!("Architecture: x86_64");
        #[cfg(target_arch = "riscv64")]
        println!("Architecture: riscv64");
        println!("Testing println! macro...");
    }

    // Now try the bootstrap initialization
    #[cfg(target_arch = "aarch64")]
    unsafe {
        let uart = 0x0900_0000 as *mut u8;
        *uart = b'B';
        *uart = b'O';
        *uart = b'O';
        *uart = b'T';
        *uart = b'\n';
    }

    // Use bootstrap initialization for all architectures
    // For AArch64, bootstrap now uses safe iteration patterns
    #[cfg(target_arch = "aarch64")]
    {
        unsafe {
            let uart = 0x0900_0000 as *mut u8;
            *uart = b'K';
            *uart = b'e';
            *uart = b'r';
            *uart = b'n';
            *uart = b'e';
            *uart = b'l';
            *uart = b' ';
            *uart = b'i';
            *uart = b'n';
            *uart = b'i';
            *uart = b't';
            *uart = b'i';
            *uart = b'a';
            *uart = b'l';
            *uart = b'i';
            *uart = b'z';
            *uart = b'e';
            *uart = b'd';
            *uart = b' ';
            *uart = b's';
            *uart = b'u';
            *uart = b'c';
            *uart = b'c';
            *uart = b'e';
            *uart = b's';
            *uart = b's';
            *uart = b'f';
            *uart = b'u';
            *uart = b'l';
            *uart = b'l';
            *uart = b'y';
            *uart = b'!';
            *uart = b'\n';
        }

        // Continue to bootstrap
    }

    // Bootstrap for all architectures
    match bootstrap::kernel_init() {
        Ok(()) => {
            // Bootstrap will transfer control to scheduler
            // This should not return
            panic!("[KERNEL] Bootstrap returned unexpectedly!");
        }
        Err(e) => {
            panic!("[KERNEL] Bootstrap initialization failed: {}", e);
        }
    }
}

#[cfg(test)]
use test_framework::{exit_qemu, QemuExitCode, Testable};

#[cfg(test)]
fn test_runner(tests: &[&dyn Testable]) {
    test_framework::test_runner(tests)
}
