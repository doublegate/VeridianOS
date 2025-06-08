//! VeridianOS Kernel Library
//!
//! This library provides the core functionality for the VeridianOS kernel
//! and exports necessary items for testing.

#![no_std]
#![cfg_attr(test, no_main)]
#![feature(custom_test_frameworks)]
#![feature(abi_x86_interrupt)]
#![test_runner(crate::test_runner)]
#![reexport_test_harness_main = "test_main"]

use linked_list_allocator::LockedHeap;

#[global_allocator]
static ALLOCATOR: LockedHeap = LockedHeap::empty();

#[macro_use]
mod print;

mod arch;
mod cap;
mod ipc;
mod mm;
mod sched;
pub mod serial;

mod test_framework;

pub mod bench;

// Re-export for tests and benchmarks
pub use test_framework::{exit_qemu, test_panic_handler, test_runner, QemuExitCode, Testable};

#[cfg(test)]
#[no_mangle]
pub extern "C" fn _start() -> ! {
    test_main();
    loop {
        core::hint::spin_loop();
    }
}

// Kernel main function for normal boot
pub fn kernel_main() -> ! {
    println!("VeridianOS Kernel v{}", env!("CARGO_PKG_VERSION"));
    #[cfg(target_arch = "x86_64")]
    println!("Architecture: x86_64");
    #[cfg(target_arch = "aarch64")]
    println!("Architecture: aarch64");
    #[cfg(target_arch = "riscv64")]
    println!("Architecture: riscv64");
    println!("Kernel initialized successfully!");

    loop {
        core::hint::spin_loop();
    }
}

#[cfg(test)]
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    test_framework::test_panic_handler(info)
}
