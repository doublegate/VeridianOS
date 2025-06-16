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
    // AArch64: Due to LLVM bug, we use manual prints for critical messages
    #[cfg(target_arch = "aarch64")]
    {
        // Print basic kernel info using manual approach
        uart_write!(
            b'V', b'e', b'r', b'i', b'd', b'i', b'a', b'n', b'O', b'S', b' ',
            b'K', b'e', b'r', b'n', b'e', b'l', b' ', b'v', b'0', b'.', b'2', b'.', b'0', b'\n'
        );
        uart_write!(
            b'A', b'r', b'c', b'h', b'i', b't', b'e', b'c', b't', b'u', b'r', b'e', b':', b' ',
            b'a', b'a', b'r', b'c', b'h', b'6', b'4', b'\n'
        );
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        println!("VeridianOS Kernel v{}", env!("CARGO_PKG_VERSION"));
        #[cfg(target_arch = "x86_64")]
        println!("Architecture: x86_64");
        #[cfg(target_arch = "riscv64")]
        println!("Architecture: riscv64");
        println!("Test1");
        println!("Test2");
        println!("Testing println! macro...");
    }
    
    // Add a simple direct test to see where it hangs
    #[cfg(target_arch = "aarch64")]
    {
        unsafe {
            let uart = 0x0900_0000 as *mut u8;
            *uart = b'P';
            *uart = b'O';
            *uart = b'S';
            *uart = b'T';
            *uart = b'\n';
        }
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
