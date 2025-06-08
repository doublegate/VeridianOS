//! VeridianOS Kernel Library
//!
//! This library provides the core functionality for the VeridianOS kernel
//! and exports necessary items for testing.

#![no_std]
#![cfg_attr(test, no_main)]
#![feature(custom_test_frameworks)]
#![test_runner(crate::test_runner)]
#![reexport_test_harness_main = "test_main"]

#[macro_use]
mod print;

mod arch;
mod cap;
mod ipc;
mod mm;
mod sched;
mod serial;

#[cfg(test)]
mod test_framework;

pub mod bench;

// Re-export for tests
pub use serial::{serial_print, serial_println};

#[cfg(test)]
pub use test_framework::{Testable, test_runner, test_panic_handler};

#[cfg(test)]
pub use crate::QemuExitCode;

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum QemuExitCode {
    Success = 0x10,
    Failed = 0x11,
}

#[cfg(test)]
pub fn exit_qemu(exit_code: QemuExitCode) -> ! {
    #[cfg(target_arch = "x86_64")]
    {
        use x86_64::instructions::port::Port;
        unsafe {
            let mut port = Port::new(0xf4);
            port.write(exit_code as u32);
        }
    }
    
    #[cfg(target_arch = "aarch64")]
    {
        // Use PSCI SYSTEM_OFF for AArch64
        unsafe {
            core::arch::asm!(
                "mov w0, #0x84000008",  // PSCI SYSTEM_OFF
                "hvc #0",
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
    
    loop {}
}

#[cfg(test)]
#[no_mangle]
pub extern "C" fn _start() -> ! {
    test_main();
    loop {}
}

#[cfg(test)]
fn test_runner(tests: &[&dyn test_framework::Testable]) {
    test_framework::test_runner(tests)
}

#[cfg(test)]
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    test_framework::test_panic_handler(info)
}