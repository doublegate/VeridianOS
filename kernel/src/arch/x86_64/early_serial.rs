#![allow(clippy::macro_metavars_in_unsafe)]

// Early serial output for x86_64 boot debugging
// This bypasses lazy_static to allow output before static initialization

use core::fmt::Write;

/// Early serial port at 0x3F8 (COM1)
pub struct EarlySerial {
    base: u16,
}

impl Default for EarlySerial {
    fn default() -> Self {
        Self::new()
    }
}

impl EarlySerial {
    /// Create early serial port
    pub const fn new() -> Self {
        Self { base: 0x3F8 }
    }

    /// Initialize the serial port
    pub fn init(&mut self) {
        // SAFETY: Direct I/O port writes to configure the 16550 UART at the
        // base address (COM1 = 0x3F8). The initialization sequence (disable
        // IRQs, set baud rate via DLAB, configure line control, enable FIFO,
        // loopback test) is well-defined by the 16550 UART specification.
        unsafe {
            // Disable interrupts
            outb(self.base + 1, 0x00);

            // Enable DLAB (set baud rate divisor)
            outb(self.base + 3, 0x80);

            // Set divisor to 3 (lo byte) 38400 baud
            outb(self.base, 0x03);
            outb(self.base + 1, 0x00); // (hi byte)

            // 8 bits, no parity, one stop bit
            outb(self.base + 3, 0x03);

            // Enable FIFO, clear them, with 14-byte threshold
            outb(self.base + 2, 0xC7);

            // Enable IRQs, set RTS/DSR
            outb(self.base + 4, 0x0B);

            // Set loopback mode, test the serial chip
            outb(self.base + 4, 0x1E);

            // Test serial chip (send 0xAE and check if we receive it back)
            outb(self.base, 0xAE);

            // Check if we received the correct byte back
            if inb(self.base) != 0xAE {
                // Serial port is faulty, but continue anyway
            }

            // Set normal operation mode (not loopback)
            outb(self.base + 4, 0x0F);
        }
    }

    /// Write a single byte
    pub fn write_byte(&mut self, byte: u8) {
        // SAFETY: I/O port reads (line status register) and writes (transmit
        // buffer) to the 16550 UART. The port must have been initialized first.
        // We spin-wait until the transmit buffer is empty before sending.
        unsafe {
            // Wait for transmit buffer to be empty
            while (inb(self.base + 5) & 0x20) == 0 {
                core::hint::spin_loop();
            }

            // Send byte
            outb(self.base, byte);
        }
    }

    /// Write a string
    pub fn write_str(&mut self, s: &str) {
        for byte in s.bytes() {
            self.write_byte(byte);
        }
    }
}

impl Write for EarlySerial {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        self.write_str(s);
        Ok(())
    }
}

/// Read from I/O port
unsafe fn inb(port: u16) -> u8 {
    let value: u8;
    core::arch::asm!(
        "in al, dx",
        out("al") value,
        in("dx") port,
        options(nomem, nostack, preserves_flags)
    );
    value
}

/// Write to I/O port
unsafe fn outb(port: u16, value: u8) {
    core::arch::asm!(
        "out dx, al",
        in("dx") port,
        in("al") value,
        options(nomem, nostack, preserves_flags)
    );
}

/// Global early serial port instance
pub static mut EARLY_SERIAL: EarlySerial = EarlySerial::new();

/// Initialize early serial output
pub fn init() {
    // SAFETY: EARLY_SERIAL is a static mut EarlySerial initialized once during
    // early boot. addr_of_mut! avoids creating a mutable reference to the static
    // mut, instead operating through a raw pointer. No concurrent access is
    // possible during early single-threaded boot.
    unsafe {
        let serial = core::ptr::addr_of_mut!(EARLY_SERIAL);
        (*serial).init();
        (*serial).write_str("EARLY_SERIAL_OK\n");
    }
}

/// Early print macro for debugging
#[macro_export]
#[allow(clippy::macro_metavars_in_unsafe)]
macro_rules! early_print {
    ($($arg:tt)*) => {
        // SAFETY: addr_of_mut! accesses the EARLY_SERIAL static mut through
        // a raw pointer, avoiding a mutable reference. The write is safe
        // because this macro is only used during early single-threaded boot
        // before any concurrent access is possible.
        #[allow(clippy::macro_metavars_in_unsafe)]
        unsafe {
            use core::fmt::Write;
            let serial = core::ptr::addr_of_mut!($crate::arch::x86_64::early_serial::EARLY_SERIAL);
            let _ = write!(*serial, $($arg)*);
        }
    };
}

/// Early println macro for debugging
#[macro_export]
macro_rules! early_println {
    () => ($crate::early_print!("\n"));
    ($($arg:tt)*) => ($crate::early_print!("{}\n", format_args!($($arg)*)));
}
