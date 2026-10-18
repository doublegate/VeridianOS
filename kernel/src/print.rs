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

// Host target (x86_64-unknown-linux-gnu, for coverage/unit tests):
// Kernel serial/UART hardware is not available; use no-op to avoid SIGSEGV.
#[cfg(not(target_os = "none"))]
#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => ({ let _ = format_args!($($arg)*); });
}

#[cfg(not(target_os = "none"))]
#[macro_export]
macro_rules! println {
    () => ({});
    ($($arg:tt)*) => ({ let _ = format_args!($($arg)*); });
}

// x86_64 bare-metal — triple output: serial + framebuffer console + capture
// buffer
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => ({
        $crate::serial::_serial_print(format_args!($($arg)*));
        $crate::graphics::fbcon::_fbcon_print(format_args!($($arg)*));
        $crate::print_capture::_capture_print(format_args!($($arg)*));
    });
}

#[cfg(all(target_arch = "x86_64", target_os = "none"))]
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
        $crate::print::with_console(|| {
            let mut _w = $crate::arch::aarch64::direct_uart::DirectUartWriter;
            let _ = _w.write_fmt(format_args!($($arg)*));
        });
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

/// No CPU holds the console.
#[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
const CONSOLE_FREE: usize = usize::MAX;

/// The CPU writing to the console (AArch64 and RISC-V; x86_64 holds its
/// serial port's own lock).
#[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
static CONSOLE_OWNER: core::sync::atomic::AtomicUsize =
    core::sync::atomic::AtomicUsize::new(CONSOLE_FREE);

/// Run `f`, which writes one message to the console, with no other CPU
/// writing at the same time and interrupts off, so lines from several CPUs
/// do not interleave (a boot-test tally once came out as
/// `Results: [P50/50 passed` with four RISC-V harts printing). Reentrant on
/// one CPU, so a fault or panic while printing still prints; and a CPU that
/// stopped while holding it (a panic on another CPU) is waited for only so
/// long, then its ownership is taken over, rather than silencing the
/// console. Taking it over (not printing without it) keeps the waiting
/// CPUs from all writing at once, and each from waiting out the timeout
/// again on every message (PR #19 review).
#[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
pub fn with_console<R>(f: impl FnOnce() -> R) -> R {
    use core::sync::atomic::Ordering;
    let _irq = crate::arch::disable_interrupts();
    let me = crate::arch::percpu::this_cpu_id() as usize;
    if CONSOLE_OWNER.load(Ordering::Relaxed) == me {
        return f();
    }
    let mut spins: u64 = 0;
    let mut seen = CONSOLE_FREE;
    while let Err(owner) =
        CONSOLE_OWNER.compare_exchange_weak(CONSOLE_FREE, me, Ordering::Acquire, Ordering::Relaxed)
    {
        // The wait restarts whenever the console changes hands: only an
        // owner that holds it the whole time is taken over.
        if owner != seen {
            seen = owner;
            spins = 0;
        }
        spins += 1;
        if spins > 100_000_000
            && CONSOLE_OWNER
                .compare_exchange(owner, me, Ordering::Acquire, Ordering::Relaxed)
                .is_ok()
        {
            break;
        }
        core::hint::spin_loop();
    }
    let r = f();
    CONSOLE_OWNER.store(CONSOLE_FREE, Ordering::Release);
    r
}
