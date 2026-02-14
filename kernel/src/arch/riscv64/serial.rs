//! RISC-V serial output via SBI console putchar.
//!
//! Provides a `Uart16550Compat` wrapper that uses SBI ecall (function 0x01)
//! for console output. The QEMU virt machine places the 16550 UART at
//! `0x1000_0000`, but output goes through SBI rather than direct MMIO.

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
        // QEMU virt machine UART is already initialized by firmware
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
            // SAFETY: The ecall instruction invokes the SBI legacy console putchar
            // (function 0x01). a0 holds the character, a7 holds the function ID.
            // This is the standard mechanism for console output on RISC-V.
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
    uart.write_fmt(args).expect("serial write_fmt failed");
}
