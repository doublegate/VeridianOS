//! VeridianOS Kernel Library
//!
//! This library provides the core functionality for the VeridianOS kernel
//! and exports necessary items for testing.

#![no_std]
#![cfg_attr(all(test, target_os = "none"), no_main)]
#![feature(custom_test_frameworks)]
#![cfg_attr(target_os = "none", feature(alloc_error_handler))]
// naked_functions is stable since Rust 1.88.0, no feature flag needed
// Custom test runner only for bare-metal; host target uses standard #[test] harness.
#![cfg_attr(target_os = "none", test_runner(crate::test_runner))]
#![cfg_attr(target_os = "none", reexport_test_harness_main = "test_main")]

#[cfg(feature = "alloc")]
extern crate alloc;

// On bare-metal targets use the custom kernel heap allocators.
// On host (x86_64-unknown-linux-gnu) for coverage/testing, delegate to the
// system allocator so that test code using Vec/String/alloc compiles and runs.
#[cfg(all(
    any(target_arch = "x86_64", target_arch = "riscv64"),
    target_os = "none"
))]
use linked_list_allocator::LockedHeap;

// The bump allocator AArch64 uses until its MMU is on (N-28: locks need
// exclusives, unreliable on Device memory). Also built for host unit tests,
// which exercise it on a Vec-backed heap.
#[cfg(any(target_arch = "aarch64", test))]
#[cfg_attr(test, allow(dead_code))]
mod simple_alloc_unsafe;
#[cfg(target_arch = "aarch64")]
use simple_alloc_unsafe::UnsafeBumpAllocator;

/// x86_64 and RISC-V: a real heap that reuses freed memory (MEM-SEC-03).
#[cfg(all(
    any(target_arch = "x86_64", target_arch = "riscv64"),
    target_os = "none"
))]
#[global_allocator]
static ALLOCATOR: LockedHeap = LockedHeap::empty();

#[cfg(target_arch = "aarch64")]
#[global_allocator]
pub static ALLOCATOR: UnsafeBumpAllocator = UnsafeBumpAllocator::new();

// Host target: use the system allocator so unit tests can allocate normally.
#[cfg(not(target_os = "none"))]
extern crate std;
#[cfg(not(target_os = "none"))]
#[global_allocator]
static SYSTEM_ALLOCATOR: std::alloc::System = std::alloc::System;

/// Get a reference to the global allocator
#[cfg(all(
    any(target_arch = "x86_64", target_arch = "riscv64"),
    target_os = "none"
))]
pub fn get_allocator() -> &'static LockedHeap {
    &ALLOCATOR
}

#[macro_use]
pub mod print;
pub mod print_capture;

mod intrinsics;

pub mod arch;
pub mod audio;
pub mod bootstrap;
mod cap;
pub mod crypto;
pub mod desktop;
pub mod drivers;
pub mod elf;
pub mod error;
pub mod fs;
pub mod graphics;
pub mod ipc;
pub mod irq;
pub mod log_service;
pub mod media;
pub mod mm;
pub mod net;
pub mod perf;
pub mod phase2_validation;
pub mod pkg;
pub mod power;
pub mod process;
pub mod raii;
pub mod sched;
pub mod security;
pub mod serial;
pub mod services;
pub mod sync;
mod syscall;
pub mod sysfs;
pub mod test_tasks;
pub mod thread_api;
pub mod timer;
pub mod userland;
pub mod userspace;
pub mod utils;
pub mod video;
pub mod virt;

#[allow(dead_code)]
#[cfg(feature = "alloc")]
pub mod browser;
#[allow(dead_code)]
#[cfg(feature = "alloc")]
pub mod debug;
#[allow(dead_code)]
#[cfg(feature = "alloc")]
pub mod devtools;
#[allow(dead_code)]
#[cfg(feature = "alloc")]
pub mod verification;

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
// Re-exported so the `UserPod` compile_fail doctest can name the accessor;
// `syscall` itself stays private.
#[doc(hidden)]
pub use syscall::userspace::{read_user, UserPod};
#[cfg(test)]
pub use test_framework::test_runner;
pub use test_framework::{
    cycles_to_ns, exit_qemu, read_timestamp, test_panic_handler, BenchmarkRunner, QemuExitCode,
    Testable,
};

#[cfg(all(test, target_os = "none"))]
#[no_mangle]
pub extern "C" fn _start() -> ! {
    test_main();
    loop {
        core::hint::spin_loop();
    }
}

#[cfg(all(test, target_os = "none"))]
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    test_framework::test_panic_handler(info)
}

/// Heap allocation error handler.
///
/// Panic is intentional: heap allocation failure in a no_std kernel is
/// unrecoverable. The alloc_error_handler ABI requires `-> !`.
#[cfg(target_os = "none")]
#[alloc_error_handler]
fn alloc_error_handler(layout: core::alloc::Layout) -> ! {
    panic!("Allocation error: {:?}", layout);
}
