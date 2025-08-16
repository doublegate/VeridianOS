//! VeridianOS System Library
//! 
//! This library provides the interface between user-space programs and the kernel.

#![no_std]
#![feature(alloc_error_handler)]

extern crate alloc;

pub mod sys;
pub mod io;
pub mod allocator;

// Re-export commonly used items
pub use io::{print, println};
pub use sys::{exit, fork, exec, wait, getpid, sleep};

// Initialize the library
pub fn init() {
    allocator::init();
}

// User-space panic handler support
use core::panic::PanicInfo;

pub fn panic_handler_impl(info: &PanicInfo) -> ! {
    println!("PANIC: {}", info);
    sys::exit(255);
}