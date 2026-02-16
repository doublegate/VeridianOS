//! x86_64 boot entry point using the `bootloader` crate.
//!
//! Receives `BootInfo` from the bootloader and performs early hardware
//! initialization including serial setup, VGA output, and kernel heap init.

use bootloader_api::BootInfo;

// Store boot info for later use (mutable reference for 0.11+ API)
//
// SAFETY JUSTIFICATION: This static mut is intentionally kept because:
// 1. The bootloader crate API provides boot info as &'static mut BootInfo
// 2. This must be stored before any kernel initialization (pre-heap)
// 3. Written once during entry_point! macro callback, read-only afterwards
// 4. Cannot use OnceLock as it requires heap (alloc) which isn't available yet
// 5. The bootloader guarantees the reference is valid for the kernel lifetime
#[allow(static_mut_refs)]
pub static mut BOOT_INFO: Option<&'static mut BootInfo> = None;

// Early initialization that must happen before kernel_main
pub fn early_boot_init() {
    // SAFETY: The cli instruction disables hardware interrupts. Required during
    // early boot to prevent interrupt handlers from firing before the IDT is set
    // up.
    unsafe {
        core::arch::asm!("cli", options(nomem, nostack));
    }

    // SAFETY: 0xb8000 is the standard VGA text buffer address on x86 PCs.
    // write_volatile ensures the write is not optimized away. Always mapped
    // during early boot.
    unsafe {
        let vga = 0xb8000 as *mut u16;
        // Write 'B' in white on black
        vga.write_volatile(0x0F42);
    }

    // SAFETY: Direct I/O port writes to COM1 (0x3F8) to initialize the serial
    // port for early boot debugging. The 16550 UART initialization sequence
    // (disable IRQs, set baud rate, configure line control) is well-defined.
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

/// Framebuffer information extracted from BootInfo.
pub struct BootFramebufferInfo {
    pub buffer: *mut u8,
    pub width: usize,
    pub height: usize,
    /// Stride in bytes (bytes per scanline)
    pub stride: usize,
    /// Bytes per pixel (typically 4 for BGRA)
    pub bpp: usize,
    /// Pixel format (BGR or RGB)
    pub is_bgr: bool,
}

/// Extract framebuffer information from the UEFI boot info.
///
/// Returns `None` if BootInfo is unavailable or has no framebuffer.
/// The returned pointer is already a valid virtual address (the bootloader
/// maps the framebuffer with `Mapping::Dynamic`).
pub fn get_framebuffer_info() -> Option<BootFramebufferInfo> {
    // SAFETY: BOOT_INFO is a static mut written once during early boot
    // (in the entry_point! callback) and read-only afterwards. We are in
    // single-threaded kernel init context (Stage 2+, before scheduler).
    #[allow(static_mut_refs)]
    let boot_info = unsafe { BOOT_INFO.as_mut()? };
    let fb = boot_info.framebuffer.as_mut()?;
    let info = fb.info();
    let buffer_ptr = fb.buffer_mut().as_mut_ptr();
    Some(BootFramebufferInfo {
        buffer: buffer_ptr,
        width: info.width,
        height: info.height,
        stride: info.stride * info.bytes_per_pixel,
        bpp: info.bytes_per_pixel,
        is_bgr: matches!(info.pixel_format, bootloader_api::info::PixelFormat::Bgr),
    })
}

// I/O port functions: delegate to the canonical implementations in
// the parent module (arch/x86_64/mod.rs) to avoid duplication.
use super::{inb, outb};

#[inline]
unsafe fn write_str(base: u16, s: &str) {
    for byte in s.bytes() {
        // Wait for transmit buffer to be empty
        loop {
            let status = inb(base + 5);
            if (status & 0x20) != 0 {
                break;
            }
        }
        // Send byte
        outb(base, byte);
    }
}
