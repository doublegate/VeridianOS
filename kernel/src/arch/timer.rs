//! Architecture-independent timer interface.
//!
//! Provides the [`PlatformTimer`] trait for architecture-agnostic timer access
//! and free functions that delegate to the current platform's implementation.
//!
//! # Architecture implementations
//!
//! * **x86_64**: [`X86_64Timer`] -- PIT-based tick counter and RDTSC timestamp.
//! * **AArch64**: [`AArch64Timer`] -- Generic timer counter (CNTVCT_EL0 /
//!   CNTFRQ_EL0).
//! * **RISC-V**: [`RiscVTimer`] -- `rdtime` instruction via SBI timer.

/// Platform timer abstraction trait.
///
/// Each architecture provides a zero-sized struct implementing this trait,
/// exposing a common interface for tick counting, frequency queries, and
/// hardware timer programming.
pub trait PlatformTimer {
    /// Read the current hardware tick/cycle counter.
    ///
    /// This is a monotonically increasing counter whose frequency is
    /// given by [`ticks_per_second`](PlatformTimer::ticks_per_second).
    fn current_ticks() -> u64;

    /// Return the frequency of the tick counter in Hz.
    ///
    /// For x86_64 this is an approximate TSC frequency; for AArch64 the
    /// architectural `CNTFRQ_EL0`; for RISC-V the timebase frequency.
    fn ticks_per_second() -> u64;

    /// Program the next timer interrupt to fire after `ticks` ticks from now.
    fn set_timer(ticks: u64);
}

// ---------------------------------------------------------------------------
// x86_64 implementation
// ---------------------------------------------------------------------------

#[cfg(target_arch = "x86_64")]
pub struct X86_64Timer;

#[cfg(target_arch = "x86_64")]
impl PlatformTimer for X86_64Timer {
    #[inline]
    fn current_ticks() -> u64 {
        // SAFETY: RDTSC is a non-privileged x86_64 instruction that reads the
        // Time Stamp Counter. No side effects; always safe to call.
        unsafe { core::arch::x86_64::_rdtsc() }
    }

    #[inline]
    fn ticks_per_second() -> u64 {
        // Approximate TSC frequency -- 2 GHz is a reasonable default for QEMU.
        // A proper implementation would calibrate against the PIT or HPET.
        2_000_000_000
    }

    fn set_timer(ticks: u64) {
        // Convert ticks to PIT divisor.
        // PIT frequency is 1.193182 MHz.
        const PIT_FREQUENCY: u64 = 1_193_182;
        let divisor = if ticks == 0 {
            1u16
        } else {
            let d = PIT_FREQUENCY * ticks / Self::ticks_per_second();
            if d == 0 {
                1
            } else if d > u16::MAX as u64 {
                u16::MAX
            } else {
                d as u16
            }
        };

        // SAFETY: I/O port writes to the 8254 PIT command (0x43) and channel 0
        // data (0x40) registers. Standard PIT programming sequence.
        unsafe {
            use x86_64::instructions::port::Port;
            let mut cmd: Port<u8> = Port::new(0x43);
            let mut data: Port<u8> = Port::new(0x40);
            cmd.write(0x36);
            data.write((divisor & 0xFF) as u8);
            data.write((divisor >> 8) as u8);
        }
    }
}

// ---------------------------------------------------------------------------
// AArch64 implementation
// ---------------------------------------------------------------------------

#[cfg(target_arch = "aarch64")]
pub struct AArch64Timer;

#[cfg(target_arch = "aarch64")]
impl PlatformTimer for AArch64Timer {
    #[inline]
    fn current_ticks() -> u64 {
        let cnt: u64;
        // SAFETY: Reading CNTVCT_EL0 (virtual timer count) is a non-privileged
        // AArch64 operation with no side effects.
        unsafe {
            core::arch::asm!("mrs {}, CNTVCT_EL0", out(reg) cnt);
        }
        cnt
    }

    #[inline]
    fn ticks_per_second() -> u64 {
        let freq: u64;
        // SAFETY: Reading CNTFRQ_EL0 returns the system counter frequency.
        // Read-only, no side effects.
        unsafe {
            core::arch::asm!("mrs {}, CNTFRQ_EL0", out(reg) freq);
        }
        freq
    }

    fn set_timer(ticks: u64) {
        // SAFETY: Writing CNTP_TVAL_EL0 sets the physical timer value;
        // writing CNTP_CTL_EL0 enables the timer. Standard EL1 operations.
        unsafe {
            core::arch::asm!("msr CNTP_TVAL_EL0, {}", in(reg) ticks);
            core::arch::asm!("msr CNTP_CTL_EL0, {}", in(reg) 1u64);
        }
    }
}

// ---------------------------------------------------------------------------
// RISC-V implementation
// ---------------------------------------------------------------------------

#[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
pub struct RiscVTimer;

#[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
impl PlatformTimer for RiscVTimer {
    #[inline]
    fn current_ticks() -> u64 {
        let time: u64;
        // SAFETY: `rdtime` reads the real-time counter CSR. Always accessible
        // in supervisor mode, no side effects.
        unsafe {
            core::arch::asm!("rdtime {}", out(reg) time);
        }
        time
    }

    #[inline]
    fn ticks_per_second() -> u64 {
        // QEMU virt machine timebase frequency is 10 MHz.
        10_000_000
    }

    fn set_timer(ticks: u64) {
        let current = Self::current_ticks();
        let _ = crate::arch::riscv::sbi::set_timer(current + ticks);
    }
}

// ---------------------------------------------------------------------------
// Free functions (delegate to the active platform)
// ---------------------------------------------------------------------------

/// Get current timer tick count (from the software tick counter maintained by
/// each architecture's interrupt handler).
pub fn get_ticks() -> u64 {
    #[cfg(target_arch = "x86_64")]
    {
        crate::arch::x86_64::timer::get_ticks()
    }

    #[cfg(target_arch = "aarch64")]
    {
        crate::arch::aarch64::timer::get_ticks()
    }

    #[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
    {
        crate::arch::riscv::timer::get_ticks()
    }
}

/// Read the hardware timestamp counter directly.
///
/// Unlike [`get_ticks`], which returns a software-maintained counter
/// incremented on each timer interrupt, this reads the raw hardware
/// cycle/time counter for high-resolution timing.
pub fn read_hw_timestamp() -> u64 {
    #[cfg(target_arch = "x86_64")]
    {
        X86_64Timer::current_ticks()
    }

    #[cfg(target_arch = "aarch64")]
    {
        AArch64Timer::current_ticks()
    }

    #[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
    {
        RiscVTimer::current_ticks()
    }
}

/// Return the hardware timer frequency in Hz.
pub fn hw_ticks_per_second() -> u64 {
    #[cfg(target_arch = "x86_64")]
    {
        X86_64Timer::ticks_per_second()
    }

    #[cfg(target_arch = "aarch64")]
    {
        AArch64Timer::ticks_per_second()
    }

    #[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
    {
        RiscVTimer::ticks_per_second()
    }
}

/// Program the next timer interrupt.
pub fn set_hw_timer(ticks: u64) {
    #[cfg(target_arch = "x86_64")]
    {
        X86_64Timer::set_timer(ticks);
    }

    #[cfg(target_arch = "aarch64")]
    {
        AArch64Timer::set_timer(ticks);
    }

    #[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
    {
        RiscVTimer::set_timer(ticks);
    }
}
