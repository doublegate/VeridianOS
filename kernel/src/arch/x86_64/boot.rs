// Boot entry point for x86_64

use bootloader::{entry_point, BootInfo};

// Store boot info for later use
pub static mut BOOT_INFO: Option<&'static BootInfo> = None;

entry_point!(kernel_main_entry);

fn kernel_main_entry(boot_info: &'static BootInfo) -> ! {
    // Disable interrupts immediately
    unsafe {
        core::arch::asm!("cli", options(nomem, nostack));
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
        outb(base + 0, 0x03);
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
    
    // Store boot info in a static for later use
    unsafe {
        BOOT_INFO = Some(boot_info);
    }
    
    // Call the real kernel_main from main.rs
    extern "C" {
        fn kernel_main() -> !;
    }
    unsafe { kernel_main() }
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
