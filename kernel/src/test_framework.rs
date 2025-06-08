//! No-std test framework for VeridianOS kernel
//!
//! This module provides testing infrastructure that works in a no_std environment
//! by using serial output and QEMU exit codes to report test results.

#![cfg(test)]

use crate::serial_print;
use crate::serial_println;
use crate::QemuExitCode;
use core::panic::PanicInfo;

/// Trait that all testable functions must implement
pub trait Testable {
    fn run(&self) -> ();
}

impl<T> Testable for T
where
    T: Fn(),
{
    fn run(&self) {
        serial_print!("{}...\t", core::any::type_name::<T>());
        self();
        serial_println!("[ok]");
    }
}

/// Custom test runner for kernel tests
pub fn test_runner(tests: &[&dyn Testable]) {
    serial_println!("Running {} tests", tests.len());
    for test in tests {
        test.run();
    }
    exit_qemu(QemuExitCode::Success);
}

/// Panic handler for test mode
pub fn test_panic_handler(info: &PanicInfo) -> ! {
    serial_println!("[failed]\n");
    serial_println!("Error: {}\n", info);
    exit_qemu(QemuExitCode::Failed);
    
    // This should never be reached, but just in case
    loop {
        core::hint::spin_loop();
    }
}

/// Exit QEMU with a specific exit code
pub fn exit_qemu(exit_code: QemuExitCode) {
    crate::exit_qemu(exit_code)
}

/// Helper macro for creating test modules
#[macro_export]
macro_rules! test_module {
    ($name:ident, $($test:path),* $(,)?) => {
        #[cfg(test)]
        mod $name {
            use super::*;
            
            #[test_case]
            $(
                fn $test() {
                    $test();
                }
            )*
        }
    };
}

/// Assertion macros for kernel tests
#[macro_export]
macro_rules! kernel_assert {
    ($cond:expr) => {
        if !$cond {
            serial_println!("Assertion failed: {}", stringify!($cond));
            panic!("Assertion failed");
        }
    };
    ($cond:expr, $($arg:tt)*) => {
        if !$cond {
            serial_println!($($arg)*);
            panic!("Assertion failed");
        }
    };
}

#[macro_export]
macro_rules! kernel_assert_eq {
    ($left:expr, $right:expr) => {
        if $left != $right {
            serial_println!(
                "Assertion failed: {} != {}\n  left: {:?}\n right: {:?}",
                stringify!($left),
                stringify!($right),
                $left,
                $right
            );
            panic!("Assertion failed: not equal");
        }
    };
}

#[macro_export]
macro_rules! kernel_assert_ne {
    ($left:expr, $right:expr) => {
        if $left == $right {
            serial_println!(
                "Assertion failed: {} == {}\n  left: {:?}\n right: {:?}",
                stringify!($left),
                stringify!($right),
                $left,
                $right
            );
            panic!("Assertion failed: equal");
        }
    };
}