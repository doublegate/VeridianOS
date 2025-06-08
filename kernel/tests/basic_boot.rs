//! Basic boot tests for VeridianOS kernel

#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(crate::test_runner)]
#![reexport_test_harness_main = "test_main"]

use core::panic::PanicInfo;
use veridian_kernel::{serial_print, serial_println};

#[no_mangle]
pub extern "C" fn _start() -> ! {
    test_main();

    loop {}
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    veridian_kernel::test_panic_handler(info)
}

fn test_runner(tests: &[&dyn veridian_kernel::Testable]) {
    veridian_kernel::test_runner(tests)
}

#[test_case]
fn test_println() {
    serial_println!("test_println output");
}

#[test_case]
fn test_simple_assertion() {
    assert_eq!(1, 1);
}

#[test_case]
fn test_kernel_alive() {
    // If we get here, the kernel booted successfully
    serial_println!("Kernel is alive and running tests!");
}