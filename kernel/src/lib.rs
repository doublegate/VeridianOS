//! VeridianOS Kernel Library
//!
//! This library provides the core functionality for the VeridianOS kernel
//! and exports necessary items for testing.

#![no_std]
#![cfg_attr(test, no_main)]
#![feature(custom_test_frameworks)]
#![feature(abi_x86_interrupt)]
#![feature(alloc_error_handler)]
#![feature(naked_functions)]
#![test_runner(crate::test_runner)]
#![reexport_test_harness_main = "test_main"]

#[cfg(feature = "alloc")]
extern crate alloc;

use linked_list_allocator::LockedHeap;

#[global_allocator]
static ALLOCATOR: LockedHeap = LockedHeap::empty();

/// Get a reference to the global allocator
pub fn get_allocator() -> &'static LockedHeap {
    &ALLOCATOR
}

#[macro_use]
pub mod print;

pub mod arch;
pub mod bootstrap;
mod cap;
pub mod error;
pub mod ipc;
pub mod mm;
pub mod process;
pub mod raii;
pub mod sched;
pub mod serial;
mod syscall;

#[cfg(test)]
mod test_config;
mod test_framework;

#[cfg(test)]
mod raii_tests;

pub mod bench;

// Re-export for tests and benchmarks
// Re-export memory management for tests
pub use mm::{FrameNumber, MemoryRegion, FRAME_SIZE};
// Re-export scheduler items for tests
pub use sched::{Priority, SchedClass, Task};
#[cfg(test)]
pub use test_framework::test_runner;
pub use test_framework::{
    cycles_to_ns, exit_qemu, read_timestamp, test_panic_handler, BenchmarkRunner, QemuExitCode,
    Testable,
};

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
    // For AArch64, let's use direct UART writes until println! is stable
    #[cfg(target_arch = "aarch64")]
    unsafe {
        let uart = 0x0900_0000 as *mut u8;

        // Write "MAIN" to confirm we reached kernel_main
        *uart = b'M';
        *uart = b'A';
        *uart = b'I';
        *uart = b'N';
        *uart = b'\n';

        // Write a simple message directly - avoid iterators
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

        // Write "DONE" to confirm we finished
        *uart = b'D';
        *uart = b'O';
        *uart = b'N';
        *uart = b'E';
        *uart = b'\n';
    }

    #[cfg(not(target_arch = "aarch64"))]
    {
        println!("VeridianOS Kernel v{}", env!("CARGO_PKG_VERSION"));
        #[cfg(target_arch = "x86_64")]
        println!("Architecture: x86_64");
        #[cfg(target_arch = "riscv64")]
        println!("Architecture: riscv64");
        println!("Kernel initialized successfully!");
    }

    loop {
        core::hint::spin_loop();
    }
}

#[cfg(test)]
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    test_framework::test_panic_handler(info)
}

/// Heap allocation error handler
#[alloc_error_handler]
fn alloc_error_handler(layout: core::alloc::Layout) -> ! {
    panic!("Allocation error: {:?}", layout);
}
