//! VeridianOS System Library
//! 
//! This library provides the interface between user-space programs and the kernel.

#![no_std]

pub mod sys;
pub mod io;

// Re-export commonly used items
pub use io::{print, println};

// User-space panic handler support
use core::panic::PanicInfo;

pub fn panic_handler_impl(info: &PanicInfo) -> ! {
    println!("PANIC: {}", info);
    sys::exit(255);
}