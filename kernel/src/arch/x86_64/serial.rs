//! x86_64 serial port driver for kernel debugging output.
//!
//! Uses the `uart_16550` crate to interface with COM1 at I/O port 0x3F8.
//! Provides `serial_print!` and `serial_println!` macros for formatted output.

use lazy_static::lazy_static;
use spin::Mutex;
use uart_16550::SerialPort;

lazy_static! {
    pub static ref SERIAL1: Mutex<SerialPort> = {
        let mut serial_port = unsafe { SerialPort::new(0x3F8) };
        serial_port.init();
        Mutex::new(serial_port)
    };
}

#[doc(hidden)]
pub fn _print(args: ::core::fmt::Arguments) {
    use core::fmt::Write;

    use x86_64::instructions::interrupts;

    interrupts::without_interrupts(|| {
        SERIAL1
            .lock()
            .write_fmt(args)
            .expect("Printing to serial failed");
    });
}

// Alias for compatibility
#[doc(hidden)]
pub fn _serial_print(args: ::core::fmt::Arguments) {
    _print(args);
}
