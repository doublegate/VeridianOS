// Print macros for kernel output

#[cfg(target_arch = "x86_64")]
#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => ({
        // Print to both VGA and serial for debugging
        $crate::arch::x86_64::vga::_print(format_args!($($arg)*));
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
// See arch/aarch64/README_LLVM_BUG.md for details
// For now, println! is effectively non-functional on AArch64
// Use uart_write! macro from arch::aarch64::manual_print for critical messages
#[cfg(target_arch = "aarch64")]
#[macro_export]
macro_rules! print {
    ($s:literal) => {{
        // Placeholder - real printing must use uart_write! macro
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
        // Placeholder - use uart_write!(b'\n') for newlines
    }};
    ($s:literal) => {{
        // Placeholder - use uart_write! for actual output
    }};
    ($($arg:tt)*) => {{
        // Cannot support formatting on AArch64 due to LLVM bug
    }};
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

/// Safe println for bootstrap and critical messages
/// On AArch64, uses direct printing. On other archs, uses normal println!
#[macro_export]
macro_rules! boot_println {
    // Empty println
    () => {
        #[cfg(target_arch = "aarch64")]
        $crate::arch::aarch64::direct_uart::direct_print_newline();
        #[cfg(not(target_arch = "aarch64"))]
        println!();
    };
    // Literal string only
    ($s:literal) => {
        #[cfg(target_arch = "aarch64")]
        {
            // No-op for AArch64 to avoid LLVM bugs completely
        }
        #[cfg(not(target_arch = "aarch64"))]
        println!($s);
    };
    // Fallback for any other pattern
    ($($arg:tt)*) => {
        #[cfg(target_arch = "aarch64")]
        {
            // No-op for AArch64 to avoid LLVM bugs completely
        }
        #[cfg(not(target_arch = "aarch64"))]
        println!($($arg)*);
    };
}

/// Special macro for printing numbers on AArch64
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
