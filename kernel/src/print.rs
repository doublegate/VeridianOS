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

#[cfg(test)]
#[macro_export]
macro_rules! serial_print {
    ($($arg:tt)*) => ($crate::arch::x86_64::serial::_print(format_args!($($arg)*)));
}

#[cfg(test)]
#[macro_export]
macro_rules! serial_println {
    () => ($crate::serial_print!("\n"));
    ($($arg:tt)*) => ($crate::serial_print!("{}\n", format_args!($($arg)*)));
}

// Stub implementations for other architectures
#[cfg(not(target_arch = "x86_64"))]
#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => {};
}

#[cfg(not(target_arch = "x86_64"))]
#[macro_export]
macro_rules! println {
    ($($arg:tt)*) => {};
}
