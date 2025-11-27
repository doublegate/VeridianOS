// Boot entry point for x86_64

use bootloader_api::BootInfo;

// Store boot info for later use (mutable reference for 0.11+ API)
pub static mut BOOT_INFO: Option<&'static mut BootInfo> = None;

// Early initialization that must happen before kernel_main
pub fn early_boot_init() {
    // Disable interrupts immediately
    unsafe {
        core::arch::asm!("cli", options(nomem, nostack));
    }

    // Initialize VGA text buffer first for immediate feedback
    unsafe {
        let vga = 0xb8000 as *mut u16;
        // Write 'B' in white on black
        vga.write_volatile(0x0F42);
    }

    // Initialize early serial first thing
    unsafe {
        // Direct serial port initialization at 0x3F8
        let base: u16 = 0x3F8;

        // Disable interrupts
        outb(base + 1, 0x00);
        // Enable DLAB
        outb(base + 3, 0x80);
        // Set divisor to 3 (38400 baud)
        outb(base, 0x03);
        outb(base + 1, 0x00);
        // 8 bits, no parity, one stop bit
        outb(base + 3, 0x03);
        // Enable FIFO
        outb(base + 2, 0xC7);
        // Enable IRQs, set RTS/DSR
        outb(base + 4, 0x0B);

        // Write boot marker
        write_str(base, "BOOT_ENTRY\n");
    }
}

#[inline]
unsafe fn outb(port: u16, value: u8) {
    core::arch::asm!(
        "out dx, al",
        in("dx") port,
        in("al") value,
        options(nomem, nostack, preserves_flags)
    );
}

#[inline]
unsafe fn write_str(base: u16, s: &str) {
    for byte in s.bytes() {
        // Wait for transmit buffer to be empty
        loop {
            let status: u8;
            core::arch::asm!(
                "in al, dx",
                out("al") status,
                in("dx") base + 5,
                options(nomem, nostack, preserves_flags)
            );
            if (status & 0x20) != 0 {
                break;
            }
        }
        // Send byte
        outb(base, byte);
    }
}
