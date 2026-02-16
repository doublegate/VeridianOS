//! Kernel printing macros.
//!
//! Provides `print!` and `println!` macros that delegate to the
//! architecture-specific serial/UART output. On x86_64 this writes to
//! the VGA text buffer, on AArch64 to the PL011 UART, and on RISC-V
//! to the 16550 UART via SBI.
//!
//! Also provides `kprintln!` / `kprint!` / `kprint_num!` macros that
//! work uniformly across all architectures, including AArch64 where
//! the standard `println!` is a no-op due to an LLVM loop-compilation
//! bug. The `kprintln!` macro handles arch dispatch internally,
//! eliminating the need for per-call-site `cfg(target_arch)` blocks.

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

// AArch64 implementation - uses DirectUartWriter (assembly-based byte output)
// to bypass LLVM loop-compilation bug. The write_str() implementation calls
// uart_write_bytes_asm() which is a pure assembly loop, so LLVM's optimizer
// cannot miscompile the critical byte-output path. The core::fmt machinery
// drives the formatting but each write_str() call goes through safe assembly.
#[cfg(target_arch = "aarch64")]
#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => ({
        use core::fmt::Write;
        let mut _w = $crate::arch::aarch64::direct_uart::DirectUartWriter;
        let _ = _w.write_fmt(format_args!($($arg)*));
    });
}

#[cfg(target_arch = "aarch64")]
#[macro_export]
macro_rules! println {
    () => ($crate::print!("\n"));
    ($($arg:tt)*) => ($crate::print!("{}\n", format_args!($($arg)*)));
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

// ---------------------------------------------------------------------------
// Unified kprintln! / kprint! / kprint_num! macros
//
// These macros handle the AArch64 LLVM limitation *inside* the macro
// definition, so call-sites don't need any cfg(target_arch) blocks.
//
// * Literal-only path (`kprintln!("hello")`) works on ALL architectures
//   including AArch64 (via direct_uart assembly).
// * Formatted path (`kprintln!("x={}", 42)`) works on x86_64 and RISC-V; on
//   AArch64 it is a silent no-op (same as current `println!` behavior, but
//   centralized).
// ---------------------------------------------------------------------------

/// Print a string literal to the kernel console (all architectures).
/// For formatted output, works on x86_64 and RISC-V only; no-op on AArch64.
#[macro_export]
macro_rules! kprint {
    // Literal-only: works on all architectures
    ($lit:literal) => {{
        #[cfg(target_arch = "aarch64")]
        $crate::arch::aarch64::direct_uart::direct_print_str($lit);
        #[cfg(not(target_arch = "aarch64"))]
        $crate::print!($lit);
    }};
    // Formatted: x86_64/RISC-V only; silent no-op on AArch64
    ($($arg:tt)*) => {{
        #[cfg(not(target_arch = "aarch64"))]
        $crate::print!($($arg)*);
    }};
}

/// Print a string literal followed by newline (all architectures).
/// For formatted output, works on x86_64 and RISC-V only; no-op on AArch64.
#[macro_export]
macro_rules! kprintln {
    // No args: just a newline
    () => {{
        #[cfg(target_arch = "aarch64")]
        $crate::arch::aarch64::direct_uart::direct_print_str("\n");
        #[cfg(not(target_arch = "aarch64"))]
        $crate::println!();
    }};
    // Literal-only: works on all architectures including AArch64
    ($lit:literal) => {{
        #[cfg(target_arch = "aarch64")]
        $crate::arch::aarch64::direct_uart::direct_print_str(concat!($lit, "\n"));
        #[cfg(not(target_arch = "aarch64"))]
        $crate::println!($lit);
    }};
    // Formatted: x86_64/RISC-V only; silent no-op on AArch64
    ($($arg:tt)*) => {{
        #[cfg(not(target_arch = "aarch64"))]
        $crate::println!($($arg)*);
    }};
}

/// Print a literal prefix followed by a number on all architectures.
/// On AArch64, uses direct_uart assembly-based number printing.
#[macro_export]
macro_rules! kprint_num {
    ($prefix:literal, $n:expr) => {{
        #[cfg(target_arch = "aarch64")]
        {
            $crate::arch::aarch64::direct_uart::direct_print_str($prefix);
            $crate::arch::aarch64::direct_uart::direct_print_num($n as u64);
            $crate::arch::aarch64::direct_uart::direct_print_str("\n");
        }
        #[cfg(not(target_arch = "aarch64"))]
        $crate::println!("{}{}", $prefix, $n);
    }};
}

/// Print a runtime &str expression (not just a literal) on all architectures.
/// On AArch64, uses direct_uart; on x86_64/RISC-V, uses serial print.
#[macro_export]
macro_rules! kprint_rt {
    ($s:expr) => {{
        #[cfg(target_arch = "aarch64")]
        $crate::arch::aarch64::direct_uart::direct_print_str($s);
        #[cfg(not(target_arch = "aarch64"))]
        $crate::print!("{}", $s);
    }};
}

/// Print a u64 number without newline on all architectures.
/// On AArch64, uses direct_uart assembly; on x86_64/RISC-V, uses serial.
#[macro_export]
macro_rules! kprint_u64 {
    ($n:expr) => {{
        #[cfg(target_arch = "aarch64")]
        $crate::arch::aarch64::direct_uart::direct_print_num($n as u64);
        #[cfg(not(target_arch = "aarch64"))]
        $crate::print!("{}", $n);
    }};
}

// Legacy macros (kept for backward compatibility, prefer kprintln!/kprint!)

/// Bootstrap-safe println (legacy - prefer kprintln!)
#[macro_export]
macro_rules! boot_println {
    () => { $crate::kprintln!(); };
    ($s:literal) => { $crate::kprintln!($s); };
    ($($arg:tt)*) => { $crate::kprintln!($($arg)*); };
}

/// Number printing (legacy - prefer kprint_num!)
#[macro_export]
macro_rules! boot_print_num {
    ($prefix:literal, $n:expr) => {
        $crate::kprint_num!($prefix, $n);
    };
}
