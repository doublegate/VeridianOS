//! User-space program entry point
//!
//! This module handles the initial entry from the kernel into user-space
//! and sets up the runtime environment before calling the user's main function.

use crate::syscall;

/// Architecture-specific entry points
#[cfg(target_arch = "x86_64")]
#[naked]
#[no_mangle]
pub unsafe extern "C" fn _start() -> ! {
    // Entry point from kernel
    // Stack contains:
    // - argc
    // - argv[0..argc]
    // - envp[0..]
    // - auxv[0..]
    
    core::arch::asm!(
        // Set up stack frame
        "mov rbp, rsp",
        
        // Call Rust entry point
        "call __veridian_start",
        
        // Should never return
        "ud2",
        options(noreturn)
    );
}

#[cfg(target_arch = "aarch64")]
#[naked]
#[no_mangle]
pub unsafe extern "C" fn _start() -> ! {
    core::arch::asm!(
        // Set up stack frame
        "mov x29, sp",
        
        // Call Rust entry point
        "bl __veridian_start",
        
        // Should never return
        "brk #0",
        options(noreturn)
    );
}

#[cfg(target_arch = "riscv64")]
#[naked]
#[no_mangle]
pub unsafe extern "C" fn _start() -> ! {
    core::arch::asm!(
        // Set up stack frame
        "mv s0, sp",
        
        // Call Rust entry point
        "call __veridian_start",
        
        // Should never return
        "ebreak",
        options(noreturn)
    );
}

/// Common entry point for all architectures
#[no_mangle]
unsafe extern "C" fn __veridian_start() -> ! {
    // Parse command line arguments from stack
    let argc = *(0 as *const isize);
    let argv = (0 as *const isize).offset(1) as *const *const u8;
    
    // Initialize runtime
    crate::allocator::init();
    
    // Get main function symbol
    extern "C" {
        fn main(argc: isize, argv: *const *const u8) -> i32;
    }
    
    // Call user's main function
    let exit_code = main(argc, argv);
    
    // Exit process
    syscall::exit(exit_code);
}