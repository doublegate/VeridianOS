//! Basic boot tests for VeridianOS kernel

#![no_std]
#![no_main]

use core::panic::PanicInfo;

use veridian_kernel::{exit_qemu, serial_println, test_panic_handler, QemuExitCode};

#[no_mangle]
pub extern "C" fn _start() -> ! {
    serial_println!("Starting basic boot tests...");

    test_println();
    test_simple_assertion();
    test_kernel_alive();

    serial_println!("All tests passed!");
    exit_qemu(QemuExitCode::Success)
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    test_panic_handler(info)
}

fn test_println() {
    serial_println!("test_println output");
}

fn test_simple_assertion() {
    let x = 2 + 2;
    assert_eq!(x, 4);
}

fn test_kernel_alive() {
    // If we get here, the kernel booted successfully
    serial_println!("Kernel is alive and running tests!");
}
