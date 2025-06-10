//! No-std test framework for VeridianOS kernel
//!
//! This module provides testing infrastructure that works in a no_std
//! environment by using serial output and QEMU exit codes to report test
//! results.

use core::panic::PanicInfo;

use crate::{serial_print, serial_println};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum QemuExitCode {
    Success = 0x10,
    Failed = 0x11,
}

/// Trait that all testable functions must implement
pub trait Testable {
    fn run(&self);
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
#[allow(dead_code)]
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
}

/// Exit QEMU with a specific exit code
pub fn exit_qemu(_exit_code: QemuExitCode) -> ! {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        use x86_64::instructions::port::Port;
        let mut port = Port::new(0xf4);
        port.write(_exit_code as u32);
        core::hint::unreachable_unchecked();
    }

    #[cfg(target_arch = "aarch64")]
    {
        // Use PSCI SYSTEM_OFF for AArch64
        const PSCI_SYSTEM_OFF: u32 = 0x84000008;
        unsafe {
            core::arch::asm!(
                "mov w0, {psci_off:w}",
                "hvc #0",
                psci_off = in(reg) PSCI_SYSTEM_OFF,
                options(noreturn)
            );
        }
    }

    #[cfg(target_arch = "riscv64")]
    {
        // Use SBI shutdown call
        const SBI_SHUTDOWN: usize = 8;
        unsafe {
            core::arch::asm!(
                "li a7, {sbi_shutdown}",
                "ecall",
                sbi_shutdown = const SBI_SHUTDOWN,
                options(noreturn)
            );
        }
    }

    #[cfg(not(any(
        target_arch = "x86_64",
        target_arch = "aarch64",
        target_arch = "riscv64"
    )))]
    loop {
        core::hint::spin_loop();
    }
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
