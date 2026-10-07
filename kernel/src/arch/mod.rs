//! Architecture abstraction layer for VeridianOS.
//!
//! This module provides architecture-specific implementations for x86_64,
//! AArch64, and RISC-V 64-bit platforms. Each sub-module exports a common
//! interface (serial, boot, context switching, interrupts) that the
//! architecture-independent kernel code uses.

pub mod percpu;
pub mod smp_boot;

#[cfg(all(feature = "smp", target_arch = "aarch64", target_os = "none"))]
pub(crate) use aarch64::smp as smp_arch;
/// Architecture side of secondary-CPU bring-up (see `smp_boot`).
#[cfg(all(feature = "smp", target_arch = "riscv64", target_os = "none"))]
pub(crate) use riscv64::smp as smp_arch;
#[cfg(all(feature = "smp", target_arch = "x86_64", target_os = "none"))]
pub(crate) use x86_64::smp as smp_arch;

/// Architectures whose secondary bring-up is not implemented yet describe
/// no secondaries, so the boot CPU runs alone.
#[cfg(all(
    feature = "smp",
    not(all(
        any(
            target_arch = "riscv64",
            target_arch = "aarch64",
            target_arch = "x86_64"
        ),
        target_os = "none"
    ))
))]
pub(crate) mod smp_arch {
    use alloc::vec::Vec;

    use super::smp_boot::ApBootArgs;

    pub fn enumerate_secondaries() -> Vec<u32> {
        Vec::new()
    }
    pub fn prepare_boot_args(_args: &ApBootArgs) {}
    pub fn start_ap(_hw: u32, _args: &'static ApBootArgs) -> Result<(), &'static str> {
        Err("not implemented on this architecture")
    }
    pub fn ap_init() {}
    pub fn idle() -> ! {
        loop {
            core::hint::spin_loop();
        }
    }
}

#[cfg(target_arch = "x86_64")]
pub mod x86_64;

#[cfg(target_arch = "x86_64")]
pub use x86_64::*;

#[cfg(target_arch = "aarch64")]
pub mod aarch64;

#[cfg(target_arch = "aarch64")]
pub use aarch64::*;

#[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
pub mod riscv;

#[cfg(target_arch = "riscv64")]
pub mod riscv64;

#[cfg(all(
    any(target_arch = "aarch64", target_arch = "riscv64"),
    target_os = "none"
))]
pub(crate) mod stack_canary;

/// Whether the boot stack has never overflowed into its canary (always true
/// where the bootloader provides the stack, e.g. x86_64).
pub(crate) fn boot_stack_intact() -> bool {
    #[cfg(all(
        any(target_arch = "aarch64", target_arch = "riscv64"),
        target_os = "none"
    ))]
    {
        stack_canary::intact()
    }
    #[cfg(not(all(
        any(target_arch = "aarch64", target_arch = "riscv64"),
        target_os = "none"
    )))]
    {
        true
    }
}

#[cfg(target_arch = "riscv64")]
pub use riscv64::*;

// Common timer module
pub mod fdt;
pub mod timer;

// Common context module
pub mod context;

// Architecture-independent memory barrier abstractions
pub mod barriers;

// Architecture-independent hardware entropy abstractions
pub mod entropy;

// Serial initialization is handled per-architecture
