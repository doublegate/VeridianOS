//! Hello World - First User Space Program
//!
//! Demonstrates basic user space execution and system call interface.

#![no_std]
#![no_main]

use core::panic::PanicInfo;

/// Entry point for user space application
#[no_mangle]
pub extern "C" fn _start() -> ! {
    // Use write syscall to output message
    let message = "Hello from VeridianOS user space!\n";
    sys_write(1, message.as_bytes());

    // Exit cleanly
    sys_exit(0);
}

/// Write syscall
fn sys_write(fd: usize, buf: &[u8]) {
    unsafe {
        core::arch::asm!(
            "syscall",
            in("rax") 1,  // SYS_WRITE
            in("rdi") fd,
            in("rsi") buf.as_ptr(),
            in("rdx") buf.len(),
            options(nostack)
        );
    }
}

/// Exit syscall
fn sys_exit(code: i32) -> ! {
    unsafe {
        core::arch::asm!(
            "syscall",
            in("rax") 60,  // SYS_EXIT
            in("rdi") code,
            options(noreturn, nostack)
        );
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    sys_exit(1);
}
