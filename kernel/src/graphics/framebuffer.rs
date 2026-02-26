//! Framebuffer implementation

// Framebuffer implementation

use spin::Mutex;

use super::{Color, GraphicsContext, Rect};
use crate::error::KernelError;

/// Framebuffer configuration
pub struct Framebuffer {
    width: u32,
    height: u32,
    pitch: u32,
    #[allow(dead_code)] // Bits-per-pixel for framebuffer configuration
    bpp: u8,
    buffer: Option<*mut u32>,
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
        }
    }

    pub fn configure(&mut self, width: u32, height: u32, buffer: *mut u32) {
        self.width = width;
        self.height = height;
        self.pitch = width * 4;
        self.buffer = Some(buffer);
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
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

/// Initialize framebuffer
#[cfg_attr(target_arch = "aarch64", allow(unused_variables))]
pub fn init() -> Result<(), KernelError> {
    println!("[FB] Initializing framebuffer...");

    // TODO(phase6): Get framebuffer info from bootloader (VBE/GOP detection)
    let fb = FRAMEBUFFER.lock();

    println!(
        "[FB] Framebuffer initialized ({}x{})",
        fb.width(),
        fb.height()
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
