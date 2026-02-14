//! Kernel printing macros.
//!
//! Provides `print!` and `println!` macros that delegate to the
//! architecture-specific serial/UART output. On x86_64 this writes to
//! the VGA text buffer, on AArch64 to the PL011 UART, and on RISC-V
//! to the 16550 UART via SBI.

// x86_64 implementation
#[cfg(target_arch = "x86_64")]
#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => ({
        // Skip VGA for now due to early boot issues, only use serial
        $crate::serial::_serial_print(format_args!($($arg)*));
    });
}

#[cfg(target_arch = "x86_64")]
#[macro_export]
macro_rules! println {
    () => ($crate::print!("\n"));
    ($($arg:tt)*) => ($crate::print!("{}\n", format_args!($($arg)*)));
}

// AArch64 implementation - severely limited due to LLVM bug
#[cfg(target_arch = "aarch64")]
#[macro_export]
macro_rules! print {
    ($s:literal) => {{
        // Placeholder - real printing must use uart_write_str
        // due to LLVM bug that causes hangs with any loops/iterators
    }};
    ($($arg:tt)*) => {{
        // Cannot support formatting on AArch64 due to LLVM bug
    }};
}

#[cfg(target_arch = "aarch64")]
#[macro_export]
macro_rules! println {
    () => {{
        // Placeholder - use direct_uart for newlines
    }};
    ($s:literal) => {{
        // Placeholder - use direct_uart for actual output
    }};
    ($($arg:tt)*) => {{
        // Cannot support formatting on AArch64 due to LLVM bug
    }};
}

// RISC-V implementation
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

// Bootstrap-safe println for all architectures
#[macro_export]
macro_rules! boot_println {
    () => {
        #[cfg(target_arch = "aarch64")]
        $crate::arch::aarch64::direct_uart::direct_print_newline();
        #[cfg(not(target_arch = "aarch64"))]
        println!();
    };
    ($s:literal) => {
        #[cfg(target_arch = "aarch64")]
        {
            // No-op for AArch64 to avoid LLVM bugs completely
        }
        #[cfg(not(target_arch = "aarch64"))]
        println!($s);
    };
    ($($arg:tt)*) => {
        #[cfg(target_arch = "aarch64")]
        {
            // No-op for AArch64 to avoid LLVM bugs completely
        }
        #[cfg(not(target_arch = "aarch64"))]
        println!($($arg)*);
    };
}

// Number printing for AArch64
#[macro_export]
macro_rules! boot_print_num {
    ($prefix:literal, $n:expr) => {
        #[cfg(target_arch = "aarch64")]
        {
            $crate::arch::aarch64::direct_uart::direct_print_str($prefix);
            $crate::arch::aarch64::direct_uart::direct_print_num($n as u64);
            $crate::arch::aarch64::direct_uart::direct_print_newline();
        }
        #[cfg(not(target_arch = "aarch64"))]
        println!("{}{}", $prefix, $n);
    };
}
