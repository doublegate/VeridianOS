//! Manual printing helpers for AArch64
//!
//! Due to the LLVM bug, this is the only reliable way to print on AArch64.
//! Use these macros for critical boot messages.

/// Helper macro to reduce boilerplate for manual UART writes
/// Usage: uart_write!(b'H', b'e', b'l', b'l', b'o', b'\n');
#[macro_export]
macro_rules! uart_write {
    ($($byte:expr),*) => {{
        unsafe {
            let uart = 0x0900_0000 as *mut u8;
            $(
                core::ptr::write_volatile(uart, $byte);
            )*
        }
    }};
}

/// Print a short literal string (manually list each character)
/// Usage: uart_print_chars!(b'H', b'i', b'\n');
#[macro_export]
macro_rules! uart_print_chars {
    ($($char:expr),*) => {{
        uart_write!($($char),*);
    }};
}

/// Common messages as constants for easy use
pub mod messages {
    /// Boot success message
    pub const BOOT_OK: &[u8] = b"Boot OK\n";

    /// Error prefix
    pub const ERROR: &[u8] = b"ERROR: ";

    /// Warning prefix  
    pub const WARN: &[u8] = b"WARN: ";
}
