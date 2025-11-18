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

#[cfg(not(target_arch = "riscv64"))]
use linked_list_allocator::LockedHeap;

#[cfg(target_arch = "riscv64")]
mod simple_alloc_unsafe;
#[cfg(target_arch = "riscv64")]
use simple_alloc_unsafe::{LockedUnsafeBumpAllocator, UnsafeBumpAllocator};

#[cfg(not(target_arch = "riscv64"))]
#[global_allocator]
static ALLOCATOR: LockedHeap = LockedHeap::empty();

#[cfg(target_arch = "riscv64")]
#[global_allocator]
pub static ALLOCATOR: UnsafeBumpAllocator = UnsafeBumpAllocator::new();

#[cfg(target_arch = "riscv64")]
static LOCKED_ALLOCATOR: LockedUnsafeBumpAllocator = LockedUnsafeBumpAllocator::empty();

/// Get a reference to the global allocator
#[cfg(not(target_arch = "riscv64"))]
pub fn get_allocator() -> &'static LockedHeap {
    &ALLOCATOR
}

/// Get a reference to the global allocator for RISC-V
#[cfg(target_arch = "riscv64")]
pub fn get_allocator() -> &'static LockedUnsafeBumpAllocator {
    &LOCKED_ALLOCATOR
}

#[macro_use]
pub mod print;

mod intrinsics;

pub mod arch;
pub mod bootstrap;
mod cap;
pub mod elf;
pub mod error;
pub mod fs;
pub mod ipc;
pub mod mm;
pub mod process;
pub mod raii;
pub mod sched;
pub mod serial;
pub mod services;
pub mod stdlib;
mod syscall;
pub mod test_tasks;
pub mod thread_api;
pub mod userspace;
pub mod drivers;
pub mod userland;
pub mod phase2_validation;
pub mod security;
pub mod pkg;
pub mod perf;
pub mod graphics;
pub mod net;
pub mod desktop;

#[cfg(test)]
mod test_config;
mod test_framework;

#[cfg(test)]
mod raii_tests;

#[cfg(test)]
mod integration_tests;

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
