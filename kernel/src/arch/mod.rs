//! Architecture abstraction layer for VeridianOS.
//!
//! This module provides architecture-specific implementations for x86_64,
//! AArch64, and RISC-V 64-bit platforms. Each sub-module exports a common
//! interface (serial, boot, context switching, interrupts) that the
//! architecture-independent kernel code uses.

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

#[cfg(target_arch = "riscv64")]
pub use riscv64::*;

// Common timer module
pub mod timer;

// Common context module
pub mod context;

// Serial initialization is handled per-architecture
