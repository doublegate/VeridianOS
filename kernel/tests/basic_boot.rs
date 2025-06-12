//! Basic boot tests for VeridianOS kernel

#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(test_runner)]
#![reexport_test_harness_main = "test_main"]

// Import panic handler from shared test library
extern crate veridian_kernel;
use veridian_kernel::{exit_qemu, serial_println, QemuExitCode};

#[no_mangle]
pub extern "C" fn _start() -> ! {
    serial_println!("Starting basic boot tests...");
    test_main();
    exit_qemu(QemuExitCode::Success)
}

// Custom test runner for this test binary
pub fn test_runner(tests: &[&dyn Fn()]) {
    serial_println!("Running {} tests", tests.len());
    for test in tests {
        test();
    }
}

// Panic handler - use the one from veridian_kernel
use core::panic::PanicInfo;

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    veridian_kernel::test_panic_handler(info)
}

#[test_case]
fn test_println() {
    serial_println!("test_println...");
    serial_println!("test_println output");
    serial_println!("[ok]");
}

#[test_case]
fn test_simple_assertion() {
    serial_println!("test_simple_assertion...");
    let x = 2 + 2;
    assert_eq!(x, 4);
    serial_println!("[ok]");
}

#[test_case]
fn test_kernel_alive() {
    serial_println!("test_kernel_alive...");
    // If we get here, the kernel booted successfully
    serial_println!("Kernel is alive and running tests!");
    serial_println!("[ok]");
}
