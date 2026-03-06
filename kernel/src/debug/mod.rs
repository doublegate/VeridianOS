//! Kernel Debug Infrastructure
//!
//! Provides GDB remote serial protocol (RSP) stub for interactive debugging
//! over COM2 (0x2F8). Supports register read/write, memory access,
//! breakpoints, watchpoints, and thread awareness.

#[cfg(all(feature = "alloc", target_arch = "x86_64"))]
pub mod gdb_stub;

#[cfg(all(feature = "alloc", target_arch = "x86_64"))]
pub mod breakpoint;
