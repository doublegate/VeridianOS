// Generic serial interface for all architectures

use core::fmt;

pub struct SerialPort {
    #[cfg(target_arch = "x86_64")]
    inner: uart_16550::SerialPort,
    #[cfg(target_arch = "aarch64")]
    inner: Pl011Uart,
    #[cfg(target_arch = "riscv64")]
    inner: Uart16550Compat,
}

impl fmt::Write for SerialPort {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        #[cfg(target_arch = "x86_64")]
        {
            use core::fmt::Write;
            self.inner.write_str(s)
        }
        #[cfg(target_arch = "aarch64")]
        {
            self.inner.write_str(s)
        }
        #[cfg(target_arch = "riscv64")]
        {
            self.inner.write_str(s)
        }
    }
}

// Simple serial implementations for non-x86_64 architectures
#[cfg(target_arch = "aarch64")]
pub struct Pl011Uart {
    base_addr: usize,
}

#[cfg(target_arch = "aarch64")]
impl Pl011Uart {
    pub const fn new(base_addr: usize) -> Self {
        Self { base_addr }
    }

    pub fn init(&mut self) {
        // Simple PL011 initialization for QEMU
        // For QEMU virt machine, the UART is already initialized by firmware
    }
}

#[cfg(target_arch = "aarch64")]
impl fmt::Write for Pl011Uart {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        const UARTDR: usize = 0x000; // Data register
        const UARTFR: usize = 0x018; // Flag register
        const UARTFR_TXFF: u32 = 1 << 5; // Transmit FIFO full

        for byte in s.bytes() {
            unsafe {
                // Wait for FIFO to not be full
                while (core::ptr::read_volatile((self.base_addr + UARTFR) as *const u32)
                    & UARTFR_TXFF)
                    != 0
                {
                    core::hint::spin_loop();
                }
                // Write byte
                core::ptr::write_volatile((self.base_addr + UARTDR) as *mut u32, byte as u32);
            }
        }
        Ok(())
    }
}

#[cfg(target_arch = "riscv64")]
pub struct Uart16550Compat {
    base_addr: usize,
}

#[cfg(target_arch = "riscv64")]
impl Uart16550Compat {
    pub const fn new(base_addr: usize) -> Self {
        Self { base_addr }
    }

    pub fn init(&mut self) {
        // TODO: Initialize UART
    }
}

#[cfg(target_arch = "riscv64")]
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

impl SerialPort {
    #[cfg(target_arch = "x86_64")]
    pub fn from_inner(inner: uart_16550::SerialPort) -> Self {
        Self { inner }
    }

    #[cfg(target_arch = "aarch64")]
    pub fn from_inner(inner: Pl011Uart) -> Self {
        Self { inner }
    }

    #[cfg(target_arch = "riscv64")]
    pub fn from_inner(inner: Uart16550Compat) -> Self {
        Self { inner }
    }
}
