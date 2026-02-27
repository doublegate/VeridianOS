//! Framebuffer implementation
//!
//! Provides the low-level framebuffer device interface including physical
//! address tracking for user-space mmap and double-buffered rendering.

// Framebuffer implementation

use core::sync::atomic::{AtomicU64, Ordering};

use spin::Mutex;

use super::{Color, GraphicsContext, Rect};
use crate::error::KernelError;

/// Framebuffer information structure passed to user space via syscall.
///
/// Must be repr(C) for stable ABI across kernel/user boundary.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct FbInfo {
    pub width: u32,
    pub height: u32,
    pub pitch: u32,
    pub bpp: u32,
    pub phys_addr: u64,
    pub size: u64,
    /// Pixel format: 0 = BGRA, 1 = RGBA
    pub format: u32,
    pub _reserved: u32,
}

/// Framebuffer configuration
pub struct Framebuffer {
    width: u32,
    height: u32,
    pitch: u32,
    bpp: u8,
    buffer: Option<*mut u32>,
    /// Physical address of the framebuffer (for user-space mmap)
    phys_addr: u64,
    /// Total size of the framebuffer in bytes
    size: u64,
    /// Pixel format (0 = BGRA, 1 = RGBA)
    format: u32,
}

// SAFETY: Framebuffer contains a raw pointer to memory-mapped framebuffer
// memory that is shared via the Mutex<Framebuffer> wrapper. The pointer is
// only accessed while the Mutex is held, preventing data races. The
// underlying memory-mapped region is valid for the kernel's lifetime.
unsafe impl Send for Framebuffer {}
unsafe impl Sync for Framebuffer {}

impl Framebuffer {
    pub const fn new() -> Self {
        Self {
            width: 0,
            height: 0,
            pitch: 0,
            bpp: 32,
            buffer: None,
            phys_addr: 0,
            size: 0,
            format: 0,
        }
    }

    pub fn configure(&mut self, width: u32, height: u32, buffer: *mut u32) {
        self.width = width;
        self.height = height;
        self.pitch = width * 4;
        self.buffer = Some(buffer);
        self.size = (self.pitch as u64) * (height as u64);
    }

    /// Configure with physical address tracking (for user-space mmap).
    pub fn configure_with_phys(
        &mut self,
        width: u32,
        height: u32,
        pitch: u32,
        bpp: u8,
        buffer: *mut u32,
        phys_addr: u64,
        format: u32,
    ) {
        self.width = width;
        self.height = height;
        self.pitch = pitch;
        self.bpp = bpp;
        self.buffer = Some(buffer);
        self.phys_addr = phys_addr;
        self.size = (pitch as u64) * (height as u64);
        self.format = format;
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn pitch(&self) -> u32 {
        self.pitch
    }

    pub fn phys_addr(&self) -> u64 {
        self.phys_addr
    }

    pub fn size(&self) -> u64 {
        self.size
    }

    /// Get framebuffer info for user space.
    pub fn get_info(&self) -> FbInfo {
        FbInfo {
            width: self.width,
            height: self.height,
            pitch: self.pitch,
            bpp: self.bpp as u32,
            phys_addr: self.phys_addr,
            size: self.size,
            format: self.format,
            _reserved: 0,
        }
    }

    /// Get the raw buffer pointer (for compositor back-buffer swap).
    pub fn buffer_ptr(&self) -> Option<*mut u32> {
        self.buffer
    }
}

impl Default for Framebuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl GraphicsContext for Framebuffer {
    fn draw_pixel(&mut self, x: i32, y: i32, color: Color) {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return;
        }

        if let Some(buffer) = self.buffer {
            // SAFETY: 'buffer' is a raw pointer to the framebuffer's pixel memory,
            // set during initialization. The bounds check above guarantees x and y
            // are within [0, width) and [0, height) respectively, so the computed
            // offset is within the allocated framebuffer region.
            unsafe {
                let offset = (y as u32 * self.width + x as u32) as isize;
                *buffer.offset(offset) = color.to_u32();
            }
        }
    }

    fn draw_rect(&mut self, rect: Rect, color: Color) {
        // Draw top
        for x in rect.x..(rect.x + rect.width as i32) {
            self.draw_pixel(x, rect.y, color);
            self.draw_pixel(x, rect.y + rect.height as i32 - 1, color);
        }
        // Draw sides
        for y in rect.y..(rect.y + rect.height as i32) {
            self.draw_pixel(rect.x, y, color);
            self.draw_pixel(rect.x + rect.width as i32 - 1, y, color);
        }
    }

    fn fill_rect(&mut self, rect: Rect, color: Color) {
        for y in rect.y..(rect.y + rect.height as i32) {
            for x in rect.x..(rect.x + rect.width as i32) {
                self.draw_pixel(x, y, color);
            }
        }
    }

    fn clear(&mut self, color: Color) {
        self.fill_rect(
            Rect {
                x: 0,
                y: 0,
                width: self.width,
                height: self.height,
            },
            color,
        );
    }
}

static FRAMEBUFFER: Mutex<Framebuffer> = Mutex::new(Framebuffer::new());

/// Execute a closure with the framebuffer (mutable access)
pub fn with_framebuffer<R, F: FnOnce(&mut Framebuffer) -> R>(f: F) -> R {
    f(&mut FRAMEBUFFER.lock())
}

/// Global tracking of the framebuffer's physical address for user-space mmap.
/// Set during fbcon init when the bootloader provides framebuffer info.
static FB_PHYS_ADDR: AtomicU64 = AtomicU64::new(0);

/// Store the framebuffer physical address (called from bootstrap).
pub fn set_phys_addr(phys: u64) {
    FB_PHYS_ADDR.store(phys, Ordering::Release);
    FRAMEBUFFER.lock().phys_addr = phys;
}

/// Get the framebuffer physical address.
pub fn get_phys_addr() -> u64 {
    FB_PHYS_ADDR.load(Ordering::Acquire)
}

/// Get framebuffer info (safe to call from syscall context).
pub fn get_fb_info() -> Option<FbInfo> {
    let fb = FRAMEBUFFER.lock();
    if fb.width == 0 || fb.height == 0 {
        return None;
    }
    Some(fb.get_info())
}

/// Initialize framebuffer
#[cfg_attr(target_arch = "aarch64", allow(unused_variables))]
pub fn init() -> Result<(), KernelError> {
    println!("[FB] Initializing framebuffer...");

    let fb = FRAMEBUFFER.lock();

    println!(
        "[FB] Framebuffer initialized ({}x{}, phys=0x{:x})",
        fb.width(),
        fb.height(),
        fb.phys_addr(),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_framebuffer_create() {
        let fb = Framebuffer::new();
        assert_eq!(fb.width(), 0);
        assert_eq!(fb.height(), 0);
    }
}
