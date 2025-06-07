#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![feature(abi_x86_interrupt)]
#![test_runner(crate::test_runner)]
#![reexport_test_harness_main = "test_main"]

use core::panic::PanicInfo;

#[macro_use]
mod print;

mod arch;
mod cap;
mod ipc;
mod mm;
mod sched;
mod serial;

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

    #[cfg(not(target_arch = "aarch64"))]
    {
        serial_println!("[KERNEL PANIC] {}", _info);
        exit_qemu(QemuExitCode::Failed);
    }

    #[cfg(target_arch = "aarch64")]
    loop {
        core::hint::spin_loop();
    }
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

        // For now, let's just loop with serial output
        writeln!(serial_port, "Kernel initialization complete!").unwrap();

        // Simple loop with periodic output
        let mut counter = 0u64;
        loop {
            if counter % 100000000 == 0 {
                writeln!(serial_port, "VeridianOS running... {}", counter / 100000000).unwrap();
            }
            counter = counter.wrapping_add(1);

            // Also update VGA periodically on x86_64
            #[cfg(target_arch = "x86_64")]
            if counter % 100000000 == 0 {
                unsafe {
                    let vga_buffer = 0xb8000 as *mut u8;
                    let offset = ((counter / 100000000) % 10) * 2;
                    *vga_buffer.offset(20 + offset as isize) =
                        b'0' + ((counter / 100000000) % 10) as u8;
                    *vga_buffer.offset(21 + offset as isize) = 0x0f;
                }
            }
        }
    }
}

#[cfg(test)]
fn test_runner(tests: &[&dyn Testable]) {
    serial_println!("Running {} tests", tests.len());
    for test in tests {
        test.run();
    }
    exit_qemu(QemuExitCode::Success);
}

#[cfg(test)]
trait Testable {
    fn run(&self) -> ();
}

#[cfg(test)]
impl<T> Testable for T
where
    T: Fn(),
{
    fn run(&self) {
        serial_print!("{}...\t", core::any::type_name::<T>());
        self();
        serial_println!("[ok]");
    }
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum QemuExitCode {
    Success = 0x10,
    Failed = 0x11,
}

#[cfg(test)]
pub fn exit_qemu(exit_code: QemuExitCode) -> ! {
    use x86_64::instructions::port::Port;

    unsafe {
        let mut port = Port::new(0xf4);
        port.write(exit_code as u32);
    }
    unreachable!();
}
