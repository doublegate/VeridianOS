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
    // For AArch64, use uart_write_str for panic message
    #[cfg(target_arch = "aarch64")]
    unsafe {
        use arch::aarch64::direct_uart::uart_write_str;
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
    // Initialize early serial for x86_64 before any println! usage
    #[cfg(target_arch = "x86_64")]
    {
        arch::x86_64::early_serial::init();
        early_println!("[EARLY] x86_64 kernel_main reached!");
        early_println!("[EARLY] VeridianOS Kernel v{}", env!("CARGO_PKG_VERSION"));
        early_println!("[EARLY] Architecture: x86_64");
    }
    
    // AArch64: Use uart_write_str for descriptive messages
    #[cfg(target_arch = "aarch64")]
    {
        use arch::aarch64::direct_uart::uart_write_str;
        
        unsafe {
            uart_write_str("[KERNEL] AArch64 kernel_main reached successfully\n");
            uart_write_str("[KERNEL] VeridianOS Kernel v");
            uart_write_str(env!("CARGO_PKG_VERSION"));
            uart_write_str("\n");
            uart_write_str("[KERNEL] Architecture: AArch64\n");
            uart_write_str("[KERNEL] Starting kernel initialization...\n");
        }
    }
    
    // For non-x86_64, non-AArch64 architectures
    #[cfg(all(not(target_arch = "aarch64"), not(target_arch = "x86_64")))]
    {
        println!("VeridianOS Kernel v{}", env!("CARGO_PKG_VERSION"));
        #[cfg(target_arch = "riscv64")]
        println!("Architecture: riscv64");
    }

    // Use unified bootstrap initialization for all architectures
    #[cfg(target_arch = "aarch64")]
    {
        use arch::aarch64::direct_uart::uart_write_str;
        
        unsafe {
            uart_write_str("[KERNEL] Starting kernel initialization...\n");
            uart_write_str("[KERNEL] Initializing bootstrap sequence...\n");
        }
    }
    
    #[cfg(target_arch = "x86_64")]
    early_println!("[EARLY] Starting bootstrap initialization...");
    
    // Call unified bootstrap for all architectures
    match bootstrap::kernel_init() {
        Ok(()) => {
            // Bootstrap will transfer control to scheduler
            // This should not return
            #[cfg(target_arch = "x86_64")]
            early_println!("[EARLY] Bootstrap returned unexpectedly!");
            
            #[cfg(target_arch = "aarch64")]
            unsafe {
                use arch::aarch64::direct_uart::uart_write_str;
                uart_write_str("[KERNEL] Bootstrap returned unexpectedly!\n");
            }
            
            panic!("[KERNEL] Bootstrap returned unexpectedly!");
        }
        Err(e) => {
            #[cfg(target_arch = "x86_64")]
            early_println!("[EARLY] Bootstrap initialization failed!");
            
            #[cfg(target_arch = "aarch64")]
            unsafe {
                use arch::aarch64::direct_uart::uart_write_str;
                uart_write_str("[KERNEL] Bootstrap initialization failed!\n");
            }
            
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
