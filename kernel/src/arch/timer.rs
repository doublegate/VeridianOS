//! Architecture-independent timer interface

/// Get current timer tick count
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
