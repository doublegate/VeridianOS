//! VeridianOS Platform Layer for Rust std
//!
//! This crate provides the platform-specific implementation layer that bridges
//! Rust user-space code to VeridianOS kernel syscalls. It is the equivalent of
//! `std::sys::unix` but for VeridianOS.
//!
//! # Architecture
//!
//! ```text
//! User Rust Code
//!       |
//!       v
//!   Rust std (core, alloc)
//!       |
//!       v
//!   veridian-std  <-- THIS CRATE
//!       |
//!       v
//!   VeridianOS kernel (via syscall instruction)
//! ```
//!
//! # Supported Operations
//!
//! - **File I/O**: open, close, read, write, seek, stat
//! - **Process**: exit, getpid, fork, exec, waitpid
//! - **Memory**: mmap, munmap, brk
//! - **Threads**: clone, futex
//! - **Time**: clock_gettime, nanosleep
//! - **I/O**: stdin/stdout/stderr via fd 0/1/2
//! - **OS**: environment variables, command-line arguments
//! - **Network**: stub (not yet available in kernel)
//!
//! # Usage
//!
//! This crate is `no_std` and uses inline assembly for syscall invocation.
//! It can also link against the VeridianOS libc for C-compatible wrappers.
//!
//! ```rust,no_run
//! use veridian_std::sys::veridian::{fs, process};
//!
//! // Write to stdout
//! let msg = b"Hello from VeridianOS!\n";
//! fs::write(1, msg);
//!
//! // Exit cleanly
//! process::exit(0);
//! ```

#![no_std]
#![allow(dead_code)]

pub mod sys;

/// Re-export the VeridianOS platform module for convenience.
pub use sys::veridian as platform;
