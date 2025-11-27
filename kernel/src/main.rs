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

// For x86_64, use bootloader_api 0.11+ for entry point and boot info
#[cfg(target_arch = "x86_64")]
use bootloader_api::{config::Mapping, entry_point, BootInfo, BootloaderConfig};
// Global allocator is defined in lib.rs

// Use the kernel library
use veridian_kernel::*;

/// Bootloader configuration for x86_64
/// Maps physical memory at a fixed address for kernel access
#[cfg(target_arch = "x86_64")]
const BOOTLOADER_CONFIG: BootloaderConfig = {
    let mut config = BootloaderConfig::new_default();
    // Map physical memory for kernel access (required for page table management)
    config.mappings.physical_memory = Some(Mapping::Dynamic);
    config.kernel_stack_size = 128 * 1024; // 128 KiB kernel stack
    config
};

#[cfg(target_arch = "x86_64")]
entry_point!(x86_64_kernel_entry, config = &BOOTLOADER_CONFIG);

#[cfg(target_arch = "x86_64")]
fn x86_64_kernel_entry(boot_info: &'static mut BootInfo) -> ! {
    // First thing: try direct serial output to prove we're running
    // This uses I/O ports which don't require memory mapping
    unsafe {
        // Initialize serial port at 0x3F8 (COM1)
        let base: u16 = 0x3F8;

        // Disable interrupts
        core::arch::asm!("out dx, al", in("dx") base + 1, in("al") 0u8, options(nomem, nostack, preserves_flags));
        // Enable DLAB
        core::arch::asm!("out dx, al", in("dx") base + 3, in("al") 0x80u8, options(nomem, nostack, preserves_flags));
        // Set divisor to 3 (38400 baud)
        core::arch::asm!("out dx, al", in("dx") base, in("al") 0x03u8, options(nomem, nostack, preserves_flags));
        core::arch::asm!("out dx, al", in("dx") base + 1, in("al") 0x00u8, options(nomem, nostack, preserves_flags));
        // 8 bits, no parity, one stop bit
        core::arch::asm!("out dx, al", in("dx") base + 3, in("al") 0x03u8, options(nomem, nostack, preserves_flags));
        // Enable FIFO
        core::arch::asm!("out dx, al", in("dx") base + 2, in("al") 0xC7u8, options(nomem, nostack, preserves_flags));
        // Enable IRQs, RTS/DSR
        core::arch::asm!("out dx, al", in("dx") base + 4, in("al") 0x0Bu8, options(nomem, nostack, preserves_flags));

        // Output "KERNEL_ENTRY\n"
        for &b in b"KERNEL_ENTRY\n" {
            // Wait for transmit buffer
            loop {
                let status: u8;
                core::arch::asm!("in al, dx", out("al") status, in("dx") base + 5, options(nomem, nostack, preserves_flags));
                if (status & 0x20) != 0 {
                    break;
                }
            }
            core::arch::asm!("out dx, al", in("dx") base, in("al") b, options(nomem, nostack, preserves_flags));
        }
    }

    // Helper to output debug strings via serial
    fn serial_puts(s: &[u8]) {
        unsafe {
            let base: u16 = 0x3F8;
            for &b in s {
                loop {
                    let status: u8;
                    core::arch::asm!("in al, dx", out("al") status, in("dx") base + 5, options(nomem, nostack, preserves_flags));
                    if (status & 0x20) != 0 {
                        break;
                    }
                }
                core::arch::asm!("out dx, al", in("dx") base, in("al") b, options(nomem, nostack, preserves_flags));
            }
        }
    }

    // Try to write to VGA using the physical memory offset from bootloader
    serial_puts(b"PHYS_MEM...");
    if let Some(phys_mem_offset) = boot_info.physical_memory_offset.into_option() {
        serial_puts(b"OK\n");
        unsafe {
            let vga_addr = phys_mem_offset + 0xb8000;
            let vga = vga_addr as *mut u16;
            vga.write_volatile(0x0F45); // 'E' in white on black
            vga.offset(1).write_volatile(0x0F4E); // 'N' for entry
            vga.offset(2).write_volatile(0x0F54); // 'T' for entry
            vga.offset(3).write_volatile(0x0F52); // 'R' for entry
            vga.offset(4).write_volatile(0x0F59); // 'Y'
        }
    } else {
        serial_puts(b"NONE\n");
    }

    // Skip early_boot_init for now since serial is already initialized
    // Run early boot initialization (serial port, etc.)
    // arch::x86_64::boot::early_boot_init();
    serial_puts(b"BOOT_INFO...");

    // Store boot info for later use
    unsafe {
        arch::x86_64::boot::BOOT_INFO = Some(boot_info);
    }
    serial_puts(b"OK\n");

    serial_puts(b"KERNEL_MAIN...\n");
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
