// RISC-V serial implementation using 16550 UART

use core::fmt;

pub struct Uart16550Compat {
    base_addr: usize,
}

impl Uart16550Compat {
    pub const fn new(base_addr: usize) -> Self {
        Self { base_addr }
    }

    pub fn init(&mut self) {
        // TODO: Initialize UART
        // For QEMU virt machine, the UART is already initialized
    }
}

impl fmt::Write for Uart16550Compat {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        const THR: usize = 0x00; // Transmitter Holding Register
        const LSR: usize = 0x05; // Line Status Register
        const LSR_THRE: u8 = 1 << 5; // Transmitter Holding Register Empty

        for byte in s.bytes() {
            unsafe {
                // Wait for transmitter to be ready
                while (core::ptr::read_volatile((self.base_addr + LSR) as *const u8) & LSR_THRE)
                    == 0
                {
                    core::hint::spin_loop();
                }
                // Write byte
                core::ptr::write_volatile((self.base_addr + THR) as *mut u8, byte);
            }
        }
        Ok(())
    }
}

pub type SerialPort = Uart16550Compat;

pub fn create_serial_port() -> SerialPort {
    // QEMU virt machine places 16550 UART at 0x10000000
    let mut uart = Uart16550Compat::new(0x1000_0000);
    uart.init();
    uart
}

#[doc(hidden)]
pub fn _serial_print(args: fmt::Arguments) {
    use core::fmt::Write;
    let mut uart = create_serial_port();
    uart.write_fmt(args).unwrap();
}