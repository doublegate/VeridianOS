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
    #[allow(dead_code)]
    pub const fn new(base_addr: usize) -> Self {
        Self { base_addr }
    }

    #[allow(dead_code)]
    pub fn init(&mut self) {
        // Simple PL011 initialization for QEMU
        // For QEMU virt machine, the UART is already initialized by firmware
    }
}

#[cfg(target_arch = "aarch64")]
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
    #[allow(dead_code)]
    pub fn from_inner(inner: uart_16550::SerialPort) -> Self {
        Self { inner }
    }

    #[cfg(target_arch = "aarch64")]
    #[allow(dead_code)]
    pub fn from_inner(inner: Pl011Uart) -> Self {
        Self { inner }
    }

    #[cfg(target_arch = "riscv64")]
    #[allow(dead_code)]
    pub fn from_inner(inner: Uart16550Compat) -> Self {
        Self { inner }
    }
}

// Serial print macros for testing
#[macro_export]
macro_rules! serial_print {
    ($($arg:tt)*) => {
        $crate::serial::_serial_print(format_args!($($arg)*))
    };
}

#[macro_export]
macro_rules! serial_println {
    () => ($crate::serial_print!("\n"));
    ($($arg:tt)*) => {
        $crate::serial_print!("{}\n", format_args!($($arg)*))
    };
}

#[doc(hidden)]
pub fn _serial_print(args: fmt::Arguments) {
    use core::fmt::Write;

    #[cfg(target_arch = "x86_64")]
    {
        use uart_16550::SerialPort;
        use x86_64::instructions::interrupts;

        interrupts::without_interrupts(|| {
            let mut port = unsafe { SerialPort::new(0x3F8) };
            port.write_fmt(args).unwrap();
        });
    }

    #[cfg(target_arch = "aarch64")]
    {
        let mut uart = Pl011Uart::new(0x0900_0000);
        uart.write_fmt(args).unwrap();
    }

    #[cfg(target_arch = "riscv64")]
    {
        let mut uart = Uart16550Compat::new(0x1000_0000);
        uart.write_fmt(args).unwrap();
    }
}
