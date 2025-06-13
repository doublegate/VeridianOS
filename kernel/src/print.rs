// Print macros for kernel output

#[cfg(target_arch = "x86_64")]
#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => ($crate::arch::x86_64::vga::_print(format_args!($($arg)*)));
}

#[cfg(target_arch = "x86_64")]
#[macro_export]
macro_rules! println {
    () => ($crate::print!("\n"));
    ($($arg:tt)*) => ($crate::print!("{}\n", format_args!($($arg)*)));
}

// AArch64 implementation using serial port
#[cfg(target_arch = "aarch64")]
#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => ($crate::serial::_serial_print(format_args!($($arg)*)));
}

#[cfg(target_arch = "aarch64")]
#[macro_export]
macro_rules! println {
    () => ($crate::print!("\n"));
    ($($arg:tt)*) => ($crate::print!("{}\n", format_args!($($arg)*)));
}

// RISC-V implementation using serial port
#[cfg(target_arch = "riscv64")]
#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => ($crate::serial::_serial_print(format_args!($($arg)*)));
}

#[cfg(target_arch = "riscv64")]
#[macro_export]
macro_rules! println {
    () => ($crate::print!("\n"));
    ($($arg:tt)*) => ($crate::print!("{}\n", format_args!($($arg)*)));
}
