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

#[macro_use]
mod print;

mod arch;
mod cap;
pub mod ipc;
pub mod mm;
pub mod process;
pub mod sched;
pub mod serial;
mod syscall;

mod test_framework;

pub mod bench;

// Re-export for tests and benchmarks
// Re-export memory management for tests
pub use mm::{FrameNumber, MemoryRegion, FRAME_SIZE};
// Re-export scheduler items for tests
pub use sched::{Priority, SchedClass, Task};
pub use test_framework::{
    cycles_to_ns, exit_qemu, read_timestamp, test_panic_handler, test_runner, BenchmarkRunner,
    QemuExitCode, Testable,
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

/// Heap allocation error handler
#[alloc_error_handler]
fn alloc_error_handler(layout: core::alloc::Layout) -> ! {
    panic!("Allocation error: {:?}", layout);
}
