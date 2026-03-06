//! Developer Tools
//!
//! Native development tools for VeridianOS including a Git client,
//! IDE with LSP support, CI runner, and profiling GUI.

#[cfg(feature = "alloc")]
pub mod git;

#[cfg(feature = "alloc")]
pub mod ide;

#[cfg(feature = "alloc")]
pub mod ci;

#[cfg(feature = "alloc")]
pub mod profiler;
