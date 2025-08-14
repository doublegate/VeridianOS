// AArch64 serial implementation using PL011 UART

use core::fmt;

pub struct Pl011Uart {
    base_addr: usize,
}

impl Pl011Uart {
    pub const fn new(base_addr: usize) -> Self {
        Self { base_addr }
    }

    pub fn init(&mut self) {
        // Simple PL011 initialization for QEMU
        // For QEMU virt machine, the UART is already initialized by firmware
    }
}

impl fmt::Write for Pl011Uart {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        const UARTDR: usize = 0x000; // Data register

        // Direct UART access without iterators for AArch64
        let bytes = s.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            unsafe {
                // Direct write without FIFO check for simplicity
                let uart_dr = (self.base_addr + UARTDR) as *mut u8;
                *uart_dr = bytes[i];
            }
            i += 1;
        }
        Ok(())
    }
}

pub type SerialPort = Pl011Uart;

pub fn create_serial_port() -> SerialPort {
    Pl011Uart::new(0x0900_0000)
}

#[doc(hidden)]
pub fn _serial_print(args: fmt::Arguments) {
    use core::fmt::Write;
    let mut uart = create_serial_port();
    uart.write_fmt(args).unwrap();
}