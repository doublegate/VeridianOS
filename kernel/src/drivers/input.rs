//! Input multiplexer — unified character input from keyboard and serial.
//!
//! Checks all available input sources (PS/2 keyboard on x86_64, then serial)
//! and returns the first available byte. This replaces the per-architecture
//! inline serial reads in the shell.

/// Read a single character from any available input source (non-blocking).
///
/// On x86_64: polls PS/2 keyboard controller, checks ring buffer, then serial.
/// On AArch64: checks PL011 UART.
/// On RISC-V: checks SBI console_getchar.
pub fn read_char() -> Option<u8> {
    #[cfg(target_arch = "x86_64")]
    {
        // Poll the PS/2 keyboard controller directly. The APIC takes over
        // interrupt routing from the PIC, so IRQ1 may never fire. Polling
        // the keyboard controller status port (0x64) ensures keystrokes
        // from the QEMU graphical window are captured regardless.
        poll_ps2_keyboard();

        // Check decoded key buffer (filled by polling above or IRQ handler)
        if let Some(key) = crate::drivers::keyboard::read_key() {
            return Some(key);
        }
        // Fall back to serial port (COM1 at 0x3F8)
        read_serial_x86_64()
    }

    #[cfg(target_arch = "aarch64")]
    {
        read_uart_aarch64()
    }

    #[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
    {
        read_sbi_riscv()
    }
}

/// Poll the PS/2 keyboard controller for available scancodes.
///
/// Reads the keyboard controller status register (port 0x64). If bit 0
/// (Output Buffer Status) is set, a scancode is waiting in port 0x60.
/// We read it and feed it to the keyboard driver's decoder, which pushes
/// decoded ASCII bytes to the ring buffer.
///
/// This is necessary because the APIC (initialized during boot) takes
/// over interrupt routing from the legacy PIC. The PIC's IRQ1 (keyboard)
/// may never fire, so we poll instead.
#[cfg(target_arch = "x86_64")]
fn poll_ps2_keyboard() {
    let status: u8;
    // SAFETY: Reading the PS/2 controller status register (port 0x64).
    unsafe {
        core::arch::asm!(
            "in al, dx",
            out("al") status,
            in("dx") 0x64u16,
            options(nomem, nostack)
        );
    }
    if (status & 1) != 0 {
        let scancode: u8;
        // SAFETY: Reading the PS/2 data register (port 0x60) to get the
        // pending scancode. This clears the output buffer.
        unsafe {
            core::arch::asm!(
                "in al, dx",
                out("al") scancode,
                in("dx") 0x60u16,
                options(nomem, nostack)
            );
        }
        crate::drivers::keyboard::handle_scancode(scancode);
    }
}

/// Read from COM1 serial port (x86_64).
#[cfg(target_arch = "x86_64")]
fn read_serial_x86_64() -> Option<u8> {
    let status: u8;
    // SAFETY: Reading the Line Status Register (port 0x3FD) to check if
    // data is available. Port 0x3F8 is the COM1 data register.
    unsafe {
        core::arch::asm!(
            "in al, dx",
            out("al") status,
            in("dx") 0x3FDu16,
            options(nomem, nostack)
        );
    }
    if (status & 1) != 0 {
        let data: u8;
        unsafe {
            core::arch::asm!(
                "in al, dx",
                out("al") data,
                in("dx") 0x3F8u16,
                options(nomem, nostack)
            );
        }
        Some(data)
    } else {
        None
    }
}

/// Read from PL011 UART (AArch64, QEMU virt).
#[cfg(target_arch = "aarch64")]
fn read_uart_aarch64() -> Option<u8> {
    const UART_BASE: usize = 0x0900_0000;
    const UART_FR: usize = UART_BASE + 0x18; // Flag register
    const UART_DR: usize = UART_BASE; // Data register

    // SAFETY: Reading MMIO registers for PL011 UART. The QEMU virt
    // machine maps UART at this address.
    unsafe {
        let flags = core::ptr::read_volatile(UART_FR as *const u32);
        if (flags & (1 << 4)) == 0 {
            // RXFE bit clear = data available
            let data = core::ptr::read_volatile(UART_DR as *const u32);
            Some((data & 0xFF) as u8)
        } else {
            None
        }
    }
}

/// Read via SBI console_getchar (RISC-V).
#[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
fn read_sbi_riscv() -> Option<u8> {
    let result: isize;
    // SAFETY: SBI call to console_getchar (legacy extension 0x02).
    // Returns the character or -1 if no input is available.
    unsafe {
        core::arch::asm!(
            "li a7, 0x02",  // SBI_CONSOLE_GETCHAR
            "ecall",
            out("a0") result,
            out("a7") _,
            options(nomem)
        );
    }
    if result >= 0 {
        Some(result as u8)
    } else {
        None
    }
}
