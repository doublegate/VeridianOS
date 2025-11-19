// Generic serial interface that delegates to architecture-specific
// implementations

use core::fmt;

#[cfg(target_arch = "aarch64")]
pub use crate::arch::aarch64::serial::*;
#[cfg(target_arch = "riscv64")]
pub use crate::arch::riscv64::serial::*;
// Re-export architecture-specific serial implementations
#[cfg(target_arch = "x86_64")]
pub use crate::arch::x86_64::serial::*;

// Serial print macros for testing
#[macro_export]
macro_rules! serial_print {
    ($($arg:tt)*) => {
        $crate::serial::_serial_print(format_args!($($arg)*))
    };
}

#[macro_export]
macro_rules! serial_println {
    () => ($crate::serial_print!("\n"));
    ($($arg:tt)*) => {
        $crate::serial_print!("{}\n", format_args!($($arg)*))
    };
}

// Delegate to architecture-specific implementation
#[doc(hidden)]
pub fn _serial_print(args: fmt::Arguments) {
    #[cfg(target_arch = "x86_64")]
    crate::arch::x86_64::serial::_serial_print(args);

    #[cfg(target_arch = "aarch64")]
    crate::arch::aarch64::serial::_serial_print(args);

    #[cfg(target_arch = "riscv64")]
    crate::arch::riscv64::serial::_serial_print(args);
}
