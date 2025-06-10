//! # VeridianOS Microkernel
//!
//! A next-generation microkernel operating system written in Rust.
//!
//! ## Architecture Support
//!
//! - x86_64 - Full support with UEFI/BIOS boot
//! - AArch64 - Full support with device tree
//! - RISC-V - Full support with OpenSBI
//!
//! ## Key Components
//!
//! - [`mm`] - Memory management subsystem
//! - [`sched`] - Process scheduling
//! - [`ipc`] - Inter-process communication
//! - [`cap`] - Capability-based security
//!
//! ## Safety
//!
//! This is kernel code - most functions are `unsafe` and require careful
//! handling. See individual module documentation for specific requirements.

#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![feature(abi_x86_interrupt)]
#![test_runner(crate::test_runner)]
#![reexport_test_harness_main = "test_main"]
#![doc(
    html_logo_url = "https://raw.githubusercontent.com/doublegate/VeridianOS/main/docs/assets/logo.png",
    html_favicon_url = "https://raw.githubusercontent.com/doublegate/VeridianOS/main/images/veridian_os.ico",
    issue_tracker_base_url = "https://github.com/doublegate/VeridianOS/issues/"
)]

use core::panic::PanicInfo;

use linked_list_allocator::LockedHeap;

#[global_allocator]
static ALLOCATOR: LockedHeap = LockedHeap::empty();

#[macro_use]
mod print;

mod arch;
mod bench;
mod cap;
mod ipc;
mod mm;
mod process;
mod sched;
mod serial;
mod syscall;

#[cfg(test)]
mod test_framework;

#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    // For AArch64, just write PANIC to UART and loop
    #[cfg(target_arch = "aarch64")]
    unsafe {
        let uart = 0x0900_0000 as *mut u8;
        core::ptr::write_volatile(uart, b'P');
        core::ptr::write_volatile(uart, b'A');
        core::ptr::write_volatile(uart, b'N');
        core::ptr::write_volatile(uart, b'I');
        core::ptr::write_volatile(uart, b'C');
        core::ptr::write_volatile(uart, b'\n');
    }

    #[cfg(target_arch = "x86_64")]
    println!("[KERNEL PANIC] {}", _info);

    arch::halt();
}

#[cfg(test)]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    test_framework::test_panic_handler(info)
}

#[no_mangle]
pub extern "C" fn kernel_main() -> ! {
    // AArch64 implementation
    #[cfg(target_arch = "aarch64")]
    {
        // Simple UART output for AArch64 - no iterators!
        unsafe {
            let uart = 0x0900_0000 as *mut u8;
            // Write "KERNEL MAIN\n" manually
            *uart = b'K';
            *uart = b'E';
            *uart = b'R';
            *uart = b'N';
            *uart = b'E';
            *uart = b'L';
            *uart = b' ';
            *uart = b'M';
            *uart = b'A';
            *uart = b'I';
            *uart = b'N';
            *uart = b'\n';
        }

        // Simple loop
        loop {
            unsafe {
                core::arch::asm!("wfe");
            }
        }
    }

    #[cfg(not(target_arch = "aarch64"))]
    {
        // Initialize serial port first for debugging (architecture-specific)
        let mut serial_port = arch::serial_init();

        // Write to serial port directly
        use core::fmt::Write;
        writeln!(serial_port, "VeridianOS kernel started!").unwrap();

        // Architecture-specific early output
        #[cfg(target_arch = "x86_64")]
        {
            // First, let's just try to write directly to VGA buffer to test
            unsafe {
                let vga_buffer = 0xb8000 as *mut u8;
                *vga_buffer = b'H';
                *vga_buffer.offset(1) = 0x0f; // white on black
                *vga_buffer.offset(2) = b'E';
                *vga_buffer.offset(3) = 0x0f;
                *vga_buffer.offset(4) = b'L';
                *vga_buffer.offset(5) = 0x0f;
                *vga_buffer.offset(6) = b'L';
                *vga_buffer.offset(7) = 0x0f;
                *vga_buffer.offset(8) = b'O';
                *vga_buffer.offset(9) = 0x0f;
            }

            writeln!(serial_port, "VGA buffer write complete").unwrap();
        }

        // For now, let's skip the println! macros and just use serial
        writeln!(serial_port, "VeridianOS v{}", env!("CARGO_PKG_VERSION")).unwrap();
        writeln!(serial_port, "Initializing microkernel...").unwrap();

        // Initialize architecture-specific features
        writeln!(serial_port, "Initializing arch...").unwrap();
        arch::init();
        writeln!(serial_port, "Arch initialized").unwrap();

        // Initialize memory management
        writeln!(serial_port, "Initializing memory management...").unwrap();
        mm::init_default();
        writeln!(serial_port, "Memory management initialized").unwrap();

        // Initialize kernel heap
        writeln!(serial_port, "Initializing kernel heap...").unwrap();
        if let Err(e) = mm::init_heap() {
            writeln!(serial_port, "Failed to initialize heap: {}", e).unwrap();
        } else {
            writeln!(serial_port, "Kernel heap initialized").unwrap();

            // Test heap allocation
            #[cfg(feature = "alloc")]
            {
                extern crate alloc;
                use alloc::boxed::Box;
                let test_box = Box::new(42);
                writeln!(serial_port, "Heap test: Box::new(42) = {}", *test_box).unwrap();
            }
        }

        // Initialize IPC
        writeln!(serial_port, "Initializing IPC...").unwrap();
        ipc::init();
        writeln!(serial_port, "IPC initialized").unwrap();

        // Initialize process management
        writeln!(serial_port, "Initializing process management...").unwrap();
        process::init();
        writeln!(serial_port, "Process management initialized").unwrap();

        // Initialize scheduler
        writeln!(serial_port, "Initializing scheduler...").unwrap();
        sched::init();
        writeln!(serial_port, "Scheduler initialized").unwrap();

        // Create test processes
        #[cfg(feature = "alloc")]
        {
            extern crate alloc;
            use alloc::string::ToString;

            writeln!(serial_port, "Creating test processes...").unwrap();

            // Create first test process
            match process::lifecycle::create_process("test_process_1".to_string(), 0) {
                Ok(pid1) => {
                    writeln!(serial_port, "Created test process 1 with PID {}", pid1.0).unwrap();

                    // Create a thread for it
                    if let Some(proc) = process::table::get_process_mut(pid1) {
                        let entry = test_task_1 as usize;
                        match process::create_thread(entry, 0, 0, 0) {
                            Ok(tid) => {
                                writeln!(
                                    serial_port,
                                    "Created thread {} for process {}",
                                    tid.0, pid1.0
                                )
                                .unwrap();

                                // Schedule the thread
                                if let Some(thread) = proc.get_thread(tid) {
                                    if let Err(e) = sched::schedule_thread(pid1, tid, thread) {
                                        writeln!(serial_port, "Failed to schedule thread: {}", e)
                                            .unwrap();
                                    } else {
                                        writeln!(serial_port, "Thread scheduled successfully")
                                            .unwrap();
                                    }
                                }
                            }
                            Err(e) => {
                                writeln!(serial_port, "Failed to create thread: {}", e).unwrap()
                            }
                        }
                    }
                }
                Err(e) => writeln!(serial_port, "Failed to create process 1: {}", e).unwrap(),
            }

            // Create second test process
            match process::lifecycle::create_process("test_process_2".to_string(), 0) {
                Ok(pid2) => {
                    writeln!(serial_port, "Created test process 2 with PID {}", pid2.0).unwrap();

                    // Create a thread for it
                    if let Some(proc) = process::table::get_process_mut(pid2) {
                        let entry = test_task_2 as usize;
                        match process::create_thread(entry, 0, 0, 0) {
                            Ok(tid) => {
                                writeln!(
                                    serial_port,
                                    "Created thread {} for process {}",
                                    tid.0, pid2.0
                                )
                                .unwrap();

                                // Schedule the thread
                                if let Some(thread) = proc.get_thread(tid) {
                                    if let Err(e) = sched::schedule_thread(pid2, tid, thread) {
                                        writeln!(serial_port, "Failed to schedule thread: {}", e)
                                            .unwrap();
                                    } else {
                                        writeln!(serial_port, "Thread scheduled successfully")
                                            .unwrap();
                                    }
                                }
                            }
                            Err(e) => {
                                writeln!(serial_port, "Failed to create thread: {}", e).unwrap()
                            }
                        }
                    }
                }
                Err(e) => writeln!(serial_port, "Failed to create process 2: {}", e).unwrap(),
            }
        }

        // For now, let's just loop with serial output
        writeln!(serial_port, "Kernel initialization complete!").unwrap();

        // Start scheduler - this will run the idle task
        writeln!(serial_port, "Starting scheduler...").unwrap();
        sched::run();
    }
}

// Test task 1
#[cfg(feature = "alloc")]
#[allow(dead_code)]
extern "C" fn test_task_1() {
    use core::fmt::Write;

    // Use architecture-specific serial port initialization
    let mut serial_port = arch::serial_init();
    let mut counter = 0;

    loop {
        if counter % 1000 == 0 {
            writeln!(serial_port, "[Task 1] Running... count={}", counter).unwrap();
        }
        counter += 1;

        // Yield to other tasks
        if counter % 100 == 0 {
            sched::yield_cpu();
        }
    }
}

// Test task 2
#[cfg(feature = "alloc")]
#[allow(dead_code)]
extern "C" fn test_task_2() {
    use core::fmt::Write;

    // Use architecture-specific serial port initialization
    let mut serial_port = arch::serial_init();
    let mut counter = 0;

    loop {
        if counter % 1000 == 0 {
            writeln!(serial_port, "[Task 2] Running... count={}", counter).unwrap();
        }
        counter += 1;

        // Yield to other tasks
        if counter % 150 == 0 {
            sched::yield_cpu();
        }
    }
}

#[cfg(test)]
use test_framework::{exit_qemu, QemuExitCode, Testable};

#[cfg(test)]
fn test_runner(tests: &[&dyn Testable]) {
    test_framework::test_runner(tests)
}
