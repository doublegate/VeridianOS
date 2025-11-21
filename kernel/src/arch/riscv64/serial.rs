// RISC-V serial implementation using 16550 UART

use core::fmt;

pub struct Uart16550Compat {
    #[allow(dead_code)]
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
        self.write_bytes(s.as_bytes());
        Ok(())
    }
}

impl Uart16550Compat {
    pub fn write_bytes(&self, bytes: &[u8]) {
        // Use SBI console putchar for output on RISC-V
        for &byte in bytes {
            unsafe {
                // SBI console putchar using ecall (legacy interface)
                core::arch::asm!(
                    "ecall",
                    in("a0") byte as usize,     // Character to print
                    in("a7") 0x01usize,       // SBI function ID 0x01 = console_putchar
                    options(nostack, nomem)
                );
            }
        }
    }

    pub fn write_str_direct(&self, s: &str) {
        self.write_bytes(s.as_bytes());
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
