//! VeridianOS User-Space Runtime Library
//!
//! This library provides the basic runtime support for user-space programs
//! running on VeridianOS. It includes:
//! - Program startup and initialization
//! - System call wrappers
//! - Basic memory allocation
//! - Thread support
//! - Standard library primitives

#![no_std]
#![feature(lang_items)]
#![feature(naked_functions)]
#![feature(asm_const)]

// Re-export modules
pub mod entry;
pub mod syscall;
pub mod panic;
pub mod allocator;

// Language items required for no_std
#[lang = "eh_personality"]
fn eh_personality() {}

#[cfg(target_arch = "x86_64")]
#[lang = "start"]
fn start<T>(main: fn() -> T, _argc: isize, _argv: *const *const u8) -> isize
where
    T: Termination,
{
    main().report() as isize
}

/// Termination trait for main function return types
pub trait Termination {
    fn report(self) -> i32;
}

impl Termination for () {
    fn report(self) -> i32 {
        0
    }
}

impl Termination for i32 {
    fn report(self) -> i32 {
        self
    }
}

impl Termination for Result<(), i32> {
    fn report(self) -> i32 {
        match self {
            Ok(()) => 0,
            Err(code) => code,
        }
    }
}